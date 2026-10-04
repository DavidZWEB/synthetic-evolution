//! Shared native/WASM sensor-inheritance and sparse-founder scenario.
//!
//! Plant-funded births exercise new organs, removal, limits, and slot reuse.
//! These are controlled infrastructure cases, not evidence of ecological adaptation.
//!
//! M4 integration first refreshed only the control reference for real species labels.
//! The subsequent coverage-only refresh adds full classifier state to both values.
//! M5's additional coverage-only refresh includes lifetime birth identities.

use sim_core::World;
use sim_core::control::BrainInheritance;
use sim_core::genome::{Gene, Modality};
use sim_core::mutate::StructuralMutationCounts;
use sim_core::spawn::SpawnFailureCounts;

pub const EVOLVING_GOLDEN: u64 = 0x4686_ca3e_ba5b_bbfd;
pub const CONTROL_GOLDEN: u64 = 0x01ec_d732_c3a7_3111;

fn run(mode: BrainInheritance) -> u64 {
    let mut params = crate::scenario::params();
    params.world.max_agents = 4;
    params.plants.max_plants = 8;
    params.plants.max_energy = 300.0;
    params.feeding.rate = 300.0;
    params.reproduction.maturity_ticks = 0;
    params.sensing.vision_rays = 0;
    params.sensing.chemo_sensors = 1;
    params.sensing.energy_sensors = 0;
    params.brain.hidden_neurons = 0;
    params.brain.oscillators = 0;
    params.brain.connections_per_target = Some(1);
    params.storage.max_neurons = 11;
    // Exactly room for the founder (25 genes since Phase 3's muscle and mouth), one
    // vision sensor, and one connection, so the next addition hits the genome limit.
    params.storage.max_genes = 31;
    params.mutation.organs.add_sensor_rate = 1.0;
    params.mutation.organs.chemo_weight = 0.0;
    params.mutation.organs.energy_weight = 0.0;
    params.mutation.structural.add_connection_rate = 1.0;
    let mut world = World::new_with_brain_inheritance(42, params.clone(), mode).unwrap();
    let parent = world.spawn_founder(world.plants().position()[0]).unwrap();
    assert_eq!(world.genome(parent).len(), 25);
    world.intents_mut().ingest[parent.index()] = 1.0;
    world.resolve_feeding();
    world.intents_mut().reproduce[parent.index()] = 1.0;
    let mut edits = StructuralMutationCounts::default();
    let mut spawns = SpawnFailureCounts::default();
    assert_eq!(
        world.resolve_births_with_observers(
            |error| spawns.record(error),
            |event| edits.record(event)
        ),
        1
    );
    let child = world.pool().iter_live().find(|&id| id != parent).unwrap();
    assert_eq!(world.brain(child).len(), 11);
    assert_eq!(world.genome(child).len(), 31);
    assert_eq!(edits.add_sensor.unwrap().applied, 1);
    assert!(world.genome(child).iter().any(|gene| matches!(gene,
        Gene::Sensor(sensor) if sensor.modality == Modality::VisionRay)));

    params.mutation.organs.add_sensor_rate = 0.0;
    params.mutation.organs.remove_sensor_rate = 1.0;
    params.mutation.structural.add_connection_rate = 0.0;
    world.set_params(params.clone()).unwrap();
    world.intents_mut().reproduce[parent.index()] = 0.0;
    world.intents_mut().reproduce[child.index()] = 1.0;
    assert_eq!(
        world.resolve_births_with_observers(
            |error| spawns.record(error),
            |event| edits.record(event)
        ),
        1
    );
    let grandchild = world
        .pool()
        .iter_live()
        .find(|&id| id != parent && id != child)
        .unwrap();
    assert_eq!(world.brain(grandchild).len(), 11);
    assert_eq!(world.genome(grandchild).len(), 30);
    assert_eq!(edits.remove_sensor.unwrap().applied, 1);

    params.mutation.organs.add_sensor_rate = 1.0;
    params.mutation.organs.remove_sensor_rate = 0.0;
    world.set_params(params).unwrap();
    world.agents_mut().position[child.index()] = world.plants().position()[1];
    world.intents_mut().ingest[child.index()] = 1.0;
    world.resolve_feeding();
    assert_eq!(
        world.resolve_births_with_observers(
            |error| spawns.record(error),
            |event| edits.record(event)
        ),
        1
    );
    assert_eq!(edits.add_sensor.unwrap().genome_limit, 1);
    assert_eq!(world.population(), 4);
    let before = world.agents().energy[child.index()];
    assert_eq!(
        world.resolve_births_with_observers(
            |error| spawns.record(error),
            |event| edits.record(event)
        ),
        0
    );
    assert_eq!(world.agents().energy[child.index()], before);
    assert_eq!(spawns.pool_full, 1);
    assert!(world.despawn(grandchild));
    assert_eq!(
        world.resolve_births_with_observers(
            |error| spawns.record(error),
            |event| edits.record(event)
        ),
        1
    );
    for _ in 0..10 {
        world.step_with_observers(|error| spawns.record(error), |event| edits.record(event));
    }
    assert!((world.total_energy() - world.ledger().expected_stock()).abs() < 1e-6);
    world.state_hash()
}

pub fn check_organ_runs() {
    let hashes = (
        run(BrainInheritance::Evolving),
        run(BrainInheritance::RandomizedAtBirth),
    );
    assert_eq!(
        hashes,
        (EVOLVING_GOLDEN, CONTROL_GOLDEN),
        "organs evolving={:016x}, control={:016x}",
        hashes.0,
        hashes.1
    );
}
