/** Inspector boundary parsing and generated-genome integration tests. */

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  birthIdLabel, decodeInspection, dietLabel, parentBirthLabel, parentSlotLabel, speciesLabel,
  summarizeGenes,
} from './model.ts';

const inspection = {
  index: 3,
  incarnation: 7,
  tick: '9007199254740993',
  energy: 71.5,
  age: 12,
  size: 3,
  signature: [0.1, 0.2, 0.3],
  species_id: 0,
  birth_id: '9007199254740993',
  parent_birth_a: '9007199254740992',
  parent_birth_b: null,
  parent_a: 1,
  parent_b: 4294967295,
  brain_units: 10,
  sensor_load: 4,
  health: 0.75,
  muscle: 1.5,
  mouth: 0.5,
  eaten_plants: 30,
  eaten_animals: 10,
  kills: 2,
  activations: [0.25, -0.5],
  genome: [
    { Neuron: { id: 1, bias: 0, tau: 1, activation: 'Tanh', period: 0 } },
    { Neuron: { id: 2, bias: 0, tau: 1, activation: 'Sigmoid', period: 0 } },
    { Body: { trait_: 'Size', value: 3 } },
  ],
};

test('inspection JSON is validated and decoded', () => {
  const decoded = decodeInspection(JSON.stringify(inspection));
  assert.equal(decoded.index, 3);
  assert.deepEqual(summarizeGenes(decoded.genome), [
    { kind: 'Neuron', count: 2 },
    { kind: 'Body', count: 1 },
  ]);
});

test('malformed inspection JSON is rejected', () => {
  assert.throws(
    () => decodeInspection(JSON.stringify({ ...inspection, activations: ['not a number'] })),
    /invalid inspection payload/,
  );
});

test('combat fields are validated: health in (0, 1], diets non-negative, kills a count', () => {
  const decoded = decodeInspection(JSON.stringify(inspection));
  assert.deepEqual(
    [decoded.health, decoded.muscle, decoded.mouth, decoded.kills],
    [0.75, 1.5, 0.5, 2],
  );
  const cases: Array<[string, unknown]> = [
    ['health', 0], ['health', 1.5], ['health', null], ['muscle', 'strong'], ['mouth', null],
    ['eaten_plants', -1], ['eaten_animals', -0.5], ['eaten_animals', null],
    ['kills', -1], ['kills', 1.5], ['kills', 4294967296],
  ];
  for (const [field, value] of cases) {
    assert.throws(
      () => decodeInspection(JSON.stringify({ ...inspection, [field]: value })),
      /invalid inspection payload/,
      `accepted ${field} = ${JSON.stringify(value)}`,
    );
  }
  const missing: Record<string, unknown> = { ...inspection };
  delete missing.kills;
  assert.throws(() => decodeInspection(JSON.stringify(missing)), /invalid inspection payload/);
});

test('a diet reads as its meat and plant shares, or unfed', () => {
  assert.equal(dietLabel(30, 10), '25% meat, 75% plants');
  assert.equal(dietLabel(0, 4), '100% meat, 0% plants');
  assert.equal(dietLabel(0, 0), 'unfed');
  assert.equal(dietLabel(Number.MAX_VALUE, Number.MAX_VALUE), '50% meat, 50% plants');
  assert.equal(dietLabel(Number.MIN_VALUE, Number.MIN_VALUE), '50% meat, 50% plants');
});

test('birth identities preserve exact decimal strings above the safe integer range', () => {
  for (const id of [null, '0', '9007199254740993', '18446744073709551614']) {
    const decoded = decodeInspection(JSON.stringify({
      ...inspection, birth_id: id, parent_birth_a: id, parent_birth_b: id,
    }));
    assert.equal(decoded.birth_id, id);
    assert.equal(decoded.parent_birth_a, id);
    assert.equal(decoded.parent_birth_b, id);
    assert.equal(birthIdLabel(decoded.birth_id), id === null ? 'unavailable' : `#${id} (this world)`);
  }
});

test('all birth identity fields reject missing, numeric, noncanonical and out-of-range values', () => {
  const malformed = [
    undefined, 0, 9007199254740992, -1, true, {}, [], '', '00', '01', '-1', '-0',
    '+1', ' 1', '1 ', '1\n', '1.0', '1e3', '0x10', '１', 'null',
    '18446744073709551615', '18446744073709551616', '100000000000000000000',
  ];
  for (const field of ['birth_id', 'parent_birth_a', 'parent_birth_b']) {
    for (const value of malformed) {
      assert.throws(
        () => decodeInspection(JSON.stringify({ ...inspection, [field]: value })),
        /invalid inspection payload/,
        `${field} accepted ${JSON.stringify(value)}`,
      );
    }
  }
});

test('parent labels distinguish absent parents from unavailable persistent identities', () => {
  const nullSlot = 4294967295;
  assert.equal(birthIdLabel(null), 'unavailable');
  assert.equal(parentBirthLabel(null, nullSlot, 'A'), '— (founder)');
  assert.equal(parentBirthLabel(null, nullSlot, 'B'), '— (asexual)');
  for (const parent of ['A', 'B'] as const) {
    assert.equal(parentBirthLabel(null, 0, parent), 'unavailable');
    assert.equal(parentBirthLabel('0', 17, parent), '#0 (this world)');
    assert.equal(parentBirthLabel('0', nullSlot, parent), '#0 (this world)');
    assert.equal(parentSlotLabel(17, parent), 'slot #17 at birth (may be reused)');
  }
});

test('two persistent parents are independent of recycled legacy slots', () => {
  const decoded = decodeInspection(JSON.stringify({
    ...inspection,
    birth_id: '18446744073709551614',
    parent_birth_a: '9007199254740993',
    parent_birth_b: '9007199254740994',
    parent_a: 3,
    parent_b: 3,
  }));
  assert.equal(
    parentBirthLabel(decoded.parent_birth_a, decoded.parent_a, 'A'),
    '#9007199254740993 (this world)',
  );
  assert.equal(
    parentBirthLabel(decoded.parent_birth_b, decoded.parent_b, 'B'),
    '#9007199254740994 (this world)',
  );
  assert.deepEqual(decoded.genome, inspection.genome);
  assert.deepEqual(decoded.activations, inspection.activations);
});

test('parent status rejects invalid legacy slot values rather than labeling them unknown', () => {
  for (const slot of [-1, 4294967296, 0.5, NaN]) {
    for (const field of ['parent_a', 'parent_b']) {
      assert.throws(
        () => decodeInspection(JSON.stringify({ ...inspection, [field]: slot })),
        /invalid inspection payload/,
      );
    }
    assert.throws(() => parentSlotLabel(slot, 'A'), /invalid parent slot/);
    assert.throws(() => parentBirthLabel('0', slot, 'A'), /invalid parent slot/);
  }
});

test('species labels distinguish world-local IDs from the unclassified sentinel', () => {
  for (const id of [0, 8, 4294967294]) {
    const decoded = decodeInspection(JSON.stringify({ ...inspection, species_id: id }));
    assert.equal(speciesLabel(decoded.species_id), `#${id} (this world)`);
  }
  const unclassified = decodeInspection(JSON.stringify({ ...inspection, species_id: 4294967295 }));
  assert.equal(speciesLabel(unclassified.species_id), 'unclassified');
  for (const id of [-1, 4294967296, 0.5]) {
    assert.throws(
      () => decodeInspection(JSON.stringify({ ...inspection, species_id: id })),
      /invalid inspection payload/,
    );
    assert.throws(() => speciesLabel(id), /invalid species ID/);
  }
});

test('sensor-born brains decode with a different activation and genome length', () => {
  const child = {
    ...inspection,
    index: 4,
    birth_id: '9007199254740994',
    parent_birth_a: inspection.birth_id,
    parent_a: inspection.index,
    activations: [...inspection.activations, 0.2, 0.3, 0.4],
    genome: [
      ...inspection.genome,
      ...[3, 4, 5].map((id) => ({
        Neuron: { id, bias: 0, tau: 1, activation: 'Sigmoid', period: 0 },
      })),
      { Sensor: { id: 6, modality: 'Chemo', params: [0, 20, 0, 0], targets: [3, 4, 5, 4294967295] } },
    ],
  };
  assert.equal(decodeInspection(JSON.stringify(inspection)).activations.length, 2);
  const decoded = decodeInspection(JSON.stringify(child));
  assert.equal(decoded.activations.length, 5);
  assert.deepEqual(summarizeGenes(decoded.genome), [
    { kind: 'Neuron', count: 5 },
    { kind: 'Body', count: 1 },
    { kind: 'Sensor', count: 1 },
  ]);
  assert.equal(decodeInspection(JSON.stringify(inspection)).activations.length, 2);
});
