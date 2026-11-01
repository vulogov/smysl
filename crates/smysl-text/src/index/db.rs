//! `index/substrate.redb` — the substrate index's backing store (RFC SMYSL-2.4 §3.8,
//! `substrate-redb`).
//!
//! # Why a database, when a sidecar already works
//!
//! The sidecar is adequate for every table here but one. `postings` is a 32-byte
//! content-addressed uid and a `u32` per posting, about 1.1 KB a unit: 169 MB at 155k units,
//! about 1.1 GB at the 10⁶ SMYSL-2.8 sizes for — and **all of it resident while the library is open**,
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
//! leaves the gate alone. The dense id is worth having as an **on-disk** encoding, where it is a
//! change to this module's value format and nothing else — and since version 2 it is one.
//!
//! # The dense id, and why the re-ranking costs nothing here
//!
//! A document's id is its **rank in its partition's uid order**: the position of its row in the
//! partition's `docs` blob, which holds every `(uid, length)` of the partition sorted by uid.
//! The id is assigned when the table is **encoded**, never kept — the `Postings` seam, the
//! in-memory table and everything that ranks still speak uids — so the objection above does not
//! reach it. [`Db::write`] replaces the whole table in one transaction already, so re-ranking a
//! partition on every write is the price of a write this file was paying anyway, and a rank is a
//! function of the table alone: two tables that agree encode to the same bytes, which is §3.8's
//! gate restated one level down (`tests/substrate_db.rs`).
//!
//! Ranks make a postings list an ascending run of small integers, which is what delta encoding
//! needs: a gap and a term frequency as LEB128 varints, usually a byte each, where a posting was
//! a 32-byte uid and a four-byte count. A list is cut into blocks of [`BLOCK`] postings behind a
//! fixed-width directory of `(first id, offset)`, so [`View::tf`] still finds one posting by a
//! binary search and a decode of at most one block, and `df` is a header read.
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
//! | `docs` | partition key | every `(uid, length)` of the partition, uid order, 36 bytes each — the row's position is the document's id |
//! | `postings` | (partition key, term) | the list: `n`, block count, a directory of `(first id, offset)`, then per posting a varint id gap (none for a block's first) and a varint tf |
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
//! `docs` is read into memory when a [`View`] opens; `postings` is read a term at a time and
//! never held. (Before version 2 the lengths were a `doclen` row per document; the measurements
//! below are of that layout.) That split was measured rather than designed. A scorer asks `doc_len` once per
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
//! became resident, which is the whole of the change.
//!
//! Version 2, the dense id, measured against version 1 on one machine on one day
//! (`tests/postings_timing.rs`, 155,510 units, `--release`):
//!
//! | | version 1 | version 2 |
//! |---|---|---|
//! | this file | 539.5 MB long, 327.4 MB allocated | 84.5 MB long, 42.5 MB allocated |
//! | view opened in | 17.5 ms | 1.5 ms |
//! | index written in | 646 ms | 405 ms |
//! | query p95 | 27.2 ms | 27.0 ms |
//!
//! The p95 was 36.8 ms in version 2's first form, which decoded a list block by block into a
//! vector each and re-sorted partitions' runs that were already sorted; decoding in one pass and
//! merging the runs is what brought it back. It is affordable because a length is 36
//! bytes a unit against a postings list's 1.1 KB — 5.6 MB at 155k units and 36 MB at 10⁶,
//! against the gigabyte that made this file worth having. So the table that is asked per posting is
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
///
/// 2 is the dense id: the `docs` blob in place of a `doclen` row per document, and postings
/// lists of ranks in place of uids. A file of another version is a cache of a format this build
/// does not write, and [`Db::open`] replaces it rather than refusing.
pub const VERSION: u32 = 2;

const META: TableDefinition<'static, &str, &[u8]> = TableDefinition::new("meta");
const PARTITIONS: TableDefinition<'static, &[u8], &[u8]> = TableDefinition::new("partitions");
const POSTINGS: TableDefinition<'static, (&[u8], &str), &[u8]> = TableDefinition::new("postings");
const DOCS: TableDefinition<'static, &[u8], &[u8]> = TableDefinition::new("docs");

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

/// A partition's documents, in uid order, as the `docs` blob holds them: an index into this is
/// a document's id.
type Docs = Vec<(Uid, u32)>;

const META_VERSION: &str = "version";
const META_STAMP: &str = "catalog";

/// A `docs` row: a uid and the document's length.
const DOC: usize = 32 + 4;

/// Postings per block of a list. A lookup decodes at most one block, and the directory costs
/// eight bytes a block, so 128 keeps the directory under a tenth of a typical list and the
/// decode under a microsecond.
pub const BLOCK: usize = 128;

/// A list's header: the posting count and the block count, four bytes each.
const HEAD: usize = 8;

/// One directory entry: the block's first id and its data offset, four bytes each.
const DIR: usize = 8;

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

fn put_varint(out: &mut Vec<u8>, mut v: u32) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// A LEB128 `u32` at `*at`, advancing past it; `None` for a truncated or over-long one.
fn get_varint(b: &[u8], at: &mut usize) -> Option<u32> {
    let mut v: u32 = 0;
    for shift in [0u32, 7, 14, 21, 28] {
        let byte = *b.get(*at)?;
        *at += 1;
        if shift == 28 && byte > 0x0f {
            return None;
        }
        v |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// A partition's documents as the `docs` blob: `(uid, length)` rows in uid order.
fn encode_docs<'a>(lens: impl Iterator<Item = (&'a Uid, u32)>) -> Vec<u8> {
    let mut out = Vec::new();
    for (uid, len) in lens {
        out.extend_from_slice(uid.as_bytes());
        out.extend_from_slice(&len.to_be_bytes());
    }
    out
}

/// The `docs` blob back, refusing one whose uids are not strictly ascending — the order is what
/// makes a row's position an id, so a blob out of order would map every list to the wrong
/// documents rather than fail.
fn decode_docs(b: &[u8]) -> Result<Docs, DbError> {
    if b.len() % DOC != 0 {
        return Err(DbError::Malformed("a documents row of partial documents"));
    }
    let mut out: Docs = Vec::with_capacity(b.len() / DOC);
    for chunk in b.chunks_exact(DOC) {
        let mut uid = [0u8; 32];
        uid.copy_from_slice(&chunk[..32]);
        let uid = Uid::from_bytes(uid);
        if out.last().is_some_and(|(last, _)| *last >= uid) {
            return Err(DbError::Malformed("documents out of uid order"));
        }
        out.push((
            uid,
            u32::from_be_bytes([chunk[32], chunk[33], chunk[34], chunk[35]]),
        ));
    }
    Ok(out)
}

/// A postings list as ranks: `list` in uid order, `ids` each uid's rank in its partition.
///
/// A uid with no rank is a posting for a document the partition does not count, which the
/// in-memory table never holds; it is refused rather than dropped, because a write that dropped
/// it would store a table the gate then reports as different from the one it was given.
fn encode_list(list: &[(Uid, u32)], ids: &BTreeMap<Uid, u32>) -> Result<Vec<u8>, DbError> {
    let blocks = list.len().div_ceil(BLOCK);
    let mut dir = Vec::with_capacity(blocks * DIR);
    let mut data = Vec::with_capacity(list.len() * 2);
    let mut prev = 0u32;
    for (i, (uid, tf)) in list.iter().enumerate() {
        let id = *ids.get(uid).ok_or(DbError::Malformed(
            "a posting for a document with no length",
        ))?;
        if i % BLOCK == 0 {
            dir.extend_from_slice(&id.to_be_bytes());
            dir.extend_from_slice(&(data.len() as u32).to_be_bytes());
        } else {
            put_varint(&mut data, id - prev);
        }
        put_varint(&mut data, *tf);
        prev = id;
    }
    let mut out = Vec::with_capacity(HEAD + dir.len() + data.len());
    out.extend_from_slice(&(list.len() as u32).to_be_bytes());
    out.extend_from_slice(&(blocks as u32).to_be_bytes());
    out.extend_from_slice(&dir);
    out.extend_from_slice(&data);
    Ok(out)
}

/// A list's posting count, from its header.
fn df_of(b: &[u8]) -> u32 {
    be32(b, 0).unwrap_or(0)
}

/// The postings of block `k` of a list, as `(id, tf)`, checked against the header and against
/// the partition's document count.
fn block_of(b: &[u8], k: usize, docs: usize) -> Result<Vec<(u32, u32)>, DbError> {
    let bad = |what| DbError::Malformed(what);
    let n = be32(b, 0).ok_or(bad("a postings list with no header"))? as usize;
    let blocks = be32(b, 4).ok_or(bad("a postings list with no header"))? as usize;
    if blocks != n.div_ceil(BLOCK) || k >= blocks {
        return Err(bad("a postings list whose blocks do not match its count"));
    }
    let start = HEAD + blocks * DIR;
    let entry = HEAD + k * DIR;
    let first = be32(b, entry).ok_or(bad("a truncated block directory"))?;
    let offset = be32(b, entry + 4).ok_or(bad("a truncated block directory"))? as usize;
    let data = b.get(start..).ok_or(bad("a truncated block directory"))?;
    let mut at = offset;
    let len = BLOCK.min(n - k * BLOCK);
    let mut out = Vec::with_capacity(len);
    let mut id = first;
    for i in 0..len {
        if i > 0 {
            let gap = get_varint(data, &mut at).ok_or(bad("a truncated postings block"))?;
            if gap == 0 {
                return Err(bad("a postings list out of id order"));
            }
            id = id
                .checked_add(gap)
                .ok_or(bad("a postings list out of id order"))?;
        }
        if id as usize >= docs {
            return Err(bad("a posting for an id past the partition's documents"));
        }
        let tf = get_varint(data, &mut at).ok_or(bad("a truncated postings block"))?;
        out.push((id, tf));
    }
    Ok(out)
}

/// A whole list back as `(uid, tf)` in uid order, through the partition's documents.
///
/// One pass over the data rather than [`block_of`] per block: a query reads every posting of
/// every term it names, so this is the scorer's hot path, and the blocks are laid out in order,
/// so the directory is needed only for each block's first id and to check that the offsets say
/// what the pass found.
fn decode_list(b: &[u8], docs: &Docs) -> Result<Vec<(Uid, u32)>, DbError> {
    let bad = DbError::Malformed;
    let n = be32(b, 0).ok_or(bad("a postings list with no header"))? as usize;
    let blocks = be32(b, 4).ok_or(bad("a postings list with no header"))? as usize;
    if blocks != n.div_ceil(BLOCK) {
        return Err(bad("a postings list whose blocks do not match its count"));
    }
    let data = b
        .get(HEAD + blocks * DIR..)
        .ok_or(bad("a truncated block directory"))?;
    let mut out = Vec::with_capacity(n);
    let mut at = 0usize;
    let mut id = 0u32;
    for i in 0..n {
        if i % BLOCK == 0 {
            let entry = HEAD + (i / BLOCK) * DIR;
            let first = be32(b, entry).ok_or(bad("a truncated block directory"))?;
            if be32(b, entry + 4) != Some(at as u32) || (i > 0 && first <= id) {
                return Err(bad("a block directory that disagrees with its data"));
            }
            id = first;
        } else {
            let gap = get_varint(data, &mut at).ok_or(bad("a truncated postings block"))?;
            if gap == 0 {
                return Err(bad("a postings list out of id order"));
            }
            id = id
                .checked_add(gap)
                .ok_or(bad("a postings list out of id order"))?;
        }
        let uid = docs
            .get(id as usize)
            .ok_or(bad("a posting for an id past the partition's documents"))?
            .0;
        let tf = get_varint(data, &mut at).ok_or(bad("a truncated postings block"))?;
        out.push((uid, tf));
    }
    Ok(out)
}

/// One posting's term frequency, found without decoding the list.
///
/// A binary search over the block directory's first ids, then a decode of the one block that
/// can hold `id`. It is what makes [`View::tf`] cheap enough to decompose a score with: the
/// seam's default answer materialises a list that can be as long as the corpus. A malformed
/// list answers 0, as a missing one does; the file is a cache, and its reader rebuilds.
fn tf_in(b: &[u8], id: u32, docs: usize) -> u32 {
    let Some(blocks) = be32(b, 4) else { return 0 };
    let (mut lo, mut hi) = (0usize, blocks as usize);
    // The last block whose first id is at most `id`.
    while lo < hi {
        let mid = (lo + hi) / 2;
        match be32(b, HEAD + mid * DIR) {
            Some(first) if first <= id => lo = mid + 1,
            Some(_) => hi = mid,
            None => return 0,
        }
    }
    let Some(k) = lo.checked_sub(1) else { return 0 };
    match block_of(b, k, docs) {
        Ok(block) => block
            .iter()
            .find(|(i, _)| *i == id)
            .map_or(0, |(_, tf)| *tf),
        Err(_) => 0,
    }
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
            // A cache in a format this build does not write: replaced, as a stale stamp is, and
            // not refused — a refusal here would leave the file in place and every later write
            // refused with it. Only a file that cannot be removed is still an error.
            Some(other) => {
                drop(me);
                std::fs::remove_file(&target).map_err(|_| DbError::UnsupportedVersion(other))?;
                let me = Db {
                    db: Database::create(&target).map_err(backend)?,
                };
                me.set_version()?;
                return Ok(me);
            }
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
    /// the whole table in memory at this point, and it is what makes the dense id free: a
    /// partition is re-ranked on every write because every write rewrites it.
    pub fn write(&self, table: &Table, len: u64, hash: &[u8; 32]) -> Result<(), DbError> {
        let mut txn = self.db.begin_write().map_err(backend)?;
        txn.set_durability(Durability::Immediate);
        {
            let mut meta = txn.open_table(META).map_err(backend)?;
            let mut parts = txn.open_table(PARTITIONS).map_err(backend)?;
            let mut post = txn.open_table(POSTINGS).map_err(backend)?;
            let mut docs = txn.open_table(DOCS).map_err(backend)?;

            // The stamp goes last in intent and first in code: it is cleared here so that a
            // transaction that fails part-way leaves no file claiming to describe a catalog.
            meta.remove(META_STAMP).map_err(backend)?;
            parts.retain(|_, _| false).map_err(backend)?;
            post.retain(|_, _| false).map_err(backend)?;
            docs.retain(|_, _| false).map_err(backend)?;

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
                // The ranks: `lengths` is in uid order, so a document's position is its id.
                let ids: BTreeMap<Uid, u32> = row
                    .lengths()
                    .enumerate()
                    .map(|(i, (uid, _))| (*uid, i as u32))
                    .collect();
                docs.insert(key.as_slice(), encode_docs(row.lengths()).as_slice())
                    .map_err(backend)?;
                for (term, list) in row.iter() {
                    post.insert((key.as_slice(), term), encode_list(list, &ids)?.as_slice())
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
        let txn = self.db.begin_read().map_err(backend)?;
        // The documents first: a list is ranks, and a rank means nothing without them.
        let mut docs: BTreeMap<Vec<u8>, Docs> = BTreeMap::new();
        match txn.open_table(DOCS) {
            Ok(t) => {
                for entry in t.iter().map_err(backend)? {
                    let (k, v) = entry.map_err(backend)?;
                    docs.insert(k.value().to_vec(), decode_docs(v.value())?);
                }
            }
            Err(redb::TableError::TableDoesNotExist(_)) => {}
            Err(e) => return Err(backend(e)),
        }
        let mut terms: TermsByPartition = BTreeMap::new();
        match txn.open_table(POSTINGS) {
            Ok(t) => {
                for entry in t.iter().map_err(backend)? {
                    let (k, v) = entry.map_err(backend)?;
                    let (key, term) = k.value();
                    let of = docs.get(key).ok_or(DbError::Malformed(
                        "a term under a partition with no documents",
                    ))?;
                    terms
                        .entry(key.to_vec())
                        .or_default()
                        .insert(term.to_string(), decode_list(v.value(), of)?);
                }
            }
            Err(redb::TableError::TableDoesNotExist(_)) => {}
            Err(e) => return Err(backend(e)),
        }
        let known: std::collections::BTreeSet<&Vec<u8>> = rows.iter().map(|r| &r.key).collect();
        for key in terms.keys().chain(docs.keys()) {
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
                    docs.remove(&row.key)
                        .unwrap_or_default()
                        .into_iter()
                        .collect(),
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
        let docs = self.docs_of(&txn, &parts)?;
        Ok(View {
            parts,
            postings,
            docs,
        })
    }

    /// The documents of each partition named, in memory, in the order `parts` names them.
    ///
    /// **The one table a view hydrates**, and the measurement is the argument. A scorer asks
    /// `doc_len` once per *posting*, so a common term costs one b-tree descent per document —
    /// 120 ms a query at 155k units against 35 ms for the same arithmetic over resident maps,
    /// and the one query in the harness whose term has a single posting came back in 0.88 ms,
    /// which is what says the cost is per posting rather than per lookup. Since version 2 it is
    /// also what turns a rank back into a uid.
    ///
    /// It is affordable for the reason the postings lists are not: a document is 36 bytes
    /// against a list's share of 1.1 KB — 5.6 MB at 155k units and 36 MB at 10⁶. So the table
    /// that is asked per posting is resident and the table that is most of the bytes stays on
    /// disk, which is the split the file exists for.
    fn docs_of(&self, txn: &redb::ReadTransaction, parts: &[Row]) -> Result<Vec<Docs>, DbError> {
        let table = match txn.open_table(DOCS) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => {
                return Ok(vec![Vec::new(); parts.len()])
            }
            Err(e) => return Err(backend(e)),
        };
        parts
            .iter()
            .map(|r| match table.get(r.key.as_slice()).map_err(backend)? {
                Some(v) => decode_docs(v.value()),
                None => Ok(Vec::new()),
            })
            .collect()
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
            let Some(at) = rows.iter().position(|r| r.key == key) else {
                return Err(format!("the store has no partition `{partition}`"));
            };
            let stored = &rows[at];
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
                    .list(at, term)
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
        let held: usize = view.docs.iter().map(Vec::len).sum();
        if held != want {
            return Err(format!(
                "the store holds {held} document length(s) and the table {want}"
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
    /// Each partition's documents in uid order, beside `parts`, read when the view opened: the
    /// lengths a scorer asks for, and the uid of every rank a list holds. See [`Db::docs_of`]
    /// for why this one table is in memory and the postings lists are not.
    docs: Vec<Docs>,
}

impl std::fmt::Debug for View {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("View")
            .field("partitions", &self.parts.len())
            .field("documents", &self.docs.iter().map(Vec::len).sum::<usize>())
            .finish_non_exhaustive()
    }
}

impl View {
    /// How many partitions this view spans.
    pub fn partitions(&self) -> usize {
        self.parts.len()
    }

    /// Partition `at`'s list for `term`, in uids.
    fn list(&self, at: usize, term: &str) -> Result<Option<Vec<(Uid, u32)>>, DbError> {
        let Some(table) = self.postings.as_ref() else {
            return Ok(None);
        };
        match table
            .get((self.parts[at].key.as_slice(), term))
            .map_err(backend)?
        {
            Some(v) => Ok(Some(decode_list(v.value(), &self.docs[at])?)),
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
    /// point: `df` reads the header and `tf` searches the block directory, and a postings list
    /// in this corpus can be a megabyte. Returning a `Vec` is what the first version did,
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
            .map(|r| self.with_bytes(&r.key, term, df_of).unwrap_or(0))
            .sum()
    }

    fn postings(&self, term: &str) -> Vec<(Uid, u32)> {
        let mut out: Vec<(Uid, u32)> = (0..self.parts.len())
            .filter_map(|at| self.list(at, term).ok().flatten())
            .flatten()
            .collect();
        // Uid order across partitions, as the seam requires and as the in-memory view does.
        // Stable, because each partition's run is already in uid order and a merge sort finds
        // the runs: the whole cost is the merge.
        out.sort_by_key(|a| a.0);
        out
    }

    /// A map hit, as the in-memory view's is. A unit is in exactly one partition, so the lengths
    /// of the partitions this view spans are one map with no ambiguity in it.
    fn doc_len(&self, uid: &Uid) -> Option<u32> {
        self.docs.iter().find_map(|d| {
            d.binary_search_by_key(uid, |(u, _)| *u)
                .ok()
                .map(|i| d[i].1)
        })
    }

    /// Searched in the stored bytes of the one partition holding `uid`, never through the
    /// merged list: the uid's rank there, then the block that can hold it.
    fn tf(&self, term: &str, uid: &Uid) -> u32 {
        self.parts
            .iter()
            .zip(&self.docs)
            .find_map(|(r, d)| {
                let id = d.binary_search_by_key(uid, |(u, _)| *u).ok()?;
                self.with_bytes(&r.key, term, |b| tf_in(b, id as u32, d.len()))
            })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `n` documents with uids in ascending order, each its own length.
    fn docs(n: u32) -> Docs {
        (0..n)
            .map(|i| {
                let mut b = [0u8; 32];
                b[28..].copy_from_slice(&i.to_be_bytes());
                (Uid::from_bytes(b), i + 1)
            })
            .collect()
    }

    fn ids(d: &Docs) -> BTreeMap<Uid, u32> {
        d.iter()
            .enumerate()
            .map(|(i, (u, _))| (*u, i as u32))
            .collect()
    }

    /// A list over every `step`th document, with term frequencies that need more than one varint
    /// byte now and then, long enough to span several blocks and end inside one.
    fn list(d: &Docs, step: usize) -> Vec<(Uid, u32)> {
        d.iter()
            .step_by(step)
            .enumerate()
            .map(|(i, (u, _))| (*u, (i as u32 * 37) % 300 + 1))
            .collect()
    }

    #[test]
    fn a_list_round_trips_across_blocks_and_gaps_of_every_width() {
        let d = docs(100_000);
        for step in [1, 3, 200, 20_000] {
            let l = list(&d, step);
            let b = encode_list(&l, &ids(&d)).unwrap();
            assert_eq!(df_of(&b) as usize, l.len());
            assert_eq!(decode_list(&b, &d).unwrap(), l, "step {step}");
        }
        assert_eq!(
            decode_list(&encode_list(&[], &ids(&d)).unwrap(), &d).unwrap(),
            vec![]
        );
    }

    #[test]
    fn a_dense_list_is_a_few_bytes_a_posting() {
        let d = docs(100_000);
        let l = list(&d, 1);
        let b = encode_list(&l, &ids(&d)).unwrap();
        // One byte of gap and at most two of tf, and the directory's eight bytes a block.
        assert!(b.len() < l.len() * 3, "{} bytes for {}", b.len(), l.len());
        assert!(b.len() * 12 < l.len() * 36, "not a twelfth of the uid form");
    }

    #[test]
    fn every_posting_s_tf_is_found_and_no_other_document_has_one() {
        let d = docs(5_000);
        let l = list(&d, 7);
        let b = encode_list(&l, &ids(&d)).unwrap();
        let held: BTreeMap<Uid, u32> = l.iter().copied().collect();
        for (i, (uid, _)) in d.iter().enumerate() {
            assert_eq!(
                tf_in(&b, i as u32, d.len()),
                held.get(uid).copied().unwrap_or(0),
                "id {i}"
            );
        }
        assert_eq!(tf_in(&b, d.len() as u32 + 5, d.len()), 0);
    }

    #[test]
    fn a_posting_for_a_document_with_no_length_is_refused() {
        let d = docs(10);
        let stranger = (Uid::from_bytes([0xff; 32]), 1);
        assert!(encode_list(&[stranger], &ids(&d)).is_err());
    }

    #[test]
    fn documents_out_of_order_are_malformed() {
        let d = docs(3);
        let mut b = encode_docs(d.iter().map(|(u, l)| (u, *l)));
        assert_eq!(decode_docs(&b).unwrap(), d);
        b.swap(31, 31 + DOC);
        assert!(decode_docs(&b).is_err());
        assert!(decode_docs(&b[..DOC + 1]).is_err());
    }

    /// The file is a cache, and a cache read from a damaged file must say so rather than panic:
    /// every truncation of a list, and every single-byte change to it, decodes to an error or to
    /// a list, and looks up to a number.
    #[test]
    fn a_damaged_list_never_panics() {
        let d = docs(1_000);
        let b = encode_list(&list(&d, 2), &ids(&d)).unwrap();
        for cut in 0..b.len() {
            let _ = decode_list(&b[..cut], &d);
            let _ = tf_in(&b[..cut], 10, d.len());
            let _ = df_of(&b[..cut]);
        }
        for at in 0..b.len() {
            for flip in [0x01u8, 0x80, 0xff] {
                let mut c = b.clone();
                c[at] ^= flip;
                let _ = decode_list(&c, &d);
                let _ = tf_in(&c, 10, d.len());
            }
        }
    }

    #[test]
    fn a_varint_is_leb128_and_an_overlong_one_is_refused() {
        for v in [0, 1, 127, 128, 16_383, 16_384, u32::MAX] {
            let mut b = Vec::new();
            put_varint(&mut b, v);
            let mut at = 0;
            assert_eq!(get_varint(&b, &mut at), Some(v));
            assert_eq!(at, b.len());
        }
        let mut at = 0;
        assert_eq!(get_varint(&[0xff, 0xff, 0xff, 0xff, 0x1f], &mut at), None);
        let mut at = 0;
        assert_eq!(get_varint(&[0x80], &mut at), None);
    }

    #[test]
    fn a_file_of_another_version_is_replaced_rather_than_refused() {
        let dir = std::env::temp_dir().join(format!("smysl-db-version-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        {
            let db = Db::open(&dir).unwrap();
            let mut txn = db.db.begin_write().unwrap();
            {
                let mut meta = txn.open_table(META).unwrap();
                meta.insert(META_VERSION, &1u32.to_be_bytes()[..]).unwrap();
            }
            txn.set_durability(Durability::Immediate);
            txn.commit().unwrap();
        }
        let db = Db::open(&dir).expect("an old cache is replaced");
        assert_eq!(db.version().unwrap(), Some(VERSION));
        assert_eq!(db.stamp().unwrap(), None, "and starts empty");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
