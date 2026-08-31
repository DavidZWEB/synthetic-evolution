//! `World`: everything one simulation owns, and the entry points for changing it.
//!
//! A process must be able to hold several worlds at once — the native shell runs
//! parameter sweeps that way, and the random-brain control population is a second
//! world beside the first. That is why nothing here is `static`, including the
//! innovation counter, which is a plain field (spec §7.2, CLAUDE.md invariant 3).
//!
//! Deliberately not here yet: the tick. `step()` arrives at M8 with the 11 phases of
//! spec §2.4 in `tick.rs`; this module owns state, lifecycle, and the thin wiring that
//! hands a system the slices belonging to one agent.

use glam::Vec3;

use crate::agents::{Agents, Handles, SpawnSpec};
use crate::arena::Arena;
use crate::brain::{self, Neuron, Synapse};
use crate::chemo::ChemoField;
use crate::effectors::{self, AgentIntents, Effector, Intents};
use crate::founder::FounderPlan;
use crate::genome::{self, BodyTrait, Gene};
use crate::ids::{AgentId, InnovationId};
use crate::ledger::EnergyLedger;
use crate::metabolism;
use crate::movement;
use crate::params::{ParamError, SimParams};
use crate::perceive::{self, SelfView, Sensor, WorldView};
use crate::plants::Plants;
use crate::pool::SlotPool;
use crate::rng::Rng;
use crate::spatial::SpatialHash;

/// One part per agent, at the agent's own origin, for all of V1 (spec §9.1).
const PARTS_PER_AGENT: u32 = 1;

/// A single simulation: its state, its parameters, and its random stream.
pub struct World {
    params: SimParams,
    rng: Rng,
    tick: u64,
    /// Monotonic source of [`InnovationId`]s. A field rather than a `static` so two
    /// worlds in one process cannot hand out ids from the same counter (spec §3.1).
    next_innovation: u32,
    pool: SlotPool,
    agents: Agents,
    /// Compiled neurons, one block per agent.
    brains: Arena<Neuron>,
    /// Compiled wiring, one block per agent. Separate from `brains` because a synapse
    /// and a neuron are different element types, not because they have different
    /// lifetimes — the two blocks are claimed and freed together.
    synapses: Arena<Synapse>,
    /// Compiled sensors, one block per agent: an organ's parameters and the brain slots
    /// it writes to, resolved at birth (spec §2.2c).
    sensors: Arena<Sensor>,
    /// Compiled effectors, one block per agent: the brain slot that drives each one.
    effectors: Arena<Effector>,
    /// What every agent's effectors asked for this tick. Written at step 4, drained by
    /// the systems that follow it (spec §2.4).
    intents: Intents,
    /// Gene lists, one block per agent.
    genes: Arena<Gene>,
    /// The founding topology, whose innovation ids every founder in this world shares
    /// (spec §3.1).
    plan: FounderPlan,
    /// Reusable buffer for building a genome before it is copied into the arena.
    /// Owned by the world and sized once, so a birth allocates nothing.
    genome_scratch: Vec<Gene>,
    /// Part offsets relative to the agent origin. One zeroed entry per agent in V1.
    parts: Arena<f32>,
    /// Neighbour lookup, rebuilt at the top of every tick (spec §2.4 step 1).
    hash: SpatialHash,
    /// Pheromone concentrations: what plants scent and what the chemo sensor reads.
    field: ChemoField,
    /// The autotrophs. Every joule in the world enters through them (spec §5.1).
    plants: Plants,
    /// Every joule that entered and left. The acceptance criterion for M7 is that this
    /// agrees with the stock actually present (spec §5.1).
    ledger: EnergyLedger,
    /// Slots that reached zero energy this tick, resolved at step 10 in agent-index
    /// order. Owned by the world and sized once, so a death allocates nothing.
    dying: Vec<AgentId>,
}

impl World {
    /// Builds an empty world.
    ///
    /// Params are validated here, at the boundary. Past this point the tick treats
    /// their invariants as established and uses `debug_assert!` rather than threading
    /// `Result` through the hot loop.
    pub fn new(seed: u64, params: SimParams) -> Result<Self, ParamError> {
        params.validate()?;
        let capacity = params.world.max_agents;
        let mut next_innovation = 0u32;
        // The plan draws the world's first innovation ids, before any agent exists.
        let plan = FounderPlan::new(&params, || {
            let id = InnovationId::new(next_innovation);
            next_innovation += 1;
            id
        });
        let brain_stride = plan.neuron_count() as u32;
        let synapse_stride = brain::synapse_count(plan.genes()) as u32;
        let sensor_stride = perceive::sensor_count(plan.genes()) as u32;
        let effector_stride = effectors::effector_count(plan.genes()) as u32;
        let genome_stride = plan.len() as u32;

        // One generator, drawn from in order: the plants are seeded first and agents
        // continue after them. A second `Rng::from_seed(seed)` would be the *same*
        // stream, making every plant coordinate bit-identical to the genome scalar
        // drawn at the same position — two processes that look independent and are not.
        let mut rng = Rng::from_seed(seed);
        let plants = Plants::new(&params, &mut rng);

        Ok(Self {
            rng,
            tick: 0,
            next_innovation,
            pool: SlotPool::with_capacity(capacity),
            agents: Agents::with_capacity(capacity),
            brains: Arena::with_capacity(capacity, brain_stride),
            synapses: Arena::with_capacity(capacity, synapse_stride),
            sensors: Arena::with_capacity(capacity, sensor_stride),
            effectors: Arena::with_capacity(capacity, effector_stride),
            intents: Intents::with_capacity(capacity),
            genes: Arena::with_capacity(capacity, genome_stride),
            genome_scratch: vec![Gene::default(); plan.len()],
            plan,
            parts: Arena::with_capacity(capacity, PARTS_PER_AGENT),
            hash: SpatialHash::new(
                params.world.size,
                params.sensing.max_sense_radius(),
                capacity,
            ),
            field: ChemoField::new(&params.chemo, params.world.size),
            // Opened against an empty world: plants start bare and no agent exists yet,
            // so every joule that ever appears has to arrive through `grow_plants`.
            ledger: EnergyLedger::opening(0.0),
            dying: Vec::with_capacity(capacity as usize),
            plants,
            params,
        })
    }

    /// Claims a slot and its arena blocks, and compiles the genome into a runnable
    /// brain. `None` when any pool is full — a normal condition at the population
    /// ceiling, not an error.
    ///
    /// **The caller owns the energy accounting.** This hands the new agent
    /// `spec.energy` and tells the ledger nothing, because the two ways an agent comes
    /// into existence account for it differently: a founder's tank is energy entering
    /// the world and is recorded as input by [`Self::spawn_founder`], while an
    /// offspring's is taken from its parent and is a transfer that must *not* be
    /// recorded at all. Getting this wrong is invisible until the conservation test
    /// runs, which is exactly why that test exists (spec §5.1).
    pub fn spawn(&mut self, spec: &SpawnSpec, genes: &[Gene]) -> Option<AgentId> {
        debug_assert!(
            genome::validate(genes).is_ok(),
            "spawning an incoherent genome"
        );
        let id = self.pool.alloc()?;
        let Some(handles) = self.claim_blocks(genes) else {
            self.pool.free(id);
            return None;
        };
        self.genes.get_mut(handles.genome).copy_from_slice(genes);
        // Compiled once, here, and never read from the genome again during a tick
        // — resolving an innovation id costs a binary search (`brain`, `perceive`).
        brain::compile(
            genes,
            self.brains.get_mut(handles.brain),
            self.synapses.get_mut(handles.synapses),
        );
        perceive::compile(genes, self.sensors.get_mut(handles.sensors));
        effectors::compile(genes, self.effectors.get_mut(handles.effectors));
        self.agents.init(id, spec, &handles);
        // Derived from the genome rather than the caller, and cached because metabolism
        // charges for them every tick and they cannot change while the agent lives.
        self.agents.brain_units[id.index()] = genome::brain_complexity(genes);
        self.agents.sensor_load[id.index()] = genome::sensor_load(genes);
        Some(id)
    }

    /// Claims one block from every per-agent arena, or none of them.
    ///
    /// Each arena holds `max_agents` blocks and they are claimed and freed in lockstep
    /// with the pool, so a partial failure is unreachable. It is unwound rather than
    /// asserted because a leaked block does not fail loudly — it shows up as a world
    /// that quietly stops accepting births some hours into a run.
    fn claim_blocks(&mut self, genes: &[Gene]) -> Option<Handles> {
        let brain = self.brains.alloc(genome::neuron_count(genes) as u32);
        let synapses = self.synapses.alloc(brain::synapse_count(genes) as u32);
        let sensors = self.sensors.alloc(perceive::sensor_count(genes) as u32);
        let effectors = self
            .effectors
            .alloc(effectors::effector_count(genes) as u32);
        let genome = self.genes.alloc(genes.len() as u32);
        let parts = self.parts.alloc(PARTS_PER_AGENT);

        match (brain, synapses, sensors, effectors, genome, parts) {
            (
                Some(brain),
                Some(synapses),
                Some(sensors),
                Some(effectors),
                Some(genome),
                Some(parts),
            ) => Some(Handles {
                brain,
                synapses,
                sensors,
                effectors,
                genome,
                parts,
            }),
            _ => {
                if let Some(block) = brain {
                    self.brains.free(block);
                }
                if let Some(block) = synapses {
                    self.synapses.free(block);
                }
                if let Some(block) = sensors {
                    self.sensors.free(block);
                }
                if let Some(block) = effectors {
                    self.effectors.free(block);
                }
                if let Some(block) = genome {
                    self.genes.free(block);
                }
                if let Some(block) = parts {
                    self.parts.free(block);
                }
                None
            }
        }
    }

    /// Spawns a founder: the world's fixed topology with fresh random scalars.
    ///
    /// Body traits come out of the genome rather than the caller, because that is the
    /// point of carrying them genetically — an offspring inherits its parent's size
    /// and colour without anything else having to remember to copy them.
    pub fn spawn_founder(&mut self, position: Vec3) -> Option<AgentId> {
        // Taken out of `self` so the borrow checker sees the buffer and the world as
        // separate; put back before returning.
        let mut scratch = core::mem::take(&mut self.genome_scratch);
        debug_assert_eq!(scratch.len(), self.plan.len());
        self.plan
            .instantiate(&mut self.rng, &self.params, &mut scratch);

        let yaw = self
            .rng
            .range(-core::f32::consts::PI, core::f32::consts::PI);
        let spec = SpawnSpec {
            position,
            yaw,
            energy: self.params.reproduction.start_energy,
            size: genome::body_trait(&scratch, BodyTrait::Size).unwrap_or(self.params.body.size),
            signature: Vec3::new(
                genome::body_trait(&scratch, BodyTrait::SignatureR).unwrap_or(0.5),
                genome::body_trait(&scratch, BodyTrait::SignatureG).unwrap_or(0.5),
                genome::body_trait(&scratch, BodyTrait::SignatureB).unwrap_or(0.5),
            ),
            parent_a: AgentId::NULL,
        };
        let spawned = self.spawn(&spec, &scratch);
        self.genome_scratch = scratch;
        if spawned.is_some() {
            // A founder's tank is the one energy source that is not a plant. It is a
            // boundary condition — the experimenter seeding a world — not an ongoing
            // leak, and recording it as input is what keeps §5.1's books balanced
            // without pretending the agent arrived empty. An *offspring* is different:
            // its energy comes out of its parent, so reproduction records nothing.
            self.ledger.record_input(spec.energy);
        }
        spawned
    }

    /// One agent's genes.
    #[inline]
    pub fn genome(&self, id: AgentId) -> &[Gene] {
        self.genes.get(self.agents.genome[id.index()])
    }

    /// Returns a slot and its arena blocks. Despawning a dead agent is a no-op, so a
    /// double death cannot free the same block twice.
    pub fn despawn(&mut self, id: AgentId) -> bool {
        if !self.pool.is_alive(id) {
            return false;
        }
        let i = id.index();
        self.brains.free(self.agents.brain[i]);
        self.synapses.free(self.agents.synapses[i]);
        self.sensors.free(self.agents.sensors[i]);
        self.effectors.free(self.agents.effectors[i]);
        self.genes.free(self.agents.genome[i]);
        self.parts.free(self.agents.parts[i]);
        self.agents.clear(id);
        self.pool.free(id)
    }

    /// Runs every live agent's sensors and writes what they return into its brain.
    /// Step 2 of the tick (spec §2.4).
    ///
    /// A whole pass before [`Self::step_brains`], not fused with it: every agent must
    /// perceive the same world, and a fused loop would let agent 0's decision reach
    /// agent 1's eye within the same tick.
    pub fn perceive_all(&mut self) {
        for id in self.pool.iter_live() {
            let i = id.index();
            let agent = SelfView {
                index: id.raw(),
                position: self.agents.position[i],
                orientation: self.agents.orientation[i],
                energy_tanks: self.agents.energy[i] / self.params.reproduction.start_energy,
            };
            let world = WorldView {
                positions: &self.agents.position,
                signatures: &self.agents.signature,
                sizes: &self.agents.size,
                hash: &self.hash,
                field: &self.field,
                plants: &self.plants,
                plant_radius: self.params.plants.radius,
                plant_signature: Vec3::from(self.params.plants.signature),
            };
            perceive::perceive(
                self.sensors.get(self.agents.sensors[i]),
                &agent,
                &world,
                self.brains.get_mut(self.agents.brain[i]),
            );
        }
    }

    /// Advances every live brain one Euler step. Step 3 of the tick (spec §2.4).
    ///
    /// Lives here rather than in `brain` for the reason `rebuild_spatial_hash` does:
    /// this is the only place that knows which blocks belong to which agent. The step
    /// itself takes plain slices and is testable without a world.
    ///
    /// Agent-index order, so that nothing about the result depends on pool layout —
    /// though with every brain reading only the previous step's outputs, the order is
    /// belt as well as braces here.
    pub fn step_brains(&mut self) {
        let dt = self.params.world.dt;
        for id in self.pool.iter_live() {
            let i = id.index();
            let (brain, synapses) = (self.agents.brain[i], self.agents.synapses[i]);
            brain::step(self.brains.get_mut(brain), self.synapses.get(synapses), dt);
        }
    }

    /// Reads every live agent's effectors into the intent buffer. Step 4 of the tick
    /// (spec §2.4).
    ///
    /// Nothing here changes the world. The buffer is cleared first, so an agent that
    /// lost an effector coasts rather than repeating its last request forever.
    pub fn drive_effectors(&mut self) {
        self.intents.clear();
        let movement = &self.params.movement;
        for id in self.pool.iter_live() {
            let i = id.index();
            effectors::drive(
                self.effectors.get(self.agents.effectors[i]),
                self.brains.get(self.agents.brain[i]),
                movement,
                &mut AgentIntents {
                    thrust: &mut self.intents.thrust[i],
                    turn: &mut self.intents.turn[i],
                    ingest: &mut self.intents.ingest[i],
                    reproduce: &mut self.intents.reproduce[i],
                },
            );
        }
    }

    /// Applies the movement intents. Step 5 of the tick (spec §2.4).
    ///
    /// Split from [`Self::drive_effectors`] rather than fused with it because every
    /// agent must decide against the same world: an agent early in the pool moving
    /// before a later one has chosen is exactly what the intent buffer exists to
    /// prevent.
    pub fn integrate_movement(&mut self) {
        for id in self.pool.iter_live() {
            let i = id.index();
            movement::integrate(
                &mut self.agents.position[i],
                &mut self.agents.velocity[i],
                &mut self.agents.orientation[i],
                self.intents.thrust[i],
                self.intents.turn[i],
                &self.params.movement,
                &self.params.world,
            );
        }
    }

    /// What every agent's effectors asked for on the most recent step 4.
    #[inline]
    pub fn intents(&self) -> &Intents {
        &self.intents
    }

    /// Diffuses and decays the pheromone field by one tick. Step 8 of the tick
    /// (spec §2.4), after whatever deposited into it.
    pub fn update_chemo(&mut self) {
        self.field.update(&self.params.chemo);
    }

    /// Rebuilds the neighbour grid from current positions. Step 1 of the tick.
    ///
    /// Lives here rather than in `spatial` because it is the only place that knows
    /// which slices belong together; the hash itself takes plain slices so it can be
    /// tested without a world.
    pub fn rebuild_spatial_hash(&mut self) {
        self.hash.rebuild(
            &self.agents.position,
            self.pool.alive_flags(),
            &mut self.agents.grid_cell,
        );
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

    /// Grows the plants by one tick and scents the field. Part of step 8, and the only
    /// place energy enters the world (spec §5.1).
    ///
    /// What the plants *actually* absorbed goes into the ledger, not the nominal input
    /// rate: at carrying capacity the surplus never enters, and conservation has to be
    /// measured rather than inferred.
    pub fn grow_plants(&mut self) -> f32 {
        let dt = self.params.world.dt;
        let absorbed = self.plants.grow(&self.params.plants, dt);
        self.ledger.record_input(absorbed);
        self.plants.scent(&mut self.field, &self.params.plants, dt);
        absorbed
    }

    /// Charges every live agent its upkeep and notes who ran out. Step 9 of the tick
    /// (spec §2.4).
    ///
    /// An agent is charged only what it has left, so energy never goes negative and the
    /// amount dissipated is exactly the amount that existed. The death itself is
    /// deferred to step 10: mutating the pool here would make free-list allocation
    /// depend on iteration order, which is the fastest way to lose determinism.
    pub fn charge_metabolism(&mut self) {
        self.dying.clear();
        for id in self.pool.iter_live() {
            let i = id.index();
            let cost = metabolism::cost_per_tick(
                self.agents.size[i],
                self.agents.brain_units[i],
                self.agents.sensor_load[i],
                self.intents.thrust[i],
                &self.params.metabolism,
            );
            // Only what is there. Charging past zero would dissipate energy the world
            // never held, and the ledger would report a leak that is really an
            // overdraft.
            let charged = cost.min(self.agents.energy[i]).max(0.0);
            self.agents.energy[i] -= charged;
            self.ledger.record_dissipated(charged);
            if self.agents.energy[i] <= 0.0 {
                self.dying.push(id);
            }
        }
    }

    /// Removes the agents that ran out of energy. Part of step 10 (spec §2.4).
    ///
    /// Agent-index order, because `iter_live` is ascending and that is what fills
    /// `dying`. Deaths resolved in any other order would hand the free list back in a
    /// different sequence and the next births would land in different slots.
    ///
    /// A dying agent holds no energy — `charge_metabolism` took exactly what was left —
    /// so nothing is dissipated here and nothing is lost. Corpses that return part of
    /// an agent to the world arrive with predation in Phase 3.
    pub fn resolve_deaths(&mut self) -> usize {
        let dying = core::mem::take(&mut self.dying);
        let mut removed = 0;
        for &id in &dying {
            debug_assert!(
                self.agents.energy[id.index()] <= 0.0,
                "despawning an agent that still holds energy"
            );
            if self.despawn(id) {
                removed += 1;
            }
        }
        self.dying = dying;
        self.dying.clear();
        removed
    }

    /// Every joule the world currently holds, in plants and in agents.
    pub fn total_energy(&self) -> f32 {
        let agents: f32 = self
            .pool
            .iter_live()
            .map(|id| self.agents.energy[id.index()])
            .sum();
        self.plants.total_energy() + agents
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
    pub fn next_innovation(&mut self) -> InnovationId {
        let id = InnovationId::new(self.next_innovation);
        self.next_innovation += 1;
        id
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
    use glam::Vec3;

    fn small_world() -> World {
        let mut params = SimParams::default();
        params.world.max_agents = 32;
        World::new(7, params).expect("defaults are valid")
    }

    #[test]
    fn rejects_invalid_params_at_construction() {
        let mut params = SimParams::default();
        params.world.dt = 0.0;
        assert!(World::new(1, params).is_err());
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
            w.spawn_founder(Vec3::new(0.0, 0.0, 0.0)).is_none(),
            "full pool must refuse"
        );
    }

    #[test]
    fn arena_blocks_come_back_on_death() {
        let mut w = small_world();
        let ids: Vec<_> = (0..32)
            .map(|i| w.spawn_founder(Vec3::new(i as f32, 0.0, 0.0)).unwrap())
            .collect();
        assert!(w.spawn_founder(Vec3::new(0.0, 0.0, 0.0)).is_none());
        for id in &ids {
            w.despawn(*id);
        }
        assert_eq!(w.population(), 0);
        // If blocks leaked, the pool would have slots but the arena would not.
        for i in 0..32 {
            assert!(
                w.spawn_founder(Vec3::new(i as f32, 0.0, 0.0)).is_some(),
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
                w.spawn_founder(Vec3::new(i as f32, 0.0, 0.0)).is_some(),
                "the world leaked its arena blocks"
            );
        }
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
        let first = a.next_innovation();
        assert!(
            first.raw() > 0,
            "the founding plan should already have drawn ids"
        );
        assert_eq!(a.next_innovation().raw(), first.raw() + 1);
        assert_eq!(b.next_innovation(), first, "counter leaked between worlds");
    }

    #[test]
    fn brain_width_matches_the_founding_topology() {
        let w = small_world();
        assert_eq!(w.brains.stride() as usize, w.founder_plan().neuron_count());
        assert_eq!(w.genes.stride() as usize, w.founder_plan().len());
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
