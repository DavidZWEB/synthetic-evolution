//! The structural null's birth contract (spec section 7.8): a child starts from a
//! living non-parent donor's topology with its parent's body, then redraws every
//! neural scalar. These tests pin the transfer, not its ecological consequences.

use glam::Vec3;
use sim_core::control::BrainInheritance;
use sim_core::genome::Gene;
use sim_core::ids::{AgentId, InnovationId};
use sim_core::{SimParams, SpawnSpec, World};

fn breeder_params() -> SimParams {
    let mut params = SimParams::default();
    params.world.max_agents = 8;
    params.reproduction.maturity_ticks = 0;
    params
}

/// Gene kind and innovation ID, in genome order: structure without scalars.
type Topology = Vec<(u8, Option<InnovationId>)>;

fn topology(genes: &[Gene]) -> Topology {
    genes
        .iter()
        .filter(|gene| !matches!(gene, Gene::Body(_) | Gene::Meta(_)))
        .map(|gene| (gene.sort_key().0, gene.innovation()))
        .collect()
}

fn body(genes: &[Gene]) -> Vec<Gene> {
    genes
        .iter()
        .copied()
        .filter(|gene| matches!(gene, Gene::Body(_) | Gene::Meta(_)))
        .collect()
}

fn breed(world: &mut World, parent: AgentId) -> AgentId {
    let before: Vec<_> = world.pool().iter_live().collect();
    world.agents_mut().energy[parent.index()] = 300.0;
    world.intents_mut().reproduce[parent.index()] = 1.0;
    assert_eq!(
        world.resolve_births_with_observer(|error| panic!("{error:?}")),
        1
    );
    world
        .pool()
        .iter_live()
        .find(|id| !before.contains(id))
        .unwrap()
}

#[test]
fn a_child_takes_the_donor_topology_and_the_parent_body() {
    let mut world =
        World::new_with_brain_inheritance(42, breeder_params(), BrainInheritance::StructuralNull)
            .unwrap();
    let parent = world.spawn_founder(Vec3::ZERO).unwrap();
    // A donor that differs from the parent in both topology and body.
    let mut donor_genes = world.genome(parent).to_vec();
    let last_connection = donor_genes
        .iter()
        .rposition(|gene| matches!(gene, Gene::Connection(_)))
        .unwrap();
    donor_genes.remove(last_connection);
    for gene in &mut donor_genes {
        if let Gene::Body(trait_gene) = gene {
            trait_gene.value += 0.25;
        }
    }
    let donor = world
        .spawn(
            &SpawnSpec {
                position: Vec3::ONE,
                yaw: 0.0,
                energy: 0.0,
                size: 3.0,
                signature: Vec3::ONE,
                parent_a: AgentId::NULL,
            },
            &donor_genes,
        )
        .unwrap();

    let child = breed(&mut world, parent);
    let (parent_genes, donor_genes, child_genes) = (
        world.genome(parent),
        world.genome(donor),
        world.genome(child),
    );
    assert_eq!(topology(child_genes), topology(donor_genes));
    assert_ne!(topology(child_genes), topology(parent_genes));
    assert_eq!(body(child_genes), body(parent_genes));
    let weights = |genes: &[Gene]| -> Vec<f32> {
        genes
            .iter()
            .filter_map(|gene| match gene {
                Gene::Connection(connection) => Some(connection.weight),
                _ => None,
            })
            .collect()
    };
    assert_ne!(
        weights(child_genes),
        weights(donor_genes),
        "the donor's neural scalars were inherited, not redrawn"
    );
}

#[test]
fn a_lone_parent_is_its_own_donor_and_the_draw_is_skipped() {
    let params = breeder_params();
    let mut null =
        World::new_with_brain_inheritance(42, params.clone(), BrainInheritance::StructuralNull)
            .unwrap();
    let mut scalar =
        World::new_with_brain_inheritance(42, params, BrainInheritance::RandomizedAtBirth).unwrap();
    let null_parent = null.spawn_founder(Vec3::ZERO).unwrap();
    let scalar_parent = scalar.spawn_founder(Vec3::ZERO).unwrap();
    let null_child = breed(&mut null, null_parent);
    let scalar_child = breed(&mut scalar, scalar_parent);
    // Without another living agent there is nothing to draw, so the null reduces to
    // the scalar control exactly, random stream included.
    assert_eq!(null.genome(null_child), scalar.genome(scalar_child));
    assert_eq!(
        null.rng_mut().state_fingerprint(),
        scalar.rng_mut().state_fingerprint()
    );
}

#[test]
fn donors_are_drawn_from_every_living_non_parent() {
    let mut world =
        World::new_with_brain_inheritance(42, breeder_params(), BrainInheritance::StructuralNull)
            .unwrap();
    let parent = world.spawn_founder(Vec3::ZERO).unwrap();
    let base = world.genome(parent).to_vec();
    // Each potential donor lacks a different connection, so a child's topology names
    // its donor.
    let connections: Vec<usize> = base
        .iter()
        .enumerate()
        .filter_map(|(index, gene)| matches!(gene, Gene::Connection(_)).then_some(index))
        .take(3)
        .collect();
    let donors: Vec<(AgentId, Topology)> = connections
        .iter()
        .map(|&index| {
            let mut genes = base.clone();
            genes.remove(index);
            let donor = world
                .spawn(
                    &SpawnSpec {
                        position: Vec3::ONE,
                        yaw: 0.0,
                        energy: 0.0,
                        size: 3.0,
                        signature: Vec3::ONE,
                        parent_a: AgentId::NULL,
                    },
                    &genes,
                )
                .unwrap();
            (donor, topology(&genes))
        })
        .collect();
    let mut seen = [0; 3];
    for _ in 0..60 {
        let child = breed(&mut world, parent);
        let found = topology(world.genome(child));
        let donor = donors
            .iter()
            .position(|(_, donor_topology)| *donor_topology == found)
            .expect("child topology matches no non-parent donor");
        seen[donor] += 1;
        assert!(world.despawn(child));
    }
    assert!(seen.iter().all(|&count| count > 0), "{seen:?}");
}

fn spawn_with(world: &mut World, genes: &[Gene], x: f32) -> AgentId {
    world
        .spawn(
            &SpawnSpec {
                position: Vec3::new(x, 0.0, 0.0),
                yaw: 0.0,
                energy: 0.0,
                size: 3.0,
                signature: Vec3::ONE,
                parent_a: AgentId::NULL,
            },
            genes,
        )
        .unwrap()
}

#[test]
fn a_v2_child_keeps_parent_scalars_on_shared_genes_and_the_donor_structure() {
    let mut params = breeder_params();
    // No mutation, so every scalar's origin is exact.
    params.mutation.weight_perturb_rate = 0.0;
    params.mutation.weight_reset_rate = 0.0;
    params.mutation.neuron_perturb_rate = 0.0;
    let mut world =
        World::new_with_brain_inheritance(42, params, BrainInheritance::StructuralNullV2).unwrap();
    let founder = world.spawn_founder(Vec3::ZERO).unwrap();
    let base = world.genome(founder).to_vec();
    assert!(world.despawn(founder));

    // The parent lacks one connection the donor has; the donor differs in every
    // neural scalar, one enable state, and its body.
    let connections: Vec<usize> = base
        .iter()
        .enumerate()
        .filter_map(|(index, gene)| matches!(gene, Gene::Connection(_)).then_some(index))
        .collect();
    let donor_only = match base[connections[0]] {
        Gene::Connection(connection) => connection.id,
        _ => unreachable!(),
    };
    let mut parent_genes = base.clone();
    parent_genes.remove(connections[0]);
    let mut donor_genes = base.clone();
    for gene in &mut donor_genes {
        match gene {
            Gene::Connection(connection) => connection.weight += 0.5,
            Gene::Neuron(neuron) => {
                neuron.bias += 0.25;
                neuron.tau += 1.0;
            }
            Gene::Body(trait_gene) => trait_gene.value += 0.25,
            _ => {}
        }
    }
    if let Gene::Connection(connection) = &mut donor_genes[connections[1]] {
        connection.enabled = !connection.enabled;
    }
    let parent = spawn_with(&mut world, &parent_genes, 0.0);
    let donor = spawn_with(&mut world, &donor_genes, 1.0);

    let child = breed(&mut world, parent);
    let (parent_genes, donor_genes, child_genes) = (
        world.genome(parent).to_vec(),
        world.genome(donor).to_vec(),
        world.genome(child).to_vec(),
    );
    assert_eq!(topology(&child_genes), topology(&donor_genes));
    assert_eq!(body(&child_genes), body(&parent_genes));
    let find = |genes: &[Gene], key: (u8, u32)| genes.iter().find(|g| g.sort_key() == key).copied();
    for gene in &child_genes {
        let from_parent = find(&parent_genes, gene.sort_key());
        let from_donor = find(&donor_genes, gene.sort_key()).unwrap();
        match (gene, from_parent, from_donor) {
            (Gene::Connection(c), _, Gene::Connection(d)) if c.id == donor_only => {
                assert_eq!(
                    c.weight, d.weight,
                    "a donor-only gene keeps the donor's weight"
                );
            }
            (Gene::Connection(c), Some(Gene::Connection(p)), Gene::Connection(d)) => {
                assert_eq!(c.weight, p.weight);
                assert_eq!(c.enabled, d.enabled, "enable state is structure");
            }
            (Gene::Neuron(c), Some(Gene::Neuron(p)), _) => {
                assert_eq!((c.bias, c.tau, c.period), (p.bias, p.tau, p.period));
            }
            (Gene::Sensor(_) | Gene::Effector(_), _, donor_gene) => {
                assert_eq!(
                    *gene, donor_gene,
                    "organs travel with the donor's structure"
                );
            }
            _ => {}
        }
    }
}

#[test]
fn a_lone_v2_parent_reduces_to_the_evolving_child() {
    let params = breeder_params();
    let mut null =
        World::new_with_brain_inheritance(42, params.clone(), BrainInheritance::StructuralNullV2)
            .unwrap();
    let mut evolving =
        World::new_with_brain_inheritance(42, params, BrainInheritance::Evolving).unwrap();
    let null_parent = null.spawn_founder(Vec3::ZERO).unwrap();
    let evolving_parent = evolving.spawn_founder(Vec3::ZERO).unwrap();
    let null_child = breed(&mut null, null_parent);
    let evolving_child = breed(&mut evolving, evolving_parent);
    assert_eq!(null.genome(null_child), evolving.genome(evolving_child));
    assert_eq!(
        null.rng_mut().state_fingerprint(),
        evolving.rng_mut().state_fingerprint()
    );
}
