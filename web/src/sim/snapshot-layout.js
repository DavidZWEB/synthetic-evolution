/**
 * Byte layout of one snapshot frame — the contract between the worker that writes a
 * frame and the renderer that reads it.
 *
 * One module because both threads have to agree exactly, and two copies of an offset
 * table is how a renderer ends up drawing orientation as position with no error anywhere.
 *
 * Field order is not cosmetic: every four-byte array comes first so each typed-array
 * view lands naturally aligned, and the one-byte `alive` array goes last. A `Float32Array`
 * over a non-multiple-of-4 offset throws, and putting `alive` in the middle would make
 * every array after it depend on the capacity being even.
 *
 * The totals here must match `sim_core::snapshot::BYTES_PER_AGENT` (57). The Rust side
 * pins that number with a test; this side derives it, and `bytesPerAgent` below is what
 * a test can compare.
 */

/** Ints at the head of a shared frame buffer, before either of the two frames. */
export const HEADER = {
  /** Which of the two frames holds the most recently published one: 0 or 1. */
  ACTIVE: 0,
  /** Tick, split because `Atomics` works on 32-bit lanes and a tick is 64 bits. */
  TICK_LO: 1,
  TICK_HI: 2,
  POPULATION: 3,
  CAPACITY: 4,
  /** Bumped on every publish, so a reader can tell a new frame from a repeated one. */
  GENERATION: 5,
  LENGTH: 8,
};

export const HEADER_BYTES = HEADER.LENGTH * 4;

/**
 * Offsets and lengths of every array within one frame, for `capacity` slots.
 *
 * Offsets are relative to the start of the frame, so the same table serves a frame at
 * any base — which is what makes double-buffering a change of base rather than a second
 * layout.
 */
export function frameLayout(capacity) {
  let offset = 0;
  const wide = (count) => {
    const field = { offset, length: count };
    offset += count * 4;
    return field;
  };

  const layout = {
    position: wide(capacity * 3),
    orientation: wide(capacity * 4),
    size: wide(capacity),
    signature: wide(capacity * 3),
    species: wide(capacity),
    partOffset: wide(capacity),
    partCount: wide(capacity),
    alive: { offset, length: capacity },
  };
  offset += capacity;

  layout.bytes = offset;
  layout.capacity = capacity;
  return layout;
}

/** What one agent costs across a frame. Should be 57 (spec §7.5). */
export function bytesPerAgent() {
  return frameLayout(1).bytes;
}

/**
 * Typed-array views over one frame.
 *
 * Rebuild these whenever the underlying buffer changes identity — for WASM memory that
 * is any time it grows, which detaches every existing view silently (spec §7.3).
 */
export function frameViews(buffer, base, layout) {
  const f32 = (field) => new Float32Array(buffer, base + field.offset, field.length);
  const u32 = (field) => new Uint32Array(buffer, base + field.offset, field.length);
  return {
    position: f32(layout.position),
    orientation: f32(layout.orientation),
    size: f32(layout.size),
    signature: f32(layout.signature),
    species: u32(layout.species),
    partOffset: u32(layout.partOffset),
    partCount: u32(layout.partCount),
    alive: new Uint8Array(buffer, base + layout.alive.offset, layout.alive.length),
  };
}

/** Field names in the order a frame stores them. */
export const FIELDS = [
  'position',
  'orientation',
  'size',
  'signature',
  'species',
  'partOffset',
  'partCount',
  'alive',
];

/** Copies every array from one set of views into another of the same layout. */
export function copyFrame(from, into) {
  for (const field of FIELDS) into[field].set(from[field]);
}
