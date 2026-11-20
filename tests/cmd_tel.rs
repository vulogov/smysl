//! `smysl tel` (RFC SMYSL-3 §23, TL-P2 step 5): reading snapshots into a library.
//!
//! What the command promises, held end to end against the binary:
//!
//! - a run without `--yes` writes **nothing** to the library — not even the directory — and
//!   stages the units it would commit;
//! - the same run with `--yes` writes the snapshots it said it would, by the same manifest ids;
//! - a later run diffs against the head of each chain, so a change is seen once;
//! - the clock, the licence and the profile are the caller's to give, and a missing one is a
//!   refusal rather than a default.

#![cfg(all(feature = "cli", feature = "tel"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

fn fixture(rel: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/tel/diff")
        .join(rel)
        .display()
        .to_string()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-tel-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .current_dir(dir)
        .args(args)
        .output()
        .expect("the binary runs")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn read_args<'a>(files: &[&'a str], at: &'a str, yes: bool) -> Vec<&'a str> {
    let mut v = vec!["tel", "read", "k8s/1"];
    v.extend_from_slice(files);
    v.extend_from_slice(&[
        "--profile",
        "k8s-deployments/1",
        "--at",
        at,
        "--licence",
        "unknown",
        "-s",
        "lib",
    ]);
    if yes {
        v.push("--yes");
    }
    v
}

fn manifests(o: &Output) -> Vec<String> {
    out(o)
        .lines()
        .filter(|l| l.starts_with("manifest") || l.starts_with("would add"))
        .map(|l| {
            l.split_whitespace()
                .nth(l.starts_with("would") as usize + 1)
                .unwrap()
                .to_string()
        })
        .collect()
}

#[test]
fn a_review_writes_nothing_and_yes_writes_what_it_said() {
    let dir = scratch("review");
    let base = fixture("base.json");
    let review = run(&dir, &read_args(&[&base], "1768557600000", false));
    assert_eq!(review.status.code(), Some(10), "staged: {}", out(&review));
    assert!(!dir.join("lib").exists(), "a review creates no library");
    assert!(dir.join(".smysl/staged.smy").exists());

    let commit = run(&dir, &read_args(&[&base], "1768557600000", true));
    assert!(
        commit.status.success(),
        "{}",
        String::from_utf8_lossy(&commit.stderr)
    );
    assert_eq!(
        manifests(&review),
        manifests(&commit),
        "the same snapshot, the same mid"
    );
    assert!(dir.join("lib/LIBRARY").exists());

    let check = run(&dir, &["check", "lib"]);
    assert!(check.status.success(), "{}", out(&check));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_chain_is_diffed_once_and_noise_is_nothing() {
    let dir = scratch("chain");
    let (base, beat, bump) = (
        fixture("base.json"),
        fixture("heartbeat-only/s1.json"),
        fixture("image-bump/s1.json"),
    );
    assert!(run(&dir, &read_args(&[&base], "1768557600000", true))
        .status
        .success());
    let second = run(&dir, &read_args(&[&beat, &bump], "1768557660000", true));
    assert!(second.status.success());
    let text = out(&second);
    assert!(
        text.contains("1 new snapshot(s), 1 unchanged; 1 change(s)"),
        "{text}"
    );
    // The same file again: the head already is this snapshot.
    let third = run(&dir, &read_args(&[&bump], "1768557720000", true));
    assert!(
        out(&third).contains("0 new snapshot(s), 1 unchanged; 0 change(s)"),
        "{}",
        out(&third)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_clock_the_licence_and_the_profile_are_required() {
    let dir = scratch("required");
    let base = fixture("base.json");
    for missing in ["--at", "--licence", "--profile"] {
        let mut args = read_args(&[&base], "1768557600000", true);
        let at = args.iter().position(|a| *a == missing).unwrap();
        args.drain(at..at + 2);
        let o = run(&dir, &args);
        assert_eq!(o.status.code(), Some(2), "{missing}");
    }
    // A profile for another scheme, and a reader that is not one.
    let mut wrong = read_args(&[&base], "1", true);
    let p = wrong
        .iter()
        .position(|a| *a == "k8s-deployments/1")
        .unwrap();
    wrong[p] = "zabbix-items/1";
    assert_eq!(run(&dir, &wrong).status.code(), Some(2));
    let mut reader = read_args(&[&base], "1", true);
    reader[2] = "json/1";
    assert_eq!(run(&dir, &reader).status.code(), Some(2));
    // zabbix/1 without the API version.
    let z = run(
        &dir,
        &[
            "tel",
            "read",
            "zabbix/1",
            &base,
            "--profile",
            "zabbix-items/1",
            "--at",
            "1",
            "--licence",
            "unknown",
            "-s",
            "lib",
        ],
    );
    assert_eq!(z.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&z.stderr).contains("--zabbix-api"));
    assert!(!dir.join("lib").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_profile_checks_and_prints_its_hash() {
    let dir = scratch("profile");
    let o = run(&dir, &["tel", "profile", "check", "k8s-deployments/1"]);
    assert!(o.status.success());
    assert!(out(&o).contains("hash      b3:"), "{}", out(&o));
    let file = run(
        &dir,
        &["tel", "profile", "check", &fixture("profile.hjson")],
    );
    assert!(
        file.status.success(),
        "{}",
        String::from_utf8_lossy(&file.stderr)
    );
    assert_eq!(
        run(&dir, &["tel", "profile", "check", "nope/1"])
            .status
            .code(),
        Some(2)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
