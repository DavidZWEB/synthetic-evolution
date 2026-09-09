//! Shared native/WASM variable-storage continuation scenario.
//!
//! A hand-expanded founder feeds, buds, hits a pooled-neuron limit, and then runs
//! normal ticks. This is an infrastructure scenario, not evidence of adaptation.

use sim_core::control::BrainInheritance;
use sim_core::genome::{Activation, BodyTrait, Gene, NeuronGene, body_trait};
use sim_core::spawn::SpawnFailureCounts;
use sim_core::{AgentId, Rng, SimParams, SpawnSpec, World};

// M4 coverage-only refresh: include full classifier state (spec section 7.8).
pub const EVOLVING_GOLDEN: u64 = 0xd24c_2bd2_19c7_9add;
pub const CONTROL_GOLDEN: u64 = 0x4955_91ce_4b2b_95f7;

fn run(mode: BrainInheritance) -> u64 {
    let mut params = SimParams::default();
    params.world.max_agents = 4;
    params.plants.max_plants = 8;
    params.plants.max_energy = 300.0;
    params.feeding.rate = 300.0;
    params.reproduction.maturity_ticks = 0;
    params.storage.neurons_per_slot = 18;
    let mut world = World::new_with_brain_inheritance(42, params.clone(), mode).unwrap();
    let mut genes = vec![Gene::default(); world.founder_plan().len()];
    world
        .founder_plan()
        .instantiate(&mut Rng::from_seed(77), &params, &mut genes);
    for _ in 0..8 {
        genes.push(Gene::Neuron(NeuronGene {
            id: world.next_innovation().expect("IDs available"),
            bias: 0.0,
            tau: 1.0,
            activation: Activation::Sigmoid,
            period: 1.0,
        }));
    }
    genes.sort_unstable_by_key(Gene::sort_key);
    let spec = SpawnSpec {
        position: world.plants().position()[0],
        yaw: 0.0,
        energy: 0.0,
        size: body_trait(&genes, BodyTrait::Size).unwrap(),
        signature: [
            body_trait(&genes, BodyTrait::SignatureR).unwrap(),
            body_trait(&genes, BodyTrait::SignatureG).unwrap(),
            body_trait(&genes, BodyTrait::SignatureB).unwrap(),
        ]
        .into(),
        parent_a: AgentId::NULL,
    };
    let parent = world.spawn(&spec, &genes).unwrap();
    world.intents_mut().ingest[parent.index()] = 1.0;
    world.resolve_feeding();
    world.intents_mut().reproduce[parent.index()] = 1.0;
    let mut failures = SpawnFailureCounts::default();
    assert_eq!(
        world.resolve_births_with_observer(|error| failures.record(error)),
        1
    );
    assert_eq!(world.population(), 2);
    assert!(
        world
            .pool()
            .iter_live()
            .all(|id| world.brain(id).len() == 36)
    );

    world.agents_mut().position[parent.index()] = world.plants().position()[1];
    world.resolve_feeding();
    let before = world.agents().energy[parent.index()];
    assert_eq!(
        world.resolve_births_with_observer(|error| failures.record(error)),
        0
    );
    assert_eq!(failures.arena_capacity, 1);
    assert_eq!(world.agents().energy[parent.index()], before);
    for _ in 0..10 {
        world.step_with_spawn_observer(|error| failures.record(error));
    }
    assert!((world.total_energy() - world.ledger().expected_stock()).abs() < 1e-6);
    world.state_hash()
}

pub fn check_variable_storage_runs() {
    let hashes = (
        run(BrainInheritance::Evolving),
        run(BrainInheritance::RandomizedAtBirth),
    );
    assert_eq!(
        hashes,
        (EVOLVING_GOLDEN, CONTROL_GOLDEN),
        "variable evolving={:016x}, control={:016x}",
        hashes.0,
        hashes.1
    );
}
