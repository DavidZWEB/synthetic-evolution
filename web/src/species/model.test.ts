import assert from 'node:assert/strict';
import test from 'node:test';
import { decodeSpeciesSnapshot, NULL_SPECIES } from './model.ts';

function response(data = {
  populations: [{ species_id: 0, population: 2 }, { species_id: 4294967294, population: 1 }],
  unclassified_population: 1,
}) {
  return { tick: '9007199254740993', population: 4, diagnostics: JSON.stringify(data) };
}

test('species observations preserve exact ticks, sparse IDs, and unclassified membership', () => {
  assert.deepEqual(decodeSpeciesSnapshot(response()), {
    tick: 9007199254740993n, population: 4,
    populations: [{ id: 0, population: 2 }, { id: 4294967294, population: 1 }],
    unclassifiedPopulation: 1,
  });
  assert.deepEqual(decodeSpeciesSnapshot({
    tick: '0', population: 0,
    diagnostics: '{"populations":[],"unclassified_population":0}',
  }).populations, []);
  assert.equal(decodeSpeciesSnapshot({
    tick: '0', population: 4,
    diagnostics: '{"populations":[],"unclassified_population":4}',
  }).unclassifiedPopulation, 4);
});

test('malformed species identities, ordering, totals, and ticks are rejected', () => {
  for (const tick of ['', '-1', '01', '1.0', '0x1', '18446744073709551616']) {
    assert.throws(() => decodeSpeciesSnapshot({ ...response(), tick }));
  }
  for (const id of [-1, 0.5, NULL_SPECIES, NULL_SPECIES + 1, NaN]) {
    assert.throws(() => decodeSpeciesSnapshot(response({
      populations: [{ species_id: id, population: 3 }], unclassified_population: 1,
    })));
  }
  for (const populations of [
    [{ species_id: 0, population: 0 }],
    [{ species_id: 0, population: 1 }],
    [{ species_id: 0, population: 2 }, { species_id: 0, population: 1 }],
    [{ species_id: 2, population: 2 }, { species_id: 1, population: 1 }],
  ]) assert.throws(() => decodeSpeciesSnapshot(response({ populations, unclassified_population: 1 })));
  assert.throws(() => decodeSpeciesSnapshot({ ...response(), diagnostics: 'null' }));
  assert.throws(() => decodeSpeciesSnapshot({ ...response(), population: 3.5 }));
});
