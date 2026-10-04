/** Wire structure and complete parameter fields; numerical simulation policy stays in WASM. */

export const U64_MAX = 18446744073709551615n;
export const U32_MAX = 4294967295;

export function requireThat(condition, message) {
  if (!condition) throw new Error(`Invalid history: ${message}`);
}

export function object(value, keys, label) {
  requireThat(value !== null && typeof value === 'object' && !Array.isArray(value), `${label} must be an object`);
  const actual = Object.keys(value);
  requireThat(actual.length === keys.length && keys.every((key) => Object.hasOwn(value, key)),
    `${label} has missing or unknown fields`);
}

export function decimal(value, label = 'counter') {
  requireThat(typeof value === 'string' && /^(0|[1-9][0-9]{0,19})$/.test(value),
    `${label} must be a canonical decimal u64 string`);
  const result = BigInt(value);
  requireThat(result <= U64_MAX, `${label} exceeds u64`);
  return result;
}

export function uint(value, label, max = U32_MAX) {
  requireThat(Number.isSafeInteger(value) && !Object.is(value, -0) && value >= 0 && value <= max,
    `${label} must be an unsigned integer`);
}

export function birthId(value) {
  if (value === null) return null;
  const id = decimal(value, 'birth ID');
  requireThat(id !== U64_MAX, 'unavailable birth ID must be null');
  return id;
}

export function speciesId(value) {
  uint(value, 'species ID', U32_MAX - 1);
  return value;
}

export function equal(a, b) {
  if (a === b) return true;
  if (!a || !b || typeof a !== 'object' || typeof b !== 'object') return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const keys = Object.keys(a);
  return keys.length === Object.keys(b).length &&
    keys.every((key) => Object.hasOwn(b, key) && equal(a[key], b[key]));
}

const f = 'finite';
const u = 'u32';
// This is the wire shape, not defaults. A missing field must never acquire a future default.
const PARAMS = {
  world: { size: f, max_agents: u, dt: f, founder_spread: f },
  storage: {
    genes_per_slot: u, neurons_per_slot: u, synapses_per_slot: u, sensors_per_slot: u,
    effectors_per_slot: u, max_genes: u, max_neurons: u, max_connections: u,
    max_sensors: u, max_vision_rays: u, max_effectors: u, max_memory_bytes: 'integer',
  },
  body: { size: f },
  metabolism: { base: f, k_size: f, k_brain: f, k_sensor: f, k_move: f },
  movement: { max_thrust: f, max_turn_rate: f, drag: f, max_speed: f },
  sensing: {
    vision_range: f, vision_fov: f, vision_rays: u, chemo_sensors: u,
    energy_sensors: u, chemo_radius: f,
  },
  brain: {
    hidden_neurons: u, oscillators: u, connections_per_target: 'nullable-u32',
    tau_min: f, tau_max: f, oscillator_period_min: f, oscillator_period_max: f, weight_init_scale: f,
  },
  reproduction: { start_energy: f, threshold: f, gate: f, energy_split: f, spawn_radius: f, maturity_ticks: u },
  mutation: {
    structural: {
      remove_connection_rate: f, remove_neuron_rate: f, toggle_connection_rate: f,
      add_connection_rate: f, add_neuron_rate: f, add_oscillator_rate: f,
      split_neuron_bias: f, split_input_weight: f,
    },
    organs: {
      remove_sensor_rate: f, add_sensor_rate: f, vision_weight: f, chemo_weight: f,
      energy_weight: f, neuron_bias: f,
    },
    weight_perturb_rate: f, weight_perturb_sigma: f, weight_reset_rate: f, weight_limit: f,
    neuron_perturb_rate: f, bias_perturb_sigma: f, tau_perturb_factor: f,
  },
  distance: { disjoint_coefficient: f, excess_coefficient: f, weight_coefficient: f },
  species: { capacity: u, threshold: f },
  feeding: { rate: f, gate: f, reach: f },
  plants: {
    energy_input_rate: f, max_plants: u, max_energy: f, grazing_lag: f, patchiness: f,
    patch_scale: f, death_stock: f, death_seconds: f, local_dispersal: f,
    dispersal_radius: f, radius: f, scent_rate: f, initial_fill: f, signature: ['finite', 3],
  },
  corpses: {
    energy_fraction: f, decay: f, min_energy: f, max_corpses: u, radius: f,
    signature: ['finite', 3],
  },
  chemo: { cells: ['u32', 3], decay: ['finite'], diffuse: f },
};

/**
 * Params fields added after archives were first written. An older archive omits them
 * because it ran without them, which their zero default describes exactly.
 */
const LATER_PARAMS = new Set([
  'add_oscillator_rate', 'grazing_lag', 'patchiness', 'patch_scale', 'death_stock',
  'death_seconds', 'local_dispersal', 'dispersal_radius', 'corpses',
]);

export function integerParam(path) {
  let shape = PARAMS;
  for (const key of path) shape = Array.isArray(shape) ? shape[0] : shape?.[key];
  return ['u32', 'nullable-u32', 'integer'].includes(shape);
}

export function validateParamsShape(value, shape = PARAMS, path = 'params') {
  if (Array.isArray(shape)) {
    requireThat(Array.isArray(value) && (shape[1] === undefined || value.length === shape[1]), `${path} is not a valid array`);
    value.forEach((item) => validateParamsShape(item, shape[0], path));
  } else if (typeof shape === 'object') {
    const keys = Object.keys(shape).filter((key) => !LATER_PARAMS.has(key) || Object.hasOwn(value ?? {}, key));
    object(value, keys, path);
    for (const key of keys) validateParamsShape(value[key], shape[key], `${path}.${key}`);
  } else if (shape === f) {
    requireThat(typeof value === 'number' && Number.isFinite(value), `${path} must be finite`);
  } else if (shape !== 'nullable-u32' || value !== null) {
    uint(value, path, shape === 'integer' ? Number.MAX_SAFE_INTEGER : U32_MAX);
  }
}
