//! Stable organism identities at the browser inspection boundary.
//!
//! Exercises real admissions and slot reuse without changing the render snapshot
//! or adding test-only mutation APIs to the shell.

#![cfg(target_arch = "wasm32")]

use serde_json::Value;
use sim_core::ids::BirthId;
use sim_core::params::SimParams;
use wasm::Sim;
use wasm_bindgen_test::wasm_bindgen_test;

fn inspect(sim: &Sim, slot: u32, incarnation: u32) -> Value {
    serde_json::from_str(
        &sim.inspect_agent(slot, incarnation)
            .expect("live individual"),
    )
    .expect("valid inspection JSON")
}

fn birth_params() -> SimParams {
    let mut params: SimParams =
        serde_json::from_str(include_str!("../../native/tests/fixtures/structural.json")).unwrap();
    params.world.max_agents = 4;
    params = params.without_structural_mutation();
    params.reproduction.energy_split = 0.9;
    params
}

#[wasm_bindgen_test]
fn founder_ids_are_unique_world_local_and_only_issued_for_successful_admissions() {
    let params = r#"{"world":{"max_agents":4},"plants":{"max_plants":0}}"#;
    let mut sim = Sim::new(7, Some(params.into())).unwrap();
    assert_eq!(sim.seed_founders(2), 2);
    assert_eq!(sim.seed_founders(8), 2);
    assert_eq!(sim.seed_founders(1), 0);
    let before = sim.snapshot_layout().unwrap();
    for slot in 0..4 {
        let agent = inspect(&sim, slot, 1);
        assert_eq!(agent["birth_id"], slot.to_string());
        assert_eq!(agent["parent_birth_a"], Value::Null);
        assert_eq!(agent["parent_birth_b"], Value::Null);
        assert_eq!(agent["parent_a"], u32::MAX);
        assert_eq!(agent["parent_b"], u32::MAX);
    }
    assert_eq!(sim.snapshot_layout().unwrap(), before);
    let layout: Value = serde_json::from_str(&before).unwrap();
    for key in ["birth_id", "parent_birth_a", "parent_birth_b"] {
        assert!(
            layout.get(key).is_none(),
            "identity widened the render snapshot"
        );
    }

    let mut recreated = Sim::new(117, Some(params.into())).unwrap();
    assert_eq!(recreated.seed_founders(1), 1);
    assert_eq!(inspect(&recreated, 0, 1)["birth_id"], "0");
    assert_eq!(inspect(&sim, 3, 1)["birth_id"], "3");
}

#[wasm_bindgen_test]
fn newborn_keeps_its_parent_identity_after_the_parent_slot_is_recycled() {
    let mut params = birth_params();
    let mut sim = Sim::new(7, Some(serde_json::to_string(&params).unwrap())).unwrap();
    assert_eq!(sim.seed_founders(1), 1);
    let founder = inspect(&sim, 0, 1);
    sim.step_many(1);
    assert_eq!(sim.population(), 2);
    assert_eq!(sim.descendants(), 1);
    let parent = inspect(&sim, 0, 1);
    let child = inspect(&sim, 1, 1);
    assert_eq!(parent["birth_id"], founder["birth_id"]);
    assert_eq!(
        parent["genome"], founder["genome"],
        "birth changed a frozen genome"
    );
    assert_ne!(parent["activations"], founder["activations"]);
    assert_eq!(child["birth_id"], "1");
    assert_eq!(child["parent_birth_a"], founder["birth_id"]);
    assert_eq!(child["parent_birth_b"], Value::Null);
    assert_eq!(child["parent_a"], 0);
    assert_eq!(child["parent_b"], u32::MAX);

    // The larger child survives a cost that retires only its parent. This lets the
    // real free list recycle the legacy parent slot (spec §3.4).
    let parent_energy = parent["energy"].as_f64().unwrap();
    let child_energy = child["energy"].as_f64().unwrap();
    assert!(child_energy > parent_energy);
    params.feeding.rate = 0.0;
    params.reproduction.gate = 1.0;
    params.metabolism.base = ((parent_energy + child_energy) / 2.0) as f32;
    sim.set_params(&serde_json::to_string(&params).unwrap())
        .unwrap();
    sim.step_many(1);
    assert_eq!(sim.population(), 1);
    assert!(sim.inspect_agent(0, 1).is_err());
    assert_eq!(sim.seed_founders(1), 1);
    let replacement = inspect(&sim, 0, 2);
    assert_eq!(replacement["birth_id"], "2");
    assert_eq!(replacement["parent_birth_a"], Value::Null);
    assert!(
        sim.inspect_agent(0, 1).is_err(),
        "incarnation guard was bypassed"
    );

    let survivor = inspect(&sim, 1, 1);
    assert_eq!(survivor["birth_id"], child["birth_id"]);
    assert_eq!(survivor["parent_birth_a"], founder["birth_id"]);
    assert_ne!(survivor["parent_birth_a"], replacement["birth_id"]);
    assert_eq!(survivor["parent_a"], replacement["index"]);
    assert_eq!(survivor["genome"], child["genome"]);
}

#[wasm_bindgen_test]
fn a_reused_slot_has_a_new_birth_id_but_a_recreated_world_restarts_at_zero() {
    let params = r#"{
        "world":{"max_agents":1},
        "plants":{"max_plants":0},
        "feeding":{"rate":0.0},
        "metabolism":{"base":2.0,"k_size":0.0,"k_brain":0.0,"k_sensor":0.0,"k_move":0.0},
        "reproduction":{"start_energy":1.0,"threshold":10.0}
    }"#;
    let mut sim = Sim::new(7, Some(params.into())).unwrap();
    assert_eq!(sim.seed_founders(1), 1);
    assert_eq!(inspect(&sim, 0, 1)["birth_id"], "0");
    assert_eq!(sim.seed_founders(3), 0);
    sim.step_many(1);
    assert_eq!(sim.population(), 0);
    assert_eq!(sim.seed_founders(1), 1);
    let replacement = inspect(&sim, 0, 2);
    assert_eq!(
        replacement["birth_id"], "1",
        "failed admissions consumed IDs"
    );
    assert_eq!(replacement["parent_birth_a"], Value::Null);
    assert_eq!(replacement["parent_birth_b"], Value::Null);
    assert!(sim.inspect_agent(0, 1).is_err());

    let mut recreated = Sim::new(7, Some(params.into())).unwrap();
    assert_eq!(recreated.seed_founders(1), 1);
    assert_eq!(inspect(&recreated, 0, 1)["birth_id"], "0");
    assert_eq!(inspect(&sim, 0, 2)["birth_id"], "1");
}

#[wasm_bindgen_test]
fn birth_ids_serialize_to_exact_high_u64_json_strings_on_wasm() {
    let ids = [
        BirthId::new(0),
        BirthId::new(9_007_199_254_740_993),
        BirthId::new(u64::MAX - 1),
        BirthId::NULL,
    ];
    let json = serde_json::to_string(&ids).unwrap();
    assert_eq!(
        json,
        r#"["0","9007199254740993","18446744073709551614",null]"#
    );
    assert_eq!(serde_json::from_str::<[BirthId; 4]>(&json).unwrap(), ids);
}
