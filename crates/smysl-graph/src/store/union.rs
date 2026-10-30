//! A store seen together with records it does not hold (SMYSL-2.8 §3.7).
//!
//! [`Overlay`] is one base store and an in-memory batch, answering every [`StoreRead`] question
//! as `Store::from_records(base ++ batch)` would — without that copy. `smysl_ingest::stage`
//! checks a batch against the union of store and batch, and until TX-P13a step 10 it built the
//! union by copying every record of the store into a new one: the batch's cost was the store's.
//!
//! **What is copied, and what is read through.** The batch becomes a small [`Store`] of its
//! own (less any record the base already holds). The interpretive layer and the library are the
//! base's, cloned, with the batch folded in by the same `absorb_hot` a store uses — proportional
//! to judgements, not to text. Units are read through: a uid the batch does not touch answers
//! from the base, and a uid it does (a new unit, or a base unit gaining an attestation) answers
//! from a small map of merged units. The adjacency and the `unfounded` set are derived lazily
//! over the union, as a store derives its own.
//!
//! **One assumption about the base**: that it holds no record twice, as anything built by
//! `from_records` or `append` does. A base opened from a log with repeats keeps them in
//! `records()` and in `duplicate_records`, where the store it stands for would not.

use std::borrow::Cow;
use std::collections::btree_map;
use std::collections::{BTreeMap, BTreeSet};
use std::iter::Peekable;
use std::ops::Deref;
use std::sync::OnceLock;

use smysl_core::{
    canonical_uid, Attestation, Commit, Contention, Record, RelKind, Relation, Status, Uid, Unit,
};

use super::interp::Interpretive;
use super::library::StoreLibrary;
use super::read::{empty_commits, sealed};
use super::{
    digest_of_hashes, state_hash_of, Commits, HotKind, HotRecords, Relations, Resolutions, Store,
    StoreRead, Threads, Views, Withdrawals,
};
use crate::adjacency::Adjacency;

/// A base store and a batch, read as their union.
///
/// Read-only: there is no [`super::StoreWrite`] for it, because appending to a view of two
/// things would have to choose which of them changes.
pub struct Overlay<'s, S: StoreRead + ?Sized> {
    base: &'s S,
    /// The batch, less what the base already holds.
    batch: Store,
    /// Units whose answer differs from the base's: every unit of the batch, and every base unit
    /// a batch attestation lands on, each merged with whatever the other side holds of it.
    units: BTreeMap<Uid, Unit>,
    /// How many of `units` the base does not hold.
    added: usize,
    interp: Interpretive,
    library: StoreLibrary,
    /// Attestations whose subject is in neither.
    pending: Vec<Attestation>,
    derived: OnceLock<Derived>,
}

struct Derived {
    adjacency: Adjacency,
    unfounded: BTreeSet<Uid>,
}

impl<S: StoreRead + ?Sized> std::fmt::Debug for Overlay<'_, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Overlay")
            .field("batch_records", &self.batch.len())
            .field("units", &self.units.len())
            .field("added", &self.added)
            .finish_non_exhaustive()
    }
}

impl<'s, S: StoreRead + ?Sized> Overlay<'s, S> {
    /// `base`, seen with `batch` added. Nothing is written to either.
    pub fn new(base: &'s S, batch: Vec<Record>) -> Overlay<'s, S> {
        let fresh: Vec<Record> = batch
            .into_iter()
            .filter(|r| !sealed::Sealed::has_record(base, &Store::record_hash(r)))
            .collect();
        let batch = Store::from_records(fresh);

        // The interpretive layer: the base's, then the batch's records folded in by the same
        // code a store absorbs them with. Relations come from the batch *store*, which has
        // already attached the batch's own attestations to them; the raw records would not
        // carry those.
        let mut interp = sealed::Sealed::interp(base).clone();
        for r in &batch.records {
            if !matches!(r, Record::Relation(_)) {
                interp.absorb_hot(r);
            }
        }
        for rel in batch.interp.relations.values() {
            interp.absorb_hot(&Record::Relation(rel.clone()));
        }

        let mut library = base.library().clone();
        for r in &batch.records {
            match r {
                Record::Manifest(m) => library.absorb_manifest(m),
                Record::Dating(d) => library.absorb_dating(d),
                Record::Redaction(x) => library.absorb_redaction(x),
                _ => {}
            }
        }
        for r in &batch.records {
            if let Record::Unit(u) = r {
                library.absorb_unit(canonical_uid(u), u.source.as_ref());
            }
        }

        // Units the batch holds, merged with the base's copy where there is one.
        let mut units: BTreeMap<Uid, Unit> = BTreeMap::new();
        let mut added = 0;
        for (uid, u) in &batch.units {
            let merged = match base.get(uid) {
                Some(b) => {
                    let mut m = (*b).clone();
                    m.attestations.extend(u.attestations.iter().cloned());
                    m
                }
                None => {
                    added += 1;
                    u.clone()
                }
            };
            units.insert(*uid, merged);
        }

        // Attestations either side was still waiting on: the other side may hold the subject.
        let mut pending = Vec::new();
        let waiting = batch
            .pending_attestations
            .iter()
            .chain(sealed::Sealed::pending_attestations(base));
        for a in waiting {
            if let Some(u) = units.get_mut(&a.uid) {
                u.attestations.insert(a.clone());
            } else if let Some(b) = base.get(&a.uid) {
                let mut m = (*b).clone();
                m.attestations.insert(a.clone());
                units.insert(a.uid, m);
            } else if !interp.attach_to_relation(a) {
                pending.push(a.clone());
            }
        }

        Overlay {
            base,
            batch,
            units,
            added,
            interp,
            library,
            pending,
            derived: OnceLock::new(),
        }
    }

    /// The base this overlay reads through to.
    pub fn base(&self) -> &'s S {
        self.base
    }

    /// The batch's records the base did not already hold, in the order given.
    pub fn batch_records(&self) -> &[Record] {
        &self.batch.records
    }

    fn derived(&self) -> &Derived {
        self.derived.get_or_init(|| {
            // As `Store::derive`: withdrawn edges are kept and not followed, and `unfounded`
            // is only worth computing when something retracts.
            let relations: Vec<Relation> = self
                .interp
                .relations
                .values()
                .filter(|r| !self.interp.is_withdrawn(r))
                .cloned()
                .collect();
            let units: Vec<(Uid, OverlayUnit<'_, S::UnitRef<'_>>)> =
                self.units().map(|(u, x)| (*u, x)).collect();
            let adjacency = Adjacency::from_parts(&units, &relations);
            let mut unfounded = BTreeSet::new();
            if self
                .interp
                .relations
                .keys()
                .any(|(k, _, _)| k == RelKind::Retracts.as_str())
            {
                let eff =
                    crate::merge::effective_status(self, crate::merge::RetractionPolicy::Strict);
                unfounded = units
                    .iter()
                    .map(|(u, _)| *u)
                    .filter(|u| eff.get(u) == Some(Status::Unfounded))
                    .collect();
            }
            Derived {
                adjacency,
                unfounded,
            }
        })
    }

    fn observed(u: &Unit) -> Option<u64> {
        u.core.source.as_ref().and_then(|s| s.observed)
    }
}

/// A unit read from an [`Overlay`]: the base's own handle, or the overlay's merged copy.
#[derive(Debug)]
pub enum OverlayUnit<'a, R> {
    /// As the base holds it.
    Base(R),
    /// Merged with the batch.
    Own(&'a Unit),
}

impl<R: Clone> Clone for OverlayUnit<'_, R> {
    fn clone(&self) -> Self {
        match self {
            OverlayUnit::Base(r) => OverlayUnit::Base(r.clone()),
            OverlayUnit::Own(u) => OverlayUnit::Own(u),
        }
    }
}

impl<R: Deref<Target = Unit>> Deref for OverlayUnit<'_, R> {
    type Target = Unit;
    fn deref(&self) -> &Unit {
        match self {
            OverlayUnit::Base(r) => r,
            OverlayUnit::Own(u) => u,
        }
    }
}

/// An [`Overlay`]'s units, in ascending uid order: the base's and the batch's, merged.
pub struct OverlayUnits<'a, I: Iterator, R> {
    base: Peekable<I>,
    own: Peekable<btree_map::Iter<'a, Uid, Unit>>,
    _r: std::marker::PhantomData<R>,
}

impl<'a, I, R> Iterator for OverlayUnits<'a, I, R>
where
    I: Iterator<Item = (&'a Uid, R)>,
{
    type Item = (&'a Uid, OverlayUnit<'a, R>);
    fn next(&mut self) -> Option<Self::Item> {
        let take_own = match (self.base.peek(), self.own.peek()) {
            (None, None) => return None,
            (Some(_), None) => false,
            (None, Some(_)) => true,
            (Some((b, _)), Some((o, _))) => {
                if b == o {
                    // The overlay's copy is the merged one; the base's is passed over.
                    self.base.next();
                    true
                } else {
                    o < b
                }
            }
        };
        if take_own {
            self.own.next().map(|(u, x)| (u, OverlayUnit::Own(x)))
        } else {
            self.base.next().map(|(u, x)| (u, OverlayUnit::Base(x)))
        }
    }
}

/// An [`Overlay`]'s log: the base's records, then the batch's.
pub struct OverlayRecords<'a, I> {
    base: I,
    batch: std::slice::Iter<'a, Record>,
}

impl<'a, I: Iterator<Item = Cow<'a, Record>>> Iterator for OverlayRecords<'a, I> {
    type Item = Cow<'a, Record>;
    fn next(&mut self) -> Option<Cow<'a, Record>> {
        self.base
            .next()
            .or_else(|| self.batch.next().map(Cow::Borrowed))
    }
}

/// Two ascending, duplicate-free uid lists as one.
fn merged(a: Vec<Uid>, b: Vec<Uid>) -> Vec<Uid> {
    let mut out: Vec<Uid> = a.into_iter().chain(b).collect();
    out.sort_unstable();
    out.dedup();
    out
}

impl<S: StoreRead + ?Sized> sealed::Sealed for Overlay<'_, S> {
    fn owned(&self) -> Cow<'_, Store> {
        Cow::Owned(Store::from_records(
            self.records().map(Cow::into_owned).collect(),
        ))
    }
    fn interp(&self) -> &Interpretive {
        &self.interp
    }
    fn pending_attestations(&self) -> &[Attestation] {
        &self.pending
    }
    fn has_record(&self, h: &[u8; 32]) -> bool {
        sealed::Sealed::has_record(self.base, h) || self.batch.contains_record(h)
    }
    fn record_hashes(&self) -> Box<dyn Iterator<Item = &[u8; 32]> + '_> {
        // Disjoint by construction: the batch store holds only what the base did not.
        let mut all: Vec<&[u8; 32]> = sealed::Sealed::record_hashes(self.base)
            .chain(self.batch.record_hashes.iter())
            .collect();
        all.sort_unstable();
        Box::new(all.into_iter())
    }
}

impl<S: StoreRead + ?Sized> StoreRead for Overlay<'_, S> {
    type UnitRef<'a>
        = OverlayUnit<'a, S::UnitRef<'a>>
    where
        Self: 'a;
    type Units<'a>
        = OverlayUnits<'a, S::Units<'a>, S::UnitRef<'a>>
    where
        Self: 'a;
    type Records<'a>
        = OverlayRecords<'a, S::Records<'a>>
    where
        Self: 'a;

    fn get(&self, uid: &Uid) -> Option<Self::UnitRef<'_>> {
        match self.units.get(uid) {
            Some(u) => Some(OverlayUnit::Own(u)),
            None => self.base.get(uid).map(OverlayUnit::Base),
        }
    }
    fn contains_uid(&self, uid: &Uid) -> bool {
        self.units.contains_key(uid) || self.base.contains_uid(uid)
    }
    fn units(&self) -> Self::Units<'_> {
        OverlayUnits {
            base: self.base.units().peekable(),
            own: self.units.iter().peekable(),
            _r: std::marker::PhantomData,
        }
    }
    fn unit_count(&self) -> usize {
        self.base.unit_count() + self.added
    }
    fn units_in_range(&self, lo: &Uid, hi: &Uid) -> Vec<Uid> {
        if lo > hi {
            return Vec::new();
        }
        let own: Vec<Uid> = self.units.range(*lo..=*hi).map(|(u, _)| *u).collect();
        merged(self.base.units_in_range(lo, hi), own)
    }
    fn observed_at(&self, uid: &Uid) -> Option<u64> {
        match self.units.get(uid) {
            Some(u) => Self::observed(u),
            None => self.base.observed_at(uid),
        }
    }
    fn hop_of(&self, uid: &Uid) -> Option<u32> {
        match self.units.get(uid) {
            Some(u) => u.attestations.iter().map(|a| a.hop).max(),
            None => self.base.hop_of(uid),
        }
    }
    fn units_with_source_prefix(&self, prefix: &str) -> Vec<Uid> {
        let own: Vec<Uid> = self
            .units
            .iter()
            .filter(|(_, u)| {
                u.core
                    .source
                    .as_ref()
                    .is_some_and(|s| s.reference.starts_with(prefix))
            })
            .map(|(uid, _)| *uid)
            .collect();
        merged(self.base.units_with_source_prefix(prefix), own)
    }
    fn units_in_observed_order(&self) -> Vec<Uid> {
        // The batch's units are not moved in time by an attestation, so the base's order holds
        // for every uid it has, and the batch's new units are merged into it by the same key.
        let mut timed: Vec<(u64, Uid)> = self
            .base
            .units_in_observed_order()
            .into_iter()
            .filter_map(|u| self.base.observed_at(&u).map(|ms| (ms, u)))
            .collect();
        for (uid, u) in &self.units {
            if !self.base.contains_uid(uid) {
                if let Some(ms) = Self::observed(u) {
                    timed.push((ms, *uid));
                }
            }
        }
        timed.sort();
        timed.into_iter().map(|(_, u)| u).collect()
    }

    fn adjacency(&self) -> &Adjacency {
        &self.derived().adjacency
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
        &self.interp.contentions
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
        self.interp.is_withdrawn(rel)
    }
    fn is_unfounded(&self, uid: &Uid) -> bool {
        self.derived().unfounded.contains(uid)
    }
    fn hot_records(&self, kind: HotKind) -> HotRecords<'_> {
        let mut out = self.base.hot_records(kind);
        let mut tail = &mut out;
        while tail.then.is_some() {
            tail = tail.then.as_mut().expect("checked");
        }
        tail.then = Some(Box::new(self.batch.hot_records(kind)));
        out
    }
    fn library(&self) -> &StoreLibrary {
        &self.library
    }

    fn records(&self) -> Self::Records<'_> {
        OverlayRecords {
            base: self.base.records(),
            batch: self.batch.records.iter(),
        }
    }
    fn duplicate_records(&self) -> usize {
        self.base.duplicate_records()
    }
    fn record_set_digest(&self) -> [u8; 32] {
        digest_of_hashes(sealed::Sealed::record_hashes(self))
    }
    fn state_hash(&self) -> [u8; 32] {
        state_hash_of(self.units(), &self.interp)
    }
}
