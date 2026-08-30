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
use crate::arena::{Arena, Block};
use crate::brain::{self, Neuron, Synapse};
use crate::founder::FounderPlan;
use crate::genome::{self, BodyTrait, Gene};
use crate::ids::{AgentId, InnovationId};
use crate::params::{ParamError, SimParams};
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
    /// Gene lists, one block per agent.
    genes: Arena<Gene>,
    /// The founding topology, whose innovation ids every founder in this world shares
    /// (spec §3.1).
    plan: FounderPlan,
    /// Reusable buffer for building a genome before it is copied into the arena.
    /// Owned by the world and sized once, so a birth allocates nothing.
    genome_scratch: Vec<Gene>,
    /// Per-neuron input `I` for the brain being stepped, reused across agents. Sized
    /// once at the widest brain the world can hold, so a tick allocates nothing.
    drive_scratch: Vec<f32>,
    /// Part offsets relative to the agent origin. One zeroed entry per agent in V1.
    parts: Arena<f32>,
    /// Neighbour lookup, rebuilt at the top of every tick (spec §2.4 step 1).
    hash: SpatialHash,
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
        let genome_stride = plan.len() as u32;
        Ok(Self {
            rng: Rng::from_seed(seed),
            tick: 0,
            next_innovation,
            pool: SlotPool::with_capacity(capacity),
            agents: Agents::with_capacity(capacity),
            brains: Arena::with_capacity(capacity, brain_stride),
            synapses: Arena::with_capacity(capacity, synapse_stride),
            genes: Arena::with_capacity(capacity, genome_stride),
            genome_scratch: vec![Gene::default(); plan.len()],
            drive_scratch: vec![0.0; brain_stride as usize],
            plan,
            parts: Arena::with_capacity(capacity, PARTS_PER_AGENT),
            hash: SpatialHash::new(
                params.world.size,
                params.sensing.max_sense_radius(),
                capacity,
            ),
            params,
        })
    }

    /// Claims a slot and its arena blocks, and compiles the genome into a runnable
    /// brain. `None` when any pool is full — a normal condition at the population
    /// ceiling, not an error.
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
        // — resolving an innovation id costs a binary search (`brain`).
        brain::compile(
            genes,
            self.brains.get_mut(handles.brain),
            self.synapses.get_mut(handles.synapses),
        );
        self.agents.init(id, spec, &handles);
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
        let wiring = brain::synapse_count(genes) as u32;
        // A brain with no wiring holds no block at all: `Arena::free` cannot tell a
        // zero-length block from `Block::EMPTY` and would leak the allocation.
        let synapses = if wiring == 0 {
            Some(Block::EMPTY)
        } else {
            self.synapses.alloc(wiring)
        };
        let genome = self.genes.alloc(genes.len() as u32);
        let parts = self.parts.alloc(PARTS_PER_AGENT);

        match (brain, synapses, genome, parts) {
            (Some(brain), Some(synapses), Some(genome), Some(parts)) => Some(Handles {
                brain,
                synapses,
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
        self.genes.free(self.agents.genome[i]);
        self.parts.free(self.agents.parts[i]);
        self.agents.clear(id);
        self.pool.free(id)
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
            // Zero until perception writes sensor input here at M6 (spec §2.2c), so
            // for now a brain runs on its own recurrence and its oscillators.
            let drive = &mut self.drive_scratch[..brain.len() as usize];
            drive.fill(0.0);
            brain::step(
                self.brains.get_mut(brain),
                self.synapses.get(synapses),
                drive,
                dt,
            );
        }
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
        // One drive buffer is reused for every agent in a tick. If it were ever left
        // uncleared, one agent's synaptic sums would land on the next and behaviour
        // would become an artifact of pool order (spec §2.4) — which no determinism
        // test would catch, because it would be reproducibly wrong.
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
