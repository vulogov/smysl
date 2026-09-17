//! `pack --support` and `trace --via` (1.5): the edges a caller says carry a dependency.
//!
//! A producer that links a prerequisite by `conditions` keeps it out of the decision's uid on
//! purpose. C1 and C2 read `deps` and `grounds` inside the unit, so without these flags a packed
//! decision leaves its prerequisite behind.

#![cfg(feature = "cli")]

use std::path::PathBuf;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

const DOC: &str = "\
@constraint p/locked { status: speculative }
~ The lockfile must stay unchanged on a release build.

@decision d/pin { status: speculative }
~ Pin the dependency rather than tracking the range.

@rel p/locked --conditions--> d/pin
";

fn store() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-c8-{}", std::process::id()));
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

#[test]
fn pack_carries_what_a_unit_rests_on_only_when_asked() {
    let path = store();
    let p = path.to_str().unwrap();
    let without = run(&["pack", "--budget", "24", "--focus", "d/pin", "--explain", p]);
    assert!(without.status.success(), "{}", out(&without));
    assert!(
        !out(&without).contains("lockfile"),
        "the prerequisite travelled without --support:\n{}",
        out(&without)
    );

    // With the flag the same budget cannot meet the floor: the decision rests on the
    // prerequisite, so the pair must fit or nothing does.
    let tight = run(&[
        "pack",
        "--budget",
        "24",
        "--focus",
        "d/pin",
        "--support",
        "premises",
        p,
    ]);
    assert_eq!(tight.status.code(), Some(4), "{}", out(&tight));
    assert!(out(&tight).contains("mandatory floor"), "{}", out(&tight));

    let with = run(&[
        "pack",
        "--budget",
        "100",
        "--focus",
        "d/pin",
        "--support",
        "premises",
        "--explain",
        p,
    ]);
    assert!(with.status.success(), "{}", out(&with));
    assert!(out(&with).contains("lockfile"), "{}", out(&with));
    assert!(out(&with).contains("C8"), "{}", out(&with));
    assert!(out(&with).contains("support of"), "{}", out(&with));
}

#[test]
fn trace_via_walks_the_same_edges_and_an_unknown_kind_is_refused() {
    let path = store();
    let p = path.to_str().unwrap();
    let default = run(&["trace", "d/pin", p]);
    assert_eq!(
        default.stdout.iter().filter(|b| **b == b'\n').count(),
        2,
        "grounds alone reaches nothing else:\n{}",
        out(&default)
    );

    let via = run(&["trace", "--via", "premises", "d/pin", p]);
    assert!(out(&via).contains("(rests-on)"), "{}", out(&via));

    let bad = run(&["trace", "--via", "nonsuch", "d/pin", p]);
    assert_eq!(bad.status.code(), Some(2), "{}", out(&bad));
    assert!(out(&bad).contains("not a relation kind"), "{}", out(&bad));
}
