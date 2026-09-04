/** Pointer gesture regressions for click-versus-drag and reactive pointer counts. */

import assert from 'node:assert/strict';
import test from 'node:test';

import { createPointerGestures } from './pointer-gestures.js';

const event = (x, y) => ({
  pointerId: 7,
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
  gestures.up(event(10, 20));
  gestures.click(event(10, 20));
  assert.deepEqual(clicks, [[10, 20]]);

  gestures.down(event(10, 20));
  gestures.move(event(20, 25));
  gestures.up(event(20, 25));
  gestures.click(event(20, 25));
  assert.deepEqual(pans, [[10, 5]]);
  assert.equal(clicks.length, 1);
  assert.deepEqual(counts, [1, 0, 1, 0]);
});
