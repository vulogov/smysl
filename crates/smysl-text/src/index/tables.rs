//! One `rebuild`/`on_append` pair per table of RFC SMYSL-2.4 §3.8.
//!
//! The pair is the unit of correctness here, so the two halves of each table sit beside each
//! other rather than in a `rebuild` module and an `on_append` module. A table whose two
//! builders are written a hundred lines apart is a table whose two builders drift.
//!
//! # What makes the two agree
//!
//! A rebuild walks the corpus in **identity order** — `BTreeMap` iteration over mids and tids —
//! and an append walks it in **arrival order**. So no table may hold a value that depends on
//! which manifest was seen last. Three of the six are keyed by something that determines their
//! value and are therefore safe by construction: a reading's rdid determines its rows, a mid
//! determines its locators, and a redaction count is read back out of the store rather than
//! incremented. The other three needed the shape choosing:
//!
//! - `parts` keeps a **set** of what each manifest records, not the latest one ([`super::PartRow`]).
//! - `by_part` keeps its mids sorted rather than appended.
//! - `manifests` has one field that is a fact about *other* manifests — `head` — and so the
//!   only incremental step here that reaches backwards: a successor unseats its predecessor.
//!
//! # Why the backing is not redb yet
//!
//! §3.8 names `index.redb` behind feature `substrate-redb`, and that feature is not here. The
//! reason is `rust-version`, which Cargo gives per **package** and not per feature — the
//! comment in the workspace manifest says so, which is why `smysl-tui` and the facade declare
//! 1.88 for every build and not only for the feature that needs it. `redb` 4.3 requires 1.90,
//! and `smysl-text` is a base-tier crate at 1.85 that `smysl-check` and the facade depend on.
//! Adding the dependency here would move the floor for every build of the substrate to buy a
//! backing nobody had turned on. The tables, the builders and the gate are what this step is
//! for; where the bytes rest is a backing decision, and §3.8's own sentence — "the same
//! queries run against in-memory maps, slower, same answers" — is the licence to make it
//! separately. The sidecar makes the featureless path cost one rebuild rather than one per
//! open, which is a third option §3.8 did not have and strictly better than the one it
//! described.

use std::collections::BTreeMap;

use smysl_core::ids::{Mid, Rdid, Tid};
use smysl_core::types::Record;

use super::{LocatorTable, ManifestRow, PartEntryRow, PartRow, SegmentsRow, Where};
use crate::library::Library;
use crate::limits::Caps;

// ---------------------------------------------------------------------------
// manifests
// ---------------------------------------------------------------------------

pub fn manifests_rebuild(lib: &Library) -> BTreeMap<Mid, ManifestRow> {
    lib.store()
        .manifests()
        .map(|(mid, m)| {
            (
                *mid,
                ManifestRow {
                    alias: m.alias.clone(),
                    lang: m.lang.as_str().to_string(),
                    reader: m.reader.clone(),
                    licence: m.licence.clone(),
                    length: m.length(),
                    parts: m.parts.len() as u32,
                    head: !lib.store().is_superseded(mid),
                },
            )
        })
        .collect()
}

/// Insert the appended manifests, and unseat whatever they supersede.
///
/// The one place a table reaches backwards. `head` is not a property of the manifest whose
/// record arrived: it is the absence of a successor, so the record that *creates* a successor
/// is the one that changes an older row. Reading `is_superseded` back out of the store rather
/// than clearing the flag directly is what makes it idempotent — the same manifest delivered
/// twice asks the same question twice and gets the same answer.
pub fn manifests_on_append(
    table: &mut BTreeMap<Mid, ManifestRow>,
    lib: &Library,
    records: &[Record],
) {
    for record in records {
        let Record::Manifest(m) = record else {
            continue;
        };
        let mid = m.mid();
        table.insert(
            mid,
            ManifestRow {
                alias: m.alias.clone(),
                lang: m.lang.as_str().to_string(),
                reader: m.reader.clone(),
                licence: m.licence.clone(),
                length: m.length(),
                parts: m.parts.len() as u32,
                head: !lib.store().is_superseded(&mid),
            },
        );
        if let Some(previous) = m.supersedes {
            if let Some(row) = table.get_mut(&previous) {
                row.head = !lib.store().is_superseded(&previous);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// parts
// ---------------------------------------------------------------------------

pub fn parts_rebuild(lib: &Library) -> BTreeMap<Tid, PartRow> {
    let mut out: BTreeMap<Tid, PartRow> = BTreeMap::new();
    for (_, m) in lib.store().manifests() {
        for entry in &m.parts {
            out.entry(entry.tid).or_default().absorb(PartEntryRow {
                length: entry.length,
                structure: entry.structure,
                rdid: entry.rdid,
            });
        }
    }
    out
}

pub fn parts_on_append(table: &mut BTreeMap<Tid, PartRow>, _lib: &Library, records: &[Record]) {
    for record in records {
        let Record::Manifest(m) = record else {
            continue;
        };
        for entry in &m.parts {
            table.entry(entry.tid).or_default().absorb(PartEntryRow {
                length: entry.length,
                structure: entry.structure,
                rdid: entry.rdid,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// by-part
// ---------------------------------------------------------------------------

pub fn by_part_rebuild(lib: &Library) -> BTreeMap<Tid, Vec<Mid>> {
    let mut out: BTreeMap<Tid, Vec<Mid>> = BTreeMap::new();
    for (mid, m) in lib.store().manifests() {
        for entry in &m.parts {
            insert_sorted(out.entry(entry.tid).or_default(), *mid);
        }
    }
    out
}

pub fn by_part_on_append(table: &mut BTreeMap<Tid, Vec<Mid>>, _lib: &Library, records: &[Record]) {
    for record in records {
        let Record::Manifest(m) = record else {
            continue;
        };
        let mid = m.mid();
        for entry in &m.parts {
            insert_sorted(table.entry(entry.tid).or_default(), mid);
        }
    }
}

/// Insert in sort order, ignoring a duplicate.
///
/// A manifest that records the same part twice — a text whose two parts hashed to one tid,
/// which is a text with a repeated section — must appear once, and a manifest delivered twice
/// must not double the list. Both fall out of this.
fn insert_sorted(v: &mut Vec<Mid>, mid: Mid) {
    if let Err(at) = v.binary_search(&mid) {
        v.insert(at, mid);
    }
}

// ---------------------------------------------------------------------------
// redactions
// ---------------------------------------------------------------------------

pub fn redactions_rebuild(lib: &Library) -> BTreeMap<Tid, u32> {
    lib.store()
        .redactions()
        .map(|(tid, set)| (*tid, set.len() as u32))
        .collect()
}

/// Recount from the store rather than incrementing.
///
/// Two peers may each redact the same part, and both records are facts the store keeps as a
/// set — so counting arrivals would count a redaction twice that the store holds once, and the
/// byte comparison would catch it. Asking the store is also what makes this idempotent.
pub fn redactions_on_append(table: &mut BTreeMap<Tid, u32>, lib: &Library, records: &[Record]) {
    for record in records {
        let Record::Redaction(r) = record else {
            continue;
        };
        if let Some((tid, set)) = lib.store().redactions().find(|(tid, _)| **tid == r.tid) {
            table.insert(*tid, set.len() as u32);
        }
    }
}

// ---------------------------------------------------------------------------
// segments
// ---------------------------------------------------------------------------

pub fn segments_rebuild(lib: &Library) -> BTreeMap<Rdid, SegmentsRow> {
    let mut out = BTreeMap::new();
    for (_, m) in lib.store().manifests() {
        for entry in &m.parts {
            read_segments(&mut out, lib, entry.rdid);
        }
    }
    out
}

pub fn segments_on_append(
    table: &mut BTreeMap<Rdid, SegmentsRow>,
    lib: &Library,
    records: &[Record],
) {
    for record in records {
        match record {
            Record::Manifest(m) => {
                for entry in &m.parts {
                    read_segments(table, lib, entry.rdid);
                }
            }
            // Rule Z unlinks the readings of a redacted part, so the rows have to go with
            // them. This arm is not symmetry for its own sake: `redact` appends the record
            // **before** it unlinks the objects, deliberately, so that a process killed
            // between the two is repairable on open — which means the incremental builder runs
            // while the objects are still there and cannot learn of the loss by failing to
            // read one. It has to be told by the record.
            Record::Redaction(r) => {
                let gone: Vec<Rdid> = table
                    .iter()
                    .filter(|(_, row)| row.tid == r.tid)
                    .map(|(rdid, _)| *rdid)
                    .collect();
                for rdid in gone {
                    table.remove(&rdid);
                }
            }
            _ => {}
        }
    }
}

/// Read one reading out of the object store, or leave the table alone.
///
/// Silent on failure, and that is the module's leniency rule rather than a swallowed error: a
/// reading object is missing for a redacted part and unreadable for a corrupt one, neither of
/// which may fail an open. What it costs is a row, and what a missing row costs is the search
/// path — which is the answer the library gave before this table existed.
fn read_segments(table: &mut BTreeMap<Rdid, SegmentsRow>, lib: &Library, rdid: Rdid) {
    if table.contains_key(&rdid) {
        return;
    }
    let Ok(reading) = lib.reading(&rdid) else {
        return;
    };
    let mut levels: Vec<String> = reading
        .rows
        .iter()
        .map(|r| r.level.as_str().to_string())
        .collect();
    levels.sort_unstable();
    levels.dedup();
    table.insert(
        rdid,
        SegmentsRow {
            tid: reading.tid,
            reader: reading.reader.clone(),
            rows: reading.rows.len() as u32,
            levels,
        },
    );
}

// ---------------------------------------------------------------------------
// locators
// ---------------------------------------------------------------------------

pub fn locators_rebuild(lib: &Library) -> BTreeMap<Mid, LocatorTable> {
    let mids: Vec<Mid> = lib.store().manifests().map(|(mid, _)| *mid).collect();
    let mut out = BTreeMap::new();
    for mid in mids {
        if let Some(table) = locator_table(lib, &mid) {
            out.insert(mid, table);
        }
    }
    out
}

pub fn locators_on_append(
    table: &mut BTreeMap<Mid, LocatorTable>,
    lib: &Library,
    records: &[Record],
) {
    for record in records {
        match record {
            Record::Manifest(m) => {
                let mid = m.mid();
                if let Some(built) = locator_table(lib, &mid) {
                    table.insert(mid, built);
                }
            }
            // Rule Z took a part's objects away, so every manifest that records it can no
            // longer resolve. Dropping those tables rather than editing them is the only
            // correct move: a table with the redacted part's addresses removed would answer
            // *nothing is there* for a locator the search path refuses outright, and the index
            // is not allowed to change an answer.
            Record::Redaction(r) => {
                let affected: Vec<Mid> = lib
                    .store()
                    .manifests()
                    .filter(|(_, m)| m.parts.iter().any(|e| e.tid == r.tid))
                    .map(|(mid, _)| *mid)
                    .collect();
                for mid in affected {
                    table.remove(&mid);
                }
            }
            _ => {}
        }
    }
}

/// Every address one manifest's parts answer to, or `None` when one of them could not be read.
///
/// All or nothing per manifest, which is [`super`]'s completeness rule. A half-built table
/// cannot be told apart from a complete one at lookup time, so a locator that lives in the
/// part this could not read would come back as *nothing is there*.
fn locator_table(lib: &Library, mid: &Mid) -> Option<LocatorTable> {
    let manifest = lib.store().manifest(mid)?;
    let tids: Vec<Tid> = manifest.parts.iter().map(|e| e.tid).collect();
    let mut out = LocatorTable::new();
    for tid in tids {
        let structure = lib.structure(mid, &tid, &Caps::DEFAULT).ok()?;
        for node in structure.nodes() {
            // First wins, because `Library::resolve` tries parts in document order and takes
            // the first that answers. A reading refuses a table that states one address twice
            // (TX-P1 step 3's first fuzz finding), so within a part "first" is "the only"; one
            // address in two *parts* of one manifest is what this is for.
            out.entry(node.locator.to_string()).or_insert(Where {
                tid,
                start: node.range.start,
                end: node.range.end,
            });
        }
    }
    Some(out)
}
