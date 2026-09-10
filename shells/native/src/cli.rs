//! Command-line shape for headless runs and telemetry diagnosis.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(version, about = "Headless Synthetic Evolution experiments")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
    #[command(flatten)]
    pub run: RunArgs,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Diagnose known failure signatures in a metrics JSONL file.
    Diagnose(DiagnoseArgs),
    /// Validate and summarize a completed species-history JSONL archive.
    History(HistoryArgs),
}

#[derive(Clone, Debug, Args)]
pub struct RunArgs {
    /// Deterministic world seed.
    #[arg(long, default_value_t = 42)]
    pub seed: u64,
    /// Number of simulation ticks to execute.
    #[arg(long, default_value_t = 100_000)]
    pub ticks: u64,
    /// Generation-zero population.
    #[arg(long, default_value_t = 200)]
    pub founders: u32,
    /// Tick interval between metrics samples.
    #[arg(long, default_value_t = 1_000)]
    pub sample_every: u64,
    /// Optional SimParams JSON file. Missing fields use shipped defaults.
    #[arg(long)]
    pub params: Option<PathBuf>,
    /// JSONL output path, or `-` for stdout.
    #[arg(long)]
    pub metrics: Option<PathBuf>,
    /// Species-history JSONL path, or `-` for stdout (separate from metrics).
    #[arg(long)]
    pub history: Option<PathBuf>,
    /// Preallocated history records per world; used only with --history.
    #[arg(long, default_value_t = 4096, requires = "history", value_parser = clap::value_parser!(u32).range(1..))]
    pub history_capacity: u32,
}

#[derive(Clone, Debug, Args)]
pub struct DiagnoseArgs {
    /// Metrics JSONL path, or `-` for stdin.
    pub metrics: PathBuf,
    /// Emit the report as JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Clone, Debug, Args)]
pub struct HistoryArgs {
    /// Species-history JSONL path, or `-` for stdin.
    pub history: PathBuf,
    /// Emit the summary as JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_documented_run_shape_parses_without_a_subcommand() {
        let cli = Cli::try_parse_from([
            "native",
            "--seed",
            "7",
            "--ticks",
            "500000",
            "--metrics",
            "run.jsonl",
        ])
        .expect("documented command parses");
        assert!(cli.command.is_none());
        assert_eq!(cli.run.seed, 7);
        assert_eq!(cli.run.ticks, 500_000);
    }

    #[test]
    fn diagnose_is_a_subcommand() {
        let cli = Cli::try_parse_from(["native", "diagnose", "run.jsonl", "--json"])
            .expect("diagnose parses");
        assert!(matches!(
            cli.command,
            Some(Command::Diagnose(DiagnoseArgs { json: true, .. }))
        ));
    }

    #[test]
    fn history_capture_and_readback_parse() {
        let cli = Cli::try_parse_from(["native"]).unwrap();
        assert!(cli.run.history.is_none());
        assert_eq!(cli.run.history_capacity, 4096);
        let cli =
            Cli::try_parse_from(["native", "--history", "-", "--history-capacity", "1"]).unwrap();
        assert_eq!(cli.run.history_capacity, 1);
        assert!(Cli::try_parse_from(["native", "--history-capacity", "1"]).is_err());
        assert!(
            Cli::try_parse_from(["native", "--history", "-", "--history-capacity", "0"]).is_err()
        );
        let cli = Cli::try_parse_from(["native", "history", "-", "--json"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::History(HistoryArgs { json: true, .. }))
        ));
    }
}
