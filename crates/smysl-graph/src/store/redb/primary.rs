//! Primary mode (SMYSL-2.8 §3.5.5, §3.9; TX-P13d): a library shard whose records the unit store
//! holds, and the switch between that and index mode.
//!
//! A primary log has no file. Its records are in `records`, each under the offset it would have
//! had in the file, so nothing else in the store changes when a log becomes primary: a header's
//! offset finds the core in `records` as it found it in the file, and the records in key order
//! are the file's bytes, which is what [`RedbShards::make_index`] writes back.
//!
//! The library's own records — manifests, the routing of a record to its shard — stay in the
//! library's hands; this is the store side of it. One writer at a time: each call here holds the
//! database's fold lock for its one transaction, as a fold does.

use std::fs::File;
use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;

use ::redb::Durability;
use smysl_core::{Error, Rolling};

use super::build::{self, Described, Kind};
use super::log::{self, LogReader};
use super::shared::{self, Shared};
use super::tables::{log_of, records_key, LogRow, LAYOUT, LOGS, RECORDS, RECORD_HASH};
use super::{backend, OpenReport, RedbOptions};

/// What a store describes of one of its logs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LogState {
    pub name: String,
    /// Whether the store holds its records (primary) or describes a file (index).
    pub primary: bool,
    /// The length of its records as one log.
    pub len: u64,
    /// The hash of those bytes in index mode; in primary mode a chain over its appends, which
    /// changes exactly when the log does.
    pub hash: [u8; 32],
    /// Its records, repeats included.
    pub records: u64,
}

/// The writer of a library unit store's primary shards, and the switch between modes.
pub struct RedbShards {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for RedbShards {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedbShards").finish_non_exhaustive()
    }
}

impl RedbShards {
    /// The library unit store at `store`, for writing. Fails at once with `WouldBlock` when
    /// another process holds it, as an open does.
    pub fn open(store: &Path, opts: &RedbOptions) -> Result<RedbShards, Error> {
        if let Some(dir) = store.parent() {
            std::fs::create_dir_all(dir).map_err(Error::Io)?;
        }
        let mut report = OpenReport::default();
        Ok(RedbShards {
            shared: shared::open(store, opts, &mut report)?,
        })
    }

    /// Every log the store describes, in name order.
    pub fn logs(&self) -> Result<Vec<LogState>, Error> {
        Ok(build::meta(&self.shared.db)?
            .map(|m| {
                m.logs
                    .into_iter()
                    .map(|(name, r)| LogState {
                        name,
                        primary: r.primary,
                        len: r.log_len,
                        hash: r.log_hash,
                        records: r.records,
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// What the store describes of one log, by its name: one lookup, not every log's row.
    pub fn log(&self, name: &str) -> Result<Option<LogState>, Error> {
        let txn = self.shared.db.begin_read().map_err(backend)?;
        let logs = match txn.open_table(LOGS) {
            Ok(t) => t,
            Err(::redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(backend(e)),
        };
        Ok(logs.get(name).map_err(backend)?.map(|v| {
            let r = LogRow::decode(v.value());
            LogState {
                name: name.to_string(),
                primary: r.primary,
                len: r.log_len,
                hash: r.log_hash,
                records: r.records,
            }
        }))
    }

    /// A primary log's bytes, as its file would have held them.
    pub fn bytes(&self, name: &str) -> Result<Vec<u8>, Error> {
        let row = self.row(name)?;
        if !row.primary {
            return Err(backend(format!("shard `{name}` is not primary")));
        }
        bytes_of(&self.shared, row.ix, row.log_len)
    }

    /// Append encoded records to a primary log: into the store, in one transaction, with no file.
    /// The caller has decided what is fresh, as the library's routing does.
    pub fn append(&self, name: &str, bytes: &[u8]) -> Result<LogState, Error> {
        let _folding = self.shared.folding();
        let row = self.row(name)?;
        if !row.primary {
            return Err(backend(format!(
                "shard `{name}` is in index mode: append to its log"
            )));
        }
        let from = described(&row);
        let reader = LogReader::over_bytes(row.log_len, bytes.to_vec());
        let (now, folded, _) = build::fold(
            &self.shared.db,
            Kind::Many,
            name,
            row.ix,
            reader,
            from,
            true,
        )?;
        if folded.trailing > 0 {
            return Err(backend(format!(
                "{} bytes after the last complete record of an append to `{name}`",
                folded.trailing
            )));
        }
        Ok(LogState {
            name: name.to_string(),
            primary: true,
            len: now.log_len,
            hash: now.log_hash,
            records: now.records,
        })
    }

    /// Make a log primary: copy its records into the store, under their offsets, and stop
    /// describing the file. The store must describe the whole of `log` already (an open of it
    /// catches up); the caller sets the file aside afterwards. A log already primary is left so.
    pub fn make_primary(&self, name: &str, log: &Path) -> Result<(), Error> {
        let _folding = self.shared.folding();
        let row = self.row(name)?;
        if row.primary {
            return Ok(());
        }
        let file = File::open(log).map_err(Error::Io)?;
        match log::hash_prefix(&file, row.log_len).map_err(Error::Io)? {
            Some(h) if h.finish() == row.log_hash => {}
            _ => {
                return Err(backend(format!(
                    "the unit store does not describe `{name}`'s log as it is; open the \
                     library through the store first"
                )))
            }
        }
        let mut reader = LogReader::new(Some(&file), 0, row.log_len, Rolling::new());
        let mut txn = self.shared.db.begin_write().map_err(backend)?;
        txn.set_durability(Durability::Immediate);
        {
            let mut held = txn.open_table(RECORDS).map_err(backend)?;
            while let Some((offset, _, _)) = reader.next()? {
                held.insert(&records_key(row.ix, offset), reader.raw())
                    .map_err(backend)?;
            }
            let mut logs = txn.open_table(LOGS).map_err(backend)?;
            let primary = LogRow {
                primary: true,
                ..row
            };
            logs.insert(name, &primary.encode()).map_err(backend)?;
        }
        txn.commit().map_err(backend)
    }

    /// Make a primary log an index-mode one again: write its records to `log`, which must not
    /// exist, `fsync`ed, then let the store describe the file and drop the records it held.
    /// The file first, so a crash between the two leaves a store that is still primary beside a
    /// file it does not read, and doing it again finishes the job.
    pub fn make_index(&self, name: &str, log: &Path) -> Result<(), Error> {
        let _folding = self.shared.folding();
        let row = self.row(name)?;
        if !row.primary {
            return Ok(());
        }
        let bytes = {
            let txn = self.shared.db.begin_read().map_err(backend)?;
            let held = txn.open_table(RECORDS).map_err(backend)?;
            let (lo, hi) = (records_key(row.ix, 0), records_key(row.ix, u64::MAX));
            let mut out = Vec::with_capacity(row.log_len as usize);
            for r in held.range::<&[u8; 10]>(&lo..=&hi).map_err(backend)? {
                out.extend_from_slice(r.map_err(backend)?.1.value());
            }
            out
        };
        if let Some(dir) = log.parent() {
            std::fs::create_dir_all(dir).map_err(Error::Io)?;
        }
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(log)
            .map_err(Error::Io)?;
        f.write_all(&bytes).map_err(Error::Io)?;
        f.sync_data().map_err(Error::Io)?;
        let mut hash = Rolling::new();
        hash.update(&bytes);
        let mut txn = self.shared.db.begin_write().map_err(backend)?;
        txn.set_durability(Durability::Immediate);
        {
            let mut held = txn.open_table(RECORDS).map_err(backend)?;
            let (lo, hi) = (records_key(row.ix, 0), records_key(row.ix, u64::MAX));
            held.retain_in::<&[u8; 10], _>(&lo..=&hi, |_, _| false)
                .map_err(backend)?;
            let mut logs = txn.open_table(LOGS).map_err(backend)?;
            let index = LogRow {
                primary: false,
                log_len: bytes.len() as u64,
                log_hash: hash.finish(),
                ..row
            };
            logs.insert(name, &index.encode()).map_err(backend)?;
        }
        txn.commit().map_err(backend)
    }

    /// Undo [`RedbShards::make_primary`] while the log it copied is still where it was and still
    /// what the store describes: drop the records and describe the file again. What a migration
    /// does when its round trip fails, before it has set the log aside.
    pub fn abandon_primary(&self, name: &str, log: &Path) -> Result<(), Error> {
        let _folding = self.shared.folding();
        let row = self.row(name)?;
        if !row.primary {
            return Ok(());
        }
        let file = File::open(log).map_err(Error::Io)?;
        match log::hash_prefix(&file, row.log_len).map_err(Error::Io)? {
            Some(h) if h.finish() == row.log_hash => {}
            _ => {
                return Err(backend(format!(
                    "`{name}`'s log is not the one it was made primary from; not abandoned"
                )))
            }
        }
        let mut txn = self.shared.db.begin_write().map_err(backend)?;
        txn.set_durability(Durability::Immediate);
        {
            let mut held = txn.open_table(RECORDS).map_err(backend)?;
            let (lo, hi) = (records_key(row.ix, 0), records_key(row.ix, u64::MAX));
            held.retain_in::<&[u8; 10], _>(&lo..=&hi, |_, _| false)
                .map_err(backend)?;
            let mut logs = txn.open_table(LOGS).map_err(backend)?;
            let index = LogRow {
                primary: false,
                ..row
            };
            logs.insert(name, &index.encode()).map_err(backend)?;
        }
        txn.commit().map_err(backend)
    }

    /// Every log's record-set digest, from the store's own record hashes, in one pass: what
    /// a shard's export must reproduce (`SMY-E505`).
    pub fn digests(&self) -> Result<std::collections::BTreeMap<String, [u8; 32]>, Error> {
        let names: std::collections::BTreeMap<u16, String> = build::meta(&self.shared.db)?
            .map(|m| m.logs.into_iter().map(|(n, r)| (r.ix, n)).collect())
            .unwrap_or_default();
        let mut hashes: std::collections::BTreeMap<u16, Vec<[u8; 32]>> = Default::default();
        let txn = self.shared.db.begin_read().map_err(backend)?;
        let table = txn.open_table(RECORD_HASH).map_err(backend)?;
        for row in ::redb::ReadableTable::iter(&table).map_err(backend)? {
            let k = *row.map_err(backend)?.0.value();
            let h: [u8; 32] = k[..32].try_into().expect("32 bytes");
            let v = hashes.entry(log_of(&k)).or_default();
            // Keys sort by hash and then log, so one log's hashes arrive ascending and distinct.
            if v.last() != Some(&h) {
                v.push(h);
            }
        }
        Ok(names
            .into_iter()
            .map(|(ix, name)| {
                let hs = hashes.remove(&ix).unwrap_or_default();
                (name, crate::store::digest_of_hashes(&hs))
            })
            .collect())
    }

    /// One log's row, by one lookup: a library reading its primary shards asks once a shard,
    /// and reading every log's row each time was 1,001 rows read 1,001 times (TX-P13d).
    fn row(&self, name: &str) -> Result<LogRow, Error> {
        let txn = self.shared.db.begin_read().map_err(backend)?;
        let layout = match txn.open_table(super::tables::META) {
            Ok(meta) => meta
                .get("layout")
                .map_err(backend)?
                .and_then(|v| <[u8; 2]>::try_from(v.value()).ok().map(u16::from_be_bytes)),
            Err(::redb::TableError::TableDoesNotExist(_)) => None,
            Err(e) => return Err(backend(e)),
        };
        match layout {
            None => return Err(backend("the unit store has never been built")),
            Some(l) if l != LAYOUT => {
                return Err(backend(
                    "the unit store is in another layout; open it first",
                ))
            }
            Some(_) => {}
        }
        let logs = txn.open_table(LOGS).map_err(backend)?;
        logs.get(name)
            .map_err(backend)?
            .map(|v| LogRow::decode(v.value()))
            .ok_or_else(|| backend(format!("the unit store describes no shard `{name}`")))
    }
}

fn described(row: &LogRow) -> Described {
    Described {
        layout: LAYOUT,
        log_len: row.log_len,
        log_hash: row.log_hash,
        records: row.records,
    }
}

/// A primary log's first `len` bytes in offset order, read out of `records`: only the records
/// that start before `len`, so a reader of a snapshot is not shown what was appended after it.
pub(super) fn bytes_of(shared: &Shared, ix: u16, len: u64) -> Result<Vec<u8>, Error> {
    let txn = shared.db.begin_read().map_err(backend)?;
    let held = txn.open_table(RECORDS).map_err(backend)?;
    let (lo, hi) = (records_key(ix, 0), records_key(ix, len));
    let mut out = Vec::with_capacity(len as usize);
    for r in held.range::<&[u8; 10]>(&lo..&hi).map_err(backend)? {
        out.extend_from_slice(r.map_err(backend)?.1.value());
    }
    Ok(out)
}
