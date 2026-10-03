import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import {
  completionFor, createHeader, encodeArchive, encodeLine, parseArchive,
} from './archive.js';
import { extinct, fixtureParams, origin } from './fixtures.js';

const native = () => readFile(
  new URL('../../../shells/native/tests/fixtures/history-v4-resumed.ndjson', import.meta.url), 'utf8');

function resumedHeader(extra = {}) {
  return createHeader({
    runId: 'resumed-run', seed: '7', founders: 4, params: fixtureParams(),
    simVersion: '0.1.0', sourceRevision: 'fixture-development', resumedFromTick: '100', ...extra,
  });
}

const text = ({ header, rows, completion }) => [header, ...rows, completion].map(encodeLine).join('');

test('native resumed segments import, keeping unknown prior lineage explicit', async () => {
  const archive = await parseArchive(await native());
  assert.equal(archive.header.data.schema_version, 4);
  assert.equal(archive.header.data.resumed_from_tick, '150');
  assert.deepEqual(await parseArchive(encodeArchive(archive)), archive);
  const counts = archive.completion.data.cohorts.map((c) => c.counts);
  assert.ok(counts.every((c) => c.extinctions !== '0'), 'extinctions of pre-resume species');
});

test('browser resumed segments may be quiet and carry representatives optionally', async () => {
  for (const representatives of [false, true]) {
    const header = resumedHeader({ representatives });
    assert.equal(header.data.schema_version, 4);
    assert.equal(Object.hasOwn(header.data, 'representative_genes'), representatives);
    const completion = completionFor(header, [], { tick: '140', captureEnd: 'snapshot', stateHash: '0123456789abcdef' });
    await parseArchive(text({ header, rows: [], completion }));
  }
});

test('resumed segments reject events outside them; ordinary archives keep strict lineage', async () => {
  const header = resumedHeader();
  const unknownExtinction = extinct('0', 3, '120');
  const valid = [unknownExtinction, origin('1', 5, '40', 'evolving', '130')];
  const completion = completionFor(header, valid, { tick: '140', captureEnd: 'snapshot', stateHash: '0123456789abcdef' });
  await parseArchive(text({ header, rows: valid, completion }));

  const early = [extinct('0', 3, '99')];
  assert.throws(() => completionFor(header, early, { tick: '140', captureEnd: 'snapshot', stateHash: '0123456789abcdef' }),
    /outside the run/);
  const atBoundary = [extinct('0', 3, '140')];
  assert.throws(() => completionFor(header, atBoundary, { tick: '140', captureEnd: 'snapshot', stateHash: '0123456789abcdef' }),
    /outside the run/);

  const ordinary = createHeader({
    runId: 'r', seed: '7', founders: 4, params: fixtureParams(), simVersion: '0.1.0', sourceRevision: 'dev',
  });
  assert.throws(() => completionFor(ordinary, [unknownExtinction], { tick: '140', captureEnd: 'snapshot', stateHash: '0123456789abcdef' }),
    /extinction without an active observed origin/);

  const missingTick = structuredClone(header);
  delete missingTick.data.resumed_from_tick;
  const beyondPlan = resumedHeader();
  beyondPlan.data.ticks = '50';
  for (const bad of [missingTick, beyondPlan]) {
    await assert.rejects(parseArchive(text({ header: bad, rows: [], completion: { ...completion, data: { ...completion.data } } })),
      /Invalid history/);
  }
});
