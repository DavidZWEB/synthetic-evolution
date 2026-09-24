/**
 * The on-demand polling contract shared by main-thread requests to the worker.
 *
 * At most one request is in flight, a reply is accepted only for the latest request id,
 * and fresh demand waits out a cooldown before the next request. What is requested and
 * how a reply is decoded belong to the caller; this owns only when a request may go.
 */

export function createRequestGate({ intervalMs }) {
  let requestId = 0;
  let inFlight = false;
  let refreshNeeded = false;
  let lastRequestAt = Number.NEGATIVE_INFINITY;

  return {
    /** Starts a request, returning its id, or `null` while another is in flight. */
    begin(timestamp) {
      if (inFlight) return null;
      requestId += 1;
      inFlight = true;
      refreshNeeded = false;
      lastRequestAt = timestamp;
      return requestId;
    },

    /** Settles the in-flight request if `id` is current; stale replies return false. */
    settle(id) {
      if (!inFlight || id !== requestId) return false;
      inFlight = false;
      return true;
    },

    /** Orphans any in-flight request so its reply is rejected as stale. */
    invalidate({ resetCooldown = false } = {}) {
      requestId += 1;
      inFlight = false;
      refreshNeeded = false;
      if (resetCooldown) lastRequestAt = Number.NEGATIVE_INFINITY;
    },

    /**
     * Records whether newer data is wanted. Demand arriving mid-flight is kept, so a
     * paused step that is fresh for a single animation frame is not lost.
     */
    demand(needed = true) {
      refreshNeeded = needed;
    },

    /** Whether a demanded refresh may be requested now. */
    due(timestamp) {
      return refreshNeeded && !inFlight && timestamp - lastRequestAt >= intervalMs;
    },
  };
}
