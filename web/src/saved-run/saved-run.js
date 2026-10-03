/**
 * Browser saved runs: manifests, history segments, and loaded-bundle validation.
 * Framing and checkpoint restore are WASM's, from the format shared with the native
 * shell (spec §7.10); this module owns no I/O, storage, or worker state.
 */

export const FORMAT = 'synthetic-evolution-saved-run';
export const VERSION = 1;
/** Bundles are read whole into memory, so the tab bounds what it accepts. */
export const MAX_SAVED_RUN_BYTES = 256 * 1024 * 1024;
/** Largest saved core budget (`storage.max_memory_bytes`) a tab will restore. */
export const MAX_CORE_BYTES = 256n * 1024n * 1024n;

const encoder = new TextEncoder();

export function cohortFor(heredity) {
  if (heredity === 'evolving') return 'evolving';
  if (heredity === 'randomized_at_birth') return 'random_control';
  throw new Error(`unknown heredity ${heredity}`);
}

function included(startsAt, text) {
  return { segment: { status: 'included', starts_at: String(startsAt), bytes: String(encoder.encode(text).byteLength) }, text };
}

function notRecorded(startsAt) {
  return { segment: { status: 'unavailable', starts_at: String(startsAt), reason: 'not_recorded' }, text: null };
}

/**
 * History segments for saving a world at `tick`: restored segments as loaded, then
 * this world's own capture since `startTick`. An archive that ended before the save
 * is followed by an explicit `not_recorded` gap; nothing is reconstructed.
 *
 * `archive` is `{ text, end }` with `end` its completion tick, or null.
 */
export function segmentsFor({ restored, startTick, tick, archive }) {
  const segments = [...restored];
  const start = BigInt(startTick);
  const now = BigInt(tick);
  if (archive) {
    // An archive may end where it began, holding only that tick's seeding events.
    const end = BigInt(archive.end);
    segments.push(included(start, archive.text));
    if (end < now) segments.push(notRecorded(end));
  } else if (restored.length === 0 || now > start) {
    segments.push(notRecorded(start));
  }
  return segments;
}

export function manifestFor({ provenance, writer, checkpointFormat, tick, cohort, stateHash, checkpoint, segments }) {
  return {
    format: FORMAT,
    version: VERSION,
    checkpoint_format: checkpointFormat,
    provenance,
    writer,
    tick: String(tick),
    cohorts: [{ cohort, state_hash: stateHash, bytes: String(checkpoint.byteLength) }],
    history: segments.map(({ segment }) => segment),
  };
}

/** Sections in container order: the checkpoint, then each included archive. */
export function sectionsFor(checkpoint, segments) {
  const parts = [checkpoint, ...segments.filter(({ text }) => text !== null).map(({ text }) => encoder.encode(text))];
  const out = new Uint8Array(parts.reduce((sum, part) => sum + part.byteLength, 0));
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.byteLength;
  }
  return out;
}

/** Slices a WASM-decoded bundle. Archive text must be exact UTF-8. */
export function sectionsOf(bytes, decoded) {
  const slice = ([offset, length]) => bytes.subarray(offset, offset + length);
  const utf8 = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });
  return {
    checkpoints: decoded.checkpoints.map(slice),
    texts: decoded.history.map((span) => (span === null ? null : utf8.decode(slice(span)))),
  };
}

/**
 * Validates each included archive against its segment, as the native reader does:
 * same seed, covering the bundle's cohorts, ending exactly at the segment boundary
 * so no later events leak in, and resuming at its own start unless it is the run's
 * first segment. A paired archive may accompany one cohort loaded from a paired run.
 * `parse` is `parseArchive` with the WASM validators.
 */
export async function validateSegments(manifest, texts, parse) {
  const cohorts = manifest.cohorts.map((entry) => entry.cohort);
  const segments = [];
  for (const [index, segment] of manifest.history.entries()) {
    const text = texts[index];
    if (text === null) {
      segments.push({ segment, text: null, archive: null });
      continue;
    }
    const end = manifest.history[index + 1]?.starts_at ?? manifest.tick;
    const archive = await parse(text);
    const header = archive.header.data;
    const resumed = header.resumed_from_tick ?? null;
    if (header.provenance.seed !== manifest.provenance.seed ||
      archive.completion.data.ticks !== end ||
      !cohorts.every((cohort) => header.cohorts.includes(cohort)) ||
      resumed !== (segment.starts_at === '0' ? null : segment.starts_at)) {
      throw new Error(`Saved run history segment ${index} does not belong to this run, its cohorts, or its boundary`);
    }
    segments.push({ segment, text, archive });
  }
  return segments;
}
