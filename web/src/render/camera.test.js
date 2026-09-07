/** Camera coordinate regressions, including the screen/world y-axis flip. */

import assert from 'node:assert/strict';
import test from 'node:test';

import { createCamera } from './camera.js';

function canvas() {
  return {
    clientWidth: 1000,
    clientHeight: 500,
    width: 0,
    height: 0,
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 1000, height: 500 }),
  };
}

test('zooming preserves the world point under the cursor', () => {
  const camera = createCamera(canvas(), 1000, () => {});
  camera.fit();
  const before = camera.screenToWorld(750, 125);
  camera.zoomAt(750, 125, 2);
  const after = camera.screenToWorld(750, 125);
  assert.ok(Math.abs(before.x - after.x) < 1e-9);
  assert.ok(Math.abs(before.y - after.y) < 1e-9);
});

test('screen y is inverted into upward-growing world y', () => {
  const camera = createCamera(canvas(), 1000, () => {});
  camera.fit();
  assert.equal(camera.screenToWorld(500, 125).y, 750);
  assert.equal(camera.screenToWorld(500, 375).y, 250);
});

test('unwrapped picking coordinates preserve blank fitted-world margins', () => {
  const camera = createCamera(canvas(), 1000, () => {});
  camera.fit();
  assert.deepEqual(camera.screenToWorldUnwrapped(50, 250), { x: -400, y: 500 });
  assert.deepEqual(camera.screenToWorld(50, 250), { x: 600, y: 500 });
  assert.deepEqual(camera.screenToWorldUnwrapped(550, 250), { x: 600, y: 500 });
});
