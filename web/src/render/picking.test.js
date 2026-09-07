/** Toroidal agent-picking regressions. */

import assert from 'node:assert/strict';
import test from 'node:test';

import { pickAgent } from './picking.js';

const center = { x: 0, y: 500 };

function views() {
  return {
    alive: new Uint8Array([1, 0, 1]),
    incarnation: new Uint32Array([4, 9, 2]),
    position: new Float32Array([998, 500, 0, 500, 500, 0, 4, 500, 0]),
    size: new Float32Array([3, 100, 3]),
  };
}

test('picking follows the renderer across the torus seam', () => {
  assert.deepEqual(pickAgent(views(), 3, { x: 1, y: 500 }, 1000, center), {
    index: 0,
    incarnation: 4,
  });
  assert.deepEqual(pickAgent(views(), 3, { x: 5, y: 500 }, 1000, center), {
    index: 2,
    incarnation: 2,
  });
});

test('dead slots and points outside the displayed radius are ignored', () => {
  assert.equal(pickAgent(views(), 3, { x: 500, y: 500 }, 1000, center), null);
  assert.equal(pickAgent(views(), 3, { x: 20, y: 500 }, 1000, center), null);
});

test('minimum display radius remains clickable when zoomed out', () => {
  const tiny = views();
  tiny.position[6] = 100;
  tiny.size[2] = 0.1;
  assert.deepEqual(pickAgent(tiny, 3, { x: 103, y: 500 }, 1000, center, 4), {
    index: 2,
    incarnation: 2,
  });
});

test('only the camera-relative image is clickable, not copies in blank margins', () => {
  const frame = views();
  frame.alive[0] = 0;
  frame.position[6] = 600;
  const cameraCenter = { x: 500, y: 500 };
  assert.equal(pickAgent(frame, 3, { x: -400, y: 500 }, 1000, cameraCenter), null);
  assert.deepEqual(pickAgent(frame, 3, { x: 600, y: 500 }, 1000, cameraCenter), {
    index: 2,
    incarnation: 2,
  });
});

test('half-world ties choose the same negative-side image as the shaders', () => {
  const frame = views();
  frame.alive[0] = 0;
  frame.position[6] = 0;
  const cameraCenter = { x: 500, y: 500 };
  assert.deepEqual(pickAgent(frame, 3, { x: 0, y: 500 }, 1000, cameraCenter), {
    index: 2,
    incarnation: 2,
  });
  assert.equal(pickAgent(frame, 3, { x: 1000, y: 500 }, 1000, cameraCenter), null);
});
