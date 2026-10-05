//! Shared native/WASM variable-storage continuation scenario.
//!
//! A hand-expanded founder feeds, buds, hits a pooled-neuron limit, and then runs
//! normal ticks. This is an infrastructure scenario, not evidence of adaptation.

use sim_core::control::BrainInheritance;
use sim_core::genome::{Activation, BodyTrait, Gene, NeuronGene, body_trait};
use sim_core::spawn::SpawnFailureCounts;
use sim_core::{AgentId, Rng, SpawnSpec, World};

// Phase 3 M3 coverage refreshes: the hash covers each agent's bite cooldown (spec §4.2),
// then each agent's diet and the world's bite counts (spec §7.9).
// Before it, Phase 3 M2's founders gained muscle and mouth genes and the hash covered
// both (spec §3.5), and M5's coverage-only refresh added lifetime birth identities
// (spec section 7.8).
pub const EVOLVING_GOLDEN: u64 = 0xb464_d5bd_f8f3_d155;
pub const CONTROL_GOLDEN: u64 = 0xba62_cc4f_118b_5201;

const EXTRA: usize = 8;

fn run(mode: BrainInheritance) -> u64 {
    let mut params = crate::scenario::params();
    params.world.max_agents = 4;
    params.plants.max_plants = 8;
    params.plants.max_energy = 300.0;
    params.feeding.rate = 300.0;
    params.reproduction.maturity_ticks = 0;
    // Parents carry the founder brain plus eight neurons; the pooled neuron arena
    // holds exactly two such brains, so a third birth is refused.
    let brain = World::new(42, params.clone())
        .unwrap()
        .founder_plan()
        .neuron_count()
        + EXTRA;
    params.storage.neurons_per_slot = (brain as u32).div_ceil(2);
    let mut world = World::new_with_brain_inheritance(42, params.clone(), mode).unwrap();
    let mut genes = vec![Gene::default(); world.founder_plan().len()];
    world
        .founder_plan()
        .instantiate(&mut Rng::from_seed(77), &params, &mut genes);
    for _ in 0..EXTRA {
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
            .all(|id| world.brain(id).len() == brain)
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
