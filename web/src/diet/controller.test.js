/** Diet sampling: opt-in, one request at a time, and stale or malformed replies ignored. */

import assert from 'node:assert/strict';
import test from 'node:test';

import { createDietController } from './controller.js';

function fixture() {
  const calls = [];
  const samples = [];
  const controller = createDietController({
    getSim: () => ({ requestDiets: (id) => calls.push(id) }),
    onChange: (sample) => samples.push(sample),
  });
  const reply = (id = calls.at(-1), shares = new Uint8Array([0, 255]),
    incarnation = new Uint32Array([1, 2])) =>
    controller.accept({ requestId: id, tick: '5', shares, incarnation });
  return { controller, calls, samples, reply };
}

test('diets are requested only while the mode is on, and one at a time', () => {
  const f = fixture();
  f.controller.poll({ fresh: true }, 0);
  assert.deepEqual(f.calls, [], 'off by default');
  f.controller.setActive(true, 0);
  assert.equal(f.calls.length, 1, 'turning it on asks at once');
  f.controller.poll({ fresh: true }, 100);
  assert.equal(f.calls.length, 1, 'one request in flight');
  assert.equal(f.reply(), true);
  assert.deepEqual([...f.samples[0].shares], [0, 255]);
  f.controller.poll({ fresh: true }, 300);
  assert.equal(f.calls.length, 1, 'waits out the interval');
  f.controller.poll({ fresh: false }, 600);
  assert.equal(f.calls.length, 2, 'demand from a fresh frame survives the wait');
  f.controller.setActive(false, 600);
  assert.equal(f.reply(), false, 'turning it off drops the request in flight');
  f.controller.poll({ fresh: true }, 5_000);
  assert.equal(f.calls.length, 2);
});

test('stale and malformed replies change nothing', () => {
  const f = fixture();
  f.controller.setActive(true, 0);
  assert.equal(f.reply(f.calls[0] + 1), false, 'stale');
  assert.equal(f.reply(f.calls[0], new Uint8Array(3)), true, 'settles the request');
  assert.deepEqual(f.samples, [], 'mismatched lengths are not a sample');
  f.controller.reset();
  assert.deepEqual(f.samples, [null], 'a new world clears the colours');
});
