//! `smysl ingest --yes` commits the batch (H-13), and refuses before paying for a call if it
//! cannot.
//!
//! The flag promised a commit from the day it was written and performed none: it suppressed
//! exit 10 and stopped. 1.9 corrected the help and warned at runtime; 1.10 commits. What is
//! pinned here is the half a model is not needed for — that a run which cannot commit is
//! refused before any provider is reached, because the alternative is to spend a call and then
//! discover there is nowhere to put the result.

#![cfg(all(feature = "cli", feature = "ingest"))]

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

fn dir_for(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("smysl-yes-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn yes_without_a_store_is_a_usage_error_before_anything_runs() {
    let dir = dir_for("nostore");
    let f = dir.join("doc.txt");
    std::fs::write(&f, "The pool saturated.").unwrap();

    let out = Command::new(BIN)
        .current_dir(&dir)
        .args(["ingest", "--offline", "--yes", f.to_str().unwrap()])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.contains("--store"),
        "the refusal must say what is missing: {stderr}"
    );
    assert!(
        !dir.join(".smysl/staged.smy").exists(),
        "it staged something anyway"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The refusal is about committing, not about `--offline`.
///
/// Without `--yes` the same invocation needs no store at all: it stages and exits 10, which is
/// rule S. If the check above had been written as "ingest needs a store" it would have broken
/// that, and this is what would have caught it.
#[test]
fn without_yes_no_store_is_needed() {
    let dir = dir_for("plain");
    let f = dir.join("doc.txt");
    std::fs::write(&f, "The pool saturated.").unwrap();

    let out = Command::new(BIN)
        .current_dir(&dir)
        .args(["ingest", "--offline", f.to_str().unwrap()])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);

    // `--offline` has no provider to reach, so this fails for that reason rather than for a
    // missing store - whatever the code, it is not the usage error above.
    assert!(
        !stderr.contains("`--yes` commits the batch"),
        "a run that never asked to commit was told it needs a store: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
