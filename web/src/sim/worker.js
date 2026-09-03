/**
 * The simulation worker: owns the `Sim`, steps it on a clock, and publishes frames.
 *
 * It lives off the main thread so that a slow tick costs frames rather than making the
 * page unresponsive, and so the renderer's rate and the sim's rate are independent —
 * which is the whole point of the snapshot being a buffer rather than a callback
 * (spec §2.1).
 *
 * **Views over WASM memory are rebuilt whenever the buffer changes identity.** Growing
 * WASM memory detaches every existing `TypedArray` silently, and while stepping never
 * grows it, `push_command` and `inspect_agent` allocate and might. Comparing the buffer
 * is a pointer check per frame and removes the whole class (spec §7.3).
 *
 * Deliberately not here: anything that decides what the simulation does. The worker
 * chooses *when* to step and how far, never what a step means.
 */

import init, { Sim } from '../wasm/wasm.js';
import { createWriter, preferredKind } from './transport.js';
import { bytesPerAgent, bytesPerPlant } from './snapshot-layout.js';

/** Sim seconds per real second at speed 1, matching `world.dt` of 1/60 (spec §2.1). */
const TICKS_PER_SECOND = 60;
const FRAME_MS = 1000 / 60;

let sim = null;
let memory = null;
let writer = null;

/** Cached views over the snapshot inside WASM memory, and the buffer they belong to. */
let source = null;
let sourceBuffer = null;
let sourceLayout = null;

let running = false;
let speed = 1;
let timer = null;
let lastFrameAt = 0;

/**
 * Checks that the layout this side derives matches the one Rust actually wrote.
 *
 * Two modules describe the same bytes — `snapshot-layout.js` and
 * `sim_core::snapshot` — and if they ever disagree the renderer draws one field as
 * another with no error anywhere. Rust's spans carry the truth, so compare against them
 * once rather than trusting two copies of an arithmetic to stay equal.
 */
function assertLayoutsAgree(spans) {
  const capacity = spans.alive.len;
  const agentFields = ['position', 'orientation', 'size', 'signature', 'species', 'part_offset', 'part_count'];
  const agentBytes =
    agentFields.reduce((total, field) => total + spans[field].len * 4, 0) + spans.alive.len;
  const plantBytes = (spans.plant_position.len + spans.plant_energy.len) * 4;

  const expectedAgents = capacity * bytesPerAgent();
  const expectedPlants = spans.plant_capacity * bytesPerPlant();
  if (agentBytes !== expectedAgents || plantBytes !== expectedPlants) {
    throw new Error(
      `snapshot layout disagrees: wasm says ${agentBytes}b for ${capacity} agents and ` +
        `${plantBytes}b for ${spans.plant_capacity} plants; this side expects ` +
        `${expectedAgents} and ${expectedPlants}`,
    );
  }
}

/** Rebuilds the views onto WASM memory if it has moved under us. */
function sourceViews() {
  if (source && sourceBuffer === memory.buffer) return source;

  // Offsets survive a grow — WASM memory keeps its contents and addresses — so only the
  // views need rebuilding, not the layout.
  if (!sourceLayout) {
    sourceLayout = JSON.parse(sim.snapshot_layout());
    assertLayoutsAgree(sourceLayout);
  }
  const spans = sourceLayout;
  const at = (span, Kind) => new Kind(memory.buffer, span.ptr, span.len);
  source = {
    position: at(spans.position, Float32Array),
    orientation: at(spans.orientation, Float32Array),
    size: at(spans.size, Float32Array),
    signature: at(spans.signature, Float32Array),
    species: at(spans.species, Uint32Array),
    partOffset: at(spans.part_offset, Uint32Array),
    partCount: at(spans.part_count, Uint32Array),
    plantPosition: at(spans.plant_position, Float32Array),
    plantEnergy: at(spans.plant_energy, Float32Array),
    alive: at(spans.alive, Uint8Array),
  };
  sourceBuffer = memory.buffer;
  return source;
}

function publish() {
  const message = writer.publish(sourceViews(), sim.tick(), sim.population());
  if (message) postMessage(message, writer.transfer);
}

function frame() {
  timer = null;
  if (!running) return;

  try {
    advance();
  } catch (error) {
    // A throw here used to stop the clock and nothing else: the loop reschedules at the
    // end, so one exception froze the sim at whatever tick it reached while the renderer
    // went on drawing that frame at full rate. Silent, and indistinguishable from a
    // paused world.
    stop();
    postMessage({ kind: 'error', context: 'tick', message: String(error) });
    return;
  }

  timer = setTimeout(frame, FRAME_MS);
}

function advance() {
  const now = performance.now();
  // Ticks owed since the last frame, so a slow frame catches up rather than silently
  // running the world slower than the speed says.
  const elapsed = Math.min(now - lastFrameAt, 250);
  lastFrameAt = now;
  const ticks = Math.max(1, Math.round((elapsed / 1000) * TICKS_PER_SECOND * speed));

  sim.step_many(ticks);
  publish();
}

function start() {
  if (running) return;
  running = true;
  lastFrameAt = performance.now();
  timer = setTimeout(frame, 0);
}

function stop() {
  running = false;
  if (timer !== null) clearTimeout(timer);
  timer = null;
}

const handlers = {
  async create({ seed, params, founders }) {
    const wasm = await init();
    memory = wasm.memory;

    sim = new Sim(BigInt(seed), params ?? null);
    sim.seed_founders(founders);

    // Sizes and the world's extent come from the world, not from the caller. A client
    // holding its own copy of `world.size` draws a correct picture of the wrong world
    // the moment either moves.
    const hints = JSON.parse(sim.render_hints());
    const kind = preferredKind();
    writer = createWriter(kind, hints.agent_capacity, hints.plant_capacity);
    source = null;
    sourceLayout = null;

    postMessage({
      kind: 'ready',
      transport: writer.handoff,
      isolated: kind === 'shared',
      hints,
    });
    publish();
  },

  play() {
    start();
  },

  pause() {
    stop();
  },

  setSpeed({ value }) {
    speed = Math.max(0, value);
  },

  /** Steps a fixed number of ticks while paused, for frame-by-frame inspection. */
  stepOnce({ ticks }) {
    sim.step_many(Math.max(1, ticks | 0));
    publish();
  },

  setParams({ params }) {
    try {
      sim.set_params(params);
      postMessage({ kind: 'params', params: sim.params_json() });
    } catch (error) {
      postMessage({ kind: 'error', context: 'set_params', message: String(error) });
    }
  },

  pushCommand({ command }) {
    try {
      sim.push_command(command);
    } catch (error) {
      postMessage({ kind: 'error', context: 'push_command', message: String(error) });
    }
  },

  inspect({ index }) {
    try {
      postMessage({ kind: 'inspection', agent: sim.inspect_agent(index) });
    } catch (error) {
      postMessage({ kind: 'inspection', agent: null, message: String(error) });
    }
  },

  /** A frame coming back from the renderer, for the transferable transport's pool. */
  recycle({ buffer }) {
    writer.recycle?.(buffer);
  },

  hash() {
    postMessage({ kind: 'hash', value: sim.state_hash().toString(16) });
  },
};

/**
 * Resolves once the world exists. Everything else waits on it.
 *
 * `onmessage` is async and `create` awaits `init()`, so without this the runtime is free
 * to dispatch the next message during that await — and a `play` arriving then would
 * start the clock against a null `sim`. That is not hypothetical: it is what made a
 * reseed followed quickly by play run exactly one tick and then stop.
 */
let created = null;

onmessage = async (event) => {
  const { kind } = event.data;
  const handler = handlers[kind];
  if (!handler) return;

  try {
    if (kind === 'create') {
      created = handler(event.data);
      await created;
      return;
    }
    if (created) await created;
    await handler(event.data);
  } catch (error) {
    postMessage({ kind: 'error', context: kind, message: String(error) });
  }
};
