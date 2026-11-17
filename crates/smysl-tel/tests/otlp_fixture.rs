//! `fixtures/tel/otlp/` (RFC SMYSL-3 §6, §9.1, §19, §20.1, TL-P3): three OTLP metrics exports
//! of one service, each in OTLP/JSON and in protobuf, recorded into the substrate.
//!
//! - `e0` at 2026-01-16T10:00:00Z, `e1` 15 s later, `e2` 30 s later. Between `e1` and `e2` the
//!   process restarts: its cumulative counts start again, and say so by their start time.
//! - `e1` flags one series `FLAG_NO_RECORDED_VALUE`, and a counter with no start time falls.
//! - `eN.pb` is `protoc --encode` of `eN.txtpb` with `metrics.proto`, so the binary is not
//!   smysl's own. `eN.json` and `eN.txtpb` say the same thing.
//!
//! Checked, all against the pinned `expected.tsv`:
//! 1. the JSON exports, in order, into memory;
//! 2. the protobuf exports, in order, into redb reopened after each: the same units and digests,
//!    so the two encodings are one reading;
//! 3. the samples of (1) again, last export first: the same digests.
//!
//! `SMYSL_BLESS=1` rewrites `expected.tsv`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use smysl_core::canonical_uid;
use smysl_core::schemas::tel;
use smysl_core::Uid;
use smysl_series::redb::RedbBackend;
use smysl_series::{Backend, MemBackend, Sample, SeriesInfo, Substrate};
use smysl_tel::otel::{self, SCHEME};
use smysl_tel::readers::otlp::{self, Export};
use smysl_tel::scrape::SubstrateHistory;

const MS: u64 = 1_000_000;
/// 2026-01-16T10:00:00Z.
const T0: u64 = 1_768_557_600_000;
const ROLE: &str = "collector";
const KEY: &str = "otel-collector:c1";

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/tel/otlp")
}

fn json(n: usize) -> Export {
    let f = dir().join(format!("e{n}.json"));
    let t = std::fs::read_to_string(&f).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
    otlp::read_json(&t).unwrap_or_else(|e| panic!("e{n}.json: {e}"))
}

fn proto(n: usize) -> Export {
    let f = dir().join(format!("e{n}.pb"));
    let b = std::fs::read(&f).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
    otlp::read_proto(&b).unwrap_or_else(|e| panic!("e{n}.pb: {e}"))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

type Batch = (Uid, Uid, SeriesInfo, Vec<Sample>);

fn record<B: Backend>(
    mut sub: Substrate<B>,
    export: impl Fn(usize) -> Export,
    reopen: impl Fn(Substrate<B>) -> Substrate<B>,
) -> (String, Vec<Batch>, Substrate<B>) {
    let inst = canonical_uid(&tel::instrument(ROLE, KEY).unwrap());
    let mut out = format!("instrument\t{ROLE}\t{KEY}\t{}\n", inst.canonical());
    let mut batches = Vec::new();
    for n in 0..3 {
        let e = export(n);
        let r = {
            let h = SubstrateHistory {
                substrate: &sub,
                instrument: inst,
                scheme: SCHEME,
            };
            otel::record(&e, ROLE, KEY, &h).unwrap()
        };
        writeln!(
            out,
            "export\te{n}\t{} points\t{} series\t{} not values\t{} unsplit",
            e.raw.len(),
            r.series.len(),
            r.not_values,
            e.unsplit()
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
    (out, batches, sub)
}

/// Per series, by key: sample count, the flags seen, and the digest over the whole run.
fn digests<B: Backend>(sub: &Substrate<B>, batches: &[Batch]) -> String {
    let streams: BTreeMap<(Uid, Uid), String> = batches
        .iter()
        .map(|(s, i, info, _)| ((*s, *i), info.key.clone()))
        .collect();
    let mut by_key: BTreeMap<String, String> = BTreeMap::new();
    for ((s, i), key) in streams {
        let w = sub.window(&s, &i, T0, T0 + 60_000).unwrap();
        let flags = w.samples.iter().fold(0, |f, s| f | s.flags());
        by_key.insert(
            key,
            format!("{}\t{flags}\t{}", w.samples.len(), hex(&w.digest)),
        );
    }
    by_key
        .into_iter()
        .map(|(k, v)| format!("series\t{k}\t{v}\n"))
        .collect()
}

#[test]
fn the_otlp_fixture_reproduces_in_both_encodings_and_round_trips() {
    // 1. JSON, memory.
    let (units, batches, mem) = record(Substrate::new(MemBackend::default()), json, |s| s);
    let pinned = digests(&mem, &batches);

    // 2. Protobuf, redb, reopened after every export.
    let tmp =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("otlp-fixture-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let path = tmp.join("series.redb");
    let (units_pb, batches_pb, redb) = record(
        Substrate::new(RedbBackend::open(&path).unwrap()),
        proto,
        |s| {
            drop(s);
            Substrate::new(RedbBackend::open(&path).unwrap())
        },
    );
    assert_eq!(units_pb, units, "protobuf and JSON are one reading");
    assert_eq!(digests(&redb, &batches_pb), pinned, "to the digest");
    drop(redb);
    let _ = std::fs::remove_dir_all(&tmp);

    // 3. The same samples, last export first.
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
        "# RFC SMYSL-3 §9.1, §20.1: fixtures/tel/otlp/ recorded into the series substrate.\n\
         # Written by crates/smysl-tel/tests/otlp_fixture.rs; SMYSL_BLESS=1 rewrites it.\n\
         # Exports at {T0}, +15 s, +30 s; windows [{T0}, {}). series: key, samples, flags OR'd, \
         digest.\n{units}{pinned}",
        T0 + 60_000
    );
    let path = dir().join("expected.tsv");
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::write(&path, &got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).expect("expected.tsv; SMYSL_BLESS=1 writes it");
    assert_eq!(got, want, "fixtures/tel/otlp/expected.tsv moved");
}

#[test]
fn time_is_nanoseconds_and_a_sub_millisecond_part_is_kept() {
    let e = json(0);
    let p = e
        .points()
        .into_iter()
        .find(|p| p.key.contains(r#""name":"cpu.temp""#))
        .unwrap();
    assert_eq!(p.time_ns, T0 * MS + 123_456);
}
