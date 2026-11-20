//! `smysl tel read` with the series readers, and `smysl tel at` (RFC SMYSL-3 §19.3, §23,
//! TL-P3 step 5), against the binary.
//!
//! - A review (no `--yes`) writes nothing, not even `series.redb`, and stages the units it
//!   would commit. Its files are read after one another, so its facts are the writing run's.
//! - `--yes` adds the samples to `<library>/index/series.redb` and commits the units.
//! - Reading the same export again adds nothing.
//! - `tel at` answers from the substrate, by uid or by `<scheme>:<key>`, and prints a stale
//!   sample as stale.
//! - OTLP in either encoding and Zabbix history land in the same substrate.

#![cfg(all(feature = "cli", feature = "tel"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");
const T0: u64 = 1_768_557_600_000;

fn fixture(rel: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/tel")
        .join(rel)
        .display()
        .to_string()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-tel-series-{}-{name}", std::process::id()));
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

fn scrape(dir: &Path, n: u64, yes: bool) -> Output {
    let file = fixture(&format!("prom/s{n}.prom"));
    let at = (T0 + 15_000 * n).to_string();
    let mut args = vec![
        "tel",
        "read",
        "prom/1",
        &file,
        "--job",
        "node",
        "--instance",
        "h:9100",
        "--at",
        &at,
        "-s",
        "lib",
    ];
    if yes {
        args.push("--yes");
    }
    run(dir, &args)
}

const REQUESTS: &str =
    r#"prom:http_requests_total{code="200",instance="h:9100",job="node",method="get"}"#;

fn at(dir: &Path, series: &str, t: u64) -> Output {
    run(dir, &["tel", "at", series, &t.to_string(), "-s", "lib"])
}

#[test]
fn a_review_writes_nothing_and_yes_adds_the_samples() {
    let dir = scratch("review");
    let o = scrape(&dir, 0, false);
    assert_eq!(
        o.status.code(),
        Some(10),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(
        out(&o).contains("10 sample(s) would be added"),
        "{}",
        out(&o)
    );
    assert!(!dir.join("lib").exists(), "a review creates no library");

    let o = scrape(&dir, 0, true);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(out(&o).contains("10 sample(s) added"), "{}", out(&o));
    assert!(dir.join("lib/index/series.redb").exists());

    // Again: a set, so nothing more.
    let o = scrape(&dir, 0, true);
    assert!(out(&o).contains("0 sample(s) added"), "{}", out(&o));
}

#[test]
fn the_value_in_force_by_key_and_by_uid() {
    let dir = scratch("at");
    for n in 0..4 {
        let o = scrape(&dir, n, true);
        assert_eq!(
            o.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
    let o = at(&dir, REQUESTS, T0 + 31_000);
    assert_eq!(o.status.code(), Some(0));
    let text = out(&o);
    assert!(text.contains(&format!("{}  12", T0 + 30_000)), "{text}");
    // The same series by its uid.
    let uid = text
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();
    assert_eq!(
        out(&at(&dir, &uid, T0 + 31_000)).lines().nth(1),
        text.lines().nth(1)
    );

    // Before the first sample: nothing, exit 1.
    assert_eq!(at(&dir, REQUESTS, T0 - 1).status.code(), Some(1));
    // A series that went away is stale, not its last number.
    let gone = r#"prom:node_temperature_celsius{instance="h:9100",job="node",sensor="b"}"#;
    assert!(out(&at(&dir, gone, T0 + 20_000)).contains("stale"));
}

#[test]
fn otlp_in_either_encoding_and_zabbix_history() {
    let dir = scratch("otlp");
    let t0 = T0.to_string();
    let e = |ext: &str| {
        (0..3)
            .map(|n| fixture(&format!("otlp/e{n}.{ext}")))
            .collect::<Vec<_>>()
    };
    let (pb, js) = (e("pb"), e("json"));
    let mut args = vec!["tel", "read", "otlp/1"];
    args.extend(pb.iter().map(String::as_str));
    args.extend([
        "--instrument",
        "otel-collector:c1",
        "--at",
        &t0,
        "-s",
        "lib",
        "--yes",
    ]);
    let o = run(&dir, &args);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(out(&o).contains("45 sample(s) added"), "{}", out(&o));
    // The JSON twins are the same samples.
    let mut args = vec!["tel", "read", "otlp-json/1"];
    args.extend(js.iter().map(String::as_str));
    args.extend([
        "--instrument",
        "otel-collector:c1",
        "--at",
        &t0,
        "-s",
        "lib",
        "--yes",
    ]);
    assert!(out(&run(&dir, &args)).contains("0 sample(s) added to the substrate, 45 already there"));

    let (a, b, items) = (
        fixture("zabbix/history-a.json"),
        fixture("zabbix/history-b.json"),
        fixture("zabbix/items.json"),
    );
    let o = run(
        &dir,
        &[
            "tel",
            "read",
            "zabbix/1",
            &a,
            &b,
            "--items",
            &items,
            "--zabbix-api",
            "7.0",
            "--instrument",
            "zabbix-server:zbx-main",
            "--at",
            &t0,
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
    assert!(String::from_utf8_lossy(&o.stderr).contains("not in --items: 99"));
    let qps = "zabbix:host:DB 01/item:mysql.qps";
    assert!(out(&at(&dir, qps, T0 + 61_000)).contains(&format!("{}  99.25", T0 + 60_500)));
}

#[test]
fn what_each_reader_needs_is_required() {
    let dir = scratch("usage");
    let s0 = fixture("prom/s0.prom");
    let t0 = T0.to_string();
    for args in [
        vec!["tel", "read", "prom/1", &s0, "--at", &t0, "-s", "lib"],
        vec![
            "tel",
            "read",
            "prom/1",
            &s0,
            "--job",
            "j",
            "--instance",
            "i",
            "-s",
            "lib",
        ],
        vec![
            "tel",
            "read",
            "prom/1",
            &s0,
            &s0,
            "--job",
            "j",
            "--instance",
            "i",
            "--at",
            &t0,
            "-s",
            "lib",
        ],
        vec!["tel", "read", "otlp/1", &s0, "--at", &t0, "-s", "lib"],
        vec!["tel", "at", REQUESTS, "-s", "lib"],
        vec!["tel", "at", "nonsense", "1", "-s", "lib"],
    ] {
        assert_eq!(run(&dir, &args).status.code(), Some(2), "{args:?}");
    }
    // A library with no substrate yet.
    assert_eq!(at(&dir, REQUESTS, T0).status.code(), Some(1));
}
