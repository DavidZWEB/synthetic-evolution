/**
 * Main-thread handle on the simulation worker.
 *
 * Turns the worker's message protocol into calls and callbacks, and owns the reader half
 * of the transport. Nothing here knows how a frame is carried — that is `transport.js`'s
 * job, decided once at startup.
 */

import { createReader } from './transport.js';

export function createSim({ seed, founders, capacity, params = null }) {
  const worker = new Worker(new URL('./worker.js', import.meta.url), { type: 'module' });

  let reader = null;
  const listeners = { ready: [], inspection: [], error: [], params: [], hash: [] };
  const emit = (kind, payload) => listeners[kind]?.forEach((fn) => fn(payload));

  worker.onmessage = (event) => {
    const message = event.data;

    if (message.kind === 'ready') {
      reader = createReader(message.transport);
      emit('ready', { transport: reader.kind, capacity: reader.capacity });
      return;
    }

    // A frame arriving by transfer. Hand back whatever we were holding so the worker's
    // pool never drains and the sim never waits on the renderer.
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
    emit('error', { context: 'worker', message: event.message ?? String(event) });
  };
  worker.onmessageerror = () => {
    emit('error', { context: 'worker', message: 'a message could not be deserialised' });
  };

  worker.postMessage({ kind: 'create', seed, params, founders, capacity });

  const send = (kind, payload = {}) => worker.postMessage({ kind, ...payload });

  return {
    /** The most recent frame, or null before the first has arrived. */
    latest: () => reader?.latest() ?? null,
    get transport() {
      return reader?.kind ?? null;
    },

    play: () => send('play'),
    pause: () => send('pause'),
    setSpeed: (value) => send('setSpeed', { value }),
    stepOnce: (ticks = 1) => send('stepOnce', { ticks }),
    setParams: (json) => send('setParams', { params: json }),
    pushCommand: (json) => send('pushCommand', { command: json }),
    inspect: (index) => send('inspect', { index }),
    requestHash: () => send('hash'),

    on(kind, fn) {
      listeners[kind]?.push(fn);
      return () => {
        const at = listeners[kind].indexOf(fn);
        if (at >= 0) listeners[kind].splice(at, 1);
      };
    },

    destroy: () => worker.terminate(),
  };
}
