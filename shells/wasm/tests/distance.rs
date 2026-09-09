//! Distance coefficient admission through the existing JSON parameter boundary.
//!
//! Coefficients are frozen once a world's classifier is constructed.

#![cfg(target_arch = "wasm32")]

use sim_core::params::SimParams;
use wasm::{Sim, random_control, validate_params};
use wasm_bindgen_test::wasm_bindgen_test;

#[wasm_bindgen_test]
fn distance_coefficients_round_trip_and_are_frozen_in_both_heredity_modes() {
    let initial = r#"{"world":{"max_agents":4},"plants":{"max_plants":8}}"#;
    let mut params: SimParams = serde_json::from_str(initial).unwrap();
    params.distance.disjoint_coefficient = 0.0;
    params.distance.excess_coefficient = 2.0;
    params.distance.weight_coefficient = 1.0;
    let canonical = validate_params(Some(serde_json::to_string(&params).unwrap())).unwrap();
    let mut measured = Sim::new(42, Some(canonical.clone())).unwrap();
    let mut control = random_control(42, Some(canonical)).unwrap();
    for sim in [&mut measured, &mut control] {
        sim.seed_founders(2);
    }
    assert_eq!(
        serde_json::from_str::<SimParams>(&measured.params_json().unwrap())
            .unwrap()
            .distance,
        params.distance
    );
    for sim in [&mut measured, &mut control] {
        let original = sim.params_json().unwrap();
        let before = sim.state_hash();
        for field in [
            "disjoint_coefficient",
            "excess_coefficient",
            "weight_coefficient",
        ] {
            let mut changed = serde_json::to_value(&params).unwrap();
            changed["distance"][field] = serde_json::json!(0.75);
            let changed = changed.to_string();
            assert!(validate_params(Some(changed.clone())).is_ok());
            assert!(sim.set_params(&changed).is_err());
            assert_eq!(sim.state_hash(), before);
            assert_eq!(sim.params_json().unwrap(), original);
        }
    }
}

#[wasm_bindgen_test]
fn invalid_distance_coefficients_are_rejected_without_replacing_params() {
    let mut params = SimParams::default();
    params.world.max_agents = 4;
    params.plants.max_plants = 8;
    let original = serde_json::to_string(&params).unwrap();
    let mut sim = Sim::new(42, Some(original.clone())).unwrap();
    let before = sim.state_hash();
    for field in [
        "disjoint_coefficient",
        "excess_coefficient",
        "weight_coefficient",
    ] {
        for value in [-1.0, 1e40] {
            let mut invalid = serde_json::to_value(&params).unwrap();
            invalid["distance"][field] = serde_json::json!(value);
            let json = invalid.to_string();
            assert!(validate_params(Some(json.clone())).is_err());
            assert!(Sim::new(42, Some(json.clone())).is_err());
            assert!(random_control(42, Some(json.clone())).is_err());
            assert!(sim.set_params(&json).is_err());
            assert_eq!(sim.state_hash(), before);
            assert_eq!(sim.params_json().unwrap(), original);
        }
    }
}
