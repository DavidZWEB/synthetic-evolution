/**
 * Byte layout of one snapshot frame — the contract between the worker that writes a
 * frame and the renderer that reads it.
 *
 * One module because both threads have to agree exactly, and two copies of an offset
 * table is how a renderer ends up drawing orientation as position with no error anywhere.
 *
 * Field order is not cosmetic: every four-byte array comes first so each typed-array
 * view lands naturally aligned, and the one-byte arrays (`alive`, then the combat bytes)
 * go last. A `Float32Array` over a non-multiple-of-4 offset throws, and putting a byte
 * array in the middle would make every array after it depend on the capacity.
 *
 * The totals here must match `sim_core::snapshot::BYTES_PER_AGENT` (72). The Rust side
 * pins that number with a test; this side derives it, and `bytesPerAgent` below is what
 * a test can compare.
 */

/** Three frames let the writer keep publishing while the renderer leases one to draw. */
export const SHARED_FRAME_COUNT = 3;

/** Lifecycle of one frame in the shared transport. */
export const FRAME_STATE = {
  FREE: 0,
  WRITING: 1,
  PUBLISHED: 2,
  READING: 3,
};

/**
 * Ints at the head of a shared frame buffer.
 *
 * Metadata belongs to its frame rather than to the buffer as a whole. The writer stores
 * it before publishing that frame's state, so a reader can never pair one frame's arrays
 * with another frame's tick or population.
 */
export const HEADER = {
  GENERATION: 0,
  FRAMES: 1,
  FRAME_LENGTH: 5,
  STATE: 0,
  TICK_LO: 1,
  TICK_HI: 2,
  POPULATION: 3,
  FRAME_GENERATION: 4,
  LENGTH: 1 + SHARED_FRAME_COUNT * 5,
};

export const HEADER_BYTES = HEADER.LENGTH * 4;

/** Header lane for one field of one shared frame. */
export function frameHeader(frame, field) {
  return HEADER.FRAMES + frame * HEADER.FRAME_LENGTH + field;
}

/**
 * Offsets and lengths of every array within one frame, for `capacity` slots.
 *
 * Offsets are relative to the start of the frame, so the same table serves a frame at
 * any base — which is what makes adding transport frames a change of stride rather than
 * a second layout.
 */
export function frameLayout(capacity, plantCapacity = 0, corpseCapacity = 0) {
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
    incarnation: wide(capacity),
    biteAt: wide(capacity * 2),
    plantPosition: wide(plantCapacity * 3),
    plantEnergy: wide(plantCapacity),
    corpsePosition: wide(corpseCapacity * 3),
    corpseEnergy: wide(corpseCapacity),
  };
  const bytes = (count) => {
    const field = { offset, length: count };
    offset += count;
    return field;
  };
  layout.alive = bytes(capacity);
  layout.health = bytes(capacity);
  layout.swingAge = bytes(capacity);
  layout.hurtAge = bytes(capacity);

  layout.bytes = offset;
  // The logical payload ends in the byte arrays, but every frame base must still align
  // the four-byte fields in the next frame.
  layout.stride = Math.ceil(offset / 4) * 4;
  layout.capacity = capacity;
  layout.plantCapacity = plantCapacity;
  layout.corpseCapacity = corpseCapacity;
  return layout;
}

/** What one agent costs across a frame. Should be 72 (spec §7.5). */
export function bytesPerAgent() {
  return frameLayout(1, 0).bytes;
}

/** What one plant costs: a position and how much it holds. Should be 16. */
export function bytesPerPlant() {
  return frameLayout(0, 1).bytes;
}

/** What one corpse slot costs: a position and how much it holds. Should be 16. */
export function bytesPerCorpse() {
  return frameLayout(0, 0, 1).bytes;
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
  const u8 = (field) => new Uint8Array(buffer, base + field.offset, field.length);
  return {
    position: f32(layout.position),
    orientation: f32(layout.orientation),
    size: f32(layout.size),
    signature: f32(layout.signature),
    species: u32(layout.species),
    partOffset: u32(layout.partOffset),
    partCount: u32(layout.partCount),
    incarnation: u32(layout.incarnation),
    biteAt: f32(layout.biteAt),
    plantPosition: f32(layout.plantPosition),
    plantEnergy: f32(layout.plantEnergy),
    corpsePosition: f32(layout.corpsePosition),
    corpseEnergy: f32(layout.corpseEnergy),
    alive: u8(layout.alive),
    health: u8(layout.health),
    swingAge: u8(layout.swingAge),
    hurtAge: u8(layout.hurtAge),
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
  'incarnation',
  'biteAt',
  'plantPosition',
  'plantEnergy',
  'corpsePosition',
  'corpseEnergy',
  'alive',
  'health',
  'swingAge',
  'hurtAge',
];

/** Copies every array from one set of views into another of the same layout. */
export function copyFrame(from, into) {
  for (const field of FIELDS) into[field].set(from[field]);
}
