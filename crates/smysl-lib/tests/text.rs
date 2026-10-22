//! Text in collections, and the two new layouts, of TX-P11 step 6 (SMYSL-2.7 §3.2, §3.4, §3.6).
//!
//! The step's exit: the Bible corpus cells of GE-T15 pass P1 and P3, and a redacted tid's
//! records 15 and 18 are dropped on import (`SMY-W414`). The corpus is
//! `fixtures/library/bibles/slice/`, Ruth and Jude in five editions, added with `usfm/1` under
//! `public-domain` and `carry: text`, as `tests/cmd_measure.rs` adds it. A library of texts
//! alone selects nothing, since an export selects units, so each part gets one unit drawn from
//! its first bytes, as an extraction would draw one.

use std::collections::BTreeSet;
use std::path::PathBuf;

use smysl_core::{Carry, Code, Hlc, Record, Redaction, Tid};
use smysl_lib::{
    export_library, import, original_path, retained, snapshot, verify_collection, write_collection,
    Closure, ExportOptions, ImportOptions, Input, Layout, Selection, With, INDEX_FILE,
};
use smysl_text::Library;

mod common;

use common::{agent, repo};

fn dir(name: &str) -> PathBuf {
    common::dir("text", name)
}

fn bibles(name: &str, carry: Carry) -> Library {
    common::bibles(&dir(name), carry)
}

fn tids(lib: &Library) -> BTreeSet<Tid> {
    lib.store()
        .manifests()
        .flat_map(|(_, m)| m.parts.iter().map(|e| e.tid))
        .collect()
}

fn opts(layout: Layout, closure: Closure) -> ExportOptions {
    ExportOptions::new()
        .layout(layout)
        .closure(closure)
        .with(With::new().text(true))
}

fn text_in(records: &[Record]) -> (usize, usize) {
    let parts = records
        .iter()
        .filter(|r| matches!(r, Record::PartText(_)))
        .count();
    let readings = records
        .iter()
        .filter(|r| matches!(r, Record::PartReading(_)))
        .count();
    (parts, readings)
}

#[test]
fn the_bible_cells_hold_p1_and_p3() {
    let lib = bibles("cells", Carry::Text);
    let parts = tids(&lib);
    let snap = snapshot(&lib, &With::new()).unwrap();
    let sel = Selection::all(&snap);
    for (layout, files) in [
        (Layout::Single, 1),
        (Layout::ByExpression, 10),
        (Layout::ByCollection, 5),
    ] {
        for closure in [Closure::Full, Closure::Ref] {
            let cell = format!("{}/{}", layout.as_str(), closure.as_str());
            let c = export_library(&lib, &snap, &sel, &opts(layout, closure)).unwrap();
            assert_eq!(c.files.len(), files, "{cell}");
            let (p, r) = text_in(&c.records());
            assert_eq!(
                p,
                parts.len(),
                "{cell}: every part a unit is drawn from travels"
            );
            assert_eq!(r, parts.len(), "{cell}: and its reading");
            let v = verify_collection(&c);
            assert!(v.holds(), "{cell}: {v:?}");
            assert!(v.p1.holds() && v.p3.holds(), "{cell}: {v:?}");

            // P1 through a real import, into an empty library: the text lands in its object
            // store, and the retained set is the collection's.
            let out = dir(&format!(
                "cells-out-{}-{}",
                layout.as_str(),
                closure.as_str()
            ));
            write_collection(&c, &out).unwrap();
            let mut to = Library::create(&dir(&format!(
                "cells-to-{}-{}",
                layout.as_str(),
                closure.as_str()
            )))
            .unwrap();
            let r = import(
                &mut to,
                &[Input::Index(out.join(INDEX_FILE))],
                &ImportOptions::new().accept(true),
            )
            .unwrap();
            assert!(
                r.files.iter().all(|f| f.quarantined == 0 && f.waiting == 0),
                "{cell}"
            );
            for t in &parts {
                assert!(
                    to.objects().has_part(t),
                    "{cell}: {t:?} is in the object store"
                );
            }
            let kept = retained(&to, &r.name).unwrap();
            assert_eq!(smysl_graph::record_set_digest(&kept), c.rsd(), "{cell}: P1");
        }
    }
}

#[test]
fn by_expression_and_by_collection_key_by_alias() {
    let lib = bibles("keys", Carry::Text);
    let snap = snapshot(&lib, &With::new()).unwrap();
    let sel = Selection::all(&snap);
    let c = export_library(
        &lib,
        &snap,
        &sel,
        &opts(Layout::ByCollection, Closure::Full),
    )
    .unwrap();
    let names: Vec<&str> = c.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        names,
        [
            "deu1912.cbor",
            "eng-kjv.cbor",
            "fralsg.cbor",
            "russyn.cbor",
            "sparv1909.cbor"
        ]
    );
    let c = export_library(
        &lib,
        &snap,
        &sel,
        &opts(Layout::ByExpression, Closure::Full),
    )
    .unwrap();
    assert!(
        c.file("eng-kjv-08-rut.cbor").is_some(),
        "{:?}",
        c.files.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
    // A plain segment needs no hash suffix.
    assert_eq!(c.index.counts["SMY-W497"], 0);
}

#[test]
fn text_travels_only_when_asked_and_allowed() {
    let lib = bibles("gate-text", Carry::Text);
    let snap = snapshot(&lib, &With::new()).unwrap();
    let sel = Selection::all(&snap);
    let plain = ExportOptions::new().layout(Layout::Single);
    let c = export_library(&lib, &snap, &sel, &plain).unwrap();
    assert_eq!(text_in(&c.records()), (0, 0), "no --with-text, no text");

    // `carry: ref` under `unknown`: the manifest names its parts and does not carry them.
    let reflib = bibles("gate-ref", Carry::Ref);
    let snap = snapshot(&reflib, &With::new()).unwrap();
    let c = export_library(
        &reflib,
        &snap,
        &Selection::all(&snap),
        &opts(Layout::Single, Closure::Full),
    )
    .unwrap();
    assert_eq!(text_in(&c.records()), (0, 0), "carry ref carries no text");
    assert!(verify_collection(&c).holds());
}

#[test]
fn a_redacted_part_is_dropped_at_the_door() {
    let lib = bibles("redact-from", Carry::Text);
    let snap = snapshot(&lib, &With::new()).unwrap();
    let c = export_library(
        &lib,
        &snap,
        &Selection::all(&snap),
        &opts(Layout::Single, Closure::Full),
    )
    .unwrap();
    let out = dir("redact-out");
    write_collection(&c, &out).unwrap();

    // The receiving library has redacted one part already.
    let gone = *tids(&lib).first().unwrap();
    let mut to = Library::create(&dir("redact-to")).unwrap();
    to.append_records(&[Record::Redaction(Redaction::new(
        gone,
        agent(),
        Hlc::new(1, 0, agent()),
    ))])
    .unwrap();
    let r = import(
        &mut to,
        &[Input::Index(out.join(INDEX_FILE))],
        &ImportOptions::new().accept(true),
    )
    .unwrap();
    assert!(
        r.report.iter().any(|d| d.code == Code::W414),
        "{:?}",
        r.report
    );
    assert_eq!(
        r.files.iter().map(|f| f.redacted).sum::<usize>(),
        2,
        "its text and its reading"
    );
    assert!(!to.objects().has_part(&gone));
    for t in tids(&lib).iter().filter(|t| **t != gone) {
        assert!(to.objects().has_part(t));
    }

    // A record 19 among the files does the same, for a library that had redacted nothing.
    let part = c
        .records()
        .into_iter()
        .find_map(|r| match r {
            Record::PartText(p) => Some(p),
            _ => None,
        })
        .unwrap();
    let mut bytes = smysl_core::to_cbor(&Record::PartText(part.clone()));
    bytes.extend(smysl_core::to_cbor(&Record::Redaction(Redaction::new(
        part.tid,
        agent(),
        Hlc::new(2, 0, agent()),
    ))));
    let mut fresh = Library::create(&dir("redact-input")).unwrap();
    let r = import(
        &mut fresh,
        &[Input::Bytes {
            name: "with-redaction.cbor".into(),
            bytes,
        }],
        &ImportOptions::new().accept(true),
    )
    .unwrap();
    assert_eq!(r.files[0].redacted, 1);
    assert!(r.report.iter().any(|d| d.code == Code::W414));
    assert!(!fresh.objects().has_part(&part.tid));
}

#[test]
fn staged_text_reaches_the_object_store_on_accept() {
    let lib = bibles("staged-from", Carry::Text);
    let snap = snapshot(&lib, &With::new()).unwrap();
    let c = export_library(
        &lib,
        &snap,
        &Selection::all(&snap),
        &opts(Layout::Single, Closure::Full),
    )
    .unwrap();
    let out = dir("staged-out");
    write_collection(&c, &out).unwrap();
    let mut to = Library::create(&dir("staged-to")).unwrap();
    let r = import(
        &mut to,
        &[Input::Index(out.join(INDEX_FILE))],
        &ImportOptions::new(),
    )
    .unwrap();
    let some = *tids(&lib).first().unwrap();
    assert!(
        !to.objects().has_part(&some),
        "staged text is not in the object store yet"
    );
    smysl_lib::accept(&mut to, &r.name, false).unwrap();
    assert!(to.objects().has_part(&some));
    let kept = retained(&to, &r.name).unwrap();
    assert_eq!(smysl_graph::record_set_digest(&kept), c.rsd());
}

#[test]
fn keep_original_keeps_the_bytes_under_their_hash() {
    let bytes = std::fs::read(repo().join("fixtures/corpus/F13-canon.smy")).unwrap();
    let mut lib = Library::create(&dir("original")).unwrap();
    let r = import(
        &mut lib,
        &[Input::Bytes {
            name: "F13-canon.smy".into(),
            bytes: bytes.clone(),
        }],
        &ImportOptions::new().keep_original(true),
    )
    .unwrap();
    let p = original_path(lib.root(), &r.files[0].original);
    assert_eq!(std::fs::read(p).unwrap(), bytes);
}

#[test]
fn a_part_whose_bytes_are_not_its_tid_is_quarantined() {
    let good = smysl_core::PartText::new(b"In the beginning".to_vec());
    let mut bad = good.clone();
    bad.text = b"In the end".to_vec();
    let mut lib = Library::create(&dir("corrupt")).unwrap();
    let r = import(
        &mut lib,
        &[Input::Bytes {
            name: "corrupt.cbor".into(),
            bytes: smysl_core::to_cbor(&Record::PartText(bad)),
        }],
        &ImportOptions::new().accept(true),
    )
    .unwrap();
    assert_eq!(r.files[0].quarantined, 1);
    assert!(r
        .report
        .iter()
        .any(|d| d.code == Code::W423 && d.message.contains("SMY-E446")));
    assert!(!lib.objects().has_part(&good.tid));
}
