/** Completed-tick species observations, not retained ancestry or organism identity. */
export const NULL_SPECIES = 0xffff_ffff;

export interface SpeciesPopulation {
  id: number;
  population: number;
}

export interface SpeciesSnapshot {
  tick: bigint;
  population: number;
  populations: SpeciesPopulation[];
  unclassifiedPopulation: number;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export function isSpeciesSelection(value: unknown): value is number | null {
  return value === null || (typeof value === 'number' && Number.isInteger(value) &&
    value >= 0 && value <= NULL_SPECIES);
}

export function decodeSpeciesSnapshot(response: {
  tick: unknown; population: unknown; diagnostics: string;
}): SpeciesSnapshot {
  const data: unknown = JSON.parse(response.diagnostics);
  if (
    typeof response.tick !== 'string' || !/^(0|[1-9][0-9]{0,19})$/.test(response.tick) ||
    BigInt(response.tick) > 0xffff_ffff_ffff_ffffn ||
    typeof response.population !== 'number' || !Number.isInteger(response.population) ||
    response.population < 0 || response.population > NULL_SPECIES ||
    !isRecord(data) || !Array.isArray(data.populations) ||
    typeof data.unclassified_population !== 'number' ||
    !Number.isInteger(data.unclassified_population) || data.unclassified_population < 0 ||
    data.unclassified_population > response.population
  ) throw new TypeError('invalid species population payload');

  const population = response.population;
  let previous = -1;
  let total = data.unclassified_population;
  const populations = data.populations.map((row: unknown): SpeciesPopulation => {
    if (!isRecord(row) || typeof row.species_id !== 'number' ||
      !isSpeciesSelection(row.species_id) || row.species_id === NULL_SPECIES ||
      row.species_id <= previous || typeof row.population !== 'number' ||
      !Number.isInteger(row.population) || row.population < 1 ||
      row.population > population) throw new TypeError('invalid species population row');
    previous = row.species_id;
    total += row.population;
    return { id: row.species_id, population: row.population };
  });
  if (total !== population) throw new TypeError('species populations do not sum to population');
  return {
    tick: BigInt(response.tick), population, populations,
    unclassifiedPopulation: data.unclassified_population,
  };
}
