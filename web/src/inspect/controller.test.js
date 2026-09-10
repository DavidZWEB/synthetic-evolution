/** Inspector selection identity and stale-response tests. */

import assert from 'node:assert/strict';
import test from 'node:test';

import { createInspectorController } from './controller.js';

const payload = (index, incarnation, tick = 4) =>
  JSON.stringify({
    index,
    incarnation,
    tick: String(tick),
    energy: 50,
    age: tick,
    size: 3,
    signature: [0.1, 0.2, 0.3],
    species_id: 0,
    birth_id: '10',
    parent_birth_a: null,
    parent_birth_b: null,
    parent_a: 4294967295,
    parent_b: 4294967295,
    brain_units: 2,
    sensor_load: 1,
    activations: [0.2],
    genome: [{ Body: { trait_: 'Size', value: 3 } }],
  });

test('selection rejects stale responses and polls only its incarnation', () => {
  let clock = 0;
  const requests = [];
  const changes = [];
  const selections = [];
  const controller = createInspectorController({
    getSim: () => ({ inspect: (...request) => requests.push(request) }),
    getRenderer: () => ({
      pick: () => ({ index: 2, incarnation: 7 }),
      select: (index) => selections.push(index),
    }),
    getFrame: () => ({ views: {} }),
    onChange: (change) => changes.push(change),
    now: () => clock,
    intervalMs: 250,
  });

  controller.pickAt(10, 20);
  assert.deepEqual(requests, [[2, 7, 2]]);
  assert.equal(controller.accept({ index: 2, incarnation: 7, requestId: 1, agent: payload(2, 7) }), false);
  assert.equal(controller.accept({ index: 2, incarnation: 7, requestId: 2, agent: payload(2, 7) }), true);
  assert.equal(changes.at(-1).inspection.incarnation, 7);

  clock = 300;
  controller.poll({ fresh: true }, clock);
  assert.deepEqual(requests.at(-1), [2, 7, 3]);

  controller.select({ index: 4, incarnation: 9 });
  assert.equal(controller.accept({ index: 2, incarnation: 7, requestId: 3, agent: payload(2, 7) }), false);
  assert.deepEqual(selections, [
    { index: 2, incarnation: 7 },
    { index: 4, incarnation: 9 },
  ]);
});

function pollingFixture() {
  let clock = 0;
  const requests = [];
  const changes = [];
  const controller = createInspectorController({
    getSim: () => ({ inspect: (...request) => requests.push(request) }),
    getRenderer: () => ({ select() {} }),
    getFrame: () => null,
    onChange: (change) => changes.push(change),
    now: () => clock,
  });
  controller.select({ index: 2, incarnation: 7 });
  return {
    controller,
    requests,
    changes,
    poll(timestamp, fresh) {
      clock = timestamp;
      controller.poll({ fresh }, timestamp);
    },
    accept(tick) {
      const [index, incarnation, requestId] = requests.at(-1);
      return controller.accept({ index, incarnation, requestId, agent: payload(index, incarnation, tick) });
    },
  };
}

test('a paused final frame survives the inspection cooldown', () => {
  const fixture = pollingFixture();
  assert.equal(fixture.accept(0), true);
  fixture.poll(20, true);
  assert.equal(fixture.requests.length, 1, 'polled inside the cooldown');
  fixture.poll(251, false);
  assert.equal(fixture.requests.length, 2, 'forgot the only fresh frame');
  assert.equal(fixture.accept(1), true);
  assert.equal(fixture.changes.at(-1).inspection.age, 1);
  fixture.poll(1000, false);
  assert.equal(fixture.requests.length, 2, 'unchanged paused frames triggered more requests');
});

test('slow replies remain usable while refresh demand is coalesced', () => {
  const fixture = pollingFixture();
  for (const timestamp of [300, 600, 900]) fixture.poll(timestamp, true);
  assert.equal(fixture.requests.length, 1, 'overlapping requests invalidated the pending reply');
  assert.equal(fixture.accept(0), true);
  fixture.poll(901, false);
  assert.equal(fixture.requests.length, 2, 'lost updates received while inspection was pending');
  assert.equal(fixture.accept(4), true);
  assert.equal(fixture.changes.at(-1).inspection.age, 4);
});

test('stable birth metadata does not replace incarnation and reseed request guards', () => {
  const fixture = pollingFixture();
  const [index, oldIncarnation, oldRequestId] = fixture.requests.at(-1);
  fixture.controller.select({ index, incarnation: 8 });
  const requestId = fixture.requests.at(-1)[2];
  assert.equal(fixture.controller.accept({
    index, incarnation: oldIncarnation, requestId, agent: payload(index, oldIncarnation),
  }), false, 'recycled slot accepted the old incarnation');
  assert.equal(fixture.controller.accept({
    index, incarnation: 8, requestId, agent: payload(index, 8),
  }), true);

  fixture.controller.select(null);
  fixture.controller.select({ index, incarnation: oldIncarnation });
  assert.equal(fixture.controller.accept({
    index, incarnation: oldIncarnation, requestId: oldRequestId,
    agent: payload(index, oldIncarnation),
  }), false, 'world-local birth IDs and slot incarnations can repeat after reseeding');
  assert.equal(fixture.accept(0), true);
  assert.equal(fixture.changes.at(-1).inspection.birth_id, '10');
});
