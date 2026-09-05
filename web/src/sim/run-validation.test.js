/** Run-validation ordering regressions. */

import assert from 'node:assert/strict';
import test from 'node:test';

import { createRunValidation } from './run-validation.js';

test('cancelling invalidates an older response', () => {
  const requests = [];
  const accepted = [];
  const pending = [];
  const validation = createRunValidation({
    getSim: () => ({ validateRun: (...request) => requests.push(request) }),
    onPendingChange: (value) => pending.push(value),
    onAccepted: (run) => accepted.push(run),
    onRejected: (error) => assert.fail(error),
  });

  validation.request({ seed: '42', founders: 20, params: null });
  assert.deepEqual(requests, [['42', 20, null, 1]]);
  validation.cancel();
  assert.equal(validation.accept({ requestId: 1, params: '{}' }), false);
  assert.deepEqual(accepted, []);
  assert.deepEqual(pending, [true, false]);
});

test('only the newest validation response is accepted', () => {
  const accepted = [];
  const validation = createRunValidation({
    getSim: () => ({ validateRun() {} }),
    onPendingChange() {},
    onAccepted: (run) => accepted.push(run),
    onRejected: (error) => assert.fail(error),
  });

  validation.request({ seed: '1', founders: 10, params: null });
  validation.request({ seed: '2', founders: 20, params: null });
  assert.equal(validation.accept({ requestId: 1, params: '{}' }), false);
  assert.equal(validation.accept({ requestId: 2, params: '{"world":{}}' }), true);
  assert.equal(accepted[0].seed, '2');
  assert.equal(accepted[0].params, '{"world":{}}');
});

test('rejections include the request source', () => {
  const rejected = [];
  const validation = createRunValidation({
    getSim: () => ({ validateRun() {} }),
    onPendingChange() {},
    onAccepted: (run) => assert.fail(run),
    onRejected: (error, run) => rejected.push({ error, source: run.source }),
  });

  validation.request({ seed: '1', founders: 6000, params: null, source: 'reseed' });
  assert.equal(validation.accept({ requestId: 1, error: 'too many founders' }), true);
  assert.deepEqual(rejected, [{ error: 'too many founders', source: 'reseed' }]);
});
