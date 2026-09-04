/** Toroidal agent-picking regressions. */

import assert from 'node:assert/strict';
import test from 'node:test';

import { pickAgent } from './picking.js';

function views() {
  return {
    alive: new Uint8Array([1, 0, 1]),
    position: new Float32Array([998, 500, 0, 500, 500, 0, 4, 500, 0]),
    size: new Float32Array([3, 100, 3]),
  };
}

test('picking follows the renderer across the torus seam', () => {
  assert.equal(pickAgent(views(), 3, { x: 1, y: 500 }, 1000), 0);
  assert.equal(pickAgent(views(), 3, { x: 5, y: 500 }, 1000), 2);
});

test('dead slots and points outside the displayed radius are ignored', () => {
  assert.equal(pickAgent(views(), 3, { x: 500, y: 500 }, 1000), null);
  assert.equal(pickAgent(views(), 3, { x: 20, y: 500 }, 1000), null);
});

test('minimum display radius remains clickable when zoomed out', () => {
  const tiny = views();
  tiny.position[6] = 100;
  tiny.size[2] = 0.1;
  assert.equal(pickAgent(tiny, 3, { x: 103, y: 500 }, 1000, 4), 2);
});
