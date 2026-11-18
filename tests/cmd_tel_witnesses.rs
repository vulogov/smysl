//! `smysl tel witnesses` (RFC SMYSL-3 §7, TL-P7 step 3), against the binary: how many
//! independent witnesses one quantity has, from what the readers recorded and what people
//! declared.
//!
//! - A Prometheus series and a Zabbix item declared the same quantity: two origins, two
//!   witnesses; a store both reach is common transport, and merges nothing.
//! - A recording rule (a `transform`) on both lineages makes them one witness.
//! - A target scraped by a second Prometheus is one witness carried twice.

#![cfg(all(feature = "cli", feature = "tel"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");
const T0: u64 = 1_768_557_600_000;
const REQUESTS: &str =
    r#"prom:http_requests_total{code="200",instance="h:9100",job="node",method="get"}"#;
const QPS: &str = "zabbix:host:DB 01/item:mysql.qps";

fn fixture(rel: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/tel")
        .join(rel)
        .display()
        .to_string()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-tel-witness-{}-{name}", std::process::id()));
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

fn declare(dir: &Path, args: &[&str]) {
    let mut a = vec!["tel", "declare"];
    a.extend_from_slice(args);
    let at = (T0 + 100_000).to_string();
    a.extend(["-s", "lib", "--as", "human:alice", "--at", &at]);
    ok(dir, &a);
}

fn witnesses(dir: &Path, series: &str) -> String {
    ok(dir, &["tel", "witnesses", series, "-s", "lib"])
}

/// A Prometheus scrape and a Zabbix history read, their series one quantity, both kept in one
/// store.
fn library(name: &str) -> PathBuf {
    let dir = scratch(name);
    let t0 = T0.to_string();
    let s0 = fixture("prom/s0.prom");
    ok(
        &dir,
        &[
            "tel",
            "read",
            "prom/1",
            &s0,
            "--job",
            "node",
            "--instance",
            "h:9100",
            "--at",
            &t0,
            "-s",
            "lib",
            "--yes",
        ],
    );
    let (a, items) = (
        fixture("zabbix/history-a.json"),
        fixture("zabbix/items.json"),
    );
    ok(
        &dir,
        &[
            "tel",
            "read",
            "zabbix/1",
            &a,
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
    declare(&dir, &["instrument", "store", "thanos:eu"]);
    declare(&dir, &["via", "prom-scrape:node/h:9100", "thanos:eu"]);
    declare(&dir, &["via", "zabbix-server:zbx-main", "thanos:eu"]);
    declare(&dir, &["same-quantity", REQUESTS, QPS]);
    dir
}

#[test]
fn two_origins_are_two_witnesses_and_a_shared_store_is_reported() {
    let dir = library("two");
    let w = witnesses(&dir, REQUESTS);
    assert!(w.contains("quantity      2 series"), "{w}");
    assert!(w.contains("witnesses     2"), "{w}");
    assert!(w.contains("measured by prom-target:node/h:9100"), "{w}");
    assert!(w.contains("measured by zabbix-agent:DB 01"), "{w}");
    assert!(w.contains("common        1"), "{w}");
    assert!(w.contains("store thanos:eu"), "{w}");
    assert!(w.contains("unmeasured    0"), "{w}");
    assert_eq!(
        witnesses(&dir, QPS),
        w,
        "the quantity's report, from either series"
    );
}

#[test]
fn a_recording_rule_on_both_lineages_is_one_witness() {
    let dir = library("rule");
    declare(&dir, &["instrument", "transform", "rule:qps:sum"]);
    declare(&dir, &["via", "prom-target:node/h:9100", "rule:qps:sum"]);
    declare(&dir, &["via", "zabbix-agent:DB 01", "rule:qps:sum"]);
    let w = witnesses(&dir, REQUESTS);
    assert!(w.contains("witnesses     1"), "{w}");
}

#[test]
fn a_second_scraper_is_one_witness_carried_twice() {
    let dir = library("twice");
    declare(&dir, &["instrument", "scraper", "prom-scrape:replica-b"]);
    declare(
        &dir,
        &["via", "prom-target:node/h:9100", "prom-scrape:replica-b"],
    );
    let w = witnesses(&dir, REQUESTS);
    assert!(w.contains("witnesses     2"), "{w}");
    assert!(w.contains("duplicated    1"), "{w}");
    assert!(w.contains("via "), "{w}");
    assert!(w.contains("scraper prom-scrape:replica-b"), "{w}");

    // Usage: one series, and it must be here.
    assert_eq!(
        run(&dir, &["tel", "witnesses", "-s", "lib"]).status.code(),
        Some(2)
    );
    assert_eq!(
        run(&dir, &["tel", "witnesses", "prom:absent", "-s", "lib"])
            .status
            .code(),
        Some(1)
    );
}
