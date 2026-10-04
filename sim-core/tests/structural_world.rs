//! Structural inheritance through real world births.
//!
//! Hand-driven reproduction exercises operator outcomes, resource refusal, and both
//! heredity modes without claiming that useful structure evolved.

use std::collections::BTreeSet;

use sim_core::control::BrainInheritance;
use sim_core::genome::{self, Activation, Gene, NeuronGene};
use sim_core::mutate::structural::StructuralMutationCounts;
use sim_core::spawn::{SpawnError, SpawnFailureCounts};
use sim_core::{AgentId, InnovationId, SimParams, SpawnSpec, World};

fn params() -> SimParams {
    let mut p = SimParams::default().without_structural_mutation();
    p.world.max_agents = 8;
    p.plants.max_plants = 8;
    p.reproduction.maturity_ticks = 0;
    p.mutation.weight_perturb_rate = 0.0;
    p.mutation.weight_reset_rate = 0.0;
    p.mutation.neuron_perturb_rate = 0.0;
    p
}

fn founder_genes(p: &SimParams) -> u32 {
    World::new(42, p.clone()).unwrap().founder_plan().len() as u32
}

fn ready(world: &mut World, parent: AgentId) {
    world.agents_mut().energy[parent.index()] = 300.0;
    world.intents_mut().reproduce[parent.index()] = 1.0;
}

fn assert_architecture(genes: &[Gene]) {
    genome::validate(genes).unwrap();
    let mut ids = BTreeSet::new();
    let mut pairs = BTreeSet::new();
    for gene in genes {
        if let Some(id) = gene.innovation() {
            assert!(ids.insert(id));
        }
        if let Gene::Connection(c) = gene {
            assert!(pairs.insert((c.from, c.to)));
        }
    }
}

#[test]
fn splitting_reaches_real_births_and_cached_costs_in_both_modes() {
    for mode in [
        BrainInheritance::Evolving,
        BrainInheritance::RandomizedAtBirth,
    ] {
        let mut p = params();
        p.mutation.structural.add_neuron_rate = 1.0;
        let mut world = World::new_with_brain_inheritance(42, p, mode).unwrap();
        let parent = world.spawn_founder(Default::default()).unwrap();
        let original = world.genome(parent).to_vec();
        let units = world.agents().brain_units[parent.index()];
        ready(&mut world, parent);
        let mut edits = StructuralMutationCounts::default();
        assert_eq!(
            world.resolve_births_with_observers(
                |_| panic!("birth refused"),
                |event| edits.record(event)
            ),
            1
        );
        let child = world.pool().iter_live().find(|&id| id != parent).unwrap();
        assert_eq!(edits.add_neuron.applied, 1);
        assert_eq!(world.genome(parent), original);
        assert_eq!(world.genome(child).len(), original.len() + 3);
        assert_eq!(world.brain(child).len(), world.brain(parent).len() + 1);
        assert_eq!(world.agents().brain_units[child.index()], units + 3);
        assert_architecture(world.genome(child));
        world.step();
        assert!(world.brain(child).iter().all(|n| n.output.is_finite()));
    }
}

#[test]
fn structural_edits_receive_coherent_genes_after_extreme_scalar_mutation() {
    let mut p = params();
    p.mutation.neuron_perturb_rate = 1.0;
    p.mutation.bias_perturb_sigma = f32::MAX;
    p.mutation.structural.toggle_connection_rate = 1.0;
    let mut world = World::new(42, p).unwrap();
    let parent = world.spawn_founder(Default::default()).unwrap();
    ready(&mut world, parent);
    assert_eq!(world.resolve_births(), 1);
    let child = world.pool().iter_live().find(|&id| id != parent).unwrap();
    assert_architecture(world.genome(child));
}

#[test]
fn deletion_reduces_offspring_storage_without_touching_the_parent() {
    let mut p = params();
    // Founder neurons are all protected; hidden ones give removal a candidate.
    p.brain.hidden_neurons = 2;
    p.mutation.structural.remove_neuron_rate = 1.0;
    let mut world = World::new(42, p).unwrap();
    let parent = world.spawn_founder(Default::default()).unwrap();
    let original = world.genome(parent).to_vec();
    ready(&mut world, parent);
    assert_eq!(world.resolve_births(), 1);
    let child = world.pool().iter_live().find(|&id| id != parent).unwrap();
    assert_eq!(world.genome(parent), original);
    assert_eq!(world.brain(child).len() + 1, world.brain(parent).len());
    assert!(world.genome(child).len() < original.len());
    assert!(world.agents().brain_units[child.index()] < world.agents().brain_units[parent.index()]);
    assert_architecture(world.genome(child));
}

#[test]
fn a_declined_edit_can_still_produce_a_bounded_child() {
    let mut p = params();
    // No room to grow past the founder.
    p.storage.max_genes = founder_genes(&p);
    p.mutation.structural.add_neuron_rate = 1.0;
    let mut world = World::new(42, p).unwrap();
    let parent = world.spawn_founder(Default::default()).unwrap();
    let original = world.genome(parent).to_vec();
    let next = world.next_innovation().unwrap().raw();
    ready(&mut world, parent);
    let mut edits = StructuralMutationCounts::default();
    assert_eq!(
        world.resolve_births_with_observers(
            |_| panic!("birth refused"),
            |event| edits.record(event)
        ),
        1
    );
    let child = world.pool().iter_live().find(|&id| id != parent).unwrap();
    assert_eq!(edits.add_neuron.genome_limit, 1);
    assert_eq!(edits.add_neuron.applied, 0);
    assert_eq!(world.genome(child), original);
    assert_eq!(world.next_innovation().unwrap().raw(), next + 1);
}

#[test]
fn an_edited_candidate_can_be_refused_without_rewinding_ids_or_charging_energy() {
    let mut p = params();
    p.world.max_agents = 2;
    // The pooled arena fits the parent but not a grown child.
    p.storage.genes_per_slot = founder_genes(&p).div_ceil(2);
    p.mutation.structural.add_neuron_rate = 1.0;
    let mut world = World::new(42, p).unwrap();
    let parent = world.spawn_founder(Default::default()).unwrap();
    let next = world.next_innovation().unwrap().raw();
    ready(&mut world, parent);
    let usage = world.storage_usage();
    let incarnations = world.pool().incarnations().to_vec();
    let mut spawns = SpawnFailureCounts::default();
    let mut edits = StructuralMutationCounts::default();
    assert_eq!(
        world.resolve_births_with_observers(
            |error| spawns.record(error),
            |event| edits.record(event)
        ),
        0
    );
    assert_eq!(edits.add_neuron.applied, 1);
    assert_eq!(spawns.arena_capacity, 1);
    assert_eq!(world.population(), 1);
    assert_eq!(world.agents().energy[parent.index()], 300.0);
    assert_eq!(world.storage_usage(), usage);
    assert_eq!(world.pool().incarnations(), incarnations);
    assert_eq!(world.next_innovation().unwrap().raw(), next + 4);
}

fn single_neuron(id: u32) -> Vec<Gene> {
    vec![Gene::Neuron(NeuronGene {
        id: InnovationId::new(id),
        bias: 0.0,
        tau: 1.0,
        activation: Activation::Sigmoid,
        period: 0.0,
    })]
}

fn empty_spec() -> SpawnSpec {
    SpawnSpec {
        position: Default::default(),
        yaw: 0.0,
        energy: 0.0,
        size: 1.0,
        signature: [0.5; 3].into(),
        parent_a: AgentId::NULL,
    }
}

#[test]
fn imported_ids_advance_the_counter_only_after_a_successful_spawn() {
    let mut p = params();
    p.world.max_agents = 1;
    let mut world = World::new(42, p).unwrap();
    let id = world.spawn(&empty_spec(), &single_neuron(10_000)).unwrap();
    assert_eq!(world.next_innovation().unwrap().raw(), 10_001);
    assert_eq!(
        world.spawn(&empty_spec(), &single_neuron(20_000)),
        Err(SpawnError::PoolFull)
    );
    assert_eq!(world.next_innovation().unwrap().raw(), 10_002);
    world.despawn(id);
    world
        .spawn(&empty_spec(), &single_neuron(u32::MAX - 1))
        .unwrap();
    assert!(world.next_innovation().is_err());
}

#[test]
fn disabling_edits_keeps_retained_topology_redraw_finite_for_wide_intervals() {
    let mut p = params();
    p.mutation.structural.add_neuron_rate = 1.0;
    let mut world =
        World::new_with_brain_inheritance(42, p.clone(), BrainInheritance::RandomizedAtBirth)
            .unwrap();
    let parent = world.spawn_founder(Default::default()).unwrap();
    ready(&mut world, parent);
    assert_eq!(world.resolve_births(), 1);
    let child = world.pool().iter_live().find(|&id| id != parent).unwrap();
    assert_eq!(world.brain(child).len(), world.brain(parent).len() + 1);
    p.mutation.structural.add_neuron_rate = 0.0;
    world.set_params(p.clone()).unwrap();
    p.brain.weight_init_scale = 3e38;
    world.set_params(p).unwrap();
    world.intents_mut().reproduce[parent.index()] = 0.0;
    ready(&mut world, child);
    assert_eq!(world.resolve_births(), 1);
    let grandchild = world
        .pool()
        .iter_live()
        .find(|&id| id != parent && id != child)
        .unwrap();
    assert_architecture(world.genome(grandchild));
    world.step();
    assert!(world.brain(grandchild).iter().all(|n| n.output.is_finite()));
}

#[test]
fn runtime_admission_rejects_cross_kind_ids_and_parallel_edges() {
    let mut world = World::new(42, params()).unwrap();
    let parent = world.spawn_founder(Default::default()).unwrap();
    let mut duplicate = world.genome(parent).to_vec();
    let connection = duplicate
        .iter()
        .find_map(|g| match g {
            Gene::Connection(c) => Some(*c),
            _ => None,
        })
        .unwrap();
    let mut another = connection;
    another.id = InnovationId::new(10_000);
    another.enabled = false;
    duplicate.push(Gene::Connection(another));
    duplicate.sort_unstable_by_key(Gene::sort_key);
    genome::validate(&duplicate).unwrap();
    assert_eq!(
        world.spawn(&empty_spec(), &duplicate),
        Err(SpawnError::InvalidGenome(
            genome::GenomeError::DuplicateConnection
        ))
    );

    let mut duplicate = world.genome(parent).to_vec();
    for gene in &mut duplicate {
        if let Gene::Sensor(sensor) = gene {
            sensor.id = InnovationId::new(0);
            break;
        }
    }
    duplicate.sort_unstable_by_key(Gene::sort_key);
    genome::validate(&duplicate).unwrap();
    assert_eq!(
        world.spawn(&empty_spec(), &duplicate),
        Err(SpawnError::InvalidGenome(
            genome::GenomeError::DuplicateInnovation
        ))
    );
}
