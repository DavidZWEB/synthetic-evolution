/**
 * Main-thread handle on the simulation worker.
 *
 * Turns the worker's message protocol into calls and callbacks, and owns the reader half
 * of the transport. Nothing here knows how a frame is carried — that is `transport.js`'s
 * job, decided once at startup.
 */

import { createReader } from './transport.js';
import { EVOLVING } from './brain-inheritance.js';

export function createSim({
  seed, founders, params = null, brainInheritance = EVOLVING, historyRunId = null,
  historyRepresentatives = false,
}) {
  const worker = new Worker(new URL('./worker.js', import.meta.url), { type: 'module' });

  let reader = null;
  let destroyed = false;
  const listeners = {
    ready: [],
    inspection: [],
    species: [],
    metrics: [],
    error: [],
    params: [],
    hash: [],
    status: [],
    validatedRun: [],
    historyReady: [],
    historyBatch: [],
    historyError: [],
  };
  const emit = (kind, payload) => listeners[kind]?.forEach((fn) => fn(payload));

  worker.onmessage = (event) => {
    const message = event.data;

    if (message.kind === 'ready') {
      reader = createReader(message.transport);
      emit('ready', {
        transport: reader.kind,
        capacity: reader.capacity,
        hints: message.hints,
        run: message.run,
      });
      return;
    }

    // A frame arriving by transfer. `accept` only returns a superseded pending buffer;
    // a frame already handed to rendering stays attached until the next animation read.
    if (message.kind === 'transferable') {
      const returning = reader?.accept(message);
      if (returning) worker.postMessage({ kind: 'recycle', buffer: returning }, [returning]);
      return;
    }

    emit(message.kind, message);
  };

  // Without these a throw the worker did not catch is invisible from the page: the sim
  // simply stops and the renderer keeps drawing the last frame it saw.
  worker.onerror = (event) => {
    event.preventDefault();
    emit('error', { context: 'worker', message: event.message ?? String(event), fatal: true });
  };
  worker.onmessageerror = () => {
    emit('error', {
      context: 'worker',
      message: 'a message could not be deserialised',
      fatal: true,
    });
  };

  worker.postMessage({
    kind: 'create', seed, params, founders, brainInheritance, historyRunId, historyRepresentatives,
  });

  const send = (kind, payload = {}) => {
    if (!destroyed) worker.postMessage({ kind, ...payload });
  };

  return {
    /** The most recent frame, or null before the first has arrived. */
    latest() {
      const frame = reader?.latest() ?? null;
      const returning = reader?.takeRecycle?.();
      if (returning) worker.postMessage({ kind: 'recycle', buffer: returning }, [returning]);
      return frame;
    },
    get transport() {
      return reader?.kind ?? null;
    },

    play: () => send('play'),
    pause: () => send('pause'),
    setSpeed: (value) => send('setSpeed', { value }),
    stepOnce: (ticks = 1) => send('stepOnce', { ticks }),
    setParams: (json) => send('setParams', { params: json }),
    pushCommand: (json) => send('pushCommand', { command: json }),
    inspect: (index, incarnation, requestId) =>
      send('inspect', { index, incarnation, requestId }),
    requestHash: () => send('hash'),
    requestSpecies: (requestId) => send('species', { requestId }),
    acknowledgeHistory: (batchId, error) => send('historyAck', { batchId, error }),
    historyBoundary: (captureEnd, requestId) => send('historyBoundary', { captureEnd, requestId }),
    stopHistory: (message) => send('historyStop', { message }),
    validateRun: (seed, founders, params, brainInheritance, requestId) =>
      send('validateRun', { seed, founders, params, brainInheritance, requestId }),

    on(kind, fn) {
      listeners[kind]?.push(fn);
      return () => {
        const at = listeners[kind].indexOf(fn);
        if (at >= 0) listeners[kind].splice(at, 1);
      };
    },

    destroy() {
      if (destroyed) return;
      destroyed = true;
      reader?.release?.();
      reader = null;
      worker.terminate();
    },
  };
}
