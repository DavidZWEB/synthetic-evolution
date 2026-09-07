//! Variable-genome lifecycle and transactional storage regressions.
//!
//! Exercises real world callers rather than only the arena backend. Hand-built
//! genomes prove storage/reproduction contracts, not evolved structural usefulness.

use glam::Vec3;
use sim_core::arena::AllocationFailure;
use sim_core::control::BrainInheritance;
use sim_core::genome::{Activation, ConnectionGene, Gene, NeuronGene};
use sim_core::spawn::{ArenaKind, SpawnError, SpawnFailureCounts};
use sim_core::{InnovationId, SimParams, SpawnSpec, World};

fn params() -> SimParams {
    let mut params = SimParams::default();
    params.world.max_agents = 8;
    params.plants.max_plants = 8;
    params.reproduction.maturity_ticks = 0;
    params
}

fn spec() -> SpawnSpec {
    SpawnSpec {
        position: Vec3::new(500.0, 500.0, 0.0),
        energy: 300.0,
        size: 3.0,
        signature: Vec3::ONE,
        yaw: 0.0,
        parent_a: sim_core::AgentId::NULL,
    }
}

fn neurons(count: u32) -> Vec<Gene> {
    (0..count)
        .map(|id| {
            Gene::Neuron(NeuronGene {
                id: InnovationId::new(id),
                bias: 0.0,
                tau: 1.0,
                activation: Activation::Sigmoid,
                period: 1.0,
            })
        })
        .collect()
}

fn wired_genome(len: u32) -> Vec<Gene> {
    let mut genes = neurons(16);
    for id in 16..len {
        genes.push(Gene::Connection(ConnectionGene {
            id: InnovationId::new(id),
            from: InnovationId::new((id - 16) % 16),
            to: InnovationId::new((id - 16) / 16),
            weight: 0.0,
            enabled: true,
        }));
    }
    genes
}

#[test]
fn wider_and_smaller_brains_reproduce_in_both_modes_then_founder_scratch_recovers() {
    for mode in [
        BrainInheritance::Evolving,
        BrainInheritance::RandomizedAtBirth,
    ] {
        let mut world = World::new_with_brain_inheritance(42, params(), mode).unwrap();
        let parent = world.spawn(&spec(), &neurons(96)).unwrap();
        let small = world
            .spawn(
                &SpawnSpec {
                    energy: 1.0,
                    ..spec()
                },
                &neurons(2),
            )
            .unwrap();
        assert_eq!(world.brain(parent).len(), 96);
        assert_eq!(world.brain(small).len(), 2);
        world.intents_mut().reproduce[parent.index()] = 1.0;
        assert_eq!(world.resolve_births(), 1);
        let child = world
            .pool()
            .iter_live()
            .find(|&id| id != parent && id != small)
            .unwrap();
        assert_eq!(world.brain(child).len(), 96);
        assert_eq!(world.genome(child).len(), 96);
        let founder = world.spawn_founder(Vec3::ZERO).unwrap();
        assert_eq!(world.genome(founder).len(), world.founder_plan().len());
        world.step();
        assert!(
            world
                .brain(child)
                .iter()
                .all(|neuron| neuron.output.is_finite())
        );
    }
}

#[test]
fn every_variable_arena_refusal_rolls_back_all_claims_and_slot_identity() {
    for kind in [
        ArenaKind::Neurons,
        ArenaKind::Synapses,
        ArenaKind::Sensors,
        ArenaKind::Effectors,
        ArenaKind::Genes,
    ] {
        let mut params = params();
        params.world.max_agents = 4;
        match kind {
            ArenaKind::Neurons => params.storage.neurons_per_slot = 7,
            ArenaKind::Synapses => params.storage.synapses_per_slot = 60,
            ArenaKind::Sensors => params.storage.sensors_per_slot = 2,
            ArenaKind::Effectors => params.storage.effectors_per_slot = 1,
            ArenaKind::Genes => params.storage.genes_per_slot = 100,
            ArenaKind::Parts => unreachable!(),
        }
        let mut world = World::new(7, params).unwrap();
        let parent = world.spawn_founder(Vec3::ZERO).unwrap();
        let genes = world.genome(parent).to_vec();
        let hash = world.state_hash();
        let usage = world.storage_usage();
        let incarnations = world.pool().incarnations().to_vec();
        assert_eq!(
            world.spawn(&spec(), &genes),
            Err(SpawnError::Arena {
                arena: kind,
                reason: AllocationFailure::InsufficientSpace,
            })
        );
        assert_eq!(world.population(), 1);
        assert_eq!(world.storage_usage(), usage);
        assert_eq!(world.pool().incarnations(), incarnations);
        assert_eq!(world.state_hash(), hash);
        assert!(world.despawn(parent));
        assert!(world.spawn_founder(Vec3::ZERO).is_ok(), "leaked {kind:?}");
    }
}

#[test]
fn fragmented_genes_refuse_without_leaking_preceding_compiled_claims() {
    let mut params = params();
    params.storage.genes_per_slot = 50;
    let mut world = World::new(3, params).unwrap();
    let zero_energy = SpawnSpec {
        energy: 0.0,
        ..spec()
    };
    let _a = world.spawn(&zero_energy, &wired_genome(100)).unwrap();
    let gap = world.spawn(&zero_energy, &wired_genome(100)).unwrap();
    let _c = world.spawn(&zero_energy, &wired_genome(100)).unwrap();
    let tail = world.spawn(&zero_energy, &wired_genome(60)).unwrap();
    assert!(world.despawn(gap));
    let before = world.storage_usage();
    let hash = world.state_hash();
    assert_eq!(before[0].free_elements, 140);
    assert_eq!(before[0].largest_free_block, 100);
    assert_eq!(
        world.spawn(&zero_energy, &wired_genome(120)),
        Err(SpawnError::Arena {
            arena: ArenaKind::Genes,
            reason: AllocationFailure::Fragmented,
        })
    );
    assert_eq!(world.storage_usage(), before);
    assert_eq!(world.state_hash(), hash);
    assert!(world.despawn(tail));
    assert!(world.spawn(&zero_energy, &wired_genome(100)).is_ok());
}

#[test]
fn failed_birth_preserves_parent_energy_and_reports_storage_pressure() {
    let mut params = params();
    params.world.max_agents = 4;
    params.storage.genes_per_slot = 100;
    let mut world = World::new(8, params).unwrap();
    let parent = world.spawn_founder(Vec3::ZERO).unwrap();
    world.agents_mut().energy[parent.index()] = 300.0;
    world.agents_mut().energy_reserve[parent.index()] = 1e-8;
    world.intents_mut().reproduce[parent.index()] = 1.0;
    let before = (
        world.agents().energy[parent.index()],
        world.agents().energy_reserve[parent.index()],
    );
    let usage = world.storage_usage();
    let mut failures = SpawnFailureCounts::default();
    assert_eq!(
        world.resolve_births_with_observer(|error| failures.record(error)),
        0
    );
    assert_eq!(failures.arena_capacity, 1);
    assert_eq!(world.population(), 1);
    assert_eq!(world.storage_usage(), usage);
    assert_eq!(
        (
            world.agents().energy[parent.index()],
            world.agents().energy_reserve[parent.index()]
        ),
        before
    );
}

#[test]
fn invalid_or_oversized_genomes_are_rejected_before_claiming_resources() {
    let mut world = World::new(9, params()).unwrap();
    let before = world.state_hash();
    assert!(matches!(
        world.spawn(&spec(), &neurons(129)),
        Err(SpawnError::GenomeLimit {
            kind: "neurons",
            ..
        })
    ));
    let mut bad = neurons(2);
    bad.swap(0, 1);
    assert!(matches!(
        world.spawn(&spec(), &bad),
        Err(SpawnError::InvalidGenome(_))
    ));
    assert_eq!(world.state_hash(), before);
    assert_eq!(world.population(), 0);
}

#[test]
fn sensing_retunes_do_not_budget_a_replacement_grid() {
    let mut params = params();
    params.world.max_agents = 2;
    params.plants.max_plants = 0;
    params.storage.max_memory_bytes = params.estimated_construction_bytes().unwrap();
    let mut world = World::new(42, params.clone()).unwrap();
    let cell = world.spatial_hash().cell_size();
    params.sensing.vision_range = 50.0;
    assert!(
        params.validate().is_err(),
        "a new finer grid exceeds this budget"
    );
    world
        .set_params(params)
        .expect("live retuning retains the allocated grid");
    assert_eq!(world.spatial_hash().cell_size(), cell);
    assert_eq!(world.params().sensing.vision_range, 50.0);
}

#[test]
fn observing_refusals_does_not_change_simulation_state() {
    let mut params = params();
    params.storage.genes_per_slot = 50;
    let mut a = World::new(42, params.clone()).unwrap();
    let mut b = World::new(42, params).unwrap();
    assert_eq!(a.seed_founders(8), b.seed_founders(8));
    let mut failures = SpawnFailureCounts::default();
    for tick in 0..20 {
        let command = sim_core::command::Command::at(
            tick,
            sim_core::command::Kind::SpawnFounder {
                position: Vec3::ZERO,
            },
        );
        a.push_command(command.clone());
        b.push_command(command);
        a.step();
        b.step_with_spawn_observer(|error| failures.record(error));
        assert_eq!(a.state_hash(), b.state_hash());
    }
    assert!(failures.arena_capacity > 0);
}
