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
      meanEnergy: 12,
    }),
  ];
  samples = appendMetric(
    samples,
    metricFromMessage({
      tick: '9007199254740994',
      population: 3,
      descendants: 2,
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
  const first = metricFromMessage({ tick: '8', population: 4, descendants: 2, meanEnergy: 12 });
  const updated = metricFromMessage({
    tick: '8',
    population: 3,
    descendants: 2,
    meanEnergy: 10,
  });
  assert.deepEqual(appendMetric([first], updated, 10), [updated]);
});

test('invalid metrics fail at the worker-message boundary', () => {
  assert.throws(
    () => metricFromMessage({ tick: '1', population: -1, descendants: 0, meanEnergy: 10 }),
    /invalid metrics payload/,
  );
  assert.throws(
    () => metricFromMessage({ tick: '1', population: 2, descendants: 3, meanEnergy: 10 }),
    /invalid metrics payload/,
  );
});
