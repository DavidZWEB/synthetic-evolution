/** Bounded IndexedDB archives; capture failure preserves the last committed prefix. */
import {
  byteLength, completionFor, encodeArchive, encodeLine, footerAllowance, INCOMPLETE_ENDS,
  validateArchive, validateHeader, validatePrefix,
} from './archive.js';
import { decimal, requireThat } from './shape.js';

export const DEFAULT_LIMITS = Object.freeze({
  perRunBytes: 10 * 1024 * 1024,
  totalBytes: 50 * 1024 * 1024,
  maxRuns: 20,
});
const DATABASE = 'synthetic-evolution-history';
const STORES = ['accounting', 'runs', 'archives'];

function failure(message, code) {
  return Object.assign(new Error(message), { code });
}

function storageFailure(error) {
  if (error instanceof DOMException) {
    return Object.assign(new Error(`History storage failed: ${error.message}`, { cause: error }),
      { code: 'storage_error' });
  }
  return error;
}

function metadata(run) {
  const { payloadBytes, reservedBytes, ...publicFields } = run;
  return publicFields;
}

function totals(counts, key) {
  return String(Object.values(counts).reduce((sum, cohort) => sum + BigInt(cohort[key]), 0n));
}

/**
 * Every read/modify/write shares accounting's readwrite transaction lock, including
 * across tabs. No promise or external validator may suspend the transaction body.
 */
function transaction(db, mode, operation) {
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORES, mode);
    let result;
    let error;
    const stores = Object.fromEntries(STORES.map((name) => [name, tx.objectStore(name)]));
    const abort = (reason) => {
      error = reason;
      tx.abort();
    };
    const run = (callback) => {
      try { callback(); } catch (reason) { abort(reason); }
    };
    const read = (request, callback) => {
      request.onsuccess = () => run(() => callback(request.result));
    };
    tx.oncomplete = () => resolve(result);
    tx.onabort = () => reject(error ?? tx.error ?? failure('History transaction aborted', 'storage_error'));
    run(() => operation(stores, read, (value) => { result = value; }));
  }).catch((error) => { throw storageFailure(error); });
}

export async function openHistoryStore({ limits: overrides = {} } = {}) {
  const limits = { ...DEFAULT_LIMITS, ...overrides };
  requireThat(Object.keys(limits).length === 3, 'unknown storage limit');
  for (const [key, value] of Object.entries(limits)) {
    requireThat(Number.isSafeInteger(value) && value > 0, `invalid storage limit ${key}`);
  }
  const db = await new Promise((resolve, reject) => {
    const request = indexedDB.open(DATABASE, 1);
    let blocked = false;
    request.onupgradeneeded = () => {
      request.result.createObjectStore('accounting');
      request.result.createObjectStore('runs', { keyPath: 'id' });
      request.result.createObjectStore('archives', { keyPath: 'id' });
    };
    request.onsuccess = () => {
      if (blocked) request.result.close();
      else resolve(request.result);
    };
    request.onerror = () => reject(request.error);
    request.onblocked = () => {
      blocked = true;
      reject(failure('History database upgrade is blocked by another tab', 'storage_error'));
    };
  }).catch((error) => { throw storageFailure(error); });
  db.onversionchange = () => db.close();

  function budget(accounting, oldBytes, newBytes, additionalRuns = 0) {
    if (newBytes > limits.perRunBytes ||
      accounting.bytes - oldBytes + newBytes > limits.totalBytes ||
      accounting.runs + additionalRuns > limits.maxRuns) {
      throw failure('History storage limit reached; delete a saved run or export the retained prefix', 'storage_limit');
    }
    return {
      bytes: accounting.bytes - oldBytes + newBytes,
      runs: accounting.runs + additionalRuns,
    };
  }

  function writable(run) {
    if (!run) throw failure('Saved history no longer exists', 'not_found');
    if (run.status !== 'open' || run.readOnly) throw failure('Saved history is read-only', 'read_only');
  }

  async function insert(archive, imported) {
    const header = archive.header;
    const counts = validatePrefix(header, archive.rows, {
      tick: archive.completion?.data.ticks ?? '0',
    });
    const payloadBytes = [header, ...archive.rows].reduce((sum, row) => sum + byteLength(encodeLine(row)), 0);
    const reservedBytes = imported ? byteLength(encodeLine(archive.completion)) : footerAllowance(header);
    const captureEnd = imported ? archive.completion.data.capture_end ?? 'finished' : null;
    const id = crypto.randomUUID();
    const run = {
      id, runId: header.data.run_id ?? null, seed: header.data.provenance.seed,
      cohorts: [...header.data.cohorts],
      status: imported ? 'closed' : 'open',
      captureEnd, bytes: payloadBytes + reservedBytes, payloadBytes, reservedBytes,
      createdAt: Date.now(), tick: archive.completion?.data.ticks ?? '0',
      eventCount: totals(counts, 'events'), gapCount: totals(counts, 'gaps'),
      readOnly: imported,
    };
    return transaction(db, 'readwrite', (stores, read, done) => {
      read(stores.accounting.get('total'), (existing) => {
        const next = budget(existing ?? { bytes: 0, runs: 0 }, 0, run.bytes, 1);
        stores.runs.add(run);
        stores.archives.add({ id, ...archive });
        stores.accounting.put(next, 'total');
        done(id);
      });
    });
  }

  function update(id, operation) {
    return transaction(db, 'readwrite', (stores, read, done) => {
      read(stores.runs.get(id), (run) => {
        writable(run);
        read(stores.archives.get(id), (archive) => {
          read(stores.accounting.get('total'), (accounting) => {
            const oldBytes = run.bytes;
            operation(run, archive);
            const next = budget(accounting, oldBytes, run.bytes);
            stores.runs.put(run);
            stores.archives.put(archive);
            stores.accounting.put(next, 'total');
            done(metadata(run));
          });
        });
      });
    });
  }

  const store = {
    async create(header) {
      header = structuredClone(header);
      validateHeader(header);
      requireThat(header.data.schema_version === 2, 'live capture requires schema v2');
      return insert({ header, rows: [], completion: null }, false);
    },

    async append(id, rows, { tick }) {
      rows = structuredClone(rows);
      decimal(tick, 'batch tick');
      return update(id, (run, archive) => {
        requireThat(BigInt(tick) >= BigInt(run.tick), 'committed boundary cannot move backwards');
        const combined = archive.rows.concat(rows);
        const counts = validatePrefix(archive.header, combined, { tick });
        run.payloadBytes += rows.reduce((sum, row) => sum + byteLength(encodeLine(row)), 0);
        run.bytes = run.payloadBytes + run.reservedBytes;
        run.tick = tick;
        run.eventCount = totals(counts, 'events');
        run.gapCount = totals(counts, 'gaps');
        archive.rows = combined;
      });
    },

    async finalize(id, completion) {
      completion = structuredClone(completion);
      return update(id, (run, archive) => {
        requireThat(completion?.data?.ticks === run.tick, 'footer boundary is not the last committed batch');
        archive.completion = completion;
        const text = encodeArchive(archive);
        run.bytes = byteLength(text);
        run.reservedBytes = byteLength(encodeLine(completion));
        run.captureEnd = completion.data.capture_end;
        run.status = 'closed';
        run.readOnly = true;
      });
    },

    async markIncomplete(id, reason) {
      requireThat(INCOMPLETE_ENDS.includes(reason), 'invalid incomplete capture reason');
      // Only small metadata changes here: even a quota failure must not rewrite the prefix.
      return transaction(db, 'readwrite', (stores, read, done) => {
        read(stores.runs.get(id), (run) => {
          writable(run);
          run.status = 'closed';
          run.captureEnd = reason;
          run.readOnly = true;
          stores.runs.put(run);
          done(metadata(run));
        });
      });
    },

    async list() {
      return transaction(db, 'readonly', (stores, read, done) => {
        read(stores.runs.getAll(), (runs) => {
          done(runs.sort((a, b) => b.createdAt - a.createdAt || a.id.localeCompare(b.id)).map(metadata));
        });
      });
    },

    async get(id) {
      return transaction(db, 'readonly', (stores, read, done) => {
        read(stores.runs.get(id), (run) => {
          if (!run) throw failure('Saved history no longer exists', 'not_found');
          read(stores.archives.get(id), (archive) => done({ ...archive, ...metadata(run) }));
        });
      });
    },

    async delete(id) {
      return transaction(db, 'readwrite', (stores, read, done) => {
        read(stores.runs.get(id), (run) => {
          if (!run) { done(); return; }
          read(stores.accounting.get('total'), (accounting) => {
            stores.archives.delete(id);
            stores.runs.delete(id);
            stores.accounting.put({ bytes: accounting.bytes - run.bytes, runs: accounting.runs - 1 }, 'total');
            done();
          });
        });
      });
    },

    async importArchive(archive) {
      archive = structuredClone(archive);
      await validateArchive(archive);
      return insert({ header: archive.header, rows: archive.rows, completion: archive.completion }, true);
    },

    async exportArchive(id) {
      const archive = await store.get(id);
      if (archive.completion === null) {
        archive.completion = completionFor(archive.header, archive.rows, {
          tick: archive.tick, captureEnd: archive.captureEnd ?? 'unfinalized',
        });
      }
      return encodeArchive(archive);
    },

    close() { db.close(); },
  };
  return store;
}
