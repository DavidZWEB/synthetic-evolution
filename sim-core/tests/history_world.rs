//! Species origins are captured at admission without changing World trajectories.
//!
//! Exercises identity/parent availability and observer pressure, not archive I/O.

use glam::Vec3;
use sim_core::command::{Command, Kind};
use sim_core::control::BrainInheritance;
use sim_core::history::{Event, EventKind, Parent, Recorder};
use sim_core::ids::{AgentId, BirthId, SpeciesId};
use sim_core::{SimParams, SpawnSpec, World};

#[allow(dead_code)]
#[path = "common/species_case.rs"]
mod common;

fn params() -> SimParams {
    let mut params = SimParams::default();
    params.world.max_agents = 8;
    params.plants.max_plants = 0;
    params.species.capacity = 2;
    params.distance.weight_coefficient = 1.0;
    params.reproduction.maturity_ticks = 0;
    params.mutation.weight_reset_rate = 0.0;
    params.mutation.weight_perturb_rate = 0.0;
    params.mutation.neuron_perturb_rate = 0.0;
    params
}

fn spec(parent: AgentId) -> SpawnSpec {
    SpawnSpec {
        position: Vec3::ZERO,
        yaw: 0.0,
        energy: 0.0,
        size: 3.0,
        signature: Vec3::ONE,
        parent_a: parent,
    }
}

fn spawn(world: &mut World, weight: f32, parent: AgentId, events: &mut Vec<Event>) -> AgentId {
    world
        .spawn_with_history_observer(
            &spec(parent),
            &common::genome(weight),
            |_| {},
            |event| events.push(event),
        )
        .unwrap()
}

#[test]
fn origins_capture_parent_ids_and_species_before_reuse_and_extinction_is_once() {
    let mut world = World::new(42, params()).unwrap();
    let mut events = Vec::new();
    let parent = spawn(&mut world, 0.0, AgentId::NULL, &mut events);
    let child = spawn(&mut world, 2.0, parent, &mut events);
    assert_eq!(
        events[0],
        Event {
            tick: 0,
            kind: EventKind::SpeciesOrigin {
                species_id: SpeciesId::new(0),
                founder_birth_id: BirthId::new(0),
                parent_a: Parent::Absent,
                parent_b: Parent::Absent,
            }
        }
    );
    let expected = Event {
        tick: 0,
        kind: EventKind::SpeciesOrigin {
            species_id: SpeciesId::new(1),
            founder_birth_id: BirthId::new(1),
            parent_a: Parent::Observed {
                birth_id: BirthId::new(0),
                species_id: Some(SpeciesId::new(0)),
            },
            parent_b: Parent::Absent,
        },
    };
    assert_eq!(events[1], expected);
    world.advance_tick();
    assert!(world.despawn_with_history_observer(parent, |_| {}, |e| events.push(e)));
    let count = events.len();
    assert!(!world.despawn_with_history_observer(parent, |_| {}, |e| events.push(e)));
    assert_eq!(events.len(), count);
    assert_eq!(
        events[2],
        Event {
            tick: 1,
            kind: EventKind::SpeciesExtinct {
                species_id: SpeciesId::new(0)
            }
        }
    );
    let replacement = spawn(&mut world, 0.0, AgentId::NULL, &mut events);
    assert_eq!(replacement, parent);
    assert_eq!(
        world.agents().birth_id[replacement.index()],
        BirthId::new(2)
    );
    assert_eq!(
        events[1], expected,
        "retained origin cannot depend on the reused live slot"
    );
    assert_eq!(
        world.agents().parent_birth_a[child.index()],
        BirthId::new(0)
    );
}

#[test]
fn observed_unclassified_parents_are_not_founders_or_missing_parents() {
    let mut params = params();
    params.species.capacity = 1;
    let mut world = World::new(42, params).unwrap();
    let mut events = Vec::new();
    let first = spawn(&mut world, 0.0, AgentId::NULL, &mut events);
    let parent = spawn(&mut world, 2.0, AgentId::NULL, &mut events);
    assert_eq!(
        events.len(),
        1,
        "unclassified admissions do not invent species origins"
    );
    world.despawn(first);
    world.agents_mut().energy[parent.index()] = 300.0;
    world.intents_mut().reproduce[parent.index()] = 1.0;
    assert_eq!(
        world.resolve_births_with_history_observer(|_| {}, |_| {}, |_| {}, |e| events.push(e)),
        1
    );
    assert_eq!(
        events[1].kind,
        EventKind::SpeciesOrigin {
            species_id: SpeciesId::new(1),
            founder_birth_id: BirthId::new(2),
            parent_a: Parent::Observed {
                birth_id: BirthId::new(1),
                species_id: None
            },
            parent_b: Parent::Absent,
        }
    );
}

#[test]
fn dead_parent_declarations_remain_unavailable_even_when_the_child_reuses_that_slot() {
    let mut world = World::new(42, params()).unwrap();
    let mut events = Vec::new();
    let parent = spawn(&mut world, 0.0, AgentId::NULL, &mut events);
    world.despawn(parent);
    let child = spawn(&mut world, 2.0, parent, &mut events);
    assert_eq!(child, parent);
    assert!(matches!(
        events.last().unwrap().kind,
        EventKind::SpeciesOrigin {
            parent_a: Parent::Unavailable,
            parent_b: Parent::Absent,
            ..
        }
    ));
}

#[test]
fn existing_members_and_refused_spawns_emit_no_extra_origins() {
    let mut params = params();
    params.world.max_agents = 2;
    let mut world = World::new(42, params).unwrap();
    let mut events = Vec::new();
    let parent = spawn(&mut world, 0.0, AgentId::NULL, &mut events);
    let child = spawn(&mut world, 0.25, parent, &mut events);
    assert_eq!(events.len(), 1);
    assert!(
        world
            .spawn_founder_with_history_observer(Vec3::ZERO, |_| {}, |e| events.push(e))
            .is_err()
    );
    assert_eq!(events.len(), 1);
    world.despawn_with_history_observer(parent, |_| {}, |e| events.push(e));
    assert_eq!(
        events.len(),
        1,
        "representative member death is not species extinction"
    );
    world.despawn_with_history_observer(child, |_| {}, |e| events.push(e));
    assert_eq!(events.len(), 2);
}

#[test]
fn command_origins_use_the_tick_the_command_is_applied() {
    let mut world = World::new(42, params()).unwrap();
    world.push_command(Command::at(
        1,
        Kind::SpawnFounder {
            position: Vec3::ZERO,
        },
    ));
    let mut events = Vec::new();
    world.step_with_history_observer(|_| {}, |_| {}, |_| {}, |e| events.push(e));
    assert!(events.is_empty());
    world.step_with_history_observer(|_| {}, |_| {}, |_| {}, |e| events.push(e));
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].tick, 1);
}

#[test]
fn capture_and_overflow_leave_complete_world_state_unchanged_across_seeds_and_modes() {
    for seed in [7, 42, 99] {
        for mode in [
            BrainInheritance::Evolving,
            BrainInheritance::RandomizedAtBirth,
        ] {
            let mut params = SimParams::default();
            params.world.size = 100.0;
            params.world.max_agents = 16;
            params.world.founder_spread = 0.2;
            params.plants.max_plants = 100;
            params.sensing.vision_range = 20.0;
            params.sensing.chemo_radius = 20.0;
            params.chemo.cells = [8, 8, 1];
            params.feeding.rate = 100.0;
            params.feeding.reach = 20.0;
            params.feeding.gate = 0.0;
            params.reproduction.start_energy = 1.0;
            params.reproduction.threshold = 1.1;
            params.reproduction.maturity_ticks = 0;
            params.reproduction.gate = 0.0;
            params.species.threshold = 1e-12;
            let mut plain = World::new_with_brain_inheritance(seed, params.clone(), mode).unwrap();
            let mut observed = World::new_with_brain_inheritance(seed, params, mode).unwrap();
            let mut recorder = Recorder::try_new(1).unwrap();
            plain.seed_founders(4);
            observed.seed_founders_with_history_observer(
                4,
                |_| {},
                |_| {},
                |e| {
                    recorder.record(e).unwrap();
                },
            );
            assert_eq!(plain.state_hash(), observed.state_hash());
            for tick in 0..20 {
                plain.step();
                observed.step_with_history_observer(
                    |_| {},
                    |_| {},
                    |_| {},
                    |e| {
                        recorder.record(e).unwrap();
                    },
                );
                assert_eq!(
                    plain.state_hash(),
                    observed.state_hash(),
                    "{seed}/{mode:?}/{tick}"
                );
                if tick % 7 == 0 {
                    while recorder.pop().is_some() {}
                }
            }
            assert!(plain.living_descendants() > 0);
            assert!(recorder.dropped_events() > 0);
            while recorder.pop().is_some() {}
            assert_eq!(plain.state_hash(), observed.state_hash());
        }
    }
}
