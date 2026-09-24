//! Live genome-size distributions through the WASM boundary.
//!
//! The reference is the native shell's, so browser and headless reports of the same
//! completed tick are compared exactly rather than each against itself.

#![cfg(target_arch = "wasm32")]

use wasm::Sim;
use wasm_bindgen_test::wasm_bindgen_test;

#[path = "../../shared/complexity_case.rs"]
mod complexity_case;

#[wasm_bindgen_test]
fn complexity_diagnostics_agree_with_native_metrics() {
    let mut sim = Sim::new(
        complexity_case::SEED,
        Some(complexity_case::PARAMS.to_owned()),
    )
    .unwrap();
    assert_eq!(
        sim.seed_founders(complexity_case::FOUNDERS),
        complexity_case::FOUNDERS
    );
    sim.step_many(complexity_case::TICKS);
    assert_eq!(
        sim.complexity_diagnostics().unwrap(),
        complexity_case::EXPECTED
    );
}

#[wasm_bindgen_test]
fn an_empty_world_reports_zero_distributions() {
    let sim = Sim::new(1, None).unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&sim.complexity_diagnostics().unwrap()).unwrap();
    for field in [
        "genome_genes",
        "neurons",
        "connections",
        "enabled_connections",
    ] {
        assert_eq!(value[field]["max"], 0);
        assert_eq!(value[field]["mean"], 0.0);
    }
}
