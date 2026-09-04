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
import { founderCount, parseSeed } from './inputs.js';
import { createTickScheduler } from './scheduler.js';
import { createWriter, preferredKind } from './transport.js';
import { bytesPerAgent, bytesPerPlant } from './snapshot-layout.js';

let sim = null;
let memory = null;
let writer = null;

/** Cached views over the snapshot inside WASM memory, and the buffer they belong to. */
let source = null;
let sourceBuffer = null;
let sourceLayout = null;

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

function postError(context, error, fatal = false) {
  postMessage({ kind: 'error', context, message: String(error), fatal });
}

const scheduler = createTickScheduler({
  step: (ticks) => sim.step_many(ticks),
  publish,
  onError: (error) => postError('tick', error, true),
  onRunningChange: (running) => postMessage({ kind: 'status', running }),
});

const handlers = {
  async create({ seed, params, founders }) {
    const wasm = await init();
    memory = wasm.memory;

    const nextSim = new Sim(parseSeed(seed), params ?? null);
    let hints;
    try {
      hints = JSON.parse(nextSim.render_hints());
      nextSim.seed_founders(founderCount(founders, hints.agent_capacity));
    } catch (error) {
      nextSim.free();
      throw error;
    }
    sim = nextSim;

    // Sizes and the world's extent come from the world, not from the caller. A client
    // holding its own copy of `world.size` draws a correct picture of the wrong world
    // the moment either moves.
    const kind = preferredKind();
    writer = createWriter(kind, hints.agent_capacity, hints.plant_capacity);
    source = null;
    sourceLayout = null;
    sourceBuffer = null;
    scheduler.setSecondsPerTick(hints.seconds_per_tick);

    postMessage({
      kind: 'ready',
      transport: writer.handoff,
      isolated: kind === 'shared',
      hints,
    });
    publish();
  },

  play() {
    scheduler.start();
  },

  pause() {
    scheduler.stop();
  },

  setSpeed({ value }) {
    scheduler.setSpeed(value);
  },

  /** Steps a fixed number of ticks while paused, for frame-by-frame inspection. */
  stepOnce({ ticks }) {
    if (!Number.isSafeInteger(ticks) || ticks < 1 || ticks > 10_000) {
      throw new RangeError('step count must be an integer between 1 and 10000');
    }
    sim.step_many(ticks);
    publish();
  },

  setParams({ params }) {
    try {
      sim.set_params(params);
      const hints = JSON.parse(sim.render_hints());
      scheduler.setSecondsPerTick(hints.seconds_per_tick);
      postMessage({ kind: 'params', params: sim.params_json(), hints });
    } catch (error) {
      postError('set_params', error);
    }
  },

  pushCommand({ command }) {
    try {
      sim.push_command(command);
    } catch (error) {
      postError('push_command', error);
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
  if (!handler) {
    postError('message', `unknown worker message kind: ${String(kind)}`);
    return;
  }

  try {
    if (kind === 'create') {
      created = handler(event.data);
      await created;
      return;
    }
    if (created) await created;
    await handler(event.data);
  } catch (error) {
    postError(kind, error, kind === 'create');
  }
};
