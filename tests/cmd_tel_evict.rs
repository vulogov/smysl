//! `smysl tel evict` (RFC SMYSL-3 §13 rule TL, TL-P8 step 1), against the binary.
//!
//! Three OTLP exports are read and two windows (exact and sketched) are materialised over them.
//! Then:
//!
//! - **A dry run** says what it would evict and removes nothing.
//! - **`--yes` evicts the samples and nothing else.** No unit, relation or attestation changes,
//!   and the windows' evidence becomes `SMY-W605` (not re-derivable). That is a warning, never an
//!   error.
//! - **A bound before the data** evicts nothing.
//! - **The same exports read again** bring the samples back as a set. The windows recompute
//!   clean, as they did before.

#![cfg(all(feature = "cli", feature = "tel"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use smysl::text::library::Library;
use smysl::StoreRead;

const BIN: &str = env!("CARGO_BIN_EXE_smysl");
const T0: u64 = 1_768_557_600_000;
const DAY: u64 = 86_400_000;
const SUM: &str = r#"otel:{"attributes":{},"kind":"histogram","monotonic":false,"name":"http.server.duration","part":"sum","resource":{"service.instance.id":"c1","service.name":"cart"},"scope":{"name":"io.opentelemetry.http","version":"1.2.0"},"temporality":"cumulative","unit":"s"}"#;

fn fixture(rel: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/tel")
        .join(rel)
        .display()
        .to_string()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-tel-evict-{}-{name}", std::process::id()));
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

fn ok(dir: &Path, args: &[&str]) -> String {
    let o = run(dir, args);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn read(dir: &Path) {
    let paths: Vec<String> = (0..3).map(|n| fixture(&format!("otlp/e{n}.pb"))).collect();
    let t0 = T0.to_string();
    let mut args = vec!["tel", "read", "otlp/1"];
    args.extend(paths.iter().map(String::as_str));
    args.extend([
        "--instrument",
        "otel-collector:c1",
        "--at",
        &t0,
        "-s",
        "lib",
        "--yes",
    ]);
    ok(dir, &args);
}

/// Everything the record set holds, as one comparable value: units, relations, and each unit's
/// attestations.
fn records(dir: &Path) -> (usize, usize, usize) {
    let lib = Library::open(&dir.join("lib")).unwrap();
    let s = lib.store();
    (
        StoreRead::units(s).count(),
        s.relations().count(),
        StoreRead::units(s).map(|(_, u)| u.attestations.len()).sum(),
    )
}

fn evict(dir: &Path, older: &str, at: u64, yes: bool) -> String {
    let at = at.to_string();
    let mut args = vec![
        "tel",
        "evict",
        "--older-than",
        older,
        "--at",
        &at,
        "-s",
        "lib",
    ];
    if yes {
        args.push("--yes");
    }
    ok(dir, &args)
}

#[test]
fn eviction_removes_samples_and_never_a_record() {
    let dir = scratch("evict");
    read(&dir);
    let (from, to) = (T0.to_string(), (T0 + 60_000).to_string());
    for sketch in [None, Some("0.01")] {
        let mut args = vec![
            "tel", "window", SUM, &from, &to, "-s", "lib", "--at", &to, "--yes",
        ];
        if let Some(a) = sketch {
            args.extend(["--sketch", a]);
        }
        ok(&dir, &args);
    }
    let check = |dir: &Path| ok(dir, &["tel", "check", "-s", "lib"]);
    assert!(!check(&dir).contains("SMY-W605"));
    let before = records(&dir);

    // A day later, keeping an hour: every sample is older than the bound.
    let dry = evict(&dir, "1h", T0 + DAY, false);
    assert!(dry.contains("nothing removed"), "{dry}");
    assert!(!dry.contains(" 0 chunk(s)"), "the plan finds chunks: {dry}");
    assert!(
        !check(&dir).contains("SMY-W605"),
        "a dry run removes nothing"
    );

    let done = evict(&dir, "1h", T0 + DAY, true);
    assert!(done.contains("evicted; no record written"), "{done}");
    assert_eq!(
        done.lines().next(),
        dry.lines().next(),
        "what was planned is what was done"
    );
    assert_eq!(records(&dir), before, "rule TL: units are never evicted");
    let c = check(&dir);
    assert_eq!(
        c.lines().filter(|l| l.contains("SMY-W605")).count(),
        2,
        "both windows, not re-derivable: {c}"
    );
    assert!(c.contains("0 error(s)"), "{c}");

    // Again: nothing left. And a bound before the data evicts nothing.
    assert!(evict(&dir, "1h", T0 + DAY, true).contains(" 0 chunk(s)"));

    // The exports again, as a merge with a peer that kept them would: the samples come back as a
    // set, and the windows recompute clean.
    read(&dir);
    let c = check(&dir);
    assert!(!c.contains("SMY-W605") && !c.contains("SMY-E604"), "{c}");
}

#[test]
fn a_bound_before_the_data_evicts_nothing_and_usage_is_checked() {
    let dir = scratch("usage");
    read(&dir);
    assert!(evict(&dir, "1d", T0, true).contains(" 0 chunk(s)"));
    let code = |args: &[&str]| run(&dir, args).status.code();
    assert_eq!(
        code(&["tel", "evict", "--older-than", "1d", "-s", "lib"]),
        Some(2)
    );
    assert_eq!(
        code(&[
            "tel",
            "evict",
            "--older-than",
            "a while",
            "--at",
            "5",
            "-s",
            "lib"
        ]),
        Some(2)
    );
    assert_eq!(code(&["tel", "evict", "--at", "5", "-s", "lib"]), Some(2));
    assert_eq!(
        code(&[
            "tel",
            "evict",
            "x",
            "--older-than",
            "1d",
            "--at",
            "5",
            "-s",
            "lib"
        ]),
        Some(2)
    );
}
