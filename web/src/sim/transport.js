/**
 * `SnapshotTransport`: how a frame gets from the worker to the renderer.
 *
 * Two implementations, chosen once at startup from `crossOriginIsolated` and never
 * branched on again (spec §7.7):
 *
 * - **shared** — a `SharedArrayBuffer` both threads hold. The worker publishes into one
 *   of three state-tracked frames and the renderer leases the newest complete one while
 *   drawing. Needs cross-origin isolation.
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
 * that the renderer never blocks on a message; a frame drops instead of stalling the
 * sim if every shared slot is briefly leased or in flight.
 */

import {
  FRAME_STATE,
  HEADER,
  HEADER_BYTES,
  SHARED_FRAME_COUNT,
  copyFrame,
  frameHeader,
  frameLayout,
  frameViews,
} from './snapshot-layout.js';

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
  const buffer = new SharedArrayBuffer(HEADER_BYTES + layout.stride * SHARED_FRAME_COUNT);
  const header = new Int32Array(buffer, 0, HEADER.LENGTH);
  const frames = Array.from({ length: SHARED_FRAME_COUNT }, (_, frame) =>
    frameViews(buffer, HEADER_BYTES + layout.stride * frame, layout),
  );

  const stateAt = (frame) => frameHeader(frame, HEADER.STATE);
  const generationAt = (frame) => frameHeader(frame, HEADER.FRAME_GENERATION);

  function claimFreeFrame() {
    for (let frame = 0; frame < SHARED_FRAME_COUNT; frame += 1) {
      if (
        Atomics.compareExchange(
          header,
          stateAt(frame),
          FRAME_STATE.FREE,
          FRAME_STATE.WRITING,
        ) === FRAME_STATE.FREE
      ) {
        return frame;
      }
    }
    return -1;
  }

  return {
    kind: SHARED,
    handoff: { kind: SHARED, buffer, capacity: layout.capacity, plantCapacity: layout.plantCapacity },
    transfer: [],

    publish(source, tick, population) {
      const next = claimFreeFrame();
      // A reader briefly holds two frames while swapping leases. Dropping this snapshot
      // is safer than blocking the simulation; the next publish will supersede it.
      if (next < 0) return null;

      try {
        copyFrame(source, frames[next]);
      } catch (error) {
        Atomics.store(header, stateAt(next), FRAME_STATE.FREE);
        throw error;
      }

      const generation = (Atomics.add(header, HEADER.GENERATION, 1) + 1) >>> 0;
      Atomics.store(
        header,
        frameHeader(next, HEADER.TICK_LO),
        Number(tick & 0xffffffffn) | 0,
      );
      Atomics.store(
        header,
        frameHeader(next, HEADER.TICK_HI),
        Number((tick >> 32n) & 0xffffffffn) | 0,
      );
      Atomics.store(header, frameHeader(next, HEADER.POPULATION), population);
      Atomics.store(header, generationAt(next), generation | 0);

      // This release-store publishes payload and metadata together. A reader owns the
      // frame until it returns the state to FREE, so the worker cannot overwrite arrays
      // while WebGL is uploading them.
      Atomics.store(header, stateAt(next), FRAME_STATE.PUBLISHED);

      // Once `next` is visible, any older frame not leased by the reader is redundant.
      for (let frame = 0; frame < SHARED_FRAME_COUNT; frame += 1) {
        if (frame !== next) {
          Atomics.compareExchange(
            header,
            stateAt(frame),
            FRAME_STATE.PUBLISHED,
            FRAME_STATE.FREE,
          );
        }
      }
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
  const frames = Array.from({ length: SHARED_FRAME_COUNT }, (_, frame) =>
    frameViews(handoff.buffer, HEADER_BYTES + layout.stride * frame, layout),
  );
  const stateAt = (frame) => frameHeader(frame, HEADER.STATE);
  let held = -1;
  let heldGeneration = null;

  // Generation is a wrapping u32. Published frames are only a few generations apart,
  // so the half-range rule gives a stable ordering even across wraparound.
  const newerThan = (candidate, reference) => {
    if (reference === null) return true;
    const distance = (candidate - reference) >>> 0;
    return distance !== 0 && distance < 0x80000000;
  };

  function newestPublished() {
    let newest = -1;
    let generation = heldGeneration;
    for (let frame = 0; frame < SHARED_FRAME_COUNT; frame += 1) {
      if (Atomics.load(header, stateAt(frame)) !== FRAME_STATE.PUBLISHED) continue;
      const candidate = Atomics.load(
        header,
        frameHeader(frame, HEADER.FRAME_GENERATION),
      ) >>> 0;
      if (newerThan(candidate, generation)) {
        newest = frame;
        generation = candidate;
      }
    }
    return { frame: newest, generation };
  }

  function heldSnapshot(fresh) {
    if (held < 0) return null;
    return {
      views: frames[held],
      tick:
        (BigInt(Atomics.load(header, frameHeader(held, HEADER.TICK_HI)) >>> 0) << 32n) |
        BigInt(Atomics.load(header, frameHeader(held, HEADER.TICK_LO)) >>> 0),
      population: Atomics.load(header, frameHeader(held, HEADER.POPULATION)),
      fresh,
    };
  }

  return {
    kind: SHARED,
    capacity: layout.capacity,

    /** The most recently published frame. Never null once one has been published. */
    latest() {
      // The writer may reclaim a published frame between discovery and acquisition.
      // Retry against the new state rather than returning an older frame as fresh.
      for (let attempt = 0; attempt < SHARED_FRAME_COUNT; attempt += 1) {
        const newest = newestPublished();
        if (newest.frame < 0) return heldSnapshot(false);
        if (
          Atomics.compareExchange(
            header,
            stateAt(newest.frame),
            FRAME_STATE.PUBLISHED,
            FRAME_STATE.READING,
          ) !== FRAME_STATE.PUBLISHED
        ) {
          continue;
        }

        const previous = held;
        held = newest.frame;
        heldGeneration = newest.generation;
        if (previous >= 0) Atomics.store(header, stateAt(previous), FRAME_STATE.FREE);
        return heldSnapshot(true);
      }
      return heldSnapshot(false);
    },

    release() {
      if (held >= 0) Atomics.store(header, stateAt(held), FRAME_STATE.FREE);
      held = -1;
      heldGeneration = null;
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

    release() {
      const returning = held?.buffer ?? null;
      held = null;
      fresh = false;
      return returning;
    },
  };
}
