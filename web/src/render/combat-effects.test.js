/** Combat animations: which events are new, how each one moves, and when it ends. */

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  HIT_MS, KILL_MS, OVERLAY_STRIDE, SHAPE, SWING_MS, createCombatEffects,
} from './combat-effects.js';

const NEVER = 255;

/** A frame of `capacity` empty slots facing +x, and `corpses` empty corpse slots. */
function frame(capacity, corpses = 1) {
  const views = {
    alive: new Uint8Array(capacity),
    incarnation: new Uint32Array(capacity),
    position: new Float32Array(capacity * 3),
    size: new Float32Array(capacity).fill(3),
    orientation: new Float32Array(capacity * 4),
    swingAge: new Uint8Array(capacity).fill(NEVER),
    hurtAge: new Uint8Array(capacity).fill(NEVER),
    biteAt: new Float32Array(capacity * 2).fill(NaN),
    health: new Uint8Array(capacity).fill(255),
    corpsePosition: new Float32Array(corpses * 3),
    corpseEnergy: new Float32Array(corpses),
  };
  for (let i = 0; i < capacity; i++) views.orientation[i * 4 + 3] = 1;
  return views;
}

function place(views, slot, x, y) {
  views.alive[slot] = 1;
  views.position[slot * 3] = x;
  views.position[slot * 3 + 1] = y;
}

const effects = (capacity, worldSize = 100) =>
  createCombatEffects({ capacity, corpseCapacity: 1, worldSize });
const colors = (capacity) => new Float32Array(capacity * 3).fill(0.5);
const near = (actual, expected) => Math.abs(actual - expected) < 1e-4;

/** The overlay's shapes at `now`, as plain objects. */
function shapes(fx, views, now) {
  const { data, count } = fx.overlay(views, now, Math.PI / 4, 4, 2);
  return Array.from({ length: count }, (_, k) => {
    const o = k * OVERLAY_STRIDE;
    return {
      x: data[o], y: data[o + 1], radius: data[o + 2], angle: data[o + 3],
      color: [data[o + 4], data[o + 5], data[o + 6]], alpha: data[o + 7], kind: data[o + 8],
    };
  });
}

test('the first frame sets the baseline without replaying its events', () => {
  const fx = effects(1);
  const views = frame(1);
  place(views, 0, 10, 10);
  views.swingAge[0] = 0;
  views.hurtAge[0] = 0;
  fx.observe(views, 10n, 0, colors(1), 2);
  const { offsets, flashes } = fx.agents(views, 10);
  assert.deepEqual([...offsets], [0, 0]);
  assert.deepEqual([...flashes], [0]);
  assert.equal(shapes(fx, views, 10).length, 0);
});

test('a swing newer than the last frame lunges ahead, then settles', () => {
  const fx = effects(2);
  const views = frame(2);
  place(views, 0, 10, 10);
  place(views, 1, 50, 50);
  fx.observe(views, 10n, 0, colors(2), 2);
  // Five ticks pass: slot 0 swung two ticks ago, and slot 1 five, during the step the
  // previous frame showed, so it is not new.
  views.swingAge[0] = 2;
  views.swingAge[1] = 5;
  fx.observe(views, 15n, 100, colors(2), 2);
  const mid = fx.agents(views, 100 + SWING_MS / 2).offsets;
  assert.ok(near(mid[0], 0.45 * 3), `lunged ${mid[0]} along +x`);
  assert.ok(near(mid[1], 0));
  assert.deepEqual([mid[2], mid[3]], [0, 0], 'an old swing is not replayed');
  const after = fx.agents(views, 100 + SWING_MS).offsets;
  assert.deepEqual([after[0], after[1]], [0, 0]);
  const arcs = shapes(fx, views, 100);
  assert.deepEqual(arcs.map((s) => s.kind), [SHAPE.WEDGE], 'a miss draws only its arc');
  assert.ok(near(arcs[0].radius, 3 + 4), 'the arc reaches past the body');
});

test('after a long gap a saturated age is never new, but a younger one still is', () => {
  // At high speed or in a throttled tab a frame can skip more ticks than an age holds.
  const fx = effects(2);
  const views = frame(2);
  place(views, 0, 10, 10);
  place(views, 1, 50, 50);
  fx.observe(views, 10n, 0, colors(2), 2);
  views.swingAge[1] = 200;
  fx.observe(views, 1_010n, 100, colors(2), 2);
  const { offsets, flashes } = fx.agents(views, 100 + SWING_MS / 2);
  assert.deepEqual([offsets[0], offsets[1], flashes[0], flashes[1]], [0, 0, 0, 0],
    'an agent that never swung or was hit stays still');
  assert.ok(offsets[2] !== 0, 'one that swung 200 ticks ago, unseen, still lunges');
});

test('a hit flashes and shakes its victim, fading over its time', () => {
  const fx = effects(1);
  const views = frame(1);
  place(views, 0, 10, 10);
  fx.observe(views, 10n, 0, colors(1), 2);
  views.hurtAge[0] = 0;
  fx.observe(views, 11n, 100, colors(1), 2);
  assert.equal(fx.agents(views, 100).flashes[0], 1);
  assert.ok(near(fx.agents(views, 100 + HIT_MS / 2).flashes[0], 0.5));
  const settled = fx.agents(views, 100 + HIT_MS);
  assert.equal(settled.flashes[0], 0);
  assert.deepEqual([...settled.offsets], [0, 0]);
});

test('missing health shows as a wound until it heals', () => {
  const fx = effects(2);
  const views = frame(2);
  place(views, 0, 10, 10);
  views.health[0] = 191;
  views.health[1] = 0;
  const { wounds } = fx.agents(views, 0);
  assert.ok(near(wounds[0], 64 / 255));
  assert.equal(wounds[1], 0, 'an empty slot has no wound');
});

test('a landed swing draws a line to its victim and specks at the mouth', () => {
  const fx = effects(1);
  const views = frame(1);
  place(views, 0, 10, 10);
  fx.observe(views, 10n, 0, colors(1), 2);
  views.swingAge[0] = 0;
  views.biteAt[0] = 10;
  views.biteAt[1] = 16;
  fx.observe(views, 11n, 100, colors(1), 2);
  const drawn = shapes(fx, views, 100);
  assert.deepEqual(
    drawn.map((s) => s.kind),
    [SHAPE.WEDGE, SHAPE.SEGMENT, ...Array(5).fill(SHAPE.DISC)],
  );
  const line = drawn[1];
  assert.ok(near(line.radius, 6) && near(line.angle, Math.PI / 2), 'the line runs to the victim');
  // The lunge turns toward the victim, not the heading.
  const lunge = fx.agents(views, 100 + SWING_MS / 2).offsets;
  assert.ok(near(lunge[0], 0) && lunge[1] > 0);
  assert.equal(shapes(fx, views, 100 + HIT_MS).length, 0);
});

test('a landed bite outlasts its arc: the line and specks run for a hit\'s time', () => {
  const fx = effects(1);
  const views = frame(1);
  place(views, 0, 10, 10);
  fx.observe(views, 10n, 0, colors(1), 2);
  views.swingAge[0] = 0;
  views.biteAt[0] = 10;
  views.biteAt[1] = 16;
  fx.observe(views, 11n, 100, colors(1), 2);
  const late = shapes(fx, views, 100 + (SWING_MS + HIT_MS) / 2).map((s) => s.kind);
  assert.ok(!late.includes(SHAPE.WEDGE), 'the arc has gone');
  assert.ok(late.includes(SHAPE.SEGMENT), 'the line is still there');
});

test('a newborn in a reused slot does not inherit its predecessor\'s animation', () => {
  const fx = effects(1);
  const views = frame(1);
  place(views, 0, 10, 10);
  views.incarnation[0] = 1;
  fx.observe(views, 10n, 0, colors(1), 2);
  views.swingAge[0] = 0;
  views.hurtAge[0] = 0;
  views.biteAt[0] = 10;
  views.biteAt[1] = 16;
  fx.observe(views, 11n, 100, colors(1), 2);
  // The next tick the slot holds a newborn that has done nothing yet.
  views.incarnation[0] = 2;
  views.swingAge[0] = 255;
  views.hurtAge[0] = 255;
  views.biteAt.fill(NaN);
  fx.observe(views, 12n, 120, colors(1), 2);
  const { offsets, flashes } = fx.agents(views, 150);
  assert.deepEqual([offsets[0], offsets[1], flashes[0]], [0, 0, 0]);
  assert.equal(shapes(fx, views, 150).length, 0);
});

test('a hit across the seam is drawn the short way', () => {
  const fx = effects(1, 100);
  const views = frame(1);
  place(views, 0, 99, 50);
  fx.observe(views, 10n, 0, colors(1), 2);
  views.swingAge[0] = 0;
  views.biteAt[0] = 1;
  views.biteAt[1] = 50;
  fx.observe(views, 11n, 100, colors(1), 2);
  const line = shapes(fx, views, 100)[1];
  assert.ok(near(line.radius, 2) && near(line.angle, 0), `${line.radius} at ${line.angle}`);
});

test('a kill shrinks the vanished body into its corpse inside a spreading ring', () => {
  const fx = effects(1);
  const views = frame(1);
  place(views, 0, 20, 20);
  const shown = new Float32Array([0.9, 0.1, 0.2]);
  fx.observe(views, 10n, 0, shown, 2);
  views.alive[0] = 0;
  views.corpsePosition.set([20, 20, 0]);
  views.corpseEnergy[0] = 40;
  fx.observe(views, 11n, 100, shown, 2);
  const [body, ring] = shapes(fx, views, 100);
  assert.equal(body.kind, SHAPE.DISC);
  assert.ok(near(body.radius, 3) && body.alpha === 1);
  assert.deepEqual(body.color.map((c) => Math.round(c * 10) / 10), [0.9, 0.1, 0.2]);
  assert.equal(ring.kind, SHAPE.RING);
  assert.ok(near(ring.x, 20) && near(ring.y, 20));
  // Half way, the body has become the corpse and only the ring remains.
  assert.deepEqual(shapes(fx, views, 100 + KILL_MS / 2).map((s) => s.kind), [SHAPE.RING]);
  assert.equal(shapes(fx, views, 100 + KILL_MS).length, 0);
  // A corpse already lying there is not a new kill.
  fx.observe(views, 12n, 2_000, shown, 2);
  assert.equal(shapes(fx, views, 2_000).length, 0);
});

test('a corpse slot refilled between frames is a new kill, by place or by energy', () => {
  const fx = effects(1);
  const views = frame(1);
  views.corpsePosition.set([20, 20, 0]);
  views.corpseEnergy[0] = 40;
  fx.observe(views, 10n, 0, colors(1), 2);
  const rings = (now) => shapes(fx, views, now).filter((s) => s.kind === SHAPE.RING).length;
  // Eaten a little: the same corpse.
  views.corpseEnergy[0] = 35;
  fx.observe(views, 11n, 100, colors(1), 2);
  assert.equal(rings(100), 0, 'a corpse losing energy is not a kill');
  // Freed and refilled by a death elsewhere within one frame gap.
  views.corpsePosition.set([60, 20, 0]);
  views.corpseEnergy[0] = 30;
  fx.observe(views, 12n, 1_000, colors(1), 2);
  assert.equal(rings(1_000), 1, 'a corpse that moved');
  // Refilled where it lay, by a death holding more.
  views.corpseEnergy[0] = 50;
  fx.observe(views, 13n, 2_000, colors(1), 2);
  assert.equal(rings(2_000), 1, 'a corpse that gained');
});

test('a world that starts over forgets the old one\'s animations', () => {
  const fx = effects(1);
  const views = frame(1);
  place(views, 0, 10, 10);
  fx.observe(views, 10n, 0, colors(1), 2);
  views.hurtAge[0] = 0;
  fx.observe(views, 11n, 100, colors(1), 2);
  assert.equal(fx.agents(views, 100).flashes[0], 1);
  fx.observe(views, 3n, 110, colors(1), 2);
  assert.equal(fx.agents(views, 110).flashes[0], 0);
});
