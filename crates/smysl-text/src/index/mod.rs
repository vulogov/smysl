//! The substrate index: the derived tables of RFC SMYSL-2.4 §3.8, and the two builders each
//! one has.
//!
//! **Never authoritative.** The catalog log and the object store are the truth; this is a
//! cache, so that opening a library does not mean reading every reading object and rebuilding
//! every structure tree to answer where one locator points. [`crate::library::Library::resolve`]
//! said as much until this module existed — "searching rather than indexing is what keeps this
//! correct while `text ls` is the only index there is" — and the search it describes is
//! `O(parts)` object reads per locator.
//!
//! # The two builders
//!
//! §3.8 asks for a `rebuild(&Library)` and an `on_append(&[Record])` per table, and for a test
//! that the two produce the same bytes. Both are here, in [`tables`] and [`journal`], and
//! [`Index::to_bytes`] is the dump the test compares. The gate is the same one SM-P3 set for
//! `smysl_graph::store::index`: an index rebuilt from the sources alone is byte-identical to
//! one maintained while the library grew. Anything else means the two paths disagree about the
//! corpus, which is the sort of divergence that only shows up under load.
//!
//! `on_append` takes the library as well as the records, which §3.8's signature does not. It
//! has to: `parts`, `segments` and `locators` are built from records **15** and **18**, and
//! OQ-39's answer is that a log never holds one — they live in the object store, addressed by
//! the identities a manifest records. A builder handed only the appended records could keep
//! `manifests`, `by_part` and `redactions` current and would have no way to reach the other
//! three. The records say *what changed*; the library is where the changed thing is read from.
//!
//! # Which tables are here
//!
//! [`Table`] names all twelve of §3.8 and says which TX-P4 step builds each, on the pattern
//! `smysl_check::Pass` set: a table this build does not fill reports itself as not built
//! rather than as an empty answer. Step 1 builds seven; `postings` is step 2, the three range
//! tables are step 3, and `align` is step 4.
//!
//! # Partial indexes
//!
//! A rebuild reads objects, and an object can be missing — rule Z deletes the objects of a
//! redacted part — or corrupt (`SMY-E446`). Neither may fail an open, because a derived cache
//! that can refuse to open a library is worse than no cache. So a rebuild is **lenient**, and
//! completeness is tracked per manifest: [`Index::locators`] holds a table for a mid only when
//! every part of that mid resolved into it, and `resolve` falls back to the search for a mid
//! that is not there. An index can therefore be half-built without ever being half-trusted.

pub mod journal;
pub mod tables;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use smysl_core::ids::{Mid, Rdid, Tid};
use smysl_core::types::Record;

use crate::library::Library;

/// File magic. Eight bytes, so the header is aligned and greppable.
pub const MAGIC: &[u8; 8] = b"SMYSLSIX";

/// Sidecar format version.
///
/// Independent of the wire format, and of `smysl_graph::store::index::VERSION`: this index is
/// derived, so bumping it costs a rebuild and nothing else.
pub const VERSION: u16 = 1;

/// The directory §4.1's layout reserves for the index, relative to the library root.
pub const DIR: &str = "index";

/// The sidecar, relative to the library root.
///
/// `index/substrate.idx` and not `index/index.redb`: the file the redb backing will write is
/// a different file in the same reserved directory, so a library can carry both without one
/// name meaning two formats. See the module note in [`tables`] on why the backing is separate
/// from the tables.
pub const SIDECAR: &str = "index/substrate.idx";

/// The tables of §3.8, and which TX-P4 step builds each.
///
/// All twelve are named here and only seven are built, for the reason `smysl_check::Pass`
/// names thirteen passes and runs ten: a caller that asks about a table this build does not
/// fill should be told that, not handed an empty table that reads as an empty corpus. The four
/// SMYSL-2.5 and SMYSL-2.7 tables (`vectors`, `queries`, `imported`, `waiting`, `schemas`) are
/// not here at all — they belong to RFCs this one does not implement, and naming them would
/// claim a plan this phase does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Table {
    /// Every manifest, with what a catalog listing asks for and whether it is a head.
    Manifests,
    /// Every part any manifest records, with the length and the identities recorded for it.
    Parts,
    /// Which manifests contain a part.
    ByPart,
    /// How many redactions name a part (rule Z, record 19).
    Redactions,
    /// Every reading, by rdid: which part, which reader, how many rows, which levels.
    Segments,
    /// Where a locator points, per manifest. The table that makes `resolve` an index.
    Locators,
    /// Versification alignment (TX-P4 step 4; needs OQ-4 settled for shipped maps).
    Align,
    /// `said` range table (TX-P4 step 3).
    Said,
    /// `composed` range table (TX-P4 step 3).
    Composed,
    /// `observed` range table (TX-P4 step 3).
    Observed,
    /// Persistent postings per partition (TX-P4 step 2).
    Postings,
    /// The operational journal: which units came out of which part.
    Journal,
}

impl Table {
    /// All twelve, in the order §3.8 lists them.
    pub const ALL: &'static [Table] = &[
        Table::Manifests,
        Table::Parts,
        Table::ByPart,
        Table::Redactions,
        Table::Segments,
        Table::Locators,
        Table::Align,
        Table::Said,
        Table::Composed,
        Table::Observed,
        Table::Postings,
        Table::Journal,
    ];

    /// The tables this build fills.
    pub const BUILT: &'static [Table] = &[
        Table::Manifests,
        Table::Parts,
        Table::ByPart,
        Table::Redactions,
        Table::Segments,
        Table::Locators,
        Table::Journal,
    ];

    /// The name §3.8 gives the table, which is also what `text index` prints.
    pub const fn as_str(self) -> &'static str {
        match self {
            Table::Manifests => "manifests",
            Table::Parts => "parts",
            Table::ByPart => "by-part",
            Table::Redactions => "redactions",
            Table::Segments => "segments",
            Table::Locators => "locators",
            Table::Align => "align",
            Table::Said => "said",
            Table::Composed => "composed",
            Table::Observed => "observed",
            Table::Postings => "postings",
            Table::Journal => "journal",
        }
    }

    /// Which TX-P4 step builds this table (RFC SMYSL-2.4 §6).
    pub const fn step(self) -> u8 {
        match self {
            Table::Manifests
            | Table::Parts
            | Table::ByPart
            | Table::Redactions
            | Table::Segments
            | Table::Locators
            | Table::Journal => 1,
            Table::Postings => 2,
            Table::Said | Table::Composed | Table::Observed => 3,
            Table::Align => 4,
        }
    }

    /// Whether this build fills the table.
    pub fn is_built(self) -> bool {
        Table::BUILT.contains(&self)
    }

    pub fn parse(s: &str) -> Option<Table> {
        Table::ALL.iter().copied().find(|t| t.as_str() == s)
    }
}

impl core::fmt::Display for Table {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What the `manifests` table holds for one manifest.
///
/// A summary and not the manifest: the manifest is in the log and the store already holds it
/// parsed. What a catalog listing asks for is the five fields it prints and whether anything
/// supersedes this one, and `head` is the field worth being careful about — it is a fact about
/// *other* manifests, so it moves when one arrives. [`tables::manifests_on_append`] is where
/// that is handled, and `the_arrival_of_a_successor_unseats_a_head` is the test.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct ManifestRow {
    pub alias: String,
    pub lang: String,
    pub reader: String,
    pub licence: String,
    /// The sum of the part lengths, in normalised bytes.
    pub length: u64,
    pub parts: u32,
    /// Whether no other manifest supersedes this one.
    pub head: bool,
}

/// One manifest's record of a part: the length it was recorded at and the two identities
/// derived over it.
///
/// Sorted as a tuple so that a set of these has an order that does not depend on which
/// manifest arrived first — which is what [`PartRow`] needs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct PartEntryRow {
    pub length: u64,
    pub structure: [u8; 32],
    pub rdid: Rdid,
}

/// What the `parts` table holds for one part: a **set** of what the manifests record for it.
///
/// A set and not one row, for the reason `Store::parts` keeps a set of lengths: a tid recorded
/// by two manifests that disagree about its length, its structure hash or its reading appears
/// with both records, because that disagreement is what a check is for and collapsing it here
/// would hide it.
///
/// It is also what makes the two builders agree. One row would have to be *some* manifest's,
/// and a rebuild walks manifests in mid order while an append walks them in arrival order — so
/// last-wins would put a different row in the table depending on which path filled it, and
/// §3.8's byte comparison is exactly the gate that catches that. A set has no last.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct PartRow {
    /// Sorted and deduplicated.
    pub entries: Vec<PartEntryRow>,
}

impl PartRow {
    /// Add one manifest's record of the part, keeping the set sorted and deduplicated.
    pub fn absorb(&mut self, entry: PartEntryRow) {
        if let Err(at) = self.entries.binary_search(&entry) {
            self.entries.insert(at, entry);
        }
    }

    /// Every length a manifest records for this part, which is the question `SMY-E404` asks.
    pub fn lengths(&self) -> Vec<u64> {
        let mut out: Vec<u64> = self.entries.iter().map(|e| e.length).collect();
        out.dedup();
        out
    }
}

/// What the `segments` table holds for one reading.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct SegmentsRow {
    pub tid: Tid,
    pub reader: String,
    pub rows: u32,
    /// The distinct level names, sorted — what `text show --segments` offers to filter by.
    pub levels: Vec<String>,
}

/// Where a locator points inside a manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct Where {
    pub tid: Tid,
    pub start: u64,
    pub end: u64,
}

/// One manifest's locator table: every address its parts answer to.
pub type LocatorTable = BTreeMap<String, Where>;

/// The derived substrate index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Index {
    /// Bytes of the catalog log this index describes.
    pub catalog_len: u64,
    /// BLAKE3 of those bytes.
    pub catalog_hash: [u8; 32],
    /// Whether the two tables built by **reading objects** — `segments` and `locators` — were
    /// built at all.
    ///
    /// The one field that is about the index rather than about the corpus, and it is here
    /// because opening a library may not afford them. Five of the seven tables are derived from
    /// the catalog, which is already parsed and in memory, so building them on open costs
    /// arithmetic. The other two cost **one object read and one structure tree per part**, which
    /// on a corpus of five Bibles cut at the measured 1 KiB minimum is tens of thousands of
    /// reads — an unacceptable price for `text ls`, which wants none of it.
    ///
    /// So [`Index::rebuild_catalog`] is what an open does and [`Index::rebuild`] is what
    /// `text index --rebuild` and the writing commands do, and this field is what keeps the
    /// difference from being silent: a partial index reports its two object tables as not built
    /// rather than as empty, refuses to be persisted, and answers no locator question — which
    /// sends `resolve` to the search, the answer it gave before this module existed.
    pub objects_read: bool,
    pub manifests: BTreeMap<Mid, ManifestRow>,
    pub parts: BTreeMap<Tid, PartRow>,
    pub by_part: BTreeMap<Tid, Vec<Mid>>,
    pub redactions: BTreeMap<Tid, u32>,
    pub segments: BTreeMap<Rdid, SegmentsRow>,
    /// Locator tables, by manifest. A mid is present only when **every** part of it resolved,
    /// so a lookup that misses is a lookup to make the other way rather than a locator that
    /// names nothing.
    pub locators: BTreeMap<Mid, LocatorTable>,
    pub journal: Vec<journal::Row>,
}

/// Why a sidecar could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IndexError {
    BadMagic,
    UnsupportedVersion(u16),
    Truncated,
    Malformed(&'static str),
}

impl core::fmt::Display for IndexError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            IndexError::BadMagic => f.write_str("not a smysl substrate index"),
            IndexError::UnsupportedVersion(v) => write!(f, "index version {v} is not supported"),
            IndexError::Truncated => f.write_str("index ends mid-table"),
            IndexError::Malformed(what) => write!(f, "malformed index: {what}"),
        }
    }
}

impl std::error::Error for IndexError {}

impl Index {
    /// Build every table of step 1 from the library, reading nothing but the catalog and the
    /// objects.
    ///
    /// Lenient by construction: see the module note on partial indexes. Nothing here returns
    /// an error, because every caller is on a path that must not fail for a cache.
    pub fn rebuild(lib: &Library) -> Index {
        let mut ix = Index::rebuild_catalog(lib);
        ix.objects_read = true;
        ix.segments = tables::segments_rebuild(lib);
        ix.locators = tables::locators_rebuild(lib);
        ix
    }

    /// Build only the tables derived from the catalog, reading no object.
    ///
    /// What opening a library does. See [`Index::objects_read`] for why the split exists: these
    /// five are arithmetic over a log that is already parsed, and the two left out cost I/O per
    /// part.
    pub fn rebuild_catalog(lib: &Library) -> Index {
        Index {
            catalog_len: lib.store().log_len(),
            catalog_hash: *lib.store().log_hash(),
            objects_read: false,
            manifests: tables::manifests_rebuild(lib),
            parts: tables::parts_rebuild(lib),
            by_part: tables::by_part_rebuild(lib),
            redactions: tables::redactions_rebuild(lib),
            segments: BTreeMap::new(),
            locators: BTreeMap::new(),
            journal: journal::rebuild(lib),
        }
    }

    /// Whether both object-derived tables were built.
    pub fn is_complete(&self) -> bool {
        self.objects_read
    }

    /// Bring every table forward over records that were just appended.
    ///
    /// The library is already holding them: this runs *after* the append, so that a table
    /// built by reading the library sees the same library the rebuild would. Taking the
    /// records as well is what keeps it `O(batch)` rather than `O(corpus)` — they say which
    /// mid, which tid and which rdid moved.
    pub fn on_append(&mut self, lib: &Library, records: &[Record]) {
        self.catalog_len = lib.store().log_len();
        self.catalog_hash = *lib.store().log_hash();
        tables::manifests_on_append(&mut self.manifests, lib, records);
        tables::parts_on_append(&mut self.parts, lib, records);
        tables::by_part_on_append(&mut self.by_part, lib, records);
        tables::redactions_on_append(&mut self.redactions, lib, records);
        journal::on_append(&mut self.journal, lib, records);
        // The two object tables are brought forward only when they exist. Absorbing an append
        // into a table that was never built would make it hold the parts added since the open
        // and not the ones that were there first - which is the one state the completeness flag
        // is for, since a half-built locator table cannot be told from a complete one at
        // lookup time.
        if self.objects_read {
            tables::segments_on_append(&mut self.segments, lib, records);
            tables::locators_on_append(&mut self.locators, lib, records);
        }
    }

    /// Whether this index describes the catalog it was opened beside.
    ///
    /// A stale index is a rebuild, never a failure.
    pub fn matches(&self, catalog_len: u64, catalog_hash: &[u8; 32]) -> bool {
        self.catalog_len == catalog_len && self.catalog_hash == *catalog_hash
    }

    /// How many rows a table holds, or `None` when this build does not fill it.
    ///
    /// `None` rather than `0`: a table nobody built and a table with nothing in it are
    /// different sentences, and `text index` prints them differently.
    pub fn rows(&self, table: Table) -> Option<usize> {
        if !table.is_built() {
            return None;
        }
        // A partial index says so about the two tables it did not build, rather than reporting
        // them empty. `segments 0` on a library full of readings is a false sentence.
        if !self.objects_read && matches!(table, Table::Segments | Table::Locators) {
            return None;
        }
        Some(match table {
            Table::Manifests => self.manifests.len(),
            Table::Parts => self.parts.len(),
            Table::ByPart => self.by_part.len(),
            Table::Redactions => self.redactions.len(),
            Table::Segments => self.segments.len(),
            Table::Locators => self.locators.values().map(BTreeMap::len).sum(),
            Table::Journal => self.journal.len(),
            _ => return None,
        })
    }

    /// Where a locator points in this manifest, when the index can answer.
    ///
    /// `None` means *ask the other way*, not *nothing is there*: a mid whose parts did not all
    /// resolve has no table, by the module's completeness rule.
    pub fn resolve(&self, mid: &Mid, locator: &str) -> Option<Where> {
        self.locators.get(mid)?.get(locator).copied()
    }

    /// Whether the index can answer locator questions about this manifest at all.
    ///
    /// The question [`crate::library::Library::resolve`] asks before trusting a miss. Without
    /// it a mid with no table and a mid whose every part resolved to nothing would be the same
    /// answer, and one of those means *search*.
    pub fn has_locators(&self, mid: &Mid) -> bool {
        self.objects_read && self.locators.contains_key(mid)
    }

    /// Where the sidecar for a library lives.
    pub fn path(root: &Path) -> PathBuf {
        root.join(SIDECAR)
    }

    // -- encoding ----------------------------------------------------------

    /// The deterministic dump: big-endian, every collection in key order.
    ///
    /// This is the comparison §3.8's gate is written in terms of, which is why it is a byte
    /// string and not a `PartialEq`. Two indexes can be equal as values and disagree about
    /// what they would persist if either held a map whose iteration order were not its key
    /// order — so the gate compares what reaches the disk.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.raw(MAGIC);
        w.u16(VERSION);
        w.u64(self.catalog_len);
        w.raw(&self.catalog_hash);
        w.u8(u8::from(self.objects_read));

        w.u32(self.manifests.len() as u32);
        for (mid, row) in &self.manifests {
            w.raw(mid.as_bytes());
            w.text(&row.alias);
            w.text(&row.lang);
            w.text(&row.reader);
            w.text(&row.licence);
            w.u64(row.length);
            w.u32(row.parts);
            w.u8(u8::from(row.head));
        }

        w.u32(self.parts.len() as u32);
        for (tid, row) in &self.parts {
            w.raw(tid.as_bytes());
            w.u32(row.entries.len() as u32);
            for e in &row.entries {
                w.u64(e.length);
                w.raw(&e.structure);
                w.raw(e.rdid.as_bytes());
            }
        }

        w.u32(self.by_part.len() as u32);
        for (tid, mids) in &self.by_part {
            w.raw(tid.as_bytes());
            w.u32(mids.len() as u32);
            for mid in mids {
                w.raw(mid.as_bytes());
            }
        }

        w.u32(self.redactions.len() as u32);
        for (tid, n) in &self.redactions {
            w.raw(tid.as_bytes());
            w.u32(*n);
        }

        w.u32(self.segments.len() as u32);
        for (rdid, row) in &self.segments {
            w.raw(rdid.as_bytes());
            w.raw(row.tid.as_bytes());
            w.text(&row.reader);
            w.u32(row.rows);
            w.u32(row.levels.len() as u32);
            for l in &row.levels {
                w.text(l);
            }
        }

        w.u32(self.locators.len() as u32);
        for (mid, table) in &self.locators {
            w.raw(mid.as_bytes());
            w.u32(table.len() as u32);
            for (loc, at) in table {
                w.text(loc);
                w.raw(at.tid.as_bytes());
                w.u64(at.start);
                w.u64(at.end);
            }
        }

        w.u32(self.journal.len() as u32);
        for row in &self.journal {
            w.raw(row.tid.as_bytes());
            w.raw(row.uid.as_bytes());
            w.u8(row.status.as_u8());
        }

        w.into_bytes()
    }

    pub fn from_bytes(b: &[u8]) -> Result<Index, IndexError> {
        let mut r = Reader::new(b);
        if r.take(8)? != MAGIC {
            return Err(IndexError::BadMagic);
        }
        let version = r.u16()?;
        if version != VERSION {
            return Err(IndexError::UnsupportedVersion(version));
        }
        let catalog_len = r.u64()?;
        let catalog_hash: [u8; 32] = r.take(32)?.try_into().map_err(|_| IndexError::Truncated)?;
        let objects_read = r.u8()? != 0;

        let mut manifests = BTreeMap::new();
        for _ in 0..r.u32()? {
            let mid = Mid::from_bytes(r.id()?);
            manifests.insert(
                mid,
                ManifestRow {
                    alias: r.text()?,
                    lang: r.text()?,
                    reader: r.text()?,
                    licence: r.text()?,
                    length: r.u64()?,
                    parts: r.u32()?,
                    head: r.u8()? != 0,
                },
            );
        }

        let mut parts = BTreeMap::new();
        for _ in 0..r.u32()? {
            let tid = Tid::from_bytes(r.id()?);
            let mut entries = Vec::new();
            for _ in 0..r.u32()? {
                entries.push(PartEntryRow {
                    length: r.u64()?,
                    structure: r.id()?,
                    rdid: Rdid::from_bytes(r.id()?),
                });
            }
            parts.insert(tid, PartRow { entries });
        }

        let mut by_part = BTreeMap::new();
        for _ in 0..r.u32()? {
            let tid = Tid::from_bytes(r.id()?);
            let mut mids = Vec::new();
            for _ in 0..r.u32()? {
                mids.push(Mid::from_bytes(r.id()?));
            }
            by_part.insert(tid, mids);
        }

        let mut redactions = BTreeMap::new();
        for _ in 0..r.u32()? {
            let tid = Tid::from_bytes(r.id()?);
            redactions.insert(tid, r.u32()?);
        }

        let mut segments = BTreeMap::new();
        for _ in 0..r.u32()? {
            let rdid = Rdid::from_bytes(r.id()?);
            let tid = Tid::from_bytes(r.id()?);
            let reader = r.text()?;
            let rows = r.u32()?;
            let mut levels = Vec::new();
            for _ in 0..r.u32()? {
                levels.push(r.text()?);
            }
            segments.insert(
                rdid,
                SegmentsRow {
                    tid,
                    reader,
                    rows,
                    levels,
                },
            );
        }

        let mut locators = BTreeMap::new();
        for _ in 0..r.u32()? {
            let mid = Mid::from_bytes(r.id()?);
            let mut table = LocatorTable::new();
            for _ in 0..r.u32()? {
                let loc = r.text()?;
                table.insert(
                    loc,
                    Where {
                        tid: Tid::from_bytes(r.id()?),
                        start: r.u64()?,
                        end: r.u64()?,
                    },
                );
            }
            locators.insert(mid, table);
        }

        let mut journal = Vec::new();
        for _ in 0..r.u32()? {
            let tid = Tid::from_bytes(r.id()?);
            let uid = smysl_core::Uid::from_bytes(r.id()?);
            let status = smysl_core::Status::from_u8(r.u8()?)
                .ok_or(IndexError::Malformed("journal status"))?;
            journal.push(journal::Row { tid, uid, status });
        }

        Ok(Index {
            catalog_len,
            catalog_hash,
            objects_read,
            manifests,
            parts,
            by_part,
            redactions,
            segments,
            locators,
            journal,
        })
    }
}

// ---------------------------------------------------------------------------
// Fixed-width big-endian helpers, as `smysl_graph::store::index` writes them
// ---------------------------------------------------------------------------

struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn new() -> Writer {
        Writer { buf: Vec::new() }
    }
    fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
    fn raw(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    fn text(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.raw(s.as_bytes());
    }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Reader<'a> {
        Reader { b, at: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], IndexError> {
        let end = self.at.checked_add(n).ok_or(IndexError::Truncated)?;
        let out = self.b.get(self.at..end).ok_or(IndexError::Truncated)?;
        self.at = end;
        Ok(out)
    }
    fn u8(&mut self) -> Result<u8, IndexError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, IndexError> {
        Ok(u16::from_be_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| IndexError::Truncated)?,
        ))
    }
    fn u32(&mut self) -> Result<u32, IndexError> {
        Ok(u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| IndexError::Truncated)?,
        ))
    }
    fn u64(&mut self) -> Result<u64, IndexError> {
        Ok(u64::from_be_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| IndexError::Truncated)?,
        ))
    }
    /// Thirty-two bytes: an identity's payload, or a structure hash.
    fn id(&mut self) -> Result<[u8; 32], IndexError> {
        self.take(32)?.try_into().map_err(|_| IndexError::Truncated)
    }
    fn text(&mut self) -> Result<String, IndexError> {
        let n = self.u32()? as usize;
        let b = self.take(n)?;
        String::from_utf8(b.to_vec()).map_err(|_| IndexError::Malformed("text is not UTF-8"))
    }
}
