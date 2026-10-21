//! `has_surface_form` is the writer's own predicate (SMYSL-2.7 §4.3): over every record of the
//! corpus, the wire fixtures and the library and dating fixtures, a record it calls written is
//! written by `write_surface` on its own, and one it calls omitted writes nothing.
//!
//! Views and label bindings are the two it decides from outside the record — the header view
//! and the labels the context spells units with — and are checked on those terms.

use std::path::{Path, PathBuf};

use smysl_core::surface::{has_surface_form, parse_surface, write_surface, WriteContext};
use smysl_core::{from_cbor_seq, Record};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn surface(rel: &str) -> Vec<Record> {
    let src = std::fs::read_to_string(repo().join(rel)).unwrap();
    parse_surface(&src)
        .unwrap_or_else(|e| panic!("{rel}: {e}"))
        .records
}

fn wire(rel: &str) -> Vec<Record> {
    let bytes = std::fs::read(repo().join(rel)).unwrap();
    from_cbor_seq(&bytes)
        .unwrap_or_else(|e| panic!("{rel}: {e}"))
        .0
}

fn every_record() -> Vec<(String, Record)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(repo().join("fixtures/corpus")).unwrap() {
        let name = e.unwrap().file_name().to_string_lossy().to_string();
        if name.ends_with(".smy") {
            let rel = format!("fixtures/corpus/{name}");
            out.extend(surface(&rel).into_iter().map(|r| (rel.clone(), r)));
        }
    }
    for rel in [
        "fixtures/library/wire/manifest.smy",
        "fixtures/conformance/check/time/impossible-dating.smy",
    ] {
        out.extend(surface(rel).into_iter().map(|r| (rel.to_string(), r)));
    }
    for e in std::fs::read_dir(repo().join("fixtures/wire")).unwrap() {
        let name = e.unwrap().file_name().to_string_lossy().to_string();
        if name.ends_with(".cbor") {
            let rel = format!("fixtures/wire/{name}");
            out.extend(wire(&rel).into_iter().map(|r| (rel.clone(), r)));
        }
    }
    out
}

#[test]
fn the_predicate_is_what_the_writer_writes() {
    let ctx = WriteContext::default();
    let records = every_record();
    let (mut written, mut omitted) = (0, 0);
    let mut kinds = std::collections::HashSet::new();
    for (rel, r) in &records {
        if matches!(r, Record::View(_) | Record::LabelBinding(_)) {
            continue;
        }
        let text = write_surface(None, std::slice::from_ref(r), &ctx);
        let says = has_surface_form(r, &ctx, None);
        assert_eq!(says, !text.trim().is_empty(), "{rel}: {r:?}\n{text}");
        if says {
            written += 1;
            kinds.insert(std::mem::discriminant(r));
        } else {
            omitted += 1;
        }
    }
    // The corpus is not vacuous: both answers occur, over more than a handful of types, and
    // among the written are the two the old count called omitted.
    assert!(written > 50 && omitted > 0, "{written} {omitted}");
    assert!(kinds.len() >= 8, "{} kinds written", kinds.len());
    for want in ["Redaction", "Dating"] {
        assert!(
            records.iter().any(|(_, r)| format!("{r:?}").starts_with(want)
                && has_surface_form(r, &ctx, None)),
            "no {want} written"
        );
    }
}

/// A view is written only as the header, and a binding only when the context spells its unit
/// with its label.
#[test]
fn views_and_labels_are_written_by_the_context() {
    let records = surface("fixtures/corpus/F13-canon.smy");
    let view = records
        .iter()
        .find_map(|r| match r {
            Record::View(v) => Some(v.clone()),
            _ => None,
        })
        .unwrap();
    let rv = Record::View(view.clone());
    let ctx = WriteContext::default();
    assert!(has_surface_form(&rv, &ctx, Some(&view.id)));
    assert!(!has_surface_form(&rv, &ctx, None));

    let binding = records
        .iter()
        .find_map(|r| match r {
            Record::LabelBinding(b) => Some(b.clone()),
            _ => None,
        })
        .unwrap();
    let rb = Record::LabelBinding(binding.clone());
    assert!(!has_surface_form(&rb, &ctx, None));
    let mut spelled = WriteContext::default();
    spelled.labels.insert(binding.uid, binding.label.clone());
    assert!(has_surface_form(&rb, &spelled, None));
}
