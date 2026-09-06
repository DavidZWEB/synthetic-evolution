//! Embeds the native experiment source revision in metrics headers.
//!
//! The dirty suffix covers runtime Rust sources, manifests, the lockfile, and the
//! pinned toolchain. UI, documentation, and test-only edits do not change the binary
//! whose run the header identifies.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const REVISION_INPUTS: &[&str] = &[
    "rust-toolchain.toml",
    "Cargo.toml",
    "Cargo.lock",
    "sim-core/Cargo.toml",
    "sim-core/src",
    "shells/native/Cargo.toml",
    "shells/native/build.rs",
    "shells/native/src",
];

fn main() {
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let workspace = manifest.join("../..");
    watch_git_head(&workspace);
    for path in REVISION_INPUTS {
        println!("cargo:rerun-if-changed={}", workspace.join(path).display());
    }

    let revision = std::env::var("GITHUB_SHA")
        .ok()
        .or_else(|| git_revision(&workspace))
        .map(|revision| {
            if git_is_dirty(&workspace) {
                format!("{revision}-dirty")
            } else {
                revision
            }
        });
    println!(
        "cargo:rustc-env=SYNTHETIC_EVOLUTION_REVISION={}",
        revision.as_deref().unwrap_or("unknown")
    );
}

fn git_is_dirty(workspace: &Path) -> bool {
    let mut command = Command::new("git");
    command.arg("-C").arg(workspace).args([
        "status",
        "--porcelain",
        "--untracked-files=normal",
        "--",
    ]);
    command.args(REVISION_INPUTS);
    command
        .output()
        .is_ok_and(|output| output.status.success() && !output.stdout.is_empty())
}

fn git_revision(workspace: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn watch_git_head(workspace: &Path) {
    let output = match Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(["rev-parse", "--absolute-git-dir"])
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return,
    };
    let git_dir = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let head = git_dir.join("HEAD");
    println!("cargo:rerun-if-changed={}", head.display());
    println!("cargo:rerun-if-changed={}", git_dir.join("index").display());
    if let Ok(contents) = fs::read_to_string(&head)
        && let Some(reference) = contents.strip_prefix("ref: ")
    {
        println!(
            "cargo:rerun-if-changed={}",
            git_dir.join(reference.trim()).display()
        );
    }
}
