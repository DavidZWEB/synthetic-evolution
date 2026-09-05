/**
 * Typed inspector payload consumed by the Svelte UI.
 *
 * `Gene` is generated from the Rust enum by ts-rs. The surrounding inspection envelope
 * is a WASM-shell concern rather than simulation state, so it stays local to the client.
 */

import type { Gene } from '../generated/Gene';

export interface Inspection {
  index: number;
  incarnation: number;
  tick: string;
  energy: number;
  age: number;
  size: number;
  signature: [number, number, number];
  species_id: number;
  parent_a: number;
  parent_b: number;
  brain_units: number;
  sensor_load: number;
  activations: number[];
  genome: Gene[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

const isFiniteNumber = (value: unknown): value is number =>
  typeof value === 'number' && Number.isFinite(value);

function isInspection(value: unknown): value is Inspection {
  if (!isRecord(value)) return false;
  const geneKinds = new Set(['Neuron', 'Sensor', 'Effector', 'Connection', 'Body', 'Meta']);
  return (
    Number.isSafeInteger(value.index) &&
    Number.isSafeInteger(value.incarnation) &&
    typeof value.tick === 'string' &&
    /^(0|[1-9]\d*)$/.test(value.tick) &&
    isFiniteNumber(value.energy) &&
    Number.isSafeInteger(value.age) &&
    isFiniteNumber(value.size) &&
    Array.isArray(value.signature) &&
    value.signature.length === 3 &&
    value.signature.every(isFiniteNumber) &&
    Number.isSafeInteger(value.species_id) &&
    Number.isSafeInteger(value.parent_a) &&
    Number.isSafeInteger(value.parent_b) &&
    Number.isSafeInteger(value.brain_units) &&
    isFiniteNumber(value.sensor_load) &&
    Array.isArray(value.activations) &&
    value.activations.every(isFiniteNumber) &&
    Array.isArray(value.genome) &&
    value.genome.every(
      (gene) =>
        isRecord(gene) &&
        Object.keys(gene).length === 1 &&
        geneKinds.has(Object.keys(gene)[0]),
    )
  );
}

export function decodeInspection(json: string): Inspection {
  const value: unknown = JSON.parse(json);
  if (!isInspection(value)) throw new TypeError('invalid inspection payload');
  return value;
}

export function summarizeGenes(genes: Gene[]): Array<{ kind: string; count: number }> {
  const counts = new Map<string, number>();
  for (const gene of genes) {
    const kind = Object.keys(gene)[0] ?? 'Unknown';
    counts.set(kind, (counts.get(kind) ?? 0) + 1);
  }
  return [...counts].map(([kind, count]) => ({ kind, count }));
}
