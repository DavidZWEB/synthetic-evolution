/**
 * Typed inspector payload consumed by the Svelte UI.
 *
 * `Gene` is generated from the Rust enum by ts-rs. The surrounding inspection envelope
 * is a WASM-shell concern rather than simulation state, so it stays local to the client.
 */

import type { Gene } from '../generated/Gene';
import type { BirthId } from '../generated/BirthId';

export interface Inspection {
  index: number;
  incarnation: number;
  tick: string;
  energy: number;
  age: number;
  size: number;
  signature: [number, number, number];
  species_id: number;
  birth_id: BirthId;
  parent_birth_a: BirthId;
  parent_birth_b: BirthId;
  parent_a: number;
  parent_b: number;
  brain_units: number;
  sensor_load: number;
  health: number;
  muscle: number;
  mouth: number;
  eaten_plants: number;
  eaten_animals: number;
  kills: number;
  activations: number[];
  genome: Gene[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

const isFiniteNumber = (value: unknown): value is number =>
  typeof value === 'number' && Number.isFinite(value);

const NULL_SPECIES = 0xffff_ffff;
const isUint32 = (value: unknown): value is number =>
  typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= NULL_SPECIES;

export function speciesLabel(id: number): string {
  if (!isUint32(id)) throw new TypeError('invalid species ID');
  return id === NULL_SPECIES ? 'unclassified' : `#${id} (this world)`;
}

const NULL_AGENT = 0xffff_ffff;
const MAX_BIRTH_ID = (1n << 64n) - 2n;
const isBirthId = (value: unknown): value is BirthId =>
  value === null ||
  (typeof value === 'string' &&
    value.length <= 20 &&
    !/[^0-9]/.test(value) &&
    (value === '0' || /^[1-9]/.test(value)) &&
    BigInt(value) <= MAX_BIRTH_ID);

export function birthIdLabel(id: BirthId): string {
  if (!isBirthId(id)) throw new TypeError('invalid birth ID');
  return id === null ? 'unavailable' : `#${id} (this world)`;
}

export function parentSlotLabel(slot: number, parent: 'A' | 'B'): string {
  if (!isUint32(slot)) throw new TypeError('invalid parent slot');
  if (slot !== NULL_AGENT) return `slot #${slot} at birth (may be reused)`;
  return parent === 'A' ? '— (founder)' : '— (asexual)';
}

export function parentBirthLabel(id: BirthId, slot: number, parent: 'A' | 'B'): string {
  if (!isUint32(slot)) throw new TypeError('invalid parent slot');
  if (id !== null || slot !== NULL_AGENT) return birthIdLabel(id);
  return parentSlotLabel(slot, parent);
}

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
    isUint32(value.species_id) &&
    isBirthId(value.birth_id) &&
    isBirthId(value.parent_birth_a) &&
    isBirthId(value.parent_birth_b) &&
    isUint32(value.parent_a) &&
    isUint32(value.parent_b) &&
    Number.isSafeInteger(value.brain_units) &&
    isFiniteNumber(value.sensor_load) &&
    isFiniteNumber(value.health) &&
    value.health > 0 &&
    value.health <= 1 &&
    isFiniteNumber(value.muscle) &&
    isFiniteNumber(value.mouth) &&
    isFiniteNumber(value.eaten_plants) &&
    value.eaten_plants >= 0 &&
    isFiniteNumber(value.eaten_animals) &&
    value.eaten_animals >= 0 &&
    isUint32(value.kills) &&
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

/**
 * What an agent has eaten over its life: the share taken from other agents, by bite or
 * carrion, or "unfed" before its first meal (spec §7.9).
 */
export function dietLabel(plants: number, animals: number): string {
  // Scaled by the larger first, so two finite totals cannot overflow their sum.
  const larger = Math.max(plants, animals);
  if (!(larger > 0)) return 'unfed';
  const share = animals / larger / (plants / larger + animals / larger);
  const meat = Math.round(share * 100);
  return `${meat}% meat, ${100 - meat}% plants`;
}

export function summarizeGenes(genes: Gene[]): Array<{ kind: string; count: number }> {
  const counts = new Map<string, number>();
  for (const gene of genes) {
    const kind = Object.keys(gene)[0] ?? 'Unknown';
    counts.set(kind, (counts.get(kind) ?? 0) + 1);
  }
  return [...counts].map(([kind, count]) => ({ kind, count }));
}
