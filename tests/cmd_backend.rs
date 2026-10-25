//! `--backend` (SMYSL-2.8 §4.4; TX-P13b step 2): a CBOR log read and written through the unit
//! store answers as the in-memory store does, and a build without the unit store says so.

#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

const DOC: &str = "\
@claim p/one { status: speculative }
~ Prerequisite one.

@decision d/choice { status: speculative, grounds: [p/one] }
~ A decision resting on it.
";

fn scratch(what: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-backend-{what}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// A CBOR log of [`DOC`], written by `merge`.
fn a_log(dir: &Path) {
    std::fs::write(dir.join("doc.smy"), DOC).unwrap();
    let o = run(
        dir,
        &["merge", "doc.smy", "--format", "cbor", "-o", "log.smy"],
    );
    assert!(o.status.success(), "{}", text(&o));
}

#[cfg(not(feature = "store-redb"))]
#[test]
fn a_build_without_the_unit_store_refuses_it_by_name() {
    let dir = scratch("absent");
    a_log(&dir);
    let o = run(&dir, &["--backend", "redb", "check", "-s", "log.smy"]);
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    assert!(text(&o).contains("--features store-redb"), "{}", text(&o));
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "store-redb")]
#[test]
fn check_through_the_unit_store_says_what_the_memory_store_says() {
    let dir = scratch("check");
    a_log(&dir);
    let memory = run(&dir, &["check", "-s", "log.smy"]);
    let first = run(&dir, &["--backend", "redb", "check", "-s", "log.smy"]);
    let again = run(&dir, &["--backend", "redb", "check", "-s", "log.smy"]);
    for o in [&memory, &first, &again] {
        assert!(o.status.success(), "{}", text(o));
    }
    assert_eq!(first.stdout, memory.stdout);
    assert_eq!(again.stdout, memory.stdout);
    assert!(
        String::from_utf8_lossy(&first.stderr).contains("SMY-W501"),
        "the first open builds the store"
    );
    assert!(again.stderr.is_empty(), "and the next only opens it");
    assert!(dir.join(".smysl/store/log.redb").is_file());
    // Surface text is read into memory whatever the backend.
    let surface = run(&dir, &["--backend", "redb", "check", "-s", "doc.smy"]);
    assert!(surface.status.success(), "{}", text(&surface));
    assert!(!dir.join(".smysl/store/doc.redb").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A write goes through the writer, once; a refused one writes nothing; and the store still
/// describes the log afterwards, so `reindex --verify` passes without a rebuild.
#[cfg(feature = "store-redb")]
#[test]
fn a_write_through_the_unit_store_lands_once_and_a_refused_one_not_at_all() {
    let dir = scratch("write");
    a_log(&dir);
    // The record count `check` prints.
    let records = |dir: &Path| -> usize {
        let o = run(dir, &["check", "-s", "log.smy"]);
        let out = String::from_utf8_lossy(&o.stdout).to_string();
        out.split_whitespace()
            .zip(out.split_whitespace().skip(1))
            .find(|(_, w)| w.starts_with("records"))
            .and_then(|(n, _)| n.parse().ok())
            .unwrap_or_else(|| panic!("no record count in {out}"))
    };
    let before = records(&dir);

    let refused = run(&dir, &["--backend", "redb", "retract", "p/one", "log.smy"]);
    assert!(!refused.status.success(), "{}", text(&refused));
    assert_eq!(records(&dir), before, "a refused retraction wrote nothing");

    let done = run(
        &dir,
        &[
            "--backend",
            "redb",
            "retract",
            "p/one",
            "log.smy",
            "--authority",
            "any",
            "--as",
            "human:reviewer",
        ],
    );
    assert!(done.status.success(), "{}", text(&done));
    assert_eq!(records(&dir), before + 1, "the retraction, once");
    assert!(
        !dir.join(".smysl/store/log.lock").exists(),
        "the writer's lock is released"
    );

    let verify = run(
        &dir,
        &["reindex", "--backend", "redb", "--verify", "log.smy"],
    );
    assert!(verify.status.success(), "{}", text(&verify));
    assert!(
        text(&verify).contains("unit store matches a rebuild"),
        "{}",
        text(&verify)
    );
    assert!(!text(&verify).contains("SMY-W501"), "{}", text(&verify));
    let _ = std::fs::remove_dir_all(&dir);
}

/// `reindex --backend redb` builds the store; `--verify` fails with exit 9 on a store that no
/// longer describes its log, and the next verify, of the store that open rebuilt, passes.
#[cfg(feature = "store-redb")]
#[test]
fn reindex_builds_the_unit_store_and_verify_catches_one_that_drifted() {
    let dir = scratch("reindex");
    a_log(&dir);
    let built = run(&dir, &["reindex", "--backend", "redb", "log.smy"]);
    assert!(built.status.success(), "{}", text(&built));
    assert!(
        text(&built).contains("unit store rebuilt"),
        "{}",
        text(&built)
    );

    // The log replaced by another one: the store describes something else now.
    let other = "@claim p/other { status: speculative }\n~ Something else.\n";
    std::fs::write(dir.join("other.smy"), other).unwrap();
    let o = run(
        &dir,
        &["merge", "other.smy", "--format", "cbor", "-o", "log.smy"],
    );
    assert!(o.status.success(), "{}", text(&o));
    let drifted = run(
        &dir,
        &["reindex", "--backend", "redb", "--verify", "log.smy"],
    );
    assert_eq!(drifted.status.code(), Some(9), "{}", text(&drifted));
    let again = run(
        &dir,
        &["reindex", "--backend", "redb", "--verify", "log.smy"],
    );
    assert!(again.status.success(), "{}", text(&again));
    let _ = std::fs::remove_dir_all(&dir);
}
