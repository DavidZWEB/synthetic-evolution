import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import {
  byteLength, completionFor, createHeader, encodeArchive, encodeLine, footerAllowance,
  MAX_LINE_BYTES, parseArchive, REPRESENTATIVE_GENES,
} from './archive.js';
import { createHistoryDelivery } from './delivery.js';
import { extinct, fixtureParams, origin } from './fixtures.js';

// Real-valued fields, negatives, and integral floats: all legal in core Gene JSON.
const genome = (count = 2) => Array.from({ length: count }, (_, index) => ({
  Neuron: { id: index + 1, bias: -1.25, tau: 1, activation: 'Tanh', period: 0.5 },
}));

function header() {
  return createHeader({
    runId: 'representative-run', seed: '7', founders: 4, params: fixtureParams(),
    simVersion: '0.1.0', sourceRevision: 'fixture-development', representatives: true,
  });
}

function withRepresentative(row, representative) {
  return { ...row, data: { ...row.data, representative } };
}

function archive() {
  const h = header();
  const rows = [
    withRepresentative(origin('0', 0, '0'), { status: 'recorded', genes: genome() }),
    withRepresentative(origin('1', 1, '1'), { status: 'unavailable', reason: 'capture_pressure' }),
    extinct('2', 0, '1'),
  ];
  const completion = completionFor(h, rows, { tick: '2', captureEnd: 'snapshot', stateHash: '0123456789abcdef' });
  return { header: h, rows, completion };
}

const text = ({ header: h, rows, completion }) => [h, ...rows, completion].map(encodeLine).join('');

test('browser schema 3 headers opt into fixed staging and count representatives', async () => {
  const value = archive();
  assert.equal(value.header.data.schema_version, 3);
  assert.equal(value.header.data.representative_genes, REPRESENTATIVE_GENES);
  assert.equal(REPRESENTATIVE_GENES, 65_536);
  const counts = value.completion.data.cohorts[0].counts;
  assert.equal(counts.representatives, '1');
  assert.equal(counts.unavailable_representatives, '1');
  assert.deepEqual(await parseArchive(encodeArchive(value)), value);
  const footer = byteLength(encodeLine(value.completion));
  assert.ok(footerAllowance(value.header) >= footer, 'reserved footer covers representative counts');
  assert.equal(createHeader({
    runId: 'r', seed: '7', founders: 4, params: fixtureParams(),
    simVersion: '0.1.0', sourceRevision: 'dev',
  }).data.schema_version, 2, 'representatives stay opt-in');
});

test('native schema 3 archives import through the browser codec', async () => {
  const native = await readFile(
    new URL('../../../shells/native/tests/fixtures/history-v3-native.ndjson', import.meta.url), 'utf8');
  const validated = [];
  const parsed = await parseArchive(native, undefined, (genes) => { validated.push(genes.length); });
  assert.equal(parsed.header.data.schema_version, 3);
  assert.equal(Object.hasOwn(parsed.header.data, 'run_id'), false);
  assert.deepEqual(parsed.header.data.cohorts, ['evolving', 'random_control']);
  assert.equal(validated.length, 2, 'every recorded genome reaches the WASM validator');
  for (const cohort of parsed.completion.data.cohorts) assert.equal(cohort.counts.representatives, '1');
  assert.deepEqual(await parseArchive(encodeArchive(parsed)), parsed);
});

test('the representative validator is awaited and its rejection preserved', async () => {
  await assert.rejects(
    parseArchive(text(archive()), undefined, async () => { throw new Error('incoherent genome'); }),
    /incoherent genome/);
});

test('representatives appear on every schema 3 origin and nowhere else', async () => {
  const cases = {
    'missing on an origin': (a) => { delete a.rows[0].data.representative; },
    'present on an extinction': (a) => {
      a.rows[2].data.representative = { status: 'unavailable', reason: 'capture_pressure' };
    },
    'unknown reason': (a) => { a.rows[1].data.representative.reason = 'guessed'; },
    'unknown status': (a) => { a.rows[1].data.representative = { status: 'reconstructed', reason: 'line_limit' }; },
    'null representative': (a) => { a.rows[0].data.representative = null; },
    'genes beyond max_genes': (a) => {
      a.rows[0].data.representative.genes = genome(a.header.data.params.storage.max_genes + 1);
    },
    'non-object genes': (a) => { a.rows[0].data.representative.genes = [1]; },
    'staging below one maximum genome': (a) => {
      a.header.data.representative_genes = a.header.data.params.storage.max_genes - 1;
    },
    'schema 3 without staging': (a) => { delete a.header.data.representative_genes; },
    'footer omits representative counts': (a) => {
      delete a.completion.data.cohorts[0].counts.unavailable_representatives;
    },
    'footer undercounts representatives': (a) => { a.completion.data.cohorts[0].counts.representatives = '0'; },
  };
  for (const [name, change] of Object.entries(cases)) {
    const value = archive();
    change(value);
    await assert.rejects(parseArchive(text(value)), /Invalid history/, `accepted ${name}`);
  }
  const v2 = createHeader({
    runId: 'r', seed: '7', founders: 4, params: fixtureParams(), simVersion: '0.1.0', sourceRevision: 'dev',
  });
  const rows = [withRepresentative(origin(), { status: 'unavailable', reason: 'capture_pressure' })];
  assert.throws(() => completionFor(v2, rows, { tick: '1', captureEnd: 'snapshot', stateHash: '0123456789abcdef' }),
    /Invalid history/, 'schema 2 origins cannot carry representatives');
});

test('an origin whose genome would exceed the line limit is delivered as unavailable', () => {
  const sent = [];
  let records = [];
  const sim = {
    history_pending: () => records.length > 0,
    disable_history: () => {},
    drain_history: () => {
      const drained = records;
      records = [];
      return JSON.stringify({
        records: drained, through_tick: '1', next_sequence: String(drained.length),
        dropped_events: '0', sequence_exhausted: false,
      });
    },
  };
  const delivery = createHistoryDelivery({ sim, cohort: 'evolving', send: (message) => sent.push(message) });
  const small = { status: 'recorded', genes: genome() };
  const huge = { status: 'recorded', genes: genome(Math.ceil(MAX_LINE_BYTES / 60)) };
  records = [origin('0', 0), origin('1', 1)].map((row, index) => ({
    kind: row.kind,
    data: { ...(({ cohort, ...data }) => data)(row.data), representative: index ? huge : small },
  }));
  delivery.pump();
  const [kept, limited] = sent[0].rows;
  assert.deepEqual(kept.data.representative, small);
  assert.deepEqual(limited.data.representative, { status: 'unavailable', reason: 'line_limit' });
  assert.deepEqual(Object.keys(limited.data), ['cohort', 'sequence', 'tick', 'event', 'representative']);
  assert.ok(byteLength(encodeLine(limited)) <= MAX_LINE_BYTES);
});
