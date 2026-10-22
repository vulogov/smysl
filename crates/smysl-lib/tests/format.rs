//! The `surface` and `pack` formats, of TX-P11 step 10 (SMYSL-2.7 §3.8).
//!
//! The step's exit: the surface cells of GE-T15. Per cell, the set a surface file leaves out is
//! exactly the records `has_surface_form` says the writer cannot spell, per type, and what it
//! reads back is exactly the rest. The exporter checks that itself on every file (`SMY-E424`);
//! here it is checked from outside as well, against the same export written as CBOR.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use smysl_core::surface::{has_surface_form, WriteContext};
use smysl_core::{Label, Record, Uid};
use smysl_graph::Store;
use smysl_lib::{
    export_store, import, retained, verify_collection, write_collection, Closure, Collection,
    ExportOptions, Format, ImportOptions, Input, Layout, Property, Selection, INDEX_FILE,
};
use smysl_text::Library;

mod common;

fn dir(name: &str) -> PathBuf {
    common::dir("format", name)
}

fn hashes(rs: &[Record]) -> BTreeSet<[u8; 32]> {
    rs.iter().map(Store::record_hash).collect()
}

#[test]
fn the_surface_cells_leave_out_exactly_what_has_no_surface_form() {
    for rel in common::corpus() {
        let store = Store::from_records(common::fixture(&rel));
        for layout in [
            Layout::Single,
            Layout::ByView,
            Layout::ByExpression,
            Layout::ByTime,
        ] {
            for closure in [Closure::Full, Closure::Ref] {
                let cell = format!("{rel} {}/{}", layout.as_str(), closure.as_str());
                let base = ExportOptions::new().layout(layout).closure(closure);
                let cbor = export_store(&store, &Selection::all(&store), &base).unwrap();
                let surf = export_store(
                    &store,
                    &Selection::all(&store),
                    &base.clone().format(Format::Surface),
                )
                .unwrap();
                assert!(surf.index.complete, "{cell}: {:?}", surf.report);
                assert_eq!(surf.index.format, "surface");

                let mut omitted_total = 0u64;
                for f in cbor.files.iter().filter(|f| f.view.is_some()) {
                    let smy = f.path.replace(".cbor", ".smy");
                    let s = surf
                        .file(&smy)
                        .unwrap_or_else(|| panic!("{cell}: no {smy}"));
                    let written = smysl_core::from_cbor_seq(&f.bytes).unwrap().0;
                    let read =
                        smysl_core::surface::parse_surface(std::str::from_utf8(&s.bytes).unwrap())
                            .unwrap()
                            .records;
                    let view = match &written[0] {
                        Record::View(v) => v.id.clone(),
                        _ => unreachable!(),
                    };
                    // A label is spellable only on its unit's declaration in this file.
                    let declared: BTreeSet<Uid> = written
                        .iter()
                        .filter_map(|r| r.as_unit().map(smysl_core::canonical_uid))
                        .collect();
                    let labels: BTreeMap<Label, Uid> = written
                        .iter()
                        .filter_map(|r| match r {
                            Record::LabelBinding(b) if declared.contains(&b.uid) => {
                                Some((b.label.clone(), b.uid))
                            }
                            _ => None,
                        })
                        .collect();
                    let ctx = WriteContext::from_labels(&labels).with_relations(store.relations());
                    let spellable: Vec<Record> = written
                        .iter()
                        .enumerate()
                        .filter(|(i, r)| *i == 0 || has_surface_form(r, &ctx, Some(&view)))
                        .map(|(_, r)| r.clone())
                        .collect();
                    omitted_total += (written.len() - spellable.len()) as u64;
                    assert_eq!(
                        hashes(&read),
                        hashes(&spellable),
                        "{cell} {smy}: reads back as the rest"
                    );
                }
                assert_eq!(
                    surf.index.counts["SMY-W428"], omitted_total,
                    "{cell}: W428 counts them"
                );

                let v = verify_collection(&surf);
                assert!(v.holds(), "{cell}: {v:?}");
                assert!(
                    matches!(v.p3, Property::NotApplicable(_)),
                    "{cell}: P3 is cbor's"
                );
            }
        }
    }
}

#[test]
fn a_surface_collection_imports_as_what_it_spells() {
    let store = Store::from_records(common::fixture("fixtures/corpus/F13-canon.smy"));
    let c = export_store(
        &store,
        &Selection::all(&store),
        &ExportOptions::new().format(Format::Surface),
    )
    .unwrap();
    let out = dir("surface-out");
    write_collection(&c, &out).unwrap();
    let mut lib = Library::create(&dir("surface-to")).unwrap();
    let r = import(
        &mut lib,
        &[Input::Index(out.join(INDEX_FILE))],
        &ImportOptions::new().accept(true),
    )
    .unwrap();
    assert_eq!(
        smysl_graph::record_set_digest(&retained(&lib, &r.name).unwrap()),
        c.rsd()
    );
}

fn packed(c: &Collection) -> usize {
    c.records()
        .iter()
        .filter(|r| matches!(r, Record::PackInfo(_)))
        .count()
}

#[test]
fn a_pack_collection_is_lossy_and_says_so() {
    let store = Store::from_records(common::fixture("fixtures/corpus/F1-incident.smy"));
    for layout in [Layout::Single, Layout::ByView] {
        let c = export_store(
            &store,
            &Selection::all(&store),
            &ExportOptions::new()
                .layout(layout)
                .format(Format::Pack { budget: 4000 }),
        )
        .unwrap();
        assert_eq!(c.index.format, "pack");
        assert_eq!(c.index.derived.as_deref(), Some("lossy"));
        let keyed = c.files.iter().filter(|f| f.view.is_some()).count();
        assert!(c
            .files
            .iter()
            .filter(|f| f.view.is_some())
            .all(|f| f.path.ends_with(".pack.cbor")));
        assert_eq!(packed(&c), keyed, "one pack info per file");
        let v = verify_collection(&c);
        assert!(v.holds(), "{v:?}");
        assert!(matches!(v.p3, Property::NotApplicable(_)));
    }
    // A budget no unit fits in packs nothing, and its pack info says so.
    let c = export_store(
        &store,
        &Selection::all(&store),
        &ExportOptions::new().format(Format::Pack { budget: 1 }),
    )
    .unwrap();
    assert_eq!(packed(&c), 1);
    assert!(!c.records().iter().any(|r| matches!(r, Record::Unit(_))));
}
