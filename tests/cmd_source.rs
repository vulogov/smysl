//! `find --source` and `pack --source` (1.8): one subject's units out of a shared store.
//!
//! `Store::units_with_source_prefix` has existed since 1.5 and was reachable only from Rust. The
//! question it answers — *which units came from this thing* — is the one a caller asks when a
//! store holds many subjects at once: fifty incidents, a repository's files, a fleet of hosts.
//! Prefix rather than equality because a source reference carries a locator after the subject
//! (`incident:41#auth.pool.wait_ms`), and the question is about the subject.

#![cfg(feature = "cli")]

use std::path::PathBuf;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

const DOC: &str = "\
@evidence e/a41 { status: measured, source: { kind: node, ref: \"incident:41#auth.pool.wait_ms\", captured: 2026-07-05 } }
~ Pool acquisition wait on auth reached 310 ms.

@claim c/a41 { status: inferred, grounds: [e/a41], source: { kind: node, ref: \"incident:41#analysis\" } }
~ Pool saturation on auth is the leading explanation for incident 41.

@evidence e/a42 { status: measured, source: { kind: node, ref: \"incident:42#billing.p99\", captured: 2026-07-06 } }
~ Pool acquisition wait on billing reached 940 ms.
";

fn store() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-source-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.smy");
    std::fs::write(&path, DOC).unwrap();
    path
}

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(["--format", "surface"])
        .args(args)
        .output()
        .expect("the binary under test must run")
}

fn out(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// The prefix selects one subject's units and excludes the other's.
#[test]
fn find_restricts_to_one_subject() {
    let path = store();
    let p = path.to_str().unwrap();

    let all = run(&["find", "pool acquisition wait", p]);
    assert!(all.status.success(), "{}", out(&all));
    assert!(
        out(&all).contains("auth") && out(&all).contains("billing"),
        "unrestricted, both subjects match:\n{}",
        out(&all)
    );

    let one = run(&[
        "find",
        "pool acquisition wait",
        "--source",
        "incident:41",
        p,
    ]);
    assert!(one.status.success(), "{}", out(&one));
    assert!(out(&one).contains("auth"), "{}", out(&one));
    assert!(
        !out(&one).contains("billing"),
        "incident 42's unit leaked into incident 41's query:\n{}",
        out(&one)
    );
}

/// A prefix nothing matches must not silently search everything: an empty candidate set means
/// *unrestricted* to `Query::within`, which is the opposite of what the caller asked for.
#[test]
fn a_prefix_matching_nothing_says_so() {
    let path = store();
    let o = run(&[
        "find",
        "pool",
        "--source",
        "incident:99",
        path.to_str().unwrap(),
    ]);
    assert!(o.status.success(), "{}", out(&o));
    assert!(
        out(&o).contains("no unit has a source starting with"),
        "{}",
        out(&o)
    );
    assert!(
        !out(&o).contains("auth"),
        "a prefix nobody matches returned units anyway:\n{}",
        out(&o)
    );
}

/// `pack --source` scopes the *pack*, which is the "assemble this one subject" case.
#[test]
fn pack_scopes_to_one_subject() {
    let path = store();
    let p = path.to_str().unwrap();

    let o = run(&["pack", "--budget", "400", "--source", "incident:41", p]);
    assert!(o.status.success(), "{}", out(&o));
    assert!(out(&o).contains("scoped 2 unit(s)"), "{}", out(&o));
    assert!(out(&o).contains("auth"), "{}", out(&o));
    assert!(
        !out(&o).contains("billing"),
        "the other subject was packed too:\n{}",
        out(&o)
    );
}

/// And the same guard, where an unrestricted pack would be the whole store.
#[test]
fn packing_a_prefix_matching_nothing_is_refused() {
    let path = store();
    let o = run(&[
        "pack",
        "--budget",
        "400",
        "--source",
        "incident:99",
        path.to_str().unwrap(),
    ]);
    assert!(!o.status.success(), "{}", out(&o));
    assert!(
        out(&o).contains("no unit has a source starting with"),
        "{}",
        out(&o)
    );
}
