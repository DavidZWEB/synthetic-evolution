/** Shareable seed URL regressions. */

import assert from 'node:assert/strict';
import test from 'node:test';

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
  });
});

test('a seed-only URL keeps the configured founder default', () => {
  assert.deepEqual(readRunUrl('https://example.test/#seed=42', 2000), {
    seed: '42',
    founders: 2000,
    params: null,
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
  });
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
});
