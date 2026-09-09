//! Shared native/WASM classified World continuation with exact energy accounting.
//!
//! Exercises same-tick retirement/recolonization, capacity, commands, and slot reuse.
//! This controlled fixture establishes mechanics, not adaptive species emergence.

use glam::Vec3;
use sim_core::command::{Command, Kind};
use sim_core::control::BrainInheritance;
use sim_core::genome::{Action, ConnectionGene, EffectorGene, Gene, NeuronGene};
use sim_core::ids::{AgentId, InnovationId, NULL_ID};
use sim_core::species::SpeciesEventCounts;
use sim_core::{SimParams, SpawnSpec, World};

pub const EVOLVING_GOLDEN: u64 = 0xb00e_97dc_437c_29dd;
pub const CONTROL_GOLDEN: u64 = 0xa37f_3777_7c76_e901;

fn genes(weight: f32) -> [Gene; 4] {
    [
        Gene::Neuron(NeuronGene {
            id: InnovationId::new(10_000),
            tau: 1.0,
            ..Default::default()
        }),
        Gene::Neuron(NeuronGene {
            id: InnovationId::new(10_001),
            tau: 1.0,
            ..Default::default()
        }),
        Gene::Effector(EffectorGene {
            id: InnovationId::new(10_003),
            action: Action::Reproduce,
            source: InnovationId::new(10_000),
            ..Default::default()
        }),
        Gene::Connection(ConnectionGene {
            id: InnovationId::new(10_002),
            from: InnovationId::new(10_000),
            to: InnovationId::new(10_001),
            weight,
            enabled: true,
        }),
    ]
}

fn run(mode: BrainInheritance) -> u64 {
    let mut params = SimParams::default();
    params.world.max_agents = 4;
    params.plants.max_plants = 1;
    params.plants.max_energy = 600.0;
    params.feeding.rate = 300.0;
    params.reproduction.gate = 0.0;
    params.reproduction.maturity_ticks = 0;
    params.mutation.weight_reset_rate = 1.0;
    params.mutation.weight_limit = 0.0;
    params.mutation.neuron_perturb_rate = 0.0;
    // Control redraw stays near the retired zero-weight root, far from the parent.
    params.brain.weight_init_scale = 0.01;
    params.species.capacity = 2;
    let mut world = World::new_with_brain_inheritance(42, params.clone(), mode).unwrap();
    let mut events = SpeciesEventCounts::default();
    let spec = SpawnSpec {
        position: world.plants().position()[0],
        yaw: 0.0,
        energy: 0.0,
        size: 3.0,
        signature: Vec3::ONE,
        parent_a: AgentId::NULL,
    };
    let first = world
        .spawn_with_species_observer(&spec, &genes(0.0), |e| events.record(e))
        .unwrap();
    let parent = world
        .spawn_with_species_observer(&spec, &genes(2.0), |e| events.record(e))
        .unwrap();
    world.intents_mut().ingest[parent.index()] = 1.0;
    world.resolve_feeding();
    params.feeding.rate = 0.0;
    world.set_params(params).unwrap();
    world.step_with_all_observers(
        |_| panic!("unexpected spawn refusal"),
        |_| {},
        |e| events.record(e),
    );
    assert_eq!(events.created, 3);
    assert_eq!(events.extinct, 1);
    assert_eq!(world.agents().species_id[first.index()], 2);
    assert_eq!(world.agents().parent_a[first.index()], parent.raw());
    world.push_command(Command::now(Kind::SpawnFounder {
        position: Vec3::ZERO,
    }));
    world.step_with_all_observers(
        |_| panic!("unexpected spawn refusal"),
        |_| {},
        |e| events.record(e),
    );
    let unclassified = world
        .pool()
        .iter_live()
        .find(|id| world.agents().species_id[id.index()] == NULL_ID)
        .unwrap();
    assert_eq!(world.unclassified_population(), 1);
    assert_eq!(events.unclassified_capacity, 1);
    world.despawn_with_species_observer(unclassified, |e| events.record(e));
    assert_eq!(events.extinct, 1);
    world.despawn_with_species_observer(parent, |e| events.record(e));
    assert_eq!(events.extinct, 2);
    let replacement = world
        .spawn_founder_with_species_observer(Vec3::ZERO, |e| events.record(e))
        .unwrap();
    assert_eq!(world.agents().species_id[replacement.index()], 3);
    assert_eq!(events.created, 4);
    assert_eq!(world.unclassified_population(), 0);
    for _ in 0..5 {
        world.step_with_all_observers(
            |_| panic!("unexpected spawn refusal"),
            |_| {},
            |e| events.record(e),
        );
    }
    assert_eq!(
        world
            .species()
            .active()
            .map(|(_, count)| count)
            .sum::<u32>(),
        world.population()
    );
    assert!((world.total_energy() - world.ledger().expected_stock()).abs() < 1e-6);
    world.state_hash()
}

pub fn check_classified_world_runs() {
    let hashes = (
        run(BrainInheritance::Evolving),
        run(BrainInheritance::RandomizedAtBirth),
    );
    assert_eq!(
        hashes,
        (EVOLVING_GOLDEN, CONTROL_GOLDEN),
        "classified evolving={:016x}, control={:016x}",
        hashes.0,
        hashes.1
    );
}
