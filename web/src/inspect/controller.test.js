/** Inspector selection identity and stale-response tests. */

import assert from 'node:assert/strict';
import test from 'node:test';

import { createInspectorController } from './controller.js';

const payload = (index, incarnation) =>
  JSON.stringify({
    index,
    incarnation,
    tick: '4',
    energy: 50,
    age: 4,
    size: 3,
    signature: [0.1, 0.2, 0.3],
    species_id: 0,
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
