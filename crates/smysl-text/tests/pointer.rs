//! Pointer lenses over `json/1` snapshots (SMYSL-2.5 §3.12; TX-P12 step 8).
//!
//! The exit: a two-version snapshot series answers `pointer(…)` across the rename. Two
//! configuration snapshots read by `json/1`, the second renaming `/database` to `/db` and
//! `max_size` to `maxSize`; a pointer lens `x.config/v1` declaring the rename; each snapshot's
//! manifest marking its revision. `pointer("/database/pool/max_size")` finds the value in both,
//! and so does the new spelling; a third snapshot with no marker is read without the lens and
//! counted (`SMY-W475`).
#![cfg(feature = "reader-json")]

use std::collections::BTreeMap;

use smysl_core::schema::{encode_lenses, LensOp, LensStep};
use smysl_core::types::{Manifest, Record, SchemaDecl};
use smysl_core::{
    canonical_uid, LangTag, SchemaId, SourceKind, SourceRef, Status, Uid, UnitCoreBuilder,
};
use smysl_graph::Store;
use smysl_query::{explain, parse, plan, run, Env, PlanOptions, StoreScope};
use smysl_text::limits::{Budget, Caps};
use smysl_text::locator::Locator;
use smysl_text::readers::{json::Json, read_with};
use smysl_text::{Input, Params};

const V1: &str = r#"{"database": {"pool": {"max_size": "20", "timeout": "5s"}}, "service": "api"}"#;
const V2: &str = r#"{"db": {"pool": {"maxSize": "40", "timeout": "5s"}}, "service": "api"}"#;
const V0: &str = r#"{"database": {"pool": {"max_size": "10"}}}"#;

/// One snapshot: a manifest marked with its revision (or not), and a unit per value.
fn snapshot(
    alias: &str,
    source: &str,
    revision: Option<u32>,
) -> (Vec<Record>, BTreeMap<String, Uid>) {
    let mut budget = Budget::new(Caps::DEFAULT, source.len() as u64).unwrap();
    let out = read_with(
        &Json,
        &Input::new(source.as_bytes()),
        &Params::new(),
        &mut budget,
    )
    .unwrap();
    let mut m = Manifest::new(
        alias,
        LangTag::new("und").unwrap(),
        "json/1",
        "unknown",
        "entry",
    );
    if let Some(r) = revision {
        m.identifiers.insert("x.config/v1".into(), r.to_string());
    }
    let mid = m.mid();
    let mut records = vec![Record::Manifest(m)];
    let mut by_pointer = BTreeMap::new();
    let text = out.text.as_str();
    for row in &out.rows {
        let Locator::Pointer(_) = &row.locator else {
            continue;
        };
        if row.end <= row.start {
            continue;
        }
        let pointer = row.locator.to_string();
        let value = &text[row.start as usize..row.end as usize];
        let src = SourceRef::new(SourceKind::Doc, format!("{alias}.json#{pointer}"))
            .in_part(row.start, row.end, mid)
            .unwrap();
        let unit = UnitCoreBuilder::new(
            SchemaId::parse("claim").unwrap(),
            format!("{alias}: {pointer} is {value}"),
            Status::Cited,
        )
        .source(src)
        .build()
        .unwrap();
        by_pointer.insert(pointer, canonical_uid(&unit));
        records.push(Record::Unit(unit));
    }
    (records, by_pointer)
}

fn lens() -> Vec<Record> {
    let mut d1 = SchemaDecl::new(SchemaId::parse("x.config/v1").unwrap(), 1);
    d1.types = Vec::new();
    let mut d2 = SchemaDecl::new(SchemaId::parse("x.config/v1").unwrap(), 2);
    d2.lenses = Some(encode_lenses(&[LensStep::new(
        1,
        2,
        vec![
            LensOp::RenameKey {
                from: "/database".into(),
                to: "/db".into(),
            },
            LensOp::RenameKey {
                from: "/db/pool/max_size".into(),
                to: "/db/pool/maxSize".into(),
            },
        ],
    )]));
    vec![Record::SchemaDecl(d1), Record::SchemaDecl(d2)]
}

struct Series {
    store: Store,
    v1: BTreeMap<String, Uid>,
    v2: BTreeMap<String, Uid>,
    v0: BTreeMap<String, Uid>,
}

fn series() -> Series {
    let mut records = lens();
    let (r1, v1) = snapshot("config-2026-01", V1, Some(1));
    let (r2, v2) = snapshot("config-2026-06", V2, Some(2));
    let (r0, v0) = snapshot("config-2025-09", V0, None);
    records.extend(r1);
    records.extend(r2);
    records.extend(r0);
    Series {
        store: Store::from_records(records),
        v1,
        v2,
        v0,
    }
}

fn answer(store: &Store, q: &str) -> (Vec<Uid>, Vec<String>, String) {
    let scope = StoreScope::new("config", store);
    let env = Env::new(store, &scope);
    let opts = PlanOptions::default();
    let p = plan(&parse(q).unwrap().query, &env, &opts).unwrap_or_else(|d| panic!("{q}: {d:?}"));
    let r = run(&p, &env, &opts).unwrap();
    let mut uids = r.answer.uids();
    uids.sort();
    let warnings = r
        .warnings
        .iter()
        .map(|w| w.code.as_str().to_string())
        .collect();
    (uids, warnings, explain(&p))
}

fn sorted(mut v: Vec<Uid>) -> Vec<Uid> {
    v.sort();
    v
}

/// The exit: the value is found in both snapshots, under either spelling.
#[test]
fn a_two_version_series_answers_pointer_across_the_rename() {
    let s = series();
    // json/1's locators are the pointers the snapshots spell.
    assert!(
        s.v1.contains_key("/database/pool/max_size"),
        "{:?}",
        s.v1.keys()
    );
    assert!(s.v2.contains_key("/db/pool/maxSize"), "{:?}", s.v2.keys());

    let both = sorted(vec![
        s.v1["/database/pool/max_size"],
        s.v2["/db/pool/maxSize"],
    ]);
    let (old, warnings, text) = answer(&s.store, "pointer(\"/database/pool/max_size\")");
    // The unmarked snapshot spells it the old way and is matched without a lens, and said.
    assert_eq!(
        old,
        sorted([both.clone(), vec![s.v0["/database/pool/max_size"]]].concat())
    );
    assert_eq!(warnings, vec!["SMY-W475".to_string()]);
    assert!(
        text.contains(
            "5 source: pointer \"/database/pool/max_size\" across x.config/v1@1 \
             /database/pool/max_size, x.config/v1@2 /db/pool/maxSize\n"
        ),
        "{text}"
    );

    let (new, _, _) = answer(&s.store, "pointer(\"/db/pool/maxSize\")");
    assert_eq!(new, both, "the new spelling finds the old snapshot too");
}

/// A pointer names its subtree, and a rename above it carries the subtree along.
#[test]
fn a_pointer_names_its_subtree_across_the_rename() {
    let s = series();
    let (got, _, _) = answer(&s.store, "pointer(\"/database/pool\")");
    let want = sorted(vec![
        s.v1["/database/pool/max_size"],
        s.v1["/database/pool/timeout"],
        s.v2["/db/pool/maxSize"],
        s.v2["/db/pool/timeout"],
        s.v0["/database/pool/max_size"],
    ]);
    assert_eq!(got, want);
    // A pointer no lens renames is matched as written in every snapshot.
    let (svc, warnings, _) = answer(&s.store, "pointer(\"/service\")");
    assert_eq!(svc, sorted(vec![s.v1["/service"], s.v2["/service"]]));
    assert!(warnings.is_empty(), "no lens is involved: {warnings:?}");
}
