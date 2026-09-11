/**
 * Acknowledged worker history delivery, independent of snapshot publication.
 * Only one drained batch can be outside the bounded WASM recorder at a time.
 */
export function createHistoryDelivery({ sim, cohort, send }) {
  let active = true;
  let inFlight = null;
  let pending = null;
  let serial = 0;

  function fail(message) {
    active = false;
    sim.disable_history();
    const requests = [inFlight?.requestId, pending?.requestId].filter((id) => id != null);
    inFlight = null;
    pending = null;
    send({ kind: 'historyError', message: String(message), requestIds: requests });
  }

  function pump(force = false) {
    if (!active || inFlight) return;
    try {
      if (!force && !pending && !sim.history_pending()) return;
      const boundary = pending;
      let stateHash = null;
      if (boundary) {
        stateHash = sim.state_hash().toString(16).padStart(16, '0');
        // Retuning may fail without changing the world. In that case keep capture
        // active and let the command's caller report its validation error.
        if (boundary.apply && !boundary.apply()) {
          pending = null;
          pump();
          return;
        }
      }
      const batch = JSON.parse(sim.drain_history());
      const exhausted = batch.sequence_exhausted;
      const captureEnd = exhausted ? 'capture_error' : boundary?.captureEnd;
      const closing = captureEnd && captureEnd !== 'snapshot';
      if (closing) {
        active = false;
        sim.disable_history();
      }
      const message = {
        kind: 'historyBatch',
        batchId: ++serial,
        rows: batch.records.map((row) => ({
          kind: row.kind,
          data: { cohort, ...row.data },
        })),
        tick: batch.through_tick,
        nextSequence: batch.next_sequence,
        droppedEvents: batch.dropped_events,
        captureEnd,
        stateHash,
        requestId: boundary?.requestId,
      };
      pending = null;
      inFlight = message;
      send(message);
    } catch (error) {
      fail(error);
    }
  }

  return {
    get active() { return active; },
    pump,
    acknowledge(batchId, error) {
      if (inFlight?.batchId !== batchId) {
        fail('history acknowledgement does not match the in-flight batch');
        return;
      }
      if (error) {
        fail(error);
        return;
      }
      inFlight = null;
      pump();
    },
    boundary(captureEnd, requestId, apply) {
      if (!active || pending) {
        send({
          kind: 'historyError',
          message: active ? 'another history boundary is pending' : 'history capture is stopped',
          requestIds: requestId == null ? [] : [requestId],
          requestOnly: true,
        });
        return false;
      }
      pending = { captureEnd, requestId, apply };
      pump();
      return true;
    },
    stop: fail,
  };
}
