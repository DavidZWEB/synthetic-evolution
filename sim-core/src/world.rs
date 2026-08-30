//! `World`: everything one simulation owns, and the entry points for changing it.
//!
//! A process must be able to hold several worlds at once — the native shell runs
//! parameter sweeps that way, and the random-brain control population is a second
//! world beside the first. That is why nothing here is `static`, including the
//! innovation counter, which is a plain field (spec §7.2, CLAUDE.md invariant 3).
//!
//! Deliberately not here yet: the tick. `step()` arrives at M8 with the 11 phases of
//! spec §2.4 in `tick.rs`; this module owns state and lifecycle only.

use crate::agents::{Agents, SpawnSpec};
use crate::arena::Arena;
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
        let brain_stride = Self::brain_stride(&params);
        Ok(Self {
            rng: Rng::from_seed(seed),
            tick: 0,
            next_innovation: 0,
            pool: SlotPool::with_capacity(capacity),
            agents: Agents::with_capacity(capacity),
            brains: Arena::with_capacity(capacity, brain_stride),
            parts: Arena::with_capacity(capacity, PARTS_PER_AGENT),
            hash: SpatialHash::new(
                params.world.size,
                params.sensing.max_sense_radius(),
                capacity,
            ),
            params,
        })
    }

    /// Neurons per brain under Phase 1's fixed topology.
    ///
    /// Sensor and effector counts are hardcoded this phase; they become genetic in
    /// Phase 2, at which point this becomes the arena's stride ceiling rather than
    /// every agent's exact size.
    fn brain_stride(params: &SimParams) -> u32 {
        let inputs = params.sensing.vision_rays * 4 // distance + signature RGB
            + 3 // chemo: concentration + gradient x/y
            + 1; // interoception: own energy
        let outputs = 4; // thrust, turn, ingest, reproduce
        inputs + outputs + params.brain.hidden_neurons + params.brain.oscillators
    }

    /// Claims a slot and its arena blocks. `None` when any pool is full — a normal
    /// condition at the population ceiling, not an error.
    pub fn spawn(&mut self, spec: &SpawnSpec) -> Option<AgentId> {
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
        self.agents.init(id, spec, brain, parts);
        Some(id)
    }

    /// Returns a slot and its arena blocks. Despawning a dead agent is a no-op, so a
    /// double death cannot free the same block twice.
    pub fn despawn(&mut self, id: AgentId) -> bool {
        if !self.pool.is_alive(id) {
            return false;
        }
        let i = id.index();
        let (brain, parts) = (self.agents.brain[i], self.agents.parts[i]);
        if !brain.is_empty() {
            self.brains.free(brain);
        }
        if !parts.is_empty() {
            self.parts.free(parts);
        }
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

    fn spec_at(x: f32, y: f32) -> SpawnSpec {
        SpawnSpec {
            position: Vec3::new(x, y, 0.0),
            yaw: 0.0,
            energy: SimParams::default().reproduction.start_energy,
            size: 3.0,
            signature: Vec3::splat(0.5),
            parent_a: AgentId::NULL,
        }
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
        let a = w.spawn(&spec_at(1.0, 2.0)).unwrap();
        let b = w.spawn(&spec_at(3.0, 4.0)).unwrap();
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
            .map(|i| w.spawn(&spec_at(i as f32, 0.0)).unwrap())
            .collect();
        let mut offsets: Vec<u32> = ids
            .iter()
            .map(|&id| w.agents().brain[id.index()].offset())
            .collect();
        offsets.sort();
        offsets.dedup();
        assert_eq!(offsets.len(), 32, "two agents are sharing a brain");
        assert!(
            w.spawn(&spec_at(0.0, 0.0)).is_none(),
            "full pool must refuse"
        );
    }

    #[test]
    fn arena_blocks_come_back_on_death() {
        let mut w = small_world();
        let ids: Vec<_> = (0..32)
            .map(|i| w.spawn(&spec_at(i as f32, 0.0)).unwrap())
            .collect();
        assert!(w.spawn(&spec_at(0.0, 0.0)).is_none());
        for id in &ids {
            w.despawn(*id);
        }
        assert_eq!(w.population(), 0);
        // If blocks leaked, the pool would have slots but the arena would not.
        for i in 0..32 {
            assert!(
                w.spawn(&spec_at(i as f32, 0.0)).is_some(),
                "arena leaked a block"
            );
        }
    }

    #[test]
    fn a_recycled_slot_starts_with_a_blank_brain() {
        let mut w = small_world();
        let a = w.spawn(&spec_at(0.0, 0.0)).unwrap();
        let block = w.agents().brain[a.index()];
        w.brains.get_mut(block).fill(0.75);
        w.despawn(a);
        let b = w.spawn(&spec_at(0.0, 0.0)).unwrap();
        assert!(
            w.brain(b).iter().all(|&x| x == 0.0),
            "inherited the dead agent's activations"
        );
    }

    #[test]
    fn innovation_ids_are_per_world_and_monotonic() {
        // Two worlds in one process must not share a counter (spec §7.2).
        let mut a = small_world();
        let mut b = small_world();
        assert_eq!(a.next_innovation().raw(), 0);
        assert_eq!(a.next_innovation().raw(), 1);
        assert_eq!(
            b.next_innovation().raw(),
            0,
            "counter leaked between worlds"
        );
    }

    #[test]
    fn brain_width_covers_the_hardcoded_sensor_and_effector_set() {
        let params = SimParams::default();
        let stride = World::brain_stride(&params);
        let expected = params.sensing.vision_rays * 4
            + 3
            + 1
            + 4
            + params.brain.hidden_neurons
            + params.brain.oscillators;
        assert_eq!(stride, expected);
        assert_eq!(
            small_world().brain(AgentId::new(0)).len(),
            0,
            "no agent spawned yet"
        );
    }
}
