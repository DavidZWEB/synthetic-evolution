import assert from 'node:assert/strict';
import test from 'node:test';
import { createHistoryDelivery } from './delivery.js';

function fixture() {
  let tick = 0;
  let events = [];
  let hashCalls = 0;
  let disabled = 0;
  const sent = [];
  const sim = {
    history_pending: () => events.length > 0,
    disable_history: () => { disabled++; },
    state_hash: () => { hashCalls++; return BigInt(tick); },
    drain_history: () => {
      const records = events;
      events = [];
      return JSON.stringify({
        records, through_tick: String(tick), next_sequence: String(tick),
        dropped_events: '0', sequence_exhausted: false,
      });
    },
  };
  const delivery = createHistoryDelivery({ sim, cohort: 'evolving', send: (x) => sent.push(x) });
  return {
    delivery, sim, sent,
    step() { events.push({ kind: 'event', data: { sequence: String(tick++) } }); },
    get hashCalls() { return hashCalls; },
    get disabled() { return disabled; },
  };
}

test('slow acknowledgements bound drained batches without blocking simulation', () => {
  const f = fixture();
  f.step();
  f.delivery.pump();
  for (let i = 0; i < 10; i++) { f.step(); f.delivery.pump(); }
  assert.equal(f.sent.length, 1);
  assert.equal(f.hashCalls, 0);
  f.delivery.acknowledge(1);
  assert.equal(f.sent[1].rows.length, 10);
  assert.equal(f.sent[1].tick, '11');
  assert.equal(f.sent[1].rows[0].data.cohort, 'evolving');
  f.delivery.acknowledge(2);
  assert.equal(f.sent.length, 2, 'no empty tick writes');
});

test('snapshot barriers drain a consistent boundary after the prior acknowledgement', () => {
  const f = fixture();
  f.step();
  f.delivery.pump();
  f.delivery.boundary('snapshot', 9);
  f.step();
  assert.equal(f.hashCalls, 0);
  f.delivery.acknowledge(1);
  assert.equal(f.sent[1].tick, '2');
  assert.equal(f.sent[1].stateHash, '0000000000000002');
  assert.equal(f.sent[1].requestId, 9);
  assert.equal(f.sent[1].captureEnd, 'snapshot');
  assert.equal(f.delivery.active, true);
});

test('stop emits even an empty final boundary and releases capture, not the simulation', () => {
  const f = fixture();
  f.delivery.boundary('reseeded', 1);
  assert.deepEqual(f.sent[0].rows, []);
  assert.equal(f.sent[0].captureEnd, 'reseeded');
  assert.equal(f.disabled, 1);
  assert.equal(f.delivery.active, false);
  f.delivery.acknowledge(1);
  assert.equal(f.sent.length, 1);
});

test('retune captures old hash only after successful application', () => {
  const f = fixture();
  f.step();
  f.delivery.boundary('params_changed', null, () => false);
  assert.equal(f.delivery.active, true);
  assert.equal(f.sent[0].captureEnd, undefined);
  f.delivery.boundary('params_changed', null, () => {
    f.sim.state_hash = () => 99n;
    return true;
  });
  f.delivery.acknowledge(1);
  assert.equal(f.sent[1].captureEnd, 'params_changed');
  assert.equal(f.sent[1].stateHash, '0000000000000001');
  assert.equal(f.sim.state_hash(), 99n);
  assert.equal(f.delivery.active, false);
});

test('bad or failed acknowledgements explicitly stop capture', () => {
  for (const [id, error] of [[2, undefined], [1, 'quota exceeded']]) {
    const f = fixture();
    f.delivery.pump(true);
    f.delivery.boundary('snapshot', 12);
    f.delivery.acknowledge(id, error);
    assert.equal(f.delivery.active, false);
    assert.equal(f.sent.at(-1).kind, 'historyError');
    assert.deepEqual(f.sent.at(-1).requestIds, [12]);
  }
});

test('capture failures resolve boundary errors instead of stalling export forever', () => {
  const f = fixture();
  f.sim.state_hash = () => { throw new Error('hash unavailable'); };
  f.delivery.boundary('snapshot', 4);
  assert.equal(f.delivery.active, false);
  assert.equal(f.sent[0].kind, 'historyError');
  assert.deepEqual(f.sent[0].requestIds, [4]);
  assert.match(f.sent[0].message, /hash unavailable/);
});

test('a rejected second boundary does not cancel the first or stop capture', () => {
  const f = fixture();
  f.delivery.pump(true);
  f.delivery.boundary('snapshot', 1);
  assert.equal(f.delivery.boundary('reseeded', 2), false);
  assert.equal(f.sent[1].requestOnly, true);
  assert.deepEqual(f.sent[1].requestIds, [2]);
  f.delivery.acknowledge(1);
  assert.equal(f.sent[2].requestId, 1);
  assert.equal(f.sent[2].captureEnd, 'snapshot');
  assert.equal(f.delivery.active, true);
});

test('sequence exhaustion produces an explicitly incomplete final prefix', () => {
  const f = fixture();
  f.sim.drain_history = () => JSON.stringify({
    records: [], through_tick: '9007199254740993',
    next_sequence: '18446744073709551615', dropped_events: '0',
    sequence_exhausted: true,
  });
  f.delivery.pump(true);
  assert.equal(f.sent[0].captureEnd, 'capture_error');
  assert.equal(f.sent[0].tick, '9007199254740993');
  assert.equal(f.sent[0].nextSequence, '18446744073709551615');
  assert.equal(f.sent[0].stateHash, null);
  assert.equal(f.delivery.active, false);
});
