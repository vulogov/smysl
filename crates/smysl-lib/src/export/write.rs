//! Canonical writing (SMYSL-2.7 §3.7, rule D): the generated view, the order, the bytes.

use super::layout::view_id;
use super::select::Picked;
use super::ExportedFile;
use smysl_core::{to_cbor, LangTag, Record, Uid, View};
use smysl_graph::{record_set_digest, Store};
use std::collections::BTreeSet;

/// The generated view of one file (§3.4). Every field is a function of the selection.
///
/// `roots` are the units selected for the key, `threads` the threads included, `requires` the
/// schema ids whose declarations are included, `intent` is `export`. `lang` is the one
/// language every selected unit that names one names, `mul` when they name several, and `und`
/// when none names any.
pub(crate) fn view(store: &Store, slug: &str, roots: &BTreeSet<Uid>, p: &Picked<'_>) -> View {
    let langs: BTreeSet<&LangTag> = roots
        .iter()
        .filter_map(|u| store.get(u)?.core.lang.as_ref())
        .collect();
    let lang = match langs.len() {
        0 => LangTag::new("und").expect("a BCP-47 tag"),
        1 => (*langs.first().expect("one")).clone(),
        _ => LangTag::new("mul").expect("a BCP-47 tag"),
    };
    let mut v = View::new(view_id(slug), "export")
        .with_roots(roots.iter().copied())
        .with_threads(p.threads.iter().cloned())
        .requiring(p.schemas.iter().cloned());
    v.lang = lang;
    v
}

/// One keyed file: its view first, then the records by `(type code, record hash)`.
pub(crate) fn file(
    store: &Store,
    slug: &str,
    roots: &BTreeSet<Uid>,
    p: &Picked<'_>,
) -> ExportedFile {
    let view = Record::View(view(store, slug, roots, p));
    let records: Vec<&Record> = std::iter::once(&view)
        .chain(p.records.values().copied())
        .collect();
    let id = match &view {
        Record::View(v) => v.id.clone(),
        _ => unreachable!("built above"),
    };
    encode(format!("{slug}.cbor"), &records, Some(id))
}

/// `unattributed.cbor`: the records this build cannot interpret, by code and then hash.
pub(crate) fn unattributed(records: &[&Record]) -> ExportedFile {
    let mut sorted: Vec<&Record> = records.to_vec();
    sorted.sort_by_cached_key(|r| (r.type_code(), Store::record_hash(r)));
    sorted.dedup_by_key(|r| Store::record_hash(r));
    encode("unattributed.cbor".to_string(), &sorted, None)
}

fn encode(path: String, records: &[&Record], view: Option<smysl_core::ViewId>) -> ExportedFile {
    let mut bytes = Vec::new();
    for r in records {
        bytes.extend_from_slice(&to_cbor(r));
    }
    ExportedFile {
        path,
        bytes,
        records: records.len(),
        rsd: record_set_digest(records.iter().copied()),
        view,
    }
}
