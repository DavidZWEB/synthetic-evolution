//! Native shell: headless runs, batch sweeps, and the golden-hash / cross-target
//! test path. Owns all file and stdout I/O for the simulation; `sim-core` owns none.

fn main() {
    // Argument parsing and the headless run loop arrive with the tick (M8) and
    // telemetry (M11). Scaffold only.
    println!(
        "synthetic-evolution native shell {}",
        env!("CARGO_PKG_VERSION")
    );
}
