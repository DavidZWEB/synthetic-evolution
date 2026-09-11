/** Small, complete wire fixtures shared by codec and real IndexedDB tests. */
import { completionFor, createHeader } from './archive.js';

export function fixtureParams() {
  return {
    world: { size: 1000, max_agents: 32, dt: 1 / 60, founder_spread: 0.4 },
    storage: {
      genes_per_slot: 284, neurons_per_slot: 28, synapses_per_slot: 240, sensors_per_slot: 5,
      effectors_per_slot: 4, max_genes: 1024, max_neurons: 128, max_connections: 1024,
      max_sensors: 32, max_vision_rays: 32, max_effectors: 4, max_memory_bytes: 100663296,
    },
    body: { size: 3 },
    metabolism: { base: 0.05, k_size: 0.00125, k_brain: 0.00005, k_sensor: 0.000625, k_move: 0.5 },
    movement: { max_thrust: 1, max_turn_rate: 3, drag: 0.9, max_speed: 40 },
    sensing: { vision_range: 60, vision_fov: 0.5, vision_rays: 3, chemo_sensors: 1, energy_sensors: 1, chemo_radius: 40 },
    brain: {
      hidden_neurons: 6, oscillators: 2, connections_per_target: null, tau_min: 0.05,
      tau_max: 2, oscillator_period_min: 10, oscillator_period_max: 240, weight_init_scale: 2,
    },
    reproduction: { start_energy: 100, threshold: 200, gate: 0.5, energy_split: 0.5, spawn_radius: 8, maturity_ticks: 300 },
    mutation: {
      structural: {
        remove_connection_rate: 0, remove_neuron_rate: 0, toggle_connection_rate: 0,
        add_connection_rate: 0, add_neuron_rate: 0, split_neuron_bias: 0, split_input_weight: 1,
      },
      organs: {
        remove_sensor_rate: 0, add_sensor_rate: 0, vision_weight: 1, chemo_weight: 1,
        energy_weight: 1, neuron_bias: 0,
      },
      weight_perturb_rate: 0.025, weight_perturb_sigma: 0.15, weight_reset_rate: 0.0015625,
      weight_limit: 4, neuron_perturb_rate: 0.00625, bias_perturb_sigma: 0.1, tau_perturb_factor: 0.1,
    },
    distance: { disjoint_coefficient: 1, excess_coefficient: 1, weight_coefficient: 0.4 },
    species: { capacity: 16, threshold: 0.5 },
    feeding: { rate: 1, gate: 0.5, reach: 4 },
    plants: { energy_input_rate: 12000, max_plants: 16, max_energy: 60, radius: 2, scent_rate: 0.02, initial_fill: 1, signature: [0.2, 0.8, 0.25] },
    chemo: { cells: [8, 8, 1], decay: [0.98], diffuse: 0.1 },
  };
}

export function fixtureHeader() {
  return createHeader({
    runId: 'fixture-run', seed: '18446744073709551615', founders: 4, params: fixtureParams(),
    simVersion: '0.1.0', sourceRevision: 'fixture-development',
  });
}

export function origin(sequence = '0', species = 0, birth = '0', cohort = 'evolving', tick = '0') {
  return {
    kind: 'event',
    data: {
      cohort, sequence, tick,
      event: {
        kind: 'species_origin', species_id: species, founder_birth_id: birth,
        parent_a: { status: 'absent' }, parent_b: { status: 'absent' },
      },
    },
  };
}

export function gap(first = '0', last = '0', cohort = 'evolving') {
  return { kind: 'gap', data: { cohort, first_sequence: first, last_sequence: last } };
}

export function extinct(sequence = '1', species = 0, tick = '0', cohort = 'evolving') {
  return { kind: 'event', data: { cohort, sequence, tick, event: { kind: 'species_extinct', species_id: species } } };
}

export function fixtureArchive(version = 2) {
  const header = fixtureHeader();
  const rows = [origin()];
  if (version === 1) {
    header.data.schema_version = 1;
    delete header.data.run_id;
    header.data.cohorts = ['evolving', 'random_control'];
    header.data.ticks = '1';
    header.data.drain_every = '1';
    rows.push(origin('0', 0, '0', 'random_control'));
  }
  const completion = completionFor(header, rows, {
    tick: '1', captureEnd: 'snapshot', stateHash: '0123456789abcdef',
  });
  return { header, rows, completion };
}
