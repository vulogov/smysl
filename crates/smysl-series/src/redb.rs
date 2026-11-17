//! The substrate's tables in redb: `<library>/index/series.redb` (RFC SMYSL-3 §19.3).

use std::path::Path;

use ::redb::{Database, ReadableTable, TableDefinition};
use smysl_core::Uid;

use crate::store::{chunk_key, key_inst, Backend, ChunkKey};
use crate::Error;

const CHUNKS: TableDefinition<'_, &[u8; 72], &[u8]> = TableDefinition::new("chunks");
const SERIES: TableDefinition<'_, &[u8; 32], &[u8]> = TableDefinition::new("series");
const EVICTED: TableDefinition<'_, &[u8; 72], u64> = TableDefinition::new("evicted");
const META: TableDefinition<'_, &str, &[u8]> = TableDefinition::new("meta");

/// The `meta` table's format version (§19.3).
pub const FORMAT: &[u8] = b"smysl-series/1";

fn backend(e: impl std::fmt::Display) -> Error {
    Error::Backend(e.to_string())
}

/// The four tables in one redb file.
#[derive(Debug)]
pub struct RedbBackend {
    db: Database,
}

impl RedbBackend {
    /// Open the file at `path`, creating it with its four tables if it is not there, and refuse
    /// one written in another format.
    ///
    /// A file already in this format is opened with a read transaction only. A write
    /// transaction commits, and a commit is a durable flush: `tel at` paid one for every
    /// question, which was most of its time.
    pub fn open(path: &Path) -> Result<RedbBackend, Error> {
        let db = Database::create(path).map_err(backend)?;
        {
            let txn = db.begin_read().map_err(backend)?;
            match txn.open_table(META) {
                Ok(meta) => {
                    if let Some(f) = meta.get("format").map_err(backend)? {
                        if f.value() != FORMAT {
                            return Err(Error::Backend(format!(
                                "{} is series format `{}`, not `{}`",
                                path.display(),
                                String::from_utf8_lossy(f.value()),
                                String::from_utf8_lossy(FORMAT)
                            )));
                        }
                        return Ok(RedbBackend { db });
                    }
                }
                Err(::redb::TableError::TableDoesNotExist(_)) => {}
                Err(e) => return Err(backend(e)),
            }
        }
        let txn = db.begin_write().map_err(backend)?;
        {
            let mut meta = txn.open_table(META).map_err(backend)?;
            let found = meta
                .get("format")
                .map_err(backend)?
                .map(|v| v.value().to_vec());
            match found {
                Some(f) if f != FORMAT => {
                    return Err(Error::Backend(format!(
                        "{} is series format `{}`, not `{}`",
                        path.display(),
                        String::from_utf8_lossy(&f),
                        String::from_utf8_lossy(FORMAT)
                    )))
                }
                Some(_) => {}
                None => {
                    meta.insert("format", FORMAT).map_err(backend)?;
                }
            }
            txn.open_table(CHUNKS).map_err(backend)?;
            txn.open_table(SERIES).map_err(backend)?;
            txn.open_table(EVICTED).map_err(backend)?;
        }
        txn.commit().map_err(backend)?;
        Ok(RedbBackend { db })
    }
}

impl Backend for RedbBackend {
    fn chunk(&self, key: &ChunkKey) -> Result<Option<Vec<u8>>, Error> {
        let txn = self.db.begin_read().map_err(backend)?;
        let t = txn.open_table(CHUNKS).map_err(backend)?;
        Ok(t.get(key).map_err(backend)?.map(|v| v.value().to_vec()))
    }

    fn chunks(
        &self,
        series: &Uid,
        inst: &Uid,
        lo: u64,
        hi: u64,
    ) -> Result<Vec<(u64, Vec<u8>)>, Error> {
        let txn = self.db.begin_read().map_err(backend)?;
        let t = txn.open_table(CHUNKS).map_err(backend)?;
        let (lo, hi) = (chunk_key(series, inst, lo), chunk_key(series, inst, hi));
        let mut out = Vec::new();
        for row in t.range::<&[u8; 72]>(&lo..=&hi).map_err(backend)? {
            let (k, v) = row.map_err(backend)?;
            out.push((start(k.value()), v.value().to_vec()));
        }
        Ok(out)
    }

    fn chunk_before(
        &self,
        series: &Uid,
        inst: &Uid,
        before: u64,
    ) -> Result<Option<(u64, Vec<u8>)>, Error> {
        let txn = self.db.begin_read().map_err(backend)?;
        let t = txn.open_table(CHUNKS).map_err(backend)?;
        let (lo, hi) = (chunk_key(series, inst, 0), chunk_key(series, inst, before));
        let last = t.range::<&[u8; 72]>(&lo..&hi).map_err(backend)?.next_back();
        match last {
            Some(row) => {
                let (k, v) = row.map_err(backend)?;
                Ok(Some((start(k.value()), v.value().to_vec())))
            }
            None => Ok(None),
        }
    }

    fn instruments(&self, series: &Uid) -> Result<Vec<Uid>, Error> {
        use core::ops::Bound::{Excluded, Included};
        let txn = self.db.begin_read().map_err(backend)?;
        let t = txn.open_table(CHUNKS).map_err(backend)?;
        let end = chunk_key(series, &Uid::from_bytes([0xff; 32]), u64::MAX);
        let mut out = Vec::new();
        let mut from = Included(chunk_key(series, &Uid::from_bytes([0; 32]), 0));
        // One seek per instrument, not one step per chunk.
        loop {
            let next = t
                .range::<&[u8; 72]>((from.as_ref(), Included(&end)))
                .map_err(backend)?
                .next();
            let Some(row) = next else { break };
            let inst = key_inst(row.map_err(backend)?.0.value());
            out.push(inst);
            from = Excluded(chunk_key(series, &inst, u64::MAX));
        }
        Ok(out)
    }

    fn evicted(&self, series: &Uid, inst: &Uid, lo: u64, hi: u64) -> Result<Vec<u64>, Error> {
        let txn = self.db.begin_read().map_err(backend)?;
        let t = txn.open_table(EVICTED).map_err(backend)?;
        let (lo, hi) = (chunk_key(series, inst, lo), chunk_key(series, inst, hi));
        let mut out = Vec::new();
        for row in t.range::<&[u8; 72]>(&lo..=&hi).map_err(backend)? {
            out.push(start(row.map_err(backend)?.0.value()));
        }
        Ok(out)
    }

    fn row(&self, series: &Uid) -> Result<Option<Vec<u8>>, Error> {
        let txn = self.db.begin_read().map_err(backend)?;
        let t = txn.open_table(SERIES).map_err(backend)?;
        Ok(t.get(series.as_bytes())
            .map_err(backend)?
            .map(|v| v.value().to_vec()))
    }

    fn rows(&self) -> Result<Vec<(Uid, Vec<u8>)>, Error> {
        let txn = self.db.begin_read().map_err(backend)?;
        let t = txn.open_table(SERIES).map_err(backend)?;
        let mut out = Vec::new();
        for row in t.iter().map_err(backend)? {
            let (k, v) = row.map_err(backend)?;
            out.push((Uid::from_bytes(*k.value()), v.value().to_vec()));
        }
        Ok(out)
    }

    fn write(
        &mut self,
        chunks: Vec<(ChunkKey, Vec<u8>)>,
        rows: Vec<(Uid, Vec<u8>)>,
    ) -> Result<(), Error> {
        let txn = self.db.begin_write().map_err(backend)?;
        {
            let mut c = txn.open_table(CHUNKS).map_err(backend)?;
            for (k, v) in &chunks {
                c.insert(k, v.as_slice()).map_err(backend)?;
            }
            let mut s = txn.open_table(SERIES).map_err(backend)?;
            for (u, v) in &rows {
                s.insert(u.as_bytes(), v.as_slice()).map_err(backend)?;
            }
        }
        txn.commit().map_err(backend)
    }
}

fn start(k: &[u8; 72]) -> u64 {
    u64::from_be_bytes(k[64..].try_into().expect("eight bytes"))
}
