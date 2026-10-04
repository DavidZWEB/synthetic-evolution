//! The params every mechanism scenario starts from, owned by the tests.
//!
//! A complete document rather than `SimParams::default()`, so changing a shipped
//! default moves only the shipped-defaults golden runs, not every scenario's
//! reference. Mutation rates are zero; scenarios opt into the operators they test.
//! Fields added to `SimParams` later take their defaults until written here.

use sim_core::SimParams;

pub fn params() -> SimParams {
    serde_json::from_str(include_str!("../fixtures/scenario-params.json"))
        .expect("valid scenario params")
}
