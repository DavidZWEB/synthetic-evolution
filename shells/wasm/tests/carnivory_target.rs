//! WASM half of the carnivory mechanisms' cross-target check (research only).
//!
//! Run with `wasm-pack test --node shells/wasm --test carnivory_target`.

#![cfg(target_arch = "wasm32")]

use wasm_bindgen_test::wasm_bindgen_test;

#[path = "../../../sim-core/tests/common/scenario.rs"]
mod scenario;

#[path = "../../../sim-core/tests/common/carnivory_case.rs"]
mod carnivory_case;

#[wasm_bindgen_test]
fn carnivory_mechanisms_agree_with_native() {
    assert_eq!(carnivory_case::carnivory_hashes(), carnivory_case::GOLDEN);
}
