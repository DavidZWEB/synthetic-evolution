/** Live telemetry parsing and retention tests. */

import assert from 'node:assert/strict';
import test from 'node:test';

import { appendMetric, metricFromMessage } from './history.ts';

test('metrics preserve large ticks and stay bounded', () => {
  let samples = [
    metricFromMessage({
      tick: '9007199254740993',
      population: 4,
      descendants: 3,
      speciesCount: 2,
      unclassifiedPopulation: 1,
      meanEnergy: 12,
    }),
  ];
  samples = appendMetric(
    samples,
    metricFromMessage({
      tick: '9007199254740994',
      population: 3,
      descendants: 2,
      speciesCount: 2,
      unclassifiedPopulation: 0,
      meanEnergy: 11,
    }),
    2,
  );
  samples = appendMetric(
    samples,
    metricFromMessage({
      tick: '9007199254740995',
      population: 2,
      descendants: 1,
      speciesCount: 1,
      unclassifiedPopulation: 0,
      meanEnergy: 10,
    }),
    2,
  );
  assert.deepEqual(
    samples.map((sample) => sample.tick),
    [9007199254740994n, 9007199254740995n],
  );
});

test('a repeated tick replaces its prior sample', () => {
  const first = metricFromMessage({
    tick: '8', population: 4, descendants: 2, meanEnergy: 12,
    speciesCount: 2, unclassifiedPopulation: 1,
  });
  const updated = metricFromMessage({
    tick: '8',
    population: 3,
    descendants: 2,
    speciesCount: 1,
    unclassifiedPopulation: 2,
    meanEnergy: 10,
  });
  assert.deepEqual(appendMetric([first], updated, 10), [updated]);
});

test('invalid metrics fail at the worker-message boundary', () => {
  assert.throws(
    () => metricFromMessage({
      tick: '1', population: -1, descendants: 0, meanEnergy: 10,
      speciesCount: 0, unclassifiedPopulation: 0,
    }),
    /invalid metrics payload/,
  );
  assert.throws(
    () => metricFromMessage({
      tick: '1', population: 2, descendants: 3, meanEnergy: 10,
      speciesCount: 1, unclassifiedPopulation: 0,
    }),
    /invalid metrics payload/,
  );
});

test('species status is required and malformed current messages never become zero counts', () => {
  const message = {
    tick: '1', population: 3, descendants: 1, meanEnergy: 10,
    speciesCount: 1, unclassifiedPopulation: 1,
  };
  assert.equal(metricFromMessage(message).speciesCount, 1);
  assert.equal(metricFromMessage(message).unclassifiedPopulation, 1);
  for (const field of ['speciesCount', 'unclassifiedPopulation'] as const) {
    for (const value of [undefined, null, '1', -1, 0.5, NaN, Infinity, 4]) {
      assert.throws(
        () => metricFromMessage({ ...message, [field]: value }),
        /invalid metrics payload/,
      );
    }
  }
  for (const counts of [
    { speciesCount: 0, unclassifiedPopulation: 1 },
    { speciesCount: 2, unclassifiedPopulation: 2 },
    { speciesCount: 1, unclassifiedPopulation: 3 },
  ]) {
    assert.throws(() => metricFromMessage({ ...message, ...counts }), /invalid metrics payload/);
  }
  const unclassified = metricFromMessage({
    ...message, speciesCount: 0, unclassifiedPopulation: 3,
  });
  assert.equal(unclassified.speciesCount, 0);
  assert.equal(unclassified.unclassifiedPopulation, 3);
});
