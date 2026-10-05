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

#[wasm_bindgen_test]
fn an_archive_from_before_evolving_bodies_reads_as_the_run_it_recorded() {
    // Written before Phase 3: no trait ranges or trait upkeep, and a body size today's
    // default range would refuse. Read as that run had them, as the native reader
    // reads them, rather than with today's defaults.
    let params = json!({"world": {"max_agents": 8}, "plants": {"max_plants": 4},
        "species": {"capacity": 8, "threshold": 0.000001}})
    .to_string();
    let mut sim = Sim::new(7, Some(params)).unwrap();
    sim.enable_history(16, Some(65_536)).unwrap();
    assert_eq!(sim.seed_founders(1), 1);
    let drain: Value = serde_json::from_str(&sim.drain_history().unwrap()).unwrap();
    let genes = drain["records"][0]["data"]["representative"]["genes"].to_string();
    let mut older: Value = serde_json::from_str(&sim.params_json().unwrap()).unwrap();
    older["body"] = json!({"size": 1.0});
    for field in ["k_muscle", "k_mouth"] {
        older["metabolism"].as_object_mut().unwrap().remove(field);
    }
    for field in ["body_trait_rate", "body_trait_sigma"] {
        older["mutation"].as_object_mut().unwrap().remove(field);
    }
    for section in ["combat", "founder"] {
        older.as_object_mut().unwrap().remove(section);
    }
    let older = older.to_string();
    let same: Value =
        serde_json::from_str(&compare_representatives(&genes, &genes, &older).unwrap()).unwrap();
    assert_eq!(same["value"], 0.0);
}

#[wasm_bindgen_test]
fn a_small_world_from_before_the_bite_reads_without_a_founder_bite() {
    // Its founders never bit, so a bite reach that no 30-unit world could fit past two
    // of its largest bodies must not refuse it.
    let params = json!({"world": {"max_agents": 8}, "plants": {"max_plants": 4},
        "species": {"capacity": 8, "threshold": 0.000001}})
    .to_string();
    let mut sim = Sim::new(7, Some(params)).unwrap();
    sim.enable_history(16, Some(65_536)).unwrap();
    assert_eq!(sim.seed_founders(1), 1);
    let drain: Value = serde_json::from_str(&sim.drain_history().unwrap()).unwrap();
    let genes = drain["records"][0]["data"]["representative"]["genes"].to_string();
    let mut older: Value = serde_json::from_str(&sim.params_json().unwrap()).unwrap();
    older["world"]["size"] = json!(30.0);
    older["sensing"]["vision_range"] = json!(10.0);
    older["sensing"]["chemo_radius"] = json!(10.0);
    older["plants"]["dispersal_radius"] = json!(10.0);
    older["plants"]["patch_scale"] = json!(10.0);
    for section in ["combat", "founder"] {
        older.as_object_mut().unwrap().remove(section);
    }
    let older = older.to_string();
    let same: Value =
        serde_json::from_str(&compare_representatives(&genes, &genes, &older).unwrap()).unwrap();
    assert_eq!(same["value"], 0.0);
}

#[wasm_bindgen_test]
fn an_archive_whose_founders_bite_must_record_their_combat() {
    // Today's defaults could describe attack rules the run never had.
    let params = json!({"world": {"max_agents": 8}, "plants": {"max_plants": 4},
        "species": {"capacity": 8, "threshold": 0.000001}})
    .to_string();
    let mut sim = Sim::new(7, Some(params)).unwrap();
    sim.enable_history(16, Some(65_536)).unwrap();
    assert_eq!(sim.seed_founders(1), 1);
    let drain: Value = serde_json::from_str(&sim.drain_history().unwrap()).unwrap();
    let genes = drain["records"][0]["data"]["representative"]["genes"].to_string();
    let mut biting: Value = serde_json::from_str(&sim.params_json().unwrap()).unwrap();
    biting["founder"]["bite"] = json!(true);
    assert!(compare_representatives(&genes, &genes, &biting.to_string()).is_ok());
    let mut empty = biting.clone();
    empty["combat"] = json!({});
    assert!(compare_representatives(&genes, &genes, &empty.to_string()).is_err());
    // Without a bite, a present section must still be complete.
    empty["founder"]["bite"] = json!(false);
    assert!(compare_representatives(&genes, &genes, &empty.to_string()).is_err());
    let mut empty_founder = biting.clone();
    empty_founder["founder"] = json!({});
    assert!(compare_representatives(&genes, &genes, &empty_founder.to_string()).is_err());
    biting.as_object_mut().unwrap().remove("combat");
    assert!(compare_representatives(&genes, &genes, &biting.to_string()).is_err());
}
