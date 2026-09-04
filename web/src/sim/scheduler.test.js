/**
 * Scheduler regressions for bounded work and explicit yielding. Fake timers make the
 * worker's responsiveness contract deterministic without starting a browser worker.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { createTickScheduler } from './scheduler.js';

test('a slow simulation performs one bounded batch before yielding', () => {
  let clock = 0;
  const timers = [];
  const batches = [];
  const statuses = [];
  const scheduler = createTickScheduler({
    step(ticks) {
      batches.push(ticks);
      clock += 100 * ticks;
    },
    publish() {},
    onError(error) {
      assert.fail(error);
    },
    onRunningChange(value) {
      statuses.push(value);
    },
    now: () => clock,
    schedule(callback, delay) {
      timers.push({ callback, delay });
      return timers.length;
    },
    cancel() {},
  });

  scheduler.setSpeed(100);
  scheduler.start();
  assert.deepEqual(statuses, [true]);

  clock = 20;
  timers.shift().callback();
  assert.deepEqual(batches, [1], 'the first unmeasured slice must contain one tick');
  assert.equal(timers.length, 1, 'work must yield through the timer queue');

  timers.shift().callback();
  assert.deepEqual(batches, [1, 1], 'a measured 100ms tick must not be batched');
});

test('invalid runtime timing values are rejected at the worker boundary', () => {
  const scheduler = createTickScheduler({
    step() {},
    publish() {},
    onError() {},
    onRunningChange() {},
  });

  for (const speed of [-1, Number.NaN, Number.POSITIVE_INFINITY]) {
    assert.throws(() => scheduler.setSpeed(speed), /finite non-negative/);
  }
  for (const dt of [0, -1, Number.NaN, Number.POSITIVE_INFINITY]) {
    assert.throws(() => scheduler.setSecondsPerTick(dt), /finite positive/);
  }
});

test('zero speed discards work already queued at the old speed', () => {
  let clock = 0;
  const timers = [];
  let ticksRun = 0;
  const scheduler = createTickScheduler({
    step(ticks) {
      ticksRun += ticks;
      clock += 100;
    },
    publish() {},
    onError(error) {
      assert.fail(error);
    },
    onRunningChange() {},
    now: () => clock,
    schedule(callback) {
      timers.push(callback);
      return timers.length;
    },
    cancel() {},
  });

  scheduler.setSpeed(100);
  scheduler.start();
  clock = 20;
  timers.shift()();
  assert.equal(ticksRun, 1);

  scheduler.setSpeed(0);
  timers.shift()();
  assert.equal(ticksRun, 1, 'queued debt advanced the simulation at zero speed');
});
