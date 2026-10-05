//! The render snapshot: the narrow projection of world state a frame needs.
//!
//! Spec §2.2b's render fields: agent appearance and identity, what an attack needs to
//! be drawn, plant position and current stock, and corpse position and energy. A slot incarnation lets click
//! selection survive free-list reuse without exposing private agent state. No agent
//! energy, genomes, or brain state. The buffer is written once per tick and read by the
//! main thread at whatever rate it happens to be drawing, so every field added is
//! bandwidth paid 60 times a second at capacity.
//!
//! **Slot-indexed, not compacted.** Arrays are `capacity` long and `alive` says which
//! entries are real, which costs a byte per slot and buys the thing compaction destroys:
//! index `i` is the same agent next tick as it was this one. That is what lets a click
//! identify an agent, and what any interpolation between snapshots would need
//! (spec §9.5). A compacted buffer renumbers everything the moment one agent dies.
//!
//! **Sized once, at capacity, and never grown.** Growing WASM memory detaches every JS
//! typed-array view over it, silently (spec §7.3). This is the buffer JS actually views,
//! and at 72 bytes per agent it is under 1% of per-agent state — 0.36 MB at the default
//! 5000 — so pre-allocating it is free and removes the hazard rather than managing it.
//!
//! Deliberately not here: inspector data. A genome, a lineage, and live activations are
//! pulled for one selected agent on demand through the command queue, at human speed,
//! rather than streamed for everybody (spec §2.2b).

use crate::world::World;

/// Bytes one agent occupies across every array here. Spec §7.5 budgets the snapshot at
/// 72 bytes per agent, 11 of them Phase 3's combat fields, and the arithmetic that makes
/// pre-allocation obviously free rests on it staying that way.
pub const BYTES_PER_AGENT: usize = 12 + 16 + 4 + 12 + 1 + 4 + 4 + 4 + 4 + 1 + 1 + 1 + 8;

/// Bytes one plant occupies: its position and current stock (spec §2.2b).
pub const BYTES_PER_PLANT: usize = 12 + 4;

/// Bytes one corpse slot occupies: its position and energy (spec §2.2b).
pub const BYTES_PER_CORPSE: usize = 12 + 4;

/// A frame's worth of world state, in struct-of-arrays form.
///
/// One array per field rather than interleaved records: a renderer uploads per-instance
/// attributes as separate buffers, so this is the layout WebGL2 wants, and JS can view
/// each array as a single typed array without a `DataView` or an unaligned read.
pub struct Snapshot {
    tick: u64,
    /// Slots, live or not. Constant for the life of the snapshot.
    capacity: u32,
    /// How many of those slots currently hold an agent. A count, not a bound — the
    /// arrays stay `capacity` long and the live ones are wherever `alive` says.
    population: u32,
    position: Vec<f32>,
    orientation: Vec<f32>,
    size: Vec<f32>,
    signature: Vec<f32>,
    alive: Vec<u8>,
    species: Vec<u32>,
    /// Always 0 and always 1 in V1. Present because the parts indirection is cashed in
    /// at Phase 5 and a renderer that assumed one part per agent would have to be
    /// rewritten rather than extended (spec §3.5, §9.1).
    part_offset: Vec<u32>,
    part_count: Vec<u32>,
    /// Allocation generation for each slot, so a recycled index is a new identity.
    incarnation: Vec<u32>,
    /// Health quantized to a byte, 255 at full: enough to draw a wound (spec §2.2b).
    health: Vec<u8>,
    /// Ticks since each agent last swung and since it was last hit, saturating at 255, so
    /// a renderer that skipped frames still learns that it happened.
    swing_age: Vec<u8>,
    hurt_age: Vec<u8>,
    /// `x, y` per slot: where the latest swing landed, the victim's centre, or NaN in both
    /// for a miss. A position rather than a slot, because the victim's slot may already
    /// hold a newborn by the time the frame is published (spec §2.2b).
    bite_at: Vec<f32>,
    plant_capacity: u32,
    plant_position: Vec<f32>,
    /// What each site currently holds. An emptied plant stays in the world and stays
    /// drawable — the site persists and regrows (spec §5.1) — so this is what tells a
    /// fat one from a bare one.
    plant_energy: Vec<f32>,
    corpse_capacity: u32,
    corpse_position: Vec<f32>,
    /// What each corpse slot holds. Zero marks a free slot, so the frame needs no
    /// liveness array: decomposition removes a corpse before it can hold nothing
    /// (spec §5.1), and a corpse that holds nothing is not food to draw.
    corpse_energy: Vec<f32>,
}

impl Snapshot {
    /// Allocates every array at capacity. The only allocation this type ever does.
    pub fn new(capacity: u32, plant_capacity: u32, corpse_capacity: u32) -> Self {
        let n = capacity as usize;
        let p = plant_capacity as usize;
        let c = corpse_capacity as usize;
        Self {
            tick: 0,
            capacity,
            population: 0,
            position: vec![0.0; n * 3],
            orientation: vec![0.0; n * 4],
            size: vec![0.0; n],
            signature: vec![0.0; n * 3],
            alive: vec![0; n],
            species: vec![0; n],
            part_offset: vec![0; n],
            part_count: vec![0; n],
            incarnation: vec![0; n],
            health: vec![0; n],
            swing_age: vec![u8::MAX; n],
            hurt_age: vec![u8::MAX; n],
            bite_at: vec![f32::NAN; n * 2],
            plant_capacity,
            plant_position: vec![0.0; p * 3],
            plant_energy: vec![0.0; p],
            corpse_capacity,
            corpse_position: vec![0.0; c * 3],
            corpse_energy: vec![0.0; c],
        }
    }

    /// A snapshot sized for `world`, which is the only size it can usefully be.
    pub fn for_world(world: &World) -> Self {
        Self::new(
            world.pool().capacity(),
            world.plants().len() as u32,
            world.corpses().len() as u32,
        )
    }

    /// Projects the world into this buffer, overwriting whatever it held.
    ///
    /// Dead slots have their `alive` byte cleared and are otherwise left alone. Zeroing
    /// them would cost a pass over the whole capacity every tick to hide values nothing
    /// reads — `alive` is what a reader checks, and a stale position behind a cleared
    /// flag is not visible to anything that respects it.
    pub fn update(&mut self, world: &World) {
        debug_assert_eq!(
            self.capacity,
            world.pool().capacity(),
            "snapshot sized for a different world"
        );
        self.tick = world.tick_count();
        self.population = world.population();

        // Every slot's flag, then only the live slots' data. `alive_flags` is already
        // one byte per slot in pool order, so this is a copy rather than a scan.
        self.alive.copy_from_slice(world.pool().alive_flags());
        self.incarnation
            .copy_from_slice(world.pool().incarnations());

        let agents = world.agents();
        for id in world.pool().iter_live() {
            let i = id.index();
            let p = agents.position[i];
            self.position[i * 3] = p.x;
            self.position[i * 3 + 1] = p.y;
            self.position[i * 3 + 2] = p.z;

            let q = agents.orientation[i];
            self.orientation[i * 4] = q.x;
            self.orientation[i * 4 + 1] = q.y;
            self.orientation[i * 4 + 2] = q.z;
            self.orientation[i * 4 + 3] = q.w;

            self.size[i] = agents.size[i];

            let s = agents.signature[i];
            self.signature[i * 3] = s.x;
            self.signature[i * 3 + 1] = s.y;
            self.signature[i * 3 + 2] = s.z;

            self.species[i] = agents.species_id[i];
            self.part_offset[i] = agents.parts[i].offset();
            self.part_count[i] = agents.parts[i].len();

            // A living agent's health lies in (0, 1], so this rounds onto 0..=255.
            self.health[i] = (agents.health[i] * 255.0 + 0.5) as u8;
            self.swing_age[i] = agents.swing_age[i];
            self.hurt_age[i] = agents.hurt_age[i];
            let [x, y] = agents.bite_at[i];
            self.bite_at[i * 2] = x;
            self.bite_at[i * 2 + 1] = y;
        }

        // Every plant, every frame. There is no alive flag to respect: an eaten plant
        // regrows, and a dead one has already reseeded at its new site (spec §5.1).
        let plants = world.plants();
        for (i, (&position, &energy)) in plants
            .position()
            .iter()
            .zip(plants.energy().iter())
            .enumerate()
        {
            self.plant_position[i * 3] = position.x;
            self.plant_position[i * 3 + 1] = position.y;
            self.plant_position[i * 3 + 2] = position.z;
            self.plant_energy[i] = energy;
        }

        // Every corpse slot, a free one as zero energy. The pool is a few thousand
        // slots, so writing them all is cheaper than a liveness array in every frame.
        let corpses = world.corpses();
        for (i, (&position, &energy)) in corpses
            .position()
            .iter()
            .zip(corpses.energy().iter())
            .enumerate()
        {
            if corpses.alive()[i] == 1 {
                self.corpse_position[i * 3] = position.x;
                self.corpse_position[i * 3 + 1] = position.y;
                self.corpse_position[i * 3 + 2] = position.z;
                self.corpse_energy[i] = energy;
            } else {
                self.corpse_energy[i] = 0.0;
            }
        }
    }

    /// The tick this snapshot was taken at.
    ///
    /// On every snapshot because ordering needs it and lockstep would need it, and it
    /// costs eight bytes once rather than per agent (spec §9.5).
    #[inline]
    pub fn tick(&self) -> u64 {
        self.tick
    }

    #[inline]
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    #[inline]
    pub fn population(&self) -> u32 {
        self.population
    }

    /// `x, y, z` per slot.
    #[inline]
    pub fn position(&self) -> &[f32] {
        &self.position
    }

    /// `x, y, z, w` per slot.
    #[inline]
    pub fn orientation(&self) -> &[f32] {
        &self.orientation
    }

    #[inline]
    pub fn size(&self) -> &[f32] {
        &self.size
    }

    /// `r, g, b` per slot.
    #[inline]
    pub fn signature(&self) -> &[f32] {
        &self.signature
    }

    /// 1 where a slot holds an agent. Everything else here is meaningless where this is 0.
    #[inline]
    pub fn alive(&self) -> &[u8] {
        &self.alive
    }

    #[inline]
    pub fn species(&self) -> &[u32] {
        &self.species
    }

    #[inline]
    pub fn part_offset(&self) -> &[u32] {
        &self.part_offset
    }

    #[inline]
    pub fn part_count(&self) -> &[u32] {
        &self.part_count
    }

    #[inline]
    pub fn incarnation(&self) -> &[u32] {
        &self.incarnation
    }

    /// Health per slot, 0..=255 for none to full.
    #[inline]
    pub fn health(&self) -> &[u8] {
        &self.health
    }

    /// Ticks since each slot's agent last swung, saturating at 255.
    #[inline]
    pub fn swing_age(&self) -> &[u8] {
        &self.swing_age
    }

    /// Ticks since each slot's agent was last hit, saturating at 255.
    #[inline]
    pub fn hurt_age(&self) -> &[u8] {
        &self.hurt_age
    }

    /// `x, y` per slot where the latest swing landed; NaN in both for a miss.
    #[inline]
    pub fn bite_at(&self) -> &[f32] {
        &self.bite_at
    }

    #[inline]
    pub fn plant_capacity(&self) -> u32 {
        self.plant_capacity
    }

    /// `x, y, z` per plant.
    #[inline]
    pub fn plant_position(&self) -> &[f32] {
        &self.plant_position
    }

    #[inline]
    pub fn plant_energy(&self) -> &[f32] {
        &self.plant_energy
    }

    #[inline]
    pub fn corpse_capacity(&self) -> u32 {
        self.corpse_capacity
    }

    /// `x, y, z` per corpse slot; meaningless where the slot's energy is 0.
    #[inline]
    pub fn corpse_position(&self) -> &[f32] {
        &self.corpse_position
    }

    /// What each corpse slot holds; 0 for a free slot.
    #[inline]
    pub fn corpse_energy(&self) -> &[f32] {
        &self.corpse_energy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::SimParams;
    use glam::Vec3;

    fn world_of(agents: u32, capacity: u32) -> World {
        let mut params = SimParams::default();
        params.world.max_agents = capacity;
        params.plants.max_plants = 16;
        let mut world = World::new(19, params).expect("defaults are valid");
        for i in 0..agents {
            world
                .spawn_founder(Vec3::new(200.0 + i as f32 * 15.0, 500.0, 0.0))
                .expect("pool has room");
        }
        world
    }

    #[test]
    fn the_larder_travels_with_the_frame() {
        // Depleted sites persist and regrow, so they remain in the frame at zero stock
        // rather than disappearing (spec §2.2b, §5.1).
        let mut world = world_of(2, 8);
        let mut snap = Snapshot::for_world(&world);
        snap.update(&world);

        assert_eq!(snap.plant_capacity(), world.plants().len() as u32);
        assert!(snap.plant_capacity() > 0, "no plants to draw");
        assert_eq!(
            snap.plant_position().len(),
            snap.plant_capacity() as usize * 3
        );

        // Stocked at construction, so the very first frame already has food in it.
        assert!(snap.plant_energy().iter().all(|&e| e > 0.0));
        for (i, &position) in world.plants().position().iter().enumerate() {
            assert_eq!(snap.plant_position()[i * 3], position.x);
        }

        // And an eaten site stays drawable rather than disappearing.
        for _ in 0..200 {
            world.step();
        }
        snap.update(&world);
        assert_eq!(snap.plant_capacity(), world.plants().len() as u32);
    }

    #[test]
    fn one_plant_costs_sixteen_bytes() {
        let snap = Snapshot::new(1, 1, 0);
        let bytes = snap.plant_position().len() * 4 + snap.plant_energy().len() * 4;
        assert_eq!(bytes, BYTES_PER_PLANT);
        assert_eq!(bytes, 16);
    }

    #[test]
    fn one_agent_costs_the_budgeted_seventy_two_bytes() {
        // Spec §7.5 budgets the snapshot at 72 bytes per agent, and the argument that
        // pre-allocating it at capacity is free rests on that number. A field added
        // without noticing is bandwidth paid 60 times a second forever.
        let snap = Snapshot::new(1, 0, 0);
        let bytes = snap.position().len() * 4
            + snap.orientation().len() * 4
            + snap.size().len() * 4
            + snap.signature().len() * 4
            + snap.alive().len()
            + snap.species().len() * 4
            + snap.part_offset().len() * 4
            + snap.part_count().len() * 4
            + snap.incarnation().len() * 4
            + snap.health().len()
            + snap.swing_age().len()
            + snap.hurt_age().len()
            + snap.bite_at().len() * 4;
        assert_eq!(bytes, BYTES_PER_AGENT);
        assert_eq!(bytes, 72);
    }

    #[test]
    fn an_attack_travels_with_the_frame() {
        // Health as a byte, both ages, and where the swing landed: what a renderer
        // needs to draw a bite, a wound, and a hit (spec §2.2b).
        let mut world = world_of(2, 4);
        let [biter, victim] = [0, 1].map(|n| world.pool().iter_live().nth(n).unwrap().index());
        let mut snap = Snapshot::for_world(&world);
        snap.update(&world);
        assert_eq!(&snap.health()[..2], &[255, 255], "founders start whole");
        assert_eq!(&snap.swing_age()[..2], &[255, 255]);
        assert!(snap.bite_at()[..4].iter().all(|v| v.is_nan()));

        let agents = world.agents_mut();
        agents.health[victim] = 0.5;
        agents.swing_age[biter] = 0;
        agents.bite_at[biter] = [215.0, 500.0];
        agents.hurt_age[victim] = 3;
        snap.update(&world);
        assert_eq!(
            snap.health()[victim],
            128,
            "half health rounds to 128 of 255"
        );
        assert_eq!(snap.swing_age()[biter], 0);
        assert_eq!(snap.hurt_age()[victim], 3);
        assert_eq!(&snap.bite_at()[biter * 2..biter * 2 + 2], &[215.0, 500.0]);
        assert!(
            snap.bite_at()[victim * 2].is_nan(),
            "the victim has not swung"
        );
    }

    #[test]
    fn one_corpse_slot_costs_sixteen_bytes() {
        let snap = Snapshot::new(0, 0, 1);
        let bytes = snap.corpse_position().len() * 4 + snap.corpse_energy().len() * 4;
        assert_eq!(bytes, BYTES_PER_CORPSE);
        assert_eq!(bytes, 16);
    }

    #[test]
    fn a_corpse_travels_with_the_frame_and_a_free_slot_holds_nothing() {
        let mut world = world_of(4, 16);
        let mut snap = Snapshot::for_world(&world);
        assert_eq!(snap.corpse_capacity(), world.corpses().len() as u32);
        snap.update(&world);
        assert!(snap.corpse_energy().iter().all(|&e| e == 0.0));

        // Founders hold energy, so a removed one leaves a corpse where it stood.
        let victim = world.pool().iter_live().nth(2).expect("four agents");
        let at = world.agents().position[victim.index()];
        world.despawn(victim);
        snap.update(&world);
        let slot = world
            .corpses()
            .alive()
            .iter()
            .position(|&alive| alive == 1)
            .expect("the death left a corpse");
        assert_eq!(snap.corpse_position()[slot * 3], at.x);
        assert_eq!(snap.corpse_position()[slot * 3 + 1], at.y);
        assert_eq!(snap.corpse_energy()[slot], world.corpses().energy()[slot]);
        assert!(snap.corpse_energy()[slot] > 0.0);

        // Once it decomposes, its slot is free again and the frame stops drawing it.
        let mut params = world.params().clone();
        params.corpses.min_energy = 1e9;
        world.set_params(params).expect("a live retune");
        world.step();
        assert_eq!(world.corpses().count(), 0);
        snap.update(&world);
        assert_eq!(snap.corpse_energy()[slot], 0.0);
    }

    #[test]
    fn it_carries_what_the_world_holds() {
        let world = world_of(4, 16);
        let mut snap = Snapshot::for_world(&world);
        snap.update(&world);

        assert_eq!(snap.population(), 4);
        assert_eq!(snap.tick(), 0);
        for id in world.pool().iter_live() {
            let i = id.index();
            assert_eq!(snap.alive()[i], 1);
            assert_eq!(snap.position()[i * 3], world.agents().position[i].x);
            assert_eq!(snap.position()[i * 3 + 1], world.agents().position[i].y);
            assert_eq!(
                snap.orientation()[i * 4 + 3],
                world.agents().orientation[i].w
            );
            assert_eq!(snap.size()[i], world.agents().size[i]);
            assert_eq!(snap.signature()[i * 3], world.agents().signature[i].x);
            assert_eq!(snap.part_count()[i], 1, "one part per agent in V1");
            assert_eq!(snap.incarnation()[i], world.pool().incarnation(id).unwrap());
        }
    }

    #[test]
    fn a_dead_slot_is_marked_dead() {
        let mut world = world_of(4, 16);
        let mut snap = Snapshot::for_world(&world);
        let victim = world.pool().iter_live().nth(1).expect("four agents");
        world.despawn(victim);
        snap.update(&world);

        assert_eq!(snap.population(), 3);
        assert_eq!(snap.alive()[victim.index()], 0);
        assert_eq!(
            snap.alive().iter().filter(|&&a| a == 1).count(),
            3,
            "exactly the survivors"
        );
    }

    #[test]
    fn a_slot_keeps_its_identity_across_ticks() {
        // The reason this is slot-indexed rather than compacted. A renderer that wants
        // to interpolate, or a click that wants to name an agent, needs index `i` to
        // mean the same agent it meant last tick — which compaction destroys the moment
        // anything dies.
        let mut world = world_of(6, 16);
        let mut snap = Snapshot::for_world(&world);

        let watched = world.pool().iter_live().nth(4).expect("six agents");
        let doomed = world.pool().iter_live().nth(1).expect("six agents");
        world.despawn(doomed);
        snap.update(&world);
        let before = snap.position()[watched.index() * 3];

        for _ in 0..30 {
            world.step();
        }
        snap.update(&world);
        assert_eq!(snap.alive()[watched.index()], 1, "the watched agent died");
        assert_ne!(
            snap.position()[watched.index() * 3],
            before,
            "it never moved, so this proves nothing about its identity"
        );
        assert_eq!(snap.alive()[doomed.index()], 0, "a dead slot came back");
    }

    #[test]
    fn the_tick_travels_with_the_frame() {
        let mut world = world_of(2, 8);
        let mut snap = Snapshot::for_world(&world);
        for expected in 0..5 {
            snap.update(&world);
            assert_eq!(snap.tick(), expected);
            world.step();
        }
    }

    #[test]
    fn updating_is_deterministic() {
        let run = || {
            let mut world = world_of(8, 16);
            let mut snap = Snapshot::for_world(&world);
            for _ in 0..40 {
                world.step();
            }
            snap.update(&world);
            (snap.position().to_vec(), snap.alive().to_vec(), snap.tick())
        };
        assert_eq!(run(), run());
    }
}
