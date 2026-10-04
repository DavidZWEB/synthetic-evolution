//! Configurable founders and sensor evolution through the actual browser boundary.
//!
//! These cover JSON validation, live retunes, diagnostics and variable inspector
//! payloads; core tests own operator mechanics and deterministic reference hashes.

#![cfg(target_arch = "wasm32")]

use sim_core::genome::{Action, Gene, Modality};
use sim_core::mutate::StructuralMutationCounts;
use sim_core::params::SimParams;
use wasm::{Sim, random_control, validate_params};
use wasm_bindgen_test::wasm_bindgen_test;

fn birth_params() -> SimParams {
    let mut params: SimParams =
        serde_json::from_str(include_str!("../../native/tests/fixtures/structural.json")).unwrap();
    params = params.without_structural_mutation();
    params
}

fn inspect(sim: &Sim, slot: u32) -> serde_json::Value {
    serde_json::from_str(&sim.inspect_agent(slot, 1).unwrap()).unwrap()
}

fn genome(sim: &Sim, slot: u32) -> Vec<Gene> {
    serde_json::from_value(inspect(sim, slot)["genome"].clone()).unwrap()
}

/// Sensors on this params' founder, measured rather than assumed.
fn founder_sensors(params: &SimParams) -> u32 {
    let mut probe = Sim::new(7, Some(serde_json::to_string(params).unwrap())).unwrap();
    probe.seed_founders(1);
    genome(&probe, 0)
        .iter()
        .filter(|gene| matches!(gene, Gene::Sensor(_)))
        .count() as u32
}

fn counts(sim: &Sim) -> StructuralMutationCounts {
    serde_json::from_str(&sim.structural_mutation_diagnostics().unwrap()).unwrap()
}

fn connections(genes: &[Gene]) -> Vec<(u32, u32, u32)> {
    genes
        .iter()
        .filter_map(|gene| match gene {
            Gene::Connection(connection) => Some((
                connection.id.raw(),
                connection.from.raw(),
                connection.to.raw(),
            )),
            _ => None,
        })
        .collect()
}

#[wasm_bindgen_test]
fn founder_shapes_round_trip_and_share_one_reproducible_template() {
    for (vision, chemo, energy, hidden, oscillators, fan_in) in [
        (3, 1, 1, 6, 2, None),
        (0, 1, 0, 0, 0, Some(1)),
        (0, 2, 1, 2, 1, Some(2)),
        (1, 0, 0, 0, 0, Some(0)),
        (0, 0, 0, 0, 0, None),
        (0, 1, 0, 0, 0, Some(u32::MAX)),
    ] {
        let mut params = birth_params();
        params.sensing.vision_rays = vision;
        params.sensing.chemo_sensors = chemo;
        params.sensing.energy_sensors = energy;
        params.brain.hidden_neurons = hidden;
        params.brain.oscillators = oscillators;
        params.brain.connections_per_target = fan_in;
        let json = serde_json::to_string(&params).unwrap();
        let canonical = validate_params(Some(json.clone())).unwrap();
        assert_eq!(
            serde_json::from_str::<SimParams>(&canonical).unwrap(),
            params
        );
        let mut sim = Sim::new(7, Some(json.clone())).unwrap();
        let mut replay = Sim::new(7, Some(json.clone())).unwrap();
        let mut control = random_control(7, Some(json)).unwrap();
        for sim in [&mut sim, &mut replay, &mut control] {
            assert_eq!(sim.seed_founders(2), 2);
        }
        assert_eq!(sim.state_hash(), replay.state_hash());
        assert_eq!(sim.state_hash(), control.state_hash());
        let sources = vision * 4 + chemo * 3 + energy + hidden + oscillators;
        let expected_neurons = sources + 4;
        let expected_edges = fan_in.unwrap_or(sources).min(sources) * (hidden + 4);
        let genes = genome(&sim, 0);
        sim_core::genome::validate(&genes).unwrap();
        assert_eq!(
            inspect(&sim, 0)["activations"].as_array().unwrap().len(),
            expected_neurons as usize
        );
        assert_eq!(
            genes
                .iter()
                .filter(|gene| matches!(gene, Gene::Neuron(_)))
                .count(),
            expected_neurons as usize
        );
        assert_eq!(connections(&genes).len(), expected_edges as usize);
        assert_eq!(connections(&genes), connections(&genome(&sim, 1)));
        for (modality, expected) in [
            (Modality::VisionRay, vision),
            (Modality::Chemo, chemo),
            (Modality::Interoception, energy),
        ] {
            assert_eq!(
                genes
                    .iter()
                    .filter(
                        |gene| matches!(gene, Gene::Sensor(sensor) if sensor.modality == modality)
                    )
                    .count(),
                expected as usize
            );
        }
        let mut endpoints = connections(&genes);
        endpoints.sort_by_key(|&(_, from, to)| (from, to));
        assert!(
            endpoints
                .windows(2)
                .all(|pair| (pair[0].1, pair[0].2) != (pair[1].1, pair[1].2))
        );
        for action in [
            Action::Thrust,
            Action::Turn,
            Action::Ingest,
            Action::Reproduce,
        ] {
            assert_eq!(
                genes
                    .iter()
                    .filter(|gene| matches!(gene, Gene::Effector(e) if e.action == action))
                    .count(),
                1
            );
        }
        // Size, three colour channels, and since Phase 3 muscle and mouth (spec §3.5).
        assert_eq!(
            genes
                .iter()
                .filter(|gene| matches!(gene, Gene::Body(_)))
                .count(),
            6
        );
        assert_eq!(
            genes
                .iter()
                .filter(|gene| matches!(gene, Gene::Meta(_)))
                .count(),
            3
        );
        sim.step_many(1);
        replay.step_many(1);
        assert_eq!(sim.state_hash(), replay.state_hash());
    }
}

#[wasm_bindgen_test]
fn founder_composition_is_frozen_but_bad_shapes_fail_before_world_replacement() {
    let params = serde_json::to_value(birth_params()).unwrap();
    let mut sim = Sim::new(7, Some(params.to_string())).unwrap();
    sim.seed_founders(1);
    let original_params = sim.params_json().unwrap();
    for path in [
        "/sensing/vision_rays",
        "/sensing/chemo_sensors",
        "/sensing/energy_sensors",
        "/brain/hidden_neurons",
        "/brain/oscillators",
        "/brain/connections_per_target",
    ] {
        let mut changed = params.clone();
        // Any different value; composition is frozen whatever it currently is.
        let field = changed.pointer_mut(path).unwrap();
        *field = match field.as_u64() {
            Some(0) => serde_json::json!(1),
            Some(_) => serde_json::json!(0),
            None => serde_json::json!(1),
        };
        validate_params(Some(changed.to_string())).expect("valid construction-time shape");
        let before = sim.state_hash();
        assert!(
            sim.set_params(&changed.to_string()).is_err(),
            "{path} retuned"
        );
        assert_eq!(sim.state_hash(), before);
        assert_eq!(sim.params_json().unwrap(), original_params);
    }
    for (path, invalid) in [
        ("/sensing/chemo_sensors", serde_json::json!(-1)),
        ("/sensing/energy_sensors", serde_json::json!(0.5)),
        ("/sensing/vision_rays", serde_json::json!(4294967295u64)),
        ("/sensing/chemo_sensors", serde_json::json!(4294967295u64)),
        ("/sensing/energy_sensors", serde_json::json!(4294967295u64)),
        ("/brain/connections_per_target", serde_json::json!(-1)),
        ("/brain/connections_per_target", serde_json::json!(0.5)),
        (
            "/brain/connections_per_target",
            serde_json::json!(4294967296u64),
        ),
        ("/storage/max_memory_bytes", serde_json::json!(1)),
        (
            "/storage/max_sensors",
            serde_json::json!(founder_sensors(&birth_params()) - 1),
        ),
        ("/storage/max_vision_rays", serde_json::json!(0)),
        ("/storage/max_neurons", serde_json::json!(4)),
        ("/storage/max_connections", serde_json::json!(0)),
        ("/storage/max_genes", serde_json::json!(10)),
    ] {
        let mut changed = params.clone();
        *changed.pointer_mut(path).unwrap() = invalid;
        assert!(
            validate_params(Some(changed.to_string())).is_err(),
            "{path}"
        );
        assert!(Sim::new(7, Some(changed.to_string())).is_err(), "{path}");
        assert!(
            random_control(7, Some(changed.to_string())).is_err(),
            "{path}"
        );
        let before = sim.state_hash();
        assert!(sim.set_params(&changed.to_string()).is_err(), "{path}");
        assert_eq!(sim.state_hash(), before);
    }
}

#[wasm_bindgen_test]
fn sensor_addition_has_fresh_unwired_targets_and_preserves_inherited_organs_in_both_modes() {
    for modality in [
        Modality::VisionRay,
        Modality::Chemo,
        Modality::Interoception,
    ] {
        let mut params = birth_params();
        params.mutation.organs.add_sensor_rate = 1.0;
        params.mutation.organs.vision_weight = if modality == Modality::VisionRay {
            1.0
        } else {
            0.0
        };
        params.mutation.organs.chemo_weight = if modality == Modality::Chemo {
            1.0
        } else {
            0.0
        };
        params.mutation.organs.energy_weight = if modality == Modality::Interoception {
            1.0
        } else {
            0.0
        };
        params.mutation.organs.neuron_bias = 0.375;
        let json = serde_json::to_string(&params).unwrap();
        let mut evolving = Sim::new(7, Some(json.clone())).unwrap();
        let mut control = random_control(7, Some(json)).unwrap();
        for sim in [&mut evolving, &mut control] {
            assert_eq!(sim.seed_founders(1), 1);
        }
        let parent = genome(&evolving, 0);
        assert_eq!(genome(&control, 0), parent);
        for (sim, is_control) in [(&mut evolving, false), (&mut control, true)] {
            sim.step_many(1);
            assert_eq!(sim.descendants(), 1);
            assert_eq!(counts(sim).add_sensor.unwrap().applied, 1);
            assert_eq!(counts(sim).remove_sensor.unwrap().attempted, 0);
            assert_eq!(genome(sim, 0), parent);
            let child = genome(sim, 1);
            sim_core::genome::validate(&child).unwrap();
            assert_eq!(child.len(), parent.len() + modality.channels() + 1);
            assert_eq!(
                connections(&child),
                connections(&parent),
                "addition auto-wired targets"
            );
            for gene in parent
                .iter()
                .filter(|gene| !matches!(gene, Gene::Neuron(_) | Gene::Connection(_)))
            {
                assert!(
                    child.contains(gene),
                    "birth changed an inherited non-neural gene"
                );
            }
            let sensor = child
                .iter()
                .find_map(|gene| match gene {
                    Gene::Sensor(sensor)
                        if !parent
                            .iter()
                            .any(|gene| gene.innovation() == Some(sensor.id)) =>
                    {
                        Some(sensor)
                    }
                    _ => None,
                })
                .unwrap();
            assert_eq!(sensor.modality, modality);
            match modality {
                Modality::VisionRay => {
                    assert!(
                        (-core::f32::consts::PI..=core::f32::consts::PI)
                            .contains(&sensor.params[0])
                    );
                    assert_eq!(
                        sensor.params[1..],
                        [0.0, params.sensing.vision_range, params.sensing.vision_fov]
                    );
                }
                Modality::Chemo => {
                    assert_eq!(sensor.params, [0.0, params.sensing.chemo_radius, 0.0, 0.0]);
                }
                Modality::Interoception => assert_eq!(sensor.params, [0.0; 4]),
            }
            for target in &sensor.targets[..modality.channels()] {
                assert!(!parent.iter().any(|gene| gene.innovation() == Some(*target)));
                let neuron = child
                    .iter()
                    .find_map(|gene| match gene {
                        Gene::Neuron(neuron) if neuron.id == *target => Some(neuron),
                        _ => None,
                    })
                    .unwrap();
                if is_control {
                    assert_ne!(neuron.bias, params.mutation.organs.neuron_bias);
                } else {
                    assert_eq!(neuron.bias, params.mutation.organs.neuron_bias);
                }
            }
            assert_eq!(
                inspect(sim, 1)["activations"].as_array().unwrap().len(),
                inspect(sim, 0)["activations"].as_array().unwrap().len() + modality.channels()
            );
        }
    }
}

#[wasm_bindgen_test]
fn sensor_removal_retains_neurons_and_connections_and_precedes_addition() {
    let mut params = birth_params();
    params.mutation.organs.remove_sensor_rate = 1.0;
    let mut sim = Sim::new(7, Some(serde_json::to_string(&params).unwrap())).unwrap();
    sim.seed_founders(1);
    let parent = genome(&sim, 0);
    sim.step_many(1);
    assert_eq!(sim.descendants(), 1);
    let child = genome(&sim, 1);
    assert_eq!(child.len(), parent.len() - 1);
    assert_eq!(counts(&sim).remove_sensor.unwrap().applied, 1);
    for gene in parent
        .iter()
        .filter(|gene| !matches!(gene, Gene::Sensor(_)))
    {
        assert!(child.contains(gene), "removal changed a non-sensor gene");
    }
    assert_eq!(
        inspect(&sim, 1)["activations"].as_array().unwrap().len(),
        inspect(&sim, 0)["activations"].as_array().unwrap().len()
    );

    // At the founder's own sensor count, so addition fits only after removal.
    let sensors = founder_sensors(&params);
    params.storage.max_sensors = sensors;
    params.mutation.organs.add_sensor_rate = 1.0;
    let mut sim = Sim::new(7, Some(serde_json::to_string(&params).unwrap())).unwrap();
    sim.seed_founders(1);
    sim.step_many(1);
    assert_eq!(counts(&sim).remove_sensor.unwrap().applied, 1);
    assert_eq!(
        counts(&sim).add_sensor.unwrap().applied,
        1,
        "addition ran before removal freed the sensor cap"
    );
    assert_eq!(
        genome(&sim, 1)
            .iter()
            .filter(|gene| matches!(gene, Gene::Sensor(_)))
            .count(),
        sensors as usize
    );
}

#[wasm_bindgen_test]
fn sensor_candidate_caps_and_arena_birth_refusals_are_distinct() {
    let mut params = birth_params();
    params.mutation.organs.add_sensor_rate = 1.0;
    let sensors = founder_sensors(&params);
    params.storage.max_sensors = sensors;
    let mut capped = Sim::new(7, Some(serde_json::to_string(&params).unwrap())).unwrap();
    capped.seed_founders(1);
    capped.step_many(1);
    assert_eq!(counts(&capped).add_sensor.unwrap().genome_limit, 1);
    assert_eq!(counts(&capped).add_sensor.unwrap().applied, 0);
    assert_eq!(capped.descendants(), 1);

    params.storage.max_sensors = 32;
    params.world.max_agents = 2;
    // Two slots pool room for the founder's sensors but not a grown child's too.
    params.storage.sensors_per_slot = sensors.div_ceil(2);
    let mut full = Sim::new(7, Some(serde_json::to_string(&params).unwrap())).unwrap();
    full.seed_founders(1);
    full.step_many(1);
    assert_eq!(counts(&full).add_sensor.unwrap().applied, 1);
    assert_eq!(full.descendants(), 0);
    let storage: serde_json::Value =
        serde_json::from_str(&full.storage_diagnostics().unwrap()).unwrap();
    assert_eq!(storage["spawn_failures"]["arena_capacity"], 1);
}

#[wasm_bindgen_test]
fn organ_params_validate_and_retune_without_resetting_optional_observations() {
    let mut params = serde_json::to_value(birth_params()).unwrap();
    let mut sim = Sim::new(7, Some(params.to_string())).unwrap();
    sim.seed_founders(1);
    let pristine = counts(&sim);
    assert_eq!(pristine.add_sensor.unwrap().attempted, 0);
    assert_eq!(pristine.remove_sensor.unwrap().attempted, 0);
    for (field, value) in [
        ("remove_sensor_rate", -0.1),
        ("remove_sensor_rate", 1.1),
        ("add_sensor_rate", -0.1),
        ("add_sensor_rate", 1.1),
        ("vision_weight", -1.0),
        ("chemo_weight", -1.0),
        ("energy_weight", -1.0),
        ("neuron_bias", 1e40),
        ("vision_weight", 1e40),
        ("chemo_weight", 1e40),
        ("energy_weight", 1e40),
    ] {
        let mut invalid = params.clone();
        invalid["mutation"]["organs"][field] = serde_json::json!(value);
        assert!(
            validate_params(Some(invalid.to_string())).is_err(),
            "{field}"
        );
        assert!(Sim::new(7, Some(invalid.to_string())).is_err(), "{field}");
        assert!(
            random_control(7, Some(invalid.to_string())).is_err(),
            "{field}"
        );
        let before = sim.state_hash();
        assert!(sim.set_params(&invalid.to_string()).is_err(), "{field}");
        assert_eq!(sim.state_hash(), before);
    }
    let mut weights = params.clone();
    for field in ["vision_weight", "chemo_weight", "energy_weight"] {
        weights["mutation"]["organs"][field] = 0.into();
    }
    validate_params(Some(weights.to_string())).expect("disabled addition needs no positive weight");
    sim.set_params(&weights.to_string()).unwrap();
    weights["mutation"]["organs"]["add_sensor_rate"] = 1.into();
    assert!(validate_params(Some(weights.to_string())).is_err());
    assert!(Sim::new(7, Some(weights.to_string())).is_err());
    assert!(random_control(7, Some(weights.to_string())).is_err());
    assert!(sim.set_params(&weights.to_string()).is_err());
    params["mutation"]["organs"]["add_sensor_rate"] = 1.into();
    params["mutation"]["organs"]["neuron_bias"] = 0.25.into();
    sim.set_params(&params.to_string()).unwrap();
    sim.step_many(1);
    let observed = counts(&sim);
    assert_eq!(observed.add_sensor.unwrap().applied, 1);
    params["mutation"]["organs"]["add_sensor_rate"] = 0.into();
    sim.set_params(&params.to_string()).unwrap();
    sim.step_many(1);
    assert_eq!(counts(&sim), observed);
    let mut fresh = Sim::new(7, Some(params.to_string())).unwrap();
    fresh.seed_founders(1);
    assert_eq!(counts(&fresh), pristine);
}
