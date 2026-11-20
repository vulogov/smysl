//! `smysl tel window`, `smysl tel check` and `tel read zabbix/1 --trends` (RFC SMYSL-3 §9, §23,
//! TL-P4 step 5), against the binary.
//!
//! TL-P4's exit criterion is here: **window uids reproduced by a second run and a sharded run.**
//! - A second run: the same exports read into a fresh library, the same windows asked for,
//!   the same uids.
//! - A sharded run: the exports read one per invocation, last first, as shards of the samples
//!   arriving in another order; the same uids again, exact and sketched.

#![cfg(all(feature = "cli", feature = "tel"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");
const T0: u64 = 1_768_557_600_000;
const SUM: &str = r#"otel:{"attributes":{},"kind":"histogram","monotonic":false,"name":"http.server.duration","part":"sum","resource":{"service.instance.id":"c1","service.name":"cart"},"scope":{"name":"io.opentelemetry.http","version":"1.2.0"},"temporality":"cumulative","unit":"s"}"#;

fn fixture(rel: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/tel")
        .join(rel)
        .display()
        .to_string()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-tel-window-{}-{name}", std::process::id()));
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

fn read_otlp(dir: &Path, files: &[usize]) {
    let paths: Vec<String> = files
        .iter()
        .map(|n| fixture(&format!("otlp/e{n}.pb")))
        .collect();
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
    let o = run(dir, &args);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}

/// The window uids `tel window` prints for the exact and the sketched window.
fn windows(dir: &Path, yes: bool) -> Vec<String> {
    let (from, to, at) = (
        T0.to_string(),
        (T0 + 60_000).to_string(),
        (T0 + 60_000).to_string(),
    );
    let mut uids = Vec::new();
    for sketch in [None, Some("0.01")] {
        let mut args = vec!["tel", "window", SUM, &from, &to, "-s", "lib"];
        if let Some(a) = sketch {
            args.extend(["--sketch", a]);
        }
        if yes {
            args.extend(["--at", &at, "--yes"]);
        }
        let o = run(dir, &args);
        assert_eq!(
            o.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
        for l in out(&o).lines().filter(|l| l.starts_with("window")) {
            uids.push(l.split_whitespace().nth(1).unwrap().to_string());
        }
    }
    uids
}

#[test]
fn window_uids_are_reproduced_by_a_second_run_and_a_sharded_run() {
    let first = scratch("first");
    read_otlp(&first, &[0, 1, 2]);
    let a = windows(&first, true);
    assert_eq!(a.len(), 2, "one exact, one sketched");
    assert_ne!(a[0], a[1]);

    // A second run, into a fresh library.
    let second = scratch("second");
    read_otlp(&second, &[0, 1, 2]);
    assert_eq!(windows(&second, true), a, "a second run");

    // A sharded run: each export its own invocation, last first.
    let sharded = scratch("sharded");
    for n in [2, 1, 0] {
        read_otlp(&sharded, &[n]);
    }
    assert_eq!(windows(&sharded, false), a, "a sharded run");

    // And what was committed checks clean, rule TW and recomputation both.
    let o = run(&first, &["tel", "check", "-s", "lib"]);
    assert_eq!(o.status.code(), Some(0), "{}", out(&o));
    assert!(out(&o).contains("checked   2 window(s): 0 error(s), 0 warning(s)"));
}

#[test]
fn a_review_writes_nothing() {
    let dir = scratch("review");
    read_otlp(&dir, &[0, 1, 2]);
    let before = std::fs::read_dir(dir.join("lib")).unwrap().count();
    let o = run(
        &dir,
        &[
            "tel",
            "window",
            SUM,
            &T0.to_string(),
            &(T0 + 60_000).to_string(),
            "-s",
            "lib",
        ],
    );
    assert!(out(&o).contains("nothing was written"));
    assert_eq!(std::fs::read_dir(dir.join("lib")).unwrap().count(), before);
    let o = run(&dir, &["tel", "check", "-s", "lib"]);
    assert!(out(&o).contains("checked   0 window(s)"), "{}", out(&o));
}

#[test]
fn late_data_makes_a_committed_window_e604_and_a_missing_substrate_w605() {
    let dir = scratch("late");
    let t = |n: u64| (T0 + 15_000 * n).to_string();
    let scrape = |n: u64| {
        let f = fixture(&format!("prom/s{n}.prom"));
        let o = run(
            &dir,
            &[
                "tel",
                "read",
                "prom/1",
                &f,
                "--job",
                "node",
                "--instance",
                "h:9100",
                "--at",
                &t(n),
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
    };
    for n in 0..3 {
        scrape(n);
    }
    let series =
        r#"prom:http_requests_total{code="200",instance="h:9100",job="node",method="get"}"#;
    let o = run(
        &dir,
        &[
            "tel",
            "window",
            series,
            &t(0),
            &t(4),
            "-s",
            "lib",
            "--at",
            &t(4),
            "--yes",
        ],
    );
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(out(&o).contains("count 3"), "{}", out(&o));
    assert_eq!(
        run(&dir, &["tel", "check", "-s", "lib"]).status.code(),
        Some(0)
    );

    // The fourth scrape lands inside the window: it is still true of three samples, and the
    // substrate now gives four.
    scrape(3);
    let o = run(&dir, &["tel", "check", "-s", "lib"]);
    assert_eq!(o.status.code(), Some(3), "{}", out(&o));
    assert!(out(&o).contains("SMY-E604"), "{}", out(&o));
    assert!(out(&o).contains("digest, count"), "{}", out(&o));

    // Without the substrate, the same window is not re-derivable, and nothing more.
    std::fs::remove_file(dir.join("lib/index/series.redb")).unwrap();
    let o = run(&dir, &["tel", "check", "-s", "lib"]);
    assert_eq!(o.status.code(), Some(0), "{}", out(&o));
    assert!(out(&o).contains("SMY-W605"), "{}", out(&o));
}

#[test]
fn zabbix_trends_are_cited_windows() {
    let dir = scratch("trends");
    let (trends, items) = (fixture("zabbix/trends.json"), fixture("zabbix/items.json"));
    let base = [
        "tel",
        "read",
        "zabbix/1",
        &trends,
        "--items",
        &items,
        "--trends",
        "--zabbix-api",
        "7.0",
        "--instrument",
        "zabbix-server:zbx-main",
        "-s",
        "lib",
    ];
    let t0 = T0.to_string();
    let mut review = base.to_vec();
    review.extend(["--at", &t0]);
    let o = run(&dir, &review);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(out(&o).matches("platform cited").count(), 9);
    assert!(!dir.join("lib").exists(), "a review writes nothing");

    let mut yes = review.clone();
    yes.push("--yes");
    let o = run(&dir, &yes);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let o = run(&dir, &["tel", "check", "-s", "lib"]);
    assert_eq!(o.status.code(), Some(0), "{}", out(&o));
    assert!(out(&o).contains("checked   9 window(s): 0 error(s), 0 warning(s)"));
}

#[test]
fn what_window_needs_is_required() {
    let dir = scratch("usage");
    read_otlp(&dir, &[0]);
    let (f, t) = (T0.to_string(), (T0 + 1).to_string());
    for args in [
        vec!["tel", "window", SUM, &f, "-s", "lib"],
        vec!["tel", "window", SUM, "x", &t, "-s", "lib"],
        vec!["tel", "window", SUM, &f, &t, "--qs", "0.5", "-s", "lib"],
        vec!["tel", "window", SUM, &f, &t, "--yes", "-s", "lib"],
        vec!["tel", "window", SUM, &f, &t, "--sketch", "x", "-s", "lib"],
    ] {
        assert_eq!(run(&dir, &args).status.code(), Some(2), "{args:?}");
    }
    // An accuracy out of range is the materialiser's refusal.
    let o = run(
        &dir,
        &["tel", "window", SUM, &f, &t, "--sketch", "0.9", "-s", "lib"],
    );
    assert_eq!(o.status.code(), Some(1));
}
