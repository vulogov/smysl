//! `fixtures/tel/zabbix/` (RFC SMYSL-3 §6, §9.1, §19, §20.1, TL-P3): Zabbix history, read
//! against its item export and recorded into the substrate.
//!
//! - `items.json` is `item.get` with `selectHosts: ["host"]`: three numeric items on two hosts
//!   (one host named `DB 01`) and a character item.
//! - `history-a.json` and `history-b.json` are two `history.get` exports of five minutes, split
//!   into two ranges that share the third minute. `b` is newest first, as the API returns it
//!   with `sortorder: DESC`. `a` has a row of an item the item export does not have, and the
//!   load average's fourth minute is out of `dec`'s range.
//! - `history-char.json` is the character item's history: read, and not a series.
//!
//! Checked against the pinned `expected.tsv`:
//! 1. `a` then `b`, as API 7.0, into memory;
//! 2. `b` then `a`, as API 6.0, into redb reopened between them: the same units and digests,
//!    because the overlap is idempotent and the API major does not change history's shape;
//! 3. the samples of (1) again, last first.
//!
//! `SMYSL_BLESS=1` rewrites `expected.tsv`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use smysl_core::canonical_uid;
use smysl_series::redb::RedbBackend;
use smysl_series::{Backend, MemBackend, Substrate};
use smysl_tel::readers::zabbix_history;
use smysl_tel::zabbix_history::{instrument_key, record};

/// 2026-01-16T10:00:00Z.
const T0: u64 = 1_768_557_600_000;

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/tel/zabbix")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(dir().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Record `files` in order, reopening between them. Returns the report and the substrate.
fn run<B: Backend>(
    mut sub: Substrate<B>,
    files: &[&str],
    api: &str,
    reopen: impl Fn(Substrate<B>) -> Substrate<B>,
) -> (String, Substrate<B>) {
    let items = read("items.json");
    let key = instrument_key("zbx-main");
    let mut out = String::new();
    let mut streams = std::collections::BTreeMap::new();
    for f in files {
        let h = zabbix_history::read(&read(f), &items, api).unwrap_or_else(|e| panic!("{f}: {e}"));
        let r = record(&h, &key).unwrap();
        writeln!(
            out,
            "export\t{f}\t{} series\t{} not numeric\t{} not values\tunknown {:?}",
            r.series.len(),
            h.not_numeric,
            r.not_values,
            h.unknown_items
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
        for ss in &r.series {
            sub.add(&ss.uid, &r.inst, &ss.info, &ss.samples).unwrap();
            streams.insert(ss.info.key.clone(), (ss.uid, r.inst));
        }
        sub = reopen(sub);
    }
    let mut digests = String::new();
    for (key, (s, i)) in streams {
        let w = sub.window(&s, &i, T0, T0 + 300_000).unwrap();
        let flags = w.samples.iter().fold(0, |f, s| f | s.flags());
        writeln!(
            digests,
            "series\t{key}\t{}\t{flags}\t{}",
            w.samples.len(),
            hex(&w.digest)
        )
        .unwrap();
    }
    (format!("{out}\u{0}{digests}"), sub)
}

#[test]
fn the_zabbix_history_fixture_reproduces_and_round_trips() {
    let files = ["history-a.json", "history-b.json", "history-char.json"];
    let (mem, sub) = run(Substrate::new(MemBackend::default()), &files, "7.0", |s| s);
    let (units, digests) = mem.split_once('\u{0}').unwrap();

    let tmp = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("zabbix-fixture-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let path = tmp.join("series.redb");
    let reversed = ["history-char.json", "history-b.json", "history-a.json"];
    let (redb, db) = run(
        Substrate::new(RedbBackend::open(&path).unwrap()),
        &reversed,
        "6.0.30",
        |s| {
            drop(s);
            Substrate::new(RedbBackend::open(&path).unwrap())
        },
    );
    let (_, redb_digests) = redb.split_once('\u{0}').unwrap();
    assert_eq!(
        redb_digests, digests,
        "order, overlap, backend and API major"
    );
    drop(db);
    let _ = std::fs::remove_dir_all(&tmp);

    // Recording `a` again changes nothing: the overlap and the repeat are idempotent.
    let (again, _) = run(sub, &["history-a.json"], "7.0", |s| s);
    let (_, again_digests) = again.split_once('\u{0}').unwrap();
    assert_eq!(again_digests, digests, "not one sample more");

    let got = format!(
        "# RFC SMYSL-3 §6, §20.1: fixtures/tel/zabbix/ history recorded into the series substrate.\n\
         # Written by crates/smysl-tel/tests/zabbix_history_fixture.rs; SMYSL_BLESS=1 rewrites it.\n\
         # Instrument zabbix-server:zbx-main; windows [{T0}, {}). series: key, samples, flags OR'd, \
         digest.\n{units}{digests}",
        T0 + 300_000
    );
    let path = dir().join("expected.tsv");
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::write(&path, &got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).expect("expected.tsv; SMYSL_BLESS=1 writes it");
    assert_eq!(got, want, "fixtures/tel/zabbix/expected.tsv moved");
}

/// `trends.json` (TL-P4 step 4): three hours of `trend.get` for the three numeric items, each
/// hour a `cited` platform window, pinned by uid in `expected-trends.tsv`. One `value_max` is
/// outside `dec`'s range and left out of its window; one row is of an item not exported.
#[test]
fn the_zabbix_trends_fixture_is_cited_windows_by_uid() {
    use smysl_core::schemas::tel;
    use smysl_tel::zabbix_history::record_trends;
    let items = read("items.json");
    let t = zabbix_history::read_trends(&read("trends.json"), &items, "7.0").unwrap();
    // The same export as API 6.0 is the same windows.
    let t6 = zabbix_history::read_trends(&read("trends.json"), &items, "6.0.30").unwrap();
    assert_eq!(t, t6);
    let r = record_trends(&t, &instrument_key("zbx-main")).unwrap();
    let mut out = format!(
        "# RFC SMYSL-3 §9.4, §20.1: fixtures/tel/zabbix/trends.json as cited platform windows.\n\
         # Written by crates/smysl-tel/tests/zabbix_history_fixture.rs; SMYSL_BLESS=1 rewrites it.\n\
         export\ttrends.json\t{} windows\t{} not values\tunknown {:?}\n",
        r.windows.len(),
        r.not_values,
        t.unknown_items
    );
    for w in &r.windows {
        let u = w.unit();
        assert_eq!(u.status, smysl_core::Status::Cited);
        assert!(
            tel::rule_tw(&tel::window_fields(u.payload.as_deref().unwrap()).unwrap())
                .is_some_and(|(s, _, _)| s == smysl_core::Status::Cited)
        );
        writeln!(
            out,
            "window\t{}\t{}\t{}\tcount {}\tmax {}",
            canonical_uid(&u).canonical(),
            w.key,
            w.from_ms,
            w.summary.count,
            w.summary.max.map_or("-".into(), |m| m.to_string())
        )
        .unwrap();
    }
    let path = dir().join("expected-trends.tsv");
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::write(&path, &out).unwrap();
        return;
    }
    let want =
        std::fs::read_to_string(&path).expect("expected-trends.tsv; SMYSL_BLESS=1 writes it");
    assert_eq!(out, want, "fixtures/tel/zabbix/expected-trends.tsv moved");
}
