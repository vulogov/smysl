//! `smysl tel calibrate` (RFC SMYSL-3 §21.1), against the binary, over the GE-M3 samples read as
//! one OTLP/JSON export.
//!
//! - Each level detector's calibrated recipe holds its trials to the rate, and `tel detect`
//!   takes it as printed.
//! - A second calibration gives the same recipe.
//! - What calibration needs is required.

#![cfg(all(feature = "cli", feature = "tel"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");
const START: u64 = 1_768_550_400_000;
const MIN: u64 = 60_000;

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

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .current_dir(dir)
        .args(args)
        .output()
        .expect("the binary runs")
}

fn library(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("smysl-tel-calibrate-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
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

/// Ten trials of a twenty-minute reference and the ten minutes after it, over the first two hours.
fn windows() -> Vec<String> {
    (0..10)
        .map(|k| {
            let r = START + k * 10 * MIN;
            format!("{}-{}-{}", r, r + 20 * MIN, r + 30 * MIN)
        })
        .collect()
}

fn calibrate(dir: &Path, recipe: &str, rate: &str) -> Output {
    let cpu = series("cpu");
    let ws = windows();
    let mut args = vec!["tel", "calibrate", recipe, "--series", &cpu, "--rate", rate];
    for w in &ws {
        args.extend(["--window", w]);
    }
    args.extend(["-s", "lib"]);
    run(dir, &args)
}

#[test]
fn a_calibrated_recipe_holds_its_rate_and_detect_takes_it() {
    let dir = library("holds");
    for name in ["mad", "cusum", "pelt"] {
        let o = calibrate(&dir, &fixture(&format!("{name}.hjson")), "0.2");
        let text = String::from_utf8_lossy(&o.stdout).into_owned();
        assert_eq!(
            o.status.code(),
            Some(0),
            "{name}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        let mut lines = text.lines();
        let recipe = lines.next().expect("a recipe");
        assert!(
            recipe.starts_with(&format!("{{ detector: {name}/1,")),
            "{recipe}"
        );
        let summary = lines.next().expect("a summary");
        let over: usize = summary
            .split_whitespace()
            .nth(2)
            .and_then(|n| n.parse().ok())
            .expect("a count");
        assert!(over <= 2, "{summary}");
        assert!(summary.contains("of 10 trial(s)"), "{summary}");

        let again = calibrate(&dir, &fixture(&format!("{name}.hjson")), "0.2");
        assert_eq!(
            String::from_utf8_lossy(&again.stdout),
            text,
            "{name}: twice"
        );

        let path = dir.join(format!("{name}-calibrated.hjson"));
        std::fs::write(&path, recipe).unwrap();
        let cpu = series("cpu");
        let o = run(
            &dir,
            &[
                "tel",
                "detect",
                path.to_str().unwrap(),
                "--series",
                &cpu,
                "--during",
                "1768550400000-1768564800000",
                "-s",
                "lib",
            ],
        );
        assert!(
            matches!(o.status.code(), Some(0 | 10)),
            "{name}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
}

#[test]
fn what_calibration_needs_is_required() {
    let dir = library("usage");
    let (cusum, episode) = (fixture("cusum.hjson"), fixture("episode.hjson"));
    let cpu = series("cpu");
    let w = windows()[0].clone();
    for args in [
        vec![
            "tel",
            "calibrate",
            &episode,
            "--series",
            &cpu,
            "--window",
            &w,
            "--rate",
            "0.1",
            "-s",
            "lib",
        ],
        vec![
            "tel",
            "calibrate",
            &cusum,
            "--window",
            &w,
            "--rate",
            "0.1",
            "-s",
            "lib",
        ],
        vec![
            "tel",
            "calibrate",
            &cusum,
            "--series",
            &cpu,
            "--rate",
            "0.1",
            "-s",
            "lib",
        ],
        vec![
            "tel",
            "calibrate",
            &cusum,
            "--series",
            &cpu,
            "--window",
            "3-2-1",
            "--rate",
            "0.1",
            "-s",
            "lib",
        ],
        vec![
            "tel",
            "calibrate",
            &cusum,
            "--series",
            &cpu,
            "--window",
            &w,
            "-s",
            "lib",
        ],
        vec![
            "tel",
            "calibrate",
            &cusum,
            "--series",
            &cpu,
            "--window",
            &w,
            "--rate",
            "1.5",
            "-s",
            "lib",
        ],
    ] {
        let o = run(&dir, &args);
        assert_eq!(
            o.status.code(),
            Some(2),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
}
