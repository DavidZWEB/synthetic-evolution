//! Command-line shape for headless runs and telemetry diagnosis.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use sim_core::control::{BrainInheritance, RANDOMIZED_AT_BIRTH_PROTOCOL, STRUCTURAL_NULL_PROTOCOL};

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
    /// Validate and summarize a species-history archive or explicit capture prefix.
    History(HistoryArgs),
    /// Continue a saved run's worlds from their checkpoint, without replaying.
    Resume(ResumeArgs),
    /// Validate every component of a saved run and summarize it.
    SavedRun(SavedRunArgs),
    /// Summarize several metrics files across seeds, per configuration and cohort.
    Summarize(SummarizeArgs),
}

/// Native hosts accept saved worlds up to this core budget unless told otherwise.
const DEFAULT_MAX_CORE_BYTES: u64 = 8 << 30;

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
    /// Also archive each new species' representative genome at origin (history schema 3).
    #[arg(long, requires = "history")]
    pub representatives: bool,
    /// Preallocated genes per world for representatives awaiting a drain; used only
    /// with --representatives. A full buffer records the representative as unavailable.
    #[arg(long, default_value_t = 65_536, requires = "representatives", value_parser = clap::value_parser!(u32).range(1..))]
    pub representative_genes: u32,
    /// Write a saved-run bundle (checkpoints plus available history) after the final tick.
    #[arg(long)]
    pub save_run: Option<PathBuf>,
    /// Heredity of the paired control world. The structural null supports metrics only.
    #[arg(long, value_enum, default_value_t = Control::Scalar)]
    pub control: Control,
}

/// The paired control world's heredity protocol (spec section 7.8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum Control {
    /// Inherit topology, redraw neural scalars (`randomized_at_birth_v3`).
    #[default]
    Scalar,
    /// Take a living donor's topology, redraw neural scalars (`structural_null_v1`).
    StructuralNull,
}

impl Control {
    pub fn heredity(self) -> BrainInheritance {
        match self {
            Self::Scalar => BrainInheritance::RandomizedAtBirth,
            Self::StructuralNull => BrainInheritance::StructuralNull,
        }
    }

    pub fn protocol(self) -> &'static str {
        match self {
            Self::Scalar => RANDOMIZED_AT_BIRTH_PROTOCOL,
            Self::StructuralNull => STRUCTURAL_NULL_PROTOCOL,
        }
    }
}

impl RunArgs {
    /// The staging bound when representatives were requested.
    pub fn representative_genes(&self) -> Option<u32> {
        self.representatives.then_some(self.representative_genes)
    }
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
pub struct ResumeArgs {
    /// Saved-run bundle to continue.
    pub bundle: PathBuf,
    /// Additional ticks to run after the saved boundary.
    #[arg(long)]
    pub ticks: u64,
    /// Write a new saved-run bundle at the end of the continuation.
    #[arg(long)]
    pub save_run: Option<PathBuf>,
    /// Largest saved core budget (`storage.max_memory_bytes`) this host will restore.
    #[arg(long, default_value_t = DEFAULT_MAX_CORE_BYTES)]
    pub max_core_bytes: u64,
    /// Capture this continuation's species history (schema 4 segment), or `-` for stdout.
    #[arg(long)]
    pub history: Option<PathBuf>,
    /// Preallocated history records per world; used only with --history.
    #[arg(long, default_value_t = 4096, requires = "history", value_parser = clap::value_parser!(u32).range(1..))]
    pub history_capacity: u32,
    /// Also archive each new species' representative genome at origin.
    #[arg(long, requires = "history")]
    pub representatives: bool,
    /// Preallocated genes per world for representatives awaiting a drain.
    #[arg(long, default_value_t = 65_536, requires = "representatives", value_parser = clap::value_parser!(u32).range(1..))]
    pub representative_genes: u32,
    /// Ticks between history drains; used only with --history.
    #[arg(long, default_value_t = 1_000, requires = "history", value_parser = clap::value_parser!(u64).range(1..))]
    pub drain_every: u64,
}

#[derive(Clone, Debug, Args)]
pub struct SavedRunArgs {
    /// Saved-run bundle to validate.
    pub bundle: PathBuf,
    /// Emit the manifest as JSON instead of a human-readable summary.
    #[arg(long)]
    pub json: bool,
    /// Largest saved core budget (`storage.max_memory_bytes`) this host will restore.
    #[arg(long, default_value_t = DEFAULT_MAX_CORE_BYTES)]
    pub max_core_bytes: u64,
}

#[derive(Clone, Debug, Args)]
pub struct SummarizeArgs {
    /// Completed metrics JSONL files: one per seed and control protocol.
    #[arg(required = true)]
    pub metrics: Vec<PathBuf>,
    /// Emit the summary as JSON instead of human-readable text.
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
    fn representative_capture_requires_history_and_bounds_staging() {
        let cli = Cli::try_parse_from(["native", "--history", "-"]).unwrap();
        assert_eq!(cli.run.representative_genes(), None);
        let cli = Cli::try_parse_from(["native", "--history", "-", "--representatives"]).unwrap();
        assert_eq!(cli.run.representative_genes(), Some(65_536));
        let cli = Cli::try_parse_from([
            "native",
            "--history",
            "-",
            "--representatives",
            "--representative-genes",
            "2048",
        ])
        .unwrap();
        assert_eq!(cli.run.representative_genes(), Some(2048));
        for args in [
            &["native", "--representatives"][..],
            &["native", "--history", "-", "--representative-genes", "2048"],
            &[
                "native",
                "--history",
                "-",
                "--representatives",
                "--representative-genes",
                "0",
            ],
        ] {
            assert!(Cli::try_parse_from(args).is_err(), "accepted {args:?}");
        }
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
