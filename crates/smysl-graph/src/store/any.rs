//! A store whose backend is chosen at run time (SMYSL-2.8 §3.2, §4.4).
//!
//! `StoreRead` has generic associated types, so it is not object-safe and there is no
//! `dyn StoreRead`. The CLI picks a backend from a flag, and an enum that answers by `match` is
//! all that needs: it is a `StoreRead` like any other, so every operation takes it.
//!
//! **In memory only, today.** The redb backend arrives in TX-P13b with a second variant. The
//! associated types are newtypes over a private enum from the start, so that variant changes no
//! type a caller can name, and the enum is `#[non_exhaustive]` so a `match` outside this crate
//! already has to say what it does with a backend it does not know.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::ops::Deref;

use smysl_core::{Attestation, Commit, Contention, Error, Record, RelKind, Relation, Uid, Unit};

use super::interp::Interpretive;
use super::library::StoreLibrary;
use super::read::sealed;
use super::{
    AppendReport, Commits, HotKind, HotRecords, Relations, Resolutions, Store, StoreRead,
    StoreWrite, Threads, Views, Withdrawals,
};
use crate::adjacency::Adjacency;

/// A store of whichever backend was opened.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum AnyStore {
    /// The in-memory store, from surface text or a CBOR log.
    Memory(Store),
}

impl AnyStore {
    /// The in-memory store, when that is the backend.
    pub fn as_memory(&self) -> Option<&Store> {
        match self {
            AnyStore::Memory(s) => Some(s),
        }
    }

    /// The in-memory store, mutably, when that is the backend.
    pub fn as_memory_mut(&mut self) -> Option<&mut Store> {
        match self {
            AnyStore::Memory(s) => Some(s),
        }
    }

    /// An in-memory store holding the same records: this one, or one read from the backend.
    pub fn into_memory(self) -> Store {
        match self {
            AnyStore::Memory(s) => s,
        }
    }
}

impl From<Store> for AnyStore {
    fn from(s: Store) -> AnyStore {
        AnyStore::Memory(s)
    }
}

/// A unit read from an [`AnyStore`].
#[derive(Debug, Clone)]
pub struct AnyUnit<'a>(AnyUnitInner<'a>);

#[derive(Debug, Clone)]
enum AnyUnitInner<'a> {
    Memory(&'a Unit),
}

impl Deref for AnyUnit<'_> {
    type Target = Unit;
    fn deref(&self) -> &Unit {
        match &self.0 {
            AnyUnitInner::Memory(u) => u,
        }
    }
}

/// An [`AnyStore`]'s units, in ascending uid order.
pub struct AnyUnits<'a>(AnyUnitsInner<'a>);

enum AnyUnitsInner<'a> {
    Memory(<Store as StoreRead>::Units<'a>),
}

impl<'a> Iterator for AnyUnits<'a> {
    type Item = (&'a Uid, AnyUnit<'a>);
    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.0 {
            AnyUnitsInner::Memory(it) => it
                .next()
                .map(|(u, x)| (u, AnyUnit(AnyUnitInner::Memory(x)))),
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.0 {
            AnyUnitsInner::Memory(it) => it.size_hint(),
        }
    }
}

/// An [`AnyStore`]'s log, in log order.
pub struct AnyRecords<'a>(AnyRecordsInner<'a>);

enum AnyRecordsInner<'a> {
    Memory(<Store as StoreRead>::Records<'a>),
}

impl<'a> Iterator for AnyRecords<'a> {
    type Item = Cow<'a, Record>;
    fn next(&mut self) -> Option<Cow<'a, Record>> {
        match &mut self.0 {
            AnyRecordsInner::Memory(it) => it.next(),
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.0 {
            AnyRecordsInner::Memory(it) => it.size_hint(),
        }
    }
}

/// Answer by the backend's own method.
macro_rules! by_backend {
    ($self:ident, $s:ident => $e:expr) => {
        match $self {
            AnyStore::Memory($s) => $e,
        }
    };
}

impl sealed::Sealed for AnyStore {
    fn owned(&self) -> Cow<'_, Store> {
        by_backend!(self, s => sealed::Sealed::owned(s))
    }
    fn interp(&self) -> &Interpretive {
        by_backend!(self, s => sealed::Sealed::interp(s))
    }
    fn pending_attestations(&self) -> &[Attestation] {
        by_backend!(self, s => sealed::Sealed::pending_attestations(s))
    }
    fn has_record(&self, h: &[u8; 32]) -> bool {
        by_backend!(self, s => sealed::Sealed::has_record(s, h))
    }
    fn record_hashes(&self) -> Box<dyn Iterator<Item = &[u8; 32]> + '_> {
        by_backend!(self, s => sealed::Sealed::record_hashes(s))
    }
}

impl StoreRead for AnyStore {
    type UnitRef<'a> = AnyUnit<'a>;
    type Units<'a> = AnyUnits<'a>;
    type Records<'a> = AnyRecords<'a>;

    fn get(&self, uid: &Uid) -> Option<AnyUnit<'_>> {
        by_backend!(self, s => StoreRead::get(s, uid).map(|u| AnyUnit(AnyUnitInner::Memory(u))))
    }
    fn contains_uid(&self, uid: &Uid) -> bool {
        by_backend!(self, s => StoreRead::contains_uid(s, uid))
    }
    fn units(&self) -> AnyUnits<'_> {
        by_backend!(self, s => AnyUnits(AnyUnitsInner::Memory(StoreRead::units(s))))
    }
    fn unit_count(&self) -> usize {
        by_backend!(self, s => StoreRead::unit_count(s))
    }
    fn units_in_range(&self, lo: &Uid, hi: &Uid) -> Vec<Uid> {
        by_backend!(self, s => StoreRead::units_in_range(s, lo, hi))
    }
    fn observed_at(&self, uid: &Uid) -> Option<u64> {
        by_backend!(self, s => StoreRead::observed_at(s, uid))
    }
    fn hop_of(&self, uid: &Uid) -> Option<u32> {
        by_backend!(self, s => StoreRead::hop_of(s, uid))
    }
    fn units_with_source_prefix(&self, prefix: &str) -> Vec<Uid> {
        by_backend!(self, s => StoreRead::units_with_source_prefix(s, prefix))
    }
    fn units_in_observed_order(&self) -> Vec<Uid> {
        by_backend!(self, s => StoreRead::units_in_observed_order(s))
    }
    fn adjacency(&self) -> &Adjacency {
        by_backend!(self, s => StoreRead::adjacency(s))
    }
    fn relations(&self) -> Relations<'_> {
        by_backend!(self, s => StoreRead::relations(s))
    }
    fn relation_by_id(&self, rid: &Uid) -> Option<&Relation> {
        by_backend!(self, s => StoreRead::relation_by_id(s, rid))
    }
    fn relation_by_key(&self, kind: &RelKind, from: &Uid, to: &Uid) -> Option<&Relation> {
        by_backend!(self, s => StoreRead::relation_by_key(s, kind, from, to))
    }
    fn threads(&self) -> Threads<'_> {
        by_backend!(self, s => StoreRead::threads(s))
    }
    fn views(&self) -> Views<'_> {
        by_backend!(self, s => StoreRead::views(s))
    }
    fn contentions(&self) -> &[Contention] {
        by_backend!(self, s => StoreRead::contentions(s))
    }
    fn withdrawals(&self) -> Withdrawals<'_> {
        by_backend!(self, s => StoreRead::withdrawals(s))
    }
    fn resolutions(&self) -> Resolutions<'_> {
        by_backend!(self, s => StoreRead::resolutions(s))
    }
    fn commits(&self) -> Commits<'_> {
        by_backend!(self, s => StoreRead::commits(s))
    }
    fn commits_of(&self, uid: &Uid) -> &BTreeSet<Commit> {
        by_backend!(self, s => StoreRead::commits_of(s, uid))
    }
    fn is_withdrawn(&self, rel: &Relation) -> bool {
        by_backend!(self, s => StoreRead::is_withdrawn(s, rel))
    }
    fn is_unfounded(&self, uid: &Uid) -> bool {
        by_backend!(self, s => StoreRead::is_unfounded(s, uid))
    }
    fn hot_records(&self, kind: HotKind) -> HotRecords<'_> {
        by_backend!(self, s => StoreRead::hot_records(s, kind))
    }
    fn library(&self) -> &StoreLibrary {
        by_backend!(self, s => StoreRead::library(s))
    }
    fn records(&self) -> AnyRecords<'_> {
        by_backend!(self, s => AnyRecords(AnyRecordsInner::Memory(StoreRead::records(s))))
    }
    fn duplicate_records(&self) -> usize {
        by_backend!(self, s => StoreRead::duplicate_records(s))
    }
    fn record_set_digest(&self) -> [u8; 32] {
        by_backend!(self, s => StoreRead::record_set_digest(s))
    }
    fn state_hash(&self) -> [u8; 32] {
        by_backend!(self, s => StoreRead::state_hash(s))
    }

    // The provided methods, answered by the backend's: for the in-memory store that is its
    // inherent method, which is the one the defaults are tested against.
    fn relations_of_kind(&self, kind: &RelKind) -> Vec<&Relation> {
        by_backend!(self, s => StoreRead::relations_of_kind(s, kind))
    }
    fn has_relation(&self, kind: &RelKind, from: &Uid, to: &Uid) -> bool {
        by_backend!(self, s => StoreRead::has_relation(s, kind, from, to))
    }
    fn is_live_rebuttal(&self, rel: &Relation) -> bool {
        by_backend!(self, s => StoreRead::is_live_rebuttal(s, rel))
    }
    fn rebuttals_of(&self, uid: &Uid) -> Vec<Uid> {
        by_backend!(self, s => StoreRead::rebuttals_of(s, uid))
    }
    fn supports_of(&self, uid: &Uid, edges: &crate::EdgeSet) -> Vec<Uid> {
        by_backend!(self, s => StoreRead::supports_of(s, uid, edges))
    }
}

impl StoreWrite for AnyStore {
    fn append(&mut self, records: &[Record]) -> Result<AppendReport, Error> {
        by_backend!(self, s => StoreWrite::append(s, records))
    }
}
