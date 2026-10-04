//! Native CLI contracts exercised through the compiled executable.

use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use sim_core::control::{RANDOMIZED_AT_BIRTH_PROTOCOL, STRUCTURAL_NULL_PROTOCOL};

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
    assert_eq!(header["data"]["schema_version"], 8);
    assert_eq!(header["data"]["control"], RANDOMIZED_AT_BIRTH_PROTOCOL);
    assert_eq!(header["data"]["params"]["species"]["capacity"], 256);
    assert_eq!(header["data"]["params"]["species"]["threshold"], 0.5);
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
            // Five neural, one oscillator, and two sensor operators.
            assert_eq!(mutations.len(), 8);
            for counts in mutations.values() {
                let counts = counts.as_object().unwrap();
                assert_eq!(counts.len(), 6);
                assert!(counts.values().all(|count| count.as_u64() == Some(0)));
            }
            // Small genomes cross the provisional species threshold within a few
            // births, so assert consistent bookkeeping rather than a species count.
            let species = &sample["data"][cohort]["species"];
            let rows = species["populations"].as_array().unwrap();
            let classified: u64 = rows
                .iter()
                .map(|row| row["population"].as_u64().unwrap())
                .sum();
            assert_eq!(
                serde_json::Value::from(classified),
                sample["data"][cohort]["population"]
            );
            assert_eq!(species["unclassified_population"], 0);
            let created = species["events"]["created"].as_u64().unwrap();
            let extinct = species["events"]["extinct"].as_u64().unwrap();
            assert!(created >= 1, "founder classification was observed");
            assert_eq!(created - extinct, rows.len() as u64);
            assert_eq!(species["events"]["unclassified_capacity"], 0);
            // The report describes the final sample only.
            assert!(
                report["species"][cohort]["active_species"]
                    .as_u64()
                    .unwrap()
                    >= 1
            );
            assert_eq!(report["species"][cohort]["species_capacity"], 256);
            assert_eq!(report["species"][cohort]["unclassified_population"], 0);
            assert_eq!(sample["data"][cohort]["history"], serde_json::Value::Null);
            let complexity = &sample["data"][cohort]["complexity"];
            for field in [
                "genome_genes",
                "neurons",
                "connections",
                "enabled_connections",
            ] {
                assert!(complexity[field]["min"].as_u64().unwrap() > 0, "{field}");
            }
            let brain_units = sample["data"][cohort]["brain_units"]["mean"]
                .as_f64()
                .unwrap();
            let neurons = complexity["neurons"]["mean"].as_f64().unwrap();
            let connections = complexity["connections"]["mean"].as_f64().unwrap();
            assert!((brain_units - (neurons + connections)).abs() < 1e-9);
        }
    }
    for cohort in ["evolving", "random_control"] {
        assert_eq!(report["history"][cohort]["status"], "off");
        assert_eq!(report["complexity"][cohort]["tick"], 10);
    }
    assert!(
        !report["unavailable"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason.as_str().unwrap().contains("species"))
    );
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
    let (lines, report) = run_and_diagnose(include_str!("fixtures/extinction.json"), 7, 20, 5, 4);
    assert!(
        report["evolving"]
            .as_array()
            .expect("evolving findings")
            .iter()
            .any(|finding| finding["code"] == "early_extinction")
    );
    let final_sample: serde_json::Value = serde_json::from_str(lines.last().unwrap()).unwrap();
    for cohort in ["evolving", "random_control"] {
        let species = &final_sample["data"][cohort]["species"];
        assert_eq!(species["populations"], serde_json::json!([]));
        assert_eq!(species["unclassified_population"], 0);
        assert!(species["events"]["created"].as_u64().unwrap() > 0);
        assert_eq!(species["events"]["created"], species["events"]["extinct"]);
        assert_eq!(report["species"][cohort]["active_species"], 0);
        assert_eq!(report["species"][cohort]["unclassified_population"], 0);
        assert!(
            !report[cohort]
                .as_array()
                .unwrap()
                .iter()
                .any(|finding| finding["code"].as_str().unwrap().starts_with("species_"))
        );
    }
}

#[test]
fn diagnose_finds_a_deliberately_collapsed_genome_population() {
    // This real no-mutation sweep depends on founder placement and composition. Seed 5
    // reaches a stable one-genome population with the minimal founder (a small base
    // metabolic cost supplies the turnover drift needs); if either changes, reselect
    // a deterministic collapsing seed rather than weakening the diagnostic assertion
    // or replacing the simulator-driven fixture.
    let (_, report) = run_and_diagnose(include_str!("fixtures/monoculture.json"), 5, 4_000, 50, 2);
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
    let params = half_founder_slot(
        r#"{
        "world":{"size":100.0,"max_agents":2},
        "storage":{},
        "sensing":{"vision_range":20.0,"chemo_radius":20.0,"vision_rays":1},
        "reproduction":{"start_energy":1.0,"threshold":1.1,"gate":0.0,"maturity_ticks":0},
        "feeding":{"rate":100.0,"gate":0.0,"reach":20.0},
        "plants":{"max_plants":100,"max_energy":100.0,"initial_fill":1.0},
        "metabolism":{"base":0.0,"k_size":0.0,"k_brain":0.0,"k_sensor":0.0,"k_move":0.0}
    }"#,
    );
    let (lines, report) = run_and_diagnose(&params, 7, 2, 1, 1);
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
fn classification_capacity_observes_founders_and_births_without_refusing_either() {
    for capacity in [0, 1] {
        let mut params: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/structural.json")).unwrap();
        params["species"] = serde_json::json!({"capacity": capacity, "threshold": 1e-12});
        let (lines, report) = run_and_diagnose(&params.to_string(), 7, 1, 1, 1);
        let initial: serde_json::Value = serde_json::from_str(&lines[1]).unwrap();
        let final_sample: serde_json::Value = serde_json::from_str(lines.last().unwrap()).unwrap();
        for cohort in ["evolving", "random_control"] {
            let founder = &initial["data"][cohort]["species"];
            assert_eq!(founder["events"]["created"], capacity);
            assert_eq!(founder["events"]["unclassified_capacity"], 1 - capacity);
            assert_eq!(founder["unclassified_population"], 1 - capacity);
            let metrics = &final_sample["data"][cohort];
            assert_eq!(
                metrics["population"], 2,
                "classification must not deny births"
            );
            assert_eq!(metrics["descendants"], 1);
            assert_eq!(
                metrics["species"]["populations"].as_array().unwrap().len(),
                capacity as usize
            );
            assert_eq!(metrics["species"]["events"]["created"], capacity);
            assert_eq!(
                metrics["species"]["events"]["unclassified_capacity"],
                2 - capacity
            );
            assert_eq!(metrics["species"]["unclassified_population"], 2 - capacity);
            assert!(
                metrics["spawn_failures"]
                    .as_object()
                    .unwrap()
                    .values()
                    .all(|count| count.as_u64() == Some(0))
            );
            let findings = report[cohort].as_array().unwrap();
            let pressure = findings
                .iter()
                .find(|finding| finding["code"] == "species_capacity")
                .unwrap();
            assert!(
                pressure["signal"]
                    .as_str()
                    .unwrap()
                    .contains("not spawn refusals")
            );
            assert!(
                !findings
                    .iter()
                    .any(|finding| finding["code"] == "storage_capacity")
            );
        }
    }
}

#[test]
fn configured_sensor_edits_are_observed_with_sparse_no_eye_founders() {
    let mut params: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/structural.json")).unwrap();
    params["sensing"]["vision_rays"] = 0.into();
    params["sensing"]["chemo_sensors"] = 1.into();
    params["sensing"]["energy_sensors"] = 0.into();
    params["brain"]["hidden_neurons"] = 0.into();
    params["brain"]["oscillators"] = 0.into();
    params["brain"]["connections_per_target"] = 1.into();
    params["mutation"]["structural"] = serde_json::json!({});
    params["mutation"]["organs"] = serde_json::json!({
        "remove_sensor_rate": 1.0,
        "add_sensor_rate": 1.0,
        "vision_weight": 0.0,
        "chemo_weight": 1.0,
        "energy_weight": 0.0
    });
    let (lines, report) = run_and_diagnose(&params.to_string(), 7, 1, 1, 1);
    let header: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(header["data"]["schema_version"], 8);
    assert_eq!(header["data"]["control"], "randomized_at_birth_v3");
    let initial: serde_json::Value = serde_json::from_str(&lines[1]).unwrap();
    let final_sample: serde_json::Value = serde_json::from_str(lines.last().unwrap()).unwrap();
    for cohort in ["evolving", "random_control"] {
        assert_eq!(initial["data"][cohort]["genome_genes"]["mean"], 25.0);
        assert_eq!(final_sample["data"][cohort]["descendants"], 1);
        for operator in ["remove_sensor", "add_sensor"] {
            assert_eq!(
                initial["data"][cohort]["structural_mutations"][operator]["attempted"],
                0
            );
            let counts = &final_sample["data"][cohort]["structural_mutations"][operator];
            assert_eq!(counts["attempted"], 1);
            assert_eq!(counts["applied"], 1);
        }
    }
    assert!(
        !report["unavailable"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason.as_str().unwrap().contains("sensor-mutation"))
    );
}

#[test]
fn founder_storage_undersupply_is_an_error_with_or_without_metrics() {
    let params = temporary("undersupplied-params.json");
    fs::write(
        &params,
        half_founder_slot(r#"{"world":{"max_agents":2},"storage":{},"sensing":{"vision_rays":1}}"#),
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

#[test]
fn the_structural_null_control_writes_metrics_that_diagnose_names() {
    {
        let (control, protocol, label) = (
            "structural-null",
            STRUCTURAL_NULL_PROTOCOL,
            "structural null",
        );
        let metrics = temporary("null.jsonl");
        let run = Command::new(env!("CARGO_BIN_EXE_native"))
            .args(["--seed", "7", "--ticks", "10", "--sample-every", "5"])
            .args(["--founders", "4", "--control", control, "--metrics"])
            .arg(&metrics)
            .output()
            .expect("run native shell");
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        let text = fs::read_to_string(&metrics).expect("metrics file");
        let header: serde_json::Value =
            serde_json::from_str(text.lines().next().unwrap()).expect("header JSON");
        assert_eq!(header["data"]["control"], protocol);
        assert_eq!(diagnose_file(&metrics)["control_label"], label);
        fs::remove_file(metrics).expect("remove metrics");

        for extra in [["--history", "-"], ["--save-run", "null.sevrun"]] {
            let refused = Command::new(env!("CARGO_BIN_EXE_native"))
                .args(["--ticks", "1", "--control", control])
                .args(extra)
                .output()
                .expect("run native shell");
            assert!(!refused.status.success(), "{control} {extra:?} accepted");
            assert!(
                String::from_utf8_lossy(&refused.stderr).contains("metrics only"),
                "{}",
                String::from_utf8_lossy(&refused.stderr)
            );
        }
    }
}

fn metrics_run(seed: u64, control: &str) -> std::path::PathBuf {
    let params = temporary("summary-params.json");
    fs::write(&params, include_str!("fixtures/sustaining.json")).expect("write params");
    let metrics = temporary("summary.jsonl");
    let run = Command::new(env!("CARGO_BIN_EXE_native"))
        .args([
            "--seed",
            &seed.to_string(),
            "--ticks",
            "10",
            "--sample-every",
            "5",
        ])
        .args(["--founders", "4", "--control", control, "--params"])
        .arg(&params)
        .arg("--metrics")
        .arg(&metrics)
        .output()
        .expect("run native shell");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    fs::remove_file(params).expect("remove params");
    metrics
}

fn summarize(files: &[&std::path::PathBuf]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_native"))
        .arg("summarize")
        .args(files)
        .arg("--json")
        .output()
        .expect("summarize")
}

#[test]
fn summarize_pairs_seeds_and_reports_every_cohort_unranked() {
    let runs: Vec<_> = [(1, "scalar"), (1, "structural-null"), (2, "scalar")]
        .into_iter()
        .map(|(seed, control)| metrics_run(seed, control))
        .collect();
    let output = summarize(&runs.iter().collect::<Vec<_>>());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let configurations = summary["configurations"].as_array().unwrap();
    assert_eq!(configurations.len(), 1);
    let configuration = &configurations[0];
    assert_eq!(configuration["seeds"], serde_json::json!(["1", "2"]));
    assert_eq!(configuration["unpaired"], serde_json::json!(["2"]));
    let cohorts: Vec<_> = configuration["cohorts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|cohort| (cohort["name"].clone(), cohort["seeds"].clone()))
        .collect();
    assert_eq!(
        cohorts,
        [
            ("evolving".into(), serde_json::json!(["1", "2"])),
            ("scalar control".into(), serde_json::json!(["1", "2"])),
            ("structural null".into(), serde_json::json!(["1"])),
        ]
    );

    // A duplicate seed/control is not a second sample of the same configuration.
    let duplicate = summarize(&[&runs[0], &runs[0]]);
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("two runs with control"));
    for run in runs {
        fs::remove_file(run).expect("remove metrics");
    }
}

/// `params` with `genes_per_slot` set to half its founder's genes, so two slots hold
/// exactly one founder and a second is refused (spec section 2.2a).
fn half_founder_slot(params: &str) -> String {
    let mut value: serde_json::Value = serde_json::from_str(params).unwrap();
    let parsed: sim_core::SimParams = serde_json::from_value(value.clone()).unwrap();
    let founder = sim_core::World::new(1, parsed)
        .unwrap()
        .founder_plan()
        .len();
    assert_eq!(
        founder % 2,
        0,
        "an odd founder cannot fill two slots exactly"
    );
    value["storage"]["genes_per_slot"] = (founder / 2).into();
    value.to_string()
}

fn retune_run(
    retune: Option<(&str, &str)>,
    extra: &[&str],
) -> (std::process::Output, std::path::PathBuf) {
    let params = temporary("retune-params.json");
    fs::write(&params, include_str!("fixtures/sustaining.json")).expect("write params");
    let metrics = temporary("retune.jsonl");
    let mut command = Command::new(env!("CARGO_BIN_EXE_native"));
    command
        .args([
            "--seed",
            "7",
            "--ticks",
            "40",
            "--sample-every",
            "10",
            "--founders",
            "8",
        ])
        .arg("--params")
        .arg(&params)
        .arg("--metrics")
        .arg(&metrics);
    let file = retune.map(|(json, at)| {
        let file = temporary("retune.json");
        fs::write(&file, json).expect("write retune");
        command.arg("--retune").arg(&file).args(["--retune-at", at]);
        file
    });
    // "RETUNE" in the extra arguments stands for the retune file's own path.
    for arg in extra {
        match (*arg, &file) {
            ("RETUNE", Some(file)) => command.arg(file),
            _ => command.arg(arg),
        };
    }
    let output = command.output().expect("run native shell");
    fs::remove_file(params).expect("remove params");
    if let Some(file) = file {
        fs::remove_file(file).expect("remove retune");
    }
    (output, metrics)
}

fn samples(metrics: &std::path::Path) -> Vec<serde_json::Value> {
    fs::read_to_string(metrics)
        .expect("metrics file")
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSON line"))
        .collect()
}

#[test]
fn a_scheduled_retune_matches_the_plain_run_until_its_tick_then_diverges() {
    // Silencing food scent at tick 20: the founders' only sense of food.
    let knockout = r#"{"plants":{"scent_rate":0.0}}"#;
    let (plain, plain_metrics) = retune_run(None, &[]);
    let (retuned, retuned_metrics) = retune_run(Some((knockout, "20")), &[]);
    for output in [&plain, &retuned] {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let (plain, retuned) = (samples(&plain_metrics), samples(&retuned_metrics));
    let retune = &retuned[0]["data"]["retune"];
    assert_eq!(retune["at_tick"], 20);
    assert_eq!(retune["params"]["plants"]["scent_rate"], 0.0);
    assert_eq!(
        retune["params"]["plants"]["max_plants"],
        plain[0]["data"]["params"]["plants"]["max_plants"],
        "fields the retune did not name keep the run's values"
    );
    assert!(
        plain[0]["data"].get("retune").is_none(),
        "a plain run records no retune"
    );
    // Samples at ticks 0, 10, and 20 precede any effect of the retune.
    for index in 1..=3 {
        assert_eq!(
            plain[index], retuned[index],
            "sample {index} differs before the retune acted"
        );
    }
    let last = plain.len() - 1;
    assert_ne!(
        plain[last]["data"]["final_state_hashes"], retuned[last]["data"]["final_state_hashes"],
        "the retune changed nothing"
    );
    for path in [plain_metrics, retuned_metrics] {
        fs::remove_file(path).expect("remove metrics");
    }
}

#[test]
fn an_illegal_or_misplaced_retune_is_refused_before_the_run() {
    let cases: [(&str, &str, &[&str], &str); 4] = [
        (
            r#"{"plants":{"max_plants":7}}"#,
            "10",
            &[],
            "--retune: invalid SimParams: plants.max_plants is fixed",
        ),
        (
            r#"{"plants":{"scent_rate":0.0}}"#,
            "41",
            &[],
            "--retune-at must be at most --ticks",
        ),
        (
            r#"{"plants":{"scent_rate":0.0}}"#,
            "10",
            &["--history", "-"],
            "cannot be combined with --history",
        ),
        (
            r#"{"plants":{"scent_rate":0.0}}"#,
            "10",
            &["--save-run", "RETUNE"],
            "must not overwrite --retune input",
        ),
    ];
    for (json, at, extra, message) in cases {
        let (output, metrics) = retune_run(Some((json, at)), extra);
        assert!(
            !output.status.success(),
            "{json} at {at} {extra:?} was accepted"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(message), "{stderr}");
        let _ = fs::remove_file(metrics);
    }
}

#[test]
fn diagnose_refuses_a_retuned_run_and_readers_refuse_an_impossible_retune() {
    let (output, metrics) = retune_run(Some((r#"{"plants":{"scent_rate":0.0}}"#, "20")), &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let diagnosis = Command::new(env!("CARGO_BIN_EXE_native"))
        .arg("diagnose")
        .arg(&metrics)
        .output()
        .expect("diagnose");
    assert!(!diagnosis.status.success(), "diagnose read a retuned run");
    assert!(String::from_utf8_lossy(&diagnosis.stderr).contains("does not read retuned runs"));

    // Edit the recorded retune into two the run could not have applied.
    let text = fs::read_to_string(&metrics).expect("metrics");
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for (edit, message) in [
        (
            ("at_tick", serde_json::json!(41)),
            "after the run's last tick",
        ),
        (
            ("max_plants", serde_json::json!(7)),
            "not a legal live retune",
        ),
    ] {
        let mut edited = lines.clone();
        let retune = &mut edited[0]["data"]["retune"];
        match edit.0 {
            "at_tick" => retune["at_tick"] = edit.1,
            _ => retune["params"]["plants"]["max_plants"] = edit.1,
        }
        let path = temporary("bad-retune.jsonl");
        let body: Vec<String> = edited.iter().map(|value| value.to_string()).collect();
        fs::write(&path, body.join("\n") + "\n").expect("write edited metrics");
        let read = summarize(&[&path]);
        assert!(!read.status.success(), "an impossible retune was accepted");
        assert!(
            String::from_utf8_lossy(&read.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&read.stderr)
        );
        fs::remove_file(path).expect("remove edited metrics");
    }
    fs::remove_file(metrics).expect("remove metrics");
}
