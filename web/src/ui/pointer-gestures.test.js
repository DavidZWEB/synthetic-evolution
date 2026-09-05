/** Pointer gesture regressions for click-versus-drag and reactive pointer counts. */

import assert from 'node:assert/strict';
import test from 'node:test';

import { createPointerGestures } from './pointer-gestures.js';

const event = (x, y, pointerId = 7) => ({
  pointerId,
  clientX: x,
  clientY: y,
  currentTarget: { setPointerCapture() {} },
});

test('a stationary pointer selects while a drag only pans', () => {
  const counts = [];
  const clicks = [];
  const pans = [];
  const gestures = createPointerGestures({
    getRenderer: () => ({ panBy: (...delta) => pans.push(delta) }),
    onPointerCount: (count) => counts.push(count),
    onClick: (...point) => clicks.push(point),
  });

  gestures.down(event(10, 20));
  gestures.move(event(12, 22));
  gestures.up(event(10, 20));
  gestures.click(event(10, 20));
  assert.deepEqual(clicks, [[10, 20]]);
  assert.deepEqual(pans, [], 'movement inside the click threshold panned the camera');

  gestures.down(event(10, 20));
  gestures.move(event(20, 25));
  gestures.up(event(20, 25));
  gestures.click(event(20, 25));
  assert.deepEqual(pans, [[10, 5]]);
  assert.equal(clicks.length, 1);
  assert.deepEqual(counts, [1, 0, 1, 0]);
});

test('returning from three fingers re-baselines the remaining pinch pair', () => {
  const zooms = [];
  const gestures = createPointerGestures({
    getRenderer: () => ({ zoomAt: (...zoom) => zooms.push(zoom) }),
    onPointerCount() {},
    onClick() {},
  });

  gestures.down(event(0, 0, 1));
  gestures.down(event(10, 0, 2));
  gestures.down(event(100, 0, 3));
  gestures.move(event(20, 0, 2));
  assert.deepEqual(zooms, [], 'three fingers should suspend pinch zoom');

  gestures.up(event(0, 0, 1));
  gestures.move(event(28, 0, 2));
  assert.equal(zooms.length, 1);
  assert.ok(Math.abs(zooms[0][2] - 0.9) < 1e-9, `unexpected factor ${zooms[0][2]}`);
});
