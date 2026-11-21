//! RFC SMYSL-5 GE-I1 and C-Tel-Info (TI-P1): can every statistic be replayed?
//!
//! Every recipe in `fixtures/tel/info/` over every series of its `series.tsv`, **twice** in one
//! process and compared byte for byte, and every outcome pinned in `expected.tsv`: the unit's
//! uid and the numbers it states, or the refusal's code. The file was written on aarch64 macOS;
//! CI runs this test on x86_64 Linux (glibc and musl), x86_64 Windows and aarch64 macOS, so a
//! statistic whose bytes differ on any of them fails there, which is GE-I1's stop condition:
//! "any byte difference keeps that statistic off until fixed".
//!
//! `SMYSL_BLESS=1` rewrites `expected.tsv`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use smysl_core::schemas::tel;
use smysl_core::{canonical_uid, to_cbor_seq, Decimal, Record, Uid};
use smysl_series::{MemBackend, Sample, SeriesInfo, Substrate};
use smysl_tel::info::{run_on, Outcome, Spec};

const FROM: u64 = 1_768_550_400_000;
const TO: u64 = FROM + 360 * 60_000;

const RECIPES: &[&str] = &[
    "ent-width",
    "ent-baseline",
    "ent-diff",
    "ent-increase",
    "perm-3",
    "perm-4-tau2",
    "ent-strict",
];

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/tel/info")
}

fn substrate() -> (Substrate<MemBackend>, Vec<Uid>) {
    let text = std::fs::read_to_string(dir().join("series.tsv")).unwrap();
    let inst = canonical_uid(&tel::instrument("scraper", "prom-scrape:ge-i1/sim:9100").unwrap());
    let mut by_key: BTreeMap<(String, String), Vec<Sample>> = BTreeMap::new();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = line.split('\t').collect();
        let s = Sample::new(
            f[2].parse().unwrap(),
            None,
            Decimal::parse_token(f[3]).unwrap(),
            f[4].parse().unwrap(),
        )
        .unwrap();
        by_key
            .entry((f[0].to_string(), f[1].to_string()))
            .or_default()
            .push(s);
    }
    let mut sub = Substrate::new(MemBackend::default());
    let mut series = Vec::new();
    for ((key, class), samples) in by_key {
        let uid = canonical_uid(&tel::series("prom", &key).unwrap());
        let info = SeriesInfo {
            scheme: "prom".into(),
            key,
            class,
        };
        sub.add(&uid, &inst, &info, &samples).unwrap();
        series.push(uid);
    }
    (sub, series)
}

fn spec(name: &str) -> Spec {
    Spec::parse(&std::fs::read_to_string(dir().join(format!("{name}.hjson"))).unwrap()).unwrap()
}

fn run_all() -> Vec<(&'static str, Vec<Outcome>)> {
    let (sub, series) = substrate();
    RECIPES
        .iter()
        .map(|name| {
            (
                *name,
                run_on(&spec(name), &sub, &series, None, FROM, TO).unwrap(),
            )
        })
        .collect()
}

fn report(runs: &[(&str, Vec<Outcome>)]) -> String {
    let mut out = String::from(
        "# RFC SMYSL-5 GE-I1 / C-Tel-Info: every recipe over fixtures/tel/info/, by uid.\n\
         # Written by crates/smysl-tel/tests/info_cases.rs; SMYSL_BLESS=1 rewrites it.\n\
         # case\tseries\tuid or `refused`\tvalue\tmm\tn\tk\tmissing, or the refusal\n",
    );
    for (name, outcomes) in runs {
        for o in outcomes {
            match &o.result {
                Ok(c) => writeln!(
                    out,
                    "{name}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    o.key,
                    canonical_uid(&c.unit).canonical(),
                    c.value,
                    c.mm,
                    c.n,
                    c.k,
                    c.missing
                ),
                Err(r) => writeln!(out, "{name}\t{}\trefused\t{}\t{}", o.key, r.code, r.message),
            }
            .unwrap();
        }
    }
    out
}

/// The bytes of every unit and edge, for comparing two runs.
fn bytes(runs: &[(&str, Vec<Outcome>)]) -> Vec<u8> {
    let records: Vec<Record> = runs
        .iter()
        .flat_map(|(_, os)| os.iter())
        .filter_map(|o| o.result.as_ref().ok())
        .flat_map(|c| {
            [
                Record::Unit(c.unit.clone()),
                Record::Relation(c.relation.clone()),
            ]
        })
        .collect();
    to_cbor_seq(&records)
}

#[test]
fn every_statistic_replays_to_the_byte_and_matches_its_pins() {
    let (first, second) = (run_all(), run_all());
    assert_eq!(bytes(&first), bytes(&second), "two runs, two byte strings");
    let got = report(&first);
    let path = dir().join("expected.tsv");
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::write(&path, &got).unwrap();
    }
    let want = std::fs::read_to_string(&path).expect("fixtures/tel/info/expected.tsv");
    assert_eq!(
        got, want,
        "an info unit's bytes moved; SMYSL_BLESS=1 rewrites expected.tsv, which C-Tel-Info pins"
    );
}

/// The refusals the fixture is built to show, each by its code.
#[test]
fn the_refusals_are_the_ones_the_fixture_plants() {
    let runs = run_all();
    let code = |case: &str, key: &str| {
        let (_, os) = runs.iter().find(|(n, _)| *n == case).unwrap();
        let o = os.iter().find(|o| o.key == key).unwrap();
        o.result.as_ref().err().map(|r| r.code.as_str())
    };
    // A counter's level and a histogram are not read (SMY-E701); a counter's increase is.
    assert_eq!(code("ent-width", "info_requests"), Some("SMY-E701"));
    assert_eq!(code("ent-width", "info_hist"), Some("SMY-E701"));
    assert_eq!(code("ent-increase", "info_requests"), None);
    // The noise's gap and stale marker leave points without a symbol: under cover 1/1, refused.
    assert_eq!(code("ent-strict", "info_noise"), Some("SMY-W703"));
    assert_eq!(code("ent-strict", "info_osc"), None);
    // A gauge's increase is missing wherever it falls (§5.2): the noise and the oscillation
    // fall half the time, and are refused for coverage.
    assert_eq!(code("ent-increase", "info_noise"), Some("SMY-W703"));
    assert_eq!(code("ent-increase", "info_osc"), Some("SMY-W703"));
    // m = 4 at τ = 2 wants 240 patterns; the noise's gaps leave 316, enough.
    assert_eq!(code("perm-4-tau2", "info_noise"), None);
    // Ranks are refused before any series is read.
    let rank = std::fs::read_to_string(dir().join("ent-rank.hjson")).unwrap();
    assert_eq!(Spec::parse(&rank).unwrap_err().code.as_str(), "SMY-E702");
}

/// A sample the window does not hold changes the digest and so the unit; one outside it does
/// not (§3, "What is read").
#[test]
fn only_the_windows_samples_enter_the_unit() {
    let (sub, series) = substrate();
    let osc = series
        .iter()
        .copied()
        .find(|s| sub.row(s).unwrap().unwrap().key == "info_osc")
        .unwrap();
    let s = spec("ent-width");
    let a = run_on(&s, &sub, &[osc], None, FROM, FROM + 300 * 60_000).unwrap();
    let mut sub2 = sub;
    let inst = sub2.instruments(&osc).unwrap()[0];
    let row = sub2.row(&osc).unwrap().unwrap();
    let info = SeriesInfo {
        scheme: row.scheme,
        key: row.key,
        class: row.class,
    };
    let late = Sample::new(TO + 60_000, None, Decimal::from_i64(99), 0).unwrap();
    sub2.add(&osc, &inst, &info, &[late]).unwrap();
    let b = run_on(&s, &sub2, &[osc], None, FROM, FROM + 300 * 60_000).unwrap();
    let uid = |o: &[Outcome]| canonical_uid(&o[0].result.as_ref().unwrap().unit);
    assert_eq!(uid(&a), uid(&b));
}
