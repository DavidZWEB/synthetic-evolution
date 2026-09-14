/** Versioned species-history archives, independent of storage, World, and presentation. */
import {
  decimal, equal, object, requireThat, uint, U64_MAX, validateParamsShape,
} from './shape.js';
import { acceptRow, COUNT_KEYS, newState, wireCounts } from './lifecycle.js';
import { byteLength, encodeLine, parseLine } from './json-lines.js';

export { byteLength, encodeLine, MAX_LINE_BYTES } from './json-lines.js';

const COHORTS = ['evolving', 'random_control'];
const PROVENANCE_KEYS = ['sim_version', 'source_revision', 'phase', 'seed', 'control'];
const HEADER_KEYS = [
  'schema_version', 'provenance', 'cohorts', 'ticks', 'founders',
  'drain_every', 'capacity_per_cohort', 'params',
];
const COMPLETION_KEYS = ['schema_version', 'provenance', 'ticks', 'cohorts'];
export const INCOMPLETE_ENDS = ['unfinalized', 'storage_limit', 'storage_error', 'capture_error'];
const CAPTURE_ENDS = ['finished', 'snapshot', 'stopped', 'reseeded', 'params_changed', ...INCOMPLETE_ENDS];
// BirthIdentities-era Option<history::Record> is 64 bytes; native's portable ceiling is i32::MAX.
const MAX_RECORDER_CAPACITY = Math.floor(2147483647 / 64);

function record(row, kind) {
  object(row, ['kind', 'data'], 'record');
  requireThat(row.kind === kind, `expected ${kind} record`);
  encodeLine(row);
}

function provenance(data) {
  object(data, PROVENANCE_KEYS, 'provenance');
  for (const key of ['sim_version', 'source_revision']) {
    requireThat(typeof data[key] === 'string' && data[key].trim() !== '' &&
      data[key].isWellFormed(), `invalid ${key}`);
  }
  requireThat(data.phase === 2 && data.control === 'randomized_at_birth_v3',
    'unsupported history phase/control protocol');
  decimal(data.seed, 'seed');
}

export function validateHeader(header) {
  record(header, 'header');
  const data = header.data;
  requireThat(data?.schema_version === 1 || data?.schema_version === 2, 'unsupported history schema');
  const v2 = data.schema_version === 2;
  object(data, v2 ? [...HEADER_KEYS, 'run_id'] : HEADER_KEYS, 'header');
  provenance(data.provenance);
  requireThat(equal(data.cohorts, COHORTS) || (v2 && Array.isArray(data.cohorts) &&
    data.cohorts.length === 1 && COHORTS.includes(data.cohorts[0])), 'invalid cohort membership/order');
  if (v2) {
    requireThat(typeof data.run_id === 'string' && data.run_id.length > 0 &&
      data.run_id.isWellFormed() && [...data.run_id].length <= 128, 'invalid run ID');
  }
  if (!v2 || data.ticks !== null) decimal(data.ticks, 'planned ticks');
  if (!v2 || data.drain_every !== null) {
    requireThat(decimal(data.drain_every, 'drain interval') > 0n, 'drain interval must be positive');
  }
  uint(data.founders, 'founders');
  uint(data.capacity_per_cohort, 'recorder capacity');
  requireThat(data.capacity_per_cohort > 0 && data.capacity_per_cohort <= MAX_RECORDER_CAPACITY,
    'recorder capacity is outside the portable byte ceiling');
  validateParamsShape(data.params);
  requireThat(data.founders > 0 && data.founders <= data.params.world.max_agents, 'invalid founder count');
  return header;
}

function readPrefix(header, rows, tick) {
  validateHeader(header);
  requireThat(Array.isArray(rows), 'rows must be an array');
  const planned = header.data.ticks === null ? null : decimal(header.data.ticks);
  const boundary = tick === undefined ? planned : decimal(tick, 'committed tick');
  requireThat(planned === null || boundary === null || boundary <= planned, 'boundary exceeds planned ticks');
  const states = Object.fromEntries(header.data.cohorts.map((cohort) => [cohort, newState()]));
  for (const row of rows) {
    object(row, ['kind', 'data'], 'record');
    encodeLine(row);
    requireThat(row.data && Object.hasOwn(states, row.data.cohort), 'row cohort is not declared in header');
    acceptRow(states[row.data.cohort], row, header.data.params, boundary, header.data.founders);
  }
  return states;
}

export function validatePrefix(header, rows, { tick } = {}) {
  const states = readPrefix(header, rows, tick);
  return Object.fromEntries(header.data.cohorts.map((cohort) => [cohort, wireCounts(states[cohort])]));
}

export function countsFor(header, rows) {
  return validatePrefix(header, rows);
}

function validateCompletion(header, rows, completion) {
  record(completion, 'complete');
  const data = completion.data;
  const v2 = header.data.schema_version === 2;
  object(data, v2 ? [...COMPLETION_KEYS, 'run_id', 'capture_end'] : COMPLETION_KEYS, 'completion');
  provenance(data.provenance);
  requireThat(data.schema_version === header.data.schema_version &&
    equal(data.provenance, header.data.provenance), 'completion provenance does not match header');
  const boundary = decimal(data.ticks, 'completion ticks');
  if (v2) {
    requireThat(data.run_id === header.data.run_id, 'completion run ID does not match header');
    requireThat(CAPTURE_ENDS.includes(data.capture_end), 'unknown capture end');
    requireThat(data.capture_end !== 'finished' || header.data.ticks === data.ticks,
      'finished capture requires matching planned ticks');
  } else {
    requireThat(header.data.ticks === data.ticks, 'completion ticks do not match header');
  }
  const counts = validatePrefix(header, rows, { tick: String(boundary) });
  const incomplete = v2 && INCOMPLETE_ENDS.includes(data.capture_end);
  requireThat(Array.isArray(data.cohorts) && data.cohorts.length === header.data.cohorts.length,
    'completion cohort list does not match header');
  data.cohorts.forEach((cohort, index) => {
    object(cohort, ['cohort', 'counts', 'history_complete', 'final_state_hash'], 'cohort completion');
    requireThat(cohort.cohort === header.data.cohorts[index], 'completion cohort order does not match header');
    object(cohort.counts, COUNT_KEYS, 'counts');
    for (const key of COUNT_KEYS) decimal(cohort.counts[key], key);
    requireThat(equal(cohort.counts, counts[cohort.cohort]), 'completion totals do not match records');
    requireThat(incomplete || header.data.params.species.capacity === 0 ||
      cohort.counts.next_sequence !== '0', 'completed classified cohort has no history');
    requireThat(cohort.history_complete === (!incomplete && cohort.counts.dropped_events === '0'),
      'invalid history completeness');
    requireThat((incomplete && cohort.final_state_hash === null) ||
      (typeof cohort.final_state_hash === 'string' && /^[0-9a-f]{16}$/.test(cohort.final_state_hash)),
    'invalid final state hash');
  });
}

function validate(archive) {
  requireThat(archive !== null && typeof archive === 'object', 'archive must be an object');
  validateHeader(archive.header);
  validateCompletion(archive.header, archive.rows, archive.completion);
  return archive;
}

/** The application supplies the WASM validator; it must reject, never fill absent fields. */
export async function validateArchive(archive, validateParams) {
  validate(archive);
  if (validateParams) await validateParams(archive.header.data.params);
  return archive;
}

export async function parseArchive(text, validateParams) {
  requireThat(typeof text === 'string' && text.length > 0, 'archive must be nonempty text');
  let header = null;
  let completion = null;
  const rows = [];
  let offset = 0;
  let lineNumber = 0;
  while (offset < text.length) {
    const newline = text.indexOf('\n', offset);
    requireThat(newline !== -1, 'truncated history record (missing newline)');
    let row;
    try {
      row = parseLine(text.slice(offset, newline + 1));
      requireThat(completion === null, 'record after history completion');
      object(row, ['kind', 'data'], 'record');
      if (header === null) {
        validateHeader(row);
        header = row;
      } else if (row.kind === 'complete') completion = row;
      else {
        requireThat(row.kind === 'event' || row.kind === 'gap', 'duplicate header or unknown record kind');
        rows.push(row);
      }
    } catch (error) {
      throw new Error(`History line ${lineNumber + 1}: ${error.message}`, { cause: error });
    }
    lineNumber++;
    offset = newline + 1;
  }
  requireThat(completion !== null, 'history archive has no completion marker');
  return validateArchive({ header, rows, completion }, validateParams);
}

export function createHeader({
  runId, seed, founders, params, brainInheritance = 'evolving', simVersion, sourceRevision,
}) {
  requireThat(['evolving', 'randomized_at_birth'].includes(brainInheritance), 'unsupported brain inheritance');
  const header = {
    kind: 'header',
    data: {
      schema_version: 2, run_id: runId,
      provenance: {
        sim_version: simVersion, source_revision: sourceRevision, phase: 2,
        seed, control: 'randomized_at_birth_v3',
      },
      cohorts: [brainInheritance === 'randomized_at_birth' ? 'random_control' : 'evolving'],
      ticks: null, founders, drain_every: null, capacity_per_cohort: 4096,
      params: structuredClone(params),
    },
  };
  return validateHeader(header);
}

export function completionFor(header, rows, { tick, captureEnd, stateHash = null }) {
  const counts = validatePrefix(header, rows, { tick });
  const v2 = header.data.schema_version === 2;
  const incomplete = v2 && INCOMPLETE_ENDS.includes(captureEnd);
  const completion = {
    kind: 'complete',
    data: {
      schema_version: header.data.schema_version,
      ...(v2 ? { run_id: header.data.run_id } : {}),
      provenance: structuredClone(header.data.provenance),
      ticks: tick,
      ...(v2 ? { capture_end: captureEnd } : {}),
      cohorts: header.data.cohorts.map((cohort) => ({
        cohort, counts: counts[cohort],
        history_complete: !incomplete && counts[cohort].dropped_events === '0',
        final_state_hash: typeof stateHash === 'string' ? stateHash : stateHash?.[cohort] ?? null,
      })),
    },
  };
  validateCompletion(header, rows, completion);
  return completion;
}

export function encodeArchive(archive) {
  validate(archive);
  return [archive.header, ...archive.rows, archive.completion].map(encodeLine).join('');
}

/** Reserve an upper bound, so a live prefix always has room for a reimportable footer. */
export function footerAllowance(header) {
  validateHeader(header);
  requireThat(header.data.schema_version === 2, 'live capture requires schema v2');
  return byteLength(encodeLine({
    kind: 'complete',
    data: {
      schema_version: 2, run_id: header.data.run_id, provenance: header.data.provenance,
      ticks: String(U64_MAX), capture_end: 'params_changed',
      cohorts: header.data.cohorts.map((cohort) => ({
        cohort,
        counts: Object.fromEntries(COUNT_KEYS.map((key) => [key, String(U64_MAX)])),
        history_complete: false, final_state_hash: 'ffffffffffffffff',
      })),
    },
  }));
}
