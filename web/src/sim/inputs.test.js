/** Numeric worker-boundary regressions. */

import assert from 'node:assert/strict';
import test from 'node:test';

import { founderCount, parseSeed } from './inputs.js';

test('seed parsing preserves the full unsigned 64-bit range', () => {
  assert.equal(parseSeed('9007199254740992'), 9007199254740992n);
  assert.equal(parseSeed('9007199254740993'), 9007199254740993n);
  assert.equal(parseSeed('18446744073709551615'), 18446744073709551615n);
  assert.throws(() => parseSeed('18446744073709551616'), /u64 range/);
  assert.throws(() => parseSeed('-1'), /unsigned decimal/);
  assert.throws(() => parseSeed('1.5'), /unsigned decimal/);
});

test('founder counts cannot overflow the WASM u32 boundary or pool', () => {
  assert.equal(founderCount(1, 5000), 1);
  assert.equal(founderCount(5000, 5000), 5000);
  for (const count of [-1, 0, 5001, Number.NaN, Number.POSITIVE_INFINITY, 1.5]) {
    assert.throws(() => founderCount(count, 5000), /between 1 and 5000/);
  }
});
