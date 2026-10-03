//! Real world classification through the WASM boundary and its on-demand diagnostics.
//!
//! These test admission, lifecycle observation, and parameter policy, not the core
//! distance algorithm or cross-world species identity.

#![cfg(target_arch = "wasm32")]

use serde::Deserialize;
use sim_core::NULL_ID;
use sim_core::params::SimParams;
use sim_core::species::SpeciesEventCounts;
use wasm::{Sim, random_control, validate_params};
use wasm_bindgen_test::wasm_bindgen_test;

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Population {
    species_id: u32,
    population: u32,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Diagnostics {
    populations: Vec<Population>,
    unclassified_population: u32,
    events: SpeciesEventCounts,
}

fn params() -> SimParams {
    serde_json::from_str(
        r#"{"world":{"max_agents":8},"plants":{"max_plants":8},
            "chemo":{"cells":[8,8,1]},"species":{"capacity":4,"threshold":1e-12}}"#,
    )
    .unwrap()
}

fn make(params: &SimParams, control: bool) -> Sim {
    let json = Some(serde_json::to_string(params).unwrap());
    if control {
        random_control(7, json).unwrap()
    } else {
        Sim::new(7, json).unwrap()
    }
}

fn diagnostics(sim: &Sim) -> Diagnostics {
    let observed: Diagnostics = serde_json::from_str(&sim.species_diagnostics().unwrap()).unwrap();
    assert_eq!(observed.populations.len() as u32, sim.species_count());
    assert_eq!(
        observed.unclassified_population,
        sim.unclassified_population()
    );
    assert_eq!(
        observed
            .populations
            .iter()
            .map(|p| p.population)
            .sum::<u32>()
            + observed.unclassified_population,
        sim.population()
    );
    assert!(
        observed
            .populations
            .iter()
            .all(|p| p.species_id != NULL_ID && p.population > 0)
    );
    assert!(
        observed
            .populations
            .windows(2)
            .all(|p| p[0].species_id < p[1].species_id)
    );
    observed
}

fn inspection(sim: &Sim, slot: u32, incarnation: u32) -> serde_json::Value {
    serde_json::from_str(&sim.inspect_agent(slot, incarnation).unwrap()).unwrap()
}

fn assert_no_spawn_refusals(sim: &Sim) {
    let storage: serde_json::Value =
        serde_json::from_str(&sim.storage_diagnostics().unwrap()).unwrap();
    assert!(
        storage["spawn_failures"]
            .as_object()
            .unwrap()
            .values()
            .all(|value| value == 0)
    );
}

#[wasm_bindgen_test]
fn species_params_are_canonical_validated_and_frozen_at_every_boundary() {
    let defaults: SimParams = serde_json::from_str(&validate_params(None).unwrap()).unwrap();
    assert_eq!(defaults.species.capacity, 256);
    assert_eq!(defaults.species.threshold, 0.5);
    let params = params();
    let canonical = validate_params(Some(serde_json::to_string(&params).unwrap())).unwrap();
    assert_eq!(
        serde_json::from_str::<SimParams>(&canonical)
            .unwrap()
            .species,
        params.species
    );
    // The classifier threshold is f64, not narrowed at the JS boundary.
    assert!(validate_params(Some(r#"{"species":{"threshold":1e40}}"#.into())).is_ok());

    for control in [false, true] {
        let mut sim = make(&params, control);
        sim.seed_founders(2);
        let original = sim.params_json().unwrap();
        let hash = sim.state_hash();
        let observed = diagnostics(&sim);
        for policy in [
            serde_json::json!({"capacity":0,"threshold":1e-12}),
            serde_json::json!({"capacity":4,"threshold":0.5}),
        ] {
            let mut changed = serde_json::to_value(&params).unwrap();
            changed["species"] = policy;
            assert!(validate_params(Some(changed.to_string())).is_ok());
            assert!(sim.set_params(&changed.to_string()).is_err());
        }
        for policy in [
            serde_json::json!({"threshold":0}),
            serde_json::json!({"threshold":-1}),
            serde_json::json!({"threshold":null}),
            serde_json::json!({"threshold":"NaN"}),
            serde_json::json!({"capacity":-1}),
            serde_json::json!({"capacity":0.5}),
            serde_json::json!({"capacity":4294967296_u64}),
            serde_json::json!({"unknown":1}),
        ] {
            let mut invalid = serde_json::to_value(&params).unwrap();
            invalid["species"] = policy;
            let json = invalid.to_string();
            assert!(validate_params(Some(json.clone())).is_err());
            assert!(Sim::new(7, Some(json.clone())).is_err());
            assert!(random_control(7, Some(json.clone())).is_err());
            assert!(sim.set_params(&json).is_err());
            assert_eq!(sim.params_json().unwrap(), original);
            assert_eq!(sim.state_hash(), hash);
            assert_eq!(diagnostics(&sim), observed);
        }
        sim.set_params(&original).unwrap();
    }
}

#[wasm_bindgen_test]
fn sparse_thresholds_create_root_species_and_recreation_resets_counters() {
    for control in [false, true] {
        let params = params();
        let mut sim = make(&params, control);
        assert_eq!(diagnostics(&sim).events, SpeciesEventCounts::default());
        assert_eq!(sim.seed_founders(4), 4);
        let observed = diagnostics(&sim);
        assert_eq!(
            observed.populations,
            (0..4)
                .map(|species_id| Population {
                    species_id,
                    population: 1
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(
            observed.events,
            SpeciesEventCounts {
                created: 4,
                ..Default::default()
            }
        );
        for slot in 0..4 {
            assert_eq!(inspection(&sim, slot, 1)["species_id"], slot);
        }
        let hash = sim.state_hash();
        sim.step_many(0);
        assert_eq!(diagnostics(&sim), observed);
        assert_eq!(sim.state_hash(), hash);
        let mut recreated = make(&params, control);
        assert_eq!(
            diagnostics(&recreated).events,
            SpeciesEventCounts::default()
        );
        assert_eq!(recreated.seed_founders(4), 4);
        assert_eq!(diagnostics(&recreated), observed);
        assert_eq!(
            diagnostics(&sim),
            observed,
            "another world's seeding changed counters"
        );
    }
}

#[wasm_bindgen_test]
fn zero_capacity_admits_live_unclassified_founders_and_due_commands() {
    for control in [false, true] {
        let mut params = params();
        params.species.capacity = 0;
        let mut sim = make(&params, control);
        assert_eq!(sim.seed_founders(2), 2);
        assert_eq!(sim.population(), 2);
        assert_eq!(sim.unclassified_population(), 2);
        assert_eq!(inspection(&sim, 0, 1)["species_id"], NULL_ID);
        assert_eq!(
            diagnostics(&sim).events,
            SpeciesEventCounts {
                unclassified_capacity: 2,
                ..Default::default()
            }
        );
        sim.push_command(r#"{"apply_at_tick":0,"kind":{"SpawnFounder":{"position":[500,500,0]}}}"#)
            .unwrap();
        sim.step_many(1);
        assert_eq!(sim.population(), 3);
        assert_eq!(sim.unclassified_population(), 3);
        assert_eq!(inspection(&sim, 2, 1)["species_id"], NULL_ID);
        assert_eq!(
            diagnostics(&sim).events,
            SpeciesEventCounts {
                unclassified_capacity: 3,
                ..Default::default()
            }
        );
        assert_no_spawn_refusals(&sim);
    }
}

#[wasm_bindgen_test]
fn full_representative_capacity_matches_known_species_but_does_not_refuse_incompatible_births() {
    for control in [false, true] {
        for (compatible, capacity) in [(true, 1), (false, 1), (false, 2)] {
            let mut params: SimParams =
                serde_json::from_str(include_str!("../../native/tests/fixtures/structural.json"))
                    .unwrap();
            params.species.capacity = capacity;
            params.species.threshold = 1e-12;
            params = params.without_structural_mutation();
            params.mutation.structural.add_neuron_rate = 1.0;
            if compatible {
                params.distance.disjoint_coefficient = 0.0;
                params.distance.excess_coefficient = 0.0;
                params.distance.weight_coefficient = 0.0;
            }
            let mut sim = make(&params, control);
            assert_eq!(sim.seed_founders(1), 1);
            sim.step_many(1);
            let unclassified = !compatible && capacity == 1;
            let created = if !compatible && capacity == 2 { 2 } else { 1 };
            assert_eq!(sim.population(), 2);
            assert_eq!(sim.descendants(), 1);
            assert_eq!(sim.species_count(), created);
            assert_eq!(sim.unclassified_population(), u32::from(unclassified));
            assert_eq!(
                inspection(&sim, 1, 1)["species_id"],
                if compatible {
                    0
                } else if unclassified {
                    NULL_ID
                } else {
                    1
                }
            );
            let observed = diagnostics(&sim);
            assert_eq!(
                observed.populations[0].population,
                if compatible { 2 } else { 1 }
            );
            assert_eq!(
                observed.events,
                SpeciesEventCounts {
                    created: u64::from(created),
                    unclassified_capacity: u64::from(unclassified),
                    ..Default::default()
                }
            );
            assert_no_spawn_refusals(&sim);
        }
    }
}

#[wasm_bindgen_test]
fn deaths_remove_members_once_and_retired_ids_are_not_reused() {
    for capacity in [0, 4] {
        let mut params = params();
        params.species.capacity = capacity;
        params.feeding.rate = 0.0;
        params.reproduction.start_energy = 1.0;
        params.reproduction.threshold = 2.0;
        params.metabolism.base = 10.0;
        let mut sim = make(&params, false);
        assert_eq!(sim.seed_founders(2), 2);
        sim.step_many(1);
        assert_eq!(sim.population(), 0);
        let expected = if capacity == 0 {
            SpeciesEventCounts {
                unclassified_capacity: 2,
                ..Default::default()
            }
        } else {
            SpeciesEventCounts {
                created: 2,
                extinct: 2,
                ..Default::default()
            }
        };
        assert_eq!(diagnostics(&sim).events, expected);
        sim.step_many(2);
        assert_eq!(diagnostics(&sim).events, expected);
        assert_eq!(sim.seed_founders(2), 2);
        let observed = diagnostics(&sim);
        if capacity == 0 {
            assert_eq!(observed.events.unclassified_capacity, 4);
        } else {
            assert_eq!(observed.events.created, 4);
            assert_eq!(observed.events.extinct, 2);
            assert_eq!(
                observed
                    .populations
                    .iter()
                    .map(|p| p.species_id)
                    .collect::<Vec<_>>(),
                vec![2, 3]
            );
        }
    }
}

#[wasm_bindgen_test]
fn classification_changes_observations_not_ecology_or_rng() {
    for control in [false, true] {
        let mut params = params();
        let mut classified = make(&params, control);
        params.species.capacity = 0;
        let mut unclassified = make(&params, control);
        assert_eq!(classified.seed_founders(4), 4);
        assert_eq!(unclassified.seed_founders(4), 4);
        for _ in 0..20 {
            for slot in 0..4 {
                let mut a = inspection(&classified, slot, 1);
                let mut b = inspection(&unclassified, slot, 1);
                a.as_object_mut().unwrap().remove("species_id");
                b.as_object_mut().unwrap().remove("species_id");
                assert_eq!(a, b);
            }
            classified.step_many(1);
            unclassified.step_many(1);
            assert_eq!(classified.population(), unclassified.population());
            assert_eq!(classified.mean_energy(), unclassified.mean_energy());
        }
    }
}
