/**
 * Main-thread persistence for one captured World. Acknowledgements follow committed
 * storage, and export barriers describe that exact prefix rather than a later frame.
 */
import { completionFor, encodeArchive } from './archive.js';

const BOUNDARY_TIMEOUT_MS = 15_000;

class CaptureError extends Error {}

export function createHistorySession({
  sim, getStore, onChange, onSaved, timeoutMs = BOUNDARY_TIMEOUT_MS,
}) {
  let id = null;
  let header = null;
  let state = 'starting';
  let chain = Promise.resolve();
  let requestSerial = 0;
  let boundaryWork = null;
  let nextSequence = 0n;
  let droppedEvents = 0n;
  const requests = new Map();
  let readyResolve;
  const ready = new Promise((resolve) => { readyResolve = resolve; });

  function report(status, message = null) {
    state = status;
    onChange({ id, status, message });
  }

  function settle(requestId, error, result) {
    const request = requests.get(requestId);
    if (!request) return;
    clearTimeout(request.timer);
    requests.delete(requestId);
    if (error) request.reject(error);
    else request.resolve(result);
  }

  async function incomplete(reason, message) {
    report('incomplete', message);
    readyResolve();
    for (const requestId of requests.keys()) settle(requestId, new Error(message));
    if (id) {
      try {
        await (await getStore()).markIncomplete(id, reason);
        await onSaved();
      } catch (error) {
        report('incomplete', `${message}; could not mark saved prefix: ${String(error)}`);
      }
    }
  }

  function enqueue(task) {
    chain = chain.then(task).catch(async (error) => {
      sim.stopHistory(String(error));
      const reason = error instanceof CaptureError ? 'capture_error'
        : error.code === 'storage_limit' ? 'storage_limit' : 'storage_error';
      await incomplete(reason, String(error));
    });
  }

  const unsubscribe = [
    sim.on('historyReady', ({ header: nextHeader }) => {
      enqueue(async () => {
        if (state !== 'starting') return;
        header = nextHeader;
        id = await (await getStore()).create(header);
        if (state === 'starting') report('recording');
        else await (await getStore()).markIncomplete(id, 'unfinalized');
        readyResolve();
        await onSaved();
      });
    }),
    sim.on('historyBatch', (batch) => {
      enqueue(async () => {
        if (state !== 'recording') return;
        const store = await getStore();
        let next = nextSequence;
        let dropped = droppedEvents;
        for (const row of batch.rows) {
          if (row.kind === 'event') {
            if (BigInt(row.data.sequence) !== next) throw new CaptureError('History batch is discontinuous');
            next++;
          } else if (row.kind === 'gap') {
            if (BigInt(row.data.first_sequence) !== next) throw new CaptureError('History gap is discontinuous');
            const end = BigInt(row.data.last_sequence) + 1n;
            if (end <= next) throw new CaptureError('History gap is reversed');
            dropped += end - next;
            next = end;
          } else {
            throw new CaptureError('Unknown history batch record');
          }
        }
        if (String(next) !== batch.nextSequence || String(dropped) !== batch.droppedEvents) {
          throw new CaptureError('History batch counters do not match its complete drained prefix');
        }
        await store.append(id, batch.rows, { tick: batch.tick });
        if (state !== 'recording') return;
        nextSequence = next;
        droppedEvents = dropped;
        let exported = null;
        if (batch.captureEnd) {
          const archive = await store.get(id);
          if (state !== 'recording') return;
          const completion = completionFor(header, archive.rows, {
            tick: batch.tick, captureEnd: batch.captureEnd, stateHash: batch.stateHash,
          });
          if (batch.captureEnd === 'snapshot') {
            exported = encodeArchive({ header, rows: archive.rows, completion });
          } else {
            await store.finalize(id, completion);
            report(batch.captureEnd === 'capture_error' ? 'incomplete' : 'stopped',
              batch.captureEnd === 'capture_error' ? 'History sequence space exhausted' : batch.captureEnd);
          }
        }
        // The snapshot string is materialized before releasing the worker: a later
        // batch must not leak into an export carrying an earlier boundary/hash.
        sim.acknowledgeHistory(batch.batchId);
        settle(batch.requestId, null, exported);
        await onSaved();
      });
    }),
    sim.on('historyError', ({ message, requestIds = [], requestOnly = false }) => {
      if (requestOnly) {
        for (const requestId of requestIds) settle(requestId, new Error(message));
      } else {
        enqueue(async () => {
          if (state !== 'incomplete') await incomplete('capture_error', message);
        });
      }
    }),
  ];

  return {
    get id() { return id; },
    get active() { return state === 'starting' || state === 'recording'; },
    waitForBoundary() { return boundaryWork ?? Promise.resolve(); },
    boundary(captureEnd) {
      if (state !== 'starting' && state !== 'recording') {
        return Promise.reject(new Error('History capture is not active'));
      }
      if (requests.size) return Promise.reject(new Error('A history operation is already pending'));
      const requestId = ++requestSerial;
      const work = new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          const message = 'History storage did not respond; capture stopped, simulation continues';
          sim.stopHistory(message);
          // Do not wait for a blocked IndexedDB request to release the UI/world.
          void incomplete('storage_error', message);
        }, timeoutMs);
        requests.set(requestId, { resolve, reject, timer });
        ready.then(() => {
          if (requests.has(requestId)) sim.historyBoundary(captureEnd, requestId);
        });
      });
      boundaryWork = work;
      return work.finally(() => {
        if (boundaryWork === work) boundaryWork = null;
      });
    },
    abort(message) {
      sim.stopHistory(message);
      enqueue(() => incomplete('capture_error', message));
    },
    detach() {
      for (const off of unsubscribe) off();
      for (const requestId of requests.keys()) {
        settle(requestId, new Error('World was closed before history finalization'));
      }
      // Page teardown is not a finalization barrier. Persisted open prefixes remain
      // explicitly unfinalized; another tab may still own a different open run.
      if (state === 'starting' || state === 'recording') state = 'detached';
      readyResolve();
    },
  };
}
