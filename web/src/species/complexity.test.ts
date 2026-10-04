import assert from 'node:assert/strict';
import test from 'node:test';
import { decodeComplexity } from './complexity.ts';

const distribution = { min: 36, p25: 49, median: 50, p75: 51, max: 66, mean: 52.21875 };
const valid = {
  genome_genes: { min: 68, p25: 81, median: 81, p75: 82, max: 95, mean: 83.09375 },
  neurons: { min: 15, p25: 15, median: 15, p75: 15, max: 15, mean: 15 },
  connections: { min: 39, p25: 52, median: 52, p75: 53, max: 66, mean: 54.09375 },
  enabled_connections: distribution,
};

test('native complexity payloads decode exactly', () => {
  const decoded = decodeComplexity(JSON.stringify(valid), 32);
  assert.deepEqual(decoded.enabledConnections, distribution);
  assert.equal(decoded.genomeGenes.mean, 83.09375);
  const zero = { min: 0, p25: 0, median: 0, p75: 0, max: 0, mean: 0 };
  const empty = JSON.stringify({
    genome_genes: zero, neurons: zero, connections: zero, enabled_connections: zero,
  });
  assert.equal(decodeComplexity(empty, 0).neurons.max, 0);
  assert.throws(() => decodeComplexity(empty, 1), 'living agents have genomes');
  assert.throws(() => decodeComplexity(JSON.stringify(valid), 0), 'no agents, no sizes');
});

test('disordered, out-of-range, or unbounded distributions are rejected', () => {
  const variants: [string, unknown][] = [
    ['genome_genes', { ...valid.genome_genes, p25: 96 }],
    ['genome_genes', { ...valid.genome_genes, min: -1 }],
    ['genome_genes', { ...valid.genome_genes, max: 2 ** 32 }],
    ['genome_genes', { ...valid.genome_genes, median: 81.5 }],
    ['neurons', { ...valid.neurons, mean: 16 }],
    ['neurons', { ...valid.neurons, mean: null }],
    ['enabled_connections', { ...distribution, max: 67, mean: 60 }],
    ['connections', { ...valid.connections, max: 96 }],
    ['connections', undefined],
  ];
  for (const [field, value] of variants) {
    assert.throws(
      () => decodeComplexity(JSON.stringify({ ...valid, [field]: value }), 32),
      `accepted ${field} ${JSON.stringify(value)}`,
    );
  }
  assert.throws(() => decodeComplexity('[]', 1));
});

test('wiring decodes when recorded, is null when absent, and is validated', () => {
  assert.equal(decodeComplexity(JSON.stringify(valid), 32).wiring, null);
  const wired = { min: 0, p25: 1, median: 2, p75: 2, max: 3, mean: 1.5 };
  const withWiring = {
    ...valid,
    wiring: { wired_hidden_neurons: wired, wired_sensors: wired, driven_effectors: wired },
  };
  const decoded = decodeComplexity(JSON.stringify(withWiring), 32);
  assert.deepEqual(decoded.wiring?.drivenEffectors, wired);
  for (const wiring of [
    [],
    { ...withWiring.wiring, wired_sensors: undefined },
    { ...withWiring.wiring, wired_hidden_neurons: { ...wired, max: 16, mean: 15.5 } },
  ]) {
    assert.throws(
      () => decodeComplexity(JSON.stringify({ ...valid, wiring }), 32),
      `accepted ${JSON.stringify(wiring)}`,
    );
  }
});
