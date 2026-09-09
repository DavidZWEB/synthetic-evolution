//! Lifetime identities survive pool reuse without changing admission or reproduction.
//!
//! These check parent capture and metadata, not retained ancestry/history or sex.

use glam::Vec3;
use sim_core::ids::{AgentId, BirthId, NULL_ID};
use sim_core::spawn::SpawnError;
use sim_core::{SimParams, SpawnSpec, World};

fn params(capacity: u32) -> SimParams {
    let mut params = SimParams::default();
    params.world.max_agents = capacity;
    params.plants.max_plants = 0;
    params.species.capacity = 2;
    params.reproduction.maturity_ticks = 0;
    params
}

#[test]
fn founders_receive_world_local_ids_not_recycled_slot_ids() {
    let mut a = World::new(42, params(2)).unwrap();
    let mut b = World::new(42, params(2)).unwrap();
    let first = a.spawn_founder(Vec3::ZERO).unwrap();
    let other = b.spawn_founder(Vec3::ZERO).unwrap();
    assert_eq!(a.agents().birth_id[first.index()], BirthId::new(0));
    assert_eq!(b.agents().birth_id[other.index()], BirthId::new(0));
    assert_eq!(a.agents().parent_birth_a[first.index()], BirthId::NULL);
    assert_eq!(a.agents().parent_birth_b[first.index()], BirthId::NULL);
    for expected in 1..5 {
        assert!(a.despawn(first));
        assert_eq!(a.agents().birth_id[first.index()], BirthId::NULL);
        let replacement = a.spawn_founder(Vec3::ZERO).unwrap();
        assert_eq!(replacement, first);
        assert_eq!(
            a.agents().birth_id[replacement.index()],
            BirthId::new(expected)
        );
    }
    assert_eq!(b.agents().birth_id[other.index()], BirthId::new(0));
}

#[test]
fn persistent_parent_is_captured_once_and_survives_parent_slot_reuse() {
    let mut world = World::new(42, params(3)).unwrap();
    let parent = world.spawn_founder(Vec3::ZERO).unwrap();
    let parent_birth = world.agents().birth_id[parent.index()];
    world.agents_mut().energy[parent.index()] = 300.0;
    world.intents_mut().reproduce[parent.index()] = 1.0;
    assert_eq!(world.resolve_births(), 1);
    let child = world.pool().iter_live().find(|&id| id != parent).unwrap();
    assert_eq!(world.agents().birth_id[child.index()], BirthId::new(1));
    assert_eq!(world.agents().parent_birth_a[child.index()], parent_birth);
    assert_eq!(world.agents().parent_birth_b[child.index()], BirthId::NULL);
    assert_eq!(world.agents().parent_a[child.index()], parent.raw());
    assert_eq!(world.agents().parent_b[child.index()], NULL_ID);
    world.despawn(parent);
    let replacement = world.spawn_founder(Vec3::ZERO).unwrap();
    assert_eq!(replacement, parent);
    assert_eq!(
        world.agents().birth_id[replacement.index()],
        BirthId::new(2)
    );
    assert_eq!(world.agents().parent_birth_a[child.index()], parent_birth);
    assert_eq!(
        world.agents().parent_birth_a[replacement.index()],
        BirthId::NULL
    );
}

#[test]
fn dead_parent_slot_cannot_become_the_newborns_own_parent() {
    let mut world = World::new(42, params(2)).unwrap();
    let parent = world.spawn_founder(Vec3::ZERO).unwrap();
    let genes = world.genome(parent).to_vec();
    world.despawn(parent);
    let child = world
        .spawn(
            &SpawnSpec {
                position: Vec3::ZERO,
                yaw: 0.0,
                energy: 0.0,
                size: 3.0,
                signature: Vec3::ONE,
                parent_a: parent,
            },
            &genes,
        )
        .unwrap();
    assert_eq!(child, parent, "fixture must reuse the referenced dead slot");
    assert_eq!(world.agents().birth_id[child.index()], BirthId::new(1));
    assert_eq!(world.agents().parent_birth_a[child.index()], BirthId::NULL);
    assert_eq!(
        world.agents().parent_a[child.index()],
        parent.raw(),
        "legacy slot is not reinterpreted"
    );
}

#[test]
fn unavailable_parent_slots_remain_unavailable_without_blocking_admission() {
    let mut world = World::new(42, params(3)).unwrap();
    let founder = world.spawn_founder(Vec3::ZERO).unwrap();
    let genes = world.genome(founder).to_vec();
    for parent in [AgentId::NULL, AgentId::new(999)] {
        let child = world
            .spawn(
                &SpawnSpec {
                    position: Vec3::ZERO,
                    yaw: 0.0,
                    energy: 0.0,
                    size: 3.0,
                    signature: Vec3::ONE,
                    parent_a: parent,
                },
                &genes,
            )
            .unwrap();
        assert_eq!(world.agents().parent_birth_a[child.index()], BirthId::NULL);
        assert_eq!(world.agents().parent_a[child.index()], parent.raw());
    }
    assert_eq!(world.population(), 3);
}

#[test]
fn pool_and_arena_refusals_do_not_consume_birth_ids() {
    for arena_refusal in [false, true] {
        let mut params = params(if arena_refusal { 3 } else { 1 });
        if arena_refusal {
            params.storage.genes_per_slot = 142;
        }
        let mut world = World::new(42, params).unwrap();
        let first = world.spawn_founder(Vec3::ZERO).unwrap();
        let failure = world.spawn_founder(Vec3::ZERO).unwrap_err();
        assert!(matches!(failure, SpawnError::Arena { .. }) == arena_refusal);
        assert_eq!(world.agents().birth_id[first.index()], BirthId::new(0));
        world.despawn(first);
        let replacement = world.spawn_founder(Vec3::ZERO).unwrap();
        assert_eq!(
            world.agents().birth_id[replacement.index()],
            BirthId::new(1)
        );
    }
}
