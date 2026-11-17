//! The behaviour every substrate backend must have (RFC SMYSL-3 §19.3), run against each.
//!
//! Outside `src/` because the redb half needs a directory, and `xtask check-purity` keeps the
//! environment, `std::env::temp_dir` included, out of the crate's own source.

use smysl_core::{Decimal, Uid};
use smysl_series::{digest, Backend, MemBackend, Sample, SeriesInfo, Span, Substrate};

fn uid(n: u8) -> Uid {
    Uid::from_bytes([n; 32])
}

fn info() -> SeriesInfo {
    SeriesInfo {
        scheme: "prom".into(),
        key: r#"up{job="node"}"#.into(),
        class: "gauge".into(),
    }
}

fn v(ts: u64, n: i64) -> Sample {
    Sample::value(ts, Decimal::from_i64(n))
}

/// The behaviour every backend must have, run against each.
fn contract<B: Backend>(mut sub: Substrate<B>) {
    let (s, i, other) = (uid(1), uid(2), uid(3));
    // 15 s scrapes: a 15 min span.
    let first: Vec<Sample> = (0..4).map(|k| v(1_000 + k * 15_000, k as i64)).collect();
    let r = sub.add(&s, &i, &info(), &first).unwrap();
    assert_eq!((r.added, r.already), (4, 0));
    let row = sub.row(&s).unwrap().unwrap();
    assert_eq!(
        (row.span, row.first_ms, row.last_ms, row.count),
        (Span::M15, 1_000, 46_000, 4)
    );

    // A set: again is nothing, and the digest does not move.
    let w1 = sub.window(&s, &i, 0, 60_000).unwrap();
    let r = sub.add(&s, &i, &info(), &first).unwrap();
    assert_eq!((r.added, r.already), (0, 4));
    assert_eq!(sub.window(&s, &i, 0, 60_000).unwrap(), w1);
    assert_eq!(w1.samples, first);
    assert_eq!(w1.digest, digest(&first));

    // Late data, across a span boundary, in any order.
    let late = [v(901_000, 9), v(31_500, 5), v(16_000, 7)];
    let r = sub.add(&s, &i, &info(), &late).unwrap();
    assert_eq!(r.added, 3);
    assert_eq!(r.conflicts, vec![(16_000, 0)], "16 000 now holds 1 and 7");
    let w = sub.window(&s, &i, 0, 1_000_000).unwrap();
    assert_eq!(w.samples.len(), 7);
    assert!(w.samples.windows(2).all(|p| p[0] < p[1]));
    assert_eq!(w.conflicts, vec![(16_000, 0)]);
    assert_eq!(sub.row(&s).unwrap().unwrap().last_ms, 901_000);

    // [from, to): the end is open.
    assert_eq!(sub.window(&s, &i, 1_000, 16_000).unwrap().samples.len(), 1);
    assert!(sub.window(&s, &i, 5, 5).unwrap().samples.is_empty());

    // The value in force: within a span, before its first sample, and before everything.
    assert_eq!(sub.at(&s, &i, 46_000).unwrap(), Some(v(46_000, 3)));
    assert_eq!(sub.at(&s, &i, 45_999).unwrap(), Some(v(31_500, 5)));
    assert_eq!(sub.at(&s, &i, 900_500).unwrap(), Some(v(46_000, 3)));
    assert_eq!(sub.at(&s, &i, 10_000_000).unwrap(), Some(v(901_000, 9)));
    assert_eq!(sub.at(&s, &i, 999).unwrap(), None);
    assert_eq!(sub.at(&s, &other, 46_000).unwrap(), None, "per instrument");

    // Coverage.
    let c = sub.present(&s, &i, 0, 2_000_000).unwrap();
    assert_eq!(c.present, vec![0, 900_000]);
    assert!(sub
        .present(&s, &i, 1_800_000, 2_000_000)
        .unwrap()
        .is_absent());

    // The series table.
    let all = sub.series(None).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].1.key, info().key);
    assert!(sub.series(Some("otel")).unwrap().is_empty());

    // Instruments: one seek each, whatever the chunks between them.
    assert_eq!(sub.instruments(&s).unwrap(), vec![i]);
    sub.add(&s, &uid(0), &info(), &[v(5, 1)]).unwrap();
    sub.add(&s, &uid(9), &info(), &[v(5, 1), v(9_000_000, 2)])
        .unwrap();
    sub.add(&uid(4), &i, &info(), &[v(5, 1)]).unwrap();
    assert_eq!(sub.instruments(&s).unwrap(), vec![uid(0), i, uid(9)]);
    assert!(sub.instruments(&other).unwrap().is_empty());
}

#[test]
fn the_memory_backend_keeps_the_contract() {
    contract(Substrate::new(MemBackend::default()));
}

#[cfg(feature = "store-redb")]
mod redb {
    use super::*;
    use smysl_series::redb::RedbBackend;

    fn path(name: &str) -> std::path::PathBuf {
        let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("smysl-series-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("series.redb")
    }

    #[test]
    fn the_redb_backend_keeps_the_contract() {
        let p = path("contract");
        contract(Substrate::new(RedbBackend::open(&p).unwrap()));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn what_is_written_is_there_when_reopened() {
        let p = path("reopen");
        let (s, i) = (uid(1), uid(2));
        let samples = [Sample::value(5, Decimal::from_i64(1))];
        {
            let mut sub = Substrate::new(RedbBackend::open(&p).unwrap());
            sub.add(&s, &i, &info(), &samples).unwrap();
        }
        let sub = Substrate::new(RedbBackend::open(&p).unwrap());
        assert_eq!(sub.window(&s, &i, 0, 10).unwrap().samples, samples);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }
}
