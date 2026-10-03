//! Browser checkpoint restore and saved-run framing through the WASM boundary.
//!
//! The browser saves and loads through these bindings, so a restored world must
//! continue exactly and a framed bundle must decode to the same sections.

#![cfg(target_arch = "wasm32")]

use serde_json::{Value, json};
use wasm::{Sim, checkpoint_format, decode_saved_run, encode_saved_run, random_control};
use wasm_bindgen_test::wasm_bindgen_test;

const MAX_CORE: u64 = 96 << 20;

fn params() -> String {
    json!({"world": {"max_agents": 32}, "plants": {"max_plants": 16}, "chemo": {"cells": [8, 8, 1]}})
        .to_string()
}

fn stepped(control: bool) -> Sim {
    let mut sim = if control {
        random_control(7, Some(params())).unwrap()
    } else {
        Sim::new(7, Some(params())).unwrap()
    };
    sim.seed_founders(8);
    sim.step_many(40);
    sim
}

#[wasm_bindgen_test]
fn restored_sims_continue_exactly_and_keep_their_identity() {
    for control in [false, true] {
        let mut original = stepped(control);
        let mut restored = Sim::restore(&original.checkpoint(), MAX_CORE).unwrap();
        assert_eq!(restored.state_hash(), original.state_hash());
        assert_eq!(restored.seed(), 7);
        assert_eq!(restored.tick(), 40);
        assert_eq!(
            restored.heredity(),
            if control {
                "randomized_at_birth"
            } else {
                "evolving"
            }
        );
        original.step_many(60);
        restored.step_many(60);
        assert_eq!(restored.state_hash(), original.state_hash());
    }
    let bytes = stepped(false).checkpoint();
    assert!(Sim::restore(&bytes, 1).is_err(), "host budget");
    assert!(Sim::restore(&bytes[..bytes.len() - 1], MAX_CORE).is_err());
}

#[wasm_bindgen_test]
fn saved_runs_frame_and_decode_through_the_shared_format() {
    let sim = stepped(false);
    let checkpoint = sim.checkpoint();
    let archive = b"history";
    let manifest = json!({
        "format": "synthetic-evolution-saved-run", "version": 1,
        "checkpoint_format": checkpoint_format(),
        "provenance": {"sim_version": "0.1.0", "source_revision": "test", "phase": 2,
            "seed": "7", "founders": 8, "control": "randomized_at_birth_v3", "run_id": "r"},
        "writer": {"sim_version": "0.1.0", "source_revision": "test"},
        "tick": "40",
        "cohorts": [{"cohort": "evolving", "state_hash": format!("{:016x}", sim.state_hash()),
            "bytes": checkpoint.len().to_string()}],
        "history": [{"status": "included", "starts_at": "0", "bytes": archive.len().to_string()}],
    });
    let sections = [checkpoint.as_slice(), archive].concat();
    let bundle = encode_saved_run(&manifest.to_string(), &sections).unwrap();
    let decoded: Value =
        serde_json::from_str(&decode_saved_run(&bundle, bundle.len(), MAX_CORE).unwrap()).unwrap();
    assert_eq!(decoded["manifest"], manifest);
    let span = |value: &Value| {
        let [offset, len] = [value[0].as_u64().unwrap(), value[1].as_u64().unwrap()];
        &bundle[offset as usize..(offset + len) as usize]
    };
    assert_eq!(span(&decoded["checkpoints"][0]), checkpoint.as_slice());
    assert_eq!(span(&decoded["history"][0]), archive);

    assert!(encode_saved_run(&manifest.to_string(), &sections[1..]).is_err());
    let mut wrong_hash = manifest.clone();
    wrong_hash["cohorts"][0]["state_hash"] = json!("0000000000000000");
    let mismatched = encode_saved_run(&wrong_hash.to_string(), &sections).unwrap();
    assert!(decode_saved_run(&mismatched, mismatched.len(), MAX_CORE).is_err());
    assert!(decode_saved_run(&bundle, bundle.len() - 1, MAX_CORE).is_err());
}
