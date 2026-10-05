import assert from 'node:assert/strict';
import test from 'node:test';
import { displayColors, speciesSwatch, validateSpeciesView } from './species-colors.js';
import { NULL_SPECIES } from '../species/model.ts';

test('display colors never mutate snapshots and default mode retains the signature view', () => {
  const views = {
    species: new Uint32Array([0, 1, 0, NULL_SPECIES]),
    signature: new Float32Array([0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1, 0, 0]),
  };
  const before = structuredClone(views);
  const scratch = new Float32Array(12);
  assert.equal(displayColors(views, 4, {
    colorMode: 'signature', selectedSpecies: null,
  }, scratch), views.signature);
  const result = displayColors(views, 4, { colorMode: 'species', selectedSpecies: null }, scratch);
  assert.equal(result, scratch);
  assert.deepEqual(result.slice(0, 3), result.slice(6, 9));
  assert.notDeepEqual(result.slice(0, 3), result.slice(3, 6));
  assert.equal(result[9], result[10]);
  assert.equal(result[10], result[11]);
  assert.deepEqual(views, before);
  for (const value of result) assert.ok(value >= 0 && value <= 1);
  displayColors(views, 4, { colorMode: 'signature', selectedSpecies: 1 }, scratch);
  assert.deepEqual(scratch.slice(3, 6), views.signature.slice(3, 6));
  assert.ok(scratch[0] > 0 && scratch[0] < views.signature[0]);
  assert.deepEqual(views, before);
});

test('palette handles full-width IDs and exposes matching readable legend swatches', () => {
  assert.equal(speciesSwatch(0), 'rgb(242 77 77)');
  assert.equal(speciesSwatch(NULL_SPECIES), 'rgb(140 140 140)');
  assert.notEqual(speciesSwatch(0), speciesSwatch(0x80000000));
  assert.notEqual(speciesSwatch(0), speciesSwatch(NULL_SPECIES - 1));
  assert.throws(() => speciesSwatch(null));
  assert.throws(() => validateSpeciesView({ colorMode: 'invalid', selectedSpecies: null }));
  assert.throws(() => validateSpeciesView({ colorMode: 'species', selectedSpecies: -1 }));
});

test('diet colours run from plants to meat and grey out the unfed and replaced', () => {
  const views = {
    species: new Uint32Array(4),
    signature: new Float32Array(12),
    incarnation: new Uint32Array([1, 1, 1, 2]),
  };
  const diets = {
    shares: new Uint8Array([0, 254, 255, 127]),
    incarnation: new Uint32Array([1, 1, 1, 1]),
  };
  const out = displayColors(views, 4, { colorMode: 'diet', selectedSpecies: null },
    new Float32Array(12), diets);
  const rgb = (slot) => [...out.slice(slot * 3, slot * 3 + 3)].map((v) => Math.round(v * 100));
  assert.deepEqual(rgb(0), [30, 85, 35], 'all plants is green');
  assert.deepEqual(rgb(1), [95, 20, 15], 'all meat is red');
  assert.deepEqual(rgb(2), [55, 55, 55], 'the unfed are grey');
  assert.deepEqual(rgb(3), [55, 55, 55], 'a newborn in a reused slot is not its predecessor');
  const none = displayColors(views, 4, { colorMode: 'diet', selectedSpecies: null },
    new Float32Array(12));
  assert.ok([...none].every((v) => v === Math.fround(0.55)), 'no sample yet');
  assert.doesNotThrow(() => validateSpeciesView({ colorMode: 'diet', selectedSpecies: null }));
});
