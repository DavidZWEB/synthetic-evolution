//! Native executable entry point.
//!
//! Argument parsing, simulation orchestration, metrics I/O, and diagnostics live in the
//! library modules so they can be exercised without spawning a subprocess.

fn main() {
    if let Err(error) = native::run_cli() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
