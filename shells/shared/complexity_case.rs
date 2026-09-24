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
    r#"{"genome_genes":{"min":68,"p25":81,"median":81,"p75":82,"max":95,"mean":83.09375},"#,
    r#""neurons":{"min":15,"p25":15,"median":15,"p75":15,"max":15,"mean":15.0},"#,
    r#""connections":{"min":39,"p25":52,"median":52,"p75":53,"max":66,"mean":54.09375},"#,
    r#""enabled_connections":{"min":36,"p25":49,"median":50,"p75":51,"max":66,"mean":52.21875}}"#,
);
