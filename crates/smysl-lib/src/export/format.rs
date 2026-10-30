//! The two derived forms of a file (SMYSL-2.7 §3.8, TX-P11 step 10): surface and pack.
//!
//! A collection is selected and closed in CBOR first, whatever its format, and a file's
//! `surface` or `pack` form is made from its CBOR one. So every format cuts the same files
//! from the same records, and what differs is only how each file is written.

use super::ExportedFile;
use smysl_core::surface::{has_surface_form, parse_surface, write_surface, WriteContext};
use smysl_core::{Label, Record, Uid, View};
use smysl_graph::{record_set_digest, salience, view_roots, SalienceRequest, Store};
use std::collections::{BTreeMap, BTreeSet};

/// What one file's surface form left out, and whether it read back as it should.
pub(crate) struct Surface {
    pub file: ExportedFile,
    /// Records with no surface form, by type name (`SMY-W428`).
    pub omitted: BTreeMap<&'static str, usize>,
    /// When the written text does not parse back to the selection minus `omitted`, what
    /// differs (`SMY-E424`).
    pub mismatch: Option<String>,
}

/// A file's records, decoded: the generated view first, then the rest.
fn records_of(f: &ExportedFile) -> (View, Vec<Record>) {
    let mut rs = smysl_core::from_cbor_seq(&f.bytes)
        .expect("an exported file decodes")
        .0;
    let view = match rs.remove(0) {
        Record::View(v) => v,
        _ => unreachable!("a keyed file starts with its view"),
    };
    (view, rs)
}

/// The surface form of a keyed file (§3.8): `write_surface` with the file's view as its `@doc`
/// header and the labels its bindings give, then read back. The records `has_surface_form`
/// says the writer cannot spell are counted (`SMY-W428`), and what the parser reads must be
/// exactly the rest: anything else is `SMY-E424`. That is GE-T15's "report exactly the records
/// F-18 cannot spell", checked on every file of every export rather than in a test alone.
pub(crate) fn surface(f: &ExportedFile, store: &Store) -> Surface {
    let (view, records) = records_of(f);
    // Labels for the units this file declares, and no others. The writer spells a label only
    // on the declaration of its unit, so a binding whose unit another file holds, which `ref`
    // gives a file whenever it owns the binding and not the unit, has nowhere to go. Offered to
    // the context, `has_surface_form` would call it spellable and the read-back would catch
    // the writer dropping it (found on F1 under `by-view` and `ref`).
    let declared: BTreeSet<Uid> = records
        .iter()
        .filter_map(|r| r.as_unit().map(smysl_core::canonical_uid))
        .collect();
    let labels: BTreeMap<Label, Uid> = records
        .iter()
        .filter_map(|r| match r {
            Record::LabelBinding(b) if declared.contains(&b.uid) => Some((b.label.clone(), b.uid)),
            _ => None,
        })
        .collect();
    let ctx = WriteContext::from_labels(&labels).with_relations(store.relations());
    let text = write_surface(Some(&view), &records, &ctx);

    let mut omitted: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut expected: BTreeSet<[u8; 32]> = BTreeSet::new();
    expected.insert(Store::record_hash(&Record::View(view.clone())));
    for r in &records {
        if has_surface_form(r, &ctx, Some(&view.id)) {
            expected.insert(Store::record_hash(r));
        } else {
            *omitted.entry(r.type_name()).or_default() += 1;
        }
    }
    let (parsed, mismatch) = match parse_surface(&text) {
        Ok(o) => {
            let got: BTreeSet<[u8; 32]> = o.records.iter().map(Store::record_hash).collect();
            let missing = expected.difference(&got).count();
            let extra = got.difference(&expected).count();
            let why = (missing + extra > 0).then(|| {
                format!(
                    "{}: {missing} selected record(s) do not read back, and {extra} read back \
                     that were not selected",
                    f.path
                )
            });
            (o.records, why)
        }
        Err(e) => (
            Vec::new(),
            Some(format!("{}: the written text does not parse: {e}", f.path)),
        ),
    };
    let path = format!("{}.smy", f.path.trim_end_matches(".cbor"));
    Surface {
        file: ExportedFile {
            path,
            records: parsed.len(),
            rsd: record_set_digest(&parsed),
            bytes: text.into_bytes(),
            view: Some(view.id),
            requires: f.requires.clone(),
        },
        omitted,
        mismatch,
    }
}

/// The pack form of a keyed file (§3.8): `smysl_pack` over the file's selection with the
/// budget given, seeded by the file's view, written as `smysl pack` writes one: the view, the
/// selected units, the bindings of those that survived, and the pack's self-description.
/// Lossy by design, and labelled so in the index.
pub(crate) fn pack(f: &ExportedFile, budget: u64) -> Result<ExportedFile, String> {
    let (view, records) = records_of(f);
    let store = Store::from_records(
        std::iter::once(Record::View(view.clone()))
            .chain(records.iter().cloned())
            .collect(),
    );
    let sal = salience(
        &store,
        &SalienceRequest::default().seeded(view_roots(&store)),
    );
    let mut req = smysl_pack::PackRequest::default();
    req.budget = budget;
    let packed = smysl_pack::pack(&store, &sal, &req).map_err(|e| format!("{}: {e}", f.path))?;
    let mut out: BTreeMap<(u64, [u8; 32]), Record> = BTreeMap::new();
    let mut put = |r: Record| {
        out.insert((r.type_code(), Store::record_hash(&r)), r);
    };
    for u in packed.selection.keys() {
        if let Some(unit) = store.get(u) {
            put(Record::Unit(unit.core.clone()));
        }
    }
    for r in &records {
        if let Record::LabelBinding(b) = r {
            if packed.selection.contains_key(&b.uid) {
                put(r.clone());
            }
        }
    }
    put(Record::PackInfo(packed.info.clone()));
    let all: Vec<Record> = std::iter::once(Record::View(view.clone()))
        .chain(out.into_values())
        .collect();
    let mut bytes = Vec::new();
    for r in &all {
        bytes.extend_from_slice(&smysl_core::to_cbor(r));
    }
    Ok(ExportedFile {
        path: format!("{}.pack.cbor", f.path.trim_end_matches(".cbor")),
        records: all.len(),
        rsd: record_set_digest(&all),
        bytes,
        view: Some(view.id),
        requires: Vec::new(),
    })
}
