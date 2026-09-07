//! The forward-compatibility checklist, made executable.
//!
//! Spec §9.1 and §3.4 justify the hedges audited in the Phase 1 implementation plan's
//! forward-compatibility checklist. Their Phase 1 forms can look like unused code:
//! a `parentB` that is always `NULL`, a part count that is always 1, an elevation
//! that is always 0 — each reads as something a careful person would tidy away, and the
//! comment asking them not to is the only thing standing in the way.
//!
//! So this file stands in the way instead. Removing a hedge now fails a test that says
//! what it was for.
//!
//! Six of the eleven are already pinned where they live, and are not repeated here:
//!
//! | Hedge | Where it is tested |
//! |---|---|
//! | `position`/`velocity` are 3-component, z pinned | `movement::movement_stays_on_the_plane` |
//! | orientation is a yaw-constrained quaternion | `movement::turn_is_signed_and_stays_on_the_z_axis` |
//! | turn effector carries an axis pinned to Z | `effectors::the_turn_axis_is_carried_and_pinned_to_z` |
//! | innovation counter is a field, not a `static` | `tests/invariants.rs` (bans `static mut`, `OnceLock`, …) |
//! | crossover written and unit-tested, called by nothing | `crossover`'s own tests — deleting it fails them |
//! | mutations cross the boundary as a `Command` | `command`'s tests, including the serde round-trip |
//!
//! The five below had nothing holding them.

use glam::Vec3;
use sim_core::genome::{Gene, Modality};
use sim_core::ids::NULL_ID;
use sim_core::params::SimParams;
use sim_core::world::World;

/// Several founders, each rich enough to breed.
///
/// One agent is not enough: a fixed brain with near-constant inputs holds its reproduce
/// output roughly still, so whether a single founder ever clears the gate is a property
/// of its random weights rather than of the code under test.
fn breeders(seed: u64) -> World {
    let mut world = world(seed);
    for i in 0..8 {
        let id = world
            .spawn_founder(Vec3::new(300.0 + i as f32 * 25.0, 500.0, 0.0))
            .expect("room");
        world.agents_mut().energy[id.index()] = 10_000.0;
    }
    world
}

fn world(seed: u64) -> World {
    let mut params = SimParams::default();
    params.world.max_agents = 32;
    params.plants.max_plants = 32;
    World::new(seed, params).expect("defaults are valid")
}

#[test]
fn parent_b_exists_and_is_always_null() {
    // Asexual reproduction has one parent, so the second slot never holds anything in
    // Phase 1. It exists because sexual reproduction arrives at Phase 6 and adding a
    // field to a saved genome's agent record later is a format break (spec §9.1).
    let mut world = breeders(1);
    for id in world.pool().iter_live() {
        assert_eq!(world.agents().parent_b[id.index()], NULL_ID);
    }

    // And it stays null through a birth, which is the only other way an agent appears.
    for _ in 0..600 {
        world.step();
    }
    assert!(
        world.population() > 1,
        "nothing was born, so nothing is tested"
    );
    for id in world.pool().iter_live() {
        assert_eq!(
            world.agents().parent_b[id.index()],
            NULL_ID,
            "a second parent appeared in a phase that has no sexual reproduction"
        );
    }
}

#[test]
fn every_agent_carries_exactly_one_part_at_its_origin() {
    // The parts indirection is cashed in at Phase 5, not in some distant future
    // (spec §3.5, §9.1) — it is the big one on the list. Today every agent is a sphere
    // with one part at its own origin, and an agent that held its geometry directly
    // would make multi-part morphology a rewrite of every math site rather than a
    // change of loop bound.
    let world = breeders(2);
    for id in world.pool().iter_live() {
        let parts = world.parts_of(id);
        // One slot, holding the origin. What is hedged is the *indirection* — an agent
        // reaching its geometry through `(offset, len)` — so Phase 5 widens the arena's
        // element type without touching a single caller. An agent holding its geometry
        // inline would make that a rewrite of every math site instead.
        assert_eq!(parts.len(), 1, "every agent has exactly one part");
        assert_eq!(parts, [0.0], "the part sits at the agent's own origin");
    }
}

#[test]
fn sensor_elevation_is_carried_and_never_varies() {
    // `(azimuth, elevation)` pairs with elevation clamped to 0. This is the hedge whose
    // absence "invalidates every saved genome" (spec §9.1): a genome storing bare
    // azimuths cannot gain a second angle later without breaking every population ever
    // accumulated. Mutation must not touch it, and in Phase 1 nothing touches a sensor
    // gene at all.
    let mut world = breeders(3);
    let elevations = |world: &World, id| {
        world
            .genome(id)
            .iter()
            .filter_map(|g| match g {
                Gene::Sensor(s) if s.modality == Modality::VisionRay => Some(s.params[1]),
                _ => None,
            })
            .collect::<Vec<f32>>()
    };
    let first = world.pool().iter_live().next().expect("a founder");
    let before = elevations(&world, first);
    assert!(!before.is_empty(), "no directional sensor to check");
    assert!(
        before.iter().all(|&e| e == 0.0),
        "elevation started off-plane"
    );

    for _ in 0..600 {
        world.step();
    }
    assert!(
        world.population() > 1,
        "nothing was born, so nothing mutated"
    );
    for id in world.pool().iter_live() {
        assert!(
            elevations(&world, id).iter().all(|&e| e == 0.0),
            "a descendant's sensor left the plane; the elevation operator is not disabled"
        );
    }
}

#[test]
fn the_chemo_field_is_three_dimensional_with_depth_one() {
    let world = world(4);
    assert_eq!(world.chemo().dims()[2], 1, "V1 simulates on a plane");
    assert_eq!(world.chemo().dims().len(), 3, "a 2D grid would not extrude");

    // And the validator holds the shape, so the hedge cannot be undone by a params file.
    let mut params = SimParams::default();
    params.chemo.cells[2] = 4;
    assert!(
        World::new(4, params).is_err(),
        "depth other than 1 should be refused until volumetric 3D lands"
    );
}

#[test]
fn the_neighbour_query_is_a_three_axis_loop_pinned_to_one_z_cell() {
    // 9-cell to 27-cell neighbour iteration has to be a loop bound rather than a
    // rewrite (spec §9.1). The observable form of that is a Z axis one cell deep: the
    // loop already runs, it simply has nowhere to go.
    let world = world(5);
    assert_eq!(world.spatial_hash().dims()[2], 1);

    // Agents at the same x/y but different z are neighbours, because there is only one
    // Z cell for them to be in. When the plane is lifted, this is what changes.
    let mut params = SimParams::default();
    params.world.max_agents = 4;
    let mut world = World::new(5, params).expect("valid params");
    for _ in 0..2 {
        world
            .spawn_founder(Vec3::new(500.0, 500.0, 0.0))
            .expect("room");
    }
    world.rebuild_spatial_hash();
    let mut seen = 0;
    world.spatial_hash().for_each_within(
        &world.agents().position,
        Vec3::new(500.0, 500.0, 0.0),
        1.0,
        |_, _, _| seen += 1,
    );
    assert_eq!(seen, 2, "both agents should sit in one cell");
}
