//! Substrate exchange (RFC SMYSL-3 §19.5): export, import, eviction and tampering.
//!
//! Outside `src/` because it needs a directory, and `xtask check-purity` keeps the environment,
//! `std::env::temp_dir` included, out of the crate's own source.

use std::fs;

use smysl_core::{Decimal, Uid};
use smysl_series::exchange::{export, file_name, import, DAY_MS};
use smysl_series::{Error, MemBackend, Sample, SeriesInfo, Substrate};

fn uid(b: u8) -> Uid {
    Uid::from_bytes([b; 32])
}

fn info(k: &str) -> SeriesInfo {
    SeriesInfo {
        scheme: "prom".into(),
        key: k.into(),
        class: "gauge".into(),
    }
}

fn v(ts: u64, x: i64) -> Sample {
    Sample::value(ts, Decimal::from_i64(x))
}

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "smysl-series-exchange-{}-{name}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&d);
    d
}

fn filled(days: u64) -> Substrate<MemBackend> {
    let mut s = Substrate::new(MemBackend::default());
    for (series, inst) in [(1, 9), (2, 9), (2, 8)] {
        let samples: Vec<Sample> = (0..days * 24 * 4)
            .map(|n| v(n * 900_000, (n as i64) * i64::from(series)))
            .collect();
        s.add(
            &uid(series),
            &uid(inst),
            &info(&series.to_string()),
            &samples,
        )
        .unwrap();
    }
    s
}

fn all(s: &Substrate<MemBackend>) -> Vec<(Uid, Uid, Vec<Sample>)> {
    let mut out = Vec::new();
    for (series, _) in s.series(None).unwrap() {
        for inst in s.instruments(&series).unwrap() {
            out.push((
                series,
                inst,
                s.window(&series, &inst, 0, u64::MAX).unwrap().samples,
            ));
        }
    }
    out
}

#[test]
fn a_round_trip_is_the_same_samples_and_a_second_import_adds_nothing() {
    let src = filled(3);
    let dir = scratch("round");
    let l = export(&src, &dir).unwrap();
    assert_eq!(l.held.len(), 9, "three streams of three days");
    assert_eq!(l.series.len(), 2);
    assert!(l.evicted.is_empty());
    let mut dst = Substrate::new(MemBackend::default());
    let r = import(&mut dst, &dir).unwrap();
    assert_eq!((r.files, r.already, r.conflicts), (9, 0, 0));
    assert_eq!(all(&dst), all(&src));
    let again = import(&mut dst, &dir).unwrap();
    assert_eq!((again.added, again.already), (0, r.added), "idempotent");
    // The same samples write the same files: the export is a function of the samples.
    let dir2 = scratch("round2");
    assert_eq!(export(&dst, &dir2).unwrap(), l);
}

#[test]
fn eviction_is_listed_and_a_peer_brings_the_samples_back() {
    let peer = filled(3);
    let mut mine = filled(3);
    mine.evict(DAY_MS, 5).unwrap();
    let dir = scratch("evicted");
    let l = export(&mine, &dir).unwrap();
    assert_eq!(l.held.len(), 6);
    assert_eq!(l.evicted.len(), 3, "day 0 of each stream: {:?}", l.evicted);
    assert!(l.evicted.iter().all(|e| e.2 == 0));

    // My export into the peer adds nothing; the peer's into mine restores day 0.
    let mut p = peer.clone();
    assert_eq!(import(&mut p, &dir).unwrap().added, 0);
    let pdir = scratch("peer");
    export(&peer, &pdir).unwrap();
    let r = import(&mut mine, &pdir).unwrap();
    assert_eq!(r.added, 3 * 96);
    assert_eq!(all(&mine), all(&peer));
    assert!(
        mine.evictions().unwrap().is_empty(),
        "the spans are held again"
    );
}

#[test]
fn a_file_that_is_not_its_name_adds_nothing() {
    let src = filled(2);
    let dir = scratch("tamper");
    let l = export(&src, &dir).unwrap();
    let victim = dir.join(file_name(&l.held[3].name));
    let mut b = fs::read(&victim).unwrap();
    let last = b.len() - 1;
    b[last] ^= 1;
    fs::write(&victim, b).unwrap();
    let mut dst = Substrate::new(MemBackend::default());
    assert!(matches!(import(&mut dst, &dir), Err(Error::Corrupt(_))));
    assert!(dst.series(None).unwrap().is_empty(), "nothing was added");
}
