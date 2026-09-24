/** Live genome-size distributions, decoded exactly as the native metrics report them. */
const U32_MAX = 0xffff_ffff;

export interface SizeDistribution {
  min: number;
  p25: number;
  median: number;
  p75: number;
  max: number;
  mean: number;
}

export interface Complexity {
  genomeGenes: SizeDistribution;
  neurons: SizeDistribution;
  connections: SizeDistribution;
  enabledConnections: SizeDistribution;
}

const ORDER = ['min', 'p25', 'median', 'p75', 'max'] as const;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function decodeDistribution(value: unknown, name: string): SizeDistribution {
  if (!isRecord(value)) throw new TypeError(`invalid ${name} distribution`);
  let previous = 0;
  const out: Partial<SizeDistribution> = {};
  for (const key of ORDER) {
    const item = value[key];
    if (typeof item !== 'number' || !Number.isInteger(item) || item < previous || item > U32_MAX) {
      throw new TypeError(`invalid ${name} distribution`);
    }
    out[key] = previous = item;
  }
  const mean = value.mean;
  if (typeof mean !== 'number' || !Number.isFinite(mean) || mean < out.min! || mean > out.max!) {
    throw new TypeError(`invalid ${name} distribution`);
  }
  return { ...(out as Omit<SizeDistribution, 'mean'>), mean };
}

function boundedBy(inner: SizeDistribution, outer: SizeDistribution): boolean {
  return [...ORDER, 'mean' as const].every((key) => inner[key] <= outer[key]);
}

export function decodeComplexity(json: string, population: number): Complexity {
  const data: unknown = JSON.parse(json);
  if (!isRecord(data)) throw new TypeError('invalid complexity payload');
  const complexity = {
    genomeGenes: decodeDistribution(data.genome_genes, 'genome gene'),
    neurons: decodeDistribution(data.neurons, 'neuron'),
    connections: decodeDistribution(data.connections, 'connection'),
    enabledConnections: decodeDistribution(data.enabled_connections, 'enabled connection'),
  };
  const empty = Object.values(complexity).every((d) => d.max === 0 && d.mean === 0);
  if (
    empty !== (population === 0) ||
    !boundedBy(complexity.enabledConnections, complexity.connections) ||
    !boundedBy(complexity.neurons, complexity.genomeGenes) ||
    !boundedBy(complexity.connections, complexity.genomeGenes)
  ) throw new TypeError('inconsistent complexity distributions');
  return complexity;
}
