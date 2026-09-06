//! Native shell: headless runs, metrics files, and diagnostics.
//!
//! Owns filesystem and terminal I/O around the pure `sim-core` library. Simulation
//! behavior remains in the core; this crate samples completed ticks and interprets the
//! resulting telemetry.

pub mod cli;
pub mod diagnose;
mod diagnose_output;
pub mod metrics;
mod metrics_reader;
pub mod run;

use std::error::Error;

use clap::Parser;
use cli::{Cli, Command};

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub fn run_cli() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Diagnose(args)) => diagnose::run(args),
        None => run::run(cli.run),
    }
}
