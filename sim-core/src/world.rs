//! `World`: everything one simulation owns, and the entry points for changing it.
//!
//! A process must be able to hold several worlds at once — the native shell runs
//! parameter sweeps that way, and the random-brain control population is a second
//! world beside the first. That is why nothing here is `static`, including the
//! innovation counter, which is a plain field (spec §7.2, CLAUDE.md invariant 3).
//!
//! Deliberately not here yet: the tick. `step()` arrives at M8 with the 11 phases of
//! spec §2.4 in `tick.rs`; this module owns state and lifecycle only.

use glam::Vec3;

use crate::agents::{Agents, SpawnSpec};
use crate::arena::Arena;
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
    /// Neuron activations, one block per agent.
    brains: Arena<f32>,
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
        let genome_stride = plan.len() as u32;
        Ok(Self {
            rng: Rng::from_seed(seed),
            tick: 0,
            next_innovation,
            pool: SlotPool::with_capacity(capacity),
            agents: Agents::with_capacity(capacity),
            brains: Arena::with_capacity(capacity, brain_stride),
            genes: Arena::with_capacity(capacity, genome_stride),
            genome_scratch: vec![Gene::default(); plan.len()],
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

    /// Claims a slot and its arena blocks. `None` when any pool is full — a normal
    /// condition at the population ceiling, not an error.
    pub fn spawn(&mut self, spec: &SpawnSpec, genes: &[Gene]) -> Option<AgentId> {
        debug_assert!(
            genome::validate(genes).is_ok(),
            "spawning an incoherent genome"
        );
        let id = self.pool.alloc()?;
        let Some(brain) = self.brains.alloc(self.brains.stride()) else {
            self.pool.free(id);
            return None;
        };
        let Some(parts) = self.parts.alloc(PARTS_PER_AGENT) else {
            self.brains.free(brain);
            self.pool.free(id);
            return None;
        };
        let Some(genome) = self.genes.alloc(genes.len() as u32) else {
            self.parts.free(parts);
            self.brains.free(brain);
            self.pool.free(id);
            return None;
        };
        self.genes.get_mut(genome).copy_from_slice(genes);
        self.agents.init(id, spec, brain, parts);
        self.agents.genome[id.index()] = genome;
        Some(id)
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
        let (brain, parts, genome) = (
            self.agents.brain[i],
            self.agents.parts[i],
            self.agents.genome[i],
        );
        self.brains.free(brain);
        self.parts.free(parts);
        self.genes.free(genome);
        self.agents.clear(id);
        self.pool.free(id)
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

    /// Activations of one agent's brain.
    #[inline]
    pub fn brain(&self, id: AgentId) -> &[f32] {
        self.brains.get(self.agents.brain[id.index()])
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
        let block = w.agents().brain[a.index()];
        w.brains.get_mut(block).fill(0.75);
        w.despawn(a);
        let b = w.spawn_founder(Vec3::new(0.0, 0.0, 0.0)).unwrap();
        assert!(
            w.brain(b).iter().all(|&x| x == 0.0),
            "inherited the dead agent's activations"
        );
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
