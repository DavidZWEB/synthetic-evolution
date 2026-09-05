//! Native CLI contracts exercised through the compiled executable.

use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use native::metrics::{
    MetricsRecord, RunHeader, RunSample, SCHEMA_VERSION, StateHashes, Summary, WorldMetrics,
};
use sim_core::params::SimParams;

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

fn temporary(name: &str) -> std::path::PathBuf {
    let suffix = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "synthetic-evolution-{}-{suffix}-{name}",
        std::process::id()
    ))
}

fn run_and_diagnose(
    params_json: &str,
    ticks: u64,
    sample_every: u64,
    founders: u32,
) -> (Vec<String>, serde_json::Value) {
    let params = temporary("params.json");
    let metrics = temporary("metrics.jsonl");
    fs::write(&params, params_json).expect("write params");

    let run = Command::new(env!("CARGO_BIN_EXE_native"))
        .args([
            "--seed",
            "7",
            "--ticks",
            &ticks.to_string(),
            "--founders",
            &founders.to_string(),
            "--sample-every",
            &sample_every.to_string(),
            "--params",
        ])
        .arg(&params)
        .arg("--metrics")
        .arg(&metrics)
        .output()
        .expect("run native shell");
    assert!(
        run.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let lines: Vec<_> = fs::read_to_string(&metrics)
        .expect("metrics file")
        .lines()
        .map(str::to_owned)
        .collect();
    let report = diagnose_file(&metrics);

    fs::remove_file(params).expect("remove params");
    fs::remove_file(metrics).expect("remove metrics");
    (lines, report)
}

fn diagnose_file(metrics: &std::path::Path) -> serde_json::Value {
    let diagnosis = Command::new(env!("CARGO_BIN_EXE_native"))
        .arg("diagnose")
        .arg(metrics)
        .arg("--json")
        .output()
        .expect("diagnose metrics");
    assert!(
        diagnosis.status.success(),
        "diagnose failed: {}",
        String::from_utf8_lossy(&diagnosis.stderr)
    );
    serde_json::from_slice(&diagnosis.stdout).expect("diagnosis JSON")
}

#[test]
fn run_writes_self_describing_jsonl_that_diagnose_reads() {
    let (lines, report) = run_and_diagnose(include_str!("fixtures/monoculture.json"), 10, 5, 1);

    assert_eq!(lines.len(), 4, "header plus ticks 0, 5, and 10");
    assert!(lines[0].contains(r#""kind":"header""#));
    assert!(lines[1].contains(r#""random_control""#));
    let header: serde_json::Value = serde_json::from_str(&lines[0]).expect("header JSON");
    assert_eq!(header["data"]["phase"], 1);
    assert!(
        header["data"]["source_revision"]
            .as_str()
            .is_some_and(|revision| !revision.is_empty())
    );
    let final_sample: serde_json::Value =
        serde_json::from_str(lines.last().unwrap()).expect("final sample JSON");
    assert!(
        final_sample["data"]["final_state_hashes"]["evolving"]
            .as_str()
            .is_some_and(|hash| hash.len() == 16)
    );
    assert_eq!(report["samples"], 3);
    assert!(
        !report["evolving"]
            .as_array()
            .expect("evolving findings")
            .iter()
            .any(|finding| finding["code"]
                .as_str()
                .is_some_and(|code| code.contains("extinction")))
    );
}

#[test]
fn diagnose_finds_an_extinction_induced_by_an_impossible_energy_budget() {
    let (_, report) = run_and_diagnose(include_str!("fixtures/extinction.json"), 20, 5, 4);
    assert!(
        report["evolving"]
            .as_array()
            .expect("evolving findings")
            .iter()
            .any(|finding| finding["code"] == "early_extinction")
    );
}

#[test]
fn diagnose_finds_a_deliberately_collapsed_genome_population() {
    let metrics = temporary("collapsed.jsonl");
    let header = RunHeader {
        schema_version: SCHEMA_VERSION,
        sim_version: "test".to_owned(),
        source_revision: "test".to_owned(),
        phase: 1,
        seed: "7".to_owned(),
        ticks: 5_000,
        founders: 100,
        sample_every: 1_000,
        params: SimParams::default(),
        control: "randomized_at_birth".to_owned(),
    };
    let world = |variants| WorldMetrics {
        population: 100,
        genome_variants: variants,
        speed: Summary {
            mean: 1.0,
            max: 2.0,
        },
        brain_units: Summary {
            mean: 268.0,
            max: 268.0,
        },
        sensor_load: Summary {
            mean: 16.0,
            max: 16.0,
        },
        ..WorldMetrics::default()
    };
    let mut records = vec![MetricsRecord::Header(header)];
    for tick in (0..=5_000).step_by(1_000) {
        let variants = if tick == 0 { 4 } else { 1 };
        records.push(MetricsRecord::Sample(RunSample {
            tick,
            evolving: world(variants),
            random_control: world(variants),
            final_state_hashes: (tick == 5_000).then(|| StateHashes {
                evolving: "0000000000000001".to_owned(),
                random_control: "0000000000000001".to_owned(),
            }),
        }));
    }
    let jsonl = records
        .iter()
        .map(serde_json::to_string)
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("records serialize")
        .join("\n");
    fs::write(&metrics, format!("{jsonl}\n")).expect("write collapsed metrics");

    let report = diagnose_file(&metrics);
    assert!(
        report["evolving"]
            .as_array()
            .expect("evolving findings")
            .iter()
            .any(|finding| finding["code"] == "monoculture")
    );
    fs::remove_file(metrics).expect("remove metrics");
}
