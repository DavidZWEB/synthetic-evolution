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
