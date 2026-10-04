//! One structurally mutating run whose live genome-size distributions the native and
//! WASM shells must report byte-identically.
//!
//! The expected text is shared rather than restated, so "native and browser agree"
//! cannot quietly become "each agrees with itself".

pub const PARAMS: &str = include_str!("../native/tests/fixtures/structural.json");
pub const SEED: u64 = 7;
pub const FOUNDERS: u32 = 8;
pub const TICKS: u32 = 40;
pub const EXPECTED: &str = concat!(
    r#"{"genome_genes":{"min":35,"p25":35,"median":36,"p75":36,"max":37,"mean":36.0},"#,
    r#""neurons":{"min":14,"p25":14,"median":14,"p75":14,"max":14,"mean":14.0},"#,
    r#""connections":{"min":6,"p25":6,"median":7,"p75":7,"max":8,"mean":7.0},"#,
    r#""enabled_connections":{"min":3,"p25":5,"median":6,"p75":6,"max":7,"mean":5.46875},"#,
    r#""wiring":{"wired_hidden_neurons":{"min":0,"p25":0,"median":1,"p75":1,"max":2,"mean":0.625},"#,
    r#""wired_sensors":{"min":0,"p25":1,"median":2,"p75":2,"max":2,"mean":1.6875},"#,
    r#""driven_effectors":{"min":0,"p25":2,"median":3,"p75":4,"max":4,"mean":2.9375}}}"#,
);
