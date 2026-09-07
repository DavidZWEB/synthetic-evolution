/**
 * Selection identity and live-inspection polling.
 *
 * A selection is `(slot, incarnation)`, never a slot alone. Request ids additionally
 * prevent a delayed response from an earlier click from replacing the current panel.
 */

import { decodeInspection } from './model.ts';

export function createInspectorController({
  getSim,
  getRenderer,
  getFrame,
  onChange,
  now = () => performance.now(),
  intervalMs = 250,
}) {
  let selected = null;
  let inspection = null;
  let message = null;
  let requestId = 0;
  let lastRequestAt = Number.NEGATIVE_INFINITY;
  let inFlight = false;
  let refreshNeeded = false;

  const notify = () =>
    onChange({
      selectedIndex: selected?.index ?? null,
      inspection,
      message,
    });

  function request(timestamp = now()) {
    const sim = getSim();
    if (!selected || !sim || inFlight) return;
    requestId += 1;
    lastRequestAt = timestamp;
    inFlight = true;
    refreshNeeded = false;
    sim.inspect(selected.index, selected.incarnation, requestId);
  }

  function select(next) {
    requestId += 1;
    inFlight = false;
    refreshNeeded = false;
    selected = next;
    inspection = null;
    message = null;
    getRenderer()?.select(selected);
    notify();
    request();
  }

  return {
    select,

    pickAt(cssX, cssY) {
      const frame = getFrame();
      select(getRenderer()?.pick(frame?.views ?? null, cssX, cssY) ?? null);
    },

    accept(response) {
      if (
        !selected ||
        !inFlight ||
        response.index !== selected.index ||
        response.incarnation !== selected.incarnation ||
        response.requestId !== requestId
      ) {
        return false;
      }
      inFlight = false;
      if (!response.agent) {
        inspection = null;
        message = response.message ?? 'agent is no longer alive';
        getRenderer()?.select(null);
        notify();
        return true;
      }
      try {
        inspection = decodeInspection(response.agent);
        message = null;
      } catch (error) {
        inspection = null;
        message = String(error);
      }
      notify();
      return true;
    },

    poll(frame, timestamp) {
      if (!selected || message) return;
      // A paused step may be fresh for just one animation frame inside the cooldown.
      // Keep its demand, and let a slow same-selection reply finish before polling again.
      if (frame?.fresh) refreshNeeded = true;
      if (refreshNeeded && !inFlight && timestamp - lastRequestAt >= intervalMs) {
        request(timestamp);
      }
    },
  };
}
