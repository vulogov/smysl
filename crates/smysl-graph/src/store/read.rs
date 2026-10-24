//! `StoreRead` and `StoreWrite` (SMYSL-2.8 §3.2–§3.3; TX-P13a step 4).
//!
//! The in-memory [`Store`] is one implementation of what a store is asked; a paged store, a
//! union of shards and an overlay of a batch will be others. The method set is the inventory of
//! what consumers call (§2.6), not `Store`'s whole surface.
//!
//! - **Sealed.** Every implementation lives in this crate, and the trait has to be able to grow in
//!   a minor version (`API_CONTRACT.md` freezes an unsealed trait's method set).
//! - **Generic, not object-safe.** A unit comes back through [`StoreRead::UnitRef`] — `&Unit`
//!   from the in-memory store, an owned handle from a paged one — and that needs a generic
//!   associated type. Consumers take `S: StoreRead + ?Sized`.
//! - **Required methods** are the ones an implementation answers from its own storage;
//!   **provided** ones are compositions every implementation would otherwise copy. `Store`
//!   overrides every provided method with its inherent one, so a generic caller gets exactly
//!   the answer, and the cost, a concrete caller does.
//! - **Uid order** for everything that lists units.
//!
//! Inherent methods of the same name keep their signatures on `Store`, and inherent methods win
//! method resolution, so nothing that calls them on a concrete `Store` changes.

use std::borrow::Cow;
use std::collections::{btree_map, btree_set, BTreeSet};
use std::ops::Deref;
use std::rc::Rc;
use std::sync::Arc;

use smysl_core::diag::{Code, Diagnostic, Report, Subject};
use smysl_core::{
    AgentId, Attestation, Commit, Commitment, Contention, ContentionStatus, DetectionKind, Error,
    IntegrityError, Record, RelKind, Relation, Resolution, ResolutionTarget, Thread, ThreadId, Uid,
    UidPrefix, Unit, View, ViewId, Withdrawal,
};

use super::interp::{Interpretive, RelKey};
use super::library::StoreLibrary;
use super::{AppendReport, Store};
use crate::adjacency::{Adjacency, EdgeSet};
use crate::traverse;

pub(crate) mod sealed {
    use std::borrow::Cow;

    use smysl_core::Attestation;

    use crate::store::interp::Interpretive;
    use crate::Store;

    /// Seals [`super::StoreRead`]; and, because nothing outside this crate can name it, a place
    /// for what the crate's own operations need of a store that callers must not reach.
    pub trait Sealed {
        /// The store as an in-memory [`Store`] that can be changed, for an operation that
        /// simulates one (`plan_retraction`). The in-memory store lends itself, so the caller's
        /// `into_owned()` is the clone it always paid; a paged store builds one from its records.
        fn owned(&self) -> Cow<'_, Store>;

        /// The interpretive layer, for an [`crate::Overlay`] to start its own from.
        fn interp(&self) -> &Interpretive;

        /// Attestations whose subject has not arrived: a batch laid over this store may hold it.
        fn pending_attestations(&self) -> &[Attestation];

        /// Whether a record with this hash ([`Store::record_hash`]) is in the store.
        fn has_record(&self, h: &[u8; 32]) -> bool;

        /// Every record hash, ascending: the input of the record-set digest.
        fn record_hashes(&self) -> Box<dyn Iterator<Item = &[u8; 32]> + '_>;
    }
}

/// Records that have no field of their own in a store, by kind (§3.2's typed accessors).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum HotKind {
    SchemaDecls,
    LabelBindings,
    PackInfos,
    /// Records of a type this build cannot decode (`SMY-W014`), kept per rule X.
    Unknown,
}

impl HotKind {
    /// The kind a record is held under, if it is one of these.
    pub fn of(r: &Record) -> Option<HotKind> {
        match r {
            Record::SchemaDecl(_) => Some(HotKind::SchemaDecls),
            Record::LabelBinding(_) => Some(HotKind::LabelBindings),
            Record::PackInfo(_) => Some(HotKind::PackInfos),
            Record::Unknown { .. } => Some(HotKind::Unknown),
            _ => None,
        }
    }
}

/// Relations, in canonical `(kind, from, to)` order.
#[derive(Debug, Clone)]
pub struct Relations<'a>(pub(crate) btree_map::Values<'a, RelKey, Relation>);

impl<'a> Iterator for Relations<'a> {
    type Item = &'a Relation;
    fn next(&mut self) -> Option<&'a Relation> {
        self.0.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

/// Threads, by `(id, owner)`.
#[derive(Debug, Clone)]
pub struct Threads<'a>(pub(crate) btree_map::Values<'a, (ThreadId, AgentId), Thread>);

impl<'a> Iterator for Threads<'a> {
    type Item = &'a Thread;
    fn next(&mut self) -> Option<&'a Thread> {
        self.0.next()
    }
}

/// Views, by id.
#[derive(Debug, Clone)]
pub struct Views<'a>(pub(crate) btree_map::Values<'a, ViewId, View>);

impl<'a> Iterator for Views<'a> {
    type Item = &'a View;
    fn next(&mut self) -> Option<&'a View> {
        self.0.next()
    }
}

/// Withdrawals, grouped by the rid they name, in rid order.
#[derive(Debug, Clone)]
pub struct Withdrawals<'a>(
    pub(crate) std::iter::Flatten<btree_map::Values<'a, Uid, BTreeSet<Withdrawal>>>,
);

impl<'a> Iterator for Withdrawals<'a> {
    type Item = &'a Withdrawal;
    fn next(&mut self) -> Option<&'a Withdrawal> {
        self.0.next()
    }
}

/// Resolutions, in their order.
#[derive(Debug, Clone)]
pub struct Resolutions<'a>(pub(crate) btree_set::Iter<'a, Resolution>);

impl<'a> Iterator for Resolutions<'a> {
    type Item = &'a Resolution;
    fn next(&mut self) -> Option<&'a Resolution> {
        self.0.next()
    }
}

/// Commitments, by the uid they name — present in the store or not — in uid order.
#[derive(Debug, Clone)]
pub struct Commits<'a>(pub(crate) btree_map::Iter<'a, Uid, BTreeSet<Commit>>);

impl<'a> Iterator for Commits<'a> {
    type Item = (&'a Uid, &'a BTreeSet<Commit>);
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}

/// The records of one [`HotKind`], in log order.
#[derive(Debug, Clone)]
pub struct HotRecords<'a> {
    pub(crate) records: &'a [Record],
    pub(crate) at: std::slice::Iter<'a, usize>,
    /// What follows, for an [`crate::Overlay`]: its base's records of the kind, then its batch's.
    pub(crate) then: Option<Box<HotRecords<'a>>>,
}

impl<'a> Iterator for HotRecords<'a> {
    type Item = &'a Record;
    fn next(&mut self) -> Option<&'a Record> {
        match self.at.next() {
            Some(&i) => Some(&self.records[i]),
            None => self.then.as_mut()?.next(),
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let (lo, hi) = self.at.size_hint();
        match &self.then {
            None => (lo, hi),
            Some(t) => {
                let (l, h) = t.size_hint();
                (lo + l, hi.zip(h).map(|(a, b)| a + b))
            }
        }
    }
}

pub(crate) fn empty_commits() -> &'static BTreeSet<Commit> {
    static EMPTY: std::sync::OnceLock<BTreeSet<Commit>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(BTreeSet::new)
}

/// What a store is asked (SMYSL-2.8 §3.2).
pub trait StoreRead: sealed::Sealed {
    /// A unit, borrowed or owned: `&Unit` for the in-memory store.
    type UnitRef<'a>: Deref<Target = Unit> + Clone + 'a
    where
        Self: 'a;
    /// Units, in ascending uid order.
    type Units<'a>: Iterator<Item = (&'a Uid, Self::UnitRef<'a>)> + 'a
    where
        Self: 'a;
    /// The log, in log order.
    type Records<'a>: Iterator<Item = Cow<'a, Record>> + 'a
    where
        Self: 'a;

    // -- required: units and their headers

    fn get(&self, uid: &Uid) -> Option<Self::UnitRef<'_>>;
    fn contains_uid(&self, uid: &Uid) -> bool;
    /// Every unit, ascending uid.
    fn units(&self) -> Self::Units<'_>;
    fn unit_count(&self) -> usize;
    /// Uids present in `lo..=hi`, ascending.
    fn units_in_range(&self, lo: &Uid, hi: &Uid) -> Vec<Uid>;
    fn observed_at(&self, uid: &Uid) -> Option<u64>;
    fn hop_of(&self, uid: &Uid) -> Option<u32>;
    fn units_with_source_prefix(&self, prefix: &str) -> Vec<Uid>;
    fn units_in_observed_order(&self) -> Vec<Uid>;

    // -- required: the graph and the interpretive layer

    fn adjacency(&self) -> &Adjacency;
    /// Every relation, withdrawn ones included, in canonical `(kind, from, to)` order.
    fn relations(&self) -> Relations<'_>;
    fn relation_by_id(&self, rid: &Uid) -> Option<&Relation>;
    /// The relation with this key, withdrawn or not.
    fn relation_by_key(&self, kind: &RelKind, from: &Uid, to: &Uid) -> Option<&Relation>;
    fn threads(&self) -> Threads<'_>;
    fn views(&self) -> Views<'_>;
    fn contentions(&self) -> &[Contention];
    fn withdrawals(&self) -> Withdrawals<'_>;
    fn resolutions(&self) -> Resolutions<'_>;
    fn commits(&self) -> Commits<'_>;
    fn commits_of(&self, uid: &Uid) -> &BTreeSet<Commit>;
    fn is_withdrawn(&self, rel: &Relation) -> bool;
    fn is_unfounded(&self, uid: &Uid) -> bool;
    /// Schema declarations, label bindings, pack infos or unknown records, in log order.
    fn hot_records(&self, kind: HotKind) -> HotRecords<'_>;
    /// The library layer: manifests, heads, redactions, datings, parts (SMYSL-2.4 §4.3.2).
    fn library(&self) -> &StoreLibrary;

    // -- required: the log

    fn records(&self) -> Self::Records<'_>;
    fn duplicate_records(&self) -> usize;
    fn record_set_digest(&self) -> [u8; 32];
    fn state_hash(&self) -> [u8; 32];

    // -- provided

    /// Relations of a kind, leaving out withdrawn ones, in canonical order.
    fn relations_of_kind(&self, kind: &RelKind) -> Vec<&Relation> {
        self.relations()
            .filter(|r| &r.kind == kind && !self.is_withdrawn(r))
            .collect()
    }

    fn has_relation(&self, kind: &RelKind, from: &Uid, to: &Uid) -> bool {
        self.relation_by_key(kind, from, to).is_some()
    }

    /// A `rebuts` edge, not withdrawn, from a unit present and not `unfounded`.
    fn is_live_rebuttal(&self, rel: &Relation) -> bool {
        rel.kind == RelKind::Rebuts
            && !self.is_withdrawn(rel)
            && self.contains_uid(&rel.from)
            && !self.is_unfounded(&rel.from)
    }

    fn rebuttals_of(&self, uid: &Uid) -> Vec<Uid> {
        let g = self.adjacency();
        let Some(id) = g.id(uid) else {
            return Vec::new();
        };
        traverse::rebuttals_of(g, id)
            .into_iter()
            .filter_map(|n| g.uid(n))
            .filter(|a| self.contains_uid(a) && !self.is_unfounded(a))
            .copied()
            .collect()
    }

    fn supports_of(&self, uid: &Uid, edges: &EdgeSet) -> Vec<Uid> {
        let g = self.adjacency();
        let Some(id) = g.id(uid) else {
            return Vec::new();
        };
        crate::lineage::one_hop(g, id, edges)
            .into_iter()
            .filter_map(|n| g.uid(n))
            .filter(|u| self.contains_uid(u))
            .copied()
            .collect()
    }

    fn is_resolved(&self, target: &ResolutionTarget) -> bool {
        self.resolutions().any(|r| &r.target == target)
    }

    fn contention_status(&self, c: &Contention) -> ContentionStatus {
        if self.is_resolved(&ResolutionTarget::Contention(c.id.clone())) {
            return ContentionStatus::Resolved;
        }
        if c.detected.kind == DetectionKind::LiveRebuttal {
            let live = c.positions.iter().any(|a| {
                self.relation_by_key(&RelKind::Rebuts, a, &c.over)
                    .is_some_and(|r| self.is_live_rebuttal(r))
            });
            if !live || self.is_unfounded(&c.over) {
                return ContentionStatus::Stale;
            }
        }
        c.status
    }

    fn open_contentions(&self) -> Vec<&Contention> {
        self.contentions()
            .iter()
            .filter(|c| self.contention_status(c) == ContentionStatus::Open)
            .collect()
    }

    /// The level of the latest commitment, by `(wall_ms, counter, agent)`.
    fn commitment_of(&self, uid: &Uid) -> Option<Commitment> {
        self.commits_of(uid)
            .iter()
            .max_by(|a, b| {
                (a.ts.wall_ms, a.ts.counter, a.agent.as_str()).cmp(&(
                    b.ts.wall_ms,
                    b.ts.counter,
                    b.agent.as_str(),
                ))
            })
            .map(|c| c.level)
    }

    fn units_at_commitment(&self, level: Commitment) -> Vec<Uid> {
        self.commits()
            .map(|(u, _)| *u)
            .filter(|u| self.commitment_of(u) == Some(level))
            .collect()
    }

    fn matching_prefix(&self, prefix: &UidPrefix) -> Vec<Uid> {
        let (lo, hi) = prefix.range();
        self.units_in_range(&lo, &hi)
    }

    fn resolve_prefix(&self, prefix: &UidPrefix) -> Result<Uid, IntegrityError> {
        let mut matches = self.matching_prefix(prefix);
        match matches.len() {
            1 => Ok(matches.remove(0)),
            _ => Err(IntegrityError::AmbiguousPrefix {
                prefix: format!("{} bits", prefix.bits()),
                candidates: matches,
            }),
        }
    }

    fn report_dangling(&self, report: &mut Report) {
        let g = self.adjacency();
        for n in g.dangling() {
            if let Some(uid) = g.uid(n) {
                report.push(
                    Diagnostic::new(Code::E060)
                        .with_subject(Subject::Unit(*uid))
                        .with_message("referenced by this store but not present in it"),
                );
            }
        }
    }

    /// Every attestation naming this uid, unit or edge. Owned here; `Store`'s inherent method
    /// lends its set.
    fn attestations_of(&self, uid: &Uid) -> Vec<Attestation> {
        if let Some(u) = self.get(uid) {
            return u.attestations.iter().cloned().collect();
        }
        if let Some(r) = self.relation_by_id(uid) {
            return r.attestations.iter().cloned().collect();
        }
        Vec::new()
    }

    fn attested_by(&self, uid: &Uid) -> BTreeSet<AgentId> {
        self.attestations_of(uid)
            .into_iter()
            .map(|a| a.agent)
            .collect()
    }

    fn agreement(&self, uid: &Uid, n: usize) -> bool {
        self.attested_by(uid).len() >= n
    }

    fn hops(&self) -> BTreeSet<u32> {
        self.units().filter_map(|(u, _)| self.hop_of(u)).collect()
    }

    fn latest_hop(&self) -> Option<u32> {
        self.hops().last().copied()
    }
}

/// A store that takes records (SMYSL-2.8 §3.3).
pub trait StoreWrite: StoreRead {
    fn append(&mut self, records: &[Record]) -> Result<AppendReport, Error>;
}

// ---------------------------------------------------------------------------
// The in-memory store
// ---------------------------------------------------------------------------

impl sealed::Sealed for Store {
    fn owned(&self) -> Cow<'_, Store> {
        Cow::Borrowed(self)
    }
    fn interp(&self) -> &Interpretive {
        &self.interp
    }
    fn pending_attestations(&self) -> &[Attestation] {
        &self.pending_attestations
    }
    fn has_record(&self, h: &[u8; 32]) -> bool {
        self.record_hashes.contains(h)
    }
    fn record_hashes(&self) -> Box<dyn Iterator<Item = &[u8; 32]> + '_> {
        Box::new(self.record_hashes.iter())
    }
}

fn borrowed(r: &Record) -> Cow<'_, Record> {
    Cow::Borrowed(r)
}

impl StoreRead for Store {
    type UnitRef<'a> = &'a Unit;
    type Units<'a> = btree_map::Iter<'a, Uid, Unit>;
    type Records<'a> =
        std::iter::Map<std::slice::Iter<'a, Record>, fn(&'a Record) -> Cow<'a, Record>>;

    fn get(&self, uid: &Uid) -> Option<&Unit> {
        Store::get(self, uid)
    }
    fn contains_uid(&self, uid: &Uid) -> bool {
        Store::contains_uid(self, uid)
    }
    fn units(&self) -> btree_map::Iter<'_, Uid, Unit> {
        self.units.iter()
    }
    fn unit_count(&self) -> usize {
        self.units.len()
    }
    fn units_in_range(&self, lo: &Uid, hi: &Uid) -> Vec<Uid> {
        if lo > hi {
            return Vec::new();
        }
        self.units.range(*lo..=*hi).map(|(u, _)| *u).collect()
    }
    fn observed_at(&self, uid: &Uid) -> Option<u64> {
        Store::observed_at(self, uid)
    }
    fn hop_of(&self, uid: &Uid) -> Option<u32> {
        Store::hop_of(self, uid)
    }
    fn units_with_source_prefix(&self, prefix: &str) -> Vec<Uid> {
        Store::units_with_source_prefix(self, prefix)
    }
    fn units_in_observed_order(&self) -> Vec<Uid> {
        Store::units_in_observed_order(self)
    }

    fn adjacency(&self) -> &Adjacency {
        Store::adjacency(self)
    }
    fn relations(&self) -> Relations<'_> {
        Relations(self.interp.relations.values())
    }
    fn relation_by_id(&self, rid: &Uid) -> Option<&Relation> {
        self.interp.relation_by_id(rid)
    }
    fn relation_by_key(&self, kind: &RelKind, from: &Uid, to: &Uid) -> Option<&Relation> {
        self.interp
            .relations
            .get(&(kind.as_str().to_string(), *from, *to))
    }
    fn threads(&self) -> Threads<'_> {
        Threads(self.interp.threads.values())
    }
    fn views(&self) -> Views<'_> {
        Views(self.interp.views.values())
    }
    fn contentions(&self) -> &[Contention] {
        Store::contentions(self)
    }
    fn withdrawals(&self) -> Withdrawals<'_> {
        Withdrawals(self.interp.withdrawals.values().flatten())
    }
    fn resolutions(&self) -> Resolutions<'_> {
        Resolutions(self.interp.resolutions.iter())
    }
    fn commits(&self) -> Commits<'_> {
        Commits(self.interp.commits.iter())
    }
    fn commits_of(&self, uid: &Uid) -> &BTreeSet<Commit> {
        self.interp
            .commits
            .get(uid)
            .unwrap_or_else(|| empty_commits())
    }
    fn is_withdrawn(&self, rel: &Relation) -> bool {
        Store::is_withdrawn(self, rel)
    }
    fn is_unfounded(&self, uid: &Uid) -> bool {
        Store::is_unfounded(self, uid)
    }
    fn hot_records(&self, kind: HotKind) -> HotRecords<'_> {
        static NONE: Vec<usize> = Vec::new();
        HotRecords {
            records: &self.records,
            at: self.hot.get(&kind).unwrap_or(&NONE).iter(),
            then: None,
        }
    }
    fn library(&self) -> &StoreLibrary {
        &self.library
    }

    fn records(&self) -> Self::Records<'_> {
        self.records
            .iter()
            .map(borrowed as fn(&Record) -> Cow<'_, Record>)
    }
    fn duplicate_records(&self) -> usize {
        Store::duplicate_records(self)
    }
    fn record_set_digest(&self) -> [u8; 32] {
        Store::record_set_digest(self)
    }
    fn state_hash(&self) -> [u8; 32] {
        Store::state_hash(self)
    }

    // Provided methods, answered by the inherent ones: one answer, one cost.
    fn relations_of_kind(&self, kind: &RelKind) -> Vec<&Relation> {
        Store::relations_of_kind(self, kind)
    }
    fn has_relation(&self, kind: &RelKind, from: &Uid, to: &Uid) -> bool {
        Store::has_relation(self, kind, from, to)
    }
    fn is_live_rebuttal(&self, rel: &Relation) -> bool {
        Store::is_live_rebuttal(self, rel)
    }
    fn rebuttals_of(&self, uid: &Uid) -> Vec<Uid> {
        Store::rebuttals_of(self, uid)
    }
    fn supports_of(&self, uid: &Uid, edges: &EdgeSet) -> Vec<Uid> {
        Store::supports_of(self, uid, edges)
    }
    fn is_resolved(&self, target: &ResolutionTarget) -> bool {
        Store::is_resolved(self, target)
    }
    fn contention_status(&self, c: &Contention) -> ContentionStatus {
        Store::contention_status(self, c)
    }
    fn open_contentions(&self) -> Vec<&Contention> {
        Store::open_contentions(self).collect()
    }
    fn commitment_of(&self, uid: &Uid) -> Option<Commitment> {
        Store::commitment_of(self, uid)
    }
    fn units_at_commitment(&self, level: Commitment) -> Vec<Uid> {
        Store::units_at_commitment(self, level)
    }
    fn report_dangling(&self, report: &mut Report) {
        Store::report_dangling(self, report)
    }
    fn attested_by(&self, uid: &Uid) -> BTreeSet<AgentId> {
        Store::attested_by(self, uid).into_iter().cloned().collect()
    }
    fn agreement(&self, uid: &Uid, n: usize) -> bool {
        Store::agreement(self, uid, n)
    }
    fn hops(&self) -> BTreeSet<u32> {
        Store::hops(self)
    }
    fn latest_hop(&self) -> Option<u32> {
        Store::latest_hop(self)
    }
}

impl StoreWrite for Store {
    fn append(&mut self, records: &[Record]) -> Result<AppendReport, Error> {
        Store::append(self, records)
    }
}

// ---------------------------------------------------------------------------
// Pointers: `check(&arc_store, …)` keeps compiling once `check` is generic (§3.2)
// ---------------------------------------------------------------------------

macro_rules! delegate {
    ($($ptr:ty),*) => {$(
        impl<S: StoreRead + ?Sized> sealed::Sealed for $ptr {
            fn owned(&self) -> Cow<'_, Store> { (**self).owned() }
            fn interp(&self) -> &Interpretive { (**self).interp() }
            fn pending_attestations(&self) -> &[Attestation] { (**self).pending_attestations() }
            fn has_record(&self, h: &[u8; 32]) -> bool { (**self).has_record(h) }
            fn record_hashes(&self) -> Box<dyn Iterator<Item = &[u8; 32]> + '_> {
                (**self).record_hashes()
            }
        }

        impl<S: StoreRead + ?Sized> StoreRead for $ptr {
            type UnitRef<'a> = S::UnitRef<'a> where Self: 'a;
            type Units<'a> = S::Units<'a> where Self: 'a;
            type Records<'a> = S::Records<'a> where Self: 'a;

            fn get(&self, uid: &Uid) -> Option<Self::UnitRef<'_>> { (**self).get(uid) }
            fn contains_uid(&self, uid: &Uid) -> bool { (**self).contains_uid(uid) }
            fn units(&self) -> Self::Units<'_> { (**self).units() }
            fn unit_count(&self) -> usize { (**self).unit_count() }
            fn units_in_range(&self, lo: &Uid, hi: &Uid) -> Vec<Uid> {
                (**self).units_in_range(lo, hi)
            }
            fn observed_at(&self, uid: &Uid) -> Option<u64> { (**self).observed_at(uid) }
            fn hop_of(&self, uid: &Uid) -> Option<u32> { (**self).hop_of(uid) }
            fn units_with_source_prefix(&self, prefix: &str) -> Vec<Uid> {
                (**self).units_with_source_prefix(prefix)
            }
            fn units_in_observed_order(&self) -> Vec<Uid> {
                (**self).units_in_observed_order()
            }
            fn adjacency(&self) -> &Adjacency { (**self).adjacency() }
            fn relations(&self) -> Relations<'_> { (**self).relations() }
            fn relation_by_id(&self, rid: &Uid) -> Option<&Relation> {
                (**self).relation_by_id(rid)
            }
            fn relation_by_key(&self, kind: &RelKind, from: &Uid, to: &Uid) -> Option<&Relation> {
                (**self).relation_by_key(kind, from, to)
            }
            fn threads(&self) -> Threads<'_> { (**self).threads() }
            fn views(&self) -> Views<'_> { (**self).views() }
            fn contentions(&self) -> &[Contention] { (**self).contentions() }
            fn withdrawals(&self) -> Withdrawals<'_> { (**self).withdrawals() }
            fn resolutions(&self) -> Resolutions<'_> { (**self).resolutions() }
            fn commits(&self) -> Commits<'_> { (**self).commits() }
            fn commits_of(&self, uid: &Uid) -> &BTreeSet<Commit> { (**self).commits_of(uid) }
            fn is_withdrawn(&self, rel: &Relation) -> bool { (**self).is_withdrawn(rel) }
            fn is_unfounded(&self, uid: &Uid) -> bool { (**self).is_unfounded(uid) }
            fn hot_records(&self, kind: HotKind) -> HotRecords<'_> { (**self).hot_records(kind) }
            fn library(&self) -> &StoreLibrary { (**self).library() }
            fn records(&self) -> Self::Records<'_> { (**self).records() }
            fn duplicate_records(&self) -> usize { (**self).duplicate_records() }
            fn record_set_digest(&self) -> [u8; 32] { (**self).record_set_digest() }
            fn state_hash(&self) -> [u8; 32] { (**self).state_hash() }

            fn relations_of_kind(&self, kind: &RelKind) -> Vec<&Relation> {
                (**self).relations_of_kind(kind)
            }
            fn has_relation(&self, kind: &RelKind, from: &Uid, to: &Uid) -> bool {
                (**self).has_relation(kind, from, to)
            }
            fn is_live_rebuttal(&self, rel: &Relation) -> bool { (**self).is_live_rebuttal(rel) }
            fn rebuttals_of(&self, uid: &Uid) -> Vec<Uid> { (**self).rebuttals_of(uid) }
            fn supports_of(&self, uid: &Uid, edges: &EdgeSet) -> Vec<Uid> {
                (**self).supports_of(uid, edges)
            }
            fn is_resolved(&self, target: &ResolutionTarget) -> bool {
                (**self).is_resolved(target)
            }
            fn contention_status(&self, c: &Contention) -> ContentionStatus {
                (**self).contention_status(c)
            }
            fn open_contentions(&self) -> Vec<&Contention> { (**self).open_contentions() }
            fn commitment_of(&self, uid: &Uid) -> Option<Commitment> {
                (**self).commitment_of(uid)
            }
            fn units_at_commitment(&self, level: Commitment) -> Vec<Uid> {
                (**self).units_at_commitment(level)
            }
            fn matching_prefix(&self, prefix: &UidPrefix) -> Vec<Uid> {
                (**self).matching_prefix(prefix)
            }
            fn resolve_prefix(&self, prefix: &UidPrefix) -> Result<Uid, IntegrityError> {
                (**self).resolve_prefix(prefix)
            }
            fn report_dangling(&self, report: &mut Report) { (**self).report_dangling(report) }
            fn attestations_of(&self, uid: &Uid) -> Vec<Attestation> {
                (**self).attestations_of(uid)
            }
            fn attested_by(&self, uid: &Uid) -> BTreeSet<AgentId> { (**self).attested_by(uid) }
            fn agreement(&self, uid: &Uid, n: usize) -> bool { (**self).agreement(uid, n) }
            fn hops(&self) -> BTreeSet<u32> { (**self).hops() }
            fn latest_hop(&self) -> Option<u32> { (**self).latest_hop() }
        }
    )*};
}

delegate!(&S, &mut S, Box<S>, Rc<S>, Arc<S>);

impl<S: StoreWrite + ?Sized> StoreWrite for &mut S {
    fn append(&mut self, records: &[Record]) -> Result<AppendReport, Error> {
        (**self).append(records)
    }
}

impl<S: StoreWrite + ?Sized> StoreWrite for Box<S> {
    fn append(&mut self, records: &[Record]) -> Result<AppendReport, Error> {
        (**self).append(records)
    }
}

#[cfg(test)]
mod tests {
    //! The provided bodies — the ones a paged store, a union and an overlay will run, since
    //! `Store` overrides every one of them — answer as `Store`'s inherent methods do.

    use super::*;

    /// A store that forwards the required methods only, so every provided method runs its
    /// default body.
    struct Bare<'s>(&'s Store);

    impl sealed::Sealed for Bare<'_> {
        fn owned(&self) -> Cow<'_, Store> {
            Cow::Borrowed(self.0)
        }
        fn interp(&self) -> &Interpretive {
            sealed::Sealed::interp(self.0)
        }
        fn pending_attestations(&self) -> &[Attestation] {
            sealed::Sealed::pending_attestations(self.0)
        }
        fn has_record(&self, h: &[u8; 32]) -> bool {
            sealed::Sealed::has_record(self.0, h)
        }
        fn record_hashes(&self) -> Box<dyn Iterator<Item = &[u8; 32]> + '_> {
            sealed::Sealed::record_hashes(self.0)
        }
    }

    impl<'s> StoreRead for Bare<'s> {
        type UnitRef<'a>
            = &'a Unit
        where
            Self: 'a;
        type Units<'a>
            = btree_map::Iter<'a, Uid, Unit>
        where
            Self: 'a;
        type Records<'a>
            = <Store as StoreRead>::Records<'a>
        where
            Self: 'a;

        fn get(&self, uid: &Uid) -> Option<&Unit> {
            self.0.get(uid)
        }
        fn contains_uid(&self, uid: &Uid) -> bool {
            self.0.contains_uid(uid)
        }
        fn units(&self) -> Self::Units<'_> {
            self.0.units.iter()
        }
        fn unit_count(&self) -> usize {
            StoreRead::unit_count(self.0)
        }
        fn units_in_range(&self, lo: &Uid, hi: &Uid) -> Vec<Uid> {
            StoreRead::units_in_range(self.0, lo, hi)
        }
        fn observed_at(&self, uid: &Uid) -> Option<u64> {
            self.0.observed_at(uid)
        }
        fn hop_of(&self, uid: &Uid) -> Option<u32> {
            self.0.hop_of(uid)
        }
        fn units_with_source_prefix(&self, prefix: &str) -> Vec<Uid> {
            self.0.units_with_source_prefix(prefix)
        }
        fn units_in_observed_order(&self) -> Vec<Uid> {
            self.0.units_in_observed_order()
        }
        fn adjacency(&self) -> &Adjacency {
            self.0.adjacency()
        }
        fn relations(&self) -> Relations<'_> {
            StoreRead::relations(self.0)
        }
        fn relation_by_id(&self, rid: &Uid) -> Option<&Relation> {
            self.0.relation_by_id(rid)
        }
        fn relation_by_key(&self, kind: &RelKind, from: &Uid, to: &Uid) -> Option<&Relation> {
            StoreRead::relation_by_key(self.0, kind, from, to)
        }
        fn threads(&self) -> Threads<'_> {
            StoreRead::threads(self.0)
        }
        fn views(&self) -> Views<'_> {
            StoreRead::views(self.0)
        }
        fn contentions(&self) -> &[Contention] {
            self.0.contentions()
        }
        fn withdrawals(&self) -> Withdrawals<'_> {
            StoreRead::withdrawals(self.0)
        }
        fn resolutions(&self) -> Resolutions<'_> {
            StoreRead::resolutions(self.0)
        }
        fn commits(&self) -> Commits<'_> {
            StoreRead::commits(self.0)
        }
        fn commits_of(&self, uid: &Uid) -> &BTreeSet<Commit> {
            self.0.commits_of(uid)
        }
        fn is_withdrawn(&self, rel: &Relation) -> bool {
            self.0.is_withdrawn(rel)
        }
        fn is_unfounded(&self, uid: &Uid) -> bool {
            self.0.is_unfounded(uid)
        }
        fn hot_records(&self, kind: HotKind) -> HotRecords<'_> {
            self.0.hot_records(kind)
        }
        fn library(&self) -> &StoreLibrary {
            self.0.library()
        }
        fn records(&self) -> Self::Records<'_> {
            StoreRead::records(self.0)
        }
        fn duplicate_records(&self) -> usize {
            self.0.duplicate_records()
        }
        fn record_set_digest(&self) -> [u8; 32] {
            self.0.record_set_digest()
        }
        fn state_hash(&self) -> [u8; 32] {
            self.0.state_hash()
        }
    }

    fn corpus() -> Vec<Record> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus");
        let mut paths: Vec<_> = std::fs::read_dir(root)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "smy"))
            .collect();
        paths.sort();
        paths
            .iter()
            .filter_map(|p| {
                smysl_core::surface::parse_surface(&std::fs::read_to_string(p).unwrap()).ok()
            })
            .flat_map(|o| o.records)
            .collect()
    }

    #[test]
    fn the_provided_bodies_answer_as_the_inherent_methods() {
        let s = Store::from_records(corpus());
        assert!(s.relations().count() > 10, "a corpus with edges");
        let b = Bare(&s);
        let g = s.adjacency();
        let uids: Vec<Uid> = (0..g.len() as u32)
            .filter_map(|n| g.uid(n).copied())
            .collect();
        let all = EdgeSet::all();
        for u in &uids {
            assert_eq!(b.rebuttals_of(u), s.rebuttals_of(u));
            assert_eq!(b.supports_of(u, &all), s.supports_of(u, &all));
            assert_eq!(b.commitment_of(u), s.commitment_of(u));
            assert_eq!(
                b.attestations_of(u),
                s.attestations_of(u).iter().cloned().collect::<Vec<_>>()
            );
            assert_eq!(
                b.attested_by(u),
                s.attested_by(u).into_iter().cloned().collect()
            );
            assert_eq!(b.agreement(u, 1), s.agreement(u, 1));
            let text = u.to_string();
            let p = UidPrefix::parse(&text[..29]).unwrap();
            assert_eq!(b.matching_prefix(&p), s.matching_prefix(&p));
            assert_eq!(
                format!("{:?}", b.resolve_prefix(&p)),
                format!("{:?}", s.resolve_prefix(&p))
            );
        }
        for r in s.relations() {
            assert_eq!(b.is_live_rebuttal(r), s.is_live_rebuttal(r));
            assert_eq!(
                b.has_relation(&r.kind, &r.from, &r.to),
                s.has_relation(&r.kind, &r.from, &r.to)
            );
        }
        for kind in [RelKind::Rebuts, RelKind::Backs, RelKind::Supersedes] {
            assert_eq!(b.relations_of_kind(&kind), s.relations_of_kind(&kind));
        }
        for c in s.contentions() {
            assert_eq!(b.contention_status(c), s.contention_status(c));
        }
        assert_eq!(
            b.open_contentions(),
            s.open_contentions().collect::<Vec<_>>()
        );
        for level in [Commitment::Floated, Commitment::Committed] {
            assert_eq!(b.units_at_commitment(level), s.units_at_commitment(level));
        }
        assert_eq!(b.hops(), s.hops());
        assert_eq!(b.latest_hop(), s.latest_hop());
        let (mut x, mut y) = (Report::new(), Report::new());
        b.report_dangling(&mut x);
        s.report_dangling(&mut y);
        assert_eq!(
            format!("{:?}", x.diagnostics),
            format!("{:?}", y.diagnostics)
        );
    }
}
