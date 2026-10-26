//! Folding log records into the unit store's tables (SMYSL-2.8 §3.5.5).
//!
//! One fold for both of index mode's writes: a rebuild is the fold of every log into empty
//! tables, and catching up is the fold of the tail of each log the store does not describe yet.
//! The fold is the store's `absorb` restated over tables — a unit's first core wins, an
//! attestation is kept under its subject whether or not the subject has arrived, every other
//! record goes to `interp` — so the two cannot drift: the conformance suite compares what each
//! produces against `Store::open` of the same log, and a store of several logs against
//! `Store::from_records` of theirs.
//!
//! **Several logs** (TX-P13c step 3, OQ-90). A library's shards are one store: every row says
//! which log it came from, a log is folded on its own, and what the store answers — the logs in
//! name order, each record once — is decided when it is read (`load`), not when it is written,
//! so a log that grows does not move the rows of any other.

use std::collections::BTreeMap;

use ::redb::{Database, Durability, ReadableTable, TableHandle};
use smysl_core::diag::{Code, Diagnostic};
use smysl_core::{canonical_uid, Error, LibError, Record, Rolling};

use super::log::{self, LogReader};
use super::tables::{
    attest_key, attest_range, deps_key, interp_key, log_range, observed_key, source_key, with_log,
    Header, LogIx, LogRow, ATTEST, DEPS, HEADER, HOLDS, INTERP, LAYOUT, LOGS, META, OBSERVED,
    RECORD_HASH, SOURCE_REF,
};
use super::{backend, OpenReport};
use crate::Store;

/// What a store says it describes of one log.
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

/// One store over one log (`Store::open`'s answer, repeats kept), or over several (the answer
/// of `Store::from_records` of all their records in name order, each record once).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    One,
    Many,
}

impl Kind {
    fn tag(self) -> &'static [u8] {
        match self {
            Kind::One => b"one",
            Kind::Many => b"many",
        }
    }
}

/// What a store says of itself: its layout and kind, and what it describes of each log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Meta {
    pub(super) layout: u16,
    pub(super) kind: Option<Vec<u8>>,
    pub(super) logs: BTreeMap<String, LogRow>,
}

/// What `meta` and `logs` say, or `None` for a store that has never been written.
pub(super) fn meta(db: &Database) -> Result<Option<Meta>, Error> {
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
    let layout = u16::from_be_bytes(
        layout
            .as_slice()
            .try_into()
            .map_err(|_| backend("meta.layout is not two bytes"))?,
    );
    let mut logs = BTreeMap::new();
    // A store of an older layout has no `logs`; it is rebuilt, and needs nothing more read.
    if layout == LAYOUT {
        match txn.open_table(LOGS) {
            Ok(t) => {
                for row in t.iter().map_err(backend)? {
                    let (k, v) = row.map_err(backend)?;
                    logs.insert(k.value().to_string(), LogRow::decode(v.value()));
                }
            }
            Err(::redb::TableError::TableDoesNotExist(_)) => {}
            Err(e) => return Err(backend(e)),
        }
    }
    Ok(Some(Meta {
        layout,
        kind: get("logs")?,
        logs,
    }))
}

/// What the store describes of the log called `name`, if anything.
pub(super) fn described(db: &Database, name: &str) -> Result<Option<(LogIx, Described)>, Error> {
    Ok(meta(db)?.and_then(|m| {
        m.logs.get(name).map(|r| {
            (
                r.ix,
                Described {
                    layout: m.layout,
                    log_len: r.log_len,
                    log_hash: r.log_hash,
                    records: r.records,
                },
            )
        })
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

/// A log as a catch-up is given it: its name, and the file open already (a writer's) or a path
/// to open it from, one log at a time — a store of a thousand logs must not need a thousand
/// descriptors at once.
pub(super) struct Given<'f> {
    pub(super) name: &'f str,
    pub(super) path: &'f std::path::Path,
    pub(super) file: Option<&'f std::fs::File>,
}

impl Given<'_> {
    /// The log, open; `None` if it does not exist yet, which reads as empty.
    fn open(&self) -> Result<Option<Held<'_>>, Error> {
        if let Some(f) = self.file {
            return Ok(Some(Held::Borrowed(f)));
        }
        match std::fs::File::open(self.path) {
            Ok(f) => Ok(Some(Held::Owned(f))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::Io(e)),
        }
    }
}

/// A log open for a catch-up: the writer's own, or one opened for it and closed after.
enum Held<'f> {
    Borrowed(&'f std::fs::File),
    Owned(std::fs::File),
}

impl std::ops::Deref for Held<'_> {
    type Target = std::fs::File;
    fn deref(&self) -> &std::fs::File {
        match self {
            Held::Borrowed(f) => f,
            Held::Owned(f) => f,
        }
    }
}

/// A log as the store now describes it, after a catch-up: its position, what is described, and
/// the hash up to there, still open.
pub(super) struct CaughtUp {
    pub(super) ix: LogIx,
    pub(super) described: Described,
    pub(super) hasher: Rolling,
}

/// Bring the store up to its logs: nothing for a log it describes already, the tail of one that
/// has grown, a rebuild of all of them if it describes some other log, a log it was not given,
/// an older layout, another kind, or nothing.
///
/// Returns each given log's [`CaughtUp`], in the order given. The caller holds the database's
/// fold lock.
pub(super) fn catch_up(
    db: &Database,
    kind: Kind,
    logs: &[Given<'_>],
    force_rebuild: bool,
    report: &mut OpenReport,
) -> Result<Vec<CaughtUp>, Error> {
    let held_now = meta(db)?;
    let mut rebuild = None;
    let mut hashers: BTreeMap<&str, Rolling> = BTreeMap::new();
    match &held_now {
        None => rebuild = Some("no unit store described this log; built it from the log"),
        Some(m) if m.layout > LAYOUT => {
            return Err(Error::Lib(LibError::StoreLayout {
                found: m.layout,
                supported: LAYOUT,
            }))
        }
        Some(m) if m.layout < LAYOUT => {
            rebuild = Some("the unit store is in an older layout; rebuilt from the log")
        }
        Some(_) if force_rebuild => rebuild = Some("rebuilt from the log, as asked"),
        Some(m) if m.kind.as_deref() != Some(kind.tag()) => {
            rebuild = Some("the unit store was built for another set of logs; rebuilt")
        }
        Some(m) if m.logs.keys().any(|n| !logs.iter().any(|g| g.name == n)) => {
            rebuild = Some("the unit store describes a log it was not given; rebuilt")
        }
        Some(m) => {
            for g in logs {
                let Some(row) = m.logs.get(g.name) else {
                    continue;
                };
                let prefix = match g.open()? {
                    Some(f) => log::hash_prefix(&f, row.log_len).map_err(Error::Io)?,
                    None if row.log_len == 0 => Some(Rolling::new()),
                    None => None,
                };
                match prefix {
                    Some(h) if h.finish() == row.log_hash => {
                        hashers.insert(g.name, h);
                    }
                    _ => {
                        rebuild = Some(
                            "the unit store does not describe this log (its length or hash \
                             differs); rebuilt from the log",
                        );
                        break;
                    }
                }
            }
        }
    }
    let mut rows: BTreeMap<String, LogRow> = match (rebuild, held_now) {
        (Some(why), held) => {
            if held.is_some() {
                clear(db)?;
            }
            report
                .report
                .push(Diagnostic::new(Code::W501).with_message(why));
            report.index_rebuilt = true;
            hashers.clear();
            BTreeMap::new()
        }
        (None, Some(m)) => m.logs,
        (None, None) => unreachable!("a store with no description is rebuilt"),
    };
    let mut out = Vec::with_capacity(logs.len());
    for g in logs {
        let file = g.open()?;
        let end = match &file {
            Some(f) => f.metadata().map_err(Error::Io)?.len(),
            None => 0,
        };
        let (ix, from) = match rows.get(g.name) {
            Some(r) => (
                r.ix,
                Described {
                    layout: LAYOUT,
                    log_len: r.log_len,
                    log_hash: r.log_hash,
                    records: r.records,
                },
            ),
            None => {
                let ix = rows.values().map(|r| r.ix + 1).max().unwrap_or(0);
                (ix, Described::empty())
            }
        };
        let hasher = hashers.remove(g.name).unwrap_or_default();
        let known = rows.contains_key(g.name);
        if known && end <= from.log_len {
            out.push(CaughtUp {
                ix,
                described: from,
                hasher,
            });
            continue;
        }
        let (now, folded, hasher) = fold(db, kind, g.name, ix, file.as_deref(), from, end, hasher)?;
        rows.insert(
            g.name.to_string(),
            LogRow {
                ix,
                log_len: now.log_len,
                log_hash: now.log_hash,
                records: now.records,
            },
        );
        report.replayed += folded.records;
        report.trailing_bytes += folded.trailing;
        if folded.trailing > 0 {
            report
                .report
                .push(Diagnostic::new(Code::W110).with_message(format!(
                    "{} trailing bytes after the last complete record",
                    folded.trailing
                )));
        }
        out.push(CaughtUp {
            ix,
            described: now,
            hasher,
        });
    }
    Ok(out)
}

/// Fold one log's complete records after `from` into the tables, in one transaction, and
/// record that the store now describes that log up to the last of them.
///
/// `hasher` holds the hash of the log before `from.log_len`. The transaction commits with
/// `Durability::Immediate`: the log was written first, so a crash leaves the store behind its
/// log, never ahead of it, and behind is a tail to fold again.
#[allow(clippy::too_many_arguments)]
pub(super) fn fold(
    db: &Database,
    kind: Kind,
    name: &str,
    ix: LogIx,
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
        let mut holds = txn.open_table(HOLDS).map_err(backend)?;
        let mut deps = txn.open_table(DEPS).map_err(backend)?;
        let mut attest = txn.open_table(ATTEST).map_err(backend)?;
        let mut interp = txn.open_table(INTERP).map_err(backend)?;
        let mut observed = txn.open_table(OBSERVED).map_err(backend)?;
        let mut source = txn.open_table(SOURCE_REF).map_err(backend)?;

        while let Some((offset, len, record)) = reader.next()? {
            let hash = Store::record_hash(&record);
            let here = with_log(&hash, ix);
            // New to this log, and new to the store: a record two logs hold is one record.
            let fresh = hashes.get(&here).map_err(backend)?.is_none();
            let first = fresh && {
                let (lo, hi) = log_range(&hash);
                hashes
                    .range::<&[u8; 34]>(&lo..=&hi)
                    .map_err(backend)?
                    .next()
                    .is_none()
            };
            if fresh {
                hashes.insert(&here, seq).map_err(backend)?;
            }
            match &record {
                // A unit's first core wins, as `absorb`'s `or_insert` keeps it; a repeat
                // changes nothing. Its core is the same in every log that holds it — the uid is
                // the core's — so which log it is read from is the first to bring it.
                Record::Unit(core) if fresh => {
                    let uid = canonical_uid(core);
                    if kind == Kind::Many {
                        holds
                            .insert(&with_log(uid.as_bytes(), ix), ())
                            .map_err(backend)?;
                    }
                    if header.get(uid.as_bytes()).map_err(backend)?.is_none() {
                        // Attestations that reached a log before the unit did, each once.
                        let (lo, hi) = attest_range(&uid);
                        let mut attested = 0u32;
                        let mut hop = 0u32;
                        let mut last: Option<[u8; 32]> = None;
                        for row in attest.range::<&[u8; 66]>(&lo..=&hi).map_err(backend)? {
                            let (k, v) = row.map_err(backend)?;
                            let h: [u8; 32] = k.value()[32..64].try_into().expect("32 bytes");
                            if last == Some(h) {
                                continue;
                            }
                            last = Some(h);
                            if let Record::Attestation(a) = smysl_core::from_cbor(v.value())?.0 {
                                attested += 1;
                                hop = hop.max(a.hop);
                            }
                        }
                        let observed_ms = core.source.as_ref().and_then(|s| s.observed);
                        let h = Header {
                            log: ix,
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
                // and newest hop move now if the unit is here and the attestation is new to the
                // store; if the unit arrives later, its header is written with the attestations
                // already waiting.
                Record::Attestation(a) if fresh => {
                    attest
                        .insert(&attest_key(&a.uid, &hash, ix), reader.raw())
                        .map_err(backend)?;
                    let held = header
                        .get(a.uid.as_bytes())
                        .map_err(backend)?
                        .map(|v| Header::decode(v.value()));
                    if let (Some(mut h), true) = (held, first) {
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
                // twice is listed twice. A store of several logs drops them when it is read.
                _ => {
                    interp
                        .insert(&interp_key(ix, seq), reader.raw())
                        .map_err(backend)?;
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
        meta.insert("logs", kind.tag()).map_err(backend)?;
        let mut logs = txn.open_table(LOGS).map_err(backend)?;
        let row = LogRow {
            ix,
            log_len: now.log_len,
            log_hash: now.log_hash,
            records: now.records,
        };
        logs.insert(name, &row.encode()).map_err(backend)?;
    }
    txn.commit().map_err(backend)?;
    let folded = Folded {
        records: seq - from.records,
        trailing: reader.trailing(),
    };
    Ok((now, folded, reader.into_hasher()))
}
