//! End-to-end opt-in species capture, output isolation, and streaming readback.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "synthetic-evolution-history-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn params(&self, fixture: &str) -> PathBuf {
        let mut params: Value = serde_json::from_str(fixture).unwrap();
        params["species"] = json!({"capacity": 32, "threshold": 0.000001});
        let path = self.path("params.json");
        fs::write(&path, serde_json::to_vec(&params).unwrap()).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn command(params: &Path, seed: u64, ticks: u64, founders: u32) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_native"));
    command
        .args([
            "--seed",
            &seed.to_string(),
            "--ticks",
            &ticks.to_string(),
            "--founders",
            &founders.to_string(),
            "--sample-every",
            "5",
            "--params",
        ])
        .arg(params);
    command
}

fn success(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn summary(path: &Path) -> Value {
    let output = success(
        Command::new(env!("CARGO_BIN_EXE_native"))
            .arg("history")
            .arg(path)
            .arg("--json"),
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn rows(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn oversized_history_headers_fail_before_outputs_are_truncated() {
    let scratch = Scratch::new();
    let configuration = json!({
        "world": {"max_agents": 4},
        "plants": {"max_plants": 0},
        "chemo": {"cells": [1, 1, 1], "decay": vec![0.98; 220_000]},
    });
    let params = scratch.params(&configuration.to_string());
    let parsed: sim_core::SimParams = serde_json::from_slice(&fs::read(&params).unwrap()).unwrap();
    parsed
        .validate()
        .expect("valid core configuration, too large only for the history header");
    let metrics = scratch.path("metrics.jsonl");
    let history = scratch.path("history.jsonl");
    fs::write(&metrics, b"preserve metrics").unwrap();
    fs::write(&history, b"preserve history").unwrap();
    let output = command(&params, 7, 0, 1)
        .arg("--metrics")
        .arg(&metrics)
        .arg("--history")
        .arg(&history)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("history record exceeds 1 MiB"));
    assert_eq!(fs::read(metrics).unwrap(), b"preserve metrics");
    assert_eq!(fs::read(history).unwrap(), b"preserve history");
}

fn without_history(samples: &[Value]) -> Vec<Value> {
    let mut samples = samples.to_vec();
    for sample in &mut samples[1..] {
        for cohort in ["evolving", "random_control"] {
            sample["data"][cohort]
                .as_object_mut()
                .unwrap()
                .remove("history");
        }
    }
    samples
}

fn diagnose(metrics: &Path) -> Value {
    let output = success(
        Command::new(env!("CARGO_BIN_EXE_native"))
            .arg("diagnose")
            .arg(metrics)
            .arg("--json"),
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn representatives_are_observational_and_archived_for_every_origin() {
    let scratch = Scratch::new();
    let params = scratch.params(include_str!("fixtures/structural.json"));
    for seed in [7, 42] {
        let metrics = scratch.path("baseline.jsonl");
        let history = scratch.path("history.jsonl");
        let baseline = success(
            command(&params, seed, 12, 4)
                .arg("--metrics")
                .arg(&metrics)
                .arg("--history")
                .arg(&history),
        );
        let expected_metrics = fs::read(&metrics).unwrap();
        let events_only = rows(&history);
        let observed = success(
            command(&params, seed, 12, 4)
                .arg("--metrics")
                .arg(&metrics)
                .arg("--history")
                .arg(&history)
                .arg("--representatives"),
        );
        assert_eq!(observed.stderr, baseline.stderr, "final hashes unchanged");
        assert_eq!(fs::read(&metrics).unwrap(), expected_metrics);
        let archived = rows(&history);
        assert_eq!(archived[0]["data"]["schema_version"], 3);
        assert_eq!(archived[0]["data"]["representative_genes"], 65_536);
        assert_eq!(archived.len(), events_only.len());
        let mut origins = 0;
        for (with, without) in archived.iter().zip(&events_only).skip(1) {
            if with["kind"] != "event" {
                continue;
            }
            let mut stripped = with.clone();
            let representative = stripped["data"]
                .as_object_mut()
                .unwrap()
                .remove("representative");
            assert_eq!(stripped, *without, "events themselves are unchanged");
            if with["data"]["event"]["kind"] == "species_origin" {
                origins += 1;
                let representative = representative.expect("every origin has one");
                assert_eq!(representative["status"], "recorded");
                assert!(!representative["genes"].as_array().unwrap().is_empty());
            } else {
                assert!(representative.is_none());
            }
        }
        assert!(origins > 2, "the structural fixture creates species");
        let report = summary(&history);
        for cohort in report["cohorts"].as_array().unwrap() {
            assert_eq!(
                cohort["counts"]["representatives"],
                cohort["counts"]["origins"]
            );
            assert_eq!(cohort["counts"]["unavailable_representatives"], "0");
        }
        let human = success(
            Command::new(env!("CARGO_BIN_EXE_native"))
                .arg("history")
                .arg(&history),
        );
        assert!(
            String::from_utf8(human.stdout)
                .unwrap()
                .contains("representatives archived, 0 unavailable")
        );
    }
}

#[test]
fn capture_is_observational_across_seeds_cohorts_and_overflow() {
    let scratch = Scratch::new();
    let params = scratch.params(include_str!("fixtures/structural.json"));
    for seed in [7, 42, 99] {
        let metrics = scratch.path("baseline.jsonl");
        let baseline = success(command(&params, seed, 8, 4).arg("--metrics").arg(&metrics));
        let samples = rows(&metrics);
        let last = &samples.last().unwrap()["data"];
        let plain = success(&mut command(&params, seed, 8, 4));
        assert_eq!(plain.stderr, baseline.stderr);
        for capacity in [1, 4096] {
            let history = scratch.path("history.jsonl");
            let observed = success(
                command(&params, seed, 8, 4)
                    .arg("--metrics")
                    .arg(&metrics)
                    .arg("--history")
                    .arg(&history)
                    .arg("--history-capacity")
                    .arg(capacity.to_string()),
            );
            assert_eq!(observed.stderr, baseline.stderr);
            // Capture adds only its own availability counts; every other observation
            // must match the uncaptured run.
            let observed_samples = rows(&metrics);
            assert_eq!(
                without_history(&observed_samples),
                without_history(&samples),
                "seed {seed}, capacity {capacity}"
            );
            let observed_last = &observed_samples.last().unwrap()["data"];
            let report = summary(&history);
            for (index, cohort) in ["evolving", "random_control"].into_iter().enumerate() {
                let row = &report["cohorts"][index];
                assert_eq!(row["cohort"], cohort);
                assert_eq!(last[cohort]["history"], Value::Null);
                let availability = &observed_last[cohort]["history"];
                assert_eq!(availability["capacity"], capacity);
                for (metric, count) in [
                    ("retained_events", "events"),
                    ("dropped_events", "dropped_events"),
                    ("gaps", "gaps"),
                ] {
                    assert_eq!(
                        availability[metric].as_u64().unwrap().to_string(),
                        row["counts"][count],
                        "{cohort} {metric} matches the archive"
                    );
                }
                assert_eq!(row["final_state_hash"], last["final_state_hashes"][cohort]);
                if capacity == 4096 {
                    assert_eq!(row["history_complete"], true);
                    assert_eq!(
                        row["counts"]["origins"],
                        last[cohort]["species"]["events"]["created"]
                            .as_u64()
                            .unwrap()
                            .to_string()
                    );
                    assert_eq!(
                        row["counts"]["extinctions"],
                        last[cohort]["species"]["events"]["extinct"]
                            .as_u64()
                            .unwrap()
                            .to_string()
                    );
                } else {
                    assert_eq!(row["history_complete"], false);
                    assert!(
                        row["counts"]["dropped_events"]
                            .as_str()
                            .unwrap()
                            .parse::<u64>()
                            .unwrap()
                            > 0
                    );
                }
            }
            let diagnosis = diagnose(&metrics);
            for cohort in ["evolving", "random_control"] {
                let status = &diagnosis["history"][cohort];
                assert_eq!(
                    status["status"],
                    if capacity == 4096 {
                        "complete"
                    } else {
                        "incomplete"
                    }
                );
                assert_eq!(status["tick"], 8);
            }
            assert_eq!(
                diagnosis["evolving"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|finding| finding["code"] == "history_gaps"),
                capacity == 1
            );
            let history_only = success(
                command(&params, seed, 8, 4)
                    .arg("--history")
                    .arg(&history)
                    .arg("--history-capacity")
                    .arg(capacity.to_string()),
            );
            assert_eq!(history_only.stderr, plain.stderr);
            assert_eq!(summary(&history), report);
        }
    }
}

#[test]
fn actual_origins_include_only_founding_parent_references_and_extinctions_are_observed() {
    let scratch = Scratch::new();
    let params = scratch.params(include_str!("fixtures/structural.json"));
    let history = scratch.path("history.jsonl");
    success(command(&params, 7, 1, 1).arg("--history").arg(&history));
    let records = rows(&history);
    for cohort in ["evolving", "random_control"] {
        let events: Vec<_> = records
            .iter()
            .filter(|row| row["kind"] == "event" && row["data"]["cohort"] == cohort)
            .map(|row| &row["data"]["event"])
            .collect();
        assert_eq!(events.len(), 2, "{cohort}");
        assert_eq!(events[0]["kind"], "species_origin");
        assert_eq!(events[0]["founder_birth_id"], "0");
        assert_eq!(events[0]["parent_a"], json!({"status":"absent"}));
        assert_eq!(events[0]["parent_b"], json!({"status":"absent"}));
        assert_eq!(events[1]["kind"], "species_origin");
        assert_eq!(events[1]["founder_birth_id"], "1");
        assert_eq!(
            events[1]["parent_a"],
            json!({"status":"observed","birth_id":"0","species_id":0})
        );
        assert_eq!(events[1]["parent_b"], json!({"status":"absent"}));
    }
    summary(&history);

    let params = scratch.params(include_str!("fixtures/extinction.json"));
    success(
        command(&params, 7, 20, 4)
            .arg("--history")
            .arg(&history)
            .arg("--history-capacity")
            .arg("1"),
    );
    let records = rows(&history);
    for cohort in ["evolving", "random_control"] {
        let events: Vec<_> = records
            .iter()
            .filter(|row| row["data"]["cohort"] == cohort)
            .collect();
        assert_eq!(events.len(), 4);
        assert_eq!(events[0]["data"]["sequence"], "0");
        assert_eq!(events[1]["data"]["first_sequence"], "1");
        assert_eq!(events[1]["data"]["last_sequence"], "3");
        assert_eq!(events[2]["data"]["sequence"], "4");
        assert_eq!(events[2]["data"]["event"]["kind"], "species_extinct");
        assert_eq!(events[3]["data"]["first_sequence"], "5");
        assert_eq!(events[3]["data"]["last_sequence"], "7");
    }
    for cohort in summary(&history)["cohorts"].as_array().unwrap() {
        assert_eq!(cohort["counts"]["next_sequence"], "8");
        assert_eq!(cohort["counts"]["dropped_events"], "6");
        assert_eq!(cohort["history_complete"], false);
    }
}

#[test]
fn seeding_only_disabled_classification_and_stdout_streams_are_supported() {
    let scratch = Scratch::new();
    let params = scratch.params(include_str!("fixtures/sustaining.json"));
    let history = scratch.path("history.jsonl");
    let output = success(command(&params, u64::MAX, 0, 1).arg("--history").arg("-"));
    fs::write(&history, &output.stdout).unwrap();
    let report = summary(&history);
    assert_eq!(report["ticks"], "0");
    assert_eq!(report["provenance"]["seed"], u64::MAX.to_string());
    assert_eq!(report["cohorts"][0]["counts"]["origins"], "1");

    let metrics_stdout = success(
        command(&params, 7, 0, 1)
            .arg("--history")
            .arg(&history)
            .arg("--metrics")
            .arg("-"),
    );
    assert_eq!(
        std::str::from_utf8(&metrics_stdout.stdout)
            .unwrap()
            .lines()
            .count(),
        2
    );
    summary(&history);
    let metrics = scratch.path("metrics.jsonl");
    let history_stdout = success(
        command(&params, 7, 0, 1)
            .arg("--history")
            .arg("-")
            .arg("--metrics")
            .arg(&metrics),
    );
    fs::write(&history, &history_stdout.stdout).unwrap();
    summary(&history);

    let mut input = Command::new(env!("CARGO_BIN_EXE_native"))
        .args(["history", "-", "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    input
        .stdin
        .take()
        .unwrap()
        .write_all(&output.stdout)
        .unwrap();
    let output = input.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        report
    );

    let mut value: Value = serde_json::from_slice(&fs::read(&params).unwrap()).unwrap();
    value["species"]["capacity"] = json!(0);
    fs::write(&params, serde_json::to_vec(&value).unwrap()).unwrap();
    success(command(&params, 7, 2, 1).arg("--history").arg(&history));
    assert_eq!(rows(&history).len(), 2);
    for row in summary(&history)["cohorts"].as_array().unwrap() {
        assert_eq!(row["counts"]["next_sequence"], "0");
        assert_eq!(row["history_complete"], true);
    }
}

#[test]
fn failed_validation_and_admission_do_not_clobber_outputs() {
    let scratch = Scratch::new();
    let params = scratch.params(include_str!("fixtures/extinction.json"));
    let metrics = scratch.path("metrics.jsonl");
    let history = scratch.path("history.jsonl");
    fs::write(&metrics, "metrics sentinel").unwrap();
    fs::write(&history, "history sentinel").unwrap();
    for founders in [0, 9] {
        let output = command(&params, 7, 2, founders)
            .arg("--metrics")
            .arg(&metrics)
            .arg("--history")
            .arg(&history)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(fs::read_to_string(&metrics).unwrap(), "metrics sentinel");
        assert_eq!(fs::read_to_string(&history).unwrap(), "history sentinel");
    }
    fs::write(
        &params,
        half_founder_slot(r#"{"world":{"max_agents":2},"storage":{},"sensing":{"vision_rays":1}}"#),
    )
    .unwrap();
    for metrics_enabled in [false, true] {
        let mut command = command(&params, 7, 0, 2);
        command.arg("--history").arg(&history);
        if metrics_enabled {
            command.arg("--metrics").arg(&metrics);
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("requested 2 founders but world capacity allowed only 1")
        );
        assert_eq!(fs::read_to_string(&metrics).unwrap(), "metrics sentinel");
        assert_eq!(fs::read_to_string(&history).unwrap(), "history sentinel");
    }
    for contents in ["{", r#"{"world":{"max_agents":0}}"#] {
        fs::write(&params, contents).unwrap();
        let output = command(&params, 7, 2, 1)
            .arg("--metrics")
            .arg(&metrics)
            .arg("--history")
            .arg(&history)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(fs::read_to_string(&metrics).unwrap(), "metrics sentinel");
        assert_eq!(fs::read_to_string(&history).unwrap(), "history sentinel");
    }
}

#[test]
fn conflicting_paths_and_aliases_are_rejected_before_truncation() {
    let scratch = Scratch::new();
    let params = scratch.params(include_str!("fixtures/sustaining.json"));
    let target = scratch.path("target.jsonl");
    let hard_link = scratch.path("hard-link.jsonl");
    fs::write(&target, "sentinel").unwrap();
    fs::hard_link(&target, &hard_link).unwrap();
    let mut aliases = vec![target.clone(), scratch.path("./target.jsonl")];
    aliases.push(hard_link);
    #[cfg(unix)]
    {
        let link = scratch.path("symlink.jsonl");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        aliases.push(link);
    }
    for alias in aliases {
        let output = command(&params, 7, 1, 1)
            .arg("--metrics")
            .arg(&target)
            .arg("--history")
            .arg(alias)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("distinct outputs"));
        assert_eq!(fs::read_to_string(&target).unwrap(), "sentinel");
    }
    let new = scratch.path("new.jsonl");
    let output = command(&params, 7, 1, 1)
        .arg("--metrics")
        .arg(&new)
        .arg("--history")
        .arg(scratch.path("./new.jsonl"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!new.exists());
    let original = fs::read(&params).unwrap();
    for flag in ["--metrics", "--history"] {
        let output = command(&params, 7, 1, 1)
            .arg(flag)
            .arg(&params)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(fs::read(&params).unwrap(), original);
    }
    let params_alias = scratch.path("params-alias.json");
    fs::hard_link(&params, &params_alias).unwrap();
    let output = command(&params, 7, 1, 1)
        .arg("--history")
        .arg(&params_alias)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read(&params).unwrap(), original);
    let output = command(&params, 7, 1, 1)
        .args(["--metrics", "-", "--history", "-"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[cfg(unix)]
#[test]
fn stdout_aliases_and_new_case_insensitive_aliases_cannot_mix_archives() {
    use std::os::unix::fs::MetadataExt;

    let scratch = Scratch::new();
    let params = scratch.params(include_str!("fixtures/sustaining.json"));
    let output = command(&params, 7, 0, 1)
        .args(["--metrics", "-", "--history", "/dev/stdout"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());

    let lower = scratch.path("case.jsonl");
    let upper = scratch.path("CASE.jsonl");
    let output = command(&params, 7, 0, 1)
        .arg("--metrics")
        .arg(&lower)
        .arg("--history")
        .arg(&upper)
        .output()
        .unwrap();
    let a = fs::metadata(&lower).unwrap();
    let b = fs::metadata(&upper).unwrap();
    if a.dev() == b.dev() && a.ino() == b.ino() {
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("distinct outputs"));
        assert_eq!(a.len(), 0);
    } else {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        summary(&upper);
    }
}

#[test]
fn open_and_broken_pipe_errors_are_command_failures() {
    let scratch = Scratch::new();
    let params = scratch.params(include_str!("fixtures/sustaining.json"));
    let metrics = scratch.path("metrics.jsonl");
    fs::write(&metrics, "sentinel").unwrap();
    let output = command(&params, 7, 1, 1)
        .arg("--metrics")
        .arg(&metrics)
        .arg("--history")
        .arg(&scratch.0)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read_to_string(&metrics).unwrap(), "sentinel");

    let mut child = command(&params, 7, 1, 1)
        .args(["--history", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("error:"));

    let missing = scratch.path("missing.jsonl");
    let output = Command::new(env!("CARGO_BIN_EXE_native"))
        .arg("history")
        .arg(&missing)
        .output()
        .unwrap();
    assert!(!output.status.success());
    fs::write(&missing, r#"{"kind":"header""#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_native"))
        .arg("history")
        .arg(&missing)
        .output()
        .unwrap();
    assert!(!output.status.success());
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
