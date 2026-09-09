/** Inspector boundary parsing and generated-genome integration tests. */

import assert from 'node:assert/strict';
import test from 'node:test';

import { decodeInspection, summarizeGenes } from './model.ts';

const inspection = {
  index: 3,
  incarnation: 7,
  tick: '9007199254740993',
  energy: 71.5,
  age: 12,
  size: 3,
  signature: [0.1, 0.2, 0.3],
  species_id: 0,
  parent_a: 1,
  parent_b: 4294967295,
  brain_units: 10,
  sensor_load: 4,
  activations: [0.25, -0.5],
  genome: [
    { Neuron: { id: 1, bias: 0, tau: 1, activation: 'Tanh', period: 0 } },
    { Neuron: { id: 2, bias: 0, tau: 1, activation: 'Sigmoid', period: 0 } },
    { Body: { trait_: 'Size', value: 3 } },
  ],
};

test('inspection JSON is validated and decoded', () => {
  const decoded = decodeInspection(JSON.stringify(inspection));
  assert.equal(decoded.index, 3);
  assert.deepEqual(summarizeGenes(decoded.genome), [
    { kind: 'Neuron', count: 2 },
    { kind: 'Body', count: 1 },
  ]);
});

test('malformed inspection JSON is rejected', () => {
  assert.throws(
    () => decodeInspection(JSON.stringify({ ...inspection, activations: ['not a number'] })),
    /invalid inspection payload/,
  );
});

test('sensor-born brains decode with a different activation and genome length', () => {
  const child = {
    ...inspection,
    index: 4,
    parent_a: inspection.index,
    activations: [...inspection.activations, 0.2, 0.3, 0.4],
    genome: [
      ...inspection.genome,
      ...[3, 4, 5].map((id) => ({
        Neuron: { id, bias: 0, tau: 1, activation: 'Sigmoid', period: 0 },
      })),
      { Sensor: { id: 6, modality: 'Chemo', params: [0, 20, 0, 0], targets: [3, 4, 5, 4294967295] } },
    ],
  };
  assert.equal(decodeInspection(JSON.stringify(inspection)).activations.length, 2);
  const decoded = decodeInspection(JSON.stringify(child));
  assert.equal(decoded.activations.length, 5);
  assert.deepEqual(summarizeGenes(decoded.genome), [
    { kind: 'Neuron', count: 5 },
    { kind: 'Body', count: 1 },
    { kind: 'Sensor', count: 1 },
  ]);
  assert.equal(decodeInspection(JSON.stringify(inspection)).activations.length, 2);
});
