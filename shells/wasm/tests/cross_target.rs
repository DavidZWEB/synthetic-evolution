//! Native and wasm must produce the same number from the same seed.
//!
//! This is the half of spec §7.8's determinism criterion that a native test cannot
//! reach. The failure it exists for is silent: a platform `sin` differing in the last
//! bit, an `f32` spilled to x87 80-bit on one target and not the other, a `usize` that
//! is 64 bits here and 32 there. None of it panics, none of it fails a unit test, and
//! all of it means a seed shared between the browser and a headless sweep describes two
//! different worlds.
//!
//! Run with `wasm-pack test --node shells/wasm`.
//!
//! The scenario is included from `sim-core`'s tests rather than restated, so that
//! "native and wasm agree" cannot quietly become "each agrees with itself".

#![cfg(target_arch = "wasm32")]

use wasm_bindgen_test::wasm_bindgen_test;

#[allow(dead_code)]
#[path = "../../../sim-core/tests/common/golden_case.rs"]
mod common;
use common::*;

#[path = "../../../sim-core/tests/common/storage_case.rs"]
mod storage_case;

#[path = "../../../sim-core/tests/common/structural_case.rs"]
mod structural_case;

#[path = "../../../sim-core/tests/common/organs_case.rs"]
mod organs_case;

#[wasm_bindgen_test]
fn organ_mutation_agrees_with_native_in_both_modes() {
    organs_case::check_organ_runs();
}

#[wasm_bindgen_test]
fn structural_mutation_agrees_with_native_in_both_modes() {
    structural_case::check_structural_runs();
}

#[wasm_bindgen_test]
fn variable_storage_agrees_with_native_in_both_heredity_modes() {
    storage_case::check_variable_storage_runs();
}

#[wasm_bindgen_test]
fn wasm_agrees_with_native() {
    check_golden_runs();
}

#[wasm_bindgen_test]
fn two_runs_in_one_wasm_module_agree() {
    assert_eq!(run(7, shipped(), 64, 200), run(7, shipped(), 64, 200));
}
