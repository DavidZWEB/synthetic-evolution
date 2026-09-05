//! Embeds the source revision in metrics headers.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    watch_git_head();
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    println!(
        "cargo:rerun-if-changed={}",
        manifest.join("../../sim-core/src").display()
    );
    for path in [
        manifest.join("src"),
        manifest.join("Cargo.toml"),
        manifest.join("../../Cargo.toml"),
        manifest.join("../../Cargo.lock"),
    ] {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let revision = std::env::var("GITHUB_SHA")
        .ok()
        .or_else(git_revision)
        .map(|revision| {
            if git_is_dirty() {
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

fn git_is_dirty() -> bool {
    Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .output()
        .is_ok_and(|output| output.status.success() && !output.stdout.is_empty())
}

fn git_revision() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn watch_git_head() {
    let output = match Command::new("git")
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
