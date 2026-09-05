/**
 * Regression tests for the snapshot transport's alignment, ownership, and metadata
 * contract. These use real SharedArrayBuffers but no browser APIs, so CI can exercise
 * the concurrency protocol directly under Node.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { frameLayout, frameViews } from './snapshot-layout.js';
import { SHARED, TRANSFERABLE, createReader, createWriter } from './transport.js';

function source(capacity, plantCapacity, marker) {
  const layout = frameLayout(capacity, plantCapacity);
  const views = frameViews(new ArrayBuffer(layout.bytes), 0, layout);
  for (const field of Object.values(views)) field.fill(marker);
  return views;
}

test('shared frame bases stay aligned for every capacity residue', () => {
  assert.equal(frameLayout(1, 0).bytes, 61);
  for (const capacity of [5000, 5001, 5002, 5003]) {
    assert.doesNotThrow(() => createWriter(SHARED, capacity, 7), `${capacity} slots`);
  }
});

test('a leased shared frame stays immutable while newer frames publish', () => {
  const writer = createWriter(SHARED, 5, 2);
  const reader = createReader(writer.handoff);

  assert.equal(writer.publish(source(5, 2, 1), 11n, 3), true);
  const first = reader.latest();
  assert.equal(first.fresh, true);
  assert.equal(first.tick, 11n);
  assert.equal(first.population, 3);
  assert.equal(first.views.position[0], 1);
  assert.equal(first.views.incarnation[0], 1);

  writer.publish(source(5, 2, 2), 12n, 4);
  writer.publish(source(5, 2, 3), 13n, 5);
  assert.equal(first.views.position[0], 1, 'writer overwrote a frame still leased by the reader');

  const latest = reader.latest();
  assert.equal(latest.fresh, true);
  assert.equal(latest.tick, 13n);
  assert.equal(latest.population, 5);
  assert.equal(latest.views.position[0], 3);

  const repeated = reader.latest();
  assert.equal(repeated.fresh, false);
  assert.equal(repeated.tick, 13n);
  reader.release();
});

test('transferable frames stay attached until the next animation read', () => {
  const reader = createReader({ kind: TRANSFERABLE, capacity: 2, plantCapacity: 0 });
  const layout = frameLayout(2, 0);
  const firstBuffer = new ArrayBuffer(layout.bytes);
  const secondBuffer = new ArrayBuffer(layout.bytes);
  frameViews(firstBuffer, 0, layout).alive[0] = 1;
  frameViews(secondBuffer, 0, layout).alive[1] = 1;

  reader.accept({ buffer: firstBuffer, tick: '1', population: 1 });
  const first = reader.latest();
  reader.accept({ buffer: secondBuffer, tick: '2', population: 1 });

  assert.equal(first.views.alive[0], 1, 'accept invalidated the frame held by picking');
  assert.equal(reader.takeRecycle(), null);

  const second = reader.latest();
  assert.equal(second.views.alive[1], 1);
  assert.equal(reader.takeRecycle(), firstBuffer);
});
