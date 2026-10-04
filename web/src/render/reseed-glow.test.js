/** Reseed highlighting: which plant slots moved, and how long they glow. */

import assert from 'node:assert/strict';
import test from 'node:test';

import { createReseedGlow } from './reseed-glow.js';

const sites = (...xy) => new Float32Array(xy.flatMap(([x, y]) => [x, y, 0]));

test('a new world sets the baseline without glowing', () => {
  const glow = createReseedGlow(2, 1000);
  glow.observe(sites([1, 1], [5, 5]), 0);
  assert.deepEqual([...glow.values(0)], [0, 0]);
});

test('a moved slot glows and then fades; a still one never does', () => {
  const glow = createReseedGlow(2, 1000);
  glow.observe(sites([1, 1], [5, 5]), 0);
  glow.observe(sites([1, 1], [9, 2]), 100);
  assert.deepEqual([...glow.values(100)], [0, 1]);
  assert.deepEqual([...glow.values(600)], [0, 0.5]);
  assert.deepEqual([...glow.values(1100)], [0, 0]);
});

test('a later snapshot without movement keeps the earlier reseed fading', () => {
  const glow = createReseedGlow(1, 1000);
  glow.observe(sites([1, 1]), 0);
  glow.observe(sites([2, 1]), 0);
  glow.observe(sites([2, 1]), 500);
  assert.deepEqual([...glow.values(500)], [0.5]);
});
