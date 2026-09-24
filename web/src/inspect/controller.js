/**
 * Selection identity and live-inspection polling.
 *
 * A selection is `(slot, incarnation)`, never a slot alone. Request ids additionally
 * prevent a delayed response from an earlier click from replacing the current panel.
 */

import { createRequestGate } from '../sim/request-gate.js';
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
  const gate = createRequestGate({ intervalMs });

  const notify = () =>
    onChange({
      selectedIndex: selected?.index ?? null,
      inspection,
      message,
    });

  function request(timestamp = now()) {
    const sim = getSim();
    if (!selected || !sim) return;
    const requestId = gate.begin(timestamp);
    if (requestId !== null) sim.inspect(selected.index, selected.incarnation, requestId);
  }

  function select(next) {
    gate.invalidate();
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
        response.index !== selected.index ||
        response.incarnation !== selected.incarnation ||
        !gate.settle(response.requestId)
      ) {
        return false;
      }
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
      if (frame?.fresh) gate.demand();
      if (gate.due(timestamp)) request(timestamp);
    },
  };
}
