/** Wheel delta-mode normalization tests. */

import assert from 'node:assert/strict';
import test from 'node:test';

import { wheelZoomFactor } from './wheel.js';

test('pixel, line, and page deltas normalize to the same unit', () => {
  const pixel = wheelZoomFactor({ deltaY: 16, deltaMode: 0 });
  const line = wheelZoomFactor({ deltaY: 1, deltaMode: 1 });
  assert.equal(line, pixel);

  const page = wheelZoomFactor({
    deltaY: 1,
    deltaMode: 2,
    currentTarget: { clientHeight: 500 },
  });
  assert.equal(page, wheelZoomFactor({ deltaY: 500, deltaMode: 0 }));
});
