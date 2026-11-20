//! GE-M3 (RFC SMYSL-3 §26, H-7): can the analysis layer be replayed?
//!
//! Every detector TL-P5 ships, over `fixtures/tel/detect/`, **twice** in one process and
//! compared, and every output pinned by uid in `expected.tsv`. The file was written on aarch64;
//! CI runs this test on x86_64 Linux (glibc and musl), x86_64 Windows and aarch64 macOS, so a
//! detector whose bytes differ on any of them fails there. GE-M3's stop condition is exactly
//! that: "any byte difference in a derived unit keeps that detector off the pure path".
//!
//! The data is four series of 240 one-minute samples (`series.tsv`, from `gen_series.py`): a
//! level shift, a spike, a dip and a constant that changes. Episodes group the robust z's
//! anomalies; the ranking is taken at the spike.
//!
//! `SMYSL_BLESS=1` rewrites `expected.tsv`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use smysl_core::schemas::tel;
use smysl_core::{canonical_uid, Decimal, Uid};
use smysl_series::{MemBackend, Sample, SeriesInfo, Substrate};
use smysl_tel::detect::{run_on, Detected, Spec};

const FROM: u64 = 1_768_550_400_000;
const TO: u64 = FROM + 240 * 60_000;

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/tel/detect")
}

fn substrate() -> (Substrate<MemBackend>, Vec<Uid>) {
    let text = std::fs::read_to_string(dir().join("series.tsv")).unwrap();
    let inst = canonical_uid(&tel::instrument("scraper", "prom-scrape:ge-m3/sim:9100").unwrap());
    let mut by_key: BTreeMap<String, Vec<Sample>> = BTreeMap::new();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let mut f = line.split('\t');
        let (k, t, v) = (f.next().unwrap(), f.next().unwrap(), f.next().unwrap());
        let s = Sample::new(
            t.parse().unwrap(),
            None,
            Decimal::parse_token(v).unwrap(),
            0,
        )
        .unwrap();
        by_key.entry(k.to_string()).or_default().push(s);
    }
    let mut sub = Substrate::new(MemBackend::default());
    let mut series = Vec::new();
    for (k, samples) in by_key {
        let uid = canonical_uid(&tel::series("prom", &k).unwrap());
        let info = SeriesInfo {
            scheme: "prom".into(),
            key: k,
            class: "gauge".into(),
        };
        sub.add(&uid, &inst, &info, &samples).unwrap();
        series.push(uid);
    }
    (sub, series)
}

fn spec(name: &str) -> Spec {
    Spec::parse(&std::fs::read_to_string(dir().join(format!("{name}.hjson"))).unwrap()).unwrap()
}

/// Every detector, in order; episodes over the robust z's anomalies.
fn run_all() -> Vec<(&'static str, Detected)> {
    let (sub, series) = substrate();
    let mut out = Vec::new();
    for name in ["cusum", "pelt", "mad"] {
        out.push((
            name,
            run_on(&spec(name), &sub, &series, FROM, TO, &[], None).unwrap(),
        ));
    }
    let anomalies = out[2].1.units.clone();
    out.push((
        "episode",
        run_on(&spec("episode"), &sub, &series, FROM, TO, &anomalies, None).unwrap(),
    ));
    let trigger = out[3].1.units.first().map(canonical_uid);
    out.push((
        "baro",
        run_on(&spec("baro"), &sub, &series, FROM, TO, &[], trigger).unwrap(),
    ));
    out
}

fn report(runs: &[(&str, Detected)]) -> String {
    let mut out = String::from(
        "# RFC SMYSL-3 §26 GE-M3: every TL-P5 detector over fixtures/tel/detect/, by uid.\n\
         # Written by crates/smysl-tel/tests/ge_m3.rs; SMYSL_BLESS=1 rewrites it.\n",
    );
    for (name, d) in runs {
        writeln!(
            out,
            "detector\t{name}\t{} unit(s)\t{} edge(s)",
            d.units.len(),
            d.relations.len()
        )
        .unwrap();
        for u in &d.units {
            writeln!(
                out,
                "unit\t{}\t{}\t{}",
                u.schema.as_str(),
                canonical_uid(u).canonical(),
                u.gist
            )
            .unwrap();
        }
        for r in &d.relations {
            writeln!(out, "edge\t{}\t{}", r.kind.as_str(), r.uid().canonical()).unwrap();
        }
    }
    out
}

#[test]
fn every_detector_replays_to_the_byte() {
    let first = run_all();
    let second = run_all();
    for ((name, a), (_, b)) in first.iter().zip(&second) {
        assert_eq!(
            a.units
                .iter()
                .map(smysl_core::unit_core_bytes)
                .collect::<Vec<_>>(),
            b.units
                .iter()
                .map(smysl_core::unit_core_bytes)
                .collect::<Vec<_>>(),
            "{name} is not the same bytes twice"
        );
        assert!(!a.units.is_empty(), "{name} found nothing in its fixture");
    }
    let got = report(&first);
    let path = dir().join("expected.tsv");
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::write(&path, &got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).expect("expected.tsv; SMYSL_BLESS=1 writes it");
    assert!(
        got == want,
        "fixtures/tel/detect/expected.tsv moved:\n{got}"
    );
}
