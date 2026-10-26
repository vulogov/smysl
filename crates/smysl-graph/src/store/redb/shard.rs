//! The unit store's writer (SMYSL-2.8 §3.5.5, §3.8; TX-P13b step 2).
//!
//! A [`RedbShard`] is the one writer of a log and its unit store. It holds the shard's lock
//! file for its life — a second writer, in this process or another, fails with `SMY-E445`
//! naming it — and writes in index mode's order: the log, `fsync`ed, then one redb transaction
//! carrying the new `log_len` and `log_hash`. A crash between the two leaves the store behind
//! its log, and the next open folds the tail.
//!
//! Readers are [`RedbStore`] snapshots taken from it with [`RedbShard::reader`]. They share its
//! database, so in one process they neither block it nor are blocked by it: each answers from
//! the moment it was taken while the shard goes on appending.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use smysl_core::diag::{Code, Diagnostic};
use smysl_core::{hash_bytes, to_cbor, Error, Record, Rolling};

use super::build::{self, Described, Kind};
use super::shared::{self, Shared};
use super::tables::{with_log, LogIx, RECORD_HASH};
use super::{backend, is_held, RedbOptions, RedbStore, ONE};
use crate::lock::Lock;
use crate::store::{refuse_unloggable, AppendReport, OpenReport};

/// The writer of one log and its unit store.
pub struct RedbShard {
    shared: Arc<Shared>,
    log: File,
    log_path: PathBuf,
    /// The log's position in its store: always the first, a store of one log having one.
    ix: LogIx,
    described: Described,
    /// The log's hash up to `described.log_len`, kept open so an append hashes what it adds.
    hasher: Rolling,
    opts: RedbOptions,
    // Last, so the lock is released after everything it guards is closed.
    _lock: Lock,
}

impl std::fmt::Debug for RedbShard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedbShard")
            .field("records", &self.described.records)
            .field("log_len", &self.described.log_len)
            .finish_non_exhaustive()
    }
}

impl RedbShard {
    /// The writer's lock file: `.smysl/store/<name>.lock`, beside the store (§3.5.2).
    pub fn lock_path(log: &Path) -> PathBuf {
        RedbStore::store_path(log).with_extension("lock")
    }

    /// Open a log and its unit store for writing, creating either if it does not exist.
    ///
    /// Takes the lock first, so a second writer fails with `SMY-E445` before it touches
    /// anything. Then waits up to `reader_wait_ms` for a store another process is reading,
    /// and fails with a `WouldBlock` error if it is still held: unlike a reader, a writer has
    /// nothing to fall back to that would leave the store describing its log.
    ///
    /// A torn record at the end of the log — a writer killed mid-append, whose lock somebody
    /// then broke — is cut off (`SMY-W110` says how many bytes), because a record appended
    /// after it could never be read back.
    pub fn open_rw(
        log: impl AsRef<Path>,
        opts: RedbOptions,
    ) -> Result<(RedbShard, OpenReport), Error> {
        let log_path = log.as_ref().to_path_buf();
        let path = RedbStore::store_path(&log_path);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(Error::Io)?;
        }
        let name = log_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "store".into());
        let lock =
            Lock::acquire(&Self::lock_path(&log_path), &name, &opts.command).map_err(Error::Lib)?;

        let mut report = OpenReport::default();
        let deadline = Instant::now() + Duration::from_millis(opts.reader_wait_ms);
        let shared = loop {
            match shared::open(&path, &opts, &mut report) {
                Ok(s) => break s,
                Err(e) if is_held(&e) && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => return Err(e),
            }
        };
        if let Some(dir) = log_path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir).map_err(Error::Io)?;
            }
        }
        let log = std::fs::OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&log_path)
            .map_err(Error::Io)?;
        let mut caught = OpenReport::default();
        let (ix, described, hasher) = {
            let _folding = shared.folding();
            let given = [build::Given {
                name: ONE,
                path: &log_path,
                file: Some(&log),
            }];
            let c = build::catch_up(
                &shared.db,
                Kind::One,
                &given,
                opts.force_rebuild,
                &mut caught,
            )?
            .remove(0);
            (c.ix, c.described, c.hasher)
        };
        let mut shard = RedbShard {
            shared,
            log,
            log_path,
            ix,
            described,
            hasher,
            opts,
            _lock: lock,
        };
        if caught.trailing_bytes > 0 {
            shard.cut_tail()?;
        }
        for d in caught.report.iter() {
            report.report.push(if d.code == Code::W110 {
                Diagnostic::new(Code::W110).with_message(format!(
                    "{} trailing bytes after the last complete record; cut, so that what is \
                     appended next can be read",
                    caught.trailing_bytes
                ))
            } else {
                d.clone()
            });
        }
        report.index_rebuilt |= caught.index_rebuilt;
        report.replayed = caught.replayed;
        report.trailing_bytes = caught.trailing_bytes;
        Ok((shard, report))
    }

    /// Append records: those not already in the store, once each, to the log and then to the
    /// store. The same refusals as `Store::append`, before anything is written.
    pub fn append(&mut self, records: &[Record]) -> Result<AppendReport, Error> {
        refuse_unloggable(records)?;
        let shared = self.shared.clone();
        let _folding = shared.folding();
        self.settle()?;

        let mut report = AppendReport::default();
        let mut bytes = Vec::new();
        let mut seen: BTreeSet<[u8; 32]> = BTreeSet::new();
        {
            let txn = shared.db.begin_read().map_err(backend)?;
            let hashes = txn.open_table(RECORD_HASH).map_err(backend)?;
            for r in records {
                let encoded = to_cbor(r);
                let hash = hash_bytes(&encoded);
                if hashes
                    .get(&with_log(&hash, self.ix))
                    .map_err(backend)?
                    .is_some()
                    || !seen.insert(hash)
                {
                    report.duplicates += 1;
                    continue;
                }
                bytes.extend_from_slice(&encoded);
                report.added += 1;
            }
        }
        if bytes.is_empty() {
            return Ok(report);
        }

        // The log, durably, before the store says it holds any of it.
        self.log.write_all(&bytes).map_err(Error::Io)?;
        self.log.sync_data().map_err(Error::Io)?;
        report.bytes_written = bytes.len() as u64;
        let end = self.described.log_len + report.bytes_written;
        let (now, _, hasher) = build::fold(
            &shared.db,
            Kind::One,
            ONE,
            self.ix,
            Some(&self.log),
            self.described,
            end,
            self.hasher.clone(),
        )?;
        self.described = now;
        self.hasher = hasher;
        Ok(report)
    }

    /// A snapshot of the store as it is now. Later appends do not change what it answers.
    pub fn reader(&self) -> Result<RedbStore, Error> {
        let mut paths = vec![None; self.ix as usize + 1];
        paths[self.ix as usize] = Some(self.log_path.clone());
        RedbStore::load(
            self.shared.clone(),
            Kind::One,
            vec![super::LogFile {
                name: ONE.to_string(),
                ix: self.ix,
                path: self.log_path.clone(),
                described: self.described,
            }],
            paths,
            false,
            &self.opts,
        )
    }

    /// Records in the log, duplicates included, as `Store::len`.
    pub fn len(&self) -> usize {
        self.described.records as usize
    }

    pub fn is_empty(&self) -> bool {
        self.described.records == 0
    }

    /// The length of the log in bytes.
    pub fn log_len(&self) -> u64 {
        self.described.log_len
    }

    /// The hash of the log, as `Store::log_hash`.
    pub fn log_hash(&self) -> &[u8; 32] {
        &self.described.log_hash
    }

    pub fn log_path(&self) -> &Path {
        &self.log_path
    }

    /// Bring this writer up to the log and the store before writing to either.
    ///
    /// Two things can have moved since the last append. A reader's open in this process can
    /// have folded a tail into the store, so the store describes more than this writer
    /// remembers; and the log can hold records the store does not — an append whose fold
    /// failed after the log write, or a writer that does not take this lock (a `Store` opened
    /// on the same log). Both are a tail to read, never a rebuild: the store and the log
    /// agreed at open, and both only grow.
    fn settle(&mut self) -> Result<(), Error> {
        if let Some((_, d)) = build::described(&self.shared.db, ONE)? {
            if d.log_len > self.described.log_len {
                let more = super::log::read_at(
                    &self.log,
                    self.described.log_len,
                    (d.log_len - self.described.log_len) as usize,
                )
                .map_err(Error::Io)?;
                self.hasher.update(&more);
                self.described = d;
            }
        }
        let end = self.log.metadata().map_err(Error::Io)?.len();
        if end > self.described.log_len {
            let (now, folded, hasher) = build::fold(
                &self.shared.db,
                Kind::One,
                ONE,
                self.ix,
                Some(&self.log),
                self.described,
                end,
                self.hasher.clone(),
            )?;
            self.described = now;
            self.hasher = hasher;
            // Not cut, as `open_rw` cuts one: holding the lock, this writer did not write
            // these bytes, so somebody is appending without it — possibly at this moment.
            if folded.trailing > 0 {
                return Err(Error::Io(std::io::Error::other(format!(
                    "the log has {} bytes after its last complete record that this writer did \
                     not write: something is appending to it without the shard's lock",
                    folded.trailing
                ))));
            }
        }
        Ok(())
    }

    /// Cut the log back to the end of its last complete record.
    fn cut_tail(&mut self) -> Result<(), Error> {
        self.log
            .set_len(self.described.log_len)
            .and_then(|()| self.log.sync_data())
            .map_err(Error::Io)
    }
}
