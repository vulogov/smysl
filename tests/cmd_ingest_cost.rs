//! `smysl ingest --text <alias> --cost` prices what the journal already records, and reaches no
//! provider to do it (SMYSL-2.4 §3.7, §6 TX-P5 step 5).
//!
//! The phase exit asks for "a cost report from the journal that replaces draft 3 §7.7's planning
//! figures". A report that needed a provider configured would be unreadable on the machine that
//! most wants it — the one that has just finished extracting a corpus and is deciding whether to
//! extract the next one — so `--cost` is handled before the registry is loaded, needs no `--yes`
//! because it commits nothing, and says plainly when a journal holds nothing for the expression
//! rather than printing zeroes that look like a measurement.

#![cfg(all(feature = "cli", feature = "ingest", feature = "text"))]

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

fn dir_for(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("smysl-cost-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A library holding one short text, built by the binary itself.
fn library(dir: &std::path::Path) -> std::path::PathBuf {
    let file = dir.join("report.txt");
    std::fs::write(
        &file,
        "the pool wait rose to 1.9 seconds\nthe rollback returned p99 to baseline\n",
    )
    .unwrap();
    let lib = dir.join("lib");
    let out = Command::new(BIN)
        .args([
            "text",
            "add",
            "--store",
            lib.to_str().unwrap(),
            "--reader",
            "txt/1",
            "--alias",
            "incident",
            "--licence",
            "public-domain",
            "--lang",
            "en",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    lib
}

#[test]
fn a_journal_with_nothing_in_it_says_so_rather_than_reporting_zeroes() {
    let dir = dir_for("empty");
    let lib = library(&dir);
    let out = Command::new(BIN)
        .args([
            "ingest",
            "--store",
            lib.to_str().unwrap(),
            "--text",
            "incident",
            "--cost",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert!(stdout.contains("expression   incident"), "{stdout}");
    assert!(
        stdout.contains("records no window"),
        "an unextracted expression was priced: {stdout}",
    );
    // The two refusals this path must *not* make: it commits nothing, so it does not ask for
    // `--yes`, and it calls nothing, so a machine with no provider configured still gets it.
    assert!(!stderr.contains("--yes"), "{stderr}");
    assert!(!stderr.to_lowercase().contains("provider"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_cost_of_a_library_that_is_not_there_is_a_failure_and_not_an_empty_report() {
    let dir = dir_for("missing");
    let out = Command::new(BIN)
        .args([
            "ingest",
            "--store",
            dir.join("nothing").to_str().unwrap(),
            "--text",
            "incident",
            "--cost",
        ])
        .output()
        .unwrap();
    assert_ne!(out.status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cost_without_a_store_names_what_is_missing() {
    let dir = dir_for("nostore");
    let out = Command::new(BIN)
        .current_dir(&dir)
        .args(["ingest", "--text", "incident", "--cost"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("--store"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}
