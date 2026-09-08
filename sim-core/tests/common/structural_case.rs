//! Shared native/WASM structural-inheritance scenario.
//!
//! Uses plant-funded births, applied edits, an arena refusal, a genome-limit refusal,
//! and removal/reuse. It exercises mechanisms, not ecological adaptation.

use sim_core::control::BrainInheritance;
use sim_core::mutate::structural::StructuralMutationCounts;
use sim_core::spawn::SpawnFailureCounts;
use sim_core::{SimParams, World};

// M3 coverage-only refresh: include the retained founder template (spec section 7.8).
pub const EVOLVING_GOLDEN: u64 = 0x42b9_0632_3a36_f0f7;
pub const CONTROL_GOLDEN: u64 = 0x9105_0e23_1dc6_9947;

fn run(mode: BrainInheritance) -> u64 {
    let mut params = SimParams::default();
    params.world.max_agents = 4;
    params.plants.max_plants = 8;
    params.plants.max_energy = 300.0;
    params.feeding.rate = 300.0;
    params.reproduction.maturity_ticks = 0;
    params.storage.genes_per_slot = 142;
    params.storage.max_genes = 284;
    let rates = &mut params.mutation.structural;
    rates.remove_connection_rate = 1.0;
    rates.remove_neuron_rate = 1.0;
    rates.toggle_connection_rate = 1.0;
    rates.add_connection_rate = 1.0;
    rates.add_neuron_rate = 1.0;
    let mut world = World::new_with_brain_inheritance(42, params.clone(), mode).unwrap();
    let parent = world.spawn_founder(world.plants().position()[0]).unwrap();
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
    assert!(world.genome(child).len() < world.genome(parent).len());
    for count in [
        edits.remove_connection.applied,
        edits.remove_neuron.applied,
        edits.toggle_connection.applied,
        edits.add_connection.applied,
        edits.add_neuron.applied,
    ] {
        assert_eq!(count, 1);
    }
    let before = world.agents().energy[parent.index()];
    assert_eq!(
        world.resolve_births_with_observers(
            |error| spawns.record(error),
            |event| edits.record(event)
        ),
        0
    );
    assert_eq!(spawns.arena_capacity, 1);
    assert_eq!(world.agents().energy[parent.index()], before);

    params.mutation.structural.remove_connection_rate = 0.0;
    params.mutation.structural.remove_neuron_rate = 0.0;
    params.mutation.structural.toggle_connection_rate = 0.0;
    params.mutation.structural.add_connection_rate = 0.0;
    world.set_params(params).unwrap();
    assert_eq!(
        world.resolve_births_with_observers(
            |error| spawns.record(error),
            |event| edits.record(event)
        ),
        0
    );
    assert_eq!(edits.add_neuron.genome_limit, 1);
    assert_eq!(spawns.arena_capacity, 2);
    assert!(world.despawn(child));
    assert_eq!(
        world.resolve_births_with_observers(
            |error| spawns.record(error),
            |event| edits.record(event)
        ),
        1
    );
    assert_eq!(edits.add_neuron.genome_limit, 2);
    assert_eq!(world.population(), 2);
    for _ in 0..10 {
        world.step_with_observers(|error| spawns.record(error), |event| edits.record(event));
    }
    assert!((world.total_energy() - world.ledger().expected_stock()).abs() < 1e-6);
    world.state_hash()
}

pub fn check_structural_runs() {
    let hashes = (
        run(BrainInheritance::Evolving),
        run(BrainInheritance::RandomizedAtBirth),
    );
    assert_eq!(
        hashes,
        (EVOLVING_GOLDEN, CONTROL_GOLDEN),
        "structural evolving={:016x}, control={:016x}",
        hashes.0,
        hashes.1
    );
}
