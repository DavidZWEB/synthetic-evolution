//! Distance coefficient admission through the existing JSON parameter boundary.
//!
//! The distance foundation does not add a species API or alter simulation dynamics.

#![cfg(target_arch = "wasm32")]

use sim_core::params::SimParams;
use wasm::{Sim, random_control, validate_params};
use wasm_bindgen_test::wasm_bindgen_test;

#[wasm_bindgen_test]
fn distance_coefficients_round_trip_and_do_not_change_trajectories() {
    let initial = r#"{"world":{"max_agents":4},"plants":{"max_plants":8}}"#;
    let mut baseline = Sim::new(42, Some(initial.to_owned())).unwrap();
    let mut params: SimParams = serde_json::from_str(initial).unwrap();
    params.distance.disjoint_coefficient = 0.0;
    params.distance.excess_coefficient = 2.0;
    params.distance.weight_coefficient = 1.0;
    let canonical = validate_params(Some(serde_json::to_string(&params).unwrap())).unwrap();
    let mut measured = Sim::new(42, Some(canonical.clone())).unwrap();
    let mut control = random_control(42, Some(canonical)).unwrap();
    for sim in [&mut baseline, &mut measured, &mut control] {
        sim.seed_founders(2);
    }
    assert_eq!(
        serde_json::from_str::<SimParams>(&measured.params_json().unwrap())
            .unwrap()
            .distance,
        params.distance
    );
    assert_eq!(measured.state_hash(), control.state_hash());
    for _ in 0..20 {
        baseline.step_many(1);
        measured.step_many(1);
        assert_eq!(baseline.state_hash(), measured.state_hash());
    }
    params.distance.weight_coefficient = 0.75;
    let before = measured.state_hash();
    measured
        .set_params(&serde_json::to_string(&params).unwrap())
        .unwrap();
    assert_eq!(measured.state_hash(), before);
    assert_eq!(
        serde_json::from_str::<SimParams>(&measured.params_json().unwrap())
            .unwrap()
            .distance,
        params.distance
    );
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
