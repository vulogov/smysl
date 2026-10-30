//! A store whose backend is chosen at run time (SMYSL-2.8 §3.2, §4.4).
//!
//! `StoreRead` has generic associated types, so it is not object-safe and there is no
//! `dyn StoreRead`. The CLI picks a backend from a flag, and an enum that answers by `match` is
//! all that needs: it is a `StoreRead` like any other, so every operation takes it.
//!
//! **Two backends**: the in-memory store, and with feature `store-redb` the unit store on disk
//! (TX-P13b step 1). The associated types are newtypes over a private enum, so the second
//! variant changed no type a caller can name, and the enum is `#[non_exhaustive]` so a `match`
//! outside this crate already says what it does with a backend it does not know.
//!
//! **And a union of either** (TX-P13c step 1): `--shard` reads some of a library's shards as
//! one store, each shard of whichever backend was asked for.

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
#[cfg(feature = "store-redb")]
use super::{ShardId, UidShards};
use super::{Union, UnionRecords, UnionUnit, UnionUnits};
use crate::adjacency::Adjacency;

/// A store of whichever backend was opened.
///
/// Not boxed, although the in-memory store is two and a half kilobytes of map headers and the
/// unit store a pointer: an `AnyStore` is made once per command and moved a handful of times,
/// and boxing the common case would add an indirection to every call for a copy nobody makes.
#[derive(Debug, Clone)]
#[non_exhaustive]
#[allow(clippy::large_enum_variant)]
pub enum AnyStore {
    /// The in-memory store, from surface text or a CBOR log.
    Memory(Store),
    /// The unit store on disk, whose cores stay in the log until asked for.
    #[cfg(feature = "store-redb")]
    Redb(super::redb::RedbStore),
    /// Shards of either backend, read as one store (SMYSL-2.8 §3.7). Read-only.
    Shards(Box<Union<AnyStore>>),
}

impl AnyStore {
    /// The in-memory store, when that is the backend.
    pub fn as_memory(&self) -> Option<&Store> {
        match self {
            AnyStore::Memory(s) => Some(s),
            _ => None,
        }
    }

    /// The in-memory store, mutably, when that is the backend.
    pub fn as_memory_mut(&mut self) -> Option<&mut Store> {
        match self {
            AnyStore::Memory(s) => Some(s),
            _ => None,
        }
    }

    /// An in-memory store holding the same records: this one, or one read from the backend.
    pub fn into_memory(self) -> Store {
        match self {
            AnyStore::Memory(s) => s,
            #[cfg(feature = "store-redb")]
            AnyStore::Redb(s) => sealed::Sealed::owned(&s).into_owned(),
            AnyStore::Shards(u) => sealed::Sealed::owned(&*u).into_owned(),
        }
    }

    /// A union of these shards (`--shard`), consulting `index` for what waits on the others.
    pub fn shards(
        shards: impl IntoIterator<Item = (super::ShardId, AnyStore)>,
        index: Option<std::sync::Arc<super::UidShards>>,
    ) -> AnyStore {
        let u = Union::new(shards);
        AnyStore::Shards(Box::new(match index {
            Some(ix) => u.with_index(ix),
            None => u,
        }))
    }

    /// The union, when that is what this is.
    pub fn as_union(&self) -> Option<&Union<AnyStore>> {
        match self {
            AnyStore::Shards(u) => Some(u),
            _ => None,
        }
    }

    /// Records in the log, duplicates included, as `Store::len`.
    pub fn len(&self) -> usize {
        match self {
            AnyStore::Memory(s) => s.len(),
            #[cfg(feature = "store-redb")]
            AnyStore::Redb(s) => s.len(),
            AnyStore::Shards(u) => StoreRead::records(&**u).count(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The unit store for a log, or the log itself when another process holds the store
    /// (SMYSL-2.8 §3.8, OQ-67).
    ///
    /// redb 2.6 locks a store's file for every open, so a second process cannot read one the
    /// first has open. This waits up to `opts.reader_wait_ms` for it, then opens the log into
    /// memory with `SMY-W504`: the same answers, at the cost of the in-memory load — slower,
    /// correct, and never blocked for longer than the wait.
    #[cfg(feature = "store-redb")]
    pub fn open_redb(
        log: impl AsRef<std::path::Path>,
        opts: super::redb::RedbOptions,
    ) -> Result<(AnyStore, super::OpenReport), Error> {
        let log = log.as_ref();
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(opts.reader_wait_ms);
        loop {
            match super::redb::RedbStore::open(log, opts.clone()) {
                Ok((s, report)) => return Ok((AnyStore::Redb(s), report)),
                Err(e) if super::redb::is_held(&e) && std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(e) if super::redb::is_held(&e) => {
                    let (s, mut report) = Store::open_with(log, super::StoreOptions::default())?;
                    report.report.push(
                        smysl_core::diag::Diagnostic::new(smysl_core::diag::Code::W504)
                            .with_message(format!(
                                "{e}; read the log into memory after waiting {} ms",
                                opts.reader_wait_ms
                            )),
                    );
                    return Ok((AnyStore::Memory(s), report));
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl AnyStore {
    /// One unit store over a library's shard logs, or some of them (TX-P13c step 3): what
    /// [`super::redb::RedbStore::open_shards`] opens, waiting for one another process holds as
    /// [`AnyStore::open_redb`] waits. After the wait, the logs read are opened into memory as
    /// a union, with `index` saying where a reference into another waits (`SMY-W504`).
    #[cfg(feature = "store-redb")]
    pub fn open_redb_shards(
        store: &std::path::Path,
        shards: &[(ShardId, std::path::PathBuf)],
        only: Option<&[ShardId]>,
        index: Option<std::sync::Arc<UidShards>>,
        opts: super::redb::RedbOptions,
    ) -> Result<(AnyStore, super::OpenReport), Error> {
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(opts.reader_wait_ms);
        loop {
            match super::redb::RedbStore::open_shards(store, shards, only, opts.clone()) {
                Ok((s, report)) => return Ok((AnyStore::Redb(s), report)),
                Err(e) if super::redb::is_held(&e) && std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(e) if super::redb::is_held(&e) => {
                    let mut stores = Vec::new();
                    for (name, log) in shards {
                        if only.is_some_and(|o| !o.contains(name)) {
                            continue;
                        }
                        let bytes = match std::fs::read(log) {
                            Ok(b) => b,
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                            Err(e) => return Err(Error::Io(e)),
                        };
                        let (records, _) = smysl_core::from_cbor_seq(&bytes)?;
                        stores.push((name.clone(), AnyStore::from(Store::from_records(records))));
                    }
                    let mut report = super::OpenReport::default();
                    report.report.push(
                        smysl_core::diag::Diagnostic::new(smysl_core::diag::Code::W504)
                            .with_message(format!(
                                "{e}; read the shards into memory after waiting {} ms",
                                opts.reader_wait_ms
                            )),
                    );
                    return Ok((AnyStore::shards(stores, index), report));
                }
                Err(e) => return Err(e),
            }
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
    #[cfg(feature = "store-redb")]
    Redb(std::sync::Arc<Unit>),
    Union(Box<UnionUnit<'a, AnyUnit<'a>>>),
}

impl Deref for AnyUnit<'_> {
    type Target = Unit;
    fn deref(&self) -> &Unit {
        match &self.0 {
            AnyUnitInner::Memory(u) => u,
            #[cfg(feature = "store-redb")]
            AnyUnitInner::Redb(u) => u,
            AnyUnitInner::Union(u) => u,
        }
    }
}

/// An [`AnyStore`]'s units, in ascending uid order.
pub struct AnyUnits<'a>(AnyUnitsInner<'a>);

enum AnyUnitsInner<'a> {
    Memory(<Store as StoreRead>::Units<'a>),
    #[cfg(feature = "store-redb")]
    Redb(super::redb::RedbUnits<'a>),
    Union(Box<UnionUnits<'a, AnyStore>>),
}

impl<'a> Iterator for AnyUnits<'a> {
    type Item = (&'a Uid, AnyUnit<'a>);
    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.0 {
            AnyUnitsInner::Memory(it) => it
                .next()
                .map(|(u, x)| (u, AnyUnit(AnyUnitInner::Memory(x)))),
            #[cfg(feature = "store-redb")]
            AnyUnitsInner::Redb(it) => it.next().map(|(u, x)| (u, AnyUnit(AnyUnitInner::Redb(x)))),
            AnyUnitsInner::Union(it) => it
                .next()
                .map(|(u, x)| (u, AnyUnit(AnyUnitInner::Union(Box::new(x))))),
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.0 {
            AnyUnitsInner::Memory(it) => it.size_hint(),
            #[cfg(feature = "store-redb")]
            AnyUnitsInner::Redb(it) => it.size_hint(),
            AnyUnitsInner::Union(it) => it.size_hint(),
        }
    }
}

/// An [`AnyStore`]'s log, in log order.
pub struct AnyRecords<'a>(AnyRecordsInner<'a>);

enum AnyRecordsInner<'a> {
    Memory(<Store as StoreRead>::Records<'a>),
    #[cfg(feature = "store-redb")]
    Redb(super::redb::RedbRecords<'a>),
    Union(Box<UnionRecords<'a, AnyStore>>),
}

impl<'a> Iterator for AnyRecords<'a> {
    type Item = Cow<'a, Record>;
    fn next(&mut self) -> Option<Cow<'a, Record>> {
        match &mut self.0 {
            AnyRecordsInner::Memory(it) => it.next(),
            #[cfg(feature = "store-redb")]
            AnyRecordsInner::Redb(it) => it.next(),
            AnyRecordsInner::Union(it) => it.next(),
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.0 {
            AnyRecordsInner::Memory(it) => it.size_hint(),
            #[cfg(feature = "store-redb")]
            AnyRecordsInner::Redb(it) => it.size_hint(),
            AnyRecordsInner::Union(it) => it.size_hint(),
        }
    }
}

/// Answer by the backend's own method.
macro_rules! by_backend {
    ($self:ident, $s:ident => $e:expr) => {
        match $self {
            AnyStore::Memory($s) => $e,
            #[cfg(feature = "store-redb")]
            AnyStore::Redb($s) => $e,
            AnyStore::Shards(u) => {
                let $s = &**u;
                $e
            }
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
        match self {
            AnyStore::Memory(s) => StoreRead::get(s, uid).map(|u| AnyUnit(AnyUnitInner::Memory(u))),
            #[cfg(feature = "store-redb")]
            AnyStore::Redb(s) => StoreRead::get(s, uid).map(|u| AnyUnit(AnyUnitInner::Redb(u))),
            AnyStore::Shards(s) => {
                StoreRead::get(&**s, uid).map(|u| AnyUnit(AnyUnitInner::Union(Box::new(u))))
            }
        }
    }
    fn contains_uid(&self, uid: &Uid) -> bool {
        by_backend!(self, s => StoreRead::contains_uid(s, uid))
    }
    fn units(&self) -> AnyUnits<'_> {
        match self {
            AnyStore::Memory(s) => AnyUnits(AnyUnitsInner::Memory(StoreRead::units(s))),
            #[cfg(feature = "store-redb")]
            AnyStore::Redb(s) => AnyUnits(AnyUnitsInner::Redb(StoreRead::units(s))),
            AnyStore::Shards(s) => AnyUnits(AnyUnitsInner::Union(Box::new(StoreRead::units(&**s)))),
        }
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
    fn attested_units(&self) -> Vec<Uid> {
        by_backend!(self, s => StoreRead::attested_units(s))
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
        match self {
            AnyStore::Memory(s) => AnyRecords(AnyRecordsInner::Memory(StoreRead::records(s))),
            #[cfg(feature = "store-redb")]
            AnyStore::Redb(s) => AnyRecords(AnyRecordsInner::Redb(StoreRead::records(s))),
            AnyStore::Shards(s) => {
                AnyRecords(AnyRecordsInner::Union(Box::new(StoreRead::records(&**s))))
            }
        }
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
    fn report_dangling(&self, report: &mut smysl_core::diag::Report) {
        by_backend!(self, s => StoreRead::report_dangling(s, report))
    }
}

/// Appends to whichever backend the store is.
///
/// The unit store is written as §3.8 says the CLI writes it: its shard opened for writing for
/// the one append — the lock taken, the log and then the store written, the lock released —
/// and the store replaced by a snapshot that includes what was appended. A second writer
/// meanwhile is refused with `SMY-E445`, and nothing holds the lock between appends.
impl StoreWrite for AnyStore {
    fn append(&mut self, records: &[Record]) -> Result<AppendReport, Error> {
        match self {
            AnyStore::Memory(s) => StoreWrite::append(s, records),
            #[cfg(feature = "store-redb")]
            AnyStore::Redb(s) if s.is_shards() => Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "a unit store of several shards is read-only: write through the library",
            ))),
            #[cfg(feature = "store-redb")]
            AnyStore::Redb(s) => {
                let (mut shard, _) =
                    super::redb::RedbShard::open_rw(s.log_path(), s.options().clone())?;
                let report = shard.append(records)?;
                *s = shard.reader()?;
                Ok(report)
            }
            // Which shard a record belongs in is the library's question (SMYSL-2.4's routing),
            // and a union cannot answer it.
            AnyStore::Shards(_) => Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "a union of shards is read-only: write through the library",
            ))),
        }
    }
}
