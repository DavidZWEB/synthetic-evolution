//! `World`: everything one simulation owns, and the entry points for changing it.
//!
//! A process must be able to hold several worlds at once — the native shell runs
//! parameter sweeps that way, and the random-brain control population is a second
//! world beside the first. That is why nothing here is `static`, including the
//! innovation counter, which is a plain field (spec §7.2, §3.1).
//!
//! Deliberately not here: tick orchestration, command dispatch, or agent lifecycle
//! transitions. `tick` owns spec §2.4's eleven steps; `command` owns the boundary
//! queue; `lifecycle` owns agent admission and removal. This file owns world state,
//! construction, and its accessors.

use glam::Vec3;

use crate::agents::Agents;
use crate::arena::{Arena, ArenaBuildError, VariableArena};
use crate::brain::{Neuron, Synapse};
use crate::chemo::ChemoField;
use crate::command::Command;
use crate::control::BrainInheritance;
use crate::effectors::{Effector, Intents};
use crate::founder::FounderPlan;
use crate::genome::Gene;
use crate::ids::{AgentId, InnovationExhausted, InnovationId, reserve_innovations};
use crate::ledger::EnergyLedger;
use crate::params::{ParamError, SimParams};
use crate::perceive::Sensor;
use crate::plants::Plants;
use crate::pool::SlotPool;
use crate::rng::Rng;
use crate::spatial::SpatialHash;
use crate::spawn::{ArenaKind, ArenaUsage};
use crate::species::{self, Classifier};
use crate::storage::StorageLayout;

mod lifecycle;

/// One part per agent, at the agent's own origin, for all of V1 (spec §9.1).
const PARTS_PER_AGENT: u32 = 1;

/// Invalid configuration or a host reservation failure while constructing a world.
#[derive(Debug)]
pub enum WorldBuildError {
    Params(ParamError),
    Arena(ArenaBuildError),
    Species(species::BuildError),
}

impl core::fmt::Display for WorldBuildError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Params(error) => error.fmt(f),
            Self::Arena(error) => error.fmt(f),
            Self::Species(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for WorldBuildError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Params(error) => Some(error),
            Self::Arena(error) => Some(error),
            Self::Species(error) => Some(error),
        }
    }
}

impl From<ParamError> for WorldBuildError {
    fn from(error: ParamError) -> Self {
        Self::Params(error)
    }
}

impl From<ArenaBuildError> for WorldBuildError {
    fn from(error: ArenaBuildError) -> Self {
        Self::Arena(error)
    }
}

impl From<species::BuildError> for WorldBuildError {
    fn from(error: species::BuildError) -> Self {
        Self::Species(error)
    }
}

/// A single simulation: its state, its parameters, and its random stream.
///
/// Fields are `pub(crate)` rather than private because `tick` splits the eleven steps
/// into their own module and Rust needs at least crate visibility to write an `impl`
/// across two files. Nothing outside `sim-core` gains anything: the shells and the
/// integration tests still go through the accessors at the bottom of this file.
pub struct World {
    pub(crate) params: SimParams,
    pub(crate) brain_inheritance: BrainInheritance,
    pub(crate) rng: Rng,
    pub(crate) tick: u64,
    /// Monotonic source of [`InnovationId`]s. A field rather than a `static` so two
    /// worlds in one process cannot hand out ids from the same counter (spec §3.1).
    pub(crate) next_innovation: u32,
    pub(crate) pool: SlotPool,
    pub(crate) agents: Agents,
    pub(crate) classifier: Classifier,
    pub(crate) unclassified: u32,
    /// Compiled neurons, one block per agent.
    pub(crate) brains: VariableArena<Neuron>,
    /// Compiled wiring, one block per agent. Separate from `brains` because a synapse
    /// and a neuron are different element types, not because they have different
    /// lifetimes — the two blocks are claimed and freed together.
    pub(crate) synapses: VariableArena<Synapse>,
    /// Compiled sensors, one block per agent: an organ's parameters and the brain slots
    /// it writes to, resolved at birth (spec §2.2c).
    pub(crate) sensors: VariableArena<Sensor>,
    /// Compiled effectors, one block per agent: the brain slot that drives each one.
    pub(crate) effectors: VariableArena<Effector>,
    /// What every agent's effectors asked for this tick. Written at step 4, drained by
    /// the systems that follow it (spec §2.4).
    pub(crate) intents: Intents,
    /// Gene lists, one block per agent.
    pub(crate) genes: VariableArena<Gene>,
    /// The founding topology, whose innovation ids every founder in this world shares
    /// (spec §3.1).
    pub(crate) plan: FounderPlan,
    /// Reusable buffer for building a genome before it is copied into the arena.
    /// Owned by the world and sized once, so a birth allocates nothing.
    pub(crate) genome_scratch: Vec<Gene>,
    pub(crate) brain_fan_in_scratch: Vec<u32>,
    /// Part offsets relative to the agent origin. One zeroed entry per agent in V1.
    pub(crate) parts: Arena<f32>,
    /// Neighbour lookup, rebuilt at the top of every tick (spec §2.4 step 1).
    pub(crate) hash: SpatialHash,
    /// Pheromone concentrations: what plants scent and what the chemo sensor reads.
    pub(crate) field: ChemoField,
    /// The autotrophs. Every joule in the world enters through them (spec §5.1).
    pub(crate) plants: Plants,
    /// Every joule that entered and left. The acceptance criterion for M7 is that this
    /// agrees with the stock actually present (spec §5.1).
    pub(crate) ledger: EnergyLedger,
    /// Slots that reached zero energy this tick, resolved at step 10 in agent-index
    /// order. Owned by the world and sized once, so a death allocates nothing.
    pub(crate) dying: Vec<AgentId>,
    /// Agents that asked to reproduce and can, resolved at step 10 after the deaths.
    /// Owned and sized for the same reason.
    pub(crate) breeding: Vec<AgentId>,
    /// Requests from outside the simulation, waiting for the tick they are stamped for
    /// (spec §2.2b). Kept in submission order, which is what makes two runs fed the same
    /// commands apply them the same way.
    pub(crate) commands: Vec<Command>,
    /// The ones due this tick, lifted out before any is applied so that a command
    /// cannot enqueue another and have it run inside the same tick.
    pub(crate) due_commands: Vec<Command>,
}

impl World {
    /// Builds an empty world.
    ///
    /// Params are validated here, at the boundary. Past this point the tick treats
    /// their invariants as established and uses `debug_assert!` rather than threading
    /// `Result` through the hot loop.
    pub fn new(seed: u64, params: SimParams) -> Result<Self, WorldBuildError> {
        Self::new_with_brain_inheritance(seed, params, BrainInheritance::Evolving)
    }

    /// Builds a world with an explicit neural-heredity mode for controlled experiments.
    pub fn new_with_brain_inheritance(
        seed: u64,
        params: SimParams,
        brain_inheritance: BrainInheritance,
    ) -> Result<Self, WorldBuildError> {
        params.validate()?;
        let layout = StorageLayout::new(&params)?;
        let capacity = params.world.max_agents;
        let mut next_innovation = 0u32;
        // Plant geography precedes optional sparse-template draws, so changing
        // founder connectivity does not move the food sites (spec section 3.3).
        let mut rng = Rng::from_seed(seed);
        let plants = Plants::new(&params, &mut rng);
        // The plan draws the world's first innovation ids, before any agent exists.
        let plan = FounderPlan::new(&params, &mut rng, || {
            let id = InnovationId::new(next_innovation);
            next_innovation += 1;
            id
        })?;
        let mut genome_scratch = Vec::with_capacity(params.storage.max_genes as usize);
        genome_scratch.resize(plan.len(), Gene::default());

        // One generator, drawn from in order: the plants are seeded first and agents
        // continue after them. A second `Rng::from_seed(seed)` would be the *same*
        // stream, making every plant coordinate bit-identical to the genome scalar
        // drawn at the same position — two processes that look independent and are not.
        Ok(Self {
            brain_inheritance,
            rng,
            tick: 0,
            next_innovation,
            pool: SlotPool::with_capacity(capacity),
            agents: Agents::with_capacity(capacity),
            classifier: Classifier::try_new(
                params.species.capacity,
                params.storage.max_genes,
                params.species.threshold,
                params.distance.clone(),
                layout.species_bytes,
            )?,
            unclassified: 0,
            brains: VariableArena::try_with_capacity(layout.neurons, capacity)?,
            synapses: VariableArena::try_with_capacity(layout.synapses, capacity)?,
            sensors: VariableArena::try_with_capacity(layout.sensors, capacity)?,
            effectors: VariableArena::try_with_capacity(layout.effectors, capacity)?,
            intents: Intents::with_capacity(capacity),
            genes: VariableArena::try_with_capacity(layout.genes, capacity)?,
            genome_scratch,
            brain_fan_in_scratch: vec![0; params.storage.max_neurons as usize],
            plan,
            parts: Arena::with_capacity(capacity, PARTS_PER_AGENT),
            hash: SpatialHash::new(
                params.world.size,
                params.sensing.max_sense_radius(),
                capacity,
            ),
            field: ChemoField::new(&params.chemo, params.world.size),
            // Against the stock, not zero: energy present before the first tick is a
            // boundary condition, the same treatment `spawn_founder` gives a founder's
            // tank (spec §5.1).
            ledger: EnergyLedger::opening(plants.total_energy()),
            dying: Vec::with_capacity(capacity as usize),
            breeding: Vec::with_capacity(capacity as usize),
            // Not sized at capacity: commands arrive at human speed, a handful per
            // second at most, and reserving a slot per agent for them would cost more
            // memory than the queue will ever hold.
            commands: Vec::new(),
            due_commands: Vec::new(),
            plants,
            params,
        })
    }

    pub fn species(&self) -> &Classifier {
        &self.classifier
    }

    pub fn species_count(&self) -> u32 {
        self.classifier.active().count() as u32
    }

    pub fn unclassified_population(&self) -> u32 {
        self.unclassified
    }

    /// One agent's genes.
    #[inline]
    pub fn genome(&self, id: AgentId) -> &[Gene] {
        self.genes.get(self.agents.genome[id.index()])
    }

    /// What every agent's effectors asked for on the most recent step 4.
    /// Mutable agent state, for shells and tests that set up a specific situation.
    /// The tick itself goes through the systems, not through here.
    #[inline]
    pub fn agents_mut(&mut self) -> &mut Agents {
        &mut self.agents
    }

    /// Mutable intent buffer, for the same reason.
    #[inline]
    pub fn intents_mut(&mut self) -> &mut Intents {
        &mut self.intents
    }

    #[inline]
    pub fn intents(&self) -> &Intents {
        &self.intents
    }

    #[inline]
    pub fn spatial_hash(&self) -> &SpatialHash {
        &self.hash
    }

    #[inline]
    pub fn chemo(&self) -> &ChemoField {
        &self.field
    }

    #[inline]
    pub fn plants(&self) -> &Plants {
        &self.plants
    }

    /// One agent's part offsets, as flat `xyz` triples relative to its own origin.
    ///
    /// Exactly one part, at the origin, for all of V1. The indirection is cashed in at
    /// Phase 5, when a body has parts worth placing (spec §3.5, §9.1).
    pub fn parts_of(&self, id: AgentId) -> &[f32] {
        self.parts.get(self.agents.parts[id.index()])
    }

    pub fn storage_usage(&self) -> [ArenaUsage; 5] {
        fn usage<T: Copy + Default>(arena: &VariableArena<T>, kind: ArenaKind) -> ArenaUsage {
            ArenaUsage {
                arena: kind,
                capacity: arena.capacity(),
                free_elements: arena.free_elements(),
                largest_free_block: arena.largest_free_block(),
                live_blocks: arena.live_blocks(),
            }
        }
        [
            usage(&self.genes, ArenaKind::Genes),
            usage(&self.brains, ArenaKind::Neurons),
            usage(&self.synapses, ArenaKind::Synapses),
            usage(&self.sensors, ArenaKind::Sensors),
            usage(&self.effectors, ArenaKind::Effectors),
        ]
    }

    /// Replaces the tunables.
    ///
    /// The policy — which fields may move on a running world and which are frozen by
    /// what they sized — is `SimParams::check_retune`, where it can be read and tested
    /// without a world to hand.
    pub fn set_params(&mut self, params: SimParams) -> Result<(), ParamError> {
        self.params.check_retune(&params, self.hash.cell_size())?;
        self.params = params;
        Ok(())
    }

    /// Every joule the world currently holds, in plants, agents, and the transfer
    /// rounding reserve.
    ///
    /// This fixed-order `f64` aggregation is shared by ledger opening, drift checks,
    /// and shell telemetry. Using a separate `f32` sum for any one of them manufactures
    /// apparent energy drift from rounding alone (spec §5.1).
    pub fn total_energy(&self) -> f64 {
        let agents: f64 = self
            .pool
            .iter_live()
            .map(|id| {
                let i = id.index();
                self.agents.energy[i] as f64 + self.agents.energy_reserve[i]
            })
            .sum();
        self.plants.total_energy() + agents
    }

    /// Energy below the visible `f32` resolution, still owned by plants or agents.
    pub fn energy_reserve(&self) -> f64 {
        let agents: f64 = self
            .pool
            .iter_live()
            .map(|id| self.agents.energy_reserve[id.index()])
            .sum();
        agents + self.plants.energy_reserve().iter().sum::<f64>()
    }

    /// Mean energy held by living agents, for on-demand instrumentation.
    ///
    /// Agent-index order keeps the aggregate reproducible, and doing the scan only when
    /// a shell asks keeps telemetry out of the simulation hot loop (spec §7.8).
    pub fn mean_agent_energy(&self) -> f64 {
        let population = self.population();
        if population == 0 {
            return 0.0;
        }
        let total: f64 = self
            .pool
            .iter_live()
            .map(|id| {
                let i = id.index();
                self.agents.energy[i] as f64 + self.agents.energy_reserve[i]
            })
            .sum();
        total / population as f64
    }

    /// Number of living agents born inside this world rather than seeded as founders.
    ///
    /// Parentage, not age, is the distinction: an old offspring is still a descendant,
    /// while a founder that survives indefinitely is not evidence of inherited
    /// adaptation (spec §7.8).
    pub fn living_descendants(&self) -> u32 {
        self.pool
            .iter_live()
            .filter(|id| self.agents.parent_a[id.index()] != AgentId::NULL.raw())
            .count() as u32
    }

    #[inline]
    pub fn ledger(&self) -> &EnergyLedger {
        &self.ledger
    }

    /// How far the world's energy has drifted from what the ledger accounts for. Zero
    /// is the invariant; the sign says which mistake to look for (spec §5.1).
    pub fn energy_drift(&self) -> f64 {
        self.ledger.drift(self.total_energy())
    }

    /// Adds to a chemo channel at a world position.
    ///
    /// Narrow rather than a `&mut ChemoField`: from M7 the field is written by plants
    /// and by the `emit_chemo` effector, both inside the tick and both through the
    /// intent buffer. Handing out the whole field would make "who wrote this trail" a
    /// question with no answer.
    pub fn deposit_chemo(&mut self, channel: usize, position: Vec3, amount: f32) {
        self.field.deposit(channel, position, amount);
    }

    /// Draws the next innovation id and advances the counter.
    pub fn next_innovation(&mut self) -> Result<InnovationId, InnovationExhausted> {
        reserve_innovations(&mut self.next_innovation, 1)
    }

    /// The world's founding topology.
    #[inline]
    pub fn founder_plan(&self) -> &FounderPlan {
        &self.plan
    }

    #[inline]
    pub fn params(&self) -> &SimParams {
        &self.params
    }

    #[inline]
    pub fn tick_count(&self) -> u64 {
        self.tick
    }

    #[inline]
    pub fn population(&self) -> u32 {
        self.pool.live_count()
    }

    #[inline]
    pub fn agents(&self) -> &Agents {
        &self.agents
    }

    #[inline]
    pub fn pool(&self) -> &SlotPool {
        &self.pool
    }

    #[inline]
    pub fn rng_mut(&mut self) -> &mut Rng {
        &mut self.rng
    }

    /// One agent's neurons, in slot order. What the M10 inspector reads.
    #[inline]
    pub fn brain(&self, id: AgentId) -> &[Neuron] {
        self.brains.get(self.agents.brain[id.index()])
    }

    /// One agent's wiring, endpoints already resolved to slots in [`Self::brain`].
    #[inline]
    pub fn wiring(&self, id: AgentId) -> &[Synapse] {
        self.synapses.get(self.agents.synapses[id.index()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::SpawnSpec;
    use crate::arena::AllocationFailure;
    use crate::genome::{self, BodyTrait};
    use crate::spawn::SpawnError;
    use glam::Vec3;

    #[test]
    fn classification_and_observation_leave_ecology_unchanged_across_seeds_and_modes() {
        use crate::species::SpeciesEventCounts;
        for seed in [7, 42, 99] {
            for mode in [
                BrainInheritance::Evolving,
                BrainInheritance::RandomizedAtBirth,
            ] {
                let mut params = SimParams::default();
                params.world.max_agents = 32;
                params.world.size = 100.0;
                params.sensing.vision_range = 20.0;
                params.sensing.chemo_radius = 20.0;
                params.plants.max_plants = 64;
                params.plants.max_energy = 100.0;
                params.feeding.rate = 100.0;
                params.feeding.reach = 20.0;
                params.feeding.gate = 0.0;
                params.reproduction.start_energy = 1.0;
                params.reproduction.threshold = 1.1;
                params.reproduction.maturity_ticks = 0;
                params.reproduction.gate = 0.0;
                params.mutation.organs.add_sensor_rate = 0.5;
                params.mutation.organs.remove_sensor_rate = 0.5;
                params.mutation.structural.add_connection_rate = 0.5;
                params.species.capacity = 1;
                let mut classified =
                    World::new_with_brain_inheritance(seed, params.clone(), mode).unwrap();
                params.species.capacity = 0;
                let mut unclassified =
                    World::new_with_brain_inheritance(seed, params, mode).unwrap();
                let mut events = SpeciesEventCounts::default();
                classified.seed_founders_with_observers(16, |_| {}, |event| events.record(event));
                unclassified.seed_founders(16);
                assert_ne!(classified.state_hash(), unclassified.state_hash());
                assert_eq!(classified.ecology_hash(), unclassified.ecology_hash());
                for _ in 0..120 {
                    classified.step_with_all_observers(
                        |_| {},
                        |_| {},
                        |event| events.record(event),
                    );
                    unclassified.step();
                    assert_eq!(
                        classified.ecology_hash(),
                        unclassified.ecology_hash(),
                        "seed {seed}, mode {mode:?}"
                    );
                }
                assert!(
                    classified.living_descendants() > 0,
                    "scenario must exercise births"
                );
                assert!(events.created > 0);
                assert!(unclassified.unclassified_population() > 0);
            }
        }
    }

    fn small_world() -> World {
        let mut params = SimParams::default();
        params.world.max_agents = 32;
        World::new(7, params).expect("defaults are valid")
    }

    #[test]
    fn retuning_a_world_delegates_to_the_policy_with_its_real_grid() {
        // What is worth testing here is the wiring, not the rules: `check_retune` owns
        // which fields may move and proves it against two structs. What only a world can
        // show is that `set_params` passes it the grid cell this world was actually
        // built with, so the sensing ceiling is the real one.
        let mut world = small_world();
        let cell = world.spatial_hash().cell_size();

        let mut tuned = world.params().clone();
        tuned.metabolism.base *= 2.0;
        assert!(world.set_params(tuned.clone()).is_ok());
        assert_eq!(world.params().metabolism.base, tuned.metabolism.base);

        let mut at_the_ceiling = world.params().clone();
        at_the_ceiling.sensing.vision_range = cell;
        assert!(
            world.set_params(at_the_ceiling).is_ok(),
            "a full cell should fit"
        );

        let mut past_it = world.params().clone();
        past_it.sensing.vision_range = cell * 1.01;
        assert!(world.set_params(past_it).is_err(), "grew past its own grid");

        // And nothing moved underneath it.
        assert_eq!(world.pool().capacity(), small_world().pool().capacity());
    }

    #[test]
    fn stocking_the_larder_does_not_move_the_rng_stream() {
        // Plant *positions* are drawn before the fill, so comparing those proves
        // nothing — the obvious assertion here passes under the bug. What has to match
        // is everything drawn after, since one generator serves the whole world.
        let genome_at = |fill: f32| {
            let mut params = SimParams::default();
            params.world.max_agents = 4;
            params.plants.max_plants = 32;
            params.plants.initial_fill = fill;
            let mut world = World::new(11, params).expect("valid params");
            let id = world
                .spawn_founder(Vec3::new(10.0, 10.0, 0.0))
                .expect("pool has room");
            world.genome(id).to_vec()
        };
        let full = genome_at(1.0);
        assert_eq!(full, genome_at(0.0));
        assert_eq!(full, genome_at(0.37));
    }

    #[test]
    fn rejects_invalid_params_at_construction() {
        let mut params = SimParams::default();
        params.world.dt = 0.0;
        assert!(World::new(1, params).is_err());
    }

    #[test]
    fn seeding_stops_when_the_fixed_pool_is_full() {
        let mut world = small_world();
        assert_eq!(world.seed_founders(u32::MAX), 32);
        assert_eq!(world.population(), 32);
        assert_eq!(world.seed_founders(u32::MAX), 0);
    }

    #[test]
    fn seeding_distributes_founders_uniformly_by_area() {
        let mut params = SimParams::default();
        params.world.max_agents = 400;
        let centre = Vec3::new(params.world.size * 0.5, params.world.size * 0.5, 0.0);
        let radius = params.world.size * params.world.founder_spread;
        let mut world = World::new(7, params).expect("defaults are valid");
        assert_eq!(world.seed_founders(400), 400);

        let inner = world
            .pool()
            .iter_live()
            .filter(|id| {
                let delta = world.agents().position[id.index()] - centre;
                delta.length_squared() < (radius * 0.5) * (radius * 0.5)
            })
            .count();

        // A half-radius disc contains one quarter of the total area. Linear radial
        // spacing would put 200 founders here instead and recreate the central pile-up.
        assert_eq!(inner, 100);
    }

    #[test]
    fn mean_agent_energy_uses_only_live_slots() {
        let mut world = small_world();
        assert_eq!(world.mean_agent_energy(), 0.0);
        let a = world.spawn_founder(Vec3::ZERO).expect("pool has room");
        let b = world
            .spawn_founder(Vec3::new(1.0, 0.0, 0.0))
            .expect("pool has room");
        world.agents_mut().energy[a.index()] = 10.0;
        world.agents_mut().energy[b.index()] = 30.0;
        assert_eq!(world.mean_agent_energy(), 20.0);
        world.despawn(a);
        assert_eq!(world.mean_agent_energy(), 30.0);
    }

    #[test]
    fn living_descendants_uses_parentage_not_age() {
        let mut world = small_world();
        let founder = world
            .spawn_founder(Vec3::new(1.0, 2.0, 0.0))
            .expect("pool has room");
        let descendant = world
            .spawn(
                &SpawnSpec {
                    position: Vec3::new(3.0, 4.0, 0.0),
                    energy: 100.0,
                    size: 1.0,
                    signature: Vec3::ONE,
                    yaw: 0.0,
                    parent_a: founder,
                },
                &[],
            )
            .expect("pool has room");
        world.agents.age[founder.index()] = 10_000;
        world.agents.age[descendant.index()] = 0;

        assert_eq!(world.living_descendants(), 1);
        world.despawn(descendant);
        assert_eq!(world.living_descendants(), 0);
    }

    #[test]
    fn removing_a_living_agent_dissipates_what_it_held() {
        // `despawn` is public and starvation is not its only caller: Phase 3 corpses
        // despawn agents that still hold energy, and a cull would too. Deleting those
        // joules instead of dissipating them breaks §5.1 in the one way no functional
        // test notices.
        let mut w = small_world();
        let id = w.spawn_founder(Vec3::new(500.0, 500.0, 0.0)).unwrap();
        let held = w.agents().energy[id.index()];
        assert!(held > 0.0);
        assert_eq!(w.energy_drift(), 0.0);

        w.despawn(id);
        assert!(
            w.energy_drift().abs() < 1e-3,
            "despawn destroyed {held} joules without dissipating them"
        );
        assert!((w.ledger().dissipated() - held as f64).abs() < 1e-3);
    }

    #[test]
    fn spawn_and_despawn_track_population() {
        let mut w = small_world();
        assert_eq!(w.population(), 0);
        let a = w.spawn_founder(Vec3::new(1.0, 2.0, 0.0)).unwrap();
        let b = w.spawn_founder(Vec3::new(3.0, 4.0, 0.0)).unwrap();
        assert_eq!(w.population(), 2);
        assert!(w.despawn(a));
        assert!(!w.despawn(a), "despawning twice must be a no-op");
        assert_eq!(w.population(), 1);
        assert!(w.pool().is_alive(b));
    }

    #[test]
    fn every_live_agent_has_a_distinct_brain_block() {
        let mut w = small_world();
        let ids: Vec<_> = (0..32)
            .map(|i| w.spawn_founder(Vec3::new(i as f32, 0.0, 0.0)).unwrap())
            .collect();
        let mut offsets: Vec<u32> = ids
            .iter()
            .map(|&id| w.agents().brain[id.index()].offset())
            .collect();
        offsets.sort();
        offsets.dedup();
        assert_eq!(offsets.len(), 32, "two agents are sharing a brain");
        assert!(
            w.spawn_founder(Vec3::new(0.0, 0.0, 0.0)).is_err(),
            "full pool must refuse"
        );
    }

    #[test]
    fn arena_blocks_come_back_on_death() {
        let mut w = small_world();
        let ids: Vec<_> = (0..32)
            .map(|i| w.spawn_founder(Vec3::new(i as f32, 0.0, 0.0)).unwrap())
            .collect();
        assert!(w.spawn_founder(Vec3::new(0.0, 0.0, 0.0)).is_err());
        for id in &ids {
            w.despawn(*id);
        }
        assert_eq!(w.population(), 0);
        // If blocks leaked, the pool would have slots but the arena would not.
        for i in 0..32 {
            assert!(
                w.spawn_founder(Vec3::new(i as f32, 0.0, 0.0)).is_ok(),
                "arena leaked a block"
            );
        }
    }

    #[test]
    fn a_recycled_slot_starts_with_a_blank_brain() {
        let mut w = small_world();
        let a = w.spawn_founder(Vec3::new(0.0, 0.0, 0.0)).unwrap();
        for _ in 0..50 {
            w.step_brains();
        }
        assert!(
            w.brain(a).iter().any(|n| n.output != 0.0),
            "the brain never ran, so this proves nothing"
        );
        w.despawn(a);

        let b = w.spawn_founder(Vec3::new(0.0, 0.0, 0.0)).unwrap();
        assert!(
            w.brain(b).iter().all(|n| n.state == 0.0 && n.output == 0.0),
            "inherited the dead agent's activations"
        );
    }

    #[test]
    fn a_degenerate_genome_does_not_exhaust_the_world() {
        // An empty gene list passes `genome::validate` — it is vacuously coherent — so
        // it reaches the arenas as a zero-length block for the brain, the wiring, and
        // the genome. Freeing one used to be a no-op that never returned its slot, and
        // eight cycles were enough to leave a world reporting a population of zero and
        // refusing every birth.
        let mut w = small_world();
        let spec = SpawnSpec {
            position: Vec3::ZERO,
            yaw: 0.0,
            energy: 1.0,
            size: 1.0,
            signature: Vec3::ZERO,
            parent_a: AgentId::NULL,
        };
        for _ in 0..64 {
            let id = w.spawn(&spec, &[]).expect("a free slot");
            assert!(w.brain(id).is_empty() && w.wiring(id).is_empty());
            w.despawn(id);
        }
        assert_eq!(w.population(), 0);
        for i in 0..32 {
            assert!(
                w.spawn_founder(Vec3::new(i as f32, 0.0, 0.0)).is_ok(),
                "the world leaked its arena blocks"
            );
        }
    }

    #[test]
    fn a_newborn_does_not_inherit_a_dead_agents_intents() {
        // Every other per-agent field is reset by `Agents::init`; the intent buffer is
        // the one that lives elsewhere and used to escape it.
        let mut w = small_world();
        let a = w.spawn_founder(Vec3::new(500.0, 500.0, 0.0)).unwrap();
        w.intents_mut().thrust[a.index()] = 42.0;
        w.intents_mut().ingest[a.index()] = 1.0;
        w.despawn(a);

        let b = w.spawn_founder(Vec3::new(500.0, 500.0, 0.0)).unwrap();
        assert_eq!(b.index(), a.index(), "expected the slot to be recycled");
        assert_eq!(
            w.intents().thrust[b.index()],
            0.0,
            "inherited a thrust request"
        );
        assert_eq!(
            w.intents().ingest[b.index()],
            0.0,
            "inherited an ingest request"
        );
    }

    #[test]
    fn spawning_compiles_a_runnable_brain() {
        let mut w = small_world();
        let id = w.spawn_founder(Vec3::ZERO).unwrap();
        assert_eq!(w.brain(id).len(), w.founder_plan().neuron_count());
        assert_eq!(w.wiring(id).len(), 240, "the founder is fully connected");
        for synapse in w.wiring(id) {
            assert!(
                synapse.from.index() < w.brain(id).len() && synapse.to.index() < w.brain(id).len(),
                "endpoint outside this agent's own brain: {synapse:?}"
            );
        }
    }

    #[test]
    fn wiring_blocks_come_back_on_death() {
        // The brain is two blocks now; leaking the second is invisible until the world
        // stops accepting births hours into a run.
        let mut w = small_world();
        let ids: Vec<_> = (0..32)
            .map(|i| w.spawn_founder(Vec3::new(i as f32, 0.0, 0.0)).unwrap())
            .collect();
        assert_eq!(w.synapses.live_blocks(), 32);
        for id in ids {
            w.despawn(id);
        }
        assert_eq!(w.synapses.live_blocks(), 0, "a wiring block leaked");
    }

    #[test]
    fn spawning_compiles_the_agents_organs() {
        let mut w = small_world();
        let id = w.spawn_founder(Vec3::ZERO).unwrap();
        let sensors = w.sensors.get(w.agents().sensors[id.index()]);
        assert_eq!(
            sensors.len(),
            w.params().sensing.vision_rays as usize + 2,
            "the founding set is n eyes, a nose, and an interoceptor"
        );
        for sensor in sensors {
            for target in &sensor.targets[..sensor.modality.channels()] {
                assert!(
                    target.index() < w.brain(id).len(),
                    "a sensor points outside its own brain: {sensor:?}"
                );
            }
        }
    }

    #[test]
    fn sensor_blocks_come_back_on_death() {
        let mut w = small_world();
        let ids: Vec<_> = (0..32)
            .map(|i| w.spawn_founder(Vec3::new(i as f32, 0.0, 0.0)).unwrap())
            .collect();
        assert_eq!(w.sensors.live_blocks(), 32);
        for id in ids {
            w.despawn(id);
        }
        assert_eq!(w.sensors.live_blocks(), 0, "a sensor block leaked");
    }

    #[test]
    fn perception_reaches_the_brain() {
        // The end-to-end claim of M6's first half: something in the world shows up as
        // input on a neuron, without the agent ever touching a world array.
        let mut w = small_world();
        let id = w.spawn_founder(Vec3::new(500.0, 500.0, 0.0)).unwrap();
        w.rebuild_spatial_hash();

        w.perceive_all();
        let quiet: f32 = w.brain(id).iter().map(|n| n.input.abs()).sum();

        // An interoceptor always reports energy, so a "silent" world is not silent —
        // but adding food to smell has to move the total.
        w.step_brains();
        w.deposit_chemo(0, Vec3::new(500.0, 500.0, 0.0), 250.0);
        w.perceive_all();
        let smelling: f32 = w.brain(id).iter().map(|n| n.input.abs()).sum();
        assert!(
            smelling > quiet,
            "depositing food changed nothing: {quiet} -> {smelling}"
        );
    }

    #[test]
    fn perception_is_consumed_by_the_brain_not_accumulated() {
        // `Neuron::input` persists between step 2 and step 3 by design. If step 3 did
        // not clear it, every tick would add to the last and the brain would saturate
        // within seconds while looking like a runaway weight problem.
        let mut w = small_world();
        let id = w.spawn_founder(Vec3::new(500.0, 500.0, 0.0)).unwrap();
        w.deposit_chemo(0, Vec3::new(500.0, 500.0, 0.0), 250.0);
        w.rebuild_spatial_hash();

        let mut previous = 0.0f32;
        for tick in 0..50 {
            w.perceive_all();
            let charged: f32 = w.brain(id).iter().map(|n| n.input.abs()).sum();
            w.step_brains();
            assert!(
                w.brain(id).iter().all(|n| n.input == 0.0),
                "step {tick} left input on a neuron"
            );
            if tick > 0 {
                assert!(
                    (charged - previous).abs() < previous.max(1.0) * 0.5,
                    "input is growing tick over tick: {previous} -> {charged}"
                );
            }
            previous = charged;
        }
    }

    #[test]
    fn brains_advance_and_stay_finite() {
        // With no perception yet a brain runs on its recurrence and its oscillators
        // alone, which is enough to tell a live network from a dead one.
        let mut w = small_world();
        let id = w.spawn_founder(Vec3::ZERO).unwrap();
        let before: Vec<f32> = w.brain(id).iter().map(|n| n.output).collect();
        for _ in 0..1_000 {
            w.step_brains();
        }
        let after: Vec<f32> = w.brain(id).iter().map(|n| n.output).collect();
        assert_ne!(before, after, "the brain did not move");
        assert!(after.iter().all(|x| x.is_finite()), "{after:?}");
    }

    #[test]
    fn stepping_brains_is_deterministic() {
        let run = || {
            let mut w = small_world();
            for i in 0..8 {
                w.spawn_founder(Vec3::new(i as f32, 0.0, 0.0)).unwrap();
            }
            for _ in 0..200 {
                w.step_brains();
            }
            (0..8)
                .map(|i| w.brain(AgentId::new(i)).to_vec())
                .collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn a_brain_runs_the_same_alone_as_in_a_crowd() {
        // Every agent's input, state, and wiring must stay its own. A handle that
        // aliased, or per-tick input parked somewhere shared, would make behaviour an
        // artifact of pool order (spec §2.4) — which no determinism test would catch,
        // because it would be reproducibly wrong.
        let trajectory = |population: u32| {
            let mut w = small_world();
            for i in 0..population {
                w.spawn_founder(Vec3::new(i as f32, 0.0, 0.0)).unwrap();
            }
            for _ in 0..300 {
                w.step_brains();
            }
            w.brain(AgentId::new(0)).to_vec()
        };
        // The first founder draws from the world RNG first either way, so it is the
        // same agent in both worlds.
        assert_eq!(trajectory(1), trajectory(16));
    }

    #[test]
    fn a_dead_agent_does_not_get_stepped() {
        // `step_brains` walks live slots; a stale block would otherwise keep ticking
        // and show up in whatever reads the arena next.
        let mut w = small_world();
        let a = w.spawn_founder(Vec3::ZERO).unwrap();
        let b = w.spawn_founder(Vec3::new(5.0, 0.0, 0.0)).unwrap();
        let block = w.agents().brain[a.index()];
        w.despawn(a);
        let frozen = w.brains.get(block).to_vec();
        for _ in 0..20 {
            w.step_brains();
        }
        assert_eq!(w.brains.get(block), frozen.as_slice());
        assert!(w.brain(b).iter().any(|n| n.output != 0.0));
    }

    #[test]
    fn innovation_ids_are_per_world_and_monotonic() {
        // Two worlds in one process must not share a counter (spec §7.2). Both start
        // wherever their own founding topology left off, not at zero — the plan draws
        // the world's first ids during construction.
        let mut a = small_world();
        let mut b = small_world();
        let first = a.next_innovation().unwrap();
        assert!(
            first.raw() > 0,
            "the founding plan should already have drawn ids"
        );
        assert_eq!(a.next_innovation().unwrap().raw(), first.raw() + 1);
        assert_eq!(
            b.next_innovation().unwrap(),
            first,
            "counter leaked between worlds"
        );
    }

    #[test]
    fn arena_capacity_uses_pooled_allowances_not_founder_stride() {
        let w = small_world();
        assert_eq!(
            w.brains.capacity(),
            w.params.world.max_agents * w.params.storage.neurons_per_slot
        );
        assert_eq!(
            w.genes.capacity(),
            w.params.world.max_agents * w.params.storage.genes_per_slot
        );
    }

    #[test]
    fn a_failed_fixed_part_claim_unwinds_all_variable_claims() {
        let mut world = small_world();
        let mut genes = vec![Gene::default(); world.plan.len()];
        world
            .plan
            .instantiate(&mut Rng::from_seed(9), &world.params, &mut genes);
        // Inject failure in the last constituent: normally the part pool and agent
        // pool share a ceiling, so this branch cannot be reached by valid seeding.
        world.parts = Arena::with_capacity(0, 1);
        let before = world.storage_usage();
        let incarnations = world.pool.incarnations().to_vec();
        let spec = SpawnSpec {
            position: Vec3::ZERO,
            yaw: 0.0,
            energy: 0.0,
            size: 1.0,
            signature: Vec3::ONE,
            parent_a: AgentId::NULL,
        };
        assert_eq!(
            world.spawn(&spec, &genes),
            Err(SpawnError::Arena {
                arena: ArenaKind::Parts,
                reason: AllocationFailure::BlockLimit,
            })
        );
        assert_eq!(world.storage_usage(), before);
        assert_eq!(world.population(), 0);
        assert_eq!(world.pool.incarnations(), incarnations);
    }

    #[test]
    fn founders_share_one_set_of_innovation_ids() {
        // Shared ancestry is what makes crossover and genetic distance mean anything.
        // Fresh ids per founder would make every agent its own lineage (spec §3.1).
        let mut w = small_world();
        let a = w.spawn_founder(Vec3::new(1.0, 1.0, 0.0)).unwrap();
        let b = w.spawn_founder(Vec3::new(2.0, 2.0, 0.0)).unwrap();
        let ids_a: Vec<_> = w.genome(a).iter().map(|g| g.sort_key()).collect();
        let ids_b: Vec<_> = w.genome(b).iter().map(|g| g.sort_key()).collect();
        assert_eq!(ids_a, ids_b, "founders diverged structurally");
        assert_ne!(w.genome(a), w.genome(b), "founders got identical scalars");
    }

    #[test]
    fn a_genome_is_valid_and_freed_with_its_agent() {
        let mut w = small_world();
        let id = w.spawn_founder(Vec3::ZERO).unwrap();
        assert!(genome::validate(w.genome(id)).is_ok());
        assert_eq!(w.genes.live_blocks(), 1);
        w.despawn(id);
        assert_eq!(w.genes.live_blocks(), 0, "genome block leaked on death");
    }

    #[test]
    fn body_traits_come_from_the_genome() {
        // The point of carrying size and colour genetically: nothing outside the
        // genome has to remember to copy them.
        let mut w = small_world();
        let id = w.spawn_founder(Vec3::ZERO).unwrap();
        let size = genome::body_trait(w.genome(id), BodyTrait::Size).unwrap();
        assert_eq!(w.agents().size[id.index()], size);
        let r = genome::body_trait(w.genome(id), BodyTrait::SignatureR).unwrap();
        assert_eq!(w.agents().signature[id.index()].x, r);
    }
}
