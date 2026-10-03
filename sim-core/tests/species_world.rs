//! Classification through actual World admission, commands, and death/birth ordering.
//!
//! Controlled genomes and feeding exercise bookkeeping, not adaptive speciation.

use glam::Vec3;
use sim_core::command::{Command, Kind};
use sim_core::genome::{Action, EffectorGene, Gene};
use sim_core::ids::{AgentId, InnovationId, NULL_ID, SpeciesId};
use sim_core::spawn::SpawnError;
use sim_core::species::{SpeciesEvent, SpeciesEventCounts, Unclassified};
use sim_core::{SimParams, SpawnSpec, World};

#[allow(dead_code)]
#[path = "common/species_case.rs"]
mod common;

fn params() -> SimParams {
    let mut params = SimParams::default().without_structural_mutation();
    params.world.max_agents = 8;
    params.plants.max_plants = 0;
    params.species.capacity = 2;
    params.distance.weight_coefficient = 1.0;
    params
}

fn spawn(world: &mut World, weight: f32, events: &mut Vec<SpeciesEvent>) -> AgentId {
    world
        .spawn_with_species_observer(
            &SpawnSpec {
                position: Vec3::ZERO,
                yaw: 0.0,
                energy: 0.0,
                size: 3.0,
                signature: Vec3::ONE,
                parent_a: AgentId::NULL,
            },
            &common::genome(weight),
            |event| events.push(event),
        )
        .unwrap()
}

fn assert_population_accounting(world: &World) {
    let populations: Vec<_> = world.species().active().collect();
    assert_eq!(world.species_count() as usize, populations.len());
    let mut unclassified = 0;
    for id in world.pool().iter_live() {
        let species = world.agents().species_id[id.index()];
        if species == NULL_ID {
            unclassified += 1;
        } else {
            assert!(populations.iter().any(|(known, _)| known.raw() == species));
        }
    }
    assert_eq!(unclassified, world.unclassified_population());
    for (species, count) in &populations {
        assert_eq!(
            *count,
            world
                .pool()
                .iter_live()
                .filter(|id| { world.agents().species_id[id.index()] == species.raw() })
                .count() as u32
        );
    }
    assert_eq!(
        populations.iter().map(|(_, count)| count).sum::<u32>() + unclassified,
        world.population()
    );
}

#[test]
fn unclassified_admissions_survive_capacity_pressure_and_are_not_retroactively_relabelled() {
    let mut params = params();
    params.species.capacity = 1;
    let mut world = World::new(42, params).unwrap();
    let mut events = Vec::new();
    let first = spawn(&mut world, 0.0, &mut events);
    let second = spawn(&mut world, 2.0, &mut events);
    assert_eq!(world.population(), 2);
    assert_eq!(
        events,
        [
            SpeciesEvent::Created(SpeciesId::new(0)),
            SpeciesEvent::Unclassified(Unclassified::Capacity)
        ]
    );
    assert_eq!(world.agents().species_id[second.index()], NULL_ID);
    assert_population_accounting(&world);
    assert!(world.despawn_with_species_observer(first, |event| events.push(event)));
    let third = spawn(&mut world, 2.0, &mut events);
    assert_eq!(
        third, first,
        "pool slot should be reused, not the species ID"
    );
    assert_eq!(world.agents().species_id[third.index()], 1);
    assert_eq!(world.agents().species_id[second.index()], NULL_ID);
    assert_population_accounting(&world);
    let before = events.len();
    assert!(world.despawn_with_species_observer(second, |event| events.push(event)));
    assert!(!world.despawn_with_species_observer(second, |event| events.push(event)));
    assert_eq!(
        events.len(),
        before,
        "unclassified death must not invent extinction"
    );
    assert_population_accounting(&world);
}

#[test]
fn descendants_of_unclassified_parents_are_independently_classified() {
    let mut params = params();
    params.species.capacity = 1;
    params.reproduction.maturity_ticks = 0;
    params.mutation.weight_reset_rate = 0.0;
    params.mutation.weight_perturb_rate = 0.0;
    params.mutation.neuron_perturb_rate = 0.0;
    let mut world = World::new(42, params).unwrap();
    let mut events = Vec::new();
    let first = spawn(&mut world, 0.0, &mut events);
    let parent = spawn(&mut world, 2.0, &mut events);
    world.despawn(first);
    world.agents_mut().energy[parent.index()] = 300.0;
    world.intents_mut().reproduce[parent.index()] = 1.0;
    assert_eq!(
        world.resolve_births_with_all_observers(|_| {}, |_| {}, |e| events.push(e)),
        1
    );
    let child = world.pool().iter_live().find(|&id| id != parent).unwrap();
    assert_eq!(world.agents().species_id[parent.index()], NULL_ID);
    assert_eq!(world.agents().species_id[child.index()], 1);
    assert_eq!(world.agents().energy[parent.index()], 150.0);
    assert_eq!(world.agents().energy[child.index()], 150.0);
    assert_population_accounting(&world);
}

#[test]
fn refused_ecological_admissions_emit_no_classification_event() {
    let mut params = params();
    params.world.max_agents = 1;
    let mut world = World::new(42, params).unwrap();
    let mut events = Vec::new();
    let first = spawn(&mut world, 0.0, &mut events);
    let original = events.clone();
    assert_eq!(
        world.spawn_founder_with_species_observer(Vec3::ZERO, |e| events.push(e)),
        Err(SpawnError::PoolFull)
    );
    assert_eq!(events, original);
    assert_eq!(
        world.species().active().collect::<Vec<_>>(),
        vec![(SpeciesId::new(0), 1)]
    );
    assert_eq!(world.agents().species_id[first.index()], 0);
}

#[test]
fn disabled_classification_is_visible_for_seeds_commands_and_deaths() {
    let mut params = params();
    params.species.capacity = 0;
    let mut world = World::new(42, params).unwrap();
    let mut events = SpeciesEventCounts::default();
    assert_eq!(
        world.seed_founders_with_observers(2, |_| {}, |e| events.record(e)),
        2
    );
    assert_eq!(events.unclassified_capacity, 2);
    world.push_command(Command::now(Kind::SpawnFounder {
        position: Vec3::ZERO,
    }));
    world.step_with_all_observers(|_| {}, |_| {}, |e| events.record(e));
    assert_eq!(events.unclassified_capacity, 3);
    assert_eq!(events.created, 0);
    assert_population_accounting(&world);
    let id = world.pool().iter_live().next().unwrap();
    world.despawn_with_species_observer(id, |e| events.record(e));
    assert_eq!(events.extinct, 0);
    assert_population_accounting(&world);
}

#[test]
fn last_death_precedes_same_tick_recolonization_with_a_new_id() {
    let mut params = params();
    params.plants.max_plants = 1;
    params.plants.max_energy = 600.0;
    params.feeding.rate = 300.0;
    params.reproduction.gate = 0.0;
    params.reproduction.maturity_ticks = 0;
    params.mutation.weight_reset_rate = 1.0;
    params.mutation.weight_limit = 0.0;
    params.mutation.neuron_perturb_rate = 0.0;
    let mut world = World::new(42, params).unwrap();
    let make_genes = |weight| {
        let mut genes = common::genome(weight).to_vec();
        genes.push(Gene::Effector(EffectorGene {
            id: InnovationId::new(3),
            action: Action::Reproduce,
            source: InnovationId::new(0),
            ..Default::default()
        }));
        genes.sort_by_key(Gene::sort_key);
        genes
    };
    let spec = SpawnSpec {
        position: world.plants().position()[0],
        yaw: 0.0,
        energy: 0.0,
        size: 3.0,
        signature: Vec3::ONE,
        parent_a: AgentId::NULL,
    };
    let dying = world.spawn(&spec, &make_genes(0.0)).unwrap();
    let parent = world.spawn(&spec, &make_genes(2.0)).unwrap();
    world.intents_mut().ingest[parent.index()] = 1.0;
    world.resolve_feeding();
    assert_eq!(world.agents().energy[parent.index()], 300.0);
    let mut events = Vec::new();
    world.step_with_all_observers(|_| {}, |_| {}, |e| events.push(e));
    assert_eq!(
        events,
        [
            SpeciesEvent::Extinct(SpeciesId::new(0)),
            SpeciesEvent::Created(SpeciesId::new(2))
        ]
    );
    assert_eq!(world.agents().species_id[dying.index()], 2);
    assert_eq!(world.agents().parent_a[dying.index()], parent.raw());
    assert_population_accounting(&world);
    assert!((world.total_energy() - world.ledger().expected_stock()).abs() < 1e-6);
}
