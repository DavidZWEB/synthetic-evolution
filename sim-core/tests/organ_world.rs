//! Sensor evolution through world admission, perception, and reproduction.
//!
//! Hand-driven births establish reachability and lifecycle contracts, not the viability
//! or ecological usefulness of a minimal founder.

use sim_core::control::BrainInheritance;
use sim_core::genome::{self, Gene, Modality};
use sim_core::mutate::StructuralMutationCounts;
use sim_core::spawn::SpawnError;
use sim_core::{AgentId, SimParams, SpawnSpec, World};

fn sparse_params() -> SimParams {
    let mut p = SimParams::default().with_dense_founder();
    p.world.max_agents = 8;
    p.plants.max_plants = 0;
    p.sensing.vision_rays = 0;
    p.sensing.chemo_sensors = 1;
    p.sensing.energy_sensors = 0;
    p.brain.hidden_neurons = 0;
    p.brain.oscillators = 0;
    p.brain.connections_per_target = Some(1);
    p.reproduction.maturity_ticks = 0;
    p.mutation.weight_perturb_rate = 0.0;
    p.mutation.weight_reset_rate = 0.0;
    p.mutation.neuron_perturb_rate = 0.0;
    p
}

fn ready(world: &mut World, parent: AgentId) {
    world.agents_mut().energy[parent.index()] = 300.0;
    world.intents_mut().reproduce[parent.index()] = 1.0;
}

fn child_of(world: &World, parent: AgentId) -> AgentId {
    world.pool().iter_live().find(|&id| id != parent).unwrap()
}

#[test]
fn a_chemo_led_sparse_founder_has_the_declared_small_controller() {
    let mut world = World::new(42, sparse_params()).unwrap();
    assert_eq!(world.seed_founders(8), 8);
    for id in world.pool().iter_live() {
        assert_eq!(world.brain(id).len(), 7);
        assert_eq!(world.wiring(id).len(), 4);
        assert_eq!(world.genome(id).len(), 23);
        assert_eq!(world.agents().sensor_load[id.index()], 3.0);
        assert_eq!(world.agents().brain_units[id.index()], 11);
        assert!(!world.genome(id).iter().any(|g| matches!(g,
            Gene::Sensor(s) if s.modality == Modality::VisionRay)));
    }
}

#[test]
fn founder_composition_and_connectivity_do_not_move_plant_geography() {
    let mut dense = SimParams::default().with_dense_founder();
    dense.world.max_agents = 8;
    dense.plants.max_plants = 64;
    let a = World::new(42, dense.clone()).unwrap();
    let mut sparse = sparse_params();
    sparse.plants.max_plants = dense.plants.max_plants;
    let b = World::new(42, sparse).unwrap();
    assert_eq!(a.plants().position(), b.plants().position());
}

#[test]
fn each_added_modality_reaches_perception_and_its_own_metabolic_cache() {
    for modality in [
        Modality::VisionRay,
        Modality::Chemo,
        Modality::Interoception,
    ] {
        let mut p = sparse_params();
        p.sensing.chemo_sensors = 0;
        p.brain.connections_per_target = Some(0);
        p.mutation.organs.add_sensor_rate = 1.0;
        p.mutation.organs.vision_weight = if modality == Modality::VisionRay {
            1.0
        } else {
            0.0
        };
        p.mutation.organs.chemo_weight = if modality == Modality::Chemo {
            1.0
        } else {
            0.0
        };
        p.mutation.organs.energy_weight = if modality == Modality::Interoception {
            1.0
        } else {
            0.0
        };
        let mut world = World::new(42, p).unwrap();
        let parent = world.spawn_founder(Default::default()).unwrap();
        ready(&mut world, parent);
        assert_eq!(world.resolve_births(), 1);
        let child = child_of(&world, parent);
        let sensor = world
            .genome(child)
            .iter()
            .find_map(|g| match g {
                Gene::Sensor(sensor) => Some(*sensor),
                _ => None,
            })
            .unwrap();
        assert_eq!(sensor.modality, modality);
        assert_eq!(world.brain(child).len(), 4 + modality.channels());
        assert_eq!(
            world.wiring(child).len(),
            0,
            "addition must not wire an organ for free"
        );
        assert_eq!(
            world.agents().brain_units[child.index()],
            4 + modality.channels() as u32
        );
        assert_eq!(
            world.agents().sensor_load[child.index()],
            modality.channels() as f32
        );
        assert_eq!(world.agents().sensor_load[parent.index()], 0.0);

        world.agents_mut().position[child.index()] = [500.0, 500.0, 0.0].into();
        world.agents_mut().orientation[child.index()] = sim_core::math::yaw_quat(0.0);
        match modality {
            Modality::VisionRay => {
                world.agents_mut().position[parent.index()] = [
                    500.0 + 10.0 * sim_core::math::cos(sensor.params[0]),
                    500.0 + 10.0 * sim_core::math::sin(sensor.params[0]),
                    0.0,
                ]
                .into();
                world.agents_mut().signature[parent.index()] = [0.2, 0.4, 0.8].into();
            }
            Modality::Chemo => world.deposit_chemo(0, [500.0, 500.0, 0.0].into(), 100.0),
            Modality::Interoception => {}
        }
        world.rebuild_spatial_hash();
        world.perceive_all();
        let slot = genome::neuron_index(world.genome(child), sensor.targets[0]).unwrap();
        assert!(
            world.brain(child)[slot].input > 0.0,
            "new organ did not query its world"
        );
        assert!(world.brain(parent).iter().all(|n| n.input == 0.0));
    }
}

#[test]
fn sensor_removal_precedes_neural_pruning_but_does_not_prune_by_itself() {
    for prune in [false, true] {
        let mut p = sparse_params();
        p.sensing.chemo_sensors = 0;
        p.sensing.energy_sensors = 1;
        p.mutation.organs.remove_sensor_rate = 1.0;
        p.mutation.structural.remove_neuron_rate = if prune { 1.0 } else { 0.0 };
        let mut world = World::new(42, p).unwrap();
        let parent = world.spawn_founder(Default::default()).unwrap();
        ready(&mut world, parent);
        let mut edits = StructuralMutationCounts::default();
        assert_eq!(
            world.resolve_births_with_observers(|_| panic!("refused"), |event| edits.record(event)),
            1
        );
        let child = child_of(&world, parent);
        assert_eq!(edits.remove_sensor.unwrap().applied, 1);
        assert_eq!(world.agents().sensor_load[child.index()], 0.0);
        assert_eq!(world.brain(child).len(), if prune { 4 } else { 5 });
        assert_eq!(world.wiring(child).len(), if prune { 0 } else { 4 });
        assert_eq!(world.brain(parent).len(), 5);
    }
}

#[test]
fn a_new_eye_can_gain_an_effector_connection_in_the_same_birth() {
    let mut connected = false;
    for seed in 0..64 {
        let mut p = sparse_params();
        p.sensing.chemo_sensors = 0;
        p.brain.connections_per_target = Some(0);
        p.mutation.organs.add_sensor_rate = 1.0;
        p.mutation.organs.chemo_weight = 0.0;
        p.mutation.organs.energy_weight = 0.0;
        p.mutation.structural.add_connection_rate = 1.0;
        let mut world = World::new(seed, p).unwrap();
        let parent = world.spawn_founder(Default::default()).unwrap();
        ready(&mut world, parent);
        assert_eq!(world.resolve_births(), 1);
        let genes = world.genome(child_of(&world, parent));
        let sensor = genes
            .iter()
            .find_map(|g| match g {
                Gene::Sensor(s) => Some(s),
                _ => None,
            })
            .unwrap();
        connected |= genes.iter().any(|gene| match gene {
            Gene::Connection(c) => {
                sensor.targets.contains(&c.from)
                    && genes
                        .iter()
                        .any(|g| matches!(g, Gene::Effector(e) if e.source == c.to))
            }
            _ => false,
        });
        if connected {
            break;
        }
    }
    assert!(
        connected,
        "sensor acquisition and useful wiring are not reachable together"
    );
}

#[test]
fn scalar_control_inherits_new_sensor_parameters_and_target_bindings() {
    let mut p = sparse_params();
    p.mutation.organs.add_sensor_rate = 1.0;
    p.mutation.organs.chemo_weight = 0.0;
    p.mutation.organs.energy_weight = 0.0;
    let mut world =
        World::new_with_brain_inheritance(42, p, BrainInheritance::RandomizedAtBirth).unwrap();
    let parent = world.spawn_founder(Default::default()).unwrap();
    ready(&mut world, parent);
    assert_eq!(world.resolve_births(), 1);
    let child = child_of(&world, parent);
    let eye = world
        .genome(child)
        .iter()
        .find_map(|g| match g {
            Gene::Sensor(s) if s.modality == Modality::VisionRay => Some(*s),
            _ => None,
        })
        .unwrap();
    assert_eq!(eye.params[1], 0.0);
    for target in &eye.targets[..4] {
        assert!(genome::neuron_index(world.genome(child), *target).is_some());
    }
    genome::validate(world.genome(child)).unwrap();
}

#[test]
fn unsupported_sensor_parameters_are_refused_before_resource_claims() {
    let mut p = SimParams::default().with_dense_founder();
    p.world.max_agents = 4;
    p.plants.max_plants = 0;
    let mut world = World::new(42, p).unwrap();
    let parent = world.spawn_founder(Default::default()).unwrap();
    let original = world.genome(parent).to_vec();
    let before = world.state_hash();
    let spec = SpawnSpec {
        position: Default::default(),
        yaw: 0.0,
        energy: 0.0,
        size: 3.0,
        signature: [0.5; 3].into(),
        parent_a: parent,
    };
    for (modality, slot, value) in [
        (Modality::VisionRay, 1, 1.0),
        (Modality::VisionRay, 2, -1.0),
        (
            Modality::VisionRay,
            2,
            world.spatial_hash().cell_size() + 1.0,
        ),
        (Modality::VisionRay, 3, -1.0),
        (Modality::Chemo, 0, -1.0),
        (Modality::Chemo, 0, 0.5),
        (Modality::Chemo, 0, 1.0),
        (Modality::Chemo, 1, -1.0),
        (Modality::Chemo, 1, world.spatial_hash().cell_size() + 1.0),
        (Modality::Interoception, 0, 1.0),
    ] {
        let mut genes = original.clone();
        for gene in &mut genes {
            if let Gene::Sensor(sensor) = gene
                && sensor.modality == modality
            {
                sensor.params[slot] = value;
                break;
            }
        }
        assert!(matches!(
            world.spawn(&spec, &genes),
            Err(SpawnError::SensorParameters(_))
        ));
        assert_eq!(world.state_hash(), before);
    }
}

#[test]
fn inherited_ranges_use_allocated_envelope_after_live_retuning() {
    let mut p = SimParams::default().with_dense_founder();
    p.world.max_agents = 4;
    p.plants.max_plants = 0;
    let mut world = World::new(42, p.clone()).unwrap();
    let parent = world.spawn_founder(Default::default()).unwrap();
    let genes = world.genome(parent).to_vec();
    p.sensing.vision_range = 20.0;
    p.sensing.chemo_radius = 20.0;
    world.set_params(p).unwrap();
    let spec = SpawnSpec {
        position: Default::default(),
        yaw: 0.0,
        energy: 0.0,
        size: 3.0,
        signature: [0.5; 3].into(),
        parent_a: parent,
    };
    assert!(world.spawn(&spec, &genes).is_ok());
    let mut invalid = genes.clone();
    for gene in &mut invalid {
        if let Gene::Sensor(s) = gene
            && s.modality == Modality::VisionRay
        {
            s.params[2] = world.spatial_hash().cell_size() + 1.0;
            break;
        }
    }
    assert!(matches!(
        world.spawn(&spec, &invalid),
        Err(SpawnError::SensorParameters(_))
    ));
}
