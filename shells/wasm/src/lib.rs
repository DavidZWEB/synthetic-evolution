//! WASM shell: `wasm-bindgen` surface for the browser worker.
//!
//! Owns the JS boundary and nothing else — it holds a `World`, steps it, and exposes
//! the render snapshot as a pointer into WASM linear memory. The agent pool is
//! pre-allocated at capacity so memory never grows, because growing it detaches every
//! JS typed-array view over the snapshot (spec §7.3).
//!
//! Deliberately not here: any simulation logic. If a function here does more than
//! marshal, it belongs in `sim-core`.

use wasm_bindgen::prelude::*;

/// Crate version, so the worker can assert it matches the JS bundle it shipped with.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
