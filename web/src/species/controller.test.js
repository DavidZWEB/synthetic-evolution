import assert from 'node:assert/strict';
import test from 'node:test';
import { createSpeciesController } from './controller.js';
import { NULL_SPECIES } from './model.ts';

const complexity = (population) => {
  const size = population === 0 ? 0 : 2;
  const distribution = { min: size, p25: size, median: size, p75: size, max: size, mean: size };
  return JSON.stringify({
    genome_genes: distribution, neurons: distribution,
    connections: distribution, enabled_connections: distribution,
  });
};

function fixture() {
  const calls = [];
  const changes = [];
  let sim = { requestSpecies: (id) => calls.push(id) };
  const controller = createSpeciesController({
    getSim: () => sim, onChange: (value) => changes.push(value), now: () => 0,
  });
  const reply = (tick, populations = [{ species_id: 0, population: 1 }], unclassified = 0, id = calls.at(-1)) =>
    controller.accept({
      requestId: id, tick: String(tick),
      population: populations.reduce((sum, row) => sum + row.population, unclassified),
      diagnostics: JSON.stringify({ populations, unclassified_population: unclassified }),
      complexity: complexity(populations.reduce((sum, row) => sum + row.population, unclassified)),
    });
  return { controller, calls, changes, reply, clearSim: () => { sim = null; } };
}

test('species polling is opt-in, bounded to one request, and catches paused steps after cooldown', () => {
  const f = fixture();
  f.controller.poll({ tick: 0n, fresh: true }, 0);
  assert.equal(f.calls.length, 0);
  f.controller.setOpen(true);
  f.controller.poll({ tick: 0n, fresh: true }, 0);
  f.reply(0);
  f.controller.poll({ tick: 0n, fresh: false }, 250);
  assert.equal(f.calls.length, 1, 'a completed sample already covers the frame');
  f.controller.poll({ tick: 1n, fresh: true }, 100);
  assert.equal(f.calls.length, 1);
  f.controller.poll({ tick: 1n, fresh: false }, 250);
  assert.equal(f.calls.length, 2);
  f.controller.poll({ tick: 2n, fresh: true }, 500);
  assert.equal(f.calls.length, 2);
  f.reply(1);
  f.controller.poll({ tick: 2n, fresh: false }, 501);
  assert.equal(f.calls.length, 3);
  f.reply(2);
  assert.equal(f.changes.at(-1).sample.tick, 2n);
});

test('selection is cleared when a species retires and never carried into a replacement world', () => {
  const f = fixture();
  f.controller.setOpen(true);
  f.reply(0);
  f.controller.select(0);
  f.controller.setOpen(false);
  f.controller.poll({ tick: 1n, fresh: true }, 250);
  assert.equal(f.calls.length, 2, 'closed panel still tracks a highlighted species');
  f.reply(1, [], 1);
  assert.equal(f.changes.at(-1).selected, null);
  assert.match(f.changes.at(-1).message, /no longer active/);
  f.controller.select(NULL_SPECIES);
  f.controller.poll({ tick: 2n, fresh: true }, 500);
  const oldRequest = f.calls.at(-1);
  f.controller.reset();
  assert.equal(f.reply(2, [], 1, oldRequest), false);
  assert.equal(f.changes.at(-1).sample, null);
  assert.equal(f.changes.at(-1).selected, null);
});

test('unclassified highlights toggle independently and clear when none remain', () => {
  const f = fixture();
  f.controller.setOpen(true);
  f.reply(0, [], 2);
  f.controller.select(NULL_SPECIES);
  assert.equal(f.changes.at(-1).selected, NULL_SPECIES);
  f.controller.select(NULL_SPECIES);
  assert.equal(f.changes.at(-1).selected, null);
  f.controller.select(NULL_SPECIES);
  f.controller.poll({ tick: 1n, fresh: true }, 250);
  f.reply(1);
  assert.equal(f.changes.at(-1).selected, null);
  assert.match(f.changes.at(-1).message, /No unclassified/);
});

test('errors are visible and stale/unsolicited replies cannot replace a sample', () => {
  const f = fixture();
  assert.equal(f.reply(0, [], 0, 9), false);
  f.controller.setOpen(true);
  f.controller.accept({ requestId: f.calls.at(-1), diagnostics: null, message: 'forced failure' });
  assert.match(f.changes.at(-1).message, /forced failure/);
  f.controller.setOpen(true);
  f.reply(5);
  f.controller.poll({ tick: 6n, fresh: true }, 250);
  f.reply(4);
  assert.match(f.changes.at(-1).message, /backwards/);
  assert.equal(f.changes.at(-1).sample.tick, 5n);
  f.clearSim();
  f.controller.reset();
  f.controller.poll({ tick: 0n, fresh: true }, 500);
  assert.equal(f.changes.at(-1).sample, null);
});
