import assert from 'node:assert/strict';
import test from 'node:test';
import { createHistorySession } from './controller.js';
import { createHistoryDelivery } from './delivery.js';
import { encodeArchive, parseArchive } from './archive.js';
import { fixtureHeader, origin } from './fixtures.js';

const turn = () => new Promise((resolve) => setImmediate(resolve));
const emptyBatch = {
  batchId: 1, rows: [], tick: '0', nextSequence: '0', droppedEvents: '0',
};

function fixture(overrides = {}, timeoutMs = 1000) {
  const listeners = new Map();
  const acknowledgements = [];
  const stops = [];
  const changes = [];
  const marked = [];
  const appended = [];
  const sim = {
    on(kind, callback) {
      listeners.set(kind, callback);
      return () => listeners.delete(kind);
    },
    acknowledgeHistory: (id) => acknowledgements.push(id),
    stopHistory: (message) => stops.push(message),
    historyBoundary() {},
  };
  const store = {
    create: async () => 'local-id',
    append: async (...args) => { appended.push(args); },
    markIncomplete: async (...args) => { marked.push(args); },
    ...overrides,
  };
  const session = createHistorySession({
    sim, getStore: async () => store, onChange: (state) => changes.push(state),
    onSaved: async () => {}, timeoutMs,
  });
  const emit = (kind, value) => listeners.get(kind)?.(value);
  return { session, emit, acknowledgements, stops, changes, marked, appended };
}

test('a batch is acknowledged only after its IndexedDB commit resolves', async () => {
  let commit;
  const f = fixture({ append: () => new Promise((resolve) => { commit = resolve; }) });
  f.emit('historyReady', { header: {} });
  f.emit('historyBatch', emptyBatch);
  await turn();
  assert.equal(f.session.id, 'local-id');
  assert.deepEqual(f.acknowledgements, []);
  commit();
  await turn();
  assert.deepEqual(f.acknowledgements, [1]);
  f.session.detach();
});

test('quota failure preserves the earlier prefix, stops capture, and rejects a pending barrier', async () => {
  const error = Object.assign(new Error('run limit exceeded'), { code: 'storage_limit' });
  const f = fixture({ append: async () => { throw error; } });
  f.emit('historyReady', { header: {} });
  await turn();
  const pending = assert.rejects(f.session.boundary('reseeded'), /run limit exceeded/);
  f.emit('historyBatch', emptyBatch);
  await pending;
  await turn();
  assert.deepEqual(f.acknowledgements, []);
  assert.equal(f.stops.length, 1);
  assert.deepEqual(f.marked, [['local-id', 'storage_limit']]);
  assert.equal(f.session.active, false);
  f.session.detach();
});

test('the complete drained prefix is checked against exact worker counters before committing', async () => {
  const f = fixture();
  f.emit('historyReady', { header: {} });
  f.emit('historyBatch', { ...emptyBatch, nextSequence: '9007199254740993' });
  await turn();
  assert.equal(f.appended.length, 0);
  assert.equal(f.acknowledgements.length, 0);
  assert.match(f.changes.at(-1).message, /counters/);
  assert.deepEqual(f.marked, [['local-id', 'capture_error']]);
  f.session.detach();
});

test('storage creation failure does not leave boundaries waiting for a ready message', async () => {
  const f = fixture({ create: async () => { throw new Error('IndexedDB unavailable'); } });
  const pending = assert.rejects(f.session.boundary('stopped'), /IndexedDB unavailable/);
  f.emit('historyReady', { header: {} });
  await pending;
  assert.equal(f.session.active, false);
  f.session.detach();
});

test('blocked storage times out a requested world transition without waiting for storage', async () => {
  const f = fixture({ create: () => new Promise(() => {}) }, 10);
  f.emit('historyReady', { header: {} });
  await assert.rejects(f.session.boundary('reseeded'), /storage did not respond/);
  assert.equal(f.session.active, false);
  assert.equal(f.stops.length, 1);
  f.session.detach();
});

test('detaching rejects pending requests and never claims a completed archive', async () => {
  const f = fixture();
  f.emit('historyReady', { header: {} });
  await turn();
  const pending = assert.rejects(f.session.boundary('snapshot'), /closed before history/);
  f.session.detach();
  await pending;
  f.emit('historyBatch', emptyBatch);
  await turn();
  assert.equal(f.appended.length, 0);
  assert.deepEqual(f.marked, []);
});

async function closingPipeline(timeoutMs = 1000) {
  const listeners = new Map();
  const archive = { header: fixtureHeader(), rows: [], completion: null };
  let release;
  let appends = 0;
  let records = [origin()];
  const marked = [];
  const stops = [];
  const store = {
    create: async () => 'local-id',
    append: async (_id, rows) => {
      if (++appends === 2) await new Promise((resolve) => { release = resolve; });
      archive.rows.push(...rows);
    },
    get: async () => archive,
    finalize: async (_id, completion) => { archive.completion = completion; },
    markIncomplete: async (_id, reason) => { marked.push(reason); },
    exportArchive: async () => encodeArchive(archive),
  };
  const wasm = {
    history_pending: () => records.length > 0,
    disable_history() {},
    state_hash: () => 42n,
    drain_history: () => {
      const batch = records;
      records = [];
      return JSON.stringify({
        records: batch, through_tick: '1', next_sequence: '1',
        dropped_events: '0', sequence_exhausted: false,
      });
    },
  };
  const delivery = createHistoryDelivery({
    sim: wasm, cohort: 'evolving',
    send: (message) => queueMicrotask(() => listeners.get(message.kind)?.(message)),
  });
  const client = {
    on(kind, callback) {
      listeners.set(kind, callback);
      return () => listeners.delete(kind);
    },
    historyBoundary: (end, requestId) => queueMicrotask(() => delivery.boundary(end, requestId)),
    acknowledgeHistory: (id) => queueMicrotask(() => delivery.acknowledge(id)),
    stopHistory: (message) => { stops.push(message); delivery.stop(message); },
  };
  const session = createHistorySession({
    sim: client, getStore: async () => store, onChange() {}, onSaved: async () => {}, timeoutMs,
  });
  listeners.get('historyReady')({ header: archive.header });
  delivery.pump(true);
  await turn();
  assert.equal(archive.rows.length, 1);
  return {
    session, delivery, archive, marked, stops,
    commit: () => release(),
  };
}

test('reseed waits for a worker-initiated retune closure to commit its footer', async () => {
  const f = await closingPipeline();
  f.delivery.boundary('params_changed', null, () => true);
  await turn();
  let switched = false;
  const reseed = (async () => {
    await f.session.waitForBoundary();
    if (f.session.active) await f.session.boundary('reseeded');
    f.session.detach();
    switched = true;
  })();
  await turn();
  assert.equal(switched, false, 'World was replaced while its final append was pending');
  f.commit();
  await reseed;
  assert.equal(f.archive.completion.data.capture_end, 'params_changed');
  assert.equal(f.archive.completion.data.cohorts[0].final_state_hash, '000000000000002a');
  assert.deepEqual(f.stops, []);
});

for (const captureEnd of ['reseeded', 'snapshot']) {
  test(`a ${captureEnd} request crossing the retune message joins its committed closure`, async () => {
    const f = await closingPipeline();
    f.delivery.boundary('params_changed', null, () => true);
    let settled = false;
    const requested = f.session.boundary(captureEnd).then((value) => {
      settled = true;
      return value;
    });
    await turn();
    assert.equal(settled, false);
    f.commit();
    const result = await requested;
    await f.session.waitForBoundary();
    assert.equal(f.session.active, false);
    assert.equal(f.archive.completion.data.capture_end, 'params_changed');
    if (captureEnd === 'snapshot') {
      assert.equal((await parseArchive(result)).completion.data.capture_end, 'params_changed');
    }
    assert.deepEqual(f.stops, []);
    f.session.detach();
  });
}

test('a blocked implicit closure times out without hanging reseed or later claiming completeness', async () => {
  const f = await closingPipeline(10);
  f.delivery.boundary('params_changed', null, () => true);
  await turn();
  await assert.rejects(f.session.waitForBoundary(), /storage did not respond/);
  assert.equal(f.session.active, false);
  f.commit();
  await turn();
  assert.equal(f.archive.completion, null);
  assert.deepEqual(f.marked, ['storage_error']);
  f.session.detach();
});

test('a snapshot sharing a retune queued behind an earlier append exports the closed prefix', async () => {
  const f = await closingPipeline();
  f.delivery.pump(true);
  f.delivery.boundary('params_changed', null, () => true);
  const requested = f.session.boundary('snapshot');
  await turn();
  assert.equal(f.archive.completion, null);
  f.commit();
  const exported = await parseArchive(await requested);
  assert.equal(exported.completion.data.capture_end, 'params_changed');
  assert.deepEqual(exported.completion, f.archive.completion);
  assert.deepEqual(f.stops, []);
  f.session.detach();
});
