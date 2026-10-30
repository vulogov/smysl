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
///
/// Collected and then sorted, rather than inserted in place one row at a time. The difference is
/// not stylistic: an ordered insert into a `Vec` moves the tail, so building a table of *n* rows
/// that way is `O(n²)` in bytes moved, and TX-P4 step 2's timing harness measured what that cost
/// at the size the phase is sized for — **4.53 s** of a 4.96 s open at 155k units, six times the
/// whole postings table. Step 1 put this table in the class it called arithmetic over a parsed
/// log, and the reasoning was right about the table and wrong about the loop. It is 47 ms now.
pub fn rebuild(lib: &Library) -> Vec<Row> {
    let mut out: Vec<Row> = lib
        .store()
        .units()
        .filter_map(|(uid, unit)| row_of(*uid, unit.core.source.as_ref(), unit.core.status))
        .collect();
    out.sort_unstable();
    // A uid is content-addressed, so a duplicate row is the same unit twice and carries the same
    // status; `dedup` after a sort is the same answer `merge_sorted`'s gives.
    out.dedup();
    out
}

/// Bring the coverage half forward over an appended batch.
pub fn on_append(table: &mut Vec<Row>, _lib: &Library, records: &[Record]) {
    let mut rows: Vec<Row> = records
        .iter()
        .filter_map(|record| {
            let Record::Unit(core) = record else {
                return None;
            };
            row_of(
                smysl_core::canonical_uid(core),
                core.source.as_ref(),
                core.status,
            )
        })
        .collect();
    merge_sorted(table, &mut rows);
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

/// Merge a batch into the table in `(tid, uid)` order, ignoring a duplicate.
///
/// A uid is content-addressed, so the same unit delivered twice is one row with one status and
/// the dedup needs no tie-break. Which is also why this is sorted rather than appended: a
/// rebuild walks units in uid order and an append walks them in arrival order, and §3.8's byte
/// comparison is written against what reaches the disk.
///
/// One sort of the batch and one pass over the table from its end, not an insert per row: each
/// insert moved every row after it, and a thousand-unit batch into a million-row table was
/// 0.68 s of a 0.69 s append, most of building GE-T7's stand-in (SMYSL-2.8 §6.1, P13c.2). Here
/// each row of the table moves at most once, by `copy_within`, and only the rows after the
/// batch's first: a single row costs what one insert did.
fn merge_sorted(v: &mut Vec<Row>, batch: &mut Vec<Row>) {
    batch.sort_unstable();
    batch.dedup();
    batch.retain(|row| v.binary_search(row).is_err());
    let Some(&first) = batch.first() else {
        return;
    };
    let mut end = v.len();
    v.resize(end + batch.len(), first);
    for (j, row) in batch.iter().enumerate().rev() {
        // Rows of the table greater than this one move up past the j batch rows before it.
        let at = v[..end].partition_point(|r| r < row);
        v.copy_within(at..end, at + j + 1);
        v[at + j] = *row;
        end = at;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A batch merged in is the table sorted and deduplicated with it, whatever the batch
    /// repeats of itself or of the table, and wherever its rows fall.
    #[test]
    fn a_merged_batch_is_the_sorted_union() {
        let row = |i: u32| Row {
            tid: crate::ids::tid(&crate::norm::Normalised::of(&format!("part {}", i % 7))),
            uid: Uid::from_bytes(smysl_core::hash_bytes(&i.to_be_bytes())),
            status: Status::Cited,
        };
        let mut table: Vec<Row> = Vec::new();
        let mut want: Vec<Row> = Vec::new();
        let mut x = 7u32;
        for size in [0usize, 1, 1, 5, 40, 3, 200, 1, 0, 17] {
            let mut batch: Vec<Row> = (0..size)
                .map(|_| {
                    x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    row(x % 500)
                })
                .collect();
            want.extend(batch.iter().copied());
            want.sort_unstable();
            want.dedup();
            merge_sorted(&mut table, &mut batch);
            assert_eq!(table, want, "after a batch of {size}");
        }
    }
}
