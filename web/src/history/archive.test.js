import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import {
  byteLength, completionFor, countsFor, createHeader, encodeArchive, encodeLine, footerAllowance,
  INCOMPLETE_ENDS, MAX_LINE_BYTES, parseArchive, validateArchive, validateHeader, validatePrefix,
} from './archive.js';
import { extinct, fixtureArchive, fixtureHeader, fixtureParams, gap, origin } from './fixtures.js';

const raw = ({ header, rows, completion }) => [header, ...rows, completion].map(encodeLine).join('');
const rejects = (change, pattern = /history|History/) => {
  const archive = fixtureArchive();
  change(archive);
  return assert.rejects(parseArchive(raw(archive)), pattern);
};

for (const version of [1, 2]) {
  test(`v${version} round-trips exact identities, independent cohorts and provenance`, async () => {
    const archive = fixtureArchive(version);
    archive.rows[0].data.event.founder_birth_id = '9007199254740993';
    assert.deepEqual(await parseArchive(encodeArchive(archive)), archive);
    assert.equal(archive.header.data.provenance.seed, '18446744073709551615');
    if (version === 1) assert.equal(archive.rows[1].data.sequence, '0');
  });
}

for (const [name, version, cohorts, captureEnd] of [
  ['history-v1.ndjson', 1, ['evolving', 'random_control'], undefined],
  ['history-v2-root.ndjson', 2, ['random_control'], 'snapshot'],
  ['history-v2-prefix.ndjson', 2, ['evolving'], 'storage_limit'],
]) {
  test(`shared native fixture ${name} round-trips through the browser codec`, async () => {
    const text = await readFile(new URL(`../../../shells/native/tests/fixtures/${name}`, import.meta.url), 'utf8');
    const archive = await parseArchive(text);
    assert.equal(archive.header.data.schema_version, version);
    assert.deepEqual(archive.header.data.cohorts, cohorts);
    assert.equal(archive.completion.data.capture_end, captureEnd);
    assert.deepEqual(await parseArchive(encodeArchive(archive)), archive);
    if (captureEnd === 'storage_limit') {
      const completed = archive.completion.data.cohorts[0];
      assert.ok(BigInt(completed.counts.next_sequence) > 2n ** 53n);
      assert.equal(completed.final_state_hash, null);
      assert.equal(completed.history_complete, false);
    }
  });
}

test('construction maps only the randomized-at-birth cohort and clones params', () => {
  const params = fixtureParams();
  const header = createHeader({
    runId: 'random', seed: '42', founders: 1, params, brainInheritance: 'randomized_at_birth',
    simVersion: '1', sourceRevision: 'test',
  });
  assert.deepEqual(header.data.cohorts, ['random_control']);
  params.world.size = 5;
  assert.equal(header.data.params.world.size, 1000);
  assert.equal(header.data.ticks, null);
});

test('optional WASM boundary validator is awaited and its rejection is preserved', async () => {
  let checked = false;
  await parseArchive(raw(fixtureArchive()), async (params) => {
    await Promise.resolve();
    assert.equal(params.world.max_agents, 32);
    checked = true;
  });
  assert.ok(checked);
  await assert.rejects(validateArchive(fixtureArchive(), async () => {
    throw new Error('WASM layout rejected');
  }), /WASM layout rejected/);
});

test('numeric parameter budgets outside JavaScript precision are rejected, never rounded', async () => {
  await rejects((archive) => {
    archive.header.data.params.storage.max_memory_bytes = Number.MAX_SAFE_INTEGER + 1;
  }, /params\.storage\.max_memory_bytes/);
});

test('a founder bite is true or false, and an archive from before it may omit it', async () => {
  await rejects((archive) => {
    archive.header.data.params.founder = { bite: 1 };
  }, /params\.founder\.bite must be true or false/);
  const biting = fixtureArchive();
  biting.header.data.params.founder = { bite: true };
  await validateArchive(biting);
});

test('every required parameter field, including explicit nullable fields, must be present', async () => {
  const visit = async (value, path = []) => {
    for (const [key, child] of Object.entries(value)) {
      const archive = fixtureArchive();
      let target = archive.header.data.params;
      for (const step of path) target = target[step];
      delete target[key];
      await assert.rejects(validateArchive(archive), /missing or unknown fields/);
      if (child && typeof child === 'object' && !Array.isArray(child)) await visit(child, [...path, key]);
    }
  };
  await visit(fixtureParams());
});

test('strict headers reject versions, unknown fields, missing nulls and cohort permutations', async () => {
  for (const change of [
    (a) => { a.header.data.schema_version = 3; },
    (a) => { a.header.extra = 1; },
    (a) => { a.header.data.params.world.extra = 1; },
    (a) => { delete a.header.data.ticks; },
    (a) => { delete a.header.data.drain_every; },
    (a) => { a.header.data.cohorts = ['random_control', 'evolving']; },
    (a) => { a.header.data.cohorts = ['evolving', 'evolving']; },
    (a) => { a.header.data.cohorts = []; },
    (a) => { a.header.data.cohorts = { 0: 'evolving', 1: 'random_control' }; },
    (a) => { a.header.data.run_id = ''; },
    (a) => { a.header.data.run_id = 'x'.repeat(129); },
    (a) => { a.header.data.provenance.phase = 1; },
    (a) => { a.header.data.provenance.control = 'randomized_at_birth_v2'; },
    (a) => { a.header.data.founders = 0; },
    (a) => { a.header.data.founders = 33; },
    (a) => { a.header.data.capacity_per_cohort = 0; },
    (a) => { a.header.data.capacity_per_cohort = 33554432; },
    (a) => { a.header.data.drain_every = '0'; },
    (a) => { a.rows[0].data.cohort = 'random_control'; },
  ]) await rejects(change);
});

test('all u64 wire numbers require canonical exact strings', async () => {
  for (const bad of [0, '00', '-1', '+1', ' 1', '1.0', '1e3', '18446744073709551616', '']) {
    await rejects((a) => { a.header.data.provenance.seed = bad; }, /u64|canonical/);
    await rejects((a) => { a.rows[0].data.sequence = bad; }, /u64|canonical/);
    await rejects((a) => { a.rows[0].data.tick = bad; }, /u64|canonical/);
    await rejects((a) => { a.completion.data.cohorts[0].counts.events = bad; }, /u64|canonical/);
  }
});

test('UTF-8 framing enforces the inclusive 1 MiB line limit, not code unit length', async () => {
  const overhead = byteLength(encodeLine({ x: '' }));
  const remaining = MAX_LINE_BYTES - overhead;
  const row = { x: 'é'.repeat(Math.floor(remaining / 2)) + 'x'.repeat(remaining % 2) };
  assert.equal(byteLength(encodeLine(row)), MAX_LINE_BYTES);
  assert.throws(() => encodeLine({ x: `${row.x}a` }), /1 MiB/);
  const text = raw(fixtureArchive());
  await assert.rejects(parseArchive(text.slice(0, -1)), /newline/);
  await assert.rejects(parseArchive(`${text}\n`));
  await assert.rejects(parseArchive(text.replace('"kind":"header"', '"kind":"header","kind":"header"')), /duplicate field/);
  await assert.rejects(parseArchive(text.replace('"run_id":"fixture-run"', '"run_id":"\\ud800"')), /Unicode/);
  const header = fixtureHeader();
  header.data.provenance.source_revision = 'x'.repeat(MAX_LINE_BYTES);
  await assert.rejects(parseArchive(`${JSON.stringify(header)}\n`), /1 MiB/);
});

test('header and completion are unique ordered records', async () => {
  const archive = fixtureArchive();
  const text = raw(archive);
  await assert.rejects(parseArchive(encodeLine(archive.rows[0]) + text), /expected header/);
  await assert.rejects(parseArchive(encodeLine(archive.header) + text), /duplicate header/);
  await assert.rejects(parseArchive(text + encodeLine(archive.rows[0])), /after history completion/);
  await assert.rejects(parseArchive(encodeLine(archive.header)), /no completion marker/);
});

test('integer JSON wire fields reject fractional/exponent syntax without restricting floats', async () => {
  const text = raw(fixtureArchive());
  for (const [before, after] of [
    ['"phase":2', '"phase":2.0'],
    ['"founders":4', '"founders":4e0'],
    ['"species_id":0', '"species_id":-0'],
    ['"max_agents":32', '"max_agents":32.0'],
    ['"cells":[8,8,1]', '"cells":[8e0,8,1]'],
  ]) await assert.rejects(parseArchive(text.replace(before, after)), /non-integer JSON/);
  await assert.doesNotReject(parseArchive(text.replace('"size":1000', '"size":1e3')));
});

test('counts retain full-width gap sequences and reject discontinuity or overflow', () => {
  const header = fixtureHeader();
  const rows = [gap('0', '9007199254740992'), origin('9007199254740993')];
  const counts = countsFor(header, rows).evolving;
  assert.equal(counts.next_sequence, '9007199254740994');
  assert.equal(counts.dropped_events, '9007199254740993');
  assert.throws(() => countsFor(header, [origin('1')]), /contiguous/);
  assert.throws(() => countsFor(header, [gap('0', '18446744073709551615')]), /contiguous/);
  assert.throws(() => countsFor(header, [gap('0', '5'), origin('5')]), /contiguous/);
  assert.equal(countsFor(header, [gap('0', '18446744073709551614')]).evolving.next_sequence,
    '18446744073709551615');
});

test('lifecycle tracks active species, increasing founder IDs, exhaustion and capacity', () => {
  const header = fixtureHeader();
  for (const rows of [
    [extinct('0')],
    [origin(), extinct(), extinct('2')],
    [origin(), origin('1', 0, '1')],
    [origin(), origin('1', 1, '0')],
    [origin('0', 0, null), origin('1', 1, '1')],
    [origin('0', 4294967295)],
    [origin('0', 0, '18446744073709551615')],
  ]) assert.throws(() => countsFor(header, rows), /history/);
  header.data.params.species.capacity = 1;
  assert.throws(() => countsFor(header, [origin(), origin('1', 1, '1')]), /capacity/);
  assert.doesNotThrow(() => countsFor(header, [origin(), extinct(), origin('2', 1, '1')]));
});

test('observed parents preserve null/absent distinctions and reject cross-species contradictions', () => {
  const header = fixtureHeader();
  const child = origin('2', 2, '20');
  child.data.event.parent_a = { status: 'observed', birth_id: '0', species_id: 1 };
  assert.throws(() => countsFor(header, [origin(), origin('1', 1, '10'), child]), /predates|contradicts/);
  child.data.event.parent_a.species_id = null;
  assert.throws(() => countsFor(header, [origin(), origin('1', 1, '10'), child]), /contradicts/);
  child.data.event.parent_a.species_id = 0;
  assert.doesNotThrow(() => countsFor(header, [origin(), origin('1', 1, '10'), child]));
  child.data.event.parent_b = structuredClone(child.data.event.parent_a);
  assert.throws(() => countsFor(header, [origin(), origin('1', 1, '10'), child]), /same observed parent/);
  delete child.data.event.parent_b.species_id;
  assert.throws(() => countsFor(header, [origin(), origin('1', 1, '10'), child]), /missing/);
});

test('gaps clear active lifecycle knowledge but not chronological identity high-water marks', () => {
  const header = fixtureHeader();
  const child = origin('2', 2, '2');
  child.data.event.parent_a = { status: 'observed', birth_id: '1', species_id: 1 };
  assert.doesNotThrow(() => countsFor(header, [origin(), gap('1', '1'), child]));
  assert.doesNotThrow(() => countsFor(header, [origin(), gap('1', '1'), extinct('2', 9)]));
  assert.throws(() => countsFor(header, [origin(), gap('1', '1'), origin('2', 0, '1')]), /reused/);
  header.data.params.species.capacity = 0;
  assert.throws(() => countsFor(header, [gap()]), /disabled classification/);
});

test('event ticks use the completed boundary with only the founder tick-zero exception', () => {
  const header = fixtureHeader();
  assert.doesNotThrow(() => validatePrefix(header, [origin()], { tick: '0' }));
  assert.throws(() => validatePrefix(header, [origin(), extinct()], { tick: '0' }), /extinctions/);
  assert.throws(() => validatePrefix(header, [origin('0', 0, '0', 'evolving', '1')], { tick: '1' }), /outside/);
  const child = origin('1', 1, '1');
  child.data.event.parent_a = { status: 'observed', birth_id: '0', species_id: 0 };
  assert.throws(() => validatePrefix(header, [origin(), child], { tick: '0' }), /seeding-only/);
  assert.throws(() => validatePrefix(header, [
    origin('0', 0, '0', 'evolving', '2'), extinct('1', 0, '1'),
  ], { tick: '3' }), /out of order/);
});

for (const version of [1, 2]) {
  test(`v${version} zero-tick origins need available birth IDs below the requested founders`, async () => {
    const header = fixtureArchive(version).header;
    if (version === 1) header.data.ticks = '0';
    for (const withGaps of [false, true]) {
      const rows = header.data.cohorts.flatMap((cohort) => withGaps
        ? [gap('0', '0', cohort), origin('1', 0, '3', cohort), gap('2', '2', cohort)]
        : [origin('0', 0, '3', cohort)]);
      const completion = completionFor(header, rows, {
        tick: '0', captureEnd: 'snapshot', stateHash: '0123456789abcdef',
      });
      await assert.doesNotReject(parseArchive(raw({ header, rows, completion })));
      for (const cohort of header.data.cohorts) {
        for (const birth of ['4', '9007199254740993', '18446744073709551614', null]) {
          const invalid = structuredClone(rows);
          invalid.find((row) => row.kind === 'event' && row.data.cohort === cohort)
            .data.event.founder_birth_id = birth;
          assert.throws(() => validatePrefix(header, invalid, { tick: '0' }), /seeding-only/);
          assert.throws(() => completionFor(header, invalid, {
            tick: '0', captureEnd: 'snapshot', stateHash: '0123456789abcdef',
          }), /seeding-only/);
          await assert.rejects(parseArchive(raw({ header, rows: invalid, completion })), /seeding-only/);
          if (version === 1) assert.throws(() => countsFor(header, invalid), /seeding-only/);
          else assert.doesNotThrow(() => countsFor(header, invalid));
        }
      }
    }
  });

  test(`v${version} tick-zero events in later captures retain high and unavailable identities`, async () => {
    const header = fixtureArchive(version).header;
    for (const birth of ['4', '9007199254740993', '18446744073709551614', null]) {
      for (const status of ['absent', 'unavailable']) {
        const rows = header.data.cohorts.map((cohort) => {
          const row = origin('0', 0, birth, cohort);
          row.data.event.parent_a = { status };
          return row;
        });
        const completion = completionFor(header, rows, {
          tick: '1', captureEnd: 'snapshot', stateHash: '0123456789abcdef',
        });
        const archive = { header, rows, completion };
        assert.deepEqual(await parseArchive(encodeArchive(archive)), archive);
      }
    }
  });
}

test('v2 zero-boundary incomplete prefixes enforce founder limits despite gaps or open plans', async () => {
  for (const planned of [null, '0', '8']) {
    for (const captureEnd of INCOMPLETE_ENDS) {
      const header = fixtureHeader();
      header.data.ticks = planned;
      const rows = [gap(), origin('1', 1, '3'), gap('2', '2')];
      const completion = completionFor(header, rows, { tick: '0', captureEnd });
      await assert.doesNotReject(parseArchive(raw({ header, rows, completion })));
      assert.doesNotThrow(() => completionFor(header, [], { tick: '0', captureEnd }));
      for (const birth of ['4', '9007199254740993', '18446744073709551614', null]) {
        const invalid = structuredClone(rows);
        invalid[1].data.event.founder_birth_id = birth;
        await assert.rejects(parseArchive(raw({ header, rows: invalid, completion })), /seeding-only/);
        assert.throws(() => validatePrefix(header, invalid, { tick: '0' }), /seeding-only/);
        if (planned === '0') assert.throws(() => countsFor(header, invalid), /seeding-only/);
        else assert.doesNotThrow(() => countsFor(header, invalid));
      }
    }
  }
});

test('completion validates exact totals, membership, provenance and required null/hash fields', async () => {
  for (const change of [
    (a) => { a.completion.data.run_id = 'other'; },
    (a) => { a.completion.data.provenance.seed = '42'; },
    (a) => { a.completion.data.cohorts.reverse(); a.completion.data.cohorts.push(a.completion.data.cohorts[0]); },
    (a) => { a.completion.data.cohorts[0].counts.events = '2'; },
    (a) => { a.completion.data.cohorts[0].history_complete = false; },
    (a) => { a.completion.data.cohorts[0].final_state_hash = null; },
    (a) => { a.completion.data.cohorts[0].final_state_hash = 'ABCDEF0123456789'; },
    (a) => { delete a.completion.data.cohorts[0].final_state_hash; },
    (a) => { a.completion.data.capture_end = 'crashed'; },
  ]) await rejects(change);
});

test('all intentional boundaries require hashes and finished requires exact known planned ticks', () => {
  const header = fixtureHeader();
  for (const captureEnd of ['snapshot', 'stopped', 'reseeded', 'params_changed']) {
    assert.throws(() => completionFor(header, [origin()], { tick: '1', captureEnd }), /hash/);
    assert.doesNotThrow(() => completionFor(header, [origin()], {
      tick: '1', captureEnd, stateHash: '0000000000000000',
    }));
  }
  assert.throws(() => completionFor(header, [origin()], {
    tick: '1', captureEnd: 'finished', stateHash: '0000000000000000',
  }), /planned/);
  header.data.ticks = '1';
  assert.doesNotThrow(() => completionFor(header, [origin()], {
    tick: '1', captureEnd: 'finished', stateHash: '0000000000000000',
  }));
  assert.throws(() => completionFor(header, [origin()], {
    tick: '2', captureEnd: 'snapshot', stateHash: '0000000000000000',
  }), /planned/);
});

test('unfinalized and failure exports accept header-only prefixes and never claim completeness', async () => {
  for (const captureEnd of INCOMPLETE_ENDS) {
    const header = fixtureHeader();
    const completion = completionFor(header, [], { tick: '0', captureEnd });
    assert.equal(completion.data.cohorts[0].history_complete, false);
    assert.equal(completion.data.cohorts[0].final_state_hash, null);
    const archive = { header, rows: [], completion };
    assert.deepEqual(await parseArchive(encodeArchive(archive)), archive);
  }
});

test('footer reservation bounds every capture end and exact counter width', () => {
  const header = fixtureHeader();
  for (const captureEnd of [...INCOMPLETE_ENDS, 'snapshot', 'stopped', 'reseeded', 'params_changed']) {
    const completion = completionFor(header, [gap('0', '18446744073709551614')], {
      tick: '18446744073709551615', captureEnd, stateHash: 'ffffffffffffffff',
    });
    assert.ok(byteLength(encodeLine(completion)) <= footerAllowance(header));
  }
  header.data.run_id = '🌱'.repeat(128);
  assert.doesNotThrow(() => validateHeader(header));
});
