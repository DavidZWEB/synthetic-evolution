import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { parseArchive } from '../history/archive.js';
import {
  cohortFor, manifestFor, sectionsFor, sectionsOf, segmentsFor, validateSegments,
} from './saved-run.js';

const statuses = (segments) => segments.map(({ segment }) =>
  `${segment.status}@${segment.starts_at}${segment.reason ? `:${segment.reason}` : ''}`);

test('segments record exactly what was captured, with explicit gaps', () => {
  const fresh = { restored: [], startTick: 0n };
  assert.deepEqual(statuses(segmentsFor({ ...fresh, tick: '0', archive: null })), ['unavailable@0:not_recorded']);
  assert.deepEqual(statuses(segmentsFor({ ...fresh, tick: '30', archive: null })), ['unavailable@0:not_recorded']);
  assert.deepEqual(statuses(segmentsFor({ ...fresh, tick: '30', archive: { text: 'a', end: '30' } })), ['included@0']);
  // Recording stopped at tick 10; the remainder is an explicit gap, never invented.
  assert.deepEqual(statuses(segmentsFor({ ...fresh, tick: '30', archive: { text: 'a', end: '10' } })),
    ['included@0', 'unavailable@10:not_recorded']);
  // An archive ending where it began still holds that tick's founder origins.
  assert.deepEqual(statuses(segmentsFor({ ...fresh, tick: '30', archive: { text: 'a', end: '0' } })),
    ['included@0', 'unavailable@0:not_recorded']);
  assert.deepEqual(statuses(segmentsFor({ ...fresh, tick: '0', archive: { text: 'a', end: '0' } })), ['included@0']);

  const restored = [{ segment: { status: 'included', starts_at: '0', bytes: '1' }, text: 'r' }];
  const loaded = { restored, startTick: 50n };
  assert.deepEqual(statuses(segmentsFor({ ...loaded, tick: '50', archive: null })), ['included@0'],
    'saving at the load tick adds nothing');
  assert.deepEqual(statuses(segmentsFor({ ...loaded, tick: '80', archive: null })),
    ['included@0', 'unavailable@50:not_recorded']);
  assert.deepEqual(statuses(segmentsFor({ ...loaded, tick: '80', archive: { text: 'b', end: '80' } })),
    ['included@0', 'included@50']);
  const [, included] = segmentsFor({ ...loaded, tick: '80', archive: { text: 'é', end: '80' } });
  assert.equal(included.segment.bytes, '2', 'lengths are UTF-8 bytes');
});

test('manifests and sections follow the container order', () => {
  const segments = segmentsFor({ restored: [], startTick: 0n, tick: '5', archive: { text: 'xy', end: '5' } });
  const checkpoint = new Uint8Array([1, 2, 3]);
  const manifest = manifestFor({
    provenance: { seed: '7' }, writer: {}, checkpointFormat: 1, tick: '5',
    cohort: cohortFor('randomized_at_birth'), stateHash: '0123456789abcdef', checkpoint, segments,
  });
  assert.deepEqual(manifest.cohorts, [{ cohort: 'random_control', state_hash: '0123456789abcdef', bytes: '3' }]);
  assert.equal(manifest.tick, '5');
  const sections = sectionsFor(checkpoint, segments);
  assert.deepEqual([...sections], [1, 2, 3, 120, 121]);
  const decoded = { checkpoints: [[0, 3]], history: [[3, 2]] };
  const sliced = sectionsOf(sections, decoded);
  assert.deepEqual([...sliced.checkpoints[0]], [1, 2, 3]);
  assert.deepEqual(sliced.texts, ['xy']);
  assert.throws(() => sectionsOf(new Uint8Array([0xff]), { checkpoints: [], history: [[0, 1]] }),
    'archive text must be exact UTF-8');
  assert.throws(() => cohortFor('sexual'));
});

const fixture = (name) => readFile(
  new URL(`../../../shells/native/tests/fixtures/${name}`, import.meta.url), 'utf8');

test('loaded segments must belong to the run, cover its cohorts, and end on their boundary', async () => {
  const first = await fixture('history-v1.ndjson');
  const resumed = await fixture('history-v4-resumed.ndjson');
  const firstArchive = await parseArchive(first);
  const resumedArchive = await parseArchive(resumed);
  const manifest = (overrides = {}) => ({
    provenance: { seed: resumedArchive.header.data.provenance.seed },
    tick: '210',
    cohorts: [{ cohort: 'evolving' }],
    history: [
      { status: 'unavailable', starts_at: '0', reason: 'not_recorded' },
      { status: 'included', starts_at: '150', bytes: '1' },
    ],
    ...overrides,
  });
  const segments = await validateSegments(manifest(), [null, resumed], parseArchive);
  assert.equal(segments[1].archive.header.data.resumed_from_tick, '150');
  assert.equal(segments[0].archive, null);

  for (const [name, bad, texts] of [
    ['ends after the save', manifest({ tick: '220' }), [null, resumed]],
    ['resumes elsewhere', manifest({ history: [
      { status: 'unavailable', starts_at: '0', reason: 'not_recorded' },
      { status: 'included', starts_at: '140', bytes: '1' },
    ] }), [null, resumed]],
    ['another seed', manifest({ provenance: { seed: '999' } }), [null, resumed]],
    ['a first segment that resumes', manifest({ history: [{ status: 'included', starts_at: '0', bytes: '1' }] }), [resumed]],
  ]) {
    await assert.rejects(validateSegments(bad, texts, parseArchive), /does not belong/, name);
  }
  // A paired archive may accompany one cohort, but must cover the bundle's cohorts.
  const paired = manifest({
    provenance: { seed: firstArchive.header.data.provenance.seed },
    tick: firstArchive.completion.data.ticks,
    cohorts: [{ cohort: 'random_control' }],
    history: [{ status: 'included', starts_at: '0', bytes: '1' }],
  });
  await validateSegments(paired, [first], parseArchive);
});
