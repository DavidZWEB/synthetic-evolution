/**
 * `SnapshotTransport`: how a frame gets from the worker to the renderer.
 *
 * Two implementations, chosen once at startup from `crossOriginIsolated` and never
 * branched on again (spec §7.7):
 *
 * - **shared** — a `SharedArrayBuffer` both threads hold. The worker writes the inactive
 *   frame and flips an atomic index; the renderer reads whichever is active, at its own
 *   rate, without waiting for a message. Needs cross-origin isolation.
 * - **transferable** — pooled `ArrayBuffer`s ping-ponged by `postMessage`. Ownership
 *   moves rather than copies, so the transfer itself is cheap, but the renderer only
 *   holds a frame between messages and the worker must wait for one to come back.
 *
 * **Both copy once, in the worker**, out of WASM memory and into the transport. Spec
 * §7.3's zero-copy read is a view straight onto WASM linear memory — which the main
 * thread can only do if that memory is itself shared, and that needs a threads-enabled
 * build (`--shared-memory`, atomics) that arrives with `wasm-bindgen-rayon` at Phase 7.
 * Until then the worker owns its memory alone, and one copy per frame is the floor
 * rather than a shortcut. At the web profile's few thousand agents it is a few hundred
 * KB — immaterial, and the same order as the fallback's copy the spec already accepts.
 *
 * What the shared transport still buys over the fallback is not the copy count: it is
 * that the renderer never blocks on a message and never runs out of buffers, so a slow
 * frame drops instead of stalling the sim.
 */

import { HEADER, HEADER_BYTES, frameLayout, frameViews, copyFrame } from './snapshot-layout.js';

export const SHARED = 'shared';
export const TRANSFERABLE = 'transferable';

/**
 * Which transport this context can use.
 *
 * Read through `globalThis` rather than as a bare identifier: on anything predating the
 * property a bare reference is a `ReferenceError`, which would blank the page instead of
 * quietly select the fallback it is meant to select.
 */
export function preferredKind() {
  return globalThis.crossOriginIsolated === true ? SHARED : TRANSFERABLE;
}

/* ------------------------------------------------------------------ worker side --- */

/**
 * Creates the writer half. `handoff` is what the worker posts to the main thread so it
 * can build the matching reader.
 */
export function createWriter(kind, capacity, plantCapacity) {
  const layout = frameLayout(capacity, plantCapacity);
  return kind === SHARED ? sharedWriter(layout) : transferableWriter(layout);
}

function sharedWriter(layout) {
  // Two frames after the header, so a reader is never looking at the one being written.
  const buffer = new SharedArrayBuffer(HEADER_BYTES + layout.bytes * 2);
  const header = new Int32Array(buffer, 0, HEADER.LENGTH);
  const frames = [
    frameViews(buffer, HEADER_BYTES, layout),
    frameViews(buffer, HEADER_BYTES + layout.bytes, layout),
  ];
  Atomics.store(header, HEADER.ACTIVE, 0);

  return {
    kind: SHARED,
    handoff: { kind: SHARED, buffer, capacity: layout.capacity, plantCapacity: layout.plantCapacity },
    transfer: [],

    publish(source, tick, population) {
      // Into the frame nobody is reading, then flip. The reader only ever follows
      // ACTIVE, so it cannot observe a half-written frame.
      const next = 1 - Atomics.load(header, HEADER.ACTIVE);
      copyFrame(source, frames[next]);
      Atomics.store(header, HEADER.TICK_LO, Number(tick & 0xffffffffn) | 0);
      Atomics.store(header, HEADER.TICK_HI, Number(tick >> 32n) | 0);
      Atomics.store(header, HEADER.POPULATION, population);
      Atomics.add(header, HEADER.GENERATION, 1);
      Atomics.store(header, HEADER.ACTIVE, next);
      return null;
    },
  };
}

function transferableWriter(layout) {
  // Two buffers so the worker can fill one while the other is out on loan. A third
  // would only deepen the queue the renderer is already behind on.
  const pool = [new ArrayBuffer(layout.bytes), new ArrayBuffer(layout.bytes)];

  return {
    kind: TRANSFERABLE,
    handoff: { kind: TRANSFERABLE, capacity: layout.capacity, plantCapacity: layout.plantCapacity },
    transfer: [],

    /** Returns a message to post, or null when no buffer is free. */
    publish(source, tick, population) {
      const buffer = pool.pop();
      // Dropped rather than queued. The renderer already has a frame it has not drawn,
      // and holding this one would grow a backlog of states nobody will ever see while
      // the sim waits to hand it over.
      if (!buffer) return null;

      copyFrame(source, frameViews(buffer, 0, layout));
      this.transfer = [buffer];
      return { kind: TRANSFERABLE, buffer, tick: tick.toString(), population };
    },

    /** Takes a buffer back from the renderer. */
    recycle(buffer) {
      if (pool.length < 2) pool.push(buffer);
    },
  };
}

/* ------------------------------------------------------------------- reader side --- */

/** Builds the reader half from whatever `createWriter` handed off. */
export function createReader(handoff) {
  const layout = frameLayout(handoff.capacity, handoff.plantCapacity);
  return handoff.kind === SHARED ? sharedReader(handoff, layout) : transferableReader(layout);
}

function sharedReader(handoff, layout) {
  const header = new Int32Array(handoff.buffer, 0, HEADER.LENGTH);
  const frames = [
    frameViews(handoff.buffer, HEADER_BYTES, layout),
    frameViews(handoff.buffer, HEADER_BYTES + layout.bytes, layout),
  ];
  let lastGeneration = -1;

  return {
    kind: SHARED,
    capacity: layout.capacity,

    /** The most recently published frame. Never null once one has been published. */
    latest() {
      const generation = Atomics.load(header, HEADER.GENERATION);
      if (generation === 0) return null;
      const active = Atomics.load(header, HEADER.ACTIVE);
      const fresh = generation !== lastGeneration;
      lastGeneration = generation;
      return {
        views: frames[active],
        tick:
          (BigInt(Atomics.load(header, HEADER.TICK_HI) >>> 0) << 32n) |
          BigInt(Atomics.load(header, HEADER.TICK_LO) >>> 0),
        population: Atomics.load(header, HEADER.POPULATION),
        fresh,
      };
    },
  };
}

function transferableReader(layout) {
  let held = null;
  let fresh = false;

  return {
    kind: TRANSFERABLE,
    capacity: layout.capacity,

    latest() {
      if (!held) return null;
      const wasFresh = fresh;
      fresh = false;
      return { ...held, fresh: wasFresh };
    },

    /** Takes ownership of a frame that arrived by `postMessage`. */
    accept(message) {
      // Whatever we were holding goes back, so the worker's pool never drains to
      // nothing and the sim never blocks waiting for a buffer.
      const returning = held ? held.buffer : null;
      held = {
        buffer: message.buffer,
        views: frameViews(message.buffer, 0, layout),
        tick: BigInt(message.tick),
        population: message.population,
      };
      fresh = true;
      return returning;
    },
  };
}
