/**
 * On-demand diet sampling for the diet colour mode.
 *
 * While the mode is on, asks the worker for one diet byte per slot at most every
 * `intervalMs`, and only after a frame has moved on. A diet changes over many meals, so a
 * few samples a second keep the colours honest without adding a byte to every frame
 * (spec §2.2b). Each sample carries the slots' incarnations, so a newborn in a reused
 * slot never wears its predecessor's diet.
 */
import { createRequestGate } from '../sim/request-gate.js';

/** The diet byte of an agent that has eaten nothing, or of an empty slot. */
export const UNFED = 255;

export function createDietController({ getSim, onChange, intervalMs = 500 }) {
  let active = false;
  const gate = createRequestGate({ intervalMs });

  function request(timestamp) {
    const sim = getSim();
    if (!sim || !active) return;
    const requestId = gate.begin(timestamp);
    if (requestId !== null) sim.requestDiets(requestId);
  }

  return {
    /** Turns sampling on or off; turning it off drops any request in flight. */
    setActive(value, timestamp) {
      if (value === active) return;
      active = value;
      if (active) {
        gate.demand();
        request(timestamp);
      } else {
        gate.invalidate();
      }
    },

    /** Forgets everything about the previous world. */
    reset() {
      gate.invalidate({ resetCooldown: true });
      gate.demand(active);
      onChange(null);
    },

    /** Takes the worker's reply; a stale or malformed one changes nothing. */
    accept(response) {
      if (!gate.settle(response.requestId)) return false;
      const { shares, incarnation } = response;
      if (
        shares instanceof Uint8Array &&
        incarnation instanceof Uint32Array &&
        shares.length === incarnation.length
      ) {
        onChange({ shares, incarnation });
      }
      return true;
    },

    /** Called every animation frame with the latest frame. */
    poll(frame, timestamp) {
      if (!active) return;
      if (frame?.fresh) gate.demand();
      if (gate.due(timestamp)) request(timestamp);
    },
  };
}
