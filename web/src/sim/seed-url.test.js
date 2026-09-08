/** Shareable seed URL regressions. */

import assert from 'node:assert/strict';
import test from 'node:test';

import { EVOLVING, RANDOMIZED_AT_BIRTH } from './brain-inheritance.js';
import { readRunUrl, writeRunUrl } from './seed-url.js';

test('run URLs preserve seeds above the JavaScript integer ceiling', () => {
  const href = writeRunUrl('https://example.test/view?theme=dark', {
    seed: '9007199254740993',
    founders: 321,
  });
  assert.equal(
    href,
    'https://example.test/view?theme=dark#seed=9007199254740993&founders=321',
  );
  assert.deepEqual(readRunUrl(href, 2000), {
    seed: '9007199254740993',
    founders: 321,
    params: null,
    brainInheritance: EVOLVING,
  });
});

test('a seed-only URL keeps the configured founder default', () => {
  assert.deepEqual(readRunUrl('https://example.test/#seed=42', 2000), {
    seed: '42',
    founders: 2000,
    params: null,
    brainInheritance: EVOLVING,
  });
});

test('parameter JSON round-trips through the fragment canonically', () => {
  const href = writeRunUrl('https://example.test/', {
    seed: '42',
    founders: 2,
    params: '{ \"world\": { \"dt\": 0.02 } }',
  });
  assert.deepEqual(readRunUrl(href, 2000), {
    seed: '42',
    founders: 2,
    params: '{"world":{"dt":0.02}}',
    brainInheritance: EVOLVING,
  });
});

test('scalar-control heredity keeps the legacy mode ID in shared fragments', () => {
  const href = writeRunUrl('https://example.test/', {
    seed: '42',
    founders: 2000,
    brainInheritance: RANDOMIZED_AT_BIRTH,
  });
  assert.equal(
    href,
    'https://example.test/#seed=42&founders=2000&inheritance=randomized_at_birth',
  );
  assert.equal(readRunUrl(href, 2000).brainInheritance, RANDOMIZED_AT_BIRTH);
});

test('scalar-control links preserve opt-in structural rates without changing the mode ID', () => {
  const params = '{"mutation":{"structural":{"add_neuron_rate":0.01}}}';
  const href = writeRunUrl('https://example.test/', {
    seed: '42',
    founders: 2,
    brainInheritance: RANDOMIZED_AT_BIRTH,
    params,
  });
  const run = readRunUrl(href, 2000);
  assert.equal(run.brainInheritance, 'randomized_at_birth');
  assert.equal(run.params, params);
});

test('sparse no-eye founder and organ config round-trips in both heredity modes', () => {
  for (const brainInheritance of [EVOLVING, RANDOMIZED_AT_BIRTH]) {
    for (const connectionsPerTarget of [null, 0, 1]) {
      const params = JSON.stringify({
        sensing: { vision_rays: 0, chemo_sensors: 1, energy_sensors: 0 },
        brain: { hidden_neurons: 0, oscillators: 0, connections_per_target: connectionsPerTarget },
        mutation: { organs: { remove_sensor_rate: 0.001, add_sensor_rate: 0.001 } },
      });
      const config = { seed: '9007199254740993', founders: 2, brainInheritance, params };
      const href = writeRunUrl('https://example.test/', config);
      assert.deepEqual(readRunUrl(href, 2000), config);
      assert.equal(writeRunUrl(href, readRunUrl(href, 2000)), href);
    }
  }
});

test('invalid run fragments fail loudly', () => {
  assert.throws(() => readRunUrl('https://example.test/#seed=-1', 2000), /unsigned/);
  assert.throws(
    () => readRunUrl('https://example.test/#seed=42&founders=1.5', 2000),
    /positive integer/,
  );
  assert.throws(
    () => readRunUrl('https://example.test/#seed=42&params=%5B%5D', 2000),
    /encode an object/,
  );
  assert.throws(
    () => readRunUrl('https://example.test/#seed=42&inheritance=random', 2000),
    /inheritance/,
  );
});
