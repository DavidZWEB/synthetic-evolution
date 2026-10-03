//! End-to-end saved runs: save, inspect, resume, and refuse bad bundles.
//!
//! A resumed run must end where an uninterrupted run of the same seed ends, for
//! both paired cohorts, with the restored history kept and the continuation marked.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "synthetic-evolution-saved-run-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// Structural births, deaths, and species churn, as in the core checkpoint case.
    fn params(&self) -> PathBuf {
        let mut params: Value =
            serde_json::from_str(include_str!("fixtures/structural.json")).unwrap();
        params["metabolism"]["base"] = json!(0.3);
        params["species"] = json!({"capacity": 32, "threshold": 0.05});
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

fn native() -> Command {
    Command::new(env!("CARGO_BIN_EXE_native"))
}

fn run(params: &Path, ticks: u64) -> Command {
    let mut command = native();
    command
        .args([
            "--seed",
            "7",
            "--founders",
            "8",
            "--sample-every",
            "10",
            "--ticks",
        ])
        .arg(ticks.to_string())
        .arg("--params")
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

fn failure(command: &mut Command) -> String {
    let output = command.output().unwrap();
    assert!(!output.status.success(), "command unexpectedly succeeded");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn manifest(bundle: &Path) -> Value {
    let output = success(native().arg("saved-run").arg(bundle).arg("--json"));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn hashes(manifest: &Value) -> Vec<Value> {
    manifest["cohorts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["state_hash"].clone())
        .collect()
}

#[test]
fn resumed_paired_runs_end_where_uninterrupted_runs_end() {
    let scratch = Scratch::new();
    let params = scratch.params();
    let (saved, resumed, straight) = (
        scratch.path("saved.sevrun"),
        scratch.path("resumed.sevrun"),
        scratch.path("straight.sevrun"),
    );
    let history = scratch.path("history.jsonl");
    success(
        run(&params, 150)
            .arg("--history")
            .arg(&history)
            .arg("--save-run")
            .arg(&saved),
    );
    success(
        native()
            .arg("resume")
            .arg(&saved)
            .args(["--ticks", "150", "--save-run"])
            .arg(&resumed),
    );
    success(run(&params, 300).arg("--save-run").arg(&straight));

    let (saved, resumed, straight) = (manifest(&saved), manifest(&resumed), manifest(&straight));
    assert_eq!(resumed["tick"], "300");
    assert_eq!(
        hashes(&resumed),
        hashes(&straight),
        "both cohorts continue exactly"
    );
    assert_ne!(
        hashes(&resumed),
        hashes(&saved),
        "the continuation actually advanced"
    );
    assert_eq!(
        resumed["provenance"], saved["provenance"],
        "origin provenance is kept"
    );
    let cohorts: Vec<_> = resumed["cohorts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["cohort"].clone())
        .collect();
    assert_eq!(cohorts, [json!("evolving"), json!("random_control")]);
    // The restored prefix is kept as saved; the continuation is its own segment.
    assert_eq!(
        resumed["history"],
        json!([
            {"status": "included", "starts_at": "0", "bytes": saved["history"][0]["bytes"]},
            {"status": "unavailable", "starts_at": "150", "reason": "not_recorded"},
        ])
    );
    assert_eq!(
        saved["history"][0]["bytes"],
        fs::metadata(&history).unwrap().len().to_string()
    );
    assert_eq!(
        straight["history"],
        json!([{"status": "unavailable", "starts_at": "0", "reason": "not_recorded"}])
    );

    let human = success(
        native()
            .arg("saved-run")
            .arg(scratch.path("resumed.sevrun")),
    );
    let text = String::from_utf8(human.stdout).unwrap();
    assert!(text.contains("saved run at tick 300 from seed 7"));
    assert!(text.contains("history from tick 150: unavailable (NotRecorded)"));
}

#[test]
fn stdout_history_is_marked_not_retained_and_representatives_survive() {
    let scratch = Scratch::new();
    let params = scratch.params();
    let streamed = scratch.path("streamed.sevrun");
    success(
        run(&params, 20)
            .args(["--history", "-", "--save-run"])
            .arg(&streamed),
    );
    assert_eq!(
        manifest(&streamed)["history"],
        json!([{"status": "unavailable", "starts_at": "0", "reason": "not_retained"}])
    );
    let kept = scratch.path("kept.sevrun");
    success(
        run(&params, 20)
            .arg("--history")
            .arg(scratch.path("h.jsonl"))
            .args(["--representatives", "--save-run"])
            .arg(&kept),
    );
    assert_eq!(manifest(&kept)["history"][0]["status"], "included");
}

#[test]
fn bad_bundles_and_unsafe_outputs_are_refused() {
    let scratch = Scratch::new();
    let params = scratch.params();
    let saved = scratch.path("saved.sevrun");
    success(run(&params, 20).arg("--save-run").arg(&saved));
    let bytes = fs::read(&saved).unwrap();

    for (name, corrupt) in [
        ("truncated", bytes[..bytes.len() - 1].to_vec()),
        ("trailing", [bytes.as_slice(), &[0]].concat()),
        ("magic", [&b"X"[..], &bytes[1..]].concat()),
        ("checkpoint byte", {
            let mut b = bytes.clone();
            let last = b.len() - 1;
            b[last] ^= 0xff;
            b
        }),
    ] {
        let path = scratch.path(&format!("{name}.sevrun"));
        fs::write(&path, corrupt).unwrap();
        failure(native().arg("saved-run").arg(&path));
        failure(native().arg("resume").arg(&path).args(["--ticks", "1"]));
    }
    let error = failure(
        native()
            .arg("saved-run")
            .arg(&saved)
            .args(["--max-core-bytes", "1"]),
    );
    assert!(error.contains("memory budget"), "{error}");

    assert!(failure(run(&params, 1).args(["--save-run", "-"])).contains("not stdout"));
    let metrics = scratch.path("metrics.jsonl");
    assert!(
        failure(
            run(&params, 1)
                .arg("--metrics")
                .arg(&metrics)
                .arg("--save-run")
                .arg(&metrics)
        )
        .contains("must not reuse")
    );
    assert!(failure(run(&params, 1).arg("--save-run").arg(&params)).contains("--params"));
}

fn rows(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Event content without per-segment sequence numbers.
fn events_from(rows: &[Value], tick: u64) -> Vec<Value> {
    rows.iter()
        .filter(|row| row["kind"] == "event")
        .filter(|row| {
            row["data"]["tick"]
                .as_str()
                .unwrap()
                .parse::<u64>()
                .unwrap()
                >= tick
        })
        .map(|row| {
            let mut data = row["data"].clone();
            data.as_object_mut().unwrap().remove("sequence");
            data
        })
        .collect()
}

#[test]
fn resumed_history_is_a_distinct_segment_matching_the_uninterrupted_run() {
    let scratch = Scratch::new();
    let params = scratch.params();
    let (first, continued, straight) = (
        scratch.path("first.jsonl"),
        scratch.path("continued.jsonl"),
        scratch.path("straight.jsonl"),
    );
    let (saved, resumed) = (scratch.path("saved.sevrun"), scratch.path("resumed.sevrun"));
    success(
        run(&params, 150)
            .arg("--history")
            .arg(&first)
            .args(["--representatives", "--save-run"])
            .arg(&saved),
    );
    success(
        native()
            .arg("resume")
            .arg(&saved)
            .args(["--ticks", "150", "--drain-every", "10", "--history"])
            .arg(&continued)
            .args(["--representatives", "--save-run"])
            .arg(&resumed),
    );
    success(
        run(&params, 300)
            .arg("--history")
            .arg(&straight)
            .arg("--representatives"),
    );

    let segment = rows(&continued);
    assert_eq!(segment[0]["data"]["schema_version"], 4);
    assert_eq!(segment[0]["data"]["resumed_from_tick"], "150");
    assert_eq!(segment[0]["data"]["ticks"], "300");
    let expected = events_from(&rows(&straight), 150);
    assert!(
        !expected.is_empty(),
        "the continuation has species events to compare"
    );
    assert_eq!(
        events_from(&segment, 0),
        expected,
        "same events as the uninterrupted run"
    );
    assert_eq!(
        segment[1]["data"]["sequence"], "0",
        "a segment numbers its own events"
    );

    let manifest = manifest(&resumed);
    assert_eq!(manifest["provenance"]["founders"], 8);
    assert_eq!(manifest["history"][0]["status"], "included");
    assert_eq!(
        manifest["history"][1],
        json!({
            "status": "included",
            "starts_at": "150",
            "bytes": fs::metadata(&continued).unwrap().len().to_string(),
        })
    );
    let summary = success(native().arg("history").arg(&continued).arg("--json"));
    let summary: Value = serde_json::from_slice(&summary.stdout).unwrap();
    assert_eq!(summary["schema_version"], 4);
}

#[test]
fn resumed_segments_reject_events_outside_them_and_unsafe_outputs() {
    let scratch = Scratch::new();
    let params = scratch.params();
    let (saved, segment) = (scratch.path("saved.sevrun"), scratch.path("segment.jsonl"));
    success(run(&params, 100).arg("--save-run").arg(&saved));
    success(
        native()
            .arg("resume")
            .arg(&saved)
            .args(["--ticks", "100", "--history"])
            .arg(&segment),
    );
    let valid = rows(&segment);
    let event = valid
        .iter()
        .position(|row| row["kind"] == "event")
        .expect("the continuation records species events");
    let write = |rows: &[Value]| {
        let path = scratch.path("mutated.jsonl");
        let text: String = rows.iter().map(|row| format!("{row}\n")).collect();
        fs::write(&path, text).unwrap();
        path
    };
    for (name, mutate) in [
        (
            "event before the resume tick",
            Box::new(move |rows: &mut Vec<Value>| {
                rows[event]["data"]["tick"] = json!("99");
            }) as Box<dyn Fn(&mut Vec<Value>)>,
        ),
        (
            "schema four without a resume tick",
            Box::new(|rows: &mut Vec<Value>| {
                rows[0]["data"]
                    .as_object_mut()
                    .unwrap()
                    .remove("resumed_from_tick");
            }),
        ),
        (
            "resume tick on an ordinary archive",
            Box::new(|rows: &mut Vec<Value>| {
                rows[0]["data"]["schema_version"] = json!(1);
                let last = rows.len() - 1;
                rows[last]["data"]["schema_version"] = json!(1);
            }),
        ),
    ] {
        let mut rows = valid.clone();
        mutate(&mut rows);
        let output = native().arg("history").arg(write(&rows)).output().unwrap();
        assert!(!output.status.success(), "accepted {name}");
    }
    assert!(
        failure(
            native()
                .arg("resume")
                .arg(&saved)
                .args(["--ticks", "1", "--history"])
                .arg(&saved)
        )
        .contains("must not overwrite")
    );
}
