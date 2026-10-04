/** Run-level neural heredity validation regressions. */

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  EVOLVING,
  RANDOMIZED_AT_BIRTH,
  STRUCTURAL_NULL,
  keepsHistory,
  parseBrainInheritance,
} from './brain-inheritance.js';

test('omitted inheritance uses the evolving population', () => {
  assert.equal(parseBrainInheritance(), EVOLVING);
});

test('every supported inheritance mode round-trips', () => {
  assert.equal(parseBrainInheritance(EVOLVING), EVOLVING);
  assert.equal(parseBrainInheritance(RANDOMIZED_AT_BIRTH), RANDOMIZED_AT_BIRTH);
  assert.equal(parseBrainInheritance(STRUCTURAL_NULL), STRUCTURAL_NULL);
});

test('only the structural null is watch-only', () => {
  assert.equal(keepsHistory(EVOLVING), true);
  assert.equal(keepsHistory(RANDOMIZED_AT_BIRTH), true);
  assert.equal(keepsHistory(STRUCTURAL_NULL), false);
});

test('unknown inheritance modes fail loudly', () => {
  assert.throws(() => parseBrainInheritance('random'), /inheritance/);
});
