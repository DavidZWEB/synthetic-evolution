import assert from 'node:assert/strict';
import test from 'node:test';
import { createHistorySession } from './controller.js';

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
