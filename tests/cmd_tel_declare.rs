//! `smysl tel declare` (RFC SMYSL-3 §4.1, §4.2, §7, TL-P7 step 2), against the binary: lineage
//! a person states, which no export can.
//!
//! - An instrument, with its version as `x.tel/instrument-meta`.
//! - `via` between instruments named by key; `same-quantity` between series named by
//!   `<scheme>:<key>`, one relation whichever order it is declared in.
//! - `measured-by` for a series the reader could not give an origin, which clears `SMY-W609`.
//! - What is not in the library is refused by name, and so is a key two roles share.

#![cfg(all(feature = "cli", feature = "tel"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use smysl::schemas::tel;
use smysl::text::library::Library;
use smysl::StoreRead;

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
    let dir = std::env::temp_dir().join(format!("smysl-tel-declare-{}-{name}", std::process::id()));
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

fn declare(dir: &Path, args: &[&str], at: u64) -> Output {
    let mut a = vec!["tel", "declare"];
    a.extend_from_slice(args);
    let at = (T0 + at).to_string();
    a.extend(["-s", "lib", "--as", "human:alice", "--at", &at]);
    run(dir, &a)
}

/// A Prometheus scrape and a Zabbix history read, in one library.
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
    dir
}

fn count(dir: &Path, kind: &str) -> usize {
    let lib = Library::open(&dir.join("lib")).unwrap();
    lib.store()
        .relations()
        .filter(|r| r.kind.as_str() == kind)
        .count()
}

#[test]
fn lineage_a_person_states() {
    let dir = library("states");
    let o = declare(
        &dir,
        &["instrument", "store", "thanos:eu", "0.36.1"],
        100_000,
    );
    assert_eq!(o.status.code(), Some(0));
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(text.contains("x.tel/instrument-meta"), "{text}");
    assert!(text.contains("thanos:eu is version 0.36.1"), "{text}");
    let lib = Library::open(&dir.join("lib")).unwrap();
    assert!(
        StoreRead::units(lib.store()).any(|(_, u)| u.core.schema.as_str() == tel::INSTRUMENT_META)
    );
    drop(lib);

    let via = count(&dir, tel::VIA);
    let o = declare(
        &dir,
        &["via", "prom-scrape:node/h:9100", "thanos:eu"],
        101_000,
    );
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(count(&dir, tel::VIA), via + 1);

    let o = declare(&dir, &["same-quantity", REQUESTS, QPS], 102_000);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let o = declare(&dir, &["same-quantity", QPS, REQUESTS], 103_000);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(
        count(&dir, tel::SAME_QUANTITY),
        1,
        "one relation, either order"
    );
}

/// A series the reader gave no origin is `SMY-W609` until someone says what measured it.
#[test]
fn measured_by_clears_w609() {
    let dir = scratch("w609");
    // An OTLP resource without `service.name`: the reader does not guess an origin.
    let export = dir.join("anon.json");
    std::fs::write(
        &export,
        r#"{"resourceMetrics":[{"resource":{"attributes":[]},"scopeMetrics":[{"scope":{"name":"s","version":"1"},"metrics":[{"name":"temp","gauge":{"dataPoints":[{"timeUnixNano":"1768557600000000000","asDouble":21.5}]}}]}]}]}"#,
    )
    .unwrap();
    let t0 = T0.to_string();
    let f = export.display().to_string();
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
    let check = |dir: &Path| {
        let o = run(dir, &["check", "-s", "lib"]);
        String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr)
    };
    assert!(check(&dir).contains("SMY-W609"), "{}", check(&dir));
    let lib = Library::open(&dir.join("lib")).unwrap();
    let series = StoreRead::units(lib.store())
        .find(|(_, u)| u.core.schema.as_str() == tel::SERIES)
        .map(|(uid, _)| uid.canonical())
        .unwrap();
    drop(lib);
    let o = declare(&dir, &["instrument", "agent", "sensor:lab-3"], 1_000);
    assert_eq!(o.status.code(), Some(0));
    let o = declare(&dir, &["measured-by", &series, "sensor:lab-3"], 2_000);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(!check(&dir).contains("SMY-W609"), "{}", check(&dir));
}

#[test]
fn what_is_not_here_is_refused() {
    let dir = library("refused");
    // Two roles, one key.
    for role in ["store", "pipeline"] {
        assert_eq!(
            declare(&dir, &["instrument", role, "vm:1"], 1)
                .status
                .code(),
            Some(0)
        );
    }
    let cases: &[(&[&str], i32, &str)] = &[
        (&["via", "nope", "vm:1"], 1, "declare it first"),
        (
            &["via", "zabbix-server:zbx-main", "vm:1"],
            1,
            "2 instruments",
        ),
        (&["same-quantity", REQUESTS, "prom:absent"], 1, "no series"),
        (
            &["same-quantity", REQUESTS, REQUESTS],
            2,
            "its own quantity",
        ),
        (
            &["measured-by", REQUESTS],
            2,
            "measured-by SERIES INSTRUMENT",
        ),
        (&["instrument", "probe", "x"], 2, "role"),
        (&["frobnicate", "a", "b"], 2, "not declarable"),
    ];
    for (args, code, why) in cases {
        let o = declare(&dir, args, 2);
        assert_eq!(o.status.code(), Some(*code), "{args:?}");
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(err.contains(why), "{args:?}: {err}");
    }
    let o = run(&dir, &["tel", "declare", "via", "a", "b", "-s", "lib"]);
    assert_eq!(o.status.code(), Some(2), "who and when are required");
}
