//! A library's shards (SMYSL-2.4 §14.2, SMYSL-2.8 §3.6; TX-P13c step 1).
//!
//! One log per expression and one catalog:
//!
//! ```text
//! <root>/log/catalog.cbor              manifests, redactions, datings, and every record that
//!                                      names no expression's unit
//! <root>/log/shards/<alias>.cbor       an expression's units, and the relations, attestations,
//!                                      commitments and withdrawals on them
//! <root>/index/shards/<alias>.uids     each shard's uids, for a union of some shards to tell a
//!                                      reference that waits on another (`SMY-W500`) from one
//!                                      that points at nothing
//! ```
//!
//! An alias may hold `/` and `:` (A-3) and neither may reach a file name, so a shard's file stem
//! spells them `~` and `+`, which A-3 does not allow — the mapping is one-to-one and a segment
//! of dots stays inside one name. The catalog shard is named `catalog`; an expression aliased
//! `catalog` is routed to it rather than given a second log of that name.
//!
//! **Routing** is by what a record is about. A unit goes to the expression whose manifest holds
//! the part its source names (the least alias if two do); a relation goes where its `from`
//! unit is, else where its `to` unit is; an attestation, a commitment or a withdrawal goes where
//! its subject is. Everything else, and anything whose subject is in no expression, goes to the
//! catalog. Routing decides only *where* a record is written: the library reads every shard,
//! and a record in the "wrong" shard is still in the library — which is also why a log written
//! directly (`-s <library>` on a command that appends to the catalog) stays a valid library.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use smysl_core::error::LibError;
use smysl_core::ids::{Tid, Uid};
use smysl_core::types::Record;
use smysl_core::{canonical_uid, from_cbor_seq, to_cbor, Rolling};
use smysl_graph::{Store, UidShards};

/// The catalog shard's name.
pub const CATALOG: &str = "catalog";

/// Where shard logs other than the catalog live, relative to the root.
pub const DIR: &str = "log/shards";

/// Where each shard's uid list lives, relative to the root.
pub const UIDS_DIR: &str = "index/shards";

const UIDS_MAGIC: &[u8] = b"smysl/uids/1\n";

/// A shard's file stem: its name with `/` as `~` and `:` as `+`.
pub fn stem(shard: &str) -> String {
    shard.replace('/', "~").replace(':', "+")
}

/// The shard a file stem names.
pub fn name_of_stem(stem: &str) -> String {
    stem.replace('~', "/").replace('+', ":")
}

/// A shard's log, relative to the root.
pub fn log_path(shard: &str) -> PathBuf {
    if shard == CATALOG {
        PathBuf::from(crate::library::CATALOG)
    } else {
        Path::new(DIR).join(format!("{}.cbor", stem(shard)))
    }
}

fn uids_path(shard: &str) -> PathBuf {
    Path::new(UIDS_DIR).join(format!("{}.uids", stem(shard)))
}

fn io(at: impl AsRef<Path>, e: &std::io::Error) -> LibError {
    LibError::Io {
        at: at.as_ref().display().to_string(),
        message: e.to_string(),
    }
}

/// Every shard a library root holds, catalog included, in name order, with each one's log.
///
/// Read from the directory, so a reader can open some shards without opening the library.
pub fn list(root: &Path) -> Result<Vec<(String, PathBuf)>, LibError> {
    let mut names: BTreeSet<String> = BTreeSet::new();
    names.insert(CATALOG.to_string());
    match std::fs::read_dir(root.join(DIR)) {
        Ok(entries) => {
            for e in entries {
                let e = e.map_err(|e| io(DIR, &e))?;
                let p = e.path();
                if p.extension().is_some_and(|x| x == "cbor") {
                    if let Some(s) = p.file_stem() {
                        names.insert(name_of_stem(&s.to_string_lossy()));
                    }
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(io(DIR, &e)),
    }
    Ok(names
        .into_iter()
        .map(|n| {
            let p = root.join(log_path(&n));
            (n, p)
        })
        .collect())
}

/// One shard's log as the library writes it.
#[derive(Debug)]
pub(crate) struct ShardLog {
    /// Relative to the root, for messages.
    rel: PathBuf,
    path: PathBuf,
    /// The complete records' bytes, and their running hash.
    len: u64,
    hasher: Rolling,
    /// Bytes after the last complete record: a writer killed mid-append left them, and the next
    /// append cuts them before writing, so the log stays a sequence of whole records.
    torn: u64,
}

impl ShardLog {
    /// Read a shard's log: its records, and the log as a writer will continue it.
    pub(crate) fn read(root: &Path, shard: &str) -> Result<(ShardLog, Vec<Record>), LibError> {
        let rel = log_path(shard);
        let path = root.join(&rel);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(io(&rel, &e)),
        };
        let (records, consumed) = from_cbor_seq(&bytes).map_err(|e| LibError::Io {
            at: rel.display().to_string(),
            message: e.to_string(),
        })?;
        let mut hasher = Rolling::new();
        hasher.update(&bytes[..consumed]);
        Ok((
            ShardLog {
                rel,
                path,
                len: consumed as u64,
                hasher,
                torn: (bytes.len() - consumed) as u64,
            },
            records,
        ))
    }

    pub(crate) fn len(&self) -> u64 {
        self.len
    }

    pub(crate) fn hash(&self) -> [u8; 32] {
        self.hasher.finish()
    }

    /// Append encoded records, cutting a torn tail first, and sync them.
    pub(crate) fn append(&mut self, bytes: &[u8]) -> Result<(), LibError> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| io(&self.rel, &e))?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| io(&self.rel, &e))?;
        if self.torn > 0 {
            f.set_len(self.len).map_err(|e| io(&self.rel, &e))?;
            self.torn = 0;
        }
        f.write_all(bytes).map_err(|e| io(&self.rel, &e))?;
        f.sync_data().map_err(|e| io(&self.rel, &e))?;
        self.len += bytes.len() as u64;
        self.hasher.update(bytes);
        Ok(())
    }
}

/// The library's stamp over its shards: what the substrate index and its backing store record,
/// so that a cache is current exactly when no shard has changed.
///
/// A library of one log stamps as that log — its length and hash — which is what the stamp was
/// before there were shards, so a sidecar written then is still current now. With shards, the
/// length is the logs' total and the hash is over each shard's name, length and hash in name
/// order.
pub(crate) fn stamp(shards: &BTreeMap<String, ShardLog>) -> (u64, [u8; 32]) {
    let live: Vec<(&String, &ShardLog)> = shards
        .iter()
        .filter(|(n, l)| n.as_str() == CATALOG || l.len > 0)
        .collect();
    if live.len() == 1 {
        let (_, l) = live[0];
        return (l.len, l.hash());
    }
    let mut h = Rolling::new();
    h.update(b"smysl/library-stamp/1");
    h.update(&[0x00]);
    let mut total = 0u64;
    for (name, l) in live {
        h.update(&(name.len() as u64).to_be_bytes());
        h.update(name.as_bytes());
        h.update(&l.len.to_be_bytes());
        h.update(&l.hash());
        total += l.len;
    }
    (total, h.finish())
}

/// Where each record of a batch goes.
pub(crate) fn route(store: &Store, index: &UidShards, records: &[Record]) -> Vec<String> {
    // Which expression holds each part, from every manifest the library and the batch hold.
    let mut part_alias: BTreeMap<Tid, String> = BTreeMap::new();
    let batch_manifests = records.iter().filter_map(|r| match r {
        Record::Manifest(m) => Some(m),
        _ => None,
    });
    for m in store.manifests().map(|(_, m)| m).chain(batch_manifests) {
        if !m.alias_is_valid() {
            continue;
        }
        for p in &m.parts {
            part_alias
                .entry(p.tid)
                .and_modify(|a| {
                    if m.alias < *a {
                        *a = m.alias.clone();
                    }
                })
                .or_insert_with(|| m.alias.clone());
        }
    }

    // Units first, so a relation in the same batch follows its endpoint.
    let mut placed: BTreeMap<Uid, String> = BTreeMap::new();
    for r in records {
        if let Record::Unit(u) = r {
            let shard = u
                .source
                .as_ref()
                .and_then(|s| Store::part_of_reference(&s.reference))
                .and_then(|tid| part_alias.get(&tid).cloned())
                .unwrap_or_else(|| CATALOG.to_string());
            placed.insert(canonical_uid(u), shard);
        }
    }
    let unit_shard = |uid: &Uid| -> Option<String> {
        placed
            .get(uid)
            .cloned()
            .or_else(|| index.shards_of(uid).next().map(str::to_string))
    };
    // A relation's shard, by its rid, from the batch or the store.
    let mut rel_shard: BTreeMap<Uid, String> = BTreeMap::new();
    let of_relation = |rel: &smysl_core::Relation| -> String {
        unit_shard(&rel.from)
            .or_else(|| unit_shard(&rel.to))
            .unwrap_or_else(|| CATALOG.to_string())
    };
    for r in records {
        if let Record::Relation(rel) = r {
            rel_shard.insert(rel.uid(), of_relation(rel));
        }
    }
    let by_rid = |rid: &Uid| -> Option<String> {
        rel_shard
            .get(rid)
            .cloned()
            .or_else(|| store.relation_by_id(rid).map(of_relation))
    };

    records
        .iter()
        .map(|r| match r {
            Record::Unit(u) => placed[&canonical_uid(u)].clone(),
            Record::Relation(rel) => rel_shard[&rel.uid()].clone(),
            Record::Attestation(a) => unit_shard(&a.uid)
                .or_else(|| by_rid(&a.uid))
                .unwrap_or_else(|| CATALOG.to_string()),
            Record::Commit(c) => unit_shard(&c.unit).unwrap_or_else(|| CATALOG.to_string()),
            Record::Withdrawal(w) => by_rid(&w.relation).unwrap_or_else(|| CATALOG.to_string()),
            _ => CATALOG.to_string(),
        })
        .collect()
}

/// Encode a batch's fresh records by shard: each record not already in the store, once.
pub(crate) fn encode_by_shard(
    store: &Store,
    records: &[Record],
    shards: &[String],
) -> BTreeMap<String, (Vec<u8>, Vec<Uid>)> {
    let mut out: BTreeMap<String, (Vec<u8>, Vec<Uid>)> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for (r, shard) in records.iter().zip(shards) {
        let bytes = to_cbor(r);
        let hash = smysl_core::hash_bytes(&bytes);
        if store.contains_record(&hash) || !seen.insert(hash) {
            continue;
        }
        let entry = out.entry(shard.clone()).or_default();
        entry.0.extend_from_slice(&bytes);
        if let Record::Unit(u) = r {
            entry.1.push(canonical_uid(u));
        }
    }
    out
}

/// Write a shard's uid list, stamped with the length of the log it lists.
///
/// A cache, so a failure is not an error: a reader that finds it missing or stale reads the
/// shard's log instead.
pub(crate) fn write_uids(root: &Path, shard: &str, log_len: u64, uids: &[Uid]) {
    let target = root.join(uids_path(shard));
    let Some(dir) = target.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let mut bytes = Vec::with_capacity(UIDS_MAGIC.len() + 8 + uids.len() * 32);
    bytes.extend_from_slice(UIDS_MAGIC);
    bytes.extend_from_slice(&log_len.to_be_bytes());
    for u in uids {
        bytes.extend_from_slice(u.as_bytes());
    }
    let tmp = target.with_extension("uids.tmp");
    if std::fs::write(&tmp, &bytes).is_ok() {
        let _ = std::fs::rename(&tmp, &target);
    }
}

fn read_uids(root: &Path, shard: &str, log_len: u64) -> Option<Vec<Uid>> {
    let bytes = std::fs::read(root.join(uids_path(shard))).ok()?;
    let rest = bytes.strip_prefix(UIDS_MAGIC)?;
    if rest.len() < 8 || (rest.len() - 8) % 32 != 0 {
        return None;
    }
    let (len, body) = rest.split_at(8);
    if u64::from_be_bytes(len.try_into().ok()?) != log_len {
        return None;
    }
    Some(
        body.chunks_exact(32)
            .map(|c| Uid::from_bytes(c.try_into().expect("32 bytes")))
            .collect(),
    )
}

/// Which shards hold each uid, for a library root, without opening the library.
///
/// From each shard's uid list where it is current — stamped with the length its log has now,
/// which for an append-only log is the log — and from the log itself where it is not.
pub fn uid_index(root: &Path) -> Result<UidShards, LibError> {
    let mut index = UidShards::new();
    for (name, path) in list(root)? {
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let uids = match read_uids(root, &name, len) {
            Some(u) => u,
            None => {
                let (log, records) = ShardLog::read(root, &name)?;
                let uids: Vec<Uid> = records
                    .iter()
                    .filter_map(|r| match r {
                        Record::Unit(u) => Some(canonical_uid(u)),
                        _ => None,
                    })
                    .collect();
                if log.torn == 0 {
                    let mut sorted = uids.clone();
                    sorted.sort_unstable();
                    sorted.dedup();
                    write_uids(root, &name, len, &sorted);
                }
                uids
            }
        };
        index.set(&name, uids);
    }
    Ok(index)
}
