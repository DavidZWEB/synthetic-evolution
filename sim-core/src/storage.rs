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

/// Historical buffer inventories, independent of any shell's wire-schema numbering.
///
/// Eras select validation accounting only. World construction always uses `CURRENT`;
/// choosing an older era does not migrate data or authorize an older runtime layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutEra {
    /// Ecological buffers before species representatives and lifetime identities.
    BeforeSpecies,
    /// Adds bounded species representatives, but not lifetime identity arrays.
    Species,
    /// Adds individual birth IDs and both persistent parent references.
    BirthIdentities,
}

impl LayoutEra {
    pub const CURRENT: Self = Self::BirthIdentities;
}

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
        Self::for_era(params, LayoutEra::CURRENT)
    }

    pub(crate) fn for_era(params: &SimParams, era: LayoutEra) -> Result<Self, ParamError> {
        let (include_species, include_birth_identity) = match era {
            LayoutEra::BeforeSpecies => (false, false),
            LayoutEra::Species => (true, false),
            LayoutEra::BirthIdentities => (true, true),
        };
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
            species_bytes: if include_species {
                Classifier::estimated_construction_bytes(
                    params.species.capacity,
                    storage.max_genes,
                )?
            } else {
                0
            },
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
        if include_birth_identity {
            requests.buffers::<BirthId>(u64::from(agents), 3)?;
        }
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
        requests.plants(grid_cells, u64::from(params.plants.max_plants))?;

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
        // Agents: position/velocity/signature, orientation, energy reserve,
        // energy/health/size/sensor load, age/species/parents/grid/brain units, handles.
        self.buffers::<Vec3>(agents, 3)?;
        self.buffer::<Quat>(agents)?;
        self.buffer::<f64>(agents)?;
        self.buffers::<f32>(agents, 4)?;
        self.buffers::<u32>(agents, 6)?;
        self.buffers::<Block>(agents, 6)?;
        // Intents: thrust, turn, ingest, reproduce.
        self.buffers::<f32>(agents, 4)?;
        // Fixed-stride parts: one f32 and one free-list index per slot (spec §9.1).
        self.buffer::<f32>(agents)?;
        self.buffer::<u32>(agents)?;
        // Deferred deaths/births. Command queues start empty and request no storage.
        self.buffers::<AgentId>(agents, 2)
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

    fn plants(&mut self, cells: u64, plants: u64) -> Result<(), ParamError> {
        self.buffer::<Vec3>(plants)?;
        self.buffer::<f32>(plants)?;
        self.buffer::<f64>(plants)?;
        self.buffer::<u8>(plants)?;
        // The constructor's per-plant grid-cell array is dropped after rebuilding.
        self.buffer::<u32>(plants)?;
        self.spatial_hash(cells, plants)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eras_own_the_complete_historical_buffer_inventory() {
        for agents in [2, 17, 5_000] {
            for capacity in [0, 1, 256] {
                let mut params = SimParams::default();
                params.world.max_agents = agents;
                params.species.capacity = capacity;
                let before = StorageLayout::for_era(&params, LayoutEra::BeforeSpecies).unwrap();
                let species = StorageLayout::for_era(&params, LayoutEra::Species).unwrap();
                let identities =
                    StorageLayout::for_era(&params, LayoutEra::BirthIdentities).unwrap();
                let classifier_bytes =
                    Classifier::estimated_construction_bytes(capacity, params.storage.max_genes)
                        .unwrap();
                assert_eq!(before.species_bytes, 0);
                assert_eq!(species.species_bytes, classifier_bytes);
                assert_eq!(
                    species.construction_bytes - before.construction_bytes,
                    classifier_bytes
                );
                assert_eq!(
                    identities.construction_bytes - species.construction_bytes,
                    3 * size_of::<BirthId>() as u64 * u64::from(agents)
                );
                assert_eq!(
                    StorageLayout::new(&params).unwrap().construction_bytes,
                    identities.construction_bytes
                );
                for layout in [&species, &identities] {
                    assert_eq!(layout.genes, before.genes);
                    assert_eq!(layout.neurons, before.neurons);
                    assert_eq!(layout.synapses, before.synapses);
                    assert_eq!(layout.sensors, before.sensors);
                    assert_eq!(layout.effectors, before.effectors);
                }
            }
        }
    }

    #[test]
    fn pre_species_inventory_does_not_depend_on_disabling_classifier_params() {
        let mut params = SimParams::default();
        let expected = StorageLayout::for_era(&params, LayoutEra::BeforeSpecies)
            .unwrap()
            .construction_bytes;
        params.species.capacity = u32::MAX;
        assert_eq!(
            StorageLayout::for_era(&params, LayoutEra::BeforeSpecies)
                .unwrap()
                .construction_bytes,
            expected
        );
        assert!(StorageLayout::for_era(&params, LayoutEra::Species).is_err());
        assert!(StorageLayout::new(&params).is_err());
    }

    #[test]
    fn historical_validation_never_selects_a_worlds_runtime_layout() {
        let mut params = SimParams::default();
        params.storage.max_memory_bytes = StorageLayout::for_era(&params, LayoutEra::BeforeSpecies)
            .unwrap()
            .construction_bytes;
        params
            .validate_for_layout(LayoutEra::BeforeSpecies)
            .unwrap();
        assert_eq!(
            params.species.capacity, 256,
            "validation must not rewrite configuration"
        );
        assert!(params.validate_for_layout(LayoutEra::Species).is_err());
        assert!(params.validate().is_err());
        assert!(crate::World::new(42, params).is_err());
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
    fn sparse_templates_charge_only_their_actual_connections() {
        let mut params = SimParams::default();
        let dense = StorageLayout::new(&params).unwrap().construction_bytes;
        let dense_connections = FounderPlan::checked_counts(&params).unwrap().synapses;
        for fan_in in [0, 1, 2, 24, u32::MAX] {
            params.brain.connections_per_target = Some(fan_in);
            let counts = FounderPlan::checked_counts(&params).unwrap();
            let sparse = StorageLayout::new(&params).unwrap().construction_bytes;
            let removed = u64::from(dense_connections - counts.synapses);
            // Template + possible sort scratch, cached scale, and the constructor's
            // gene-capacity target-index buffer. All other allocations stay fixed.
            let bytes_per_connection =
                (2 * size_of::<Gene>() + size_of::<f32>() + size_of::<u64>()) as u64;
            assert_eq!(dense - sparse, removed * bytes_per_connection);
        }
    }

    #[test]
    fn sparse_limits_do_not_require_space_for_a_dense_counterpart() {
        let mut params = SimParams::default();
        params.brain.hidden_neurons = 64;
        assert_eq!(
            params.validate().unwrap_err(),
            ParamError("founder exceeds storage.max_genes")
        );
        params.brain.connections_per_target = Some(1);
        params.storage.max_connections = 68;
        params.storage.max_genes = 170;
        params.storage.max_neurons = 86;
        params.storage.max_memory_bytes = params.estimated_construction_bytes().unwrap();
        params.validate().unwrap();
        params.storage.max_connections -= 1;
        assert_eq!(
            params.validate().unwrap_err(),
            ParamError("founder exceeds storage.max_connections")
        );
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
        type Case = (fn(&mut SimParams), &'static str);
        let cases: [Case; 6] = [
            (
                |p| p.storage.max_genes = 283,
                "founder exceeds storage.max_genes",
            ),
            (
                |p| p.storage.max_neurons = 27,
                "founder exceeds storage.max_neurons",
            ),
            (
                |p| p.storage.max_connections = 239,
                "founder exceeds storage.max_connections",
            ),
            (
                |p| p.storage.max_sensors = 4,
                "founder exceeds storage.max_sensors",
            ),
            (
                |p| p.storage.max_vision_rays = 2,
                "founder exceeds storage.max_vision_rays",
            ),
            (
                |p| p.storage.max_effectors = 3,
                "founder exceeds storage.max_effectors",
            ),
        ];
        for (change, message) in cases {
            let mut params = SimParams::default();
            change(&mut params);
            assert_eq!(params.validate().unwrap_err(), ParamError(message));
        }
    }

    #[test]
    fn every_arena_must_hold_at_least_one_founder() {
        type Case = (fn(&mut SimParams), &'static str);
        let cases: [Case; 5] = [
            (
                |p| p.storage.genes_per_slot -= 1,
                "storage.genes_per_slot times world.max_agents cannot hold one founder",
            ),
            (
                |p| p.storage.neurons_per_slot -= 1,
                "storage.neurons_per_slot times world.max_agents cannot hold one founder",
            ),
            (
                |p| p.storage.synapses_per_slot -= 1,
                "storage.synapses_per_slot times world.max_agents cannot hold one founder",
            ),
            (
                |p| p.storage.sensors_per_slot -= 1,
                "storage.sensors_per_slot times world.max_agents cannot hold one founder",
            ),
            (
                |p| p.storage.effectors_per_slot -= 1,
                "storage.effectors_per_slot times world.max_agents cannot hold one founder",
            ),
        ];
        for (change, message) in cases {
            let mut params = SimParams::default();
            params.world.max_agents = 1;
            change(&mut params);
            assert_eq!(params.validate().unwrap_err(), ParamError(message));
            params.world.max_agents = 2;
            params
                .validate()
                .expect("allowances are pooled, not per-genome maxima");
        }
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
