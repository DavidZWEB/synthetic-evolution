//! Mechanical guard for the `sim-core` invariants that fail *silently*.
//!
//! A platform `sin`, a `thread_rng`, or a stray `HashMap` produces a simulation that
//! runs, looks plausible, and diverges between native and WASM weeks later. Nothing
//! else in the test suite catches those, because each one passes every single-target
//! test. This scans the crate source instead.
//!
//! Deliberately not a substitute for the golden hash — it catches the causes it knows
//! about, not the effects. Suppress a knowing violation with a trailing
//! `// allow-invariant: <reason>`.

use std::fs;
use std::path::{Path, PathBuf};

/// `(needle, why it is banned)`. Matched against source with line comments stripped.
const BANNED: &[(&str, &str)] = &[
    // Invariant 1 — determinism.
    (
        "thread_rng",
        "seeded PRNG only (rand_pcg); thread_rng is unseedable",
    ),
    (
        "HashMap",
        "iteration order is nondeterministic; use sorted Vec or agent-index order (spec §2.4)",
    ),
    (
        "HashSet",
        "iteration order is nondeterministic; use a sorted Vec (spec §2.4)",
    ),
    (
        "SystemTime",
        "wall-clock time is I/O and is not reproducible",
    ),
    // Invariant 1 — platform transcendentals differ between native and WASM (spec §7.4).
    (
        ".sin(",
        "use libm via crate::math; platform sin differs across targets",
    ),
    (
        ".cos(",
        "use libm via crate::math; platform cos differs across targets",
    ),
    (
        ".tan(",
        "use libm via crate::math; platform tan differs across targets",
    ),
    (".asin(", "use libm via crate::math"),
    (".acos(", "use libm via crate::math"),
    (".atan(", "use libm via crate::math"),
    (".atan2(", "use libm via crate::math"),
    (".exp(", "use libm via crate::math"),
    (".ln(", "use libm via crate::math"),
    (".log(", "use libm via crate::math"),
    (".log2(", "use libm via crate::math"),
    (".log10(", "use libm via crate::math"),
    (".powf(", "use libm via crate::math"),
    (".cbrt(", "use libm via crate::math"),
    (".hypot(", "use libm via crate::math"),
    (".tanh(", "use libm via crate::math"),
    // Invariant 2 — no I/O.
    ("std::fs", "sim-core does no I/O; the shells own it"),
    ("std::net", "sim-core does no I/O; the shells own it"),
    ("std::io", "sim-core does no I/O; the shells own it"),
    ("std::time", "sim-core does no I/O and reads no clock"),
    (
        "std::env",
        "sim-core does no I/O; every tunable is SimParams",
    ),
    ("std::process", "sim-core does no I/O; the shells own it"),
    (
        "println!",
        "sim-core does not log; return data and let the shell print it",
    ),
    (
        "eprintln!",
        "sim-core does not log; return data and let the shell print it",
    ),
    (
        "dbg!",
        "debug printing is I/O and must not survive into a commit",
    ),
    // Invariant 3 — no static mutable state.
    (
        "static mut",
        "a process must be able to hold several worlds (spec §7.2)",
    ),
    (
        "thread_local!",
        "a process must be able to hold several worlds (spec §7.2)",
    ),
    (
        "lazy_static",
        "a process must be able to hold several worlds (spec §7.2)",
    ),
    (
        "OnceLock",
        "a process must be able to hold several worlds (spec §7.2)",
    ),
    (
        "OnceCell",
        "a process must be able to hold several worlds (spec §7.2)",
    ),
];

/// Everything after a `//` is prose, and prose is allowed to name what it forbids.
/// Crude about string literals containing `//`, which this crate has no reason to hold.
fn strip_comment(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).expect("sim-core/src is readable");
    // Sorted so a failure report is stable between runs.
    let mut paths: Vec<PathBuf> = entries.map(|e| e.expect("dir entry").path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn sim_core_source_holds_the_invariants() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_sources(&src, &mut files);
    assert!(
        !files.is_empty(),
        "found no sources under {}",
        src.display()
    );

    let mut violations = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).expect("source file is utf-8");
        for (n, line) in text.lines().enumerate() {
            if line.contains("allow-invariant") {
                continue;
            }
            let code = strip_comment(line);
            for (needle, why) in BANNED {
                if code.contains(needle) {
                    violations.push(format!(
                        "{}:{}: `{}` — {}\n    {}",
                        file.display(),
                        n + 1,
                        needle,
                        why,
                        code.trim()
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "sim-core invariant violations ({}):\n\n{}\n",
        violations.len(),
        violations.join("\n")
    );
}

/// The scanner is only worth having if it actually matches. Guards against a future
/// refactor that quietly makes `strip_comment` eat every line.
#[test]
fn scanner_detects_a_violation() {
    let code = strip_comment("    let x = y.sin(); // not a comment match");
    assert!(code.contains(".sin("));
    assert!(!strip_comment("// .sin( in prose is fine").contains(".sin("));
}
