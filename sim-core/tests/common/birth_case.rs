//! Native/WASM identity continuation with plant-funded reproduction and slot reuse.
//!
//! Pins persistent references, not an archive or a claim of adaptive ancestry.

use glam::Vec3;
use sim_core::control::BrainInheritance;
use sim_core::ids::BirthId;
use sim_core::{SimParams, World};

pub const EVOLVING_GOLDEN: u64 = 0xb65b_101b_e21b_cb91;
pub const CONTROL_GOLDEN: u64 = 0x5402_abcc_03c4_c75e;

fn run(mode: BrainInheritance) -> u64 {
    let mut params = SimParams::default().without_structural_mutation();
    params.world.max_agents = 3;
    params.plants.max_plants = 1;
    params.plants.max_energy = 300.0;
    params.feeding.rate = 300.0;
    params.reproduction.maturity_ticks = 0;
    let mut world = World::new_with_brain_inheritance(42, params, mode).unwrap();
    let parent = world.spawn_founder(world.plants().position()[0]).unwrap();
    world.intents_mut().ingest[parent.index()] = 1.0;
    world.resolve_feeding();
    world.intents_mut().reproduce[parent.index()] = 1.0;
    assert_eq!(world.resolve_births(), 1);
    let child = world.pool().iter_live().find(|&id| id != parent).unwrap();
    assert_eq!(world.agents().birth_id[child.index()], BirthId::new(1));
    assert_eq!(
        world.agents().parent_birth_a[child.index()],
        BirthId::new(0)
    );
    assert_eq!(world.agents().parent_birth_b[child.index()], BirthId::NULL);
    world.despawn(parent);
    let replacement = world.spawn_founder(Vec3::ZERO).unwrap();
    assert_eq!(replacement, parent);
    assert_eq!(
        world.agents().birth_id[replacement.index()],
        BirthId::new(2)
    );
    assert_eq!(
        world.agents().parent_birth_a[child.index()],
        BirthId::new(0)
    );
    let last = world.spawn_founder(Vec3::ZERO).unwrap();
    assert_eq!(world.agents().birth_id[last.index()], BirthId::new(3));
    assert!(world.spawn_founder(Vec3::ZERO).is_err());
    world.despawn(last);
    let last = world.spawn_founder(Vec3::ZERO).unwrap();
    assert_eq!(world.agents().birth_id[last.index()], BirthId::new(4));
    for _ in 0..5 {
        world.step();
    }
    assert_eq!(
        world.agents().parent_birth_a[child.index()],
        BirthId::new(0)
    );
    assert!((world.total_energy() - world.ledger().expected_stock()).abs() < 1e-6);
    world.state_hash()
}

pub fn check_birth_identity_runs() {
    let hashes = (
        run(BrainInheritance::Evolving),
        run(BrainInheritance::RandomizedAtBirth),
    );
    assert_eq!(
        hashes,
        (EVOLVING_GOLDEN, CONTROL_GOLDEN),
        "birth identities evolving={:016x}, control={:016x}",
        hashes.0,
        hashes.1
    );
}
