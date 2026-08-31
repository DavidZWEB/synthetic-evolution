//! The agent state arrays: struct-of-arrays, fixed capacity, no per-agent objects.
//!
//! There is no `Agent` struct with methods. An agent is an index, and behaviour lives
//! in systems that take the slices they need (CLAUDE.md, spec §2.2a). Everything here
//! is authoritative world state owned by the sim and never handed to the renderer
//! directly; the snapshot is a narrow projection of a few of these fields.
//!
//! Several fields look like over-engineering for a 2D sim of spheres and are
//! deliberate forward-compatibility hedges — see the comments naming spec §9.1. They
//! cost a few bytes now and are expensive to retrofit once there are saved worlds and
//! evolved populations worth keeping. Do not remove them as unused.
//!
//! Deliberately not here: allocation policy (that is `pool`), variable-length data
//! (that is `arena`), and anything that reads more than one agent at a time.

use glam::{Quat, Vec3};

use crate::arena::Block;
use crate::ids::{AgentId, NULL_ID};

// The spec specifies these arrays as `Float32Array(N * 3)` and `Float32Array(N * 4)`.
// `Vec<Vec3>` and `Vec<Quat>` are exactly that layout while staying typed — but only
// while glam is built without SIMD, which pads Vec3 to 16 bytes. Enabling a glam
// feature that changes this would silently break every zero-copy view the wasm shell
// hands to JS (spec §7.3), so it is a compile error instead.
const _: () = assert!(
    size_of::<Vec3>() == 12,
    "Vec3 must stay a tight f32x3 for the snapshot"
);
const _: () = assert!(
    size_of::<Quat>() == 16,
    "Quat must stay a tight f32x4 for the snapshot"
);

/// Parallel arrays, one entry per pool slot. Entries for dead slots are stale, not
/// meaningful — liveness lives in `SlotPool`.
#[derive(Clone, Debug)]
pub struct Agents {
    /// World position. Z is pinned to 0 in V1 (spec §9.1).
    pub position: Vec<Vec3>,
    /// World velocity. Z is pinned to 0 in V1.
    pub velocity: Vec<Vec3>,
    /// Orientation as a quaternion, constrained to yaw about Z.
    ///
    /// A scalar heading would do for a plane. This is one of the two hedges in spec
    /// §2.2 that make volumetric 3D an unlock rather than a rewrite, and it is baked
    /// into the serialized world format — see `math::yaw_quat` (spec §9.1).
    pub orientation: Vec<Quat>,
    pub energy: Vec<f32>,
    /// Damage pool. Nothing reduces it before predation lands in Phase 3.
    pub health: Vec<f32>,
    pub age: Vec<u32>,
    /// Species cluster. Assigned by genetic distance from Phase 2; 0 for everyone now.
    pub species_id: Vec<u32>,
    /// Evolvable displayed colour. `vision_ray` returns it, and `set_signature` writes
    /// it from Phase 4 — together those give aposematism, crypsis, and mimicry
    /// (spec §4.2).
    pub signature: Vec<Vec3>,
    /// Collision and render radius.
    pub size: Vec<f32>,
    /// Phylogeny. `parent_b` is `NULL_ID` for every agent in V1.
    ///
    /// With sex, lineage stops being a tree and becomes a DAG. One `parent` field
    /// bakes a tree assumption into world state, the save format, and the tree viewer
    /// at once; two fields now, one of them dead, saves reworking all three
    /// (spec §3.4).
    pub parent_a: Vec<u32>,
    pub parent_b: Vec<u32>,
    /// Spatial hash bucket, rebuilt every tick.
    pub grid_cell: Vec<u32>,
    /// Handle into the brain arena: this agent's neurons and their state.
    ///
    /// Spec §2.2a writes a single `brainOffset` because it does not say how a brain is
    /// laid out. A compiled one is three blocks — neurons here, wiring in
    /// [`Self::synapses`], organs in [`Self::sensors`] — because they are different
    /// element types, not because they have different lifetimes. All three are claimed
    /// and freed together.
    pub brain: Vec<Block>,
    /// Handle into the synapse arena: this agent's wiring, endpoints already resolved
    /// to slots in [`Self::brain`].
    pub synapses: Vec<Block>,
    /// Handle into the sensor arena: this agent's organs, targets already resolved to
    /// slots in [`Self::brain`].
    pub sensors: Vec<Block>,
    /// Handle into the genome arena.
    pub genome: Vec<Block>,
    /// Handle into the parts arena. Exactly one part at the agent's origin in V1;
    /// the indirection is what makes Phase 5 morphology additive (spec §9.1).
    pub parts: Vec<Block>,
}

/// Everything needed to bring one agent into existence.
///
/// A struct rather than a long argument list, because the next phases add fields to it
/// and a seven-argument `spawn` is where the wrong two get swapped.
#[derive(Clone, Copy, Debug)]
pub struct SpawnSpec {
    pub position: Vec3,
    pub yaw: f32,
    pub energy: f32,
    pub size: f32,
    pub signature: Vec3,
    /// `AgentId::NULL` for a founder.
    pub parent_a: AgentId,
}

/// Where one agent's variable-length data lives: the arena blocks claimed for it.
///
/// Passed to [`Agents::init`] as a set rather than assigned field by field afterwards,
/// so a handle cannot be left holding the previous tenant's block — which reads as an
/// agent sharing a dead one's brain.
#[derive(Clone, Copy, Debug, Default)]
pub struct Handles {
    pub brain: Block,
    pub synapses: Block,
    pub sensors: Block,
    pub genome: Block,
    pub parts: Block,
}

impl Agents {
    pub fn with_capacity(capacity: u32) -> Self {
        let n = capacity as usize;
        Self {
            position: vec![Vec3::ZERO; n],
            velocity: vec![Vec3::ZERO; n],
            orientation: vec![Quat::IDENTITY; n],
            energy: vec![0.0; n],
            health: vec![0.0; n],
            age: vec![0; n],
            species_id: vec![0; n],
            signature: vec![Vec3::ZERO; n],
            size: vec![0.0; n],
            parent_a: vec![NULL_ID; n],
            parent_b: vec![NULL_ID; n],
            grid_cell: vec![0; n],
            brain: vec![Block::EMPTY; n],
            synapses: vec![Block::EMPTY; n],
            sensors: vec![Block::EMPTY; n],
            genome: vec![Block::EMPTY; n],
            parts: vec![Block::EMPTY; n],
        }
    }

    pub fn capacity(&self) -> u32 {
        self.position.len() as u32
    }

    /// Writes a freshly claimed slot. Every field is assigned, so nothing survives from
    /// the slot's previous tenant.
    pub fn init(&mut self, id: AgentId, spec: &SpawnSpec, handles: &Handles) {
        let i = id.index();
        debug_assert!(i < self.position.len(), "slot out of range");
        // Z pinned to 0 — the simulation plane. Spec §2.3 and §9.1.
        self.position[i] = Vec3::new(spec.position.x, spec.position.y, 0.0);
        self.velocity[i] = Vec3::ZERO;
        self.orientation[i] = crate::math::yaw_quat(spec.yaw);
        self.energy[i] = spec.energy;
        self.health[i] = 1.0;
        self.age[i] = 0;
        self.species_id[i] = 0;
        self.signature[i] = spec.signature;
        self.size[i] = spec.size;
        self.parent_a[i] = spec.parent_a.raw();
        // Always NULL in V1. The field is the hedge, not a placeholder to fill in.
        self.parent_b[i] = NULL_ID;
        self.grid_cell[i] = 0;
        self.brain[i] = handles.brain;
        self.synapses[i] = handles.synapses;
        self.sensors[i] = handles.sensors;
        self.genome[i] = handles.genome;
        self.parts[i] = handles.parts;
    }

    /// Clears the handles of a slot being returned to the pool, so a stale block can
    /// never be freed twice through a dead agent.
    pub fn clear(&mut self, id: AgentId) {
        let i = id.index();
        self.brain[i] = Block::EMPTY;
        self.synapses[i] = Block::EMPTY;
        self.sensors[i] = Block::EMPTY;
        self.genome[i] = Block::EMPTY;
        self.parts[i] = Block::EMPTY;
        self.energy[i] = 0.0;
        self.health[i] = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> SpawnSpec {
        SpawnSpec {
            position: Vec3::new(12.0, -4.0, 99.0),
            yaw: 1.0,
            energy: 100.0,
            size: 3.0,
            signature: Vec3::new(0.2, 0.4, 0.6),
            parent_a: AgentId::NULL,
        }
    }

    #[test]
    fn every_array_has_capacity_entries() {
        let a = Agents::with_capacity(16);
        assert_eq!(a.capacity(), 16);
        assert_eq!(a.velocity.len(), 16);
        assert_eq!(a.orientation.len(), 16);
        assert_eq!(a.parent_b.len(), 16);
        assert_eq!(a.parts.len(), 16);
    }

    #[test]
    fn init_pins_z_to_the_simulation_plane() {
        let mut a = Agents::with_capacity(4);
        a.init(AgentId::new(0), &spec(), &Handles::default());
        assert_eq!(a.position[0].z, 0.0, "V1 simulates on a plane (spec §2.3)");
        assert_eq!(a.velocity[0].z, 0.0);
    }

    #[test]
    fn init_leaves_orientation_on_the_z_axis() {
        let mut a = Agents::with_capacity(4);
        a.init(AgentId::new(0), &spec(), &Handles::default());
        let q = a.orientation[0];
        assert!(
            q.x.abs() < 1e-6 && q.y.abs() < 1e-6,
            "yaw-only constraint: {q:?}"
        );
        assert!((crate::math::yaw_of(q) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn parent_b_is_always_null() {
        // V1 is asexual. The field exists so lineage can become a DAG without
        // reworking the save format (spec §3.4) — it is never written.
        let mut a = Agents::with_capacity(4);
        let mut s = spec();
        s.parent_a = AgentId::new(2);
        a.init(AgentId::new(0), &s, &Handles::default());
        assert_eq!(a.parent_a[0], 2);
        assert_eq!(a.parent_b[0], NULL_ID);
    }

    #[test]
    fn init_overwrites_the_previous_tenant() {
        let mut a = Agents::with_capacity(2);
        let mut first = spec();
        first.energy = 500.0;
        first.parent_a = AgentId::new(1);
        a.init(AgentId::new(0), &first, &Handles::default());
        a.age[0] = 9_999;
        a.clear(AgentId::new(0));

        a.init(AgentId::new(0), &spec(), &Handles::default());
        assert_eq!(a.energy[0], 100.0);
        assert_eq!(a.age[0], 0);
        assert_eq!(a.parent_a[0], NULL_ID);
    }
}
