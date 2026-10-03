/**
 * Acknowledged worker history delivery, independent of snapshot publication.
 * Only one drained batch can be outside the bounded WASM recorder at a time.
 */
import { byteLength, MAX_LINE_BYTES } from './json-lines.js';

/** A genome that would push its origin past the line limit is archived as unavailable. */
function withinLineLimit(row) {
  if (row.data.representative?.status === 'recorded' &&
    byteLength(`${JSON.stringify(row)}\n`) > MAX_LINE_BYTES) {
    return {
      ...row,
      data: { ...row.data, representative: { status: 'unavailable', reason: 'line_limit' } },
    };
  }
  return row;
}

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
          pending = boundary.fallbackEnd
            ? { captureEnd: boundary.fallbackEnd, requestId: boundary.requestId } : null;
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
      // Taken after the drain and before any further step, so a saved run's
      // checkpoint and history describe the same boundary (spec §7.10).
      const checkpoint = boundary?.checkpoint ? sim.checkpoint() : undefined;
      const message = {
        kind: 'historyBatch',
        batchId: ++serial,
        rows: batch.records.map((row) => withinLineLimit({
          kind: row.kind,
          data: { cohort, ...row.data },
        })),
        tick: batch.through_tick,
        nextSequence: batch.next_sequence,
        droppedEvents: batch.dropped_events,
        captureEnd,
        stateHash,
        requestId: boundary?.requestId,
        ...(checkpoint ? { checkpoint } : {}),
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
    boundary(captureEnd, requestId, apply, checkpoint = false) {
      // One requested barrier can share a queued retune's prefix. If the retune
      // fails validation, that request still needs its own boundary. A save cannot:
      // the retune applies before the drain, so its checkpoint would carry new params.
      if (active && pending?.apply && pending.requestId == null && !apply && requestId != null &&
        !checkpoint) {
        pending.requestId = requestId;
        pending.fallbackEnd = captureEnd;
        return true;
      }
      if (!active || pending) {
        send({
          kind: 'historyError',
          message: active ? 'another history boundary is pending' : 'history capture is stopped',
          requestIds: requestId == null ? [] : [requestId],
          requestOnly: true,
        });
        return false;
      }
      pending = { captureEnd, requestId, apply, checkpoint };
      pump();
      return true;
    },
    stop: fail,
  };
}
