/** Run-level neural heredity validation regressions. */

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  EVOLVING,
  RANDOMIZED_AT_BIRTH,
  parseBrainInheritance,
} from './brain-inheritance.js';

test('omitted inheritance uses the evolving population', () => {
  assert.equal(parseBrainInheritance(), EVOLVING);
});

test('both supported inheritance modes round-trip', () => {
  assert.equal(parseBrainInheritance(EVOLVING), EVOLVING);
  assert.equal(parseBrainInheritance(RANDOMIZED_AT_BIRTH), RANDOMIZED_AT_BIRTH);
});

test('unknown inheritance modes fail loudly', () => {
  assert.throws(() => parseBrainInheritance('random'), /inheritance/);
});
