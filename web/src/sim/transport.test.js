/**
 * Regression tests for the snapshot transport's alignment, ownership, and metadata
 * contract. These use real SharedArrayBuffers but no browser APIs, so CI can exercise
 * the concurrency protocol directly under Node.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { FRAME_STATE, frameLayout, frameViews } from './snapshot-layout.js';
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

test('shared acquisition records the generation actually leased after an ABA race', (t) => {
  const writer = createWriter(SHARED, 1, 0);
  const reader = createReader(writer.handoff);
  writer.publish(source(1, 0, 1), 1n, 1);
  assert.equal(reader.latest().tick, 1n);
  writer.publish(source(1, 0, 2), 2n, 1);

  const compareExchange = Atomics.compareExchange;
  let interleaved = false;
  let pauseReclamation = false;
  t.mock.method(Atomics, 'compareExchange', (array, index, expected, replacement) => {
    if (array.buffer === writer.handoff.buffer) {
      if (!interleaved && expected === FRAME_STATE.PUBLISHED && replacement === FRAME_STATE.READING) {
        interleaved = true;
        assert.equal(writer.publish(source(1, 0, 3), 3n, 1), true);
        // Reuse the discovered slot, then pause before reclaiming publication 3.
        // Its PUBLISHED -> FREE -> PUBLISHED cycle occurs before the reader's CAS.
        pauseReclamation = true;
        assert.equal(writer.publish(source(1, 0, 4), 4n, 1), true);
        pauseReclamation = false;
      } else if (pauseReclamation && expected === FRAME_STATE.PUBLISHED && replacement === FRAME_STATE.FREE) {
        return Atomics.load(array, index);
      }
    }
    return compareExchange(array, index, expected, replacement);
  });

  const acquired = reader.latest();
  assert.equal(interleaved, true, 'did not exercise the acquisition race');
  assert.equal(acquired.tick, 4n);
  assert.equal(acquired.views.position[0], 4);
  const repeated = reader.latest();
  assert.equal(repeated.tick, 4n, 'regressed to the older publication still awaiting reclamation');
  assert.equal(repeated.fresh, false);
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
