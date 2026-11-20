//! The behaviour every substrate backend must have (RFC SMYSL-3 §19.3), run against each.
//!
//! Outside `src/` because the redb half needs a directory, and `xtask check-purity` keeps the
//! environment, `std::env::temp_dir` included, out of the crate's own source.

use smysl_core::{Decimal, Uid};
use smysl_series::{
    digest, Backend, Event, MemBackend, Sample, SeriesInfo, Span, StreamInfo, Substrate,
};

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

/// Rule TL (§13), against each backend: eviction by whole chunk, recorded, and undone by the
/// samples coming back.
fn eviction<B: Backend>(mut sub: Substrate<B>) {
    let (s, i, j) = (uid(1), uid(2), uid(3));
    // One sample a minute for six hours, from two instruments: a 2 h span, three chunks each.
    let all: Vec<Sample> = (0..360).map(|k| v(k * 60_000, k as i64)).collect();
    sub.add(&s, &i, &info(), &all).unwrap();
    sub.add(&s, &j, &info(), &all[..10]).unwrap();
    let whole = sub.window(&s, &i, 0, 6 * 3_600_000).unwrap();
    assert_eq!(sub.row(&s).unwrap().unwrap().span, Span::H2);

    // A bound inside the second chunk evicts only the first: its span ends at 2 h ≤ 3 h; the
    // second's ends at 4 h, and it holds samples younger than the bound.
    let planned = sub.evictable(3 * 3_600_000).unwrap();
    assert_eq!(
        sub.window(&s, &i, 0, 6 * 3_600_000).unwrap(),
        whole,
        "a plan removes nothing"
    );
    let r = sub.evict(3 * 3_600_000, 99).unwrap();
    assert_eq!(r, planned);
    assert_eq!((r.chunks, r.samples, r.series), (2, 130, 1));
    let row = sub.row(&s).unwrap().unwrap();
    assert_eq!(
        (row.count, row.first_ms, row.span),
        (240, 7_200_000, Span::H2)
    );
    assert!(sub.window(&s, &i, 0, 7_200_000).unwrap().samples.is_empty());
    assert_eq!(
        sub.window(&s, &i, 7_200_000, 6 * 3_600_000)
            .unwrap()
            .samples,
        all[120..].to_vec(),
        "nothing younger than its chunk's end is touched"
    );
    let c = sub.present(&s, &i, 0, 6 * 3_600_000).unwrap();
    assert_eq!(
        (c.present, c.evicted),
        (vec![7_200_000, 14_400_000], vec![0])
    );
    assert!(sub.present(&s, &j, 0, 7_200_000).unwrap().is_absent());
    assert_eq!(sub.evictions().unwrap(), vec![(s, i, 0, 99), (s, j, 0, 99)]);
    assert!(
        sub.instruments(&s).unwrap() == vec![i],
        "j held one chunk, now evicted"
    );

    // Again, at the same bound: nothing left to do.
    assert_eq!(sub.evict(3 * 3_600_000, 100).unwrap().chunks, 0);

    // A merge with a peer that kept them brings the samples back as a set: the same digest,
    // and the span is held again.
    sub.add(&s, &i, &info(), &all).unwrap();
    assert_eq!(sub.window(&s, &i, 0, 6 * 3_600_000).unwrap(), whole);
    assert_eq!(sub.row(&s).unwrap().unwrap().count, 360);
    let c = sub.present(&s, &i, 0, 6 * 3_600_000).unwrap();
    assert!(c.evicted.is_empty() && c.present.len() == 3);
    assert_eq!(sub.evictions().unwrap(), vec![(s, j, 0, 99)]);

    // Everything evicted: the row stays, empty, with its span.
    let r = sub.evict(u64::MAX, 200).unwrap();
    assert_eq!(r.samples, 360);
    let row = sub.row(&s).unwrap().unwrap();
    assert_eq!((row.count, row.first_ms, row.last_ms), (0, u64::MAX, 0));
}

/// Events (§19.6): a set, read by range, found by tag, evicted by the hour beside the samples.
fn events<B: Backend>(mut sub: Substrate<B>) -> Substrate<B> {
    let (s, i) = (uid(7), uid(8));
    let si = StreamInfo {
        scheme: "otel".into(),
        key: r#"{"signal":"spans"}"#.into(),
        kind: "span".into(),
    };
    let tag = |e: &Event| Some([e.body[0]; 16]);
    let evs: Vec<Event> = (0..6u64)
        .map(|k| Event::at_nanos(k * 1_800_000_000_000 + 7, vec![(k % 3) as u8, k as u8]))
        .collect();
    let r = sub.add_events(&s, &i, &si, &evs, &tag).unwrap();
    assert_eq!((r.added, r.already), (6, 0));
    let r = sub.add_events(&s, &i, &si, &evs[2..], &tag).unwrap();
    assert_eq!((r.added, r.already), (0, 4));
    let all = sub.events(&s, &i, 0, u64::MAX).unwrap();
    assert_eq!(all.events, evs);
    assert_eq!(all.events[0].ns, 7, "the remainder is kept");
    let firsts = sub.events(&s, &i, 0, 3_600_000).unwrap();
    assert_eq!(firsts.events.len(), 2);
    assert_eq!(
        sub.tagged(&[0; 16]).unwrap().len(),
        2,
        "events 0 and 3: hours 0 and 1"
    );
    assert_eq!(sub.event_instruments(&s).unwrap(), vec![i]);
    assert_eq!(sub.streams().unwrap().len(), 1);
    // Samples and events in one eviction: everything before 2 h.
    sub.add(&uid(1), &uid(2), &info(), &[v(1_000, 1)]).unwrap();
    let r = sub.evict(7_200_000, 42).unwrap();
    assert_eq!((r.chunks, r.event_hours, r.events), (1, 2, 4));
    assert_eq!(sub.events(&s, &i, 0, u64::MAX).unwrap().events, evs[4..]);
    assert_eq!(sub.event_evictions().unwrap().len(), 2);
    sub
}

#[test]
fn the_memory_backend_keeps_the_event_contract() {
    events(Substrate::new(MemBackend::default()));
}

#[test]
fn the_memory_backend_keeps_the_contract() {
    contract(Substrate::new(MemBackend::default()));
}

#[test]
fn the_memory_backend_evicts_by_rule_tl() {
    eviction(Substrate::new(MemBackend::default()));
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
    fn the_redb_backend_evicts_by_rule_tl_and_keeps_it_when_reopened() {
        let p = path("evict");
        eviction(Substrate::new(RedbBackend::open(&p).unwrap()));
        let sub = Substrate::new(RedbBackend::open(&p).unwrap());
        assert_eq!(
            sub.evictions().unwrap().len(),
            4,
            "every chunk, and j's from before"
        );
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn the_redb_backend_keeps_the_event_contract_when_reopened() {
        let p = path("events");
        drop(events(Substrate::new(RedbBackend::open(&p).unwrap())));
        let sub = Substrate::new(RedbBackend::open(&p).unwrap());
        assert_eq!(sub.event_evictions().unwrap().len(), 2);
        assert_eq!(sub.streams().unwrap()[0].1.count, 2);
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
