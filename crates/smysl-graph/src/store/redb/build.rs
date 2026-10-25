//! Folding log records into the unit store's tables (SMYSL-2.8 §3.5.5).
//!
//! One fold for both of index mode's writes: a rebuild is the fold of the whole log into empty
//! tables, and catching up is the fold of the tail the store does not describe yet. The fold is
//! the store's `absorb` restated over tables — a unit's first core wins, an attestation is kept
//! under its subject whether or not the subject has arrived, every other record goes to
//! `interp` — so the two cannot drift: the conformance suite compares what each produces
//! against `Store::open` of the same log.

use ::redb::{Database, Durability, ReadableTable, TableHandle};
use smysl_core::diag::{Code, Diagnostic};
use smysl_core::{canonical_uid, Error, LibError, Record, Rolling};

use super::log::{self, LogReader};
use super::tables::{
    attest_key, attest_range, deps_key, observed_key, source_key, Header, ATTEST, DEPS, HEADER,
    INTERP, LAYOUT, META, OBSERVED, RECORD_HASH, SOURCE_REF,
};
use super::{backend, OpenReport};
use crate::Store;

/// What a store's `meta` says it describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Described {
    pub(super) layout: u16,
    pub(super) log_len: u64,
    pub(super) log_hash: [u8; 32],
    /// Records in the log up to `log_len`, duplicates included.
    pub(super) records: u64,
}

impl Described {
    pub(super) fn empty() -> Described {
        Described {
            layout: LAYOUT,
            log_len: 0,
            log_hash: Rolling::new().finish(),
            records: 0,
        }
    }
}

/// What `meta` says, or `None` for a store that has never been written.
pub(super) fn described(db: &Database) -> Result<Option<Described>, Error> {
    let txn = db.begin_read().map_err(backend)?;
    let meta = match txn.open_table(META) {
        Ok(t) => t,
        Err(::redb::TableError::TableDoesNotExist(_)) => return Ok(None),
        Err(e) => return Err(backend(e)),
    };
    let get = |k: &str| -> Result<Option<Vec<u8>>, Error> {
        Ok(meta.get(k).map_err(backend)?.map(|v| v.value().to_vec()))
    };
    let Some(layout) = get("layout")? else {
        return Ok(None);
    };
    let fixed = |v: Option<Vec<u8>>, what: &str| -> Result<Vec<u8>, Error> {
        v.ok_or_else(|| backend(format!("meta has a layout and no {what}")))
    };
    let layout = u16::from_be_bytes(
        layout
            .as_slice()
            .try_into()
            .map_err(|_| backend("meta.layout is not two bytes"))?,
    );
    let n = |v: Vec<u8>, what: &str| -> Result<u64, Error> {
        Ok(u64::from_be_bytes(v.as_slice().try_into().map_err(
            |_| backend(format!("meta.{what} is not eight bytes")),
        )?))
    };
    let log_len = n(fixed(get("log_len")?, "log_len")?, "log_len")?;
    let records = n(fixed(get("records")?, "records")?, "records")?;
    let log_hash: [u8; 32] = fixed(get("log_hash")?, "log_hash")?
        .as_slice()
        .try_into()
        .map_err(|_| backend("meta.log_hash is not 32 bytes"))?;
    Ok(Some(Described {
        layout,
        log_len,
        log_hash,
        records,
    }))
}

/// Empty every table, so the next fold is a rebuild.
pub(super) fn clear(db: &Database) -> Result<(), Error> {
    let txn = db.begin_write().map_err(backend)?;
    let names: Vec<String> = txn
        .list_tables()
        .map_err(backend)?
        .map(|t| t.name().to_string())
        .collect();
    for name in names {
        txn.delete_table(::redb::TableDefinition::<&[u8], &[u8]>::new(&name))
            .map_err(backend)?;
    }
    txn.commit().map_err(backend)
}

/// What a fold did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Folded {
    pub(super) records: u64,
    pub(super) trailing: u64,
}

/// Bring the store up to its log: nothing if it describes the log already, the tail if the log
/// has grown, a rebuild if it describes some other log, an older layout, or nothing.
///
/// Returns what the store now describes and the log's hash up to there, still open. The
/// caller holds the database's fold lock.
pub(super) fn catch_up(
    db: &Database,
    log: Option<&std::fs::File>,
    force_rebuild: bool,
    report: &mut OpenReport,
) -> Result<(Described, Rolling), Error> {
    let end = match log {
        Some(f) => f.metadata().map_err(Error::Io)?.len(),
        None => 0,
    };
    // Current, behind, or not this log's at all.
    let held_now = described(db)?;
    let mut rebuild = None;
    let mut hasher = Rolling::new();
    match held_now {
        None => rebuild = Some("no unit store described this log; built it from the log"),
        Some(d) if d.layout > LAYOUT => {
            return Err(Error::Lib(LibError::StoreLayout {
                found: d.layout,
                supported: LAYOUT,
            }))
        }
        Some(d) if d.layout < LAYOUT => {
            rebuild = Some("the unit store is in an older layout; rebuilt from the log")
        }
        Some(_) if force_rebuild => rebuild = Some("rebuilt from the log, as asked"),
        Some(d) => {
            let prefix = match log {
                Some(f) => log::hash_prefix(f, d.log_len).map_err(Error::Io)?,
                None if d.log_len == 0 => Some(Rolling::new()),
                None => None,
            };
            match prefix {
                Some(h) if h.finish() == d.log_hash => hasher = h,
                _ => {
                    rebuild = Some(
                        "the unit store does not describe this log (its length or hash \
                         differs); rebuilt from the log",
                    )
                }
            }
        }
    }
    let from = match (rebuild, held_now) {
        (Some(why), held) => {
            if held.is_some() {
                clear(db)?;
            }
            report
                .report
                .push(Diagnostic::new(Code::W501).with_message(why));
            report.index_rebuilt = true;
            Described::empty()
        }
        (None, Some(d)) => d,
        (None, None) => unreachable!("a store with no description is rebuilt"),
    };
    if rebuild.is_none() && end <= from.log_len {
        return Ok((from, hasher));
    }
    let (now, folded, hasher) = fold(db, log, from, end, hasher)?;
    report.replayed += folded.records;
    report.trailing_bytes = folded.trailing;
    if folded.trailing > 0 {
        report
            .report
            .push(Diagnostic::new(Code::W110).with_message(format!(
                "{} trailing bytes after the last complete record",
                folded.trailing
            )));
    }
    Ok((now, hasher))
}

/// Fold the log's complete records after `from` into the tables, in one transaction, and
/// record that the store now describes the log up to the last of them.
///
/// `hasher` holds the hash of the log before `from.log_len`. The transaction commits with
/// `Durability::Immediate`: the log was written first, so a crash leaves the store behind its
/// log, never ahead of it, and behind is a tail to fold again.
pub(super) fn fold(
    db: &Database,
    log: Option<&std::fs::File>,
    from: Described,
    end: u64,
    hasher: Rolling,
) -> Result<(Described, Folded, Rolling), Error> {
    let mut reader = LogReader::new(log, from.log_len, end, hasher);
    let mut txn = db.begin_write().map_err(backend)?;
    txn.set_durability(Durability::Immediate);
    let mut seq = from.records;
    {
        let mut hashes = txn.open_table(RECORD_HASH).map_err(backend)?;
        let mut header = txn.open_table(HEADER).map_err(backend)?;
        let mut deps = txn.open_table(DEPS).map_err(backend)?;
        let mut attest = txn.open_table(ATTEST).map_err(backend)?;
        let mut interp = txn.open_table(INTERP).map_err(backend)?;
        let mut observed = txn.open_table(OBSERVED).map_err(backend)?;
        let mut source = txn.open_table(SOURCE_REF).map_err(backend)?;

        while let Some((offset, len, record)) = reader.next()? {
            let hash = Store::record_hash(&record);
            let fresh = hashes.get(&hash).map_err(backend)?.is_none();
            if fresh {
                hashes.insert(&hash, seq).map_err(backend)?;
            }
            match &record {
                // A unit's first core wins, as `absorb`'s `or_insert` keeps it; a repeat
                // changes nothing.
                Record::Unit(core) if fresh => {
                    let uid = canonical_uid(core);
                    if header.get(uid.as_bytes()).map_err(backend)?.is_none() {
                        // Attestations that reached the log before the unit did.
                        let (lo, hi) = attest_range(&uid);
                        let mut attested = 0u32;
                        let mut hop = 0u32;
                        for row in attest.range::<&[u8; 64]>(&lo..=&hi).map_err(backend)? {
                            let (_, v) = row.map_err(backend)?;
                            if let Record::Attestation(a) = smysl_core::from_cbor(v.value())?.0 {
                                attested += 1;
                                hop = hop.max(a.hop);
                            }
                        }
                        let observed_ms = core.source.as_ref().and_then(|s| s.observed);
                        let h = Header {
                            offset,
                            len,
                            status: core.status.as_u8(),
                            observed: observed_ms,
                            attested,
                            hop,
                        };
                        header
                            .insert(uid.as_bytes(), &h.encode())
                            .map_err(backend)?;
                        for d in &core.deps {
                            deps.insert(&deps_key(&uid, false, d), ())
                                .map_err(backend)?;
                        }
                        for g in &core.grounds {
                            deps.insert(&deps_key(&uid, true, g), ()).map_err(backend)?;
                        }
                        if let Some(ms) = observed_ms {
                            observed
                                .insert(&observed_key(ms, &uid), ())
                                .map_err(backend)?;
                        }
                        if let Some(s) = &core.source {
                            source
                                .insert(source_key(&s.reference, &uid).as_slice(), ())
                                .map_err(backend)?;
                        }
                    }
                }
                Record::Unit(_) => {}
                // Kept under its subject whatever the subject turns out to be. A unit's count
                // and newest hop move now if the unit is here; if it arrives later, its header
                // is written with the attestations already waiting.
                Record::Attestation(a) if fresh => {
                    attest
                        .insert(&attest_key(&a.uid, &hash), reader.raw())
                        .map_err(backend)?;
                    let held = header
                        .get(a.uid.as_bytes())
                        .map_err(backend)?
                        .map(|v| Header::decode(v.value()));
                    if let Some(mut h) = held {
                        h.hop = if h.attested == 0 {
                            a.hop
                        } else {
                            h.hop.max(a.hop)
                        };
                        h.attested += 1;
                        header
                            .insert(a.uid.as_bytes(), &h.encode())
                            .map_err(backend)?;
                    }
                }
                Record::Attestation(_) => {}
                // Repeats included: `Store::open` keeps a log as it is, and a hot record given
                // twice is listed twice.
                _ => {
                    interp.insert(seq, reader.raw()).map_err(backend)?;
                }
            }
            seq += 1;
        }
    }
    let now = Described {
        layout: LAYOUT,
        log_len: reader.consumed(),
        log_hash: reader.hash(),
        records: seq,
    };
    {
        let mut meta = txn.open_table(META).map_err(backend)?;
        meta.insert("layout", LAYOUT.to_be_bytes().as_slice())
            .map_err(backend)?;
        meta.insert("mode", b"index".as_slice()).map_err(backend)?;
        meta.insert("log_len", now.log_len.to_be_bytes().as_slice())
            .map_err(backend)?;
        meta.insert("log_hash", now.log_hash.as_slice())
            .map_err(backend)?;
        meta.insert("records", now.records.to_be_bytes().as_slice())
            .map_err(backend)?;
    }
    txn.commit().map_err(backend)?;
    let folded = Folded {
        records: seq - from.records,
        trailing: reader.trailing(),
    };
    Ok((now, folded, reader.into_hasher()))
}
