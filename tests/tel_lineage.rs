//! Instrument lineage (RFC SMYSL-3 §4.2, §7, TL-P7 step 1): what each reader says measured a
//! series, and `SMY-W609` for a series nothing says measured.
//!
//! - `prom/1`: the target (`prom-target:<job>/<instance>`, an exporter) measured every series of
//!   its scrape, and the scraper carried them.
//! - `otlp/1`: the resource's process (`otel-sdk:<namespace>/<name>/<instance>`, an agent), when
//!   the resource has a `service.name`, via the collector the export was read from.
//! - `zabbix/1`: the item's host agent (`zabbix-agent:<host>`), via the server.

#![cfg(all(feature = "cli", feature = "tel"))]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use smysl::schemas::tel;
use smysl::text::library::Library;
use smysl::{canonical_uid, AgentId, Code, Hlc, Rung, Store, StoreRead, Uid};

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
    let dir = std::env::temp_dir().join(format!("smysl-tel-lineage-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    let o = Command::new(BIN)
        .current_dir(dir)
        .args(args)
        .output()
        .expect("the binary runs");
    assert_eq!(
        o.status.code(),
        Some(0),
        "{args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    o
}

fn inst(role: &str, key: &str) -> Uid {
    canonical_uid(&tel::instrument(role, key).unwrap())
}

/// A library's series, its `measured-by` edges, and its `via` edges.
type Lineage = (BTreeSet<Uid>, Vec<(Uid, Uid)>, BTreeSet<(Uid, Uid)>);

/// The library's series, `measured-by` edges and `via` edges.
fn lineage(dir: &Path) -> Lineage {
    let lib = Library::open(&dir.join("lib")).expect("the library");
    let store = lib.store();
    let series: BTreeSet<Uid> = StoreRead::units(store)
        .filter(|(_, u)| u.core.schema.as_str() == tel::SERIES)
        .map(|(uid, _)| *uid)
        .collect();
    let (mut measured, mut via) = (Vec::new(), BTreeSet::new());
    for r in store.relations() {
        match r.kind.as_str() {
            tel::MEASURED_BY => measured.push((r.from, r.to)),
            tel::VIA => {
                via.insert((r.from, r.to));
            }
            _ => {}
        }
    }
    (series, measured, via)
}

fn no_w609(dir: &Path) {
    let o = Command::new(BIN)
        .current_dir(dir)
        .args(["check", "-s", "lib"])
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr);
    assert!(!text.contains("SMY-W609"), "{text}");
}

#[test]
fn a_prometheus_target_measures_its_series_and_the_scraper_carries_them() {
    let dir = scratch("prom");
    for n in 0..2u64 {
        let f = fixture(&format!("prom/s{n}.prom"));
        let at = (T0 + 15_000 * n).to_string();
        run(
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
                &at,
                "-s",
                "lib",
                "--yes",
            ],
        );
    }
    let (series, measured, via) = lineage(&dir);
    let target = inst("exporter", "prom-target:node/h:9100");
    assert!(!series.is_empty());
    let from: BTreeSet<Uid> = measured.iter().map(|m| m.0).collect();
    assert_eq!(from, series, "every series is measured by something");
    assert!(measured.iter().all(|m| m.1 == target));
    assert_eq!(measured.len(), series.len(), "one edge per series");
    let scraper = inst("scraper", "prom-scrape:node/h:9100");
    assert_eq!(via, BTreeSet::from([(target, scraper)]));
    no_w609(&dir);
}

#[test]
fn an_otlp_resource_and_a_zabbix_host_are_origins() {
    let dir = scratch("otlp-zbx");
    let t0 = T0.to_string();
    let e: Vec<String> = (0..3)
        .map(|n| fixture(&format!("otlp/e{n}.json")))
        .collect();
    let mut args = vec!["tel", "read", "otlp-json/1"];
    args.extend(e.iter().map(String::as_str));
    args.extend([
        "--instrument",
        "otel-collector:c1",
        "--at",
        &t0,
        "-s",
        "lib",
        "--yes",
    ]);
    run(&dir, &args);
    let (a, items) = (
        fixture("zabbix/history-a.json"),
        fixture("zabbix/items.json"),
    );
    run(
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
    let (series, measured, via) = lineage(&dir);
    let from: BTreeSet<Uid> = measured.iter().map(|m| m.0).collect();
    assert_eq!(from, series);
    let sdk = inst("agent", "otel-sdk:/cart/c1");
    let collector = inst("collector", "otel-collector:c1");
    assert!(via.contains(&(sdk, collector)), "{via:?}");
    let server = inst("collector", "zabbix-server:zbx-main");
    let agents: BTreeSet<Uid> = via
        .iter()
        .filter(|(_, to)| *to == server)
        .map(|(f, _)| *f)
        .collect();
    assert!(
        agents.contains(&inst("agent", "zabbix-agent:DB 01")),
        "{via:?}"
    );
    no_w609(&dir);
}

/// A series nothing says measured is `SMY-W609`, and only that one.
#[test]
fn a_series_with_no_origin_is_w609() {
    let a = tel::series("prom", "up{job=\"a\"}").unwrap();
    let b = tel::series("prom", "up{job=\"b\"}").unwrap();
    let i = tel::instrument("exporter", "prom-target:a/h").unwrap();
    let (ua, ub, ui) = (canonical_uid(&a), canonical_uid(&b), canonical_uid(&i));
    let agent = AgentId::new("tool:t").unwrap();
    let attest = smysl::stage::Attest::new(agent.clone(), Rung::Computed, Hlc::new(1, 0, agent));
    let staged = smysl::stage::prepare_declared(
        &Store::new(),
        vec![a, b, i],
        vec![tel::measured_by(ua, ui)],
        Default::default(),
        vec![tel::decl()],
        &attest,
    );
    assert!(!staged.has_errors(), "{:?}", staged.report.diagnostics);
    let store = Store::from_records(staged.records());
    let report = smysl::check(&store, smysl::CheckOptions::default());
    let w609: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.code == Code::W609)
        .map(|d| d.subject.clone())
        .collect();
    assert_eq!(
        w609,
        vec![smysl::Subject::Unit(ub)],
        "{:?}",
        report.diagnostics
    );
    assert_eq!(
        tel::unmeasured([(&ua, &tel::series("prom", "up{job=\"a\"}").unwrap())], []),
        vec![ua]
    );
}

/// `same-quantity` is symmetric, so it is written one way: two declarations in opposite orders
/// are one relation.
#[test]
fn same_quantity_is_one_relation_either_way() {
    let a = canonical_uid(&tel::series("prom", "x").unwrap());
    let b = canonical_uid(&tel::series("otel", "y").unwrap());
    let (ab, ba) = (tel::same_quantity(a, b), tel::same_quantity(b, a));
    assert_eq!(ab, ba);
    assert!(ab.from < ab.to);
}
