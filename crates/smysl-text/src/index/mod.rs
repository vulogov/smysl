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

pub mod align;
#[cfg(feature = "substrate-redb")]
pub mod db;
pub mod journal;
pub mod postings;
pub mod ranges;
pub mod tables;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use smysl_core::ids::{Mid, Rdid, Tid};
use smysl_core::types::library::Axis;
use smysl_core::types::Record;

use crate::library::Library;
// The trait, for `MemPostings::analyzer` in the encoder. A postings row is read through the seam
// here as everywhere else: the table is the seam's own shape, written down.
use smysl_retrieve::Postings;

/// File magic. Eight bytes, so the header is aligned and greppable.
pub const MAGIC: &[u8; 8] = b"SMYSLSIX";

/// Sidecar format version.
///
/// Independent of the wire format, and of `smysl_graph::store::index::VERSION`: this index is
/// derived, so bumping it costs a rebuild and nothing else.
///
/// **2** since TX-P4 step 2. Step 1 wrote a single `objects_read` byte, which was the right
/// shape for two expensive tables and the wrong one for a third: `postings` reads no object and
/// is still too dear for an open, so "which tables does this index actually hold" stopped being
/// a boolean. It is a [`Built`] bitmask now, which is also what steps 3 and 4 will need.
pub const VERSION: u16 = 2;

/// The directory §4.1's layout reserves for the index, relative to the library root.
pub const DIR: &str = "index";

/// The sidecar, relative to the library root.
///
/// `index/substrate.idx` and not `index/substrate.redb`: the backing store is a different file
/// in the same reserved directory, so a library can carry both without one name meaning two
/// formats. See the module note in [`tables`] on why the backing is separate from the tables.
pub const SIDECAR: &str = "index/substrate.idx";

/// The backing store, relative to the library root (`substrate-redb`).
///
/// Named here rather than in [`db`] so that a build without the feature can still **say** that a
/// library's postings are in a file it cannot read. The alternative was a build that reported
/// "the index holds no postings yet" about a library whose postings were written down, which is
/// the sort of true-sounding falsehood a cache should not be able to produce.
pub const STORE: &str = "index/substrate.redb";

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
        Table::Align,
        Table::Said,
        Table::Composed,
        Table::Observed,
        Table::Postings,
        Table::Journal,
    ];

    /// The tables an **open** fills, when there is no current sidecar to load instead.
    ///
    /// The eight built tables fall into three cost classes, and only the cheapest can be
    /// afforded by every command that opens a library:
    ///
    /// | class | tables | measured at 155k units |
    /// |---|---|---|
    /// | arithmetic over the catalog's manifests | `manifests`, `parts`, `by-part`, `redactions` | 0.05 ms together |
    /// | a pass or a solve over every unit | `journal` 55, `said` 99, `composed` 41, `observed` 10 | 205 ms together |
    /// | analysing every unit's text | `postings` | 1,898 ms |
    /// | one object read and one structure tree per part | `segments`, `locators` | tens of thousands of reads on five Bibles |
    ///
    /// The first two classes are what an open builds; `text index --rebuild` and the writing
    /// commands build all eleven, and [`Index::built`] records which happened. The figures are
    /// from `tests/postings_timing.rs` (`--release`, Appendix E's gist length) and they are the
    /// reason the line falls where it does: the four tables in the second class are within a
    /// factor of ten of each other and an order of magnitude short of `postings`. Two of those
    /// numbers were orders of magnitude worse before this phase measured them — 4.53 s and 64 s
    /// — and both reasons are recorded where they were fixed, in `journal::rebuild` and in
    /// `crate::time::engine`'s `seed`.
    pub const ON_OPEN: &'static [Table] = &[
        Table::Manifests,
        Table::Parts,
        Table::ByPart,
        Table::Redactions,
        Table::Said,
        Table::Composed,
        Table::Observed,
        Table::Journal,
    ];

    /// This table's bit in a [`Built`] mask: its position in [`Table::ALL`].
    ///
    /// Position rather than discriminant, so that the mask a sidecar carries does not change
    /// meaning if a variant is ever inserted — `ALL` is the order §3.8 lists them in, and that
    /// order is part of the format.
    pub const fn bit(self) -> u16 {
        let mut i = 0;
        while i < Table::ALL.len() {
            if Table::ALL[i] as u8 == self as u8 {
                return 1u16 << i;
            }
            i += 1;
        }
        0
    }

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

/// Which tables an index actually holds (1.10, TX-P4 step 2).
///
/// The one field of [`Index`] that is about the index rather than about the corpus, and the
/// field that keeps a partial index from being a silently wrong one: a table that was not built
/// reports itself as not built, answers no question, and refuses to be persisted — because a
/// half-built sidecar would load as current on the next open, the catalog's hash agreeing.
///
/// A bitmask over [`Table::ALL`] positions rather than a `BTreeSet<Table>`: it is two bytes in
/// the sidecar, it is `Copy`, and the question asked of it — "is this one built" — is a bit
/// test. Step 1 had a boolean here and [`Table::ON_OPEN`] has the argument for why that stopped
/// being enough.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Built(u16);

impl Built {
    /// No table built. What an index of an empty library would have, and the default.
    pub const fn empty() -> Built {
        Built(0)
    }

    /// Every table of a given set.
    pub fn of(tables: &[Table]) -> Built {
        tables.iter().fold(Built::empty(), |b, t| b.with(*t))
    }

    /// The same mask with one more table built.
    pub const fn with(self, table: Table) -> Built {
        Built(self.0 | table.bit())
    }

    /// Whether a table is built.
    pub const fn has(self, table: Table) -> bool {
        self.0 & table.bit() != 0
    }

    /// Whether every table this build fills is built.
    pub fn is_complete(self) -> bool {
        Table::BUILT.iter().all(|t| self.has(*t))
    }

    /// The tables that are built, in §3.8 order.
    pub fn tables(self) -> impl Iterator<Item = Table> {
        Table::ALL
            .iter()
            .copied()
            .filter(move |t| self.has(*t))
            .collect::<Vec<_>>()
            .into_iter()
    }

    /// The same mask with one table taken out.
    ///
    /// What writing the sidecar does with [`Table::Postings`] when the backing store holds them:
    /// the table is not in the file, so the file may not claim it. See
    /// [`Index::to_bytes_without_postings`].
    pub const fn without(self, table: Table) -> Built {
        Built(self.0 & !table.bit())
    }

    /// The mask as it is written to the sidecar.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// A mask read back from a sidecar.
    ///
    /// Bits outside [`Table::ALL`] are dropped rather than refused: a sidecar written by a later
    /// build may know tables this one does not, and the answer to that is to rebuild the ones
    /// this build has — not to fail an open over a cache.
    pub fn from_bits(bits: u16) -> Built {
        let known = Table::ALL.iter().fold(0u16, |mask, t| mask | t.bit());
        Built(bits & known)
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
    /// Which tables this index holds.
    ///
    /// [`Table::ON_OPEN`] has the three cost classes and why an open builds five of the eight.
    /// A partial index reports the three it did not build as not built rather than as empty,
    /// refuses to be persisted, and answers no locator question — which sends `resolve` to the
    /// search, the answer it gave before this module existed.
    pub built: Built,
    pub manifests: BTreeMap<Mid, ManifestRow>,
    pub parts: BTreeMap<Tid, PartRow>,
    pub by_part: BTreeMap<Tid, Vec<Mid>>,
    pub redactions: BTreeMap<Tid, u32>,
    pub segments: BTreeMap<Rdid, SegmentsRow>,
    /// Locator tables, by manifest. A mid is present only when **every** part of it resolved,
    /// so a lookup that misses is a lookup to make the other way rather than a locator that
    /// names nothing.
    pub locators: BTreeMap<Mid, LocatorTable>,
    /// Which locator of one expression is which of another (TX-P4 step 4). See [`align`].
    pub align: align::Table,
    /// Effective `said` time per subject, as a range table (TX-P4 step 3).
    pub said: ranges::Ranges,
    /// Effective `composed` time per subject (TX-P4 step 3).
    pub composed: ranges::Ranges,
    /// Each unit's recorded `observed` instant, one millisecond wide (TX-P4 step 3).
    pub observed: ranges::Ranges,
    /// Postings per partition (TX-P4 step 2). See [`postings`] for the partition.
    pub postings: postings::Table,
    pub journal: Vec<journal::Row>,
}

/// Where a library's postings are, and what they hold — what `text index` prints.
///
/// One struct for both backings, because the question a reader is asking is about the corpus and
/// not about the file: how many terms, across how many partitions, split by which chains. The one
/// field that is about the file is `stored`, and it is here because a reader who is about to run
/// `text index --rebuild` should know which of the two it would rewrite.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Held {
    /// `true` when the rows are in the backing store rather than resident in this process.
    pub stored: bool,
    /// Terms, which is the quantity both backings count for [`Table::Postings`].
    pub terms: usize,
    pub partitions: usize,
    /// The analyzer ids, sorted and without repetition.
    pub analyzers: Vec<String>,
}

/// A scoring view over a library's postings, from whichever backing holds them.
///
/// The seam is [`smysl_retrieve::Postings`] and both backings implement it, so this enum exists
/// only to be *named* by a caller that does not care which: `find --library` scores one of these
/// and the ranking is the same either way, which is the claim
/// `crates/smysl-text/tests/substrate_db.rs` checks bit for bit.
///
/// Not a `Box<dyn Postings>`: the trait's `analyzer` returns a borrowed id, the in-memory view
/// borrows the index, and an enum keeps both lifetimes visible to the compiler instead of
/// hiding them behind an allocation.
#[non_exhaustive]
pub enum Source<'a> {
    /// The table this process holds.
    Memory(postings::View<'a>),
    /// The table beside the library, read a term at a time.
    ///
    /// Boxed: a stored view holds two open redb tables and is some 350 bytes, against the
    /// in-memory view's vector of references. A caller moves one of these per query, so the
    /// enum is the size of a pointer instead of the size of its largest arm.
    #[cfg(feature = "substrate-redb")]
    Stored(Box<db::View>),
}

impl Source<'_> {
    /// How many partitions this view spans.
    pub fn partitions(&self) -> usize {
        match self {
            Source::Memory(v) => v.partitions(),
            #[cfg(feature = "substrate-redb")]
            Source::Stored(v) => v.partitions(),
        }
    }

    /// Whether the rows came from the backing store.
    pub fn is_stored(&self) -> bool {
        match self {
            Source::Memory(_) => false,
            #[cfg(feature = "substrate-redb")]
            Source::Stored(_) => true,
        }
    }
}

/// Delegation, and nothing else. A method here that did arithmetic would be a third answer to a
/// question two backings already answer identically.
impl Postings for Source<'_> {
    fn df(&self, term: &str) -> u32 {
        match self {
            Source::Memory(v) => v.df(term),
            #[cfg(feature = "substrate-redb")]
            Source::Stored(v) => v.df(term),
        }
    }

    fn postings(&self, term: &str) -> Vec<(smysl_core::Uid, u32)> {
        match self {
            Source::Memory(v) => v.postings(term),
            #[cfg(feature = "substrate-redb")]
            Source::Stored(v) => v.postings(term),
        }
    }

    fn doc_len(&self, uid: &smysl_core::Uid) -> Option<u32> {
        match self {
            Source::Memory(v) => v.doc_len(uid),
            #[cfg(feature = "substrate-redb")]
            Source::Stored(v) => v.doc_len(uid),
        }
    }

    fn tf(&self, term: &str, uid: &smysl_core::Uid) -> u32 {
        match self {
            Source::Memory(v) => v.tf(term, uid),
            #[cfg(feature = "substrate-redb")]
            Source::Stored(v) => v.tf(term, uid),
        }
    }

    fn docs(&self) -> u32 {
        match self {
            Source::Memory(v) => v.docs(),
            #[cfg(feature = "substrate-redb")]
            Source::Stored(v) => v.docs(),
        }
    }

    fn total_len(&self) -> u64 {
        match self {
            Source::Memory(v) => v.total_len(),
            #[cfg(feature = "substrate-redb")]
            Source::Stored(v) => v.total_len(),
        }
    }

    fn analyzer(&self) -> Option<&str> {
        match self {
            Source::Memory(v) => v.analyzer(),
            #[cfg(feature = "substrate-redb")]
            Source::Stored(v) => v.analyzer(),
        }
    }
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
        ix.segments = tables::segments_rebuild(lib);
        ix.locators = tables::locators_rebuild(lib);
        ix.postings = postings::rebuild(lib, &ix.manifests, &ix.by_part);
        // After `locators`, which it reads: an alignment is between two locator tables, and a
        // manifest whose parts did not all resolve has none.
        ix.align = align::rebuild(lib, &ix.manifests, &ix.locators);
        ix.built = Built::of(Table::BUILT);
        ix
    }

    /// Build only the tables derived from the catalog, reading no object.
    ///
    /// What opening a library does. See [`Table::ON_OPEN`] for why the split exists: these five
    /// are arithmetic over a log that is already parsed, and the three left out cost either an
    /// analysis of every unit or I/O per part.
    pub fn rebuild_catalog(lib: &Library) -> Index {
        Index {
            catalog_len: lib.store().log_len(),
            catalog_hash: *lib.store().log_hash(),
            built: Built::of(Table::ON_OPEN),
            manifests: tables::manifests_rebuild(lib),
            parts: tables::parts_rebuild(lib),
            by_part: tables::by_part_rebuild(lib),
            redactions: tables::redactions_rebuild(lib),
            segments: BTreeMap::new(),
            locators: BTreeMap::new(),
            align: align::Table::new(),
            said: ranges::axis_rebuild(lib, Axis::Said),
            composed: ranges::axis_rebuild(lib, Axis::Composed),
            observed: ranges::observed_rebuild(lib),
            postings: postings::Table::new(),
            journal: journal::rebuild(lib),
        }
    }

    /// Whether every table this build fills was built.
    pub fn is_complete(&self) -> bool {
        self.built.is_complete()
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
        if self.built.has(Table::Said) {
            ranges::axis_on_append(&mut self.said, lib, Axis::Said, records);
        }
        if self.built.has(Table::Composed) {
            ranges::axis_on_append(&mut self.composed, lib, Axis::Composed, records);
        }
        if self.built.has(Table::Observed) {
            ranges::observed_on_append(&mut self.observed, records);
        }
        // The two object tables are brought forward only when they exist. Absorbing an append
        // into a table that was never built would make it hold the parts added since the open
        // and not the ones that were there first - which is the one state the completeness flag
        // is for, since a half-built locator table cannot be told from a complete one at
        // lookup time.
        if self.built.has(Table::Segments) {
            tables::segments_on_append(&mut self.segments, lib, records);
        }
        if self.built.has(Table::Locators) {
            tables::locators_on_append(&mut self.locators, lib, records);
        }
        // After `manifests` and `by_part`, which decide a unit's partition and which the lines
        // above have just brought forward. A postings builder reading the old `head` would put
        // the appended units in the partition the rebuild has stopped using.
        // `align` is rebuilt rather than extended, and the reason is not laziness: a row
        // depends on two manifests' numbering declarations, on the maps installed in the
        // library, and on both locator tables — so a manifest arriving can create pairs with
        // every manifest already there, and a map installed since the last build changes pairs
        // nothing in the batch mentions. The rebuild is `O(head pairs × locators)` over tables
        // already in memory, and the pairs are the texts of the library rather than its parts.
        if self.built.has(Table::Align)
            && records
                .iter()
                .any(|r| matches!(r, Record::Manifest(_) | Record::Redaction(_)))
        {
            self.align = align::rebuild(lib, &self.manifests, &self.locators);
        }
        if self.built.has(Table::Postings) {
            postings::on_append(
                &mut self.postings,
                lib,
                &self.manifests,
                &self.by_part,
                records,
            );
        }
    }

    /// Score the postings table, when this index holds one.
    ///
    /// `None` when `postings` was not built — which is an open that did not afford it — and the
    /// caller's answer to that is to analyse the corpus in memory as `find` always has, or to
    /// run `text index --rebuild` once and never pay it again. Not an empty view: an empty view
    /// would retrieve nothing and look exactly like a corpus with nothing in it.
    pub fn postings(&self) -> Option<postings::View<'_>> {
        self.built
            .has(Table::Postings)
            .then(|| postings::View::all(&self.postings))
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
        // A partial index says so about the tables it did not build, rather than reporting them
        // empty. `segments 0` on a library full of readings is a false sentence.
        if !self.built.has(table) {
            return None;
        }
        Some(match table {
            Table::Manifests => self.manifests.len(),
            Table::Parts => self.parts.len(),
            Table::ByPart => self.by_part.len(),
            Table::Redactions => self.redactions.len(),
            Table::Segments => self.segments.len(),
            Table::Locators => self.locators.values().map(BTreeMap::len).sum(),
            Table::Align => align::rows(&self.align),
            Table::Said => self.said.len(),
            Table::Composed => self.composed.len(),
            Table::Observed => self.observed.len(),
            // Terms rather than partitions or postings: a row of a postings table is a term's
            // list, and the count a reader can check against a corpus is how many distinct
            // terms it holds. `text index` prints the partition and analyzer counts beside it.
            Table::Postings => self.postings.values().map(|p| p.iter().count()).sum(),
            Table::Journal => self.journal.len(),
            // No catch-all arm, and the compiler is what says so: TX-P4 step 4 built the last of
            // §3.8's twelve, so `_ => return None` became unreachable. It was the arm that said
            // "this build has no builder for that table", and there is no such table any more.
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
        self.built.has(Table::Locators) && self.locators.contains_key(mid)
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
        self.dump(true)
    }

    /// The same dump with the `postings` table left out, and the bit for it cleared.
    ///
    /// What [`crate::library::Library::write_index`] writes when the backing store ([`db`]) is
    /// holding the postings: the sidecar is 1.1 KB a unit lighter, the store carries its own
    /// catalog stamp, and the two cannot disagree because the bit says which file to ask. A
    /// sidecar that kept the table **and** a store that held it would be the same rows in two
    /// places with one hash between them.
    ///
    /// The format does not change. A not-built table is written as a table of no rows, which is
    /// what every table a partial index does not hold has always been written as — so a build
    /// without the feature reads this file, finds the bit clear, and says what is true: the
    /// postings are not in the index it can read.
    pub fn to_bytes_without_postings(&self) -> Vec<u8> {
        self.dump(false)
    }

    fn dump(&self, postings: bool) -> Vec<u8> {
        let mut w = Writer::new();
        w.raw(MAGIC);
        w.u16(VERSION);
        w.u64(self.catalog_len);
        w.raw(&self.catalog_hash);
        w.u16(if postings {
            self.built.bits()
        } else {
            self.built.without(Table::Postings).bits()
        });

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

        w.u32(self.align.len() as u32);
        for ((a, b), row) in &self.align {
            w.raw(a.as_bytes());
            w.raw(b.as_bytes());
            w.text(&row.scheme);
            w.u32(row.uncovered);
            w.u32(row.rows.len() as u32);
            for (from, to) in &row.rows {
                w.text(from);
                w.text(to);
            }
        }

        // The three range tables, in §3.8's order. Rows only: the max-hi tree over them is a
        // function of the rows and is rebuilt on decode, so it is not a second thing that could
        // be written down wrong.
        for table in [&self.said, &self.composed, &self.observed] {
            w.u8(u8::from(table.danglers()));
            w.u32(table.rows().len() as u32);
            for row in table.rows() {
                match row.subject {
                    ranges::Subject::Unit(uid) => {
                        w.u8(0);
                        w.raw(uid.as_bytes());
                    }
                    ranges::Subject::Manifest(mid) => {
                        w.u8(1);
                        w.raw(mid.as_bytes());
                    }
                }
                bound(&mut w, row.lo);
                bound(&mut w, row.hi);
            }
        }

        w.u32(if postings {
            self.postings.len() as u32
        } else {
            0
        });
        for (partition, row) in self.postings.iter().filter(|_| postings) {
            match partition {
                postings::Partition::Text { mid, lang } => {
                    w.u8(1);
                    w.raw(mid.as_bytes());
                    w.text(lang);
                }
                postings::Partition::Loose => w.u8(0),
            }
            w.text(row.analyzer().unwrap_or_default());
            let terms: Vec<(&str, &[(smysl_core::Uid, u32)])> = row.iter().collect();
            w.u32(terms.len() as u32);
            for (term, list) in terms {
                w.text(term);
                w.u32(list.len() as u32);
                for (uid, tf) in list {
                    w.raw(uid.as_bytes());
                    w.u32(*tf);
                }
            }
            let lens: Vec<(&smysl_core::Uid, u32)> = row.lengths().collect();
            w.u32(lens.len() as u32);
            for (uid, len) in lens {
                w.raw(uid.as_bytes());
                w.u32(len);
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
        let built = Built::from_bits(r.u16()?);

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

        let mut align = align::Table::new();
        for _ in 0..r.u32()? {
            let a = Mid::from_bytes(r.id()?);
            let b = Mid::from_bytes(r.id()?);
            let scheme = r.text()?;
            let uncovered = r.u32()?;
            let mut rows = BTreeMap::new();
            for _ in 0..r.u32()? {
                let from = r.text()?;
                rows.insert(from, r.text()?);
            }
            align.insert(
                (a, b),
                align::Rows {
                    scheme,
                    rows,
                    uncovered,
                },
            );
        }

        let mut three = Vec::new();
        for _ in 0..3 {
            let danglers = r.u8()? != 0;
            let mut rows = Vec::new();
            for _ in 0..r.u32()? {
                let subject = match r.u8()? {
                    0 => ranges::Subject::Unit(smysl_core::Uid::from_bytes(r.id()?)),
                    1 => ranges::Subject::Manifest(Mid::from_bytes(r.id()?)),
                    _ => return Err(IndexError::Malformed("range subject kind")),
                };
                let lo = read_bound(&mut r)?;
                let hi = read_bound(&mut r)?;
                rows.push(ranges::Row { lo, hi, subject });
            }
            three.push(ranges::Ranges::of(rows, danglers));
        }
        let mut three = three.into_iter();
        let said = three.next().expect("three tables");
        let composed = three.next().expect("three tables");
        let observed = three.next().expect("three tables");

        let mut postings = postings::Table::new();
        for _ in 0..r.u32()? {
            let partition = match r.u8()? {
                0 => postings::Partition::Loose,
                1 => postings::Partition::Text {
                    mid: Mid::from_bytes(r.id()?),
                    lang: r.text()?,
                },
                _ => return Err(IndexError::Malformed("postings partition kind")),
            };
            let analyzer = r.text()?;
            let mut terms: BTreeMap<String, Vec<(smysl_core::Uid, u32)>> = BTreeMap::new();
            for _ in 0..r.u32()? {
                let term = r.text()?;
                let mut list = Vec::new();
                for _ in 0..r.u32()? {
                    list.push((smysl_core::Uid::from_bytes(r.id()?), r.u32()?));
                }
                terms.insert(term, list);
            }
            let mut lens: BTreeMap<smysl_core::Uid, u32> = BTreeMap::new();
            for _ in 0..r.u32()? {
                let uid = smysl_core::Uid::from_bytes(r.id()?);
                lens.insert(uid, r.u32()?);
            }
            postings.insert(
                partition,
                smysl_retrieve::MemPostings::from_parts(&analyzer, terms, lens),
            );
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
            built,
            manifests,
            parts,
            by_part,
            redactions,
            segments,
            locators,
            align,
            said,
            composed,
            observed,
            postings,
            journal,
        })
    }
}

/// A bound as a presence byte and an `i64`.
///
/// An open bound writes a zero and no instant rather than a sentinel: `Instant` is an `i64` and
/// a corpus may legitimately name a date near either extreme — EDTF's `Y`-prefixed years reach
/// the whole range — so a sentinel would make one real instant mean *unbounded*.
fn bound(w: &mut Writer, b: ranges::Bound) {
    match b {
        ranges::Bound::Open => w.u8(0),
        ranges::Bound::At(at) => {
            w.u8(1);
            w.u64(at.0 as u64);
        }
    }
}

fn read_bound(r: &mut Reader<'_>) -> Result<ranges::Bound, IndexError> {
    match r.u8()? {
        0 => Ok(ranges::Bound::Open),
        1 => Ok(ranges::Bound::At(ranges::Instant(r.u64()? as i64))),
        _ => Err(IndexError::Malformed("range bound kind")),
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
