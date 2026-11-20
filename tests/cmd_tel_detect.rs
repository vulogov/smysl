//! `smysl tel detect` (RFC SMYSL-3 §21, §23, TL-P5 step 5), against the binary, over the GE-M3
//! samples read as one OTLP/JSON export.
//!
//! - Each detector stages its units without `--yes` (exit 10) and commits them with it.
//! - A second run into a fresh library gives the same uids: replay, end to end.
//! - What each detector needs is required.

#![cfg(all(feature = "cli", feature = "tel"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");
const DURING: &str = "1768550400000-1768564800000";

fn fixture(rel: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/tel/detect")
        .join(rel)
        .display()
        .to_string()
}

fn series(name: &str) -> String {
    format!(
        r#"otel:{{"attributes":{{}},"kind":"gauge","monotonic":false,"name":"ge_m3_{name}","resource":{{"service.name":"ge-m3"}},"scope":{{"name":"sim","version":"1"}},"temporality":"unspecified","unit":"1"}}"#
    )
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-tel-detect-{}-{name}", std::process::id()));
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

fn library(name: &str) -> PathBuf {
    let dir = scratch(name);
    let f = fixture("series.otlp.json");
    let o = run(
        &dir,
        &[
            "tel",
            "read",
            "otlp-json/1",
            &f,
            "--instrument",
            "otel-collector:ge-m3",
            "--at",
            "1768550400000",
            "-s",
            "lib",
            "--yes",
        ],
    );
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    dir
}

/// Every detector, committed; the uids each printed.
fn detect_all(dir: &Path) -> Vec<String> {
    let all: Vec<String> = ["cpu", "latency", "errors", "queue"].map(series).to_vec();
    let mut uids = Vec::new();
    for recipe in ["cusum", "pelt", "mad", "episode", "baro"] {
        let r = fixture(&format!("{recipe}.hjson"));
        let mut args = vec![
            "tel",
            "detect",
            &r,
            "-s",
            "lib",
            "--at",
            "1768564800000",
            "--yes",
        ];
        if recipe != "baro" {
            args.extend(["--during", DURING]);
        }
        if recipe != "episode" {
            for s in &all {
                args.extend(["--series", s]);
            }
        }
        let o = run(dir, &args);
        assert_eq!(
            o.status.code(),
            Some(0),
            "{recipe}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        let lines: Vec<String> = out(&o)
            .lines()
            .filter(|l| l.starts_with("x.tel/"))
            .map(|l| l.split_whitespace().nth(1).unwrap().to_string())
            .collect();
        assert!(!lines.is_empty(), "{recipe} found nothing");
        uids.extend(lines);
    }
    uids
}

#[test]
fn every_detector_commits_and_a_second_run_gives_the_same_uids() {
    let a = detect_all(&library("first"));
    let b = detect_all(&library("second"));
    assert_eq!(a, b, "a second run, into a fresh library");
}

#[test]
fn a_review_stages_and_writes_nothing() {
    let dir = library("review");
    let r = fixture("pelt.hjson");
    let cpu = series("cpu");
    let before = std::fs::read_dir(dir.join("lib")).unwrap().count();
    let o = run(
        &dir,
        &[
            "tel", "detect", &r, "--series", &cpu, "--during", DURING, "-s", "lib",
        ],
    );
    assert_eq!(
        o.status.code(),
        Some(10),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(dir.join(".smysl/staged.smy").exists());
    let staged = std::fs::read_to_string(dir.join(".smysl/staged.smy")).unwrap();
    assert!(
        staged.contains("@x.tel/segment"),
        "segments have a surface form"
    );
    assert_eq!(std::fs::read_dir(dir.join("lib")).unwrap().count(), before);
}

#[test]
fn what_each_detector_needs_is_required() {
    let dir = library("usage");
    let (cusum, baro, episode) = (
        fixture("cusum.hjson"),
        fixture("baro.hjson"),
        fixture("episode.hjson"),
    );
    let cpu = series("cpu");
    for args in [
        vec!["tel", "detect", &cusum, "--during", DURING, "-s", "lib"],
        vec!["tel", "detect", &cusum, "--series", &cpu, "-s", "lib"],
        vec![
            "tel", "detect", &cusum, "--series", &cpu, "--during", "5-1", "-s", "lib",
        ],
        vec![
            "tel", "detect", &cusum, "--series", &cpu, "--during", DURING, "--yes", "-s", "lib",
        ],
        vec![
            "tel",
            "detect",
            &cusum,
            "--series",
            &cpu,
            "--during",
            DURING,
            "--trigger",
            "b3:x",
            "-s",
            "lib",
        ],
        vec!["tel", "detect", &episode, "-s", "lib"],
        vec!["tel", "detect", &baro, "-s", "lib"],
    ] {
        assert_eq!(run(&dir, &args).status.code(), Some(2), "{args:?}");
    }
    let bad = dir.join("bad.hjson");
    std::fs::write(&bad, "{ detector: cusum/1, kk: 1 }").unwrap();
    let o = run(
        &dir,
        &[
            "tel",
            "detect",
            bad.to_str().unwrap(),
            "--series",
            &cpu,
            "--during",
            DURING,
            "-s",
            "lib",
        ],
    );
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("not a parameter"));
}
