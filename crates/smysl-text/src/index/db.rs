//! `index/substrate.redb` — the substrate index's backing store (RFC SMYSL-2.4 §3.8,
//! `substrate-redb`).
//!
//! # Why a database, when a sidecar already works
//!
//! The sidecar is adequate for every table here but one. `postings` is a 32-byte
//! content-addressed uid and a `u32` per posting, about 1.1 KB a unit: 169 MB at 155k units,
//! 7 GB at the 10⁶ SMYSL-2.8 sizes for — and **all of it resident while the library is open**,
//! because a sidecar is a blob that is decoded into maps.
//!
//! Neither of the two usual remedies applies on its own:
//!
//! - **Delta encoding** cannot: a uid is a hash, so a postings list of uids is a list of
//!   random 32-byte numbers and the gaps are as large as the numbers.
//! - **A dense internal document id** would take a posting from 36 bytes to about 5, and it
//!   cannot be maintained incrementally without giving up §3.8's byte-for-byte rebuild gate. A
//!   content-derived id is a rank within its partition — that is what makes it a function of
//!   the library rather than of arrival order, which is what the gate requires — and appending
//!   one window to a partition of size *p* re-ranks *p* postings. Ingest commits window by
//!   window, so that is quadratic per manifest.
//!
//! Paging the table makes the resident cost a page cache whose size the caller chooses, and it
//! leaves the gate alone. The dense id remains worth having as an **on-disk** encoding, where it
//! is a change to this module's value format and nothing else.
//!
//! # What this file is, and is not
//!
//! It is a **cache**, exactly as the sidecar is: derived from the catalog and the object store,
//! stamped with the catalog's length and hash, and rebuilt rather than trusted when the stamp
//! does not match. Deleting it loses nothing. Nothing is read from it that cannot be
//! recomputed, and no identity is derived from it.
//!
//! It is **not byte-reproducible**, and that is a property of redb rather than a choice here:
//! page allocation depends on the history of writes, so two libraries with identical contents
//! have different files. §3.8's gate — rebuild and incremental agree byte for byte — therefore
//! cannot be run against this file, and [`Db::agrees`] is what replaces it: the comparison is
//! over the *logical* table, term by term and posting by posting. That is a weaker gate on
//! paper and the same gate in practice, because what the byte comparison was ever testing was
//! the table's contents; the sidecar's reproducibility is tested separately, by the sidecar.
//!
//! # Layout
//!
//! | table | key | value |
//! |---|---|---|
//! | `meta` | a name | the catalog stamp and this file's own format version |
//! | `partitions` | partition key | analyzer id, document count, total length |
//! | `postings` | (partition key, term) | `(uid, tf)` pairs, uid order, fixed 36-byte stride |
//! | `doclen` | (partition key, uid) | the document's length in terms |
//!
//! The partition key is one byte of kind — `0` loose, `1` text — followed for a text partition
//! by the manifest's thirty-two bytes and the language tag. The language is in the key although
//! it is a function of the mid today, for the reason [`super::postings`] gives: FC-1 lets one
//! manifest hold two languages, and this is the key that corpus will need.
//!
//! `docs` and `total_len` are stored per partition rather than derived, because `avgdl` is a
//! corpus-wide quantity asked on every query and the seam exists to avoid a scan.
//!
//! # Which table is resident, and why it is not the big one
//!
//! `doclen` is read into memory when a [`View`] opens; `postings` is read a term at a time and
//! never held. That split was measured rather than designed. A scorer asks `doc_len` once per
//! **posting**, so leaving it on disk cost one b-tree descent per document — and the harness
//! said so plainly:
//!
//! | 155,510 units, 5 partitions, `--release` | resident sidecar | this store |
//! |---|---|---|
//! | query p95 | 34.9 ms | 120 ms, then **28.6 ms** |
//! | view opened in | 0.001 ms | 18.4 ms |
//! | command, end to end | 739 ms | 700 ms |
//! | index written in | 47 ms | 673 ms, plus this file |
//! | sidecar | 176 MiB | 24.8 MiB |
//! | this file | — | 514 MiB long, 312 MiB allocated |
//!
//! The 120 ms is the first version, with both tables on disk; the 28.6 ms is after `doclen`
//! became resident, which is the whole of the change. It is affordable because a length is 36
//! bytes a unit against a postings list's 1.1 KB — 5.6 MB at 155k units and 36 MB at 10⁶,
//! against the 7 GB that made this file worth having. So the table that is asked per posting is
//! in memory and the table that is 97% of the bytes is not.
//!
//! Two costs this file still has and the sidecar does not: it is **1.8× the bytes on disk** for
//! the same rows, and writing it is an order of magnitude dearer. Both are unattributed — redb's
//! own `stats()` would say how much is fragmentation and how much is the 155k-row `doclen`
//! b-tree, and one blob per partition instead of a row per document is the obvious next
//! experiment. Neither is on the query path, which is why they are recorded rather than fixed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use redb::{
    Database, Durability, ReadOnlyTable, ReadableTable, ReadableTableMetadata, TableDefinition,
};
use smysl_core::Uid;
use smysl_retrieve::{MemPostings, Postings};

use super::postings::{Partition, Table};

/// The file, under the library's `index/` directory beside the sidecar.
pub const FILE: &str = "substrate.redb";

/// This file's own format version, in `meta`.
///
/// Separate from the sidecar's version: the two formats change for different reasons, and a
/// build that can read one is not thereby able to read the other.
pub const VERSION: u32 = 1;

const META: TableDefinition<'static, &str, &[u8]> = TableDefinition::new("meta");
const PARTITIONS: TableDefinition<'static, &[u8], &[u8]> = TableDefinition::new("partitions");
const POSTINGS: TableDefinition<'static, (&[u8], &str), &[u8]> = TableDefinition::new("postings");
const DOCLEN: TableDefinition<'static, (&[u8], &[u8]), u32> = TableDefinition::new("doclen");

/// The opened `postings` table, which a [`View`] holds for as long as it lives.
///
/// An alias because the tuple key spelled inline is the sort of type clippy asks to be named,
/// and it is right to: `(&[u8], &str)` says nothing, and "a partition and a term" says what the
/// key is.
type PostingsTable = ReadOnlyTable<(&'static [u8], &'static str), &'static [u8]>;

/// A partition's terms, as [`Db::load`] groups them before it builds a `MemPostings`.
///
/// Named because the nesting is what clippy asks to be named and it is right to: the key is a
/// partition key, and the value is the thing `MemPostings::from_parts` takes.
type TermsByPartition = BTreeMap<Vec<u8>, BTreeMap<String, Vec<(Uid, u32)>>>;

/// The same, for document lengths.
type LensByPartition = BTreeMap<Vec<u8>, BTreeMap<Uid, u32>>;

const META_VERSION: &str = "version";
const META_STAMP: &str = "catalog";

/// A posting is a uid and a term frequency, and the stride is fixed so a value can be searched
/// without being decoded.
const POSTING: usize = 32 + 4;

/// Why the backing store could not be used.
///
/// Every variant is a reason to **rebuild**, never a reason to refuse: this file is a cache.
/// `Library` turns each into exactly that, which is why there is no `LibError` conversion here.
#[derive(Debug)]
#[non_exhaustive]
pub enum DbError {
    /// redb could not open, read or commit. Carries its own message, since redb's error types
    /// are not ours to re-export through a pure crate's public surface.
    Backend(String),
    /// A file written by a version of this format that this build does not know.
    UnsupportedVersion(u32),
    /// A value whose length is not a whole number of postings, or a key this build cannot read.
    Malformed(&'static str),
}

impl core::fmt::Display for DbError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DbError::Backend(m) => write!(f, "{m}"),
            DbError::UnsupportedVersion(v) => {
                write!(f, "substrate store version {v} is not supported")
            }
            DbError::Malformed(what) => write!(f, "malformed substrate store: {what}"),
        }
    }
}

impl std::error::Error for DbError {}

/// Anything redb returns becomes one message. The caller rebuilds either way, so the variant
/// a caller could match on would be a variant nobody matches.
fn backend<E: core::fmt::Display>(e: E) -> DbError {
    DbError::Backend(e.to_string())
}

/// The partition, as a key.
///
/// Order is the byte order of these keys and is **not** [`Partition`]'s derived order: loose
/// sorts first here and last there. Nothing depends on either — the only ordering this module
/// promises is uid order inside a postings list — and saying so is cheaper than making two
/// orders agree for no reader.
fn key_of(p: &Partition) -> Vec<u8> {
    match p {
        Partition::Loose => vec![0],
        Partition::Text { mid, lang } => {
            let mut k = Vec::with_capacity(1 + 32 + lang.len());
            k.push(1);
            k.extend_from_slice(mid.as_bytes());
            k.extend_from_slice(lang.as_bytes());
            k
        }
    }
}

/// The partition a key names, for a caller that wants to report what a file holds.
pub fn partition_of_key(k: &[u8]) -> Result<Partition, DbError> {
    match k.first() {
        Some(0) if k.len() == 1 => Ok(Partition::Loose),
        Some(1) if k.len() >= 33 => {
            let mut mid = [0u8; 32];
            mid.copy_from_slice(&k[1..33]);
            let lang = core::str::from_utf8(&k[33..])
                .map_err(|_| DbError::Malformed("a language tag that is not utf-8"))?;
            Ok(Partition::Text {
                mid: smysl_core::ids::Mid::from_bytes(mid),
                lang: lang.to_string(),
            })
        }
        _ => Err(DbError::Malformed("a partition key this build cannot read")),
    }
}

fn encode_list(list: &[(Uid, u32)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(list.len() * POSTING);
    for (uid, tf) in list {
        out.extend_from_slice(uid.as_bytes());
        out.extend_from_slice(&tf.to_be_bytes());
    }
    out
}

fn decode_list(b: &[u8]) -> Result<Vec<(Uid, u32)>, DbError> {
    // `%` rather than `is_multiple_of`, which is stable since 1.87 and this crate's floor is
    // 1.85. Clippy's `incompatible_msrv` is what caught it, which is the gate working.
    if b.len() % POSTING != 0 {
        return Err(DbError::Malformed("a postings list of partial postings"));
    }
    let mut out = Vec::with_capacity(b.len() / POSTING);
    for chunk in b.chunks_exact(POSTING) {
        let mut uid = [0u8; 32];
        uid.copy_from_slice(&chunk[..32]);
        let tf = u32::from_be_bytes([chunk[32], chunk[33], chunk[34], chunk[35]]);
        out.push((Uid::from_bytes(uid), tf));
    }
    Ok(out)
}

/// One posting's term frequency, found without decoding the list.
///
/// The stride is fixed and the list is in uid order, so this is a binary search over the value
/// bytes. It is what makes [`View::tf`] cheap enough to decompose a score with: the seam's
/// default answer materialises a list that can be as long as the corpus.
fn tf_in(b: &[u8], uid: &Uid) -> u32 {
    if b.len() % POSTING != 0 {
        return 0;
    }
    let want = uid.as_bytes();
    let (mut lo, mut hi) = (0usize, b.len() / POSTING);
    while lo < hi {
        let mid = (lo + hi) / 2;
        let at = mid * POSTING;
        match b[at..at + 32].cmp(&want[..]) {
            core::cmp::Ordering::Less => lo = mid + 1,
            core::cmp::Ordering::Greater => hi = mid,
            core::cmp::Ordering::Equal => {
                return u32::from_be_bytes([b[at + 32], b[at + 33], b[at + 34], b[at + 35]]);
            }
        }
    }
    0
}

/// A partition's summary, read once when a view is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    key: Vec<u8>,
    analyzer: String,
    docs: u32,
    total: u64,
}

impl Row {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(12 + self.analyzer.len());
        out.extend_from_slice(&self.docs.to_be_bytes());
        out.extend_from_slice(&self.total.to_be_bytes());
        out.extend_from_slice(self.analyzer.as_bytes());
        out
    }

    fn decode(key: &[u8], b: &[u8]) -> Result<Row, DbError> {
        if b.len() < 12 {
            return Err(DbError::Malformed("a partition summary that is too short"));
        }
        let docs = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        let total = u64::from_be_bytes([b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11]]);
        let analyzer = core::str::from_utf8(&b[12..])
            .map_err(|_| DbError::Malformed("an analyzer id that is not utf-8"))?;
        Ok(Row {
            key: key.to_vec(),
            analyzer: analyzer.to_string(),
            docs,
            total,
        })
    }
}

/// The backing store for one library's index.
pub struct Db {
    db: Database,
}

/// The path, and nothing redb holds.
///
/// Manual rather than derived because [`crate::library::Library`] is `Debug` and holds one of
/// these. What a reader of a dump wants here is which file it is, and `Database`'s own `Debug`
/// is the b-tree's.
impl core::fmt::Debug for Db {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Db").field("file", &FILE).finish()
    }
}

impl Db {
    /// Where the file lives, given a library root.
    pub fn path(root: &Path) -> PathBuf {
        root.join(super::STORE)
    }

    /// Whether a library has a backing store already.
    ///
    /// The question an **open** asks, and the reason [`Db::open`] creating one is not a problem:
    /// opening a library is a write in exactly one case — rule Z, which unlinks the objects of a
    /// redacted part in `Library::enforce_z` (named rather than linked: a public item's
    /// documentation may not link to a private one) — and a cache is not it, so a library is
    /// only handed its store when it has one. A writing command is what creates the file, which
    /// is the same rule the sidecar has.
    pub fn exists(root: &Path) -> bool {
        Db::path(root).is_file()
    }

    /// Open the file, creating it if it is not there.
    ///
    /// Creating on open is right for a cache and wrong for a store: there is nothing to lose by
    /// making an empty one, and a caller that finds it empty rebuilds, which is the same path a
    /// stale stamp takes.
    pub fn open(root: &Path) -> Result<Db, DbError> {
        let target = Db::path(root);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(backend)?;
        }
        let db = Database::create(&target).map_err(backend)?;
        let me = Db { db };
        match me.version()? {
            None => me.set_version()?,
            Some(VERSION) => {}
            Some(other) => return Err(DbError::UnsupportedVersion(other)),
        }
        Ok(me)
    }

    fn version(&self) -> Result<Option<u32>, DbError> {
        let txn = self.db.begin_read().map_err(backend)?;
        let table = match txn.open_table(META) {
            Ok(t) => t,
            // A file with no `meta` table is a file this build has not written yet, which is
            // the empty case and not a defect.
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(backend(e)),
        };
        let found = table.get(META_VERSION).map_err(backend)?;
        match found {
            None => Ok(None),
            Some(v) => {
                let b = v.value();
                if b.len() != 4 {
                    return Err(DbError::Malformed("a version that is not four bytes"));
                }
                Ok(Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]])))
            }
        }
    }

    fn set_version(&self) -> Result<(), DbError> {
        let mut txn = self.db.begin_write().map_err(backend)?;
        txn.set_durability(Durability::Immediate);
        {
            let mut table = txn.open_table(META).map_err(backend)?;
            table
                .insert(META_VERSION, &VERSION.to_be_bytes()[..])
                .map_err(backend)?;
        }
        txn.commit().map_err(backend)
    }

    /// The catalog this file was written for: its length and its hash.
    ///
    /// `None` for a file nothing has been written to. The stamp is what makes this a cache
    /// rather than a second opinion — a caller compares it against the catalog it has open and
    /// rebuilds when they differ, which is exactly what the sidecar's `matches` does.
    pub fn stamp(&self) -> Result<Option<(u64, [u8; 32])>, DbError> {
        let txn = self.db.begin_read().map_err(backend)?;
        let table = match txn.open_table(META) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(backend(e)),
        };
        let Some(v) = table.get(META_STAMP).map_err(backend)? else {
            return Ok(None);
        };
        let b = v.value();
        if b.len() != 40 {
            return Err(DbError::Malformed(
                "a catalog stamp that is not forty bytes",
            ));
        }
        let mut len = [0u8; 8];
        len.copy_from_slice(&b[..8]);
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&b[8..]);
        Ok(Some((u64::from_be_bytes(len), hash)))
    }

    /// Whether this file describes the catalog the caller has open.
    pub fn matches(&self, len: u64, hash: &[u8; 32]) -> bool {
        matches!(self.stamp(), Ok(Some((l, h))) if l == len && &h == hash)
    }

    /// Replace the whole `postings` table with `table`, and stamp it.
    ///
    /// **One transaction**, so a reader sees the table before or after and never during, and
    /// `Durability::Immediate` so a crash after the call cannot leave the stamp current over
    /// stale postings. The write is a replacement rather than a merge because the caller holds
    /// the whole table in memory at this point — this is the step that stops being true when
    /// the dense id arrives, and the shape that makes an incremental write possible later is
    /// the per-partition key, which is already here.
    pub fn write(&self, table: &Table, len: u64, hash: &[u8; 32]) -> Result<(), DbError> {
        let mut txn = self.db.begin_write().map_err(backend)?;
        txn.set_durability(Durability::Immediate);
        {
            let mut meta = txn.open_table(META).map_err(backend)?;
            let mut parts = txn.open_table(PARTITIONS).map_err(backend)?;
            let mut post = txn.open_table(POSTINGS).map_err(backend)?;
            let mut lens = txn.open_table(DOCLEN).map_err(backend)?;

            // The stamp goes last in intent and first in code: it is cleared here so that a
            // transaction that fails part-way leaves no file claiming to describe a catalog.
            meta.remove(META_STAMP).map_err(backend)?;
            parts.retain(|_, _| false).map_err(backend)?;
            post.retain(|_, _| false).map_err(backend)?;
            lens.retain(|_, _| false).map_err(backend)?;

            for (partition, row) in table {
                let key = key_of(partition);
                let summary = Row {
                    key: key.clone(),
                    analyzer: row.analyzer().unwrap_or_default().to_string(),
                    docs: row.docs(),
                    total: row.total_len(),
                };
                parts
                    .insert(key.as_slice(), summary.encode().as_slice())
                    .map_err(backend)?;
                for (term, list) in row.iter() {
                    post.insert((key.as_slice(), term), encode_list(list).as_slice())
                        .map_err(backend)?;
                }
                for (uid, dl) in row.lengths() {
                    lens.insert((key.as_slice(), &uid.as_bytes()[..]), dl)
                        .map_err(backend)?;
                }
            }

            let mut stamp = Vec::with_capacity(40);
            stamp.extend_from_slice(&len.to_be_bytes());
            stamp.extend_from_slice(hash);
            meta.insert(META_STAMP, stamp.as_slice()).map_err(backend)?;
        }
        txn.commit().map_err(backend)
    }

    /// The whole `postings` table, back in memory.
    ///
    /// The one read that defeats the purpose of the file, and the one a **writing** command
    /// cannot do without: `on_append` extends a table it can see, and §3.8's byte-for-byte gate
    /// is over a table, not over a file. So a library whose postings live here hydrates them
    /// before it appends, and writes the whole table back afterwards — which is the same
    /// `O(corpus)` a sidecar paid on every write, now paid only by the commands that write.
    /// Reading commands never call this; that is the whole saving.
    ///
    /// The incremental write is what replaces it, and the shape that makes one possible is
    /// already here: the key is `(partition, term)`, so a batch touches the terms of the
    /// partitions it mentions. It is not done yet because a merge has to produce exactly what a
    /// replacement would, and that claim needs the gate below to be checked against both.
    ///
    /// A term under a partition the `partitions` table does not summarise is malformed rather
    /// than skipped: the summary carries the analyzer and the document count, so a term without
    /// one would be scored under the empty analyzer and counted in no corpus.
    pub fn load(&self) -> Result<Table, DbError> {
        let rows = self.rows()?;
        let mut terms: TermsByPartition = BTreeMap::new();
        let mut lens: LensByPartition = BTreeMap::new();
        let txn = self.db.begin_read().map_err(backend)?;
        match txn.open_table(POSTINGS) {
            Ok(t) => {
                for entry in t.iter().map_err(backend)? {
                    let (k, v) = entry.map_err(backend)?;
                    let (key, term) = k.value();
                    terms
                        .entry(key.to_vec())
                        .or_default()
                        .insert(term.to_string(), decode_list(v.value())?);
                }
            }
            Err(redb::TableError::TableDoesNotExist(_)) => {}
            Err(e) => return Err(backend(e)),
        }
        match txn.open_table(DOCLEN) {
            Ok(t) => {
                for entry in t.iter().map_err(backend)? {
                    let (k, v) = entry.map_err(backend)?;
                    let (key, uid) = k.value();
                    let uid: [u8; 32] = uid
                        .try_into()
                        .map_err(|_| DbError::Malformed("a document length under a short uid"))?;
                    lens.entry(key.to_vec())
                        .or_default()
                        .insert(Uid::from_bytes(uid), v.value());
                }
            }
            Err(redb::TableError::TableDoesNotExist(_)) => {}
            Err(e) => return Err(backend(e)),
        }
        let known: std::collections::BTreeSet<&Vec<u8>> = rows.iter().map(|r| &r.key).collect();
        for key in terms.keys().chain(lens.keys()) {
            if !known.contains(key) {
                return Err(DbError::Malformed("a term under an unsummarised partition"));
            }
        }
        let mut table = Table::new();
        for row in &rows {
            table.insert(
                partition_of_key(&row.key)?,
                MemPostings::from_parts(
                    &row.analyzer,
                    terms.remove(&row.key).unwrap_or_default(),
                    lens.remove(&row.key).unwrap_or_default(),
                ),
            );
        }
        Ok(table)
    }

    /// Every partition this file holds, with its analyzer, in key order.
    pub fn partitions(&self) -> Result<Vec<(Partition, String, u32)>, DbError> {
        let rows = self.rows()?;
        rows.into_iter()
            .map(|r| {
                let p = partition_of_key(&r.key)?;
                Ok((p, r.analyzer, r.docs))
            })
            .collect()
    }

    fn rows(&self) -> Result<Vec<Row>, DbError> {
        let txn = self.db.begin_read().map_err(backend)?;
        let table = match txn.open_table(PARTITIONS) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
            Err(e) => return Err(backend(e)),
        };
        let mut out = Vec::new();
        for entry in table.iter().map_err(backend)? {
            let (k, v) = entry.map_err(backend)?;
            out.push(Row::decode(k.value(), v.value())?);
        }
        Ok(out)
    }

    /// How many `(partition, term)` rows the `postings` table holds.
    ///
    /// What `text index` prints for the table, and the same quantity the in-memory path counts:
    /// terms, not partitions and not postings.
    pub fn terms(&self) -> Result<usize, DbError> {
        let txn = self.db.begin_read().map_err(backend)?;
        match txn.open_table(POSTINGS) {
            Ok(t) => Ok(t.len().map_err(backend)? as usize),
            Err(redb::TableError::TableDoesNotExist(_)) => Ok(0),
            Err(e) => Err(backend(e)),
        }
    }

    /// A scoring view over every partition.
    pub fn all(&self) -> Result<View, DbError> {
        let rows = self.rows()?;
        self.view(rows)
    }

    /// A scoring view over the partitions named, which is how a scope narrower than the library
    /// is expressed (OQ-14). A partition this file does not hold contributes nothing.
    pub fn of(&self, partitions: &[Partition]) -> Result<View, DbError> {
        let want: std::collections::BTreeSet<Vec<u8>> = partitions.iter().map(key_of).collect();
        let rows = self
            .rows()?
            .into_iter()
            .filter(|r| want.contains(&r.key))
            .collect();
        self.view(rows)
    }

    /// A scoring view over the partitions analysed under `analyzer`.
    pub fn under(&self, analyzer: &str) -> Result<View, DbError> {
        let rows = self
            .rows()?
            .into_iter()
            .filter(|r| r.analyzer == analyzer)
            .collect();
        self.view(rows)
    }

    fn view(&self, parts: Vec<Row>) -> Result<View, DbError> {
        let txn = self.db.begin_read().map_err(backend)?;
        // Tables that do not exist yet are an empty view, not an error: a file created and never
        // written has no `postings`, and a query against an empty index matches nothing, which
        // is an answer.
        let postings = match txn.open_table(POSTINGS) {
            Ok(t) => Some(t),
            Err(redb::TableError::TableDoesNotExist(_)) => None,
            Err(e) => return Err(backend(e)),
        };
        let want: std::collections::BTreeSet<&[u8]> =
            parts.iter().map(|r| r.key.as_slice()).collect();
        let lens = self.lens_of(&txn, &want)?;
        Ok(View {
            parts,
            postings,
            lens,
        })
    }

    /// Every document length of the partitions named, in memory.
    ///
    /// **The one table a view hydrates**, and the measurement is the argument. A scorer asks
    /// `doc_len` once per *posting*, so a common term costs one b-tree descent per document —
    /// 120 ms a query at 155k units against 35 ms for the same arithmetic over resident maps,
    /// and the one query in the harness whose term has a single posting came back in 0.88 ms,
    /// which is what says the cost is per posting rather than per lookup.
    ///
    /// It is affordable for the reason the postings lists are not: a length is 36 bytes a unit
    /// against their 1.1 KB — 5.6 MB at 155k units and 36 MB at 10⁶, half a percent of what
    /// holding the lists would cost. So the table that is asked per posting is resident and the
    /// table that is 97% of the bytes stays on disk, which is the split the file exists for.
    fn lens_of(
        &self,
        txn: &redb::ReadTransaction,
        want: &std::collections::BTreeSet<&[u8]>,
    ) -> Result<BTreeMap<Uid, u32>, DbError> {
        let table = match txn.open_table(DOCLEN) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(BTreeMap::new()),
            Err(e) => return Err(backend(e)),
        };
        let mut out = BTreeMap::new();
        for entry in table.iter().map_err(backend)? {
            let (k, v) = entry.map_err(backend)?;
            let (key, uid) = k.value();
            if !want.contains(key) {
                continue;
            }
            let uid: [u8; 32] = uid
                .try_into()
                .map_err(|_| DbError::Malformed("a document length under a short uid"))?;
            out.insert(Uid::from_bytes(uid), v.value());
        }
        Ok(out)
    }

    /// Whether this file's `postings` table holds exactly what `table` does.
    ///
    /// §3.8's gate for this backing. The byte comparison the sidecar gets is unavailable — redb
    /// files are not reproducible — so the comparison is over the logical table: the same
    /// partitions, each with the same analyzer, document count, total length, term set, postings
    /// lists and document lengths. A mismatch returns the first difference as a sentence,
    /// because "they differ" is not a finding anybody can act on.
    pub fn agrees(&self, table: &Table) -> Result<(), String> {
        let rows = self.rows().map_err(|e| e.to_string())?;
        if rows.len() != table.len() {
            return Err(format!(
                "the store holds {} partition(s) and the table {}",
                rows.len(),
                table.len()
            ));
        }
        let view = self.view(rows.clone()).map_err(|e| e.to_string())?;
        for (partition, row) in table {
            let key = key_of(partition);
            let Some(stored) = rows.iter().find(|r| r.key == key) else {
                return Err(format!("the store has no partition `{partition}`"));
            };
            let want = row.analyzer().unwrap_or_default();
            if stored.analyzer != want {
                return Err(format!(
                    "`{partition}` is analysed under `{}` in the store and `{want}` in the table",
                    stored.analyzer
                ));
            }
            if stored.docs != row.docs() || stored.total != row.total_len() {
                return Err(format!(
                    "`{partition}` holds {}/{} documents/terms in the store and {}/{} in the table",
                    stored.docs,
                    stored.total,
                    row.docs(),
                    row.total_len()
                ));
            }
            for (term, list) in row.iter() {
                let got = view
                    .list(&key, term)
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default();
                if got != list {
                    return Err(format!(
                        "`{partition}` term `{term}`: {} posting(s) in the store and {} in the \
                         table",
                        got.len(),
                        list.len()
                    ));
                }
            }
            if view.terms_in(&key).map_err(|e| e.to_string())? != row.iter().count() {
                return Err(format!("`{partition}` holds a term the table does not"));
            }
            for (uid, dl) in row.lengths() {
                if view.doc_len(uid) != Some(dl) {
                    return Err(format!(
                        "`{partition}` disagrees about the length of `{}`",
                        uid.short()
                    ));
                }
            }
        }
        // And no length the table does not have. The loop above would pass a store that held
        // every length the table does *and* a document the table has never heard of, which is a
        // corpus of a different size — `docs` is what `avgdl` divides by.
        let want: usize = table.values().map(|row| row.lengths().count()).sum();
        if view.lens.len() != want {
            return Err(format!(
                "the store holds {} document length(s) and the table {want}",
                view.lens.len()
            ));
        }
        Ok(())
    }
}

/// A scoring view over a set of partitions of one file.
///
/// Implements [`Postings`] the way [`super::postings::View`] does, and must: `df`, `docs` and
/// total length are counts over **disjoint** document sets — a unit is in exactly one partition
/// — so summing the partitions a scope names gives that scope's statistics exactly. The two
/// implementations are the same arithmetic over two backings, and a test scores a corpus through
/// both and compares.
pub struct View {
    parts: Vec<Row>,
    postings: Option<PostingsTable>,
    /// The document lengths of this view's partitions, read when it opened. See [`Db::lens_of`]
    /// for why this one table is in memory and the postings lists are not.
    lens: BTreeMap<Uid, u32>,
}

impl View {
    /// How many partitions this view spans.
    pub fn partitions(&self) -> usize {
        self.parts.len()
    }

    fn list(&self, key: &[u8], term: &str) -> Result<Option<Vec<(Uid, u32)>>, DbError> {
        let Some(table) = self.postings.as_ref() else {
            return Ok(None);
        };
        match table.get((key, term)).map_err(backend)? {
            Some(v) => Ok(Some(decode_list(v.value())?)),
            None => Ok(None),
        }
    }

    fn terms_in(&self, key: &[u8]) -> Result<usize, DbError> {
        let Some(table) = self.postings.as_ref() else {
            return Ok(0);
        };
        let mut n = 0usize;
        for entry in table.iter().map_err(backend)? {
            let (k, _) = entry.map_err(backend)?;
            if k.value().0 == key {
                n += 1;
            }
        }
        Ok(n)
    }

    /// A term's raw value bytes in one partition, handed to `f` **without being copied**.
    ///
    /// The two methods that do not want the list want this one, and the borrow is the whole
    /// point: `df` divides the length by the stride and `tf` binary-searches it, and a postings
    /// list in this corpus can be a megabyte. Returning a `Vec` is what the first version did,
    /// and it cost 125 ms a query at 155k units against 34 ms for the same arithmetic over the
    /// resident table — a copy of the corpus per term per partition, to read a length.
    fn with_bytes<R>(&self, key: &[u8], term: &str, f: impl FnOnce(&[u8]) -> R) -> Option<R> {
        let guard = self.postings.as_ref()?.get((key, term)).ok()??;
        Some(f(guard.value()))
    }
}

impl Postings for View {
    /// A read error counts as zero, and that is the honest answer rather than a swallowed
    /// failure: the trait returns a count, this file is a cache, and a cache that cannot be read
    /// is rebuilt by the caller that opened it. A `Result` here would put an unreachable error
    /// path in every scorer.
    fn df(&self, term: &str) -> u32 {
        self.parts
            .iter()
            .map(|r| {
                self.with_bytes(&r.key, term, |b| (b.len() / POSTING) as u32)
                    .unwrap_or(0)
            })
            .sum()
    }

    fn postings(&self, term: &str) -> Vec<(Uid, u32)> {
        let mut out: Vec<(Uid, u32)> = self
            .parts
            .iter()
            .filter_map(|r| self.list(&r.key, term).ok().flatten())
            .flatten()
            .collect();
        // Uid order across partitions, as the seam requires and as the in-memory view does.
        out.sort_unstable_by_key(|a| a.0);
        out
    }

    /// A map hit, as the in-memory view's is. A unit is in exactly one partition, so the lengths
    /// of the partitions this view spans are one map with no ambiguity in it.
    fn doc_len(&self, uid: &Uid) -> Option<u32> {
        self.lens.get(uid).copied()
    }

    /// Searched in the stored bytes, partition by partition, never through the merged list.
    fn tf(&self, term: &str, uid: &Uid) -> u32 {
        self.parts
            .iter()
            .map(|r| {
                self.with_bytes(&r.key, term, |b| tf_in(b, uid))
                    .unwrap_or(0)
            })
            .find(|tf| *tf > 0)
            .unwrap_or(0)
    }

    fn docs(&self) -> u32 {
        self.parts.iter().map(|r| r.docs).sum()
    }

    fn total_len(&self) -> u64 {
        self.parts.iter().map(|r| r.total).sum()
    }

    /// The id every partition agrees on, or `None` when they do not — the in-memory view's rule.
    ///
    /// An empty id is `None` too, where the in-memory view would have had `None` from
    /// `MemPostings::analyzer`: the stored form of "no id" is an empty string, since a row has
    /// to hold something.
    fn analyzer(&self) -> Option<&str> {
        let first = self.parts.first()?.analyzer.as_str();
        if first.is_empty() {
            return None;
        }
        self.parts
            .iter()
            .all(|r| r.analyzer == first)
            .then_some(first)
    }
}
