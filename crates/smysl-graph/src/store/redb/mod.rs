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
//! **Several logs** (TX-P13c step 3, OQ-90). [`RedbStore::open_shards`] opens one store over a
//! library's shard logs, every row naming its log, and answers as `Store::from_records` of all
//! their records in name order — or of some of them, with a reference into another reported as
//! waiting on it (`SMY-W500`). Nothing writes through it: the library writes its shards' logs,
//! and the store folds each log's new tail on its own when it is next opened. Logs are opened
//! as cores are read, in a pool as large as the process's descriptors allow.
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
mod primary;
mod shard;
mod shared;
mod tables;

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use ::redb::{ReadOnlyTable, ReadTransaction, ReadableTable};
use smysl_core::ids::Tid;
use smysl_core::{
    from_cbor, from_cbor_seq, Attestation, Commit, Contention, Error, Record, RelKind, Relation,
    Rolling, Status, Uid, Unit,
};

pub use cache::CacheStats;
pub use primary::{LogState, RedbShards};
pub use shard::RedbShard;

use smysl_core::diag::{Code, Diagnostic, Report, Subject};

use self::build::{Described, Given, Kind};
use self::cache::Cache;
use self::shared::Shared;
use self::tables::{
    attest_range, interp_key, log_of, log_range, records_key, split_source_key, uid_of, Header,
    LogIx, ATTEST, DEPS, HEADER, HOLDS, INTERP, OBSERVED, RECORDS, RECORD_HASH, SOURCE_REF,
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

/// A store whose cores stay in its CBOR logs until asked for (SMYSL-2.8 §3.5).
///
/// Over one log ([`RedbStore::open`]) it answers as `Store::open` of the log. Over several
/// ([`RedbStore::open_shards`], TX-P13c step 3) it answers as `Store::from_records` of all their
/// records in name order — a library's shards as one store — and can be opened for some of them.
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
    log: LogIx,
    pinned: bool,
}

/// One of the logs a store reads.
struct LogFile {
    name: String,
    ix: LogIx,
    path: PathBuf,
    described: Described,
    /// Its records are the store's (TX-P13d): read from `records`, not from `path`.
    primary: bool,
}

/// How many logs a store keeps open at first. A store of a thousand logs read in uid order
/// touches every one of them over and over, so the pool holds them all if the process may —
/// with a pool smaller than the logs, `check` on GE-T7's stand-in opened a file for nearly every
/// core it read, 132 s of system time. A process may also have as few as 256 descriptors, so an
/// open the system refuses for want of one halves the pool and is tried again.
const OPEN_LOGS: usize = 4096;

struct Open {
    files: Vec<(LogIx, Arc<File>, u64)>,
    tick: u64,
    cap: usize,
}

impl Default for Open {
    fn default() -> Open {
        Open {
            files: Vec::new(),
            tick: 0,
            cap: OPEN_LOGS,
        }
    }
}

impl Open {
    fn evict_oldest(&mut self) {
        if let Some(oldest) = (0..self.files.len()).min_by_key(|&i| self.files[i].2) {
            self.files.swap_remove(oldest);
        }
    }
}

/// Whether an open failed because the process has no descriptor left (`EMFILE`, `ENFILE`).
fn out_of_descriptors(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(23 | 24))
}

// Field order is drop order: the table and the transaction before the database they read.
struct Inner {
    attest: ReadOnlyTable<&'static [u8; 66], &'static [u8]>,
    txn: ReadTransaction,
    /// The database the transaction reads, shared with every other open of it in this
    /// process; held so the file stays open as long as the snapshot does.
    _shared: Arc<Shared>,
    opts: RedbOptions,
    kind: Kind,
    /// Every log the store reads, in name order.
    logs: Vec<LogFile>,
    /// A log's place in `logs`, by its position in the store.
    at: Vec<Option<usize>>,
    /// The name of every log the store describes, read or not, by position: what a reference
    /// into one not read waits on.
    names: Vec<Option<String>>,
    /// Where every log given lives, read or not, by position: a unit's core is read from the
    /// first log to bring it, which a store opened for some logs may not read.
    paths: Vec<Option<PathBuf>>,
    /// Which of them are primary, by position: their cores and records are in `records`.
    primary: Vec<bool>,
    /// Whether the store was opened for some of its logs only.
    subset: bool,
    open: Mutex<Open>,
    uids: Vec<Uid>,
    heads: Vec<Head>,
    interp: Interpretive,
    /// The records of the hot kinds, in log order, and where each kind's are among them.
    hot_records: Vec<Record>,
    hot: BTreeMap<HotKind, Vec<usize>>,
    library: StoreLibrary,
    pending: Vec<Attestation>,
    cache: Mutex<Cache>,
    hashes: OnceLock<Vec<[u8; 32]>>,
    /// Over several logs: the length and hash of their records as one log, each once.
    joined: OnceLock<(u64, [u8; 32])>,
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
            .field("logs", &self.0.logs.len())
            .field("records", &self.len())
            .finish_non_exhaustive()
    }
}

/// The name a store of one log gives it.
const ONE: &str = "";

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
        Self::open_shared(
            shared,
            Kind::One,
            &[(ONE.to_string(), log.to_path_buf())],
            None,
            &opts,
            report,
        )
    }

    /// Open one unit store over several logs — a library's shards (TX-P13c step 3, OQ-90) —
    /// at `store`, building it, rebuilding it or folding in each log's tail as the logs
    /// require. `shards` names every log the store describes, a name each; with `only`, the
    /// store answers for those logs alone, as `Store::from_records` of their records would, and
    /// a reference into one of the others is `SMY-W500` naming it, not `SMY-E060`.
    ///
    /// One store, not one a log: a store's fixed cost paid a thousand times was most of a
    /// thousand-shard library's resident set (SMYSL-2.8 §6.1, P13c.2). Fails at once with
    /// `WouldBlock` when another process holds the store, as [`RedbStore::open`] does.
    pub fn open_shards(
        store: &Path,
        shards: &[(String, PathBuf)],
        only: Option<&[String]>,
        opts: RedbOptions,
    ) -> Result<(RedbStore, OpenReport), Error> {
        if let Some(dir) = store.parent() {
            std::fs::create_dir_all(dir).map_err(Error::Io)?;
        }
        let mut report = OpenReport::default();
        let shared = shared::open(store, &opts, &mut report)?;
        Self::open_shared(shared, Kind::Many, shards, only, &opts, report)
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
        Self::open_shared(
            Arc::new(Shared::new(db)),
            Kind::One,
            &[(ONE.to_string(), log_path.to_path_buf())],
            None,
            opts,
            report,
        )
    }

    fn open_shared(
        shared: Arc<Shared>,
        kind: Kind,
        logs: &[(String, PathBuf)],
        only: Option<&[String]>,
        opts: &RedbOptions,
        mut report: OpenReport,
    ) -> Result<(RedbStore, OpenReport), Error> {
        let given: Vec<Given<'_>> = logs
            .iter()
            .map(|(name, path)| Given {
                name,
                path,
                file: None,
            })
            .collect();
        let caught = {
            let _folding = shared.folding();
            build::catch_up(&shared.db, kind, &given, opts.force_rebuild, &mut report)?
        };
        let mut paths: Vec<Option<PathBuf>> = Vec::new();
        let mut primary: Vec<bool> = Vec::new();
        for ((_, path), c) in logs.iter().zip(&caught) {
            let i = c.ix as usize;
            if paths.len() <= i {
                paths.resize(i + 1, None);
                primary.resize(i + 1, false);
            }
            paths[i] = Some(path.clone());
            primary[i] = c.primary;
        }
        let read: Vec<LogFile> = logs
            .iter()
            .zip(caught)
            .filter(|((name, _), _)| only.is_none_or(|o| o.iter().any(|n| n == name)))
            .map(|((name, path), c)| LogFile {
                name: name.clone(),
                ix: c.ix,
                path: path.clone(),
                described: c.described,
                primary: c.primary,
            })
            .collect();
        let store = Self::load(shared, kind, read, paths, primary, only.is_some(), opts)?;
        Ok((store, report))
    }

    /// Read what stays in memory: headers, the interpretive layer and the library, of the logs
    /// in `logs`.
    #[allow(clippy::too_many_arguments)]
    fn load(
        shared: Arc<Shared>,
        kind: Kind,
        mut logs: Vec<LogFile>,
        paths: Vec<Option<PathBuf>>,
        primary: Vec<bool>,
        subset: bool,
        opts: &RedbOptions,
    ) -> Result<RedbStore, Error> {
        logs.sort_by(|a, b| a.name.cmp(&b.name));
        let txn = shared.db.begin_read().map_err(backend)?;
        let width = logs.iter().map(|l| l.ix as usize + 1).max().unwrap_or(0);
        let mut at = vec![None; width];
        for (i, l) in logs.iter().enumerate() {
            at[l.ix as usize] = Some(i);
        }
        let read = |ix: LogIx| at.get(ix as usize).copied().flatten().is_some();
        let mut names: Vec<Option<String>> = Vec::new();
        if let Some(m) = build::meta(&shared.db)? {
            for (name, row) in m.logs {
                let i = row.ix as usize;
                if names.len() <= i {
                    names.resize(i + 1, None);
                }
                names[i] = Some(name);
            }
        }

        // The units of the logs read: every unit, or those some log read holds.
        let members: Option<Vec<Uid>> = if subset {
            let mut v: Vec<Uid> = Vec::new();
            for row in txn
                .open_table(HOLDS)
                .map_err(backend)?
                .iter()
                .map_err(backend)?
            {
                let (k, _) = row.map_err(backend)?;
                let k = k.value();
                let uid = uid_of(k);
                if read(log_of(k)) && v.last() != Some(&uid) {
                    v.push(uid);
                }
            }
            Some(v)
        } else {
            None
        };
        let mut uids = Vec::new();
        let mut heads = Vec::new();
        let mut by_status = Vec::new();
        let mut next_member = 0usize;
        for row in txn
            .open_table(HEADER)
            .map_err(backend)?
            .iter()
            .map_err(backend)?
        {
            let (k, v) = row.map_err(backend)?;
            let uid = Uid::from_bytes(*k.value());
            if let Some(m) = &members {
                while next_member < m.len() && m[next_member] < uid {
                    next_member += 1;
                }
                if m.get(next_member) != Some(&uid) {
                    continue;
                }
            }
            let h = Header::decode(v.value());
            uids.push(uid);
            by_status.push(h.pinned_by_status());
            heads.push(Head {
                offset: h.offset,
                len: h.len,
                attested: h.attested,
                hop: h.hop,
                observed: h.observed,
                log: h.log,
                pinned: false,
            });
        }

        // The rest of each log, the logs in name order and each in log order, folded as
        // `absorb` folds it. Over several logs a record is read once, where it first appears:
        // `from_records`, which drops repeats; over one, every record, as `Store::open` keeps
        // them.
        let mut interp = Interpretive::default();
        let mut library = StoreLibrary::default();
        let mut hot_records = Vec::new();
        let mut hot: BTreeMap<HotKind, Vec<usize>> = BTreeMap::new();
        let mut seen: BTreeSet<[u8; 32]> = BTreeSet::new();
        let table = txn.open_table(INTERP).map_err(backend)?;
        for l in &logs {
            let (lo, hi) = (interp_key(l.ix, 0), interp_key(l.ix, u64::MAX));
            for row in table.range::<&[u8; 10]>(&lo..=&hi).map_err(backend)? {
                let (_, v) = row.map_err(backend)?;
                let r = from_cbor(v.value())?.0;
                if kind == Kind::Many && !seen.insert(Store::record_hash(&r)) {
                    continue;
                }
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
        }
        drop((table, seen));
        let source = txn.open_table(SOURCE_REF).map_err(backend)?;
        for row in source.range(Tid::PREFIX.as_bytes()..).map_err(backend)? {
            let (k, _) = row.map_err(backend)?;
            let (reference, uid) = split_source_key(k.value());
            if !reference.starts_with(Tid::PREFIX.as_bytes()) {
                break;
            }
            if subset && uids.binary_search(&uid).is_err() {
                continue;
            }
            if let Ok(reference) = std::str::from_utf8(reference) {
                library.absorb_reference(uid, reference);
            }
        }

        // An attestation whose subject is a unit stays on disk until the unit is read; one
        // naming a relation joins it now; one naming nothing here waits, as in memory. Each
        // once, however many logs hold it, and only the logs read. A unit's count and newest
        // hop are the store's over every log; opened for some, they are counted again.
        let attest = txn.open_table(ATTEST).map_err(backend)?;
        let mut pending = Vec::new();
        if subset {
            for h in &mut heads {
                h.attested = 0;
                h.hop = 0;
            }
        }
        let mut last: Option<[u8; 64]> = None;
        for row in attest.iter().map_err(backend)? {
            let (k, v) = row.map_err(backend)?;
            let k = k.value();
            if !read(log_of(k)) {
                continue;
            }
            let id: [u8; 64] = k[..64].try_into().expect("64 bytes");
            if last == Some(id) {
                continue;
            }
            last = Some(id);
            match uids.binary_search(&uid_of(k)) {
                Ok(i) if subset => {
                    if let Record::Attestation(a) = from_cbor(v.value())?.0 {
                        let h = &mut heads[i];
                        h.hop = if h.attested == 0 {
                            a.hop
                        } else {
                            h.hop.max(a.hop)
                        };
                        h.attested += 1;
                    }
                }
                Ok(_) => {}
                Err(_) => {
                    if let Record::Attestation(a) = from_cbor(v.value())?.0 {
                        if !interp.attach_to_relation(&a) {
                            pending.push(a);
                        }
                    }
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

        Ok(RedbStore(Arc::new(Inner {
            attest,
            txn,
            _shared: shared,
            opts: opts.clone(),
            kind,
            logs,
            at,
            names,
            paths,
            primary,
            subset,
            open: Mutex::new(Open::default()),
            uids,
            heads,
            interp,
            hot_records,
            hot,
            library,
            pending,
            cache: Mutex::new(Cache::new(opts.core_cache_bytes)),
            hashes: OnceLock::new(),
            joined: OnceLock::new(),
            derived: OnceLock::new(),
        })))
    }

    /// Records, as `Store::len`: over one log, duplicates included; over several, each once, as
    /// `Store::from_records` keeps them.
    pub fn len(&self) -> usize {
        match self.0.kind {
            Kind::One => self.0.logs.iter().map(|l| l.described.records).sum::<u64>() as usize,
            Kind::Many => self.hashes().len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The length of the log this store describes, in bytes: the end of its last complete
    /// record. Over several logs, the length of their records as one log, each once.
    pub fn log_len(&self) -> u64 {
        match self.0.kind {
            Kind::One => self.one().described.log_len,
            Kind::Many => self.joined().0,
        }
    }

    /// The hash of those bytes, as `Store::log_hash`.
    pub fn log_hash(&self) -> &[u8; 32] {
        match self.0.kind {
            Kind::One => &self.one().described.log_hash,
            Kind::Many => &self.joined().1,
        }
    }

    /// The log this store describes; over several, the first it reads, in name order.
    pub fn log_path(&self) -> &Path {
        self.0
            .logs
            .first()
            .map(|l| l.path.as_path())
            .unwrap_or_else(|| Path::new(""))
    }

    /// The names of the logs it reads, in name order: empty for a store of one log.
    pub fn shards(&self) -> impl Iterator<Item = &str> {
        self.0
            .logs
            .iter()
            .filter(|_| self.0.kind == Kind::Many)
            .map(|l| l.name.as_str())
    }

    /// Whether it is a store of several logs, which nothing appends to through it: choosing the
    /// log a record goes to is the library's routing.
    pub fn is_shards(&self) -> bool {
        self.0.kind == Kind::Many
    }

    /// The log's bytes up to [`RedbStore::log_len`], as `Store::log_bytes` gives them.
    pub fn log_bytes(&self) -> Vec<u8> {
        match self.0.kind {
            Kind::One => match self.file(self.one().ix) {
                Some(f) => log::read_at(&f, 0, self.one().described.log_len as usize)
                    .unwrap_or_else(|e| log_moved(e)),
                None => Vec::new(),
            },
            Kind::Many => {
                let mut out = Vec::new();
                for r in self.records() {
                    out.extend_from_slice(&smysl_core::to_cbor(&r));
                }
                out
            }
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

    fn one(&self) -> &LogFile {
        &self.0.logs[0]
    }

    fn joined(&self) -> &(u64, [u8; 32]) {
        self.0.joined.get_or_init(|| {
            let mut h = Rolling::new();
            let mut n = 0u64;
            for r in self.records() {
                let b = smysl_core::to_cbor(&r);
                n += b.len() as u64;
                h.update(&b);
            }
            (n, h.finish())
        })
    }

    /// A log the store was given, open: from the pool, or opened into it.
    fn file(&self, ix: LogIx) -> Option<Arc<File>> {
        let path = self.0.paths.get(ix as usize)?.as_ref()?;
        let mut open = self.0.open.lock().unwrap_or_else(|e| e.into_inner());
        open.tick += 1;
        let tick = open.tick;
        if let Some(e) = open.files.iter_mut().find(|e| e.0 == ix) {
            e.2 = tick;
            return Some(e.1.clone());
        }
        if open.files.len() >= open.cap {
            open.evict_oldest();
        }
        let f = loop {
            match File::open(path) {
                Ok(f) => break Arc::new(f),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
                Err(e) if out_of_descriptors(&e) && open.files.len() > 1 => {
                    open.cap = (open.files.len() / 2).max(1);
                    while open.files.len() > open.cap {
                        open.evict_oldest();
                    }
                }
                Err(e) => log_moved(e),
            }
        };
        open.files.push((ix, f.clone(), tick));
        Some(f)
    }

    /// Whether a row of log `ix` belongs to what this store answers.
    fn reads(&self, ix: LogIx) -> bool {
        self.0.at.get(ix as usize).copied().flatten().is_some()
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
        // Its core is the same in every log that holds it, so any will do; a store opened for
        // some logs reads it from the first to bring it, read or not.
        let bytes = if self.0.primary.get(h.log as usize).copied().unwrap_or(false) {
            let held = self
                .0
                .txn
                .open_table(RECORDS)
                .unwrap_or_else(|e| log_moved(e));
            let row = held
                .get(&records_key(h.log, h.offset))
                .unwrap_or_else(|e| log_moved(e))
                .unwrap_or_else(|| log_moved("a primary shard's record is not in the store"));
            row.value().to_vec()
        } else {
            let file = self.file(h.log).unwrap_or_else(|| log_moved("no log"));
            log::read_at(&file, h.offset, h.len as usize).unwrap_or_else(|e| log_moved(e))
        };
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
                .range::<&[u8; 66]>(&lo..=&hi)
                .unwrap_or_else(|e| log_moved(e));
            for row in rows {
                let (k, v) = row.unwrap_or_else(|e| log_moved(e));
                if !self.reads(log_of(k.value())) {
                    continue;
                }
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
                while at < units.len() && units[at].0 < uid {
                    at += 1;
                }
                // A unit of a log this store was not opened for.
                if at == units.len() || units[at].0 != uid {
                    continue;
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

    /// Every distinct record hash of the logs read, ascending.
    fn hashes(&self) -> &[[u8; 32]] {
        self.0.hashes.get_or_init(|| {
            let t = self
                .0
                .txn
                .open_table(RECORD_HASH)
                .unwrap_or_else(|e| log_moved(e));
            let mut out: Vec<[u8; 32]> = Vec::new();
            for row in t.iter().unwrap_or_else(|e| log_moved(e)) {
                let k = *row.unwrap_or_else(|e| log_moved(e)).0.value();
                if !self.reads(log_of(&k)) {
                    continue;
                }
                let h: [u8; 32] = k[..32].try_into().expect("32 bytes");
                if out.last() != Some(&h) {
                    out.push(h);
                }
            }
            out
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

/// A [`RedbStore`]'s log, in log order, read as it is walked: over several logs, each in name
/// order, and each record once.
pub struct RedbRecords<'a> {
    /// The logs to walk, in name order, each to the length the store describes: owned, so a walk
    /// holds nothing of the store (and is as unwind-safe as a file). A primary log's bytes are
    /// read out of the store when the walk reaches it.
    logs: Vec<Walk>,
    /// The next log in `logs` to start.
    next: usize,
    reader: Option<log::LogReader<'static>>,
    seen: Option<BTreeSet<[u8; 32]>>,
    /// The database, to read a primary log out of when the walk reaches it, one log at a time.
    /// Unwind-safe as asserted: it is only read, through transactions of its own.
    shared: std::panic::AssertUnwindSafe<Arc<Shared>>,
    _store: std::marker::PhantomData<&'a ()>,
}

/// One log of a record walk: a file to read, or a primary log to read out of the store, each to
/// the length the store's snapshot describes.
enum Walk {
    File(PathBuf, u64),
    Held(LogIx, u64),
}

impl<'a> Iterator for RedbRecords<'a> {
    type Item = Cow<'a, Record>;
    fn next(&mut self) -> Option<Cow<'a, Record>> {
        loop {
            if let Some(reader) = &mut self.reader {
                match reader.next() {
                    Ok(Some((_, _, r))) => {
                        if let Some(seen) = &mut self.seen {
                            if !seen.insert(Store::record_hash(&r)) {
                                continue;
                            }
                        }
                        return Some(Cow::Owned(r));
                    }
                    Ok(None) => self.reader = None,
                    Err(e) => log_moved(e),
                }
            }
            let walk = self.logs.get(self.next)?;
            self.next += 1;
            match walk {
                Walk::Held(ix, len) => {
                    let bytes =
                        primary::bytes_of(&self.shared, *ix, *len).unwrap_or_else(|e| log_moved(e));
                    self.reader = Some(log::LogReader::over_bytes(0, bytes));
                }
                // Its own descriptor, not the pool's: a walk holds its log for as long as it takes.
                Walk::File(path, len) => match File::open(path) {
                    Ok(f) => self.reader = Some(log::LogReader::owning(f, 0, *len, Rolling::new())),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => log_moved(e),
                },
            }
        }
    }
}

impl sealed::Sealed for RedbStore {
    fn owned(&self) -> Cow<'_, Store> {
        match self.0.kind {
            Kind::One => {
                let bytes = self.log_bytes();
                let (records, consumed) = from_cbor_seq(&bytes).unwrap_or_else(|e| log_moved(e));
                // With no path, as `Store::from_records` has none: an in-memory copy is for
                // reading, and one that kept the log's path would write to it on `append` — once
                // more after the writer had, which is what `--backend redb retract` did before
                // this said so.
                Cow::Owned(Store::of_log(None, records, &bytes[..consumed]))
            }
            Kind::Many => Cow::Owned(Store::from_records(
                self.records().map(Cow::into_owned).collect(),
            )),
        }
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
    fn attested_units(&self) -> Vec<Uid> {
        self.0
            .uids
            .iter()
            .zip(&self.0.heads)
            .filter(|(_, h)| h.attested > 0)
            .map(|(u, _)| *u)
            .collect()
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
            if reference.starts_with(prefix.as_bytes())
                && (!self.0.subset || self.index_of(&uid).is_some())
            {
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
            .filter(|u| !self.0.subset || self.index_of(u).is_some())
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
        RedbRecords {
            logs: self
                .0
                .logs
                .iter()
                .map(|l| {
                    if l.primary {
                        Walk::Held(l.ix, l.described.log_len)
                    } else {
                        Walk::File(l.path.clone(), l.described.log_len)
                    }
                })
                .collect(),
            shared: std::panic::AssertUnwindSafe(self.0._shared.clone()),
            next: 0,
            reader: None,
            seen: (self.0.kind == Kind::Many).then(BTreeSet::new),
            _store: std::marker::PhantomData,
        }
    }
    fn duplicate_records(&self) -> usize {
        match self.0.kind {
            Kind::One => self.len() - self.hashes().len(),
            // `from_records` keeps one of each.
            Kind::Many => 0,
        }
    }
    fn record_set_digest(&self) -> [u8; 32] {
        digest_of_hashes(self.hashes())
    }
    fn state_hash(&self) -> [u8; 32] {
        state_hash_of(self.units(), &self.0.interp)
    }

    /// Opened for some of its logs, a reference into another that holds the uid waits on it
    /// (`SMY-W500`), as a union of shards given their index says.
    fn report_dangling(&self, report: &mut Report) {
        let g = self.adjacency();
        let holds = self.0.subset.then(|| {
            self.0
                .txn
                .open_table(HOLDS)
                .unwrap_or_else(|e| log_moved(e))
        });
        for n in g.dangling() {
            let Some(uid) = g.uid(n) else { continue };
            let mut waits: Vec<&str> = Vec::new();
            if let Some(t) = &holds {
                let (lo, hi) = log_range(uid.as_bytes());
                for row in t
                    .range::<&[u8; 34]>(&lo..=&hi)
                    .unwrap_or_else(|e| log_moved(e))
                {
                    let ix = log_of(row.unwrap_or_else(|e| log_moved(e)).0.value());
                    if let Some(Some(name)) = self.0.names.get(ix as usize) {
                        if !self.reads(ix) {
                            waits.push(name);
                        }
                    }
                }
            }
            if waits.is_empty() {
                report.push(
                    Diagnostic::new(Code::E060)
                        .with_subject(Subject::Unit(*uid))
                        .with_message("referenced by this store but not present in it"),
                );
            } else {
                report.push(
                    Diagnostic::new(Code::W500)
                        .with_subject(Subject::Unit(*uid))
                        .with_message(format!("waits on shard {}", waits.join(", "))),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests;
