import assert from 'node:assert/strict';
import test from 'node:test';
import { createRequestGate } from './request-gate.js';

test('one request is in flight and only its reply settles it', () => {
  const gate = createRequestGate({ intervalMs: 250 });
  const first = gate.begin(0);
  assert.equal(gate.begin(0), null, 'a second request waits for the first');
  assert.equal(gate.settle(first + 1), false, 'unknown ids are rejected');
  assert.equal(gate.settle(first), true);
  assert.equal(gate.settle(first), false, 'a reply settles at most once');
  assert.notEqual(gate.begin(0), null);
});

test('invalidation orphans the in-flight reply', () => {
  const gate = createRequestGate({ intervalMs: 250 });
  const stale = gate.begin(0);
  gate.invalidate();
  assert.equal(gate.settle(stale), false);
  assert.ok(gate.begin(0) > stale);
});

test('demand is kept mid-flight and waits out the cooldown', () => {
  const gate = createRequestGate({ intervalMs: 250 });
  const id = gate.begin(0);
  gate.demand();
  assert.equal(gate.due(300), false, 'nothing is due while a request is in flight');
  gate.settle(id);
  assert.equal(gate.due(249), false, 'cooldown still applies after the reply');
  assert.equal(gate.due(250), true);
  gate.demand(false);
  assert.equal(gate.due(250), false, 'demand can be withdrawn');
  gate.begin(250);
  gate.invalidate();
  gate.demand();
  assert.equal(gate.due(251), false, 'invalidation keeps the cooldown by default');
  gate.invalidate({ resetCooldown: true });
  gate.demand();
  assert.equal(gate.due(251), true);
});
