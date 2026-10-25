//! The unit store on disk (SMYSL-2.8 §3.5; TX-P13b step 1), behind feature `store-redb`.
//!
//! [`RedbStore`] is a [`StoreRead`] that keeps the part of a store every operation reads and
//! leaves the rest where it already is. In memory: the uids in order, a 40-byte header a unit,
//! the interpretive layer, the library, and the adjacency once something asks for it. On disk:
//! the cores, which stay in the CBOR log and are read from it by offset, and the attestations
//! of units, which are in the store's own `attest` table. A core read once is kept or not by
//! the policy in `cache.rs`.
//!
//! **Index mode only.** The log is the authority, as it is for the sidecar: the store records
//! the length and hash of the log it describes, and an open that finds a different log rebuilds
//! (`SMY-W501`), one that finds a longer one folds in the tail, and deleting the `.redb` file
//! loses nothing. Primary mode, where the store holds the records and the log is not written,
//! is TX-P13d's.
//!
//! **One snapshot.** A store holds one read transaction for its life, so every answer it gives
//! is from one moment. It also holds the file: redb 2.6 takes an exclusive lock on every open,
//! read-only or not, so a second process that opens the same store finds it held.
//! [`crate::AnyStore::open_redb`] waits for it, then reads the log instead (`SMY-W504`) —
//! the answer to OQ-67 for as long as the workspace is on redb 2.6. Within one process, every
//! open of a store shares one database (`shared.rs`), so readers and the writer do not hold
//! each other off.
//!
//! **The writer** is [`RedbShard`] (TX-P13b step 2): the shard's lock file, then the log,
//! `fsync`ed, then one commit. Its readers are snapshots of it, [`RedbShard::reader`].
//!
//! **What answers cost.** Header questions — `contains_uid`, `observed_at`, `hop_of`, ranges,
//! counts — cost what they cost in memory. `get` costs one positioned read of the log and a
//! decode, unless the core is cached, plus a range of `attest` for a unit with attestations.
//! A scan of `units()` reads every core it has not cached, which is the trade: the store's
//! resident size is its headers and judgements, not its text.
//!
//! Every answer is the one `Store::open` of the same log gives, which the store conformance
//! suite checks with the cache at zero bytes and at "everything fits".

mod build;
mod cache;
mod log;
mod shard;
mod shared;
mod tables;

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use ::redb::{ReadOnlyTable, ReadTransaction, ReadableTable, ReadableTableMetadata};
use smysl_core::ids::Tid;
use smysl_core::{
    from_cbor, from_cbor_seq, Attestation, Commit, Contention, Error, Record, RelKind, Relation,
    Rolling, Status, Uid, Unit,
};

pub use cache::CacheStats;
pub use shard::RedbShard;

use self::build::Described;
use self::cache::Cache;
use self::shared::Shared;
use self::tables::{
    attest_range, split_source_key, uid_of, Header, ATTEST, DEPS, HEADER, INTERP, OBSERVED,
    RECORD_HASH, SOURCE_REF,
};
use super::interp::Interpretive;
use super::library::StoreLibrary;
use super::read::{empty_commits, sealed};
use super::{
    digest_of_hashes, state_hash_of, Commits, HotKind, HotRecords, OpenReport, Relations,
    Resolutions, Store, StoreRead, Threads, Views, Withdrawals,
};
use crate::adjacency::{Adjacency, UnitEdges};

/// How to open a [`RedbStore`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RedbOptions {
    /// The paged tier of the core cache, in bytes (§3.5.4). Zero turns the cache off, pinned
    /// tier included, so every `get` reads the log.
    ///
    /// 32 MiB, not the RFC's 64: the resident set follows the budget almost byte for byte, and
    /// measured on graph-155510 `check` peaked at 164 MB with 32 MiB and 205 MB with 64 — 35%
    /// and 43% of the in-memory store's 473 MB, on either side of TX-P13b step 1's 40%. The
    /// time bought by the larger budget was 2.25 s to 1.99 s. A caller with the memory to spare
    /// and a store that fits sets it higher: everything cached ran `check` in 1.07 s.
    pub core_cache_bytes: usize,
    /// redb's own page cache (`Builder::set_cache_size`). redb's default is a gigabyte, which
    /// would be most of what this store exists to save; this one is 16 MiB.
    pub page_cache_bytes: usize,
    /// How long [`crate::AnyStore::open_redb`] waits for a store another process holds before
    /// reading the log instead (`SMY-W504`, §3.8).
    pub reader_wait_ms: u64,
    /// Rebuild from the log even when the store describes it.
    pub force_rebuild: bool,
    /// What a [`RedbShard`]'s lock file says its writer is doing, for the `SMY-E445` a second
    /// writer is shown: a command's name, never its arguments, which can hold paths. The
    /// default is the program's own file name.
    pub command: String,
}

impl Default for RedbOptions {
    fn default() -> RedbOptions {
        RedbOptions {
            core_cache_bytes: 32 << 20,
            page_cache_bytes: 16 << 20,
            reader_wait_ms: 2_000,
            force_rebuild: false,
            command: std::env::args_os()
                .next()
                .and_then(|a| {
                    Path::new(&a)
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "smysl".into()),
        }
    }
}

/// An error from redb, or from a store that is not what it should be, as an `Error`.
pub(super) fn backend(e: impl std::fmt::Display) -> Error {
    Error::Io(std::io::Error::other(format!("unit store: {e}")))
}

/// The error an open returns when another process holds the store: `WouldBlock`, which is
/// what [`crate::AnyStore::open_redb`] waits on.
pub(super) fn held(path: &Path) -> Error {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::WouldBlock,
        format!("unit store {name} is held by another process"),
    ))
}

/// Whether an error is [`held`]'s.
pub(crate) fn is_held(e: &Error) -> bool {
    matches!(e, Error::Io(io) if io.kind() == std::io::ErrorKind::WouldBlock)
}

/// A store whose cores stay in its CBOR log until asked for (SMYSL-2.8 §3.5).
///
/// Cheap to clone: a clone is the same open store, its snapshot and its cache.
#[derive(Clone)]
pub struct RedbStore(Arc<Inner>);

/// What a unit is, without its core. 40 bytes, against a kilobyte or more for the unit.
#[derive(Debug, Clone, Copy)]
struct Head {
    offset: u64,
    len: u32,
    attested: u32,
    hop: u32,
    observed: Option<u64>,
    pinned: bool,
}

// Field order is drop order: the table and the transaction before the database they read.
struct Inner {
    attest: ReadOnlyTable<&'static [u8; 64], &'static [u8]>,
    txn: ReadTransaction,
    /// The database the transaction reads, shared with every other open of it in this
    /// process; held so the file stays open as long as the snapshot does.
    _shared: Arc<Shared>,
    opts: RedbOptions,
    log: Option<File>,
    log_path: PathBuf,
    described: Described,
    uids: Vec<Uid>,
    heads: Vec<Head>,
    interp: Interpretive,
    /// The records of the hot kinds, in log order, and where each kind's are among them.
    hot_records: Vec<Record>,
    hot: BTreeMap<HotKind, Vec<usize>>,
    library: StoreLibrary,
    pending: Vec<Attestation>,
    /// Distinct records: `described.records` less the repeats.
    distinct: u64,
    cache: Mutex<Cache>,
    hashes: OnceLock<Vec<[u8; 32]>>,
    derived: OnceLock<Derived>,
}

struct Derived {
    adjacency: Adjacency,
    unfounded: BTreeSet<Uid>,
}

/// A unit's two edge lists, read from `deps` rather than from its core.
struct Edges {
    deps: Vec<Uid>,
    grounds: Vec<Uid>,
}

impl UnitEdges for Edges {
    fn deps(&self) -> impl Iterator<Item = &Uid> {
        self.deps.iter()
    }
    fn grounds(&self) -> impl Iterator<Item = &Uid> {
        self.grounds.iter()
    }
}

impl std::fmt::Debug for RedbStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedbStore")
            .field("units", &self.0.uids.len())
            .field("records", &self.0.described.records)
            .field("log_len", &self.0.described.log_len)
            .finish_non_exhaustive()
    }
}

impl RedbStore {
    /// Where the unit store for a log lives: `.smysl/store/<name>.redb` beside it, next to the
    /// sidecar's `.smysl/index/` (§3.5.2).
    pub fn store_path(log: &Path) -> PathBuf {
        let name = log
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "store".into());
        log.parent()
            .unwrap_or_else(|| Path::new("."))
            .join(".smysl")
            .join("store")
            .join(format!("{name}.redb"))
    }

    /// Open the unit store for a log, building it, rebuilding it or folding in the log's tail
    /// as the log requires.
    ///
    /// Fails at once with an `Io` error of kind `WouldBlock` when another process holds the
    /// store; [`crate::AnyStore::open_redb`] is the open that waits and falls back.
    pub fn open(
        log: impl AsRef<Path>,
        opts: RedbOptions,
    ) -> Result<(RedbStore, OpenReport), Error> {
        let log = log.as_ref();
        let path = Self::store_path(log);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(Error::Io)?;
        }
        let mut report = OpenReport::default();
        let shared = shared::open(&path, &opts, &mut report)?;
        Self::open_shared(shared, log, &opts, report)
    }

    /// [`RedbStore::open`] over a database already opened, whatever its backend: the seam the
    /// fault-injection tests open through.
    #[cfg(test)]
    pub(crate) fn open_db(
        db: ::redb::Database,
        log_path: &Path,
        opts: &RedbOptions,
        report: OpenReport,
    ) -> Result<(RedbStore, OpenReport), Error> {
        Self::open_shared(Arc::new(Shared::new(db)), log_path, opts, report)
    }

    fn open_shared(
        shared: Arc<Shared>,
        log_path: &Path,
        opts: &RedbOptions,
        mut report: OpenReport,
    ) -> Result<(RedbStore, OpenReport), Error> {
        let log = match File::open(log_path) {
            Ok(f) => Some(f),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(Error::Io(e)),
        };
        let described = {
            let _folding = shared.folding();
            build::catch_up(&shared.db, log.as_ref(), opts.force_rebuild, &mut report)?.0
        };
        let store = Self::load(shared, log, log_path, described, opts)?;
        Ok((store, report))
    }

    /// Read what stays in memory: headers, the interpretive layer and the library.
    fn load(
        shared: Arc<Shared>,
        log: Option<File>,
        log_path: &Path,
        described: Described,
        opts: &RedbOptions,
    ) -> Result<RedbStore, Error> {
        let txn = shared.db.begin_read().map_err(backend)?;

        let mut uids = Vec::new();
        let mut heads = Vec::new();
        let mut by_status = Vec::new();
        for row in txn
            .open_table(HEADER)
            .map_err(backend)?
            .iter()
            .map_err(backend)?
        {
            let (k, v) = row.map_err(backend)?;
            let h = Header::decode(v.value());
            uids.push(Uid::from_bytes(*k.value()));
            by_status.push(h.pinned_by_status());
            heads.push(Head {
                offset: h.offset,
                len: h.len,
                attested: h.attested,
                hop: h.hop,
                observed: h.observed,
                pinned: false,
            });
        }

        // The rest of the log, in log order, folded as `absorb` folds it.
        let mut interp = Interpretive::default();
        let mut library = StoreLibrary::default();
        let mut hot_records = Vec::new();
        let mut hot: BTreeMap<HotKind, Vec<usize>> = BTreeMap::new();
        for row in txn
            .open_table(INTERP)
            .map_err(backend)?
            .iter()
            .map_err(backend)?
        {
            let (_, v) = row.map_err(backend)?;
            let r = from_cbor(v.value())?.0;
            if interp.absorb_hot(&r) {
                continue;
            }
            match &r {
                Record::Manifest(m) => library.absorb_manifest(m),
                Record::Dating(d) => library.absorb_dating(d),
                Record::Redaction(x) => library.absorb_redaction(x),
                _ => {}
            }
            if let Some(kind) = HotKind::of(&r) {
                hot.entry(kind).or_default().push(hot_records.len());
                hot_records.push(r);
            }
        }
        let source = txn.open_table(SOURCE_REF).map_err(backend)?;
        for row in source.range(Tid::PREFIX.as_bytes()..).map_err(backend)? {
            let (k, _) = row.map_err(backend)?;
            let (reference, uid) = split_source_key(k.value());
            if !reference.starts_with(Tid::PREFIX.as_bytes()) {
                break;
            }
            if let Ok(reference) = std::str::from_utf8(reference) {
                library.absorb_reference(uid, reference);
            }
        }

        // An attestation whose subject is a unit stays on disk until the unit is read; one
        // naming a relation joins it now; one naming nothing here waits, as in memory.
        let attest = txn.open_table(ATTEST).map_err(backend)?;
        let mut pending = Vec::new();
        for row in attest.iter().map_err(backend)? {
            let (k, v) = row.map_err(backend)?;
            if uids.binary_search(&uid_of(k.value())).is_ok() {
                continue;
            }
            if let Record::Attestation(a) = from_cbor(v.value())?.0 {
                if !interp.attach_to_relation(&a) {
                    pending.push(a);
                }
            }
        }

        // Pinning (OQ-69): by status, and every relation endpoint.
        for (h, pinned) in heads.iter_mut().zip(&by_status) {
            h.pinned = *pinned;
        }
        for rel in interp.relations.values() {
            for end in [rel.from, rel.to] {
                if let Ok(i) = uids.binary_search(&end) {
                    heads[i].pinned = true;
                }
            }
        }

        let distinct = txn
            .open_table(RECORD_HASH)
            .map_err(backend)?
            .len()
            .map_err(backend)?;
        Ok(RedbStore(Arc::new(Inner {
            attest,
            txn,
            _shared: shared,
            opts: opts.clone(),
            log,
            log_path: log_path.to_path_buf(),
            described,
            uids,
            heads,
            interp,
            hot_records,
            hot,
            library,
            pending,
            distinct,
            cache: Mutex::new(Cache::new(opts.core_cache_bytes)),
            hashes: OnceLock::new(),
            derived: OnceLock::new(),
        })))
    }

    /// Records in the log this store describes, duplicates included, as `Store::len`.
    pub fn len(&self) -> usize {
        self.0.described.records as usize
    }

    pub fn is_empty(&self) -> bool {
        self.0.described.records == 0
    }

    /// The length of the log this store describes, in bytes: the end of its last complete
    /// record.
    pub fn log_len(&self) -> u64 {
        self.0.described.log_len
    }

    /// The hash of those bytes, as `Store::log_hash`.
    pub fn log_hash(&self) -> &[u8; 32] {
        &self.0.described.log_hash
    }

    /// The log this store describes.
    pub fn log_path(&self) -> &Path {
        &self.0.log_path
    }

    /// The log's bytes up to [`RedbStore::log_len`], as `Store::log_bytes` gives them.
    pub fn log_bytes(&self) -> Vec<u8> {
        match &self.0.log {
            Some(f) => log::read_at(f, 0, self.0.described.log_len as usize)
                .unwrap_or_else(|e| log_moved(e)),
            None => Vec::new(),
        }
    }

    /// The options it was opened with.
    pub fn options(&self) -> &RedbOptions {
        &self.0.opts
    }

    /// How the core cache has done so far.
    pub fn cache_stats(&self) -> CacheStats {
        self.cache().stats()
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.0.cache.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The unit at index `i` of the uid order: from the cache, or from the log and `attest`.
    ///
    /// `scan` is a read on the way through `units()`. It is answered from the cache, and keeps
    /// a pinned core, but does not put a paged one in: a scan of a store larger than the budget
    /// would otherwise evict every core before the next scan reached it again, and measured at
    /// 155k units that made a 64 MiB tier slower than none (2.5 s for `check` against 2.2 s).
    /// What fills the paged tier is `get`, which is what reads the same few units twice.
    fn unit_at(&self, i: usize, scan: bool) -> Arc<Unit> {
        if let Some(u) = self.cache().get(i as u32) {
            return u;
        }
        let h = self.0.heads[i];
        let file = self.0.log.as_ref().unwrap_or_else(|| log_moved("no log"));
        let bytes = log::read_at(file, h.offset, h.len as usize).unwrap_or_else(|e| log_moved(e));
        let core = match from_cbor(&bytes) {
            Ok((Record::Unit(core), _)) => core,
            Ok(_) => log_moved("a unit's offset holds another record"),
            Err(e) => log_moved(e),
        };
        let mut unit = Unit::new(core);
        if h.attested > 0 {
            let (lo, hi) = attest_range(&self.0.uids[i]);
            let rows = self
                .0
                .attest
                .range::<&[u8; 64]>(&lo..=&hi)
                .unwrap_or_else(|e| log_moved(e));
            for row in rows {
                let (_, v) = row.unwrap_or_else(|e| log_moved(e));
                if let Ok((Record::Attestation(a), _)) = from_cbor(v.value()) {
                    unit.attestations.insert(a);
                }
            }
        }
        let unit = Arc::new(unit);
        if h.pinned || !scan {
            self.cache()
                .put(i as u32, &unit, h.pinned, cache::resident(&unit));
        }
        unit
    }

    fn index_of(&self, uid: &Uid) -> Option<usize> {
        self.0.uids.binary_search(uid).ok()
    }

    fn derived(&self) -> &Derived {
        self.0.derived.get_or_init(|| {
            // Edges from `deps`, so no core is read to build them.
            let mut units: Vec<(Uid, Edges)> = self
                .0
                .uids
                .iter()
                .map(|u| {
                    (
                        *u,
                        Edges {
                            deps: Vec::new(),
                            grounds: Vec::new(),
                        },
                    )
                })
                .collect();
            let deps = self.0.txn.open_table(DEPS).unwrap_or_else(|e| log_moved(e));
            let mut at = 0usize;
            for row in deps.iter().unwrap_or_else(|e| log_moved(e)) {
                let (k, _) = row.unwrap_or_else(|e| log_moved(e));
                let k = k.value();
                let uid = uid_of(&k[..32]);
                while units[at].0 < uid {
                    at += 1;
                }
                let target = uid_of(&k[33..]);
                if k[32] == 0 {
                    units[at].1.deps.push(target);
                } else {
                    units[at].1.grounds.push(target);
                }
            }
            let relations: Vec<Relation> = self
                .0
                .interp
                .relations
                .values()
                .filter(|r| !self.0.interp.is_withdrawn(r))
                .cloned()
                .collect();
            let adjacency = Adjacency::from_parts(&units, &relations);
            drop(units);
            // As `Store::derive`: only worth computing when something retracts, and
            // `effective_status` reads units and relations, never `derived`.
            let mut unfounded = BTreeSet::new();
            if self
                .0
                .interp
                .relations
                .keys()
                .any(|(k, _, _)| k == RelKind::Retracts.as_str())
            {
                let eff =
                    crate::merge::effective_status(self, crate::merge::RetractionPolicy::Strict);
                unfounded = self
                    .0
                    .uids
                    .iter()
                    .filter(|u| eff.get(u) == Some(Status::Unfounded))
                    .copied()
                    .collect();
            }
            Derived {
                adjacency,
                unfounded,
            }
        })
    }

    fn hashes(&self) -> &[[u8; 32]] {
        self.0.hashes.get_or_init(|| {
            let t = self
                .0
                .txn
                .open_table(RECORD_HASH)
                .unwrap_or_else(|e| log_moved(e));
            t.iter()
                .unwrap_or_else(|e| log_moved(e))
                .map(|row| *row.unwrap_or_else(|e| log_moved(e)).0.value())
                .collect()
        })
    }
}

/// A store's log or tables failed it after it opened.
///
/// `StoreRead` answers, it does not fail, and an open has proved that the log holds what the
/// store describes. What is left is a log truncated or rewritten under an open store — which
/// an append-only log never is — or a disk that stopped reading. Answering on would be
/// answering about a store that no longer exists, so this stops instead.
fn log_moved(e: impl std::fmt::Display) -> ! {
    panic!("the unit store's log changed or failed while it was open: {e}")
}

/// A [`RedbStore`]'s units, in ascending uid order, each read when it is reached.
pub struct RedbUnits<'a> {
    store: &'a RedbStore,
    at: usize,
}

impl<'a> Iterator for RedbUnits<'a> {
    type Item = (&'a Uid, Arc<Unit>);
    fn next(&mut self) -> Option<Self::Item> {
        let uid = self.store.0.uids.get(self.at)?;
        let unit = self.store.unit_at(self.at, true);
        self.at += 1;
        Some((uid, unit))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.store.0.uids.len() - self.at;
        (left, Some(left))
    }
}

/// A [`RedbStore`]'s log, in log order, read as it is walked.
pub struct RedbRecords<'a>(log::LogReader<'a>);

impl<'a> Iterator for RedbRecords<'a> {
    type Item = Cow<'a, Record>;
    fn next(&mut self) -> Option<Cow<'a, Record>> {
        match self.0.next() {
            Ok(r) => r.map(|(_, _, r)| Cow::Owned(r)),
            Err(e) => log_moved(e),
        }
    }
}

impl sealed::Sealed for RedbStore {
    fn owned(&self) -> Cow<'_, Store> {
        let bytes = self.log_bytes();
        let (records, consumed) = from_cbor_seq(&bytes).unwrap_or_else(|e| log_moved(e));
        // With no path, as `Store::from_records` has none: an in-memory copy is for reading,
        // and one that kept the log's path would write to it on `append` — once more after the
        // writer had, which is what `--backend redb retract` did before this said so.
        Cow::Owned(Store::of_log(None, records, &bytes[..consumed]))
    }
    fn interp(&self) -> &Interpretive {
        &self.0.interp
    }
    fn pending_attestations(&self) -> &[Attestation] {
        &self.0.pending
    }
    fn has_record(&self, h: &[u8; 32]) -> bool {
        self.hashes().binary_search(h).is_ok()
    }
    fn record_hashes(&self) -> Box<dyn Iterator<Item = &[u8; 32]> + '_> {
        Box::new(self.hashes().iter())
    }
}

impl StoreRead for RedbStore {
    type UnitRef<'a> = Arc<Unit>;
    type Units<'a> = RedbUnits<'a>;
    type Records<'a> = RedbRecords<'a>;

    fn get(&self, uid: &Uid) -> Option<Arc<Unit>> {
        self.index_of(uid).map(|i| self.unit_at(i, false))
    }
    fn contains_uid(&self, uid: &Uid) -> bool {
        self.index_of(uid).is_some()
    }
    fn units(&self) -> RedbUnits<'_> {
        RedbUnits { store: self, at: 0 }
    }
    fn unit_count(&self) -> usize {
        self.0.uids.len()
    }
    fn units_in_range(&self, lo: &Uid, hi: &Uid) -> Vec<Uid> {
        if lo > hi {
            return Vec::new();
        }
        let uids = &self.0.uids;
        let a = uids.partition_point(|u| u < lo);
        let b = uids.partition_point(|u| u <= hi);
        uids[a..b].to_vec()
    }
    fn observed_at(&self, uid: &Uid) -> Option<u64> {
        self.index_of(uid).and_then(|i| self.0.heads[i].observed)
    }
    fn hop_of(&self, uid: &Uid) -> Option<u32> {
        let h = self.0.heads[self.index_of(uid)?];
        (h.attested > 0).then_some(h.hop)
    }
    fn units_with_source_prefix(&self, prefix: &str) -> Vec<Uid> {
        let t = self
            .0
            .txn
            .open_table(SOURCE_REF)
            .unwrap_or_else(|e| log_moved(e));
        let mut out = Vec::new();
        for row in t
            .range(prefix.as_bytes()..)
            .unwrap_or_else(|e| log_moved(e))
        {
            let (k, _) = row.unwrap_or_else(|e| log_moved(e));
            let k = k.value();
            // Every key of a matching reference starts with the prefix, and they sort
            // together; a shorter reference whose uid happens to continue it does not match.
            if !k.starts_with(prefix.as_bytes()) {
                break;
            }
            let (reference, uid) = split_source_key(k);
            if reference.starts_with(prefix.as_bytes()) {
                out.push(uid);
            }
        }
        out.sort_unstable();
        out
    }
    fn units_in_observed_order(&self) -> Vec<Uid> {
        let t = self
            .0
            .txn
            .open_table(OBSERVED)
            .unwrap_or_else(|e| log_moved(e));
        t.iter()
            .unwrap_or_else(|e| log_moved(e))
            .map(|row| uid_of(&row.unwrap_or_else(|e| log_moved(e)).0.value()[8..]))
            .collect()
    }

    fn adjacency(&self) -> &Adjacency {
        &self.derived().adjacency
    }
    fn relations(&self) -> Relations<'_> {
        Relations(self.0.interp.relations.values())
    }
    fn relation_by_id(&self, rid: &Uid) -> Option<&Relation> {
        self.0.interp.relation_by_id(rid)
    }
    fn relation_by_key(&self, kind: &RelKind, from: &Uid, to: &Uid) -> Option<&Relation> {
        self.0
            .interp
            .relations
            .get(&(kind.as_str().to_string(), *from, *to))
    }
    fn threads(&self) -> Threads<'_> {
        Threads(self.0.interp.threads.values())
    }
    fn views(&self) -> Views<'_> {
        Views(self.0.interp.views.values())
    }
    fn contentions(&self) -> &[Contention] {
        &self.0.interp.contentions
    }
    fn withdrawals(&self) -> Withdrawals<'_> {
        Withdrawals(self.0.interp.withdrawals.values().flatten())
    }
    fn resolutions(&self) -> Resolutions<'_> {
        Resolutions(self.0.interp.resolutions.iter())
    }
    fn commits(&self) -> Commits<'_> {
        Commits(self.0.interp.commits.iter())
    }
    fn commits_of(&self, uid: &Uid) -> &BTreeSet<Commit> {
        self.0
            .interp
            .commits
            .get(uid)
            .unwrap_or_else(|| empty_commits())
    }
    fn is_withdrawn(&self, rel: &Relation) -> bool {
        self.0.interp.is_withdrawn(rel)
    }
    fn is_unfounded(&self, uid: &Uid) -> bool {
        self.derived().unfounded.contains(uid)
    }
    fn hot_records(&self, kind: HotKind) -> HotRecords<'_> {
        HotRecords {
            records: &self.0.hot_records,
            at: self
                .0
                .hot
                .get(&kind)
                .map(|v| v.as_slice())
                .unwrap_or(&[])
                .iter(),
            then: None,
        }
    }
    fn library(&self) -> &StoreLibrary {
        &self.0.library
    }

    fn records(&self) -> RedbRecords<'_> {
        RedbRecords(log::LogReader::new(
            self.0.log.as_ref(),
            0,
            self.0.described.log_len,
            Rolling::new(),
        ))
    }
    fn duplicate_records(&self) -> usize {
        (self.0.described.records - self.0.distinct) as usize
    }
    fn record_set_digest(&self) -> [u8; 32] {
        digest_of_hashes(self.hashes())
    }
    fn state_hash(&self) -> [u8; 32] {
        state_hash_of(self.units(), &self.0.interp)
    }
}

#[cfg(test)]
mod tests;
