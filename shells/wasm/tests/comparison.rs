//! Representative comparison through the WASM boundary matches the classifier.
//!
//! Distinct species are at least the threshold apart; a genome compares to itself
//! at zero; and the reported terms sum to the classifier's value.

#![cfg(target_arch = "wasm32")]

use serde_json::{Value, json};
use wasm::{Sim, compare_representatives};
use wasm_bindgen_test::wasm_bindgen_test;

#[wasm_bindgen_test]
fn archived_representatives_compare_exactly_as_the_classifier_splits_them() {
    let params = json!({"world": {"max_agents": 8}, "plants": {"max_plants": 4},
        "species": {"capacity": 8, "threshold": 0.000001}})
    .to_string();
    let mut sim = Sim::new(7, Some(params.clone())).unwrap();
    sim.enable_history(16, Some(65_536)).unwrap();
    assert_eq!(sim.seed_founders(2), 2);
    let drain: Value = serde_json::from_str(&sim.drain_history().unwrap()).unwrap();
    let genomes: Vec<String> = drain["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["data"]["representative"]["genes"].to_string())
        .collect();
    let full = sim.params_json().unwrap();
    let compare = |a: &str, b: &str| -> Value {
        serde_json::from_str(&compare_representatives(a, b, &full).unwrap()).unwrap()
    };

    let same = compare(&genomes[0], &genomes[0]);
    assert_eq!(same["value"], 0.0);
    let pair = compare(&genomes[0], &genomes[1]);
    let value = pair["value"].as_f64().unwrap();
    assert!(
        value >= pair["threshold"].as_f64().unwrap(),
        "two species were created"
    );
    let terms: f64 = ["disjoint_term", "excess_term", "weight_term"]
        .iter()
        .map(|key| pair[*key].as_f64().unwrap())
        .sum();
    assert!((terms - value).abs() < 1e-12);

    let mut unsorted: Vec<Value> = serde_json::from_str(&genomes[0]).unwrap();
    unsorted.swap(0, 1);
    assert!(
        compare_representatives(&Value::from(unsorted).to_string(), &genomes[1], &full).is_err()
    );
}
