//! A command that takes a unit takes its label as well as its uid, and cannot tell them apart.
//!
//! Until 1.3 every such command refused a label with "is not a uid", and chapter 13 documented
//! that as a rule: a label "has no existence at the store level". It had one since 0.2, when
//! `Record::LabelBinding` put labels on the wire. `load_store` recovered them from every store,
//! and five of the six commands that take a unit discarded them on the spot. A caller with a store
//! full of `m/g532e4d2-2-2` bindings found each uid through `salience` or `pack --explain` and
//! typed it back in.
//!
//! The property worth pinning is not "a label works" but **a label and its uid are the same
//! argument**: identical output, byte for byte, from every command. A label that merely produced
//! *some* successful output could be resolving to the wrong unit.

#![cfg(feature = "cli")]

use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");
const STORE: &str = "fixtures/corpus/F1-incident.smy";
const LABEL: &str = "c/pool-saturation";
const UID: &str = "b3:cvhirtgs2mpvli2ethhyeo32uf";

fn run<S: AsRef<std::ffi::OsStr>>(args: &[S]) -> Output {
    Command::new(BIN)
        .args(["--format", "surface"])
        .args(args)
        .output()
        .expect("the binary under test must run")
}

/// Each command, with `{}` where the unit goes.
const COMMANDS: &[&[&str]] = &[
    &["trace", "{}", STORE],
    &["retract", "--dry-run", "{}", STORE],
    &[
        "pack",
        "--budget",
        "200",
        "--explain",
        "--focus",
        "{}",
        STORE,
    ],
    &["view", "--roots", "{}", STORE],
    &["salience", "--explain", "{}", STORE],
    &["salience", "--seed", "{}", STORE],
    &["thread", "--derive", "brief", "--scope", "{}", STORE],
];

fn with<'a>(cmd: &[&'a str], unit: &'a str) -> Vec<&'a str> {
    cmd.iter()
        .map(|a| if *a == "{}" { unit } else { *a })
        .collect()
}

#[test]
fn a_label_and_its_uid_are_the_same_argument_to_every_command() {
    // The pairing is the premise. If the fixture's label ever named a different unit, every
    // comparison below would still pass — against each other — while testing nothing.
    let by_uid = run(&["trace", UID, STORE]);
    assert!(
        by_uid.status.success(),
        "the fixture's uid no longer resolves"
    );

    for cmd in COMMANDS {
        let a = run(&with(cmd, UID));
        let b = run(&with(cmd, LABEL));
        let shown = cmd.join(" ");
        assert!(
            a.status.success(),
            "{shown} by uid failed: {}",
            String::from_utf8_lossy(&a.stderr)
        );
        assert_eq!(
            a.status.code(),
            b.status.code(),
            "{shown}: exit codes differ by how the unit was named"
        );
        assert_eq!(
            String::from_utf8_lossy(&a.stdout),
            String::from_utf8_lossy(&b.stdout),
            "{shown}: output differs by how the unit was named"
        );
    }
}

#[test]
fn an_unbound_label_is_refused_by_name_and_with_one_exit_code_everywhere() {
    for cmd in COMMANDS {
        let out = run(&with(cmd, "c/pool-saturated"));
        let shown = cmd.join(" ");
        let err = String::from_utf8_lossy(&out.stderr);
        // `retract` used to exit 2 here while `trace` exited 1: it kept its own copy of the
        // resolution, and the two had drifted.
        assert_eq!(out.status.code(), Some(1), "{shown}: {err}");
        assert!(
            err.contains("`c/pool-saturated` is a label, and nothing in this store is bound to it"),
            "{shown}: {err}"
        );
    }
}

#[test]
fn a_label_resolves_from_a_cbor_store_through_its_bindings() {
    // The surface case parses labels directly. The CBOR case is the one the old rule was wrong
    // about: there, a label exists only as a `LabelBinding` record.
    let dir = std::env::temp_dir().join(format!("smysl-label-args-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cbor = dir.join("f1.cbor");
    let cbor_s = cbor.to_str().unwrap();
    let made = Command::new(BIN)
        .args(["merge", STORE, "-o", cbor_s])
        .output()
        .unwrap();
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );

    let a = run(&["trace", UID, cbor_s]);
    let b = run(&["trace", LABEL, cbor_s]);
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    assert_eq!(a.stdout, b.stdout);
    std::fs::remove_dir_all(&dir).ok();
}
