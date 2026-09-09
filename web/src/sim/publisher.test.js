/** Snapshot retry behavior for the transferable transport. */

import assert from 'node:assert/strict';
import test from 'node:test';

import { createSnapshotPublisher } from './publisher.js';
import { frameLayout, frameViews } from './snapshot-layout.js';
import { SHARED, TRANSFERABLE, createWriter } from './transport.js';

test('a dropped forced frame is retried when a transferable buffer returns', () => {
  const layout = frameLayout(2, 0);
  const source = frameViews(new ArrayBuffer(layout.bytes), 0, layout);
  const writer = createWriter(TRANSFERABLE, 2, 0);
  const sent = [];
  let tick = 1n;
  const publisher = createSnapshotPublisher({
    writer,
    source: () => source,
    tick: () => tick,
    population: () => 2,
    descendants: () => 1,
    speciesCount: () => 1,
    unclassifiedPopulation: () => 1,
    meanEnergy: () => Number(tick),
    send: (message) => sent.push(message),
    now: () => 0,
  });

  publisher.publish();
  tick = 2n;
  publisher.publish();
  tick = 3n;
  assert.equal(publisher.publish(true), false);
  assert.equal(sent.filter((message) => message.kind === TRANSFERABLE).length, 2);
  assert.equal(sent.filter((message) => message.kind === 'metrics').at(-1).tick, '1');

  const returned = sent.find((message) => message.kind === TRANSFERABLE).buffer;
  publisher.recycle(returned);
  const frames = sent.filter((message) => message.kind === TRANSFERABLE);
  assert.equal(frames.at(-1).tick, '3');
  assert.equal(sent.filter((message) => message.kind === 'metrics').at(-1).tick, '3');
  assert.equal(sent.filter((message) => message.kind === 'metrics').at(-1).descendants, 1);
  assert.equal(sent.filter((message) => message.kind === 'metrics').at(-1).speciesCount, 1);
  assert.equal(sent.filter((message) => message.kind === 'metrics').at(-1).unclassifiedPopulation, 1);
});

test('shared publication retries after a transient lease swap', () => {
  const retries = [];
  const sent = [];
  let attempts = 0;
  const publisher = createSnapshotPublisher({
    writer: {
      kind: SHARED,
      publish() {
        attempts += 1;
        return attempts === 1 ? false : true;
      },
    },
    source: () => ({}),
    tick: () => 9n,
    population: () => 3,
    descendants: () => 2,
    speciesCount: () => 0,
    unclassifiedPopulation: () => 3,
    meanEnergy: () => 40,
    send: (message) => sent.push(message),
    schedule: (callback) => retries.push(callback),
    now: () => 0,
  });

  assert.equal(publisher.publish(true), false);
  assert.equal(retries.length, 1);
  retries.shift()();
  assert.equal(attempts, 2);
  assert.equal(sent.at(-1).kind, 'metrics');
  assert.equal(sent.at(-1).tick, '9');
  assert.equal(sent.at(-1).descendants, 2);
  assert.equal(sent.at(-1).speciesCount, 0);
  assert.equal(sent.at(-1).unclassifiedPopulation, 3);
});

for (const kind of [SHARED, TRANSFERABLE]) {
  test(`species counts are sampled only with lightweight metrics (${kind})`, () => {
    const layout = frameLayout(2, 0);
    const source = frameViews(new ArrayBuffer(layout.bytes), 0, layout);
    const sent = [];
    let sampledAt = 0;
    let speciesReads = 0;
    let unclassifiedReads = 0;
    const publisher = createSnapshotPublisher({
      writer: createWriter(kind, 2, 0),
      source: () => source,
      tick: () => 1n,
      population: () => 2,
      descendants: () => 0,
      speciesCount: () => { speciesReads += 1; return 1; },
      unclassifiedPopulation: () => { unclassifiedReads += 1; return 0; },
      meanEnergy: () => 5,
      send: (message) => sent.push(message),
      now: () => sampledAt,
    });
    publisher.publish();
    publisher.publish();
    assert.equal(speciesReads, 1);
    assert.equal(unclassifiedReads, 1);
    if (kind === TRANSFERABLE) {
      publisher.recycle(sent.find((message) => message.kind === TRANSFERABLE).buffer);
    }
    sampledAt = 250;
    publisher.publish();
    assert.equal(speciesReads, 2);
    assert.equal(unclassifiedReads, 2);
    const metrics = sent.filter((message) => message.kind === 'metrics');
    assert.equal(metrics.length, 2);
    assert.deepEqual(Object.keys(metrics[0]).sort(), [
      'descendants', 'kind', 'meanEnergy', 'population', 'speciesCount', 'tick',
      'unclassifiedPopulation',
    ]);
  });
}
