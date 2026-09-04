/** Snapshot retry behavior for the transferable transport. */

import assert from 'node:assert/strict';
import test from 'node:test';

import { createSnapshotPublisher } from './publisher.js';
import { frameLayout, frameViews } from './snapshot-layout.js';
import { TRANSFERABLE, createWriter } from './transport.js';

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
});
