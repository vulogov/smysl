//! The operational journal: which units came out of which part.
//!
//! The one table of §3.8 that is **operational** rather than derived from the corpus's
//! identities, which is why its exit clause is worded differently: §6's TX-P4 step 1 asks that
//! "the operational `journal` is rebuilt by re-checking staged/stored units", where the other
//! tables are asked to survive deletion. The difference is that a derived table can always be
//! recomputed exactly, and this one cannot: the journal ingest writes carries what a run
//! *cost* — tokens, retries, which model, which recipe — and no amount of re-checking the
//! units recovers a number nobody recorded.
//!
//! So the clause splits, and this module answers the half that exists:
//!
//! - **Coverage** — which units came out of which part, and at what status. Recomputable from
//!   the units alone, and recomputed here. This is what makes a lost journal survivable: the
//!   corpus can still say what was ingested, which is the question a resume asks.
//! - **Cost** — the per-language retry rate and the token figures TX-P5's phase exit wants a
//!   report of (§6, TX-P5 step 5). Written by the ingest that spends it, and not recoverable.
//!   There is no writer in this build, so there is nothing here to recover and nothing to
//!   pretend about.
//!
//! The staged half of "staged/stored" is the same story from the other end. Today's staging is
//! `.smysl/staged.smy` under a *project* root (`smysl_ingest::stage`), and the per-shard
//! `<library>/stage/<shard>/staged.smy` that §3.9 names arrives with TX-P5. A scanner written
//! here would be a reader for a file no writer in this build produces — the shape of mistake
//! TX-P2 step 4 removed when it deleted `by_mid` for having no source of values. It arrives
//! with the writer, in the commit that can test it.

use smysl_core::ids::{Tid, Uid};
use smysl_core::types::Record;
use smysl_core::Status;
use smysl_graph::Store;

use crate::library::Library;

/// One unit, and the part it came out of.
///
/// Flat, and sorted by `(tid, uid)` rather than grouped under the tid, because the row *is* the
/// pair: a journal grouped by part would have to decide what to do with a unit whose source
/// names a part no manifest records, and the answer is that it is still a row — a dangling
/// source is a thing to report (`SMY-E403`), not a thing to drop from the journal that would
/// have reported it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct Row {
    pub tid: Tid,
    pub uid: Uid,
    pub status: Status,
}

/// Rebuild the coverage half by re-checking every stored unit.
pub fn rebuild(lib: &Library) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::new();
    for (uid, unit) in lib.store().units() {
        if let Some(row) = row_of(*uid, unit.core.source.as_ref(), unit.core.status) {
            insert_sorted(&mut out, row);
        }
    }
    out
}

/// Bring the coverage half forward over an appended batch.
pub fn on_append(table: &mut Vec<Row>, _lib: &Library, records: &[Record]) {
    for record in records {
        let Record::Unit(core) = record else { continue };
        let uid = smysl_core::canonical_uid(core);
        if let Some(row) = row_of(uid, core.source.as_ref(), core.status) {
            insert_sorted(table, row);
        }
    }
}

/// The row a unit contributes, if its source names a part.
///
/// Both builders go through here and through [`Store::part_of_reference`], so the two paths
/// cannot disagree about what counts as naming a part. They did not have that option before:
/// the canonical-only parse was private to `smysl-graph`, and the alternative to exposing it
/// was a second copy of the one rule it exists to hold.
fn row_of(uid: Uid, source: Option<&smysl_core::types::SourceRef>, status: Status) -> Option<Row> {
    let tid = Store::part_of_reference(&source?.reference)?;
    Some(Row { tid, uid, status })
}

/// Insert in `(tid, uid)` order, ignoring a duplicate.
///
/// A uid is content-addressed, so the same unit delivered twice is one row with one status and
/// the dedup needs no tie-break. Which is also why this is sorted rather than appended: a
/// rebuild walks units in uid order and an append walks them in arrival order, and §3.8's byte
/// comparison is written against what reaches the disk.
fn insert_sorted(v: &mut Vec<Row>, row: Row) {
    if let Err(at) = v.binary_search(&row) {
        v.insert(at, row);
    }
}
