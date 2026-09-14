/** Per-cohort chronological validation, with lifecycle knowledge explicitly limited by gaps. */
import { birthId, decimal, object, requireThat, speciesId, U64_MAX } from './shape.js';

export const COUNT_KEYS = ['next_sequence', 'events', 'dropped_events', 'gaps', 'origins', 'extinctions'];

export function newState() {
  return {
    counts: Object.fromEntries(COUNT_KEYS.map((key) => [key, 0n])),
    lastTick: null, greatestSpecies: null, lastFounder: null,
    birthIdsExhausted: false, active: new Map(), hasGap: false,
  };
}

export function wireCounts(state) {
  return Object.fromEntries(COUNT_KEYS.map((key) => [key, String(state.counts[key])]));
}

function parent(value, species, founder) {
  object(value, value?.status === 'observed' ? ['status', 'birth_id', 'species_id'] : ['status'], 'parent');
  requireThat(['absent', 'unavailable', 'observed'].includes(value.status), 'unknown parent status');
  if (value.status !== 'observed') return null;
  const birth = birthId(value.birth_id);
  const id = value.species_id === null ? null : speciesId(value.species_id);
  requireThat(id === null || id < species, 'invalid observed parent species');
  requireThat(founder === null || (birth !== null && birth < founder), 'invalid observed parent birth ID');
  return { birth, species: id };
}

function origin(state, event, params, boundary, founders) {
  object(event, ['kind', 'species_id', 'founder_birth_id', 'parent_a', 'parent_b'], 'origin');
  const species = speciesId(event.species_id);
  const founder = birthId(event.founder_birth_id);
  requireThat(state.greatestSpecies === null || species > state.greatestSpecies, 'invalid or reused species origin identity');
  requireThat(founder === null || (!state.birthIdsExhausted &&
    (state.lastFounder === null || founder > state.lastFounder)), 'invalid or reused founder birth identity');
  const a = parent(event.parent_a, species, founder);
  const b = parent(event.parent_b, species, founder);
  for (const observed of [a, b]) {
    if (!observed) continue;
    if (observed.species !== null) {
      if (state.active.has(observed.species)) {
        const birth = state.active.get(observed.species);
        requireThat(observed.birth === null || (birth !== null && observed.birth >= birth),
          'observed parent predates its species origin');
      } else {
        requireThat(state.hasGap, 'observed parent species has no active origin');
      }
    }
    if (observed.birth !== null) {
      for (const [id, birth] of state.active) {
        requireThat(birth !== observed.birth || id === observed.species,
          'observed parent contradicts its recorded species origin');
      }
    }
  }
  requireThat(!a || !b || a.birth === null || a.birth !== b.birth, 'same observed parent named twice');
  requireThat(boundary !== 0n || (event.parent_a.status === 'absent' && event.parent_b.status === 'absent'),
    'a seeding-only run cannot have parental origins');
  // BirthIds start at zero and refused admissions consume no ID (spec §3.4).
  requireThat(boundary !== 0n || (founder !== null && founder < BigInt(founders)),
    'a seeding-only origin requires an available birth ID below the founder count');
  state.greatestSpecies = species;
  if (founder === null) state.birthIdsExhausted = true;
  else state.lastFounder = founder;
  state.active.set(species, founder);
  requireThat(state.active.size <= params.species.capacity, 'active species exceed classification capacity');
  state.counts.origins++;
}

export function acceptRow(state, row, params, boundary, founders) {
  const data = row.data;
  requireThat(params.species.capacity > 0, 'disabled classification cannot emit history');
  if (row.kind === 'gap') {
    object(data, ['cohort', 'first_sequence', 'last_sequence'], 'gap data');
    const first = decimal(data.first_sequence);
    const last = decimal(data.last_sequence);
    requireThat(first === state.counts.next_sequence && last >= first && last !== U64_MAX,
      'history gap range is invalid or not contiguous');
    state.counts.next_sequence = last + 1n;
    state.counts.dropped_events += last - first + 1n;
    state.counts.gaps++;
    state.hasGap = true;
    // A lost extinction could have retired any known origin (spec §3.4).
    state.active.clear();
    return;
  }
  requireThat(row.kind === 'event', 'expected an event or gap');
  object(data, ['cohort', 'sequence', 'tick', 'event'], 'event data');
  const sequence = decimal(data.sequence);
  const tick = decimal(data.tick, 'event tick');
  requireThat(sequence === state.counts.next_sequence && sequence !== U64_MAX,
    'history event sequence is not contiguous');
  requireThat((boundary === null || tick <= (boundary === 0n ? 0n : boundary - 1n)) &&
    (state.lastTick === null || tick >= state.lastTick), 'history event tick is out of order or outside the run');
  requireThat(data.event && typeof data.event === 'object', 'event must be an object');
  if (data.event.kind === 'species_origin') {
    origin(state, data.event, params, boundary, founders);
  } else {
    object(data.event, ['kind', 'species_id'], 'extinction');
    requireThat(data.event.kind === 'species_extinct', 'unknown species event kind');
    const species = speciesId(data.event.species_id);
    requireThat(boundary !== 0n, 'a seeding-only run cannot have extinctions');
    requireThat(state.active.delete(species) || state.hasGap, 'extinction without an active observed origin');
    state.greatestSpecies = Math.max(state.greatestSpecies ?? species, species);
    state.counts.extinctions++;
  }
  state.counts.next_sequence = sequence + 1n;
  state.counts.events++;
  state.lastTick = tick;
}
