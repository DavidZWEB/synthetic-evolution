import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { parseArchive } from '../history/archive.js';
import { align } from './compare.js';
import { layout, lineage, prune } from './graph.js';

const absent = { status: 'absent' };
const observed = (species) => ({ status: 'observed', birth_id: '1', species_id: species });
const origin = (species, tick, parentA = absent, representative) => ({
  kind: 'event',
  data: {
    cohort: 'evolving', sequence: '0', tick,
    event: { kind: 'species_origin', species_id: species, founder_birth_id: '0', parent_a: parentA, parent_b: absent },
    ...(representative ? { representative } : {}),
  },
});
const extinct = (species, tick) => ({
  kind: 'event', data: { cohort: 'evolving', sequence: '0', tick, event: { kind: 'species_extinct', species_id: species } },
});
const gap = { kind: 'gap', data: { cohort: 'evolving', first_sequence: '0', last_sequence: '3' } };

test('the layered tree follows recorded parents and marks what it cannot know', () => {
  const archive = {
    header: { data: {} },
    rows: [
      origin(0, '0'), origin(1, '0'),
      origin(2, '5', observed(0)), origin(3, '6', observed(2)),
      origin(4, '7', observed(null)),
      extinct(1, '8'), extinct(3, '9'),
      gap,
      origin(5, '12', observed(9)),
    ],
  };
  const { nodes, unknownBefore } = lineage(archive, 'evolving');
  const by = Object.fromEntries(nodes.map((node) => [node.id, node]));
  assert.equal(unknownBefore, true, 'a gap hides lineage');
  assert.deepEqual([by[0].depth, by[2].depth, by[3].depth], [0, 1, 2]);
  assert.deepEqual(by[2].parents, [0]);
  assert.equal(by[4].unknownParent, true, 'unclassified parent');
  assert.equal(by[5].unknownParent, true, 'parent lost to the gap');
  assert.equal(by[5].depth, 0);
  assert.equal(by[1].extinctTick, '8');
  assert.equal(lineage(archive, 'random_control').nodes.length, 0, 'cohorts stay separate');

  const pruned = prune(nodes).map((node) => node.id);
  assert.deepEqual(pruned, [0, 2, 4, 5], 'extinct leaves go; ancestors of the living stay');
  const positions = layout(nodes);
  assert.deepEqual(positions.filter((p) => p.row === 0).map((p) => [p.node.id, p.column]),
    [[0, 0], [1, 1], [4, 2], [5, 3]]);
});

test('a resumed native segment marks pre-resume parents as unknown', async () => {
  const archive = await parseArchive(await readFile(
    new URL('../../../shells/native/tests/fixtures/history-v4-resumed.ndjson', import.meta.url), 'utf8'));
  const { nodes, unknownBefore } = lineage(archive, 'evolving');
  assert.equal(unknownBefore, true);
  assert.ok(nodes.length > 0);
  for (const node of nodes) {
    assert.ok(node.parents.every((id) => nodes.some((other) => other.id === id)), 'edges stay in the graph');
  }
  assert.ok(nodes.some((node) => node.unknownParent), 'some parents predate the resume');
});

const neuron = (id) => ({ Neuron: { id, bias: 0, tau: 1, activation: 'Sigmoid', period: 0 } });
const connection = (id, weight, enabled = true) => ({ Connection: { id, from: 1, to: 2, weight, enabled } });

test('aligned comparison counts shared and unique genes and lists only differing wiring', () => {
  const a = [neuron(1), neuron(2), connection(10, 0.5), connection(11, 1), connection(12, -1)];
  const b = [neuron(1), neuron(2), neuron(3), connection(10, 0.5), connection(11, 0.25, false), connection(13, 2)];
  const { kinds, connections } = align(a, b);
  assert.deepEqual(kinds.find((k) => k.kind === 'neurons'), { kind: 'neurons', shared: 2, onlyA: 0, onlyB: 1 });
  assert.deepEqual(kinds.find((k) => k.kind === 'connections'), { kind: 'connections', shared: 2, onlyA: 1, onlyB: 1 });
  assert.deepEqual(connections.map((c) => c.id), [11, 12, 13], 'identical connection 10 is omitted');
  assert.deepEqual(connections[0], { id: 11, a: { weight: 1, enabled: true }, b: { weight: 0.25, enabled: false }, delta: 0.75 });
  assert.equal(connections[1].b, null);
  assert.equal(connections[2].a, null);
});
