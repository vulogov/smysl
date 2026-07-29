//! Dropping what supersession has already settled.
//!
//! The log is grow-only, and that is not an implementation detail: it is what makes merge a
//! join-semilattice (rule U). A union of two grow-only sets is commutative, associative and
//! idempotent, which is why peers converge with no coordinator. **Compaction removes
//! records, so it is not an operation inside that algebra.**
//!
//! What it actually is: a lossy projection producing a *new* store. Two consequences that
//! have to be said out loud rather than discovered.
//!
//! - **Compaction does not survive a merge.** A peer that still holds what was dropped will
//!   bring it back on the next union — correctly, because union is union. Compacting is a
//!   local decision about a local store, not a fact about the graph.
//! - **A retraction is a statement, and is never dropped.** Dropping a retracted unit would
//!   drop the record that it was retracted, and the next merge with a peer holding the
//!   original would resurrect it *without* its retraction. That turns compaction into a way
//!   of un-retracting things, which is the one outcome worth refusing outright.
//!
//! So this drops exactly what a replacement has made redundant: a superseded unit whose
//! successor is present and which nothing surviving still points at. Everything else stays.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::{canonical_uid, Record, RelKind, Uid};

use crate::store::Store;

/// What compaction dropped, and what it refused to.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Compacted {
    /// The records to keep. A new store, not an edit of the old one.
    pub records: Vec<Record>,
    /// Units dropped because a successor made them redundant.
    pub dropped: BTreeSet<Uid>,
    /// Superseded units kept anyway because something surviving still references them.
    /// Run `relink` first and these become droppable.
    pub still_referenced: BTreeSet<Uid>,
    /// Superseded units kept because they are also retracted. A retraction must outlive
    /// the thing it retracts, or compaction becomes a way to un-retract.
    pub retracted: BTreeSet<Uid>,
    pub records_before: usize,
}

impl Compacted {
    pub fn is_empty(&self) -> bool {
        self.dropped.is_empty()
    }

    /// Records removed.
    pub fn saved(&self) -> usize {
        self.records_before.saturating_sub(self.records.len())
    }
}

/// Units that some `retracts` edge names, either end.
fn retracted(store: &Store) -> BTreeSet<Uid> {
    store
        .relations_of_kind(&RelKind::Retracts)
        .iter()
        .flat_map(|r| [r.from, r.to])
        .collect()
}

/// Compact a store: drop superseded units that nothing surviving needs.
///
/// The result is a new record set. It is never written in place by this function, because a
/// compaction that turned out to be wrong is not undoable from the compacted store.
pub fn compact(store: &Store) -> Compacted {
    let mut out = Compacted {
        records_before: store.iter().count(),
        ..Compacted::default()
    };

    // A unit is superseded when something supersedes it and that successor is still here.
    // A successor that is itself missing means the replacement never arrived, and dropping
    // the original would lose the content entirely.
    let mut superseded: BTreeSet<Uid> = BTreeSet::new();
    for r in store.relations_of_kind(&RelKind::Supersedes) {
        if store.get(&r.from).is_some() && store.get(&r.to).is_some() {
            superseded.insert(r.to);
        }
    }
    if superseded.is_empty() {
        out.records = store.iter().cloned().collect();
        return out;
    }

    out.retracted = superseded
        .intersection(&retracted(store))
        .copied()
        .collect();

    // Anything a *survivor* points at has to stay, whatever its supersession status: a
    // dropped ground is a dangling reference, and rule M cannot be checked through one.
    // `relink` is what moves those references onto the successors first.
    let mut needed: BTreeSet<Uid> = BTreeSet::new();
    for (uid, unit) in store.units() {
        if superseded.contains(uid) {
            continue;
        }
        needed.extend(unit.core.grounds.iter().copied());
        needed.extend(unit.core.deps.iter().copied());
    }
    // Relations between two survivors count as needing both ends, but a `supersedes` edge
    // *into* a dropped unit is the edge that made it droppable and goes with it.
    for rel in store.relations() {
        if rel.kind == RelKind::Supersedes && superseded.contains(&rel.to) {
            continue;
        }
        if !superseded.contains(&rel.from) && !superseded.contains(&rel.to) {
            continue;
        }
        needed.insert(rel.from);
        needed.insert(rel.to);
    }

    out.still_referenced = superseded.intersection(&needed).copied().collect();
    out.dropped = superseded
        .iter()
        .filter(|u| !needed.contains(u) && !out.retracted.contains(u))
        .copied()
        .collect();

    if out.dropped.is_empty() {
        out.records = store.iter().cloned().collect();
        return out;
    }

    // Keep everything except the dropped units, their attestations, and the edges that
    // touch them. An orphaned edge would fail integrity, so leaving one behind would trade
    // a smaller store for a broken one.
    let keep_unit = |u: &Uid| !out.dropped.contains(u);
    out.records = store
        .iter()
        .filter(|r| match r {
            Record::Unit(core) => keep_unit(&canonical_uid(core)),
            Record::Relation(rel) => keep_unit(&rel.from) && keep_unit(&rel.to),
            Record::Attestation(a) => keep_unit(&a.uid),
            _ => true,
        })
        .cloned()
        .collect();

    out
}

/// Compaction's saving, as a share of the records that were there.
pub fn ratio(c: &Compacted) -> f64 {
    if c.records_before == 0 {
        return 0.0;
    }
    c.saved() as f64 / c.records_before as f64
}

/// Units grouped by how many supersession steps stand behind them, for a caller that wants
/// to know how much history a store is carrying before deciding to drop any.
pub fn depth_by_unit(store: &Store) -> BTreeMap<Uid, usize> {
    let mut successor: BTreeMap<Uid, Uid> = BTreeMap::new();
    for r in store.relations_of_kind(&RelKind::Supersedes) {
        successor.insert(r.to, r.from);
    }
    let mut out = BTreeMap::new();
    for (uid, _) in store.units() {
        let mut depth = 0usize;
        let mut at = *uid;
        let mut seen = BTreeSet::new();
        while let Some(next) = successor.get(&at) {
            if !seen.insert(at) {
                break;
            }
            depth += 1;
            at = *next;
        }
        out.insert(*uid, depth);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{KernelType, Relation, Status, UnitCoreBuilder};

    fn unit(gist: &str, grounds: Vec<Uid>) -> smysl_core::UnitCore {
        let mut b = UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative);
        if !grounds.is_empty() {
            b = b.grounds(grounds);
        }
        b.build().unwrap()
    }

    /// The case compaction is for: a replaced unit nothing points at any more.
    #[test]
    fn a_superseded_unit_nothing_needs_is_dropped() {
        let old = unit("the original", vec![]);
        let new = unit("the correction", vec![]);
        let (o, n) = (canonical_uid(&old), canonical_uid(&new));
        let store = Store::from_records(vec![
            Record::Unit(old),
            Record::Unit(new),
            Record::Relation(Relation::new(RelKind::Supersedes, n, o)),
        ]);

        let out = compact(&store);
        assert_eq!(out.dropped, [o].into_iter().collect());
        // The edge that made it droppable goes with it, or integrity fails.
        let after = Store::from_records(out.records.clone());
        assert!(after.get(&o).is_none());
        assert!(after.get(&n).is_some());
        assert!(after.relations_of_kind(&RelKind::Supersedes).is_empty());
    }

    /// A dropped ground is a dangling reference, and rule M cannot be checked through one.
    /// `relink` is what moves the reference first; until it has, the unit stays.
    #[test]
    fn a_superseded_unit_something_still_references_is_kept() {
        let old = unit("the original", vec![]);
        let new = unit("the correction", vec![]);
        let (o, n) = (canonical_uid(&old), canonical_uid(&new));
        let rests = unit("rests on the original", vec![o]);

        let store = Store::from_records(vec![
            Record::Unit(old),
            Record::Unit(new),
            Record::Unit(rests),
            Record::Relation(Relation::new(RelKind::Supersedes, n, o)),
        ]);

        let out = compact(&store);
        assert!(out.dropped.is_empty(), "dropped a unit still referenced");
        assert_eq!(out.still_referenced, [o].into_iter().collect());
    }

    /// **The refusal that matters.** Dropping a retracted unit drops the record that it was
    /// retracted, and the next merge with a peer holding the original resurrects it without
    /// its retraction - which would make compaction a way to un-retract things.
    #[test]
    fn a_retracted_unit_is_never_dropped() {
        let old = unit("the retracted claim", vec![]);
        let new = unit("the correction", vec![]);
        let (o, n) = (canonical_uid(&old), canonical_uid(&new));
        let store = Store::from_records(vec![
            Record::Unit(old),
            Record::Unit(new),
            Record::Relation(Relation::new(RelKind::Supersedes, n, o)),
            Record::Relation(Relation::new(RelKind::Retracts, o, o)),
        ]);

        let out = compact(&store);
        assert!(out.dropped.is_empty());
        assert_eq!(out.retracted, [o].into_iter().collect());
    }

    /// A replacement that never arrived is not a replacement. Dropping the original would
    /// lose the content outright.
    #[test]
    fn a_supersedes_edge_to_a_missing_successor_drops_nothing() {
        let old = unit("the original", vec![]);
        let o = canonical_uid(&old);
        let ghost = Uid::from_bytes([7; 32]);
        let store = Store::from_records(vec![
            Record::Unit(old),
            Record::Relation(Relation::new(RelKind::Supersedes, ghost, o)),
        ]);
        assert!(compact(&store).dropped.is_empty());
    }

    /// Compaction must not trade a smaller store for a broken one.
    #[test]
    fn the_compacted_store_has_no_dangling_references() {
        let a = unit("first", vec![]);
        let b = unit("second", vec![]);
        let c = unit("third", vec![]);
        let (ua, ub, uc) = (canonical_uid(&a), canonical_uid(&b), canonical_uid(&c));
        let store = Store::from_records(vec![
            Record::Unit(a),
            Record::Unit(b),
            Record::Unit(c),
            Record::Relation(Relation::new(RelKind::Supersedes, ub, ua)),
            Record::Relation(Relation::new(RelKind::Supersedes, uc, ub)),
        ]);

        let out = compact(&store);
        let after = Store::from_records(out.records.clone());
        let mut report = smysl_core::Report::new();
        after.report_dangling(&mut report);
        assert!(report.is_empty(), "{report}");
    }

    /// A store with nothing replaced is left exactly as it was.
    #[test]
    fn a_store_with_no_supersession_is_untouched() {
        let a = unit("a", vec![]);
        let b = unit("b", vec![canonical_uid(&a)]);
        let store = Store::from_records(vec![Record::Unit(a), Record::Unit(b)]);

        let out = compact(&store);
        assert!(out.is_empty());
        assert_eq!(out.records.len(), out.records_before);
        assert_eq!(out.saved(), 0);
    }

    /// **Compaction is not preserved by merge**, and pretending otherwise would be the
    /// dangerous reading. A peer still holding what was dropped brings it back on the next
    /// union - correctly, because union is union. This pins the semantics rather than a
    /// wish.
    #[test]
    fn merging_an_uncompacted_peer_resurrects_what_was_dropped() {
        let old = unit("the original", vec![]);
        let new = unit("the correction", vec![]);
        let (o, n) = (canonical_uid(&old), canonical_uid(&new));
        let full = vec![
            Record::Unit(old),
            Record::Unit(new),
            Record::Relation(Relation::new(RelKind::Supersedes, n, o)),
        ];
        let store = Store::from_records(full.clone());

        let out = compact(&store);
        assert_eq!(out.dropped, [o].into_iter().collect());

        // The peer never compacted, so the union has the dropped unit again.
        let mut merged = Store::from_records(out.records.clone());
        merged
            .append(&full)
            .expect("a union of records always appends");
        assert!(
            merged.get(&o).is_some(),
            "compaction was treated as a fact about the graph rather than a local choice"
        );
    }

    /// Deterministic, so two runs of a compaction agree (rule D).
    #[test]
    fn compaction_is_reproducible() {
        let old = unit("original", vec![]);
        let new = unit("correction", vec![]);
        let (o, n) = (canonical_uid(&old), canonical_uid(&new));
        let store = Store::from_records(vec![
            Record::Unit(old),
            Record::Unit(new),
            Record::Relation(Relation::new(RelKind::Supersedes, n, o)),
        ]);
        assert_eq!(compact(&store).records, compact(&store).records);
    }

    /// How much history a store carries, for a caller deciding whether to compact at all.
    #[test]
    fn depth_counts_the_supersession_steps_behind_a_unit() {
        let a = unit("first", vec![]);
        let b = unit("second", vec![]);
        let c = unit("third", vec![]);
        let (ua, ub, uc) = (canonical_uid(&a), canonical_uid(&b), canonical_uid(&c));
        let store = Store::from_records(vec![
            Record::Unit(a),
            Record::Unit(b),
            Record::Unit(c),
            Record::Relation(Relation::new(RelKind::Supersedes, ub, ua)),
            Record::Relation(Relation::new(RelKind::Supersedes, uc, ub)),
        ]);

        let d = depth_by_unit(&store);
        assert_eq!(d[&ua], 2, "two replacements stand behind the first");
        assert_eq!(d[&ub], 1);
        assert_eq!(d[&uc], 0, "the newest has nothing behind it");
    }
}
