//! `fixtures/tel/prom/` (RFC SMYSL-3 §9.1, §19, §20.1, TL-P3): two targets' scrapes, recorded
//! into the substrate, by uid and by digest.
//!
//! - `s0.prom` … `s3.prom` are one text-0.0.4 target, `job="node"`, `instance="h:9100"`, scraped
//!   every 15 s from 2026-01-16T10:00:00Z. A gauge goes away and comes back; every counter
//!   resets at once.
//! - `o0.om`, `o1.om` are one OpenMetrics target, `job="cart"`, `instance="c:8080"`, at the
//!   same instants. A summary's `_created` moves; a counter's does not.
//!
//! `expected.tsv` holds each scrape's units and, per series, its sample count and window
//! digest over the whole run. The **round trip** the TL-P3 exit criterion names is checked three
//! ways, all of which must give the pinned digests:
//! 1. recorded in order into the memory backend;
//! 2. recorded in order into the redb backend, reopened between scrapes;
//! 3. the samples of (1) added again into a fresh substrate in reverse scrape order.
//!
//! `SMYSL_BLESS=1` rewrites `expected.tsv`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use smysl_core::schemas::tel;
use smysl_core::{canonical_uid, Uid};
use smysl_series::redb::RedbBackend;
use smysl_series::{Backend, MemBackend, Sample, SeriesInfo, Substrate};
use smysl_tel::readers::prom;
use smysl_tel::scrape::{self, Scraped, SubstrateHistory, SCHEME};

/// 2026-01-16T10:00:00Z.
const T0: u64 = 1_768_557_600_000;

struct Target {
    job: &'static str,
    instance: &'static str,
    files: &'static [&'static str],
}

const TARGETS: &[Target] = &[
    Target {
        job: "node",
        instance: "h:9100",
        files: &["s0.prom", "s1.prom", "s2.prom", "s3.prom"],
    },
    Target {
        job: "cart",
        instance: "c:8080",
        files: &["o0.om", "o1.om"],
    },
];

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/tel/prom")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(dir().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

type Batch = (Uid, Uid, SeriesInfo, Vec<Sample>);

/// Record every scrape in order into `sub`, reopening it with `reopen` between scrapes. Returns
/// the units' lines and every batch added.
fn record<B: Backend>(
    mut sub: Substrate<B>,
    reopen: impl Fn(Substrate<B>) -> Substrate<B>,
) -> (String, Vec<Batch>, Substrate<B>) {
    let mut out = String::new();
    let mut batches = Vec::new();
    for t in TARGETS {
        let key = scrape::instrument_key(t.job, t.instance);
        let inst = canonical_uid(&tel::instrument("scraper", &key).unwrap());
        writeln!(out, "instrument\t{key}\t{}", inst.canonical()).unwrap();
        let labels = [("job", t.job), ("instance", t.instance)];
        for (n, file) in t.files.iter().enumerate() {
            let at = T0 + 15_000 * n as u64;
            let s = prom::read(&read(file), &labels).unwrap_or_else(|e| panic!("{file}: {e}"));
            let r: Scraped = {
                let h = SubstrateHistory {
                    substrate: &sub,
                    instrument: inst,
                    scheme: SCHEME,
                };
                scrape::record(&s, at, &key, &h).unwrap()
            };
            writeln!(
                out,
                "scrape\t{file}\t{at}\t{} series\t{} not values",
                r.series.len(),
                r.not_values
            )
            .unwrap();
            for u in &r.units {
                writeln!(
                    out,
                    "unit\t{}\t{}\t{}\t{}",
                    u.schema.as_str(),
                    u.status.as_str(),
                    canonical_uid(u).canonical(),
                    u.gist
                )
                .unwrap();
            }
            for ss in r.series {
                sub.add(&ss.uid, &r.inst, &ss.info, &ss.samples).unwrap();
                batches.push((ss.uid, r.inst, ss.info, ss.samples));
            }
            sub = reopen(sub);
        }
    }
    (out, batches, sub)
}

/// Per series: its key, sample count and digest over the whole run.
fn digests<B: Backend>(sub: &Substrate<B>, batches: &[Batch]) -> String {
    let streams: BTreeMap<(Uid, Uid), String> = batches
        .iter()
        .map(|(s, i, info, _)| ((*s, *i), info.key.clone()))
        .collect();
    let mut by_key: BTreeMap<String, String> = BTreeMap::new();
    for ((s, i), key) in streams {
        let w = sub.window(&s, &i, T0, T0 + 60_000).unwrap();
        by_key.insert(key, format!("{}\t{}", w.samples.len(), hex(&w.digest)));
    }
    by_key
        .into_iter()
        .map(|(k, v)| format!("series\t{k}\t{v}\n"))
        .collect()
}

#[test]
fn the_prom_fixture_reproduces_and_round_trips() {
    // 1. In memory, in order.
    let (units, batches, mem) = record(Substrate::new(MemBackend::default()), |s| s);
    let pinned = digests(&mem, &batches);

    // 2. In redb, reopened after every scrape.
    let tmp =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("prom-fixture-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let path = tmp.join("series.redb");
    let (units_redb, _, redb) = record(Substrate::new(RedbBackend::open(&path).unwrap()), |s| {
        drop(s);
        Substrate::new(RedbBackend::open(&path).unwrap())
    });
    assert_eq!(units_redb, units, "the units do not depend on the backend");
    assert_eq!(digests(&redb, &batches), pinned, "nor the digests");
    drop(redb);
    let _ = std::fs::remove_dir_all(&tmp);

    // 3. The same samples, last scrape first.
    let mut again = Substrate::new(MemBackend::default());
    for (s, i, info, samples) in batches.iter().rev() {
        again.add(s, i, info, samples).unwrap();
    }
    assert_eq!(
        digests(&again, &batches),
        pinned,
        "nor the order of arrival"
    );

    let got = format!(
        "# RFC SMYSL-3 §9.1, §20.1: fixtures/tel/prom/ recorded into the series substrate.\n\
         # Written by crates/smysl-tel/tests/prom_fixture.rs; SMYSL_BLESS=1 rewrites it.\n\
         # Scrapes every 15 s from {T0}; windows [{T0}, {}).\n{units}{pinned}",
        T0 + 60_000
    );
    let path = dir().join("expected.tsv");
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::write(&path, &got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).expect("expected.tsv; SMYSL_BLESS=1 writes it");
    assert_eq!(got, want, "fixtures/tel/prom/expected.tsv moved");
}
