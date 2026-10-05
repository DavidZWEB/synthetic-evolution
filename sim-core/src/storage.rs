//! Allocation-free validation of one world's eager storage layout (spec §2.2a).
//!
//! Accounts for portable requested heap bytes, including constructor temporaries, before
//! reserving anything. It does not choose population sizes, grow arenas, or promise
//! that the host can satisfy an otherwise valid reservation.

use glam::{Quat, Vec3};

use crate::arena::Block;
use crate::brain::{Neuron, Synapse};
use crate::effectors::Effector;
use crate::founder::{FounderCounts, FounderPlan};
use crate::genome::Gene;
use crate::ids::{AgentId, BirthId, InnovationId};
use crate::params::{ParamError, SimParams};
use crate::perceive::Sensor;
use crate::species::Classifier;

/// WASM32's `Vec` byte limit also applies on native, before attempting allocations.
const PORTABLE_BUFFER_BYTES: u64 = i32::MAX as u64;

#[derive(Debug)]
pub(crate) struct StorageLayout {
    pub(crate) genes: u32,
    pub(crate) neurons: u32,
    pub(crate) synapses: u32,
    pub(crate) sensors: u32,
    pub(crate) effectors: u32,
    pub(crate) construction_bytes: u64,
    pub(crate) species_bytes: u64,
}

impl StorageLayout {
    /// Called by parameter validation, so must never call `validate` or build a plan.
    pub(crate) fn new(params: &SimParams) -> Result<Self, ParamError> {
        let storage = &params.storage;
        let agents = params.world.max_agents;
        let aggregate =
            |allowance: u32, message| allowance.checked_mul(agents).ok_or(ParamError(message));
        let mut layout = Self {
            genes: aggregate(
                storage.genes_per_slot,
                "storage.genes_per_slot times world.max_agents exceeds u32 capacity",
            )?,
            neurons: aggregate(
                storage.neurons_per_slot,
                "storage.neurons_per_slot times world.max_agents exceeds u32 capacity",
            )?,
            synapses: aggregate(
                storage.synapses_per_slot,
                "storage.synapses_per_slot times world.max_agents exceeds u32 capacity",
            )?,
            sensors: aggregate(
                storage.sensors_per_slot,
                "storage.sensors_per_slot times world.max_agents exceeds u32 capacity",
            )?,
            effectors: aggregate(
                storage.effectors_per_slot,
                "storage.effectors_per_slot times world.max_agents exceeds u32 capacity",
            )?,
            construction_bytes: 0,
            species_bytes: Classifier::estimated_construction_bytes(
                params.species.capacity,
                storage.max_genes,
            )?,
        };
        let founder = FounderPlan::checked_counts(params).ok_or(ParamError(
            "founding topology exceeds representable gene counts",
        ))?;
        for (count, limit, message) in [
            (
                founder.genes,
                storage.max_genes,
                "founder exceeds storage.max_genes",
            ),
            (
                founder.neurons,
                storage.max_neurons,
                "founder exceeds storage.max_neurons",
            ),
            (
                founder.synapses,
                storage.max_connections,
                "founder exceeds storage.max_connections",
            ),
            (
                founder.sensors,
                storage.max_sensors,
                "founder exceeds storage.max_sensors",
            ),
            (
                params.sensing.vision_rays,
                storage.max_vision_rays,
                "founder exceeds storage.max_vision_rays",
            ),
            (
                founder.effectors,
                storage.max_effectors,
                "founder exceeds storage.max_effectors",
            ),
            (
                founder.genes,
                layout.genes,
                "storage.genes_per_slot times world.max_agents cannot hold one founder",
            ),
            (
                founder.neurons,
                layout.neurons,
                "storage.neurons_per_slot times world.max_agents cannot hold one founder",
            ),
            (
                founder.synapses,
                layout.synapses,
                "storage.synapses_per_slot times world.max_agents cannot hold one founder",
            ),
            (
                founder.sensors,
                layout.sensors,
                "storage.sensors_per_slot times world.max_agents cannot hold one founder",
            ),
            (
                founder.effectors,
                layout.effectors,
                "storage.effectors_per_slot times world.max_agents cannot hold one founder",
            ),
        ] {
            if count > limit {
                return Err(ParamError(message));
            }
        }

        let mut requests = AllocationRequests {
            bytes: layout.species_bytes,
        };
        requests.variable_arena::<Gene>(layout.genes, agents).map_err(|_| ParamError(
            "storage.genes_per_slot produces an arena buffer exceeding the portable byte ceiling",
        ))?;
        requests.variable_arena::<Neuron>(layout.neurons, agents).map_err(|_| ParamError(
            "storage.neurons_per_slot produces an arena buffer exceeding the portable byte ceiling",
        ))?;
        requests.variable_arena::<Synapse>(layout.synapses, agents).map_err(|_| ParamError(
            "storage.synapses_per_slot produces an arena buffer exceeding the portable byte ceiling",
        ))?;
        requests.variable_arena::<Sensor>(layout.sensors, agents).map_err(|_| ParamError(
            "storage.sensors_per_slot produces an arena buffer exceeding the portable byte ceiling",
        ))?;
        requests.variable_arena::<Effector>(layout.effectors, agents).map_err(|_| ParamError(
            "storage.effectors_per_slot produces an arena buffer exceeding the portable byte ceiling",
        ))?;
        // Cached reset values are inline Copy data, not additional heap allocations.
        requests.agent_storage(u64::from(agents))?;
        requests.buffers::<BirthId>(u64::from(agents), 3)?;
        requests.founder_plan(params, &founder)?;
        requests.buffer::<Gene>(u64::from(storage.max_genes)).map_err(|_| ParamError(
            "storage.max_genes produces a scratch buffer exceeding the portable byte ceiling",
        ))?;
        requests.buffer::<u32>(u64::from(storage.max_neurons)).map_err(|_| ParamError(
            "storage.max_neurons produces a scratch buffer exceeding the portable byte ceiling",
        ))?;

        let ratio = params.world.size / params.sensing.max_sense_radius();
        if !ratio.is_finite() || ratio <= 0.0 || ratio >= u32::MAX as f32 {
            return Err(ParamError(
                "spatial grid axis exceeds representable cell counts",
            ));
        }
        // Match SpatialHash::new's f32 division and truncation, not a rounded or f64 grid.
        let per_axis = u64::from((ratio as u32).max(1));
        let grid_cells = per_axis
            .checked_mul(per_axis)
            .ok_or(ParamError("spatial grid exceeds representable cell counts"))?;
        requests.spatial_hash(grid_cells, u64::from(agents))?;
        requests.plants(
            grid_cells,
            u64::from(params.plants.max_plants),
            u64::from(crate::fertility::Fertility::cells_per_axis(params)),
        )?;
        requests.corpses(grid_cells, u64::from(params.corpses.max_corpses))?;

        let field_cells = params
            .chemo
            .cells
            .iter()
            .try_fold(params.chemo.channels() as u64, |total, &axis| {
                total.checked_mul(u64::from(axis))
            })
            .ok_or(ParamError("chemo grid times channels overflows"))?;
        requests.buffers::<f32>(field_cells, 2)?;
        // Params move into World; include their already-owned channel allocation too.
        requests.buffer::<f32>(params.chemo.decay.capacity() as u64)?;

        layout.construction_bytes = requests.bytes;
        if layout.construction_bytes > storage.max_memory_bytes {
            return Err(ParamError(
                "estimated core construction exceeds storage.max_memory_bytes",
            ));
        }
        Ok(layout)
    }
}

/// Inventory of actual constructor buffers. Repetition counts name parallel arrays,
/// not an empirical multiplier; each buffer gets its own portable-size check.
#[derive(Default)]
struct AllocationRequests {
    bytes: u64,
}

impl AllocationRequests {
    fn buffer<T>(&mut self, count: u64) -> Result<(), ParamError> {
        self.buffers::<T>(count, 1)
    }

    fn buffers<T>(&mut self, count: u64, copies: u64) -> Result<(), ParamError> {
        let bytes = count
            .checked_mul(size_of::<T>() as u64)
            .ok_or(ParamError("construction buffer byte count overflows"))?;
        if bytes > PORTABLE_BUFFER_BYTES {
            return Err(ParamError(
                "construction buffer exceeds the portable byte ceiling",
            ));
        }
        self.bytes = bytes
            .checked_mul(copies)
            .and_then(|bytes| self.bytes.checked_add(bytes))
            .ok_or(ParamError("total construction byte count overflows"))?;
        Ok(())
    }

    fn variable_arena<T>(&mut self, elements: u32, max_blocks: u32) -> Result<(), ParamError> {
        self.buffer::<T>(u64::from(elements))?;
        // Match VariableArena: min(elements, blocks) separating live blocks, plus
        // one free span. Empty backing storage does not reserve metadata (spec §2.2a).
        let spans = if elements == 0 {
            0
        } else {
            u64::from(elements.min(max_blocks)) + 1
        };
        self.buffer::<Block>(spans)
    }

    fn agent_storage(&mut self, agents: u64) -> Result<(), ParamError> {
        // SlotPool: alive, incarnation, free.
        self.buffer::<u8>(agents)?;
        self.buffers::<u32>(agents, 2)?;
        // Agents: position/velocity/signature, orientation, energy reserve and two
        // diet totals, energy/health/size/muscle/mouth/sensor load,
        // age/species/parents/grid/brain units/bite cooldown, handles.
        self.buffers::<Vec3>(agents, 3)?;
        self.buffer::<Quat>(agents)?;
        self.buffers::<f64>(agents, 3)?;
        self.buffers::<f32>(agents, 6)?;
        self.buffers::<u32>(agents, 7)?;
        self.buffers::<Block>(agents, 6)?;
        // Intents: thrust, turn, ingest, reproduce, and the bite's drive, azimuth, and
        // reach.
        self.buffers::<f32>(agents, 7)?;
        // Fixed-stride parts: one f32 and one free-list index per slot (spec §9.1).
        self.buffer::<f32>(agents)?;
        self.buffer::<u32>(agents)?;
        // Deferred deaths/births. Command queues start empty and request no storage.
        self.buffers::<AgentId>(agents, 2)?;
        // Step 7's fixed swings, at most one per agent.
        self.buffer::<crate::combat::Swing>(agents)
    }

    fn founder_plan(
        &mut self,
        params: &SimParams,
        counts: &FounderCounts,
    ) -> Result<(), ParamError> {
        let genes = u64::from(counts.genes);
        let neurons = u64::from(counts.neurons);
        self.buffer::<Gene>(genes)?;
        self.buffer::<f32>(u64::from(counts.synapses))?;
        // Constructor-only neuron IDs, source/sink slots, fan-in counts and targets.
        // Sparse wiring shuffles the same source-slot buffer; it neither reserves
        // dense edges nor adds per-target selection scratch (spec §3.3).
        self.buffer::<InnovationId>(neurons)?;
        // Charge usize source/sink/target indices at 8 bytes on both native and
        // WASM32, so pointer width alone cannot change budget acceptance (spec §2.2a).
        self.buffer::<u64>(neurons - u64::from(counts.effectors))?;
        self.buffer::<u64>(u64::from(params.brain.hidden_neurons) + u64::from(counts.effectors))?;
        self.buffer::<u32>(neurons)?;
        self.buffer::<u64>(genes)?;
        // Stable sort may request up to a full-length auxiliary array. Small inputs
        // sort on the stack; charging the full array is a topology-sized upper bound,
        // not a general safety factor for unaccounted allocations.
        self.buffer::<Gene>(genes)
    }

    fn spatial_hash(&mut self, cells: u64, entries: u64) -> Result<(), ParamError> {
        let starts = cells
            .checked_add(1)
            .ok_or(ParamError("spatial grid sentinel overflows"))?;
        self.buffer::<u32>(starts)?;
        self.buffer::<u32>(cells)?;
        self.buffer::<u32>(entries)
    }

    /// Corpse slots: position, energy pair, liveness, free list, and grid scratch,
    /// with a grid of their own (spec §5.1).
    fn corpses(&mut self, cells: u64, corpses: u64) -> Result<(), ParamError> {
        // No slots, no grid: a world without corpses is charged nothing for them.
        if corpses == 0 {
            return Ok(());
        }
        self.buffer::<Vec3>(corpses)?;
        self.buffer::<f32>(corpses)?;
        self.buffer::<f64>(corpses)?;
        self.buffer::<u8>(corpses)?;
        self.buffers::<u32>(corpses, 2)?;
        self.spatial_hash(cells, corpses)
    }

    fn plants(&mut self, cells: u64, plants: u64, fertility_cells: u64) -> Result<(), ParamError> {
        // The fertility lattice is kept for reseeding (spec §5.1).
        self.buffer::<f32>(
            fertility_cells
                .checked_mul(fertility_cells)
                .ok_or(ParamError("fertility lattice size overflows"))?,
        )?;
        self.buffer::<Vec3>(plants)?;
        self.buffer::<f32>(plants)?;
        self.buffer::<f64>(plants)?;
        self.buffer::<u8>(plants)?;
        // Grid-cell scratch, kept for rebuilding after reseeds, and starvation counts.
        self.buffers::<u32>(plants, 2)?;
        self.spatial_hash(cells, plants)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction_charges_the_classifier_and_lifetime_identities() {
        for agents in [2, 17, 5_000] {
            for capacity in [0, 1, 256] {
                let mut params = SimParams::default();
                params.world.max_agents = agents;
                params.species.capacity = capacity;
                let layout = StorageLayout::new(&params).unwrap();
                assert_eq!(
                    layout.species_bytes,
                    Classifier::estimated_construction_bytes(capacity, params.storage.max_genes)
                        .unwrap()
                );
                params.species.capacity = 0;
                let without = StorageLayout::new(&params).unwrap();
                assert_eq!(
                    layout.construction_bytes - without.construction_bytes,
                    layout.species_bytes - without.species_bytes
                );
            }
        }
    }

    #[test]
    fn defaults_use_pooled_allowances_and_fit_the_budget() {
        let params = SimParams::default();
        let layout = StorageLayout::new(&params).unwrap();
        assert_eq!(layout.genes, 284 * params.world.max_agents);
        assert_eq!(layout.neurons, 28 * params.world.max_agents);
        assert_eq!(layout.synapses, 240 * params.world.max_agents);
        assert_eq!(layout.sensors, 5 * params.world.max_agents);
        assert_eq!(layout.effectors, 4 * params.world.max_agents);
        assert!(layout.construction_bytes < params.storage.max_memory_bytes);
        assert_eq!(
            params.estimated_construction_bytes().unwrap(),
            layout.construction_bytes
        );
    }

    #[test]
    fn founder_index_buffers_have_the_same_budget_on_native_and_wasm() {
        for fan_in in [None, Some(0), Some(1), Some(24), Some(u32::MAX)] {
            for (rays, chemo, energy) in [(3, 1, 1), (0, 0, 0), (0, 2, 3)] {
                let mut params = SimParams::default();
                params.brain.connections_per_target = fan_in;
                params.sensing.vision_rays = rays;
                params.sensing.chemo_sensors = chemo;
                params.sensing.energy_sensors = energy;
                let counts = FounderPlan::checked_counts(&params).unwrap();
                let mut requests = AllocationRequests::default();
                requests.founder_plan(&params, &counts).unwrap();
                let genes = u64::from(counts.genes);
                let neurons = u64::from(counts.neurons);
                let source_indices = neurons - u64::from(counts.effectors);
                let sink_indices =
                    u64::from(params.brain.hidden_neurons) + u64::from(counts.effectors);
                let expected = 2 * genes * size_of::<Gene>() as u64
                    + u64::from(counts.synapses) * size_of::<f32>() as u64
                    + neurons * (size_of::<InnovationId>() + size_of::<u32>()) as u64
                    + (source_indices + sink_indices + genes) * size_of::<u64>() as u64;
                assert_eq!(requests.bytes, expected);
            }
        }
    }

    #[test]
    fn changing_founder_composition_does_not_resize_the_arenas() {
        let params = SimParams::default();
        let before = StorageLayout::new(&params).unwrap();
        let mut changed = params.clone();
        changed.sensing.vision_rays += 1;
        changed.sensing.chemo_sensors += 1;
        changed.sensing.energy_sensors += 1;
        changed.brain.hidden_neurons += 1;
        let after = StorageLayout::new(&changed).unwrap();
        assert_eq!(
            (
                before.genes,
                before.neurons,
                before.synapses,
                before.sensors,
                before.effectors
            ),
            (
                after.genes,
                after.neurons,
                after.synapses,
                after.sensors,
                after.effectors
            ),
        );
        assert!(
            after.construction_bytes > before.construction_bytes,
            "the template still grows"
        );
    }

    #[test]
    fn budget_boundary_is_exact_and_can_be_explicitly_raised() {
        let mut params = SimParams::default();
        let bytes = params.estimated_construction_bytes().unwrap();
        params.storage.max_memory_bytes = bytes;
        params.validate().unwrap();
        params.storage.max_memory_bytes -= 1;
        assert_eq!(
            params.validate().unwrap_err(),
            ParamError("estimated core construction exceeds storage.max_memory_bytes"),
        );
        params.storage.max_memory_bytes = bytes * 2;
        params.world.max_agents *= 2;
        params.validate().unwrap();
    }

    #[test]
    fn zero_sensor_and_connection_arenas_fit_an_empty_source_template() {
        let mut params = SimParams::default();
        params.sensing.vision_rays = 0;
        params.sensing.chemo_sensors = 0;
        params.sensing.energy_sensors = 0;
        params.brain.hidden_neurons = 0;
        params.brain.oscillators = 0;
        params.storage.sensors_per_slot = 0;
        params.storage.synapses_per_slot = 0;
        params.storage.max_sensors = 0;
        params.storage.max_vision_rays = 0;
        params.storage.max_connections = 0;
        let layout = StorageLayout::new(&params).unwrap();
        assert_eq!(layout.sensors, 0);
        assert_eq!(layout.synapses, 0);
        params.validate().unwrap();
    }

    #[test]
    fn larger_worlds_require_an_explicit_budget_increase() {
        let mut params = SimParams::default();
        params.world.max_agents *= 2;
        assert_eq!(
            params.validate().unwrap_err(),
            ParamError("estimated core construction exceeds storage.max_memory_bytes"),
        );
        params.storage.max_memory_bytes *= 2;
        params.validate().unwrap();
    }

    #[test]
    fn checked_buffers_replace_arbitrary_population_and_grid_ceilings() {
        let mut params = SimParams::default();
        params.storage.max_memory_bytes = u64::MAX;
        params.world.max_agents = 1_000_001;
        params.storage.genes_per_slot = 1;
        params.storage.neurons_per_slot = 1;
        params.storage.synapses_per_slot = 1;
        params.storage.sensors_per_slot = 1;
        params.storage.effectors_per_slot = 1;
        params.validate().unwrap();

        params.world.max_agents = 1_000;
        params.sensing.vision_range = 0.2;
        params.sensing.chemo_radius = 0.2;
        params.validate().unwrap();
    }

    #[test]
    fn aggregate_arithmetic_and_portable_data_limits_are_checked() {
        let mut params = SimParams::default();
        params.storage.genes_per_slot = u32::MAX;
        assert_eq!(
            params.validate().unwrap_err(),
            ParamError("storage.genes_per_slot times world.max_agents exceeds u32 capacity"),
        );
        params.world.max_agents = 1;
        params.storage.max_memory_bytes = u64::MAX;
        assert_eq!(
            params.validate().unwrap_err(),
            ParamError(
                "storage.genes_per_slot produces an arena buffer exceeding the portable byte ceiling"
            ),
        );
        params.storage.genes_per_slot = StorageLayout::new(&SimParams::default()).unwrap().genes;
        params.storage.max_genes = u32::MAX;
        assert!(
            params.validate().is_err(),
            "scratch must be checked independently"
        );
    }

    #[test]
    fn metadata_is_checked_even_when_the_elements_fit() {
        let mut requests = AllocationRequests::default();
        let elements = (PORTABLE_BUFFER_BYTES / size_of::<Block>() as u64) as u32;
        assert!(u64::from(elements) < PORTABLE_BUFFER_BYTES);
        assert_eq!(
            requests
                .variable_arena::<u8>(elements, elements)
                .unwrap_err(),
            ParamError("construction buffer exceeds the portable byte ceiling"),
        );
    }

    #[test]
    fn metadata_reservation_is_bounded_by_both_elements_and_blocks() {
        for (elements, blocks, spans) in [(0, 100, 0), (3, 100, 4), (100, 3, 4), (100, 0, 1)] {
            let mut requests = AllocationRequests::default();
            requests.variable_arena::<u8>(elements, blocks).unwrap();
            assert_eq!(
                requests.bytes,
                u64::from(elements) + spans * size_of::<Block>() as u64
            );
        }
    }

    #[test]
    fn every_founder_limit_is_checked_independently() {
        type Case = (fn(&mut SimParams, &FounderCounts), &'static str);
        let cases: [Case; 6] = [
            (
                |p, c| p.storage.max_genes = c.genes - 1,
                "founder exceeds storage.max_genes",
            ),
            (
                |p, c| p.storage.max_neurons = c.neurons - 1,
                "founder exceeds storage.max_neurons",
            ),
            (
                |p, c| p.storage.max_connections = c.synapses - 1,
                "founder exceeds storage.max_connections",
            ),
            (
                |p, c| p.storage.max_sensors = c.sensors - 1,
                "founder exceeds storage.max_sensors",
            ),
            (
                // The shipped founder has no eyes; give it one to exceed a zero limit.
                |p, _| {
                    p.sensing.vision_rays = 1;
                    p.storage.max_vision_rays = 0;
                },
                "founder exceeds storage.max_vision_rays",
            ),
            (
                |p, c| p.storage.max_effectors = c.effectors - 1,
                "founder exceeds storage.max_effectors",
            ),
        ];
        for (change, message) in cases {
            let mut params = SimParams::default();
            let counts = FounderPlan::checked_counts(&params).unwrap();
            change(&mut params, &counts);
            assert_eq!(params.validate().unwrap_err(), ParamError(message));
        }
    }

    #[test]
    fn every_arena_must_hold_at_least_one_founder() {
        type Case = (fn(&mut SimParams, &FounderCounts), &'static str);
        let cases: [Case; 5] = [
            (
                |p, c| p.storage.genes_per_slot = c.genes - 1,
                "storage.genes_per_slot times world.max_agents cannot hold one founder",
            ),
            (
                |p, c| p.storage.neurons_per_slot = c.neurons - 1,
                "storage.neurons_per_slot times world.max_agents cannot hold one founder",
            ),
            (
                |p, c| p.storage.synapses_per_slot = c.synapses - 1,
                "storage.synapses_per_slot times world.max_agents cannot hold one founder",
            ),
            (
                |p, c| p.storage.sensors_per_slot = c.sensors - 1,
                "storage.sensors_per_slot times world.max_agents cannot hold one founder",
            ),
            (
                |p, c| p.storage.effectors_per_slot = c.effectors - 1,
                "storage.effectors_per_slot times world.max_agents cannot hold one founder",
            ),
        ];
        for (change, message) in cases {
            let mut params = SimParams::default();
            // Two sensors, so one fewer per slot still pools enough for two agents.
            params.sensing.chemo_sensors = 2;
            let counts = FounderPlan::checked_counts(&params).unwrap();
            params.world.max_agents = 1;
            change(&mut params, &counts);
            assert_eq!(params.validate().unwrap_err(), ParamError(message));
            params.world.max_agents = 2;
            params
                .validate()
                .expect("allowances are pooled, not per-genome maxima");
        }
    }

    #[test]
    fn a_patchy_world_charges_its_fertility_lattice() {
        let mut uniform = SimParams::default();
        uniform.plants.patchiness = 0.0;
        let mut patchy = uniform.clone();
        patchy.plants.patchiness = 2.0;
        patchy.plants.patch_scale = 100.0;
        let lattice = 10 * 10 * size_of::<f32>() as u64;
        assert_eq!(
            StorageLayout::new(&patchy).unwrap().construction_bytes,
            StorageLayout::new(&uniform).unwrap().construction_bytes + lattice
        );
    }

    #[test]
    fn fields_and_spatial_indices_have_independent_portable_checks() {
        let mut params = SimParams::default();
        params.storage.max_memory_bytes = u64::MAX;
        params.chemo.cells = [u32::MAX, 1, 1];
        assert_eq!(
            params.validate().unwrap_err(),
            ParamError("construction buffer exceeds the portable byte ceiling"),
        );
        params.chemo = SimParams::default().chemo;
        params.sensing.vision_range = 0.01;
        params.sensing.chemo_radius = 0.01;
        assert_eq!(
            params.validate().unwrap_err(),
            ParamError("construction buffer exceeds the portable byte ceiling"),
        );
    }
}
