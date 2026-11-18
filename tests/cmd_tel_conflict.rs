//! Detection kind 7, `WitnessConflict` (RFC SMYSL-3 §11.5, TL-P7 step 4), against the binary.
//!
//! Two processes report one gauge over OTLP, so the reader gives them two origins. A person
//! declares the two series the same quantity, and each gets an exact window and a sketched window
//! over one span. Then `tel check`:
//!
//! - **The values agree:** no conflict.
//! - **They differ by 1 %:** the exact windows conflict (`max` differs). The sketched windows,
//!   alpha 0.01, do not: 1 % is inside twice alpha.
//! - **A transform on both lineages:** they are one witness, and nothing conflicts.

#![cfg(all(feature = "cli", feature = "tel"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use smysl::schemas::tel;
use smysl::text::library::Library;
use smysl::StoreRead;

const BIN: &str = env!("CARGO_BIN_EXE_smysl");
const T0: u64 = 1_768_557_600_000;

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("smysl-tel-conflict-{}-{name}", std::process::id()));
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

/// An OTLP/JSON export of the gauge `disk.used` from instance `inst`, one point per value, ten
/// seconds apart from `T0`.
fn export(inst: &str, values: &[f64]) -> String {
    let points: Vec<String> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            format!(
                r#"{{"timeUnixNano":"{}","asDouble":{v}}}"#,
                (T0 + 10_000 * i as u64) * 1_000_000
            )
        })
        .collect();
    format!(
        r#"{{"resourceMetrics":[{{"resource":{{"attributes":[{{"key":"service.name","value":{{"stringValue":"disk"}}}},{{"key":"service.instance.id","value":{{"stringValue":"{inst}"}}}}]}},"scopeMetrics":[{{"scope":{{"name":"s","version":"1"}},"metrics":[{{"name":"disk.used","unit":"By","gauge":{{"dataPoints":[{}]}}}}]}}]}}]}}"#,
        points.join(",")
    )
}

/// Two instances' exports read, declared one quantity, and windowed exactly and by sketch.
fn library(name: &str, b: &[f64]) -> PathBuf {
    let dir = scratch(name);
    let t0 = T0.to_string();
    for (inst, values) in [("a", &[100.0, 200.0, 300.0][..]), ("b", b)] {
        let f = dir.join(format!("{inst}.json"));
        std::fs::write(&f, export(inst, values)).unwrap();
        let f = f.display().to_string();
        ok(
            &dir,
            &[
                "tel",
                "read",
                "otlp-json/1",
                &f,
                "--instrument",
                "otel-collector:c1",
                "--at",
                &t0,
                "-s",
                "lib",
                "--yes",
            ],
        );
    }
    let lib = Library::open(&dir.join("lib")).unwrap();
    let mut series: Vec<(String, String)> = StoreRead::units(lib.store())
        .filter(|(_, u)| u.core.schema.as_str() == tel::SERIES)
        .map(|(uid, u)| (tel::entity_key(&u.core).unwrap(), uid.canonical()))
        .collect();
    drop(lib);
    series.sort();
    assert_eq!(series.len(), 2);
    let (from, to, at) = (
        T0.to_string(),
        (T0 + 60_000).to_string(),
        (T0 + 60_000).to_string(),
    );
    for (_, s) in &series {
        ok(
            &dir,
            &[
                "tel", "window", s, &from, &to, "-s", "lib", "--at", &at, "--yes",
            ],
        );
        ok(
            &dir,
            &[
                "tel", "window", s, &from, &to, "--sketch", "0.01", "-s", "lib", "--at", &at,
                "--yes",
            ],
        );
    }
    ok(
        &dir,
        &[
            "tel",
            "declare",
            "same-quantity",
            &series[0].1,
            &series[1].1,
            "-s",
            "lib",
            "--as",
            "human:alice",
            "--at",
            &at,
        ],
    );
    dir
}

fn check(dir: &Path) -> String {
    ok(dir, &["tel", "check", "-s", "lib"])
}

#[test]
fn witnesses_that_agree_do_not_conflict() {
    let dir = library("agree", &[100.0, 200.0, 300.0]);
    let c = check(&dir);
    assert!(c.contains("0 witness conflict(s)"), "{c}");
}

#[test]
fn exact_windows_conflict_and_sketches_within_twice_alpha_do_not() {
    let dir = library("differ", &[101.0, 202.0, 303.0]);
    let c = check(&dir);
    let lines: Vec<&str> = c
        .lines()
        .filter(|l| l.contains("witness conflict over"))
        .collect();
    assert_eq!(lines.len(), 1, "the exact pair only: {c}");
    assert!(lines[0].contains("disagree on min"), "{c}");
    assert!(lines[0].starts_with("contention k/c"), "{c}");
    assert!(c.contains("1 witness conflict(s)"), "{c}");
    // A function of the record set: the same id from a second check.
    assert_eq!(check(&dir), c);
}

#[test]
fn a_shared_transform_makes_one_witness_and_no_conflict() {
    let dir = library("transform", &[101.0, 202.0, 303.0]);
    let at = (T0 + 70_000).to_string();
    let d = |args: &[&str]| {
        let mut a = vec!["tel", "declare"];
        a.extend_from_slice(args);
        a.extend(["-s", "lib", "--as", "human:alice", "--at", &at]);
        ok(&dir, &a);
    };
    d(&["instrument", "transform", "rule:disk"]);
    d(&["via", "otel-sdk:/disk/a", "rule:disk"]);
    d(&["via", "otel-sdk:/disk/b", "rule:disk"]);
    let c = check(&dir);
    assert!(c.contains("0 witness conflict(s)"), "{c}");
}

/// `tel investigate show` reports what measured its evidence, and the conflicts on it. Both
/// processes reach smysl through one collector: common transport, which merges nothing.
#[test]
fn an_investigation_sees_the_witnesses_of_its_evidence() {
    let dir = library("show", &[101.0, 202.0, 303.0]);
    let lib = Library::open(&dir.join("lib")).unwrap();
    let exact: Vec<String> = StoreRead::units(lib.store())
        .filter(|(_, u)| u.core.schema.as_str() == tel::WINDOW)
        .filter(|(_, u)| {
            !tel::window_fields(u.core.payload.as_deref().unwrap())
                .unwrap()
                .approximate
        })
        .map(|(uid, _)| uid.canonical())
        .collect();
    drop(lib);
    assert_eq!(exact.len(), 2);
    let at = |ms: u64| (T0 + ms).to_string();
    let (a1, a2) = (at(80_000), at(81_000));
    let opened = ok(
        &dir,
        &[
            "tel",
            "investigate",
            "open",
            "is the disk full?",
            "-s",
            "lib",
            "--as",
            "human:alice",
            "--at",
            &a1,
        ],
    );
    let inv = opened
        .lines()
        .find(|l| l.starts_with("x.tel/investigation"))
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap()
        .to_string();
    ok(
        &dir,
        &[
            "tel",
            "investigate",
            "consult",
            &inv,
            &exact[0],
            "-s",
            "lib",
            "--as",
            "human:alice",
            "--at",
            &a2,
        ],
    );
    let show = ok(&dir, &["tel", "investigate", "show", &inv, "-s", "lib"]);
    assert!(show.contains("witnesses     1 series"), "{show}");
    assert!(show.contains("2 of 2 series, 1 common transport"), "{show}");
    assert!(show.contains("conflicts     1"), "{show}");
}
