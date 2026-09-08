//! Native CLI contracts exercised through the compiled executable.

use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use sim_core::control::RANDOMIZED_AT_BIRTH_PROTOCOL;

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
    seed: u64,
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
            &seed.to_string(),
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
    let (lines, report) = run_and_diagnose(include_str!("fixtures/sustaining.json"), 7, 10, 5, 1);

    assert_eq!(lines.len(), 4, "header plus ticks 0, 5, and 10");
    assert!(lines[0].contains(r#""kind":"header""#));
    assert!(lines[1].contains(r#""random_control""#));
    let header: serde_json::Value = serde_json::from_str(&lines[0]).expect("header JSON");
    assert_eq!(header["data"]["phase"], 2);
    assert_eq!(header["data"]["schema_version"], 4);
    assert_eq!(header["data"]["control"], RANDOMIZED_AT_BIRTH_PROTOCOL);
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
    for line in &lines[1..] {
        let sample: serde_json::Value = serde_json::from_str(line).expect("sample JSON");
        for cohort in ["evolving", "random_control"] {
            assert_eq!(
                sample["data"][cohort]["arena_usage"]
                    .as_array()
                    .unwrap()
                    .len(),
                5
            );
            let failures = sample["data"][cohort]["spawn_failures"]
                .as_object()
                .unwrap();
            assert_eq!(failures.len(), 6);
            assert!(failures.values().all(|count| count.as_u64().is_some()));
            let mutations = sample["data"][cohort]["structural_mutations"]
                .as_object()
                .unwrap();
            assert_eq!(mutations.len(), 5);
            for counts in mutations.values() {
                let counts = counts.as_object().unwrap();
                assert_eq!(counts.len(), 6);
                assert!(counts.values().all(|count| count.as_u64() == Some(0)));
            }
        }
    }
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
    let (_, report) = run_and_diagnose(include_str!("fixtures/extinction.json"), 7, 20, 5, 4);
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
    // This real no-mutation sweep depends on founder placement. Seed 20 reaches a
    // stable one-genome population under the current equal-area layout; if placement
    // changes, reselect a deterministic collapsing seed rather than weakening the
    // diagnostic assertion or replacing the simulator-driven fixture.
    let (_, report) = run_and_diagnose(include_str!("fixtures/monoculture.json"), 20, 4_000, 50, 2);
    assert!(
        report["evolving"]
            .as_array()
            .expect("evolving findings")
            .iter()
            .any(|finding| finding["code"] == "monoculture")
    );
}

#[test]
fn collected_runs_report_arena_pressure_separately_from_structural_edits() {
    let params = r#"{
        "world":{"size":100.0,"max_agents":2},
        "storage":{"genes_per_slot":142},
        "sensing":{"vision_range":20.0,"chemo_radius":20.0},
        "reproduction":{"start_energy":1.0,"threshold":1.1,"gate":0.0,"maturity_ticks":0},
        "feeding":{"rate":100.0,"gate":0.0,"reach":20.0},
        "plants":{"max_plants":100,"max_energy":100.0,"initial_fill":1.0},
        "metabolism":{"base":0.0,"k_size":0.0,"k_brain":0.0,"k_sensor":0.0,"k_move":0.0}
    }"#;
    let (lines, report) = run_and_diagnose(params, 7, 2, 1, 1);
    let header: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(header["data"]["control"], RANDOMIZED_AT_BIRTH_PROTOCOL);
    for cohort in ["evolving", "random_control"] {
        let initial: serde_json::Value = serde_json::from_str(&lines[1]).unwrap();
        assert_eq!(
            initial["data"][cohort]["spawn_failures"]["arena_capacity"],
            0
        );
        let final_sample: serde_json::Value = serde_json::from_str(lines.last().unwrap()).unwrap();
        assert!(
            final_sample["data"][cohort]["spawn_failures"]["arena_capacity"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert_eq!(
            final_sample["data"][cohort]["spawn_failures"]["pool_full"],
            0
        );
        assert_eq!(
            final_sample["data"][cohort]["arena_usage"][0]["free_elements"],
            0
        );
        assert!(
            report[cohort]
                .as_array()
                .unwrap()
                .iter()
                .any(|finding| finding["code"] == "storage_capacity")
        );
    }
}

#[test]
fn configured_structural_edits_are_observed_in_each_cohort() {
    let (lines, report) = run_and_diagnose(include_str!("fixtures/structural.json"), 7, 1, 1, 1);
    let initial: serde_json::Value = serde_json::from_str(&lines[1]).unwrap();
    let final_sample: serde_json::Value = serde_json::from_str(lines.last().unwrap()).unwrap();
    for cohort in ["evolving", "random_control"] {
        assert_eq!(final_sample["data"][cohort]["population"], 2);
        assert_eq!(final_sample["data"][cohort]["descendants"], 1);
        for operator in [
            "remove_connection",
            "remove_neuron",
            "toggle_connection",
            "add_connection",
            "add_neuron",
        ] {
            assert_eq!(
                initial["data"][cohort]["structural_mutations"][operator]["attempted"],
                0
            );
            let counts = &final_sample["data"][cohort]["structural_mutations"][operator];
            assert_eq!(counts["attempted"], 1, "{cohort}/{operator}");
            assert_eq!(counts["applied"], 1, "{cohort}/{operator}");
        }
        assert!(
            final_sample["data"][cohort]["spawn_failures"]
                .as_object()
                .unwrap()
                .values()
                .all(|value| value.as_u64() == Some(0))
        );
    }
    assert!(
        !report["unavailable"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason.as_str().unwrap().contains("structural-mutation"))
    );
}

#[test]
fn founder_storage_undersupply_is_an_error_with_or_without_metrics() {
    let params = temporary("undersupplied-params.json");
    fs::write(
        &params,
        r#"{"world":{"max_agents":2},"storage":{"genes_per_slot":142}}"#,
    )
    .unwrap();
    for metrics in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_native"));
        command
            .args(["--ticks", "0", "--founders", "2", "--params"])
            .arg(&params);
        if metrics {
            command.args(["--metrics", "-"]);
        }
        let output = command.output().expect("run native shell");
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(
            error.contains("requested 2 founders but world capacity allowed only 1"),
            "{error}"
        );
        assert!(!error.contains("completed"), "{error}");
        if metrics {
            assert!(error.contains("Genes storage"), "{error}");
        }
    }
    fs::remove_file(params).unwrap();
}

#[test]
fn plain_run_prints_a_summary_without_streaming_metrics() {
    let output = Command::new(env!("CARGO_BIN_EXE_native"))
        .args([
            "--ticks",
            "0",
            "--founders",
            "1",
            "--params",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/sustaining.json"
            ),
        ])
        .output()
        .expect("run native shell");
    assert!(
        output.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty(), "plain run emitted metrics JSONL");
    let summary = String::from_utf8(output.stderr).expect("summary UTF-8");
    assert!(summary.contains("completed 0 ticks: evolving=1"));
    assert!(summary.contains("control=1"));
}

#[test]
fn optional_collection_preserves_the_run_and_its_final_hashes() {
    let run = |collect, fixture| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_native"));
        command.args([
            "--seed",
            "7",
            "--ticks",
            "100",
            "--founders",
            "8",
            "--params",
            fixture,
        ]);
        if collect {
            command.args(["--metrics", "-"]);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    for fixture in [
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sustaining.json"
        ),
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/structural.json"
        ),
    ] {
        let plain = run(false, fixture);
        let observed = run(true, fixture);
        assert!(plain.stdout.is_empty());
        assert!(!observed.stdout.is_empty());
        assert_eq!(
            plain.stderr, observed.stderr,
            "collection changed final population or hashes for {fixture}"
        );
    }
}

#[test]
fn metrics_stdout_remains_machine_readable() {
    let output = Command::new(env!("CARGO_BIN_EXE_native"))
        .args([
            "--ticks",
            "0",
            "--founders",
            "1",
            "--params",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/sustaining.json"
            ),
            "--metrics",
            "-",
        ])
        .output()
        .expect("run native shell");
    assert!(
        output.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lines: Vec<_> = output.stdout.split(|byte| *byte == b'\n').collect();
    assert_eq!(lines.len(), 3, "header, final sample, and trailing newline");
    serde_json::from_slice::<serde_json::Value>(lines[0]).expect("header JSON");
    serde_json::from_slice::<serde_json::Value>(lines[1]).expect("sample JSON");
    assert!(
        String::from_utf8(output.stderr)
            .expect("summary UTF-8")
            .contains("completed 0 ticks")
    );
}

#[test]
fn unsafe_params_are_reported_before_a_run_starts() {
    for (json, message) in [
        (
            r#"{"mutation":{"weight_limit":-1}}"#,
            "mutation bounds and perturbation scales",
        ),
        (
            r#"{"chemo":{"cells":[4294967295,4294967295,1],"decay":[0.98,0.5]}}"#,
            "chemo grid times channels",
        ),
        (
            r#"{"storage":{"max_memory_bytes":1}}"#,
            "storage.max_memory_bytes",
        ),
    ] {
        let params = temporary("invalid-params.json");
        fs::write(&params, json).expect("write params");
        let output = Command::new(env!("CARGO_BIN_EXE_native"))
            .args([
                "--ticks",
                "0",
                "--founders",
                "1",
                "--metrics",
                "-",
                "--params",
            ])
            .arg(&params)
            .output()
            .expect("run native shell");
        fs::remove_file(params).expect("remove params");

        assert_eq!(
            output.status.code(),
            Some(1),
            "invalid input must not panic"
        );
        assert!(output.stdout.is_empty(), "invalid run emitted metrics");
        let error = String::from_utf8(output.stderr).expect("error UTF-8");
        assert!(error.contains(message), "unexpected error: {error}");
    }
}
