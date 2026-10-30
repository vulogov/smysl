//! The library handle: its logs and an object store, opened together (SMYSL-2.4 §4.2).
//!
//! Everything else in this crate is a function of its arguments. This is the one module that
//! touches a directory, and it exists because the two halves of a library are useless apart: a
//! `Store` holds manifests and no text (OQ-39 — a log never holds a record 15 or 18), and an
//! `ObjectStore` holds text addressed by identities it cannot interpret. A locator resolves
//! only when both are open.
//!
//! # Layout
//!
//! ```text
//! <root>/LIBRARY                 the marker, holding the layout version
//! <root>/log/catalog.cbor        records 14 and 19, and what names no expression's unit
//! <root>/log/shards/<alias>.cbor an expression's units and what is said of them
//! <root>/objects/t3/ab/cd/…      part texts   (record 15 envelopes)
//! <root>/objects/r3/ab/cd/…      readings     (record 18 envelopes)
//! <root>/objects/tmp/            where an object is written before it is renamed into place
//! <root>/lock                    the library lock; `log/<shard>.lock` are the shard locks
//! ```
//!
//! **One log per expression, since TX-P13c step 1** (layout 2; [`crate::shards`] has the
//! layout and the routing). Until then it was one log: what TX-P1 needed was a catalog, and a
//! sharding scheme written then would have been one chosen before anything could measure it.
//! The library still opens every shard into one in-memory [`Store`], so every reader of
//! [`Library::store`] sees exactly what it saw; what the shards buy is a reader that wants some
//! of them — `--shard`, a union of shards on disk — and a log per expression to write to. A
//! layout-1 library opens as a library whose only shard is its catalog, and becomes layout 2
//! with the first write that creates another.
//!
//! # What `add` has to get right
//!
//! A reading's segment offsets are **part-local** — [`crate::reading::Segment`] says so, and
//! `PartEntry.length` is the part's length, so the structure hash is over a table that starts
//! at zero. A reader hands back one table over the whole text. The shift is therefore part of
//! cutting a text into parts, and it is done here, once, because every caller that cut parts by
//! hand would have to get it right separately.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use smysl_core::error::LibError;
use smysl_core::ids::{AgentId, LangTag, Mid, Rdid, Tid, Uid};
use smysl_core::types::library::{PartResolver, Redaction, Resolved};
use smysl_core::types::provenance::Hlc;
use smysl_core::types::{Carry, PartEntry, PartText, Record};
use smysl_graph::{Store, UidShards};

use crate::index::{self, Index};
use crate::limits::{Budget, Caps};
use crate::locator::Locator;
use crate::manifest::ManifestBuilder;
use crate::norm::Normalised;
use crate::objects::ObjectStore;
use crate::part::{self, Policy};
use crate::readers::{read_with, reader, Input, Params};
use crate::reading::Reading;
use crate::shards::ShardLog;
use crate::speaker;
use crate::structure::Structure;

/// The marker file that says a directory is a library.
///
/// A file rather than a naming convention, because `-s/--store` has meant "a path to a log"
/// since 1.0 and a directory passed to it has to be recognised rather than guessed at. Its
/// contents are the layout version: a library written by a later release can then be refused
/// by name instead of half-read.
pub const MARKER: &str = "LIBRARY";

/// The layout this release writes and reads.
pub const LAYOUT: &str = "smysl/library/2";

/// The layout before shards: one catalog log. Read as a library whose only shard is the catalog.
pub const LAYOUT_1: &str = "smysl/library/1";

/// The catalog log, relative to the root.
pub const CATALOG: &str = "log/catalog.cbor";

/// What `text add` was asked to do.
///
/// `reader` and `policy` are not optional and have no defaults here. The reader id is recorded
/// in the manifest and is what `SMY-E401` compares against on re-read; the part policy is
/// recorded for the same reason, and §4.5's own argument for making it a required manifest key
/// is that its default changes when GE-T14 measures it. A default in this struct would put the
/// decision back where the format took it away from.
#[derive(Debug, Clone)]
pub struct AddSpec {
    pub reader: String,
    pub params: Params,
    pub alias: String,
    /// The language the text is in. Absent means the reader's own answer, and a reader that
    /// does not know one is a refusal rather than `und`: `und` claims nobody knows, which is a
    /// different statement from nobody having said.
    pub lang: Option<LangTag>,
    pub licence: String,
    pub carry: Carry,
    pub policy: Option<Policy>,
    pub title: Option<String>,
    /// Replace every speaker in the reading with a pseudonym under this key.
    ///
    /// The key and not a `bool`, which is what SMYSL-2.4 §3.2's sketch had. A `bool` would mean
    /// this crate had to **find** the key, and finding it means creating it when it is absent,
    /// and creating it means thirty-two random bytes — a randomness source in the pure tier's
    /// default tree, which is the claim TX-P2 step 1 wrote into the purity gate when `lingua`
    /// brought `getrandom` with it. So the key is an argument: [`crate::secrets`] knows where
    /// it lives and what shape it has, the facade's `text` feature supplies the randomness, and
    /// this crate stays a function of what it was given.
    pub pseudonym_key: Option<speaker::Key>,
    /// The versification scheme this expression is numbered in (manifest key 14, TX-P4 step 4).
    ///
    /// A *name*, not a map: `kjv`, `synodal`, `lxx`. What it buys is the `align` table — two
    /// manifests can only be aligned if each says which numbering it uses, and a scheme that
    /// connects the two names is installed in the library. Absent means the expression makes no
    /// claim, and an expression that makes no claim is not aligned rather than aligned by guess.
    pub versification: Option<String>,
}

impl AddSpec {
    pub fn new(
        reader: impl Into<String>,
        alias: impl Into<String>,
        licence: impl Into<String>,
    ) -> AddSpec {
        AddSpec {
            reader: reader.into(),
            params: Params::default(),
            alias: alias.into(),
            lang: None,
            licence: licence.into(),
            carry: Carry::default(),
            policy: None,
            title: None,
            pseudonym_key: None,
            versification: None,
        }
    }

    /// Pseudonymise the speakers under this key.
    pub fn pseudonymised(mut self, key: speaker::Key) -> AddSpec {
        self.pseudonym_key = Some(key);
        self
    }

    pub fn with_lang(mut self, lang: LangTag) -> AddSpec {
        self.lang = Some(lang);
        self
    }

    pub fn with_carry(mut self, carry: Carry) -> AddSpec {
        self.carry = carry;
        self
    }

    pub fn with_policy(mut self, policy: Policy) -> AddSpec {
        self.policy = Some(policy);
        self
    }
}

/// What `add` did.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Added {
    pub mid: Mid,
    /// The parts, in document order.
    pub parts: Vec<Tid>,
    /// Objects actually written. Fewer than `2 * parts.len()` means some were already there,
    /// which is what re-adding an unchanged text looks like.
    pub objects_written: usize,
    /// Objects **not** written because rule Z forbids them: a part this library has redacted,
    /// and its reading, counted as two.
    ///
    /// Non-zero means the file held a part somebody has redacted. The manifest was still
    /// written — see `Library::add`'s rule Z comment for why — so this is the number that says
    /// the corpus will not resolve every locator the manifest names.
    pub objects_redacted: usize,
    /// The normalised length of the whole text.
    pub bytes: u64,
    /// The reader dropped something the source carried.
    pub lossy: bool,
    /// The manifest this one supersedes, for an [`Library::append`]; `None` for an `add`.
    ///
    /// Here rather than inferred from the manifest by the caller, because the caller is usually
    /// printing it, and the one place that knows whether this was a growth or a first add is the
    /// call that did it.
    pub supersedes: Option<Mid>,
}

/// What [`Library::redact`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Redacted {
    pub tid: Tid,
    /// Objects unlinked: the part, and a reading for each manifest entry that named it.
    ///
    /// Zero is an ordinary outcome and not a failure: the library may never have held the
    /// bytes. A redaction is still worth recording there — it is what stops the part arriving
    /// later from a peer that has not heard.
    pub objects_removed: usize,
    /// How many part entries, across every manifest, name this part.
    ///
    /// Printed rather than acted on. The manifests stay: an expression that was read from a
    /// text does not stop having been read from it, and a manifest with an unresolvable part is
    /// the honest record of a redacted corpus.
    pub manifests: usize,
    /// This part was already redacted before this call.
    pub already: bool,
}

/// One resolved passage: the part it came from, the byte range in it, and the text.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Passage {
    pub mid: Mid,
    pub tid: Tid,
    pub range: Range<u64>,
    pub text: String,
}

/// A library: the catalog, the objects, and the root they live under.
#[derive(Debug)]
pub struct Library {
    root: PathBuf,
    /// Every shard's records, read as one store. It has no path of its own: writes go to the
    /// shards' logs, which is where `shards` keeps them.
    store: Store,
    shards: BTreeMap<String, ShardLog>,
    /// Which shards hold each uid: what routing reads, and what each shard's uid list is
    /// written from.
    uid_shards: UidShards,
    /// The layout the marker says, which the first write that creates a shard raises to 2.
    layout: &'static str,
    objects: ObjectStore,
    /// The derived substrate index (§3.8), loaded from its sidecar or rebuilt on open.
    ///
    /// Held rather than handed out, because the only thing that may consult it is a query that
    /// would otherwise do the work it caches — see [`Library::resolve`], which is the one such
    /// query in this step. A field a caller could reach would be a field a caller could trust
    /// past its completeness rule.
    index: Index,
    /// The backing store for the `postings` table, when this library has one (§3.8).
    ///
    /// `Some` only when the file was already there: an open does not create one, for the reason
    /// [`index::db::Db::exists`] gives. `None` therefore means one of three things that all have
    /// the same answer — no store, a store this build cannot open, or a build without the
    /// feature — and the answer is that the postings are wherever the sidecar says they are.
    ///
    /// `AssertUnwindSafe`, and the assertion is the cache argument in one word. A `Library` is
    /// handed out as an `Arc<dyn PartResolver + Send + Sync + RefUnwindSafe>` — `text show` and
    /// the renderers resolve parts inside a `catch_unwind` — and redb's `Database` is not
    /// `RefUnwindSafe`, because a panic mid-write could leave its page state observable and
    /// torn. For a store of **derived** rows that is already a handled state: every read path
    /// compares the catalog stamp and rebuilds when it disagrees, a torn file fails to open and
    /// is a rebuild too, and the authoritative bytes are in the log and the object store. What
    /// unwind safety protects — a half-updated invariant that is then trusted — is the one thing
    /// a cache with a stamp cannot have.
    #[cfg(feature = "substrate-redb")]
    db: Option<std::panic::AssertUnwindSafe<index::db::Db>>,
    /// The scoring view of `db`, opened by the first query that asks and dropped by the next
    /// write, so queries between two writes share one (OQ-91). Unwind-safe as `db` is: a read
    /// snapshot, nothing in it half-updated.
    #[cfg(feature = "substrate-redb")]
    stored: std::sync::OnceLock<Option<std::panic::AssertUnwindSafe<index::db::View>>>,
}

fn io(at: impl AsRef<str>, e: &std::io::Error) -> LibError {
    LibError::Io {
        at: at.as_ref().to_string(),
        message: e.to_string(),
    }
}

impl Library {
    /// Whether this directory is a library.
    ///
    /// The question `-s/--store` asks before deciding whether its argument is a log or a root.
    /// A directory without the marker is not a library and is not made into one by being
    /// passed here: `create` is the only thing that writes a marker, so a mistyped path cannot
    /// quietly become an empty library.
    pub fn is_library(root: &Path) -> bool {
        root.join(MARKER).is_file()
    }

    /// Create a library, or open one that is already there.
    ///
    /// Idempotent, because the alternative is that `text add` into a fresh directory needs two
    /// commands and the first one has nothing to say.
    pub fn create(root: &Path) -> Result<Library, LibError> {
        std::fs::create_dir_all(root).map_err(|e| io(".", &e))?;
        let marker = root.join(MARKER);
        if !marker.is_file() {
            std::fs::write(&marker, format!("{LAYOUT}\n")).map_err(|e| io(MARKER, &e))?;
        }
        let log = root.join(CATALOG);
        if let Some(parent) = log.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io("log", &e))?;
        }
        Library::open(root)
    }

    /// Open a library.
    pub fn open(root: &Path) -> Result<Library, LibError> {
        let marker = root.join(MARKER);
        let version = std::fs::read_to_string(&marker).map_err(|e| io(MARKER, &e))?;
        let version = version.trim();
        let layout = match version {
            LAYOUT => LAYOUT,
            LAYOUT_1 => LAYOUT_1,
            _ => "",
        };
        if layout.is_empty() {
            // Named rather than guessed at. A library written by a later layout may differ in
            // where the logs are, and half-reading it would report an empty catalog for a
            // library that is full.
            return Err(LibError::Io {
                at: MARKER.to_string(),
                message: format!("layout `{version}` is not `{LAYOUT}`; this library was written by another release"),
            });
        }
        #[cfg(feature = "store-redb")]
        Self::settle_migrations(root)?;
        // The unit store held open while the shards are read, so that each primary shard's read
        // finds it open rather than opening a database of every shard's records afresh: 1,001
        // opens were 13 s of a 10⁶-unit library's open (TX-P13d).
        #[cfg(feature = "store-redb")]
        let _held = crate::shards::hold_store(root);
        // Every shard, in name order, into one store: the order a union of them reads in, so
        // the store is the union's answer whichever way it is built.
        let mut shards = BTreeMap::new();
        let mut lists = Vec::new();
        let mut all = Vec::new();
        for (name, _) in crate::shards::list(root)? {
            let (log, records) = ShardLog::read(root, &name)?;
            // The shard's uid list where it lists this log, rather than hashing every unit a
            // second time (`from_records` hashes them once); one sort for every shard after.
            let uids = crate::shards::read_uids(root, &name, log.len()).unwrap_or_else(|| {
                records
                    .iter()
                    .filter_map(|r| match r {
                        Record::Unit(u) => Some(smysl_core::canonical_uid(u)),
                        _ => None,
                    })
                    .collect()
            });
            lists.push((name.clone(), uids));
            all.extend(records);
            shards.insert(name, log);
        }
        let uid_shards = UidShards::from_lists(lists);
        let store = Store::from_records(all);
        let objects = ObjectStore::open(&root.join("objects"))?;
        let mut library = Library {
            root: root.to_path_buf(),
            store,
            shards,
            uid_shards,
            layout,
            objects,
            index: Index::default(),
            // A store that will not open is no store, and the library reports its postings the
            // way a library with none does: this is a cache, and a cache may not fail an open.
            #[cfg(feature = "substrate-redb")]
            db: index::db::Db::exists(root)
                .then(|| index::db::Db::open(root).ok())
                .flatten()
                .map(std::panic::AssertUnwindSafe),
            #[cfg(feature = "substrate-redb")]
            stored: std::sync::OnceLock::new(),
        };
        library.enforce_z()?;
        library.load_index();
        Ok(library)
    }

    /// Load the index sidecar, or build the part of the index an open can afford.
    ///
    /// Three outcomes and no fourth: the sidecar is current and is used whole; or it is stale,
    /// unreadable or absent, and the five tables derived from the catalog are built while the
    /// two derived from the **objects** are left for `text index --rebuild`. That split is
    /// [`Index::objects_read`]'s subject, and the reason is cost: the five are arithmetic over
    /// a log this method has already parsed, and the two are an object read and a structure
    /// tree per part, which `text ls` should not be paying for.
    ///
    /// A rebuild is never persisted from here — opening a library is a write in exactly one
    /// case (see [`Library::enforce_z`]) and a cache is not it, so a read-only command on a
    /// read-only corpus still works.
    ///
    /// Nothing here can fail. A sidecar that does not parse is a stale sidecar, which is a
    /// rebuild; `SMY-W110` is the graph store's precedent for saying so rather than refusing.
    fn load_index(&mut self) {
        let fresh = std::fs::read(Index::path(&self.root))
            .ok()
            .and_then(|b| Index::from_bytes(&b).ok())
            .filter(|ix| ix.matches(self.stamp().0, &self.stamp().1));
        self.index = match fresh {
            Some(ix) => ix,
            None => Index::rebuild_catalog(self),
        };
    }

    /// Build the two object-derived tables if an open left them out.
    ///
    /// What a command that is about to **write** calls, because it is already holding the locks
    /// and already paying for I/O, so the one-time cost of indexing a library that has never
    /// been indexed lands on the operation that grows it rather than on every operation that
    /// reads it. It is also what keeps a persisted sidecar whole: [`Library::write_index`]
    /// refuses to write a partial one, and without this a library first opened by this release
    /// would never get a complete sidecar at all.
    ///
    /// **The backing store is read first, and that is the point of the order.** A library whose
    /// postings live in `index/substrate.redb` loads a sidecar that does not hold them, so it is
    /// incomplete by exactly one table — and rebuilding that table means analysing every unit in
    /// the corpus (1,898 ms at 155k, [`index::Table::ON_OPEN`]) to arrive at rows that are already
    /// written down. Hydrating instead is a read of the file. When the store is stale or absent
    /// the rebuild happens as it always did.
    pub fn complete_index(&mut self) {
        if self.index.is_complete() {
            return;
        }
        self.hydrate_postings();
        if !self.index.is_complete() {
            self.reindex();
        }
    }

    /// Bring the stored `postings` table into memory, if that is where it is and it is current.
    ///
    /// `true` when the table is now resident. Nothing else moves: the other eleven tables are
    /// the sidecar's, and a store whose stamp does not match the catalog is a stale cache that
    /// [`Library::reindex`] is about to replace.
    #[cfg(feature = "substrate-redb")]
    fn hydrate_postings(&mut self) -> bool {
        if self.index.built.has(index::Table::Postings) {
            return true;
        }
        let Some(db) = self.db.as_ref() else {
            return false;
        };
        if !db.matches(self.stamp().0, &self.stamp().1) {
            return false;
        }
        match db.load() {
            Ok(table) => {
                self.index.postings = table;
                self.index.built = self.index.built.with(index::Table::Postings);
                true
            }
            // A malformed store is a rebuild, like a malformed sidecar. The next write replaces
            // it, which is the repair.
            Err(_) => false,
        }
    }

    #[cfg(not(feature = "substrate-redb"))]
    fn hydrate_postings(&mut self) -> bool {
        self.index.built.has(index::Table::Postings)
    }

    /// The derived index, as it stands.
    pub fn index(&self) -> &Index {
        &self.index
    }

    /// Rebuild the index from the catalog and the objects, and return it.
    ///
    /// §3.8's gate compares this against the index this library has been maintaining as it
    /// grew — `crates/smysl-text/tests/index.rs` is where that comparison lives, and it
    /// compares [`Index::to_bytes`] rather than the values.
    pub fn reindex(&mut self) -> &Index {
        #[cfg(feature = "substrate-redb")]
        {
            self.stored = std::sync::OnceLock::new();
        }
        self.index = Index::rebuild(self);
        &self.index
    }

    /// Write the index sidecar, creating `index/` if it is not there.
    ///
    /// The only thing that persists an index. `text index --rebuild` is the command, and the
    /// writing commands call it after they have appended, so a library that is being added to
    /// never needs the rebuild path twice.
    ///
    /// A partial index is not written: see [`Library::complete_index`]. Silently, and
    /// returning `Ok`, because every caller is reporting a *successful* operation and "the
    /// cache was not written because it was not complete" is not news to anybody — the next
    /// open rebuilds exactly as much as this one did, which is the state the library was
    /// already in.
    /// **Two files, when this build has the backing store.** The `postings` table goes into
    /// `index/substrate.redb` and the sidecar is written without it — the whole of what the
    /// feature buys, since that table is 1.1 KB a unit and the other eleven together are not.
    /// The store is written **first**: if it cannot be, the sidecar is written whole, so a
    /// library never ends up with the postings in neither file.
    pub fn write_index(&self) -> Result<(), LibError> {
        if !self.index.is_complete() {
            return Ok(());
        }
        let target = Index::path(&self.root);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io(index::DIR, &e))?;
        }
        let bytes = if self.store_postings() {
            self.index.to_bytes_without_postings()
        } else {
            self.index.to_bytes()
        };
        std::fs::write(&target, bytes).map_err(|e| io(index::SIDECAR, &e))?;
        Ok(())
    }

    /// Replace the backing store's `postings` table with the one in memory, and stamp it.
    ///
    /// `false` when there is no store to write, or when writing it failed — and a failure is not
    /// returned, because the caller's answer to it is already written down: the sidecar carries
    /// the table instead. An error here would fail a successful `text add` over a cache.
    #[cfg(feature = "substrate-redb")]
    fn store_postings(&self) -> bool {
        let owned;
        let db = match self.db.as_ref() {
            Some(db) => db,
            // The file is created by the first command that writes an index, never by an open.
            None => match index::db::Db::open(&self.root) {
                Ok(db) => {
                    owned = db;
                    &owned
                }
                Err(_) => return false,
            },
        };
        let (len, hash) = self.stamp();
        db.write(&self.index.postings, len, &hash).is_ok()
    }

    #[cfg(not(feature = "substrate-redb"))]
    fn store_postings(&self) -> bool {
        false
    }

    /// A scoring view over this library's postings, from whichever file holds them.
    ///
    /// `None` means the library has none to score — an open that did not afford them and no
    /// store to read, which is [`index::Table::ON_OPEN`]'s subject — and the caller's answer is to
    /// analyse the corpus as `find` always has, or to run `text index --rebuild` once. Not an
    /// empty view: an empty view retrieves nothing and looks exactly like an empty corpus.
    ///
    /// The resident table wins when both could answer. They cannot disagree — a write puts the
    /// same rows in both places and stamps them with the same catalog — and reading the one
    /// already in memory costs nothing.
    pub fn postings(&self) -> Option<index::Source<'_>> {
        if let Some(view) = self.index.postings() {
            return Some(index::Source::Memory(view));
        }
        self.stored_postings()
    }

    /// The postings of some units, built now, by the rule the kept table is built by: what a
    /// query reads from a library that keeps none (OQ-91).
    pub fn postings_over(&self, uids: &std::collections::BTreeSet<Uid>) -> index::postings::Built {
        index::postings::Built(index::postings::rebuild_over(
            self,
            &self.index.manifests,
            &self.index.by_part,
            uids,
        ))
    }

    /// The store's own answer, when there is a store and its stamp is the catalog's.
    #[cfg(feature = "substrate-redb")]
    fn stored_postings(&self) -> Option<index::Source<'_>> {
        self.stored
            .get_or_init(|| {
                let db = self.db.as_ref()?;
                if !db.matches(self.stamp().0, &self.stamp().1) {
                    return None;
                }
                db.all().ok().map(std::panic::AssertUnwindSafe)
            })
            .as_ref()
            .map(|view| index::Source::Stored(&view.0))
    }

    #[cfg(not(feature = "substrate-redb"))]
    fn stored_postings(&self) -> Option<index::Source<'_>> {
        None
    }

    /// Where this library's postings are and what they hold, or `None` when it holds none.
    ///
    /// What `text index` prints for the table. Counting terms in the store means walking its
    /// `postings` table, which is a scan of the rows and not of the corpus — the one place a
    /// report pays for the table being on disk.
    pub fn postings_held(&self) -> Option<index::Held> {
        if self.index.built.has(index::Table::Postings) {
            return Some(index::Held {
                stored: false,
                terms: self.index.postings.values().map(|p| p.iter().count()).sum(),
                partitions: self.index.postings.len(),
                analyzers: index::postings::analyzers(&self.index.postings)
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            });
        }
        self.stored_held()
    }

    /// The same question of the store.
    #[cfg(feature = "substrate-redb")]
    fn stored_held(&self) -> Option<index::Held> {
        let db = self.db.as_ref()?;
        if !db.matches(self.stamp().0, &self.stamp().1) {
            return None;
        }
        let partitions = db.partitions().ok()?;
        let mut analyzers: Vec<String> = partitions
            .iter()
            .filter(|(_, a, _)| !a.is_empty())
            .map(|(_, a, _)| a.clone())
            .collect();
        analyzers.sort_unstable();
        analyzers.dedup();
        Some(index::Held {
            stored: true,
            terms: db.terms().ok()?,
            partitions: partitions.len(),
            analyzers,
        })
    }

    #[cfg(not(feature = "substrate-redb"))]
    fn stored_held(&self) -> Option<index::Held> {
        None
    }

    /// Whether a backing store sits beside this library, whatever this build can do with it.
    ///
    /// Asked by a build **without** the feature, which is the only reason it does not consult
    /// the opened store: a library whose postings are in a file this binary cannot read must be
    /// told apart from one whose postings were never built, because the two have different
    /// remedies — a build with the feature, and a `--rebuild`.
    pub fn has_postings_store(&self) -> bool {
        self.root.join(index::STORE).is_file()
    }

    /// Bring the index forward over records this library has just appended.
    ///
    /// Called after the append, never before: a table built by reading the library has to see
    /// the library the rebuild would see, or the two builders are comparing different corpora.
    fn index_appended(&mut self, records: &[Record]) {
        // The borrow has to be split: `on_append` reads the library while it writes the index,
        // and the index is a field of the library. Taking it out and putting it back is the
        // honest version of that, and it costs one move of a struct of maps.
        let mut index = std::mem::take(&mut self.index);
        index.on_append(self, records);
        self.index = index;
    }

    /// Rule Z, on the way in: no object for a part this library's catalog has redacted.
    ///
    /// **Opening a library is a write in exactly one case**, and this is it. `redact` appends
    /// the record before it unlinks the objects, so a process killed between the two leaves a
    /// catalog that says a part is redacted and an object store that still holds it — and the
    /// next open is where that is put right. Doing it here rather than offering a `repair`
    /// command is the difference between a rule and a suggestion: there is no way to open this
    /// library and see the bytes.
    ///
    /// It is also why no diagnostic code was allocated for "a store holds a redacted part".
    /// SMYSL-2.4 §4.3.3 lists `C-Library` as forbidding "a redaction violation" and names no
    /// code for it; the answer this step gives is that the violation is **unrepresentable**
    /// rather than reportable — a `check` that opened the library would find the objects
    /// already gone, so a code for it would be one nothing can raise, which is worse than none.
    ///
    /// A failure to unlink is returned rather than swallowed. A library that says a part is
    /// redacted and cannot stop holding it is the one state an operator has to hear about, and
    /// the error names the path relative to the root as every `LibError::Io` here does.
    fn enforce_z(&mut self) -> Result<(), LibError> {
        if self.store.redacted_count() == 0 {
            return Ok(());
        }
        // The rdids to unlink come from the manifests, which is the only place a reading's
        // identity is written down: an object store is addressed by identity and cannot be
        // asked which readings are *of* a part.
        let redacted: Vec<Tid> = self.store.redactions().map(|(tid, _)| *tid).collect();
        let mut readings: Vec<Rdid> = Vec::new();
        for (_, manifest) in self.store.manifests() {
            for entry in &manifest.parts {
                if redacted.contains(&entry.tid) {
                    readings.push(entry.rdid);
                }
            }
        }
        for tid in &redacted {
            self.objects.remove_part(tid)?;
        }
        for rdid in &readings {
            self.objects.remove_reading(rdid)?;
        }
        Ok(())
    }

    /// Redact a part: record 19, then the bytes (rule Z).
    ///
    /// The record goes into the catalog **before** the objects are unlinked, and the order is
    /// the whole of the crash story: a process killed between the two leaves a catalog that has
    /// said what it is doing and an object store that has not caught up, which [`Library::open`]
    /// repairs. The other order leaves bytes gone with nothing to say why, which is
    /// indistinguishable from loss.
    ///
    /// What stays: the manifests that name the part, the units drawn from it, their spans and
    /// their uids. A redaction is not a retraction — the claims made from a text do not stop
    /// having been made because the text may no longer be held — and §5.2's redaction test is
    /// written around exactly that.
    pub fn redact(
        &mut self,
        tid: &Tid,
        agent: &AgentId,
        ts: Hlc,
        reason: Option<Uid>,
    ) -> Result<Redacted, LibError> {
        let mut readings: Vec<Rdid> = Vec::new();
        let mut manifests = 0usize;
        for (_, manifest) in self.store.manifests() {
            for entry in &manifest.parts {
                if &entry.tid == tid {
                    manifests += 1;
                    readings.push(entry.rdid);
                }
            }
        }
        readings.sort();
        readings.dedup();

        let mut record = Redaction::new(*tid, agent.clone(), ts);
        record.reason = reason;
        let already = self.store.is_redacted(tid);
        self.append_records(&[Record::Redaction(record)])?;

        let part = self.objects.remove_part(tid)?;
        let mut removed = usize::from(part);
        for rdid in &readings {
            if self.objects.remove_reading(rdid)? {
                removed += 1;
            }
        }
        Ok(Redacted {
            tid: *tid,
            objects_removed: removed,
            manifests,
            already,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn objects(&self) -> &ObjectStore {
        &self.objects
    }

    /// The shard names a library-wide write locks, for [`crate::lock::acquire_all`]: every
    /// shard the library holds, catalog included, in name order.
    ///
    /// A write that creates a shard creates one nobody else can be writing — every library-wide
    /// writer holds the library lock first — so the set taken at the start is enough.
    pub fn shards(&self) -> Vec<String> {
        self.shards.keys().cloned().collect()
    }

    /// The shards whose records the unit store holds rather than a log (TX-P13d), in name order.
    pub fn primary_shards(&self) -> Vec<String> {
        self.shards
            .iter()
            .filter(|(_, l)| l.is_primary())
            .map(|(n, _)| n.clone())
            .collect()
    }

    /// Make a shard primary (SMYSL-2.8 §3.5.5, §3.9; TX-P13d): its records move into the
    /// library's unit store, which is where they are written from then on, and its log is set
    /// aside as `<stem>.cbor.pre-primary`, kept and never read again.
    ///
    /// Only after a round trip: the record-set digest of the shard's log, of its records read
    /// back out of the store into an empty store, and of what the store's own tables hold, all
    /// three equal. Otherwise the store is put back as it was, the log is left where it is, and
    /// the answer is `SMY-E505`. Returns the digest. A shard already primary is left so.
    #[cfg(feature = "store-redb")]
    pub fn migrate_primary(&mut self, shard: &str) -> Result<[u8; 32], LibError> {
        Ok(self.migrate_primary_all(&[shard.to_string()])?[0])
    }

    /// [`Library::migrate_primary`] for several shards: the store brought up to the logs once
    /// and the digests taken in one pass, rather than a whole store's open per shard. Each shard
    /// is migrated or not on its own; the first that fails stops the rest, the ones before it
    /// staying primary.
    #[cfg(feature = "store-redb")]
    pub fn migrate_primary_all(&mut self, names: &[String]) -> Result<Vec<[u8; 32]>, LibError> {
        use crate::shards::{self, UNIT_STORE};
        use smysl_graph::redb::{RedbShards, RedbStore};
        let at = |e: smysl_core::Error| LibError::Io {
            at: UNIT_STORE.to_string(),
            message: e.to_string(),
        };
        for shard in names {
            if !self.shards.contains_key(shard) {
                return Err(LibError::Io {
                    at: crate::shards::DIR.to_string(),
                    message: format!("this library has no shard `{shard}`"),
                });
            }
        }
        let root = self.root.clone();
        let store = root.join(UNIT_STORE);
        let opts = smysl_graph::redb::RedbOptions::default();
        // The store brought up to every log, so it describes each of these whole.
        let all = shards::list(&root)?;
        drop(RedbStore::open_shards(&store, &all, None, opts.clone()).map_err(at)?);
        let writer = RedbShards::open(&store, &opts).map_err(at)?;
        let mut moved = Vec::new();
        let mut before = Vec::new();
        for shard in names {
            if self.shards[shard].is_primary() {
                before.push(None);
                continue;
            }
            let (_, records) = ShardLog::read(&root, shard)?;
            before.push(Some(Store::from_records(records).record_set_digest()));
            writer
                .make_primary(shard, &root.join(shards::log_path(shard)))
                .map_err(at)?;
            moved.push(shard.clone());
        }
        let tables = writer.digests().map_err(at)?;
        let mut out = Vec::with_capacity(names.len());
        for (shard, before) in names.iter().zip(before) {
            let held = tables.get(shard).copied().unwrap_or_default();
            let Some(before) = before else {
                out.push(held);
                continue;
            };
            let back = smysl_core::from_cbor_seq(&writer.bytes(shard).map_err(at)?)
                .map(|(r, _)| Store::from_records(r).record_set_digest())
                .map_err(|e| at(e.into()))?;
            if back != before || held != before {
                // This one back as it was, and every one after it not begun.
                for later in moved.iter().skip_while(|m| *m != shard) {
                    writer
                        .abandon_primary(later, &root.join(shards::log_path(later)))
                        .map_err(at)?;
                }
                return Err(LibError::PrimaryDisagrees {
                    shard: shard.clone(),
                    expected: hex(&before),
                    export: hex(if back != before { &back } else { &held }),
                });
            }
            // The marker first: a crash before the log is set aside leaves a marked shard beside
            // a log, which the next open sets aside; the other order would leave a primary
            // shard with no marker, which the next open puts back in index mode.
            let marker = root.join(shards::primary_marker(shard));
            let mut f = std::fs::File::create(&marker).map_err(|e| io(marker_rel(shard), &e))?;
            std::io::Write::write_all(&mut f, b"smysl/primary/1\n")
                .and_then(|()| f.sync_all())
                .map_err(|e| io(marker_rel(shard), &e))?;
            std::fs::rename(
                root.join(shards::log_path(shard)),
                root.join(shards::set_aside(shard)),
            )
            .map_err(|e| io(marker_rel(shard), &e))?;
            let (now, _) = ShardLog::read(&root, shard)?;
            self.shards.insert(shard.clone(), now);
            out.push(before);
        }
        Ok(out)
    }

    /// Make a primary shard an index-mode one again: its records written out as its log, from
    /// the store, byte for byte as the store holds them, and the marker removed. The log set
    /// aside when it became primary is left where it is.
    #[cfg(feature = "store-redb")]
    pub fn migrate_index(&mut self, shard: &str) -> Result<(), LibError> {
        use crate::shards::{self, UNIT_STORE};
        let at = |e: smysl_core::Error| LibError::Io {
            at: UNIT_STORE.to_string(),
            message: e.to_string(),
        };
        if !self.shards.get(shard).is_some_and(ShardLog::is_primary) {
            return Ok(());
        }
        let root = self.root.clone();
        let writer =
            smysl_graph::redb::RedbShards::open(&root.join(UNIT_STORE), &Default::default())
                .map_err(at)?;
        writer
            .make_index(shard, &root.join(shards::log_path(shard)))
            .map_err(at)?;
        drop(writer);
        std::fs::remove_file(root.join(shards::primary_marker(shard)))
            .map_err(|e| io(marker_rel(shard), &e))?;
        let (now, _) = ShardLog::read(&root, shard)?;
        self.shards.insert(shard.to_string(), now);
        Ok(())
    }

    /// `SMY-E505`, or the digest: a primary shard's records read out of the store into an empty
    /// store, against what the store's own tables hold.
    #[cfg(feature = "store-redb")]
    pub fn primary_digest(&self, shard: &str) -> Result<[u8; 32], LibError> {
        use crate::shards::UNIT_STORE;
        let at = |e: smysl_core::Error| LibError::Io {
            at: UNIT_STORE.to_string(),
            message: e.to_string(),
        };
        let writer =
            smysl_graph::redb::RedbShards::open(&self.root.join(UNIT_STORE), &Default::default())
                .map_err(at)?;
        let export = smysl_core::from_cbor_seq(&writer.bytes(shard).map_err(at)?)
            .map(|(r, _)| Store::from_records(r).record_set_digest())
            .map_err(|e| at(e.into()))?;
        let tables = writer
            .digests()
            .map_err(at)?
            .get(shard)
            .copied()
            .unwrap_or_default();
        if export != tables {
            return Err(LibError::PrimaryDisagrees {
                shard: shard.to_string(),
                expected: hex(&tables),
                export: hex(&export),
            });
        }
        Ok(tables)
    }

    /// Finish or undo a migration a crash interrupted (TX-P13d), before the shards are read.
    ///
    /// A migration to primary writes the store, then the marker, then sets the log aside; one
    /// back writes the log, then the store, then removes the marker. So: a shard the store holds
    /// with no marker and its log still there was not finished, and is put back in index mode,
    /// losing nothing, since the log holds every record; a marked shard whose log is still there
    /// was, but for setting the log aside; and a marked shard the store no longer holds was
    /// going back, and its marker goes.
    #[cfg(feature = "store-redb")]
    fn settle_migrations(root: &Path) -> Result<(), LibError> {
        use crate::shards::{self, UNIT_STORE};
        let store = root.join(UNIT_STORE);
        if !store.is_file() {
            return Ok(());
        }
        let at = |e: smysl_core::Error| LibError::Io {
            at: UNIT_STORE.to_string(),
            message: e.to_string(),
        };
        // Another process holding the store has opened the library before this one, and settled
        // it: nothing to do now that it could not.
        let writer = match smysl_graph::redb::RedbShards::open(&store, &Default::default()) {
            Ok(w) => w,
            Err(smysl_core::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                return Ok(())
            }
            Err(e) => return Err(at(e)),
        };
        let logs = writer.logs().map_err(at)?;
        for l in &logs {
            let marked = shards::is_primary(root, &l.name);
            let log = root.join(shards::log_path(&l.name));
            match (l.primary, marked, log.is_file()) {
                (true, false, true) => writer.abandon_primary(&l.name, &log).map_err(at)?,
                (true, true, true) => std::fs::rename(&log, root.join(shards::set_aside(&l.name)))
                    .map_err(|e| io(marker_rel(&l.name), &e))?,
                (false, true, true) => {
                    std::fs::remove_file(root.join(shards::primary_marker(&l.name)))
                        .map_err(|e| io(marker_rel(&l.name), &e))?
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Which shards hold each uid.
    pub fn uid_shards(&self) -> &UidShards {
        &self.uid_shards
    }

    /// What the substrate index is stamped with: a function of every shard's log (see
    /// [`crate::shards`]), equal to the catalog's length and hash while it is the only shard.
    pub fn stamp(&self) -> (u64, [u8; 32]) {
        crate::shards::stamp(&self.shards)
    }

    /// The layout this library is in, as its marker says.
    pub fn layout(&self) -> &'static str {
        self.layout
    }

    /// Read a file into the library: objects, a reading, and a manifest.
    ///
    /// The caller holds the locks (§3.9.3: the library lock, then the shards in name order).
    /// This does not take them itself, because `text add` and `ingest` take the same locks for
    /// longer than one call and a method that acquired its own would either deadlock with the
    /// caller or quietly run unlocked.
    pub fn add(
        &mut self,
        input: &Input<'_>,
        spec: &AddSpec,
        caps: &Caps,
    ) -> Result<Added, LibError> {
        self.add_with(input, spec, None, caps)
    }

    /// Read a file into the library as a **new version** of an expression.
    ///
    /// SMYSL-2.4 §3.1's growth operation: `text append <file> --alias A`. The input is the
    /// whole updated document — a chat export is always the whole conversation — and what makes
    /// this cheaper than a second `add` is that parts whose text did not change hash to the tids
    /// they already have, so their objects are already in the store and nothing is written for
    /// them. The new manifest carries `supersedes`, so the alias keeps one head.
    ///
    /// **Everything but the key comes from the head manifest**, and that is the point rather
    /// than a convenience. The reader and its parameters, the part policy, the language, the
    /// licence and the carry mode are what the expression *was built with*; re-stating them on
    /// the command line is an invitation to state them differently, and an expression whose
    /// second version was cut by another policy is one whose parts cannot be compared with its
    /// first. The key is the exception because a key is the one thing a corpus cannot hand back.
    ///
    /// **Tid reuse is a property of the part policy, not of this method.** With the default
    /// `target_min` of 64 KiB, a chat export of a few kilobytes is one part, and one part that
    /// grew is a part with a new tid — so an append of such an expression rewrites its only
    /// object and reuses nothing. That is correct and it is not nothing: `objects_written`
    /// reports it, and an operator who wants per-day parts says so in `--part-policy` at `add`.
    ///
    /// `SMY-E450` when the expression's speakers are pseudonyms and no key was given: the new
    /// messages would otherwise get a different speaker from the same person's old ones, which
    /// is the one failure a pseudonym exists to prevent and which nothing downstream could
    /// detect afterwards.
    /// `who` is the alias, or the mid of one head of a forked alias.
    ///
    /// The input comes first, as in [`Library::add`]: both operations are "read these bytes into
    /// the library", and a pair of methods that disagreed about the order of their arguments
    /// would be a pair somebody passes the wrong way round.
    pub fn append(
        &mut self,
        input: &Input<'_>,
        who: &str,
        key: Option<&speaker::Key>,
        caps: &Caps,
    ) -> Result<Added, LibError> {
        // `head` refuses an alias with two heads by name, which is the right refusal here too:
        // appending to a fork would have to pick one of them, and picking the lower-sorting mid
        // would make the answer depend on a hash. A caller that means a particular head names
        // it by mid, which `head` also accepts.
        let mid = self.head(who)?;
        let head = self
            .store
            .manifest(&mid)
            .cloned()
            .ok_or_else(|| LibError::Io {
                at: CATALOG.to_string(),
                message: format!("no manifest `{}`", mid.canonical()),
            })?;

        let (reader_id, params) =
            crate::readers::parse_reader_field(&head.reader).ok_or_else(|| LibError::BadParam {
                reader: head.reader.clone(),
                key: "reader".to_string(),
                reason: "a reader field this build could have written; this manifest was                          written by another release"
                    .to_string(),
            })?;
        let policy = Policy::parse(&head.part_policy).ok_or_else(|| LibError::BadParam {
            reader: reader_id.clone(),
            key: "part-policy".to_string(),
            reason: format!(
                "a policy this build can read, not `{}`; this manifest was written by                  another release",
                head.part_policy
            ),
        })?;

        // Whether this expression's speakers are pseudonyms is read out of the **corpus**, not
        // out of a flag: a reading whose speakers are `spk:…` says so in its own rows, so there
        // is no manifest key for it to disagree with. Reading the head's readings also verifies
        // them against their part entries (`SMY-E401`), which is a good thing to find out
        // before appending to a corpus rather than after.
        let mut pseudonymous = false;
        let mut plain_speaker = None;
        for entry in &head.parts {
            let reading = self.part_reading(&mid, &entry.tid)?;
            for row in &reading.rows {
                match &row.speaker {
                    Some(who) if speaker::is_pseudonym(who) => pseudonymous = true,
                    Some(who) => plain_speaker = Some(who.clone()),
                    None => {}
                }
            }
        }
        match (pseudonymous, key) {
            (true, None) => {
                return Err(LibError::PseudonymKeyMissing {
                    alias: head.alias.clone(),
                })
            }
            // A key offered to an expression whose speakers are plain names. Refused rather
            // than applied: half a corpus pseudonymised and half not would give one person two
            // speakers, which is the same defect `SMY-E450` prevents from the other side.
            (false, Some(_)) if plain_speaker.is_some() => {
                return Err(LibError::BadParam {
                    reader: reader_id.clone(),
                    key: "pseudonymise".to_string(),
                    reason: format!(
                        "`{}` names its speakers plainly (`{}`); pseudonymising only what is                          appended would give one person two speakers",
                        head.alias,
                        plain_speaker.unwrap_or_default()
                    ),
                })
            }
            _ => {}
        }

        let mut spec = AddSpec::new(reader_id, head.alias.clone(), head.licence.clone());
        spec.params = params;
        spec.lang = Some(head.lang.clone());
        spec.carry = head.carry;
        spec.policy = Some(policy);
        spec.pseudonym_key = key.cloned();
        // Carried from the head, like the licence and the policy: an append is a new version of
        // the *same* expression, and an expression does not change its numbering between
        // versions. A successor that dropped it would unseat an aligned head with an unaligned
        // one and the `align` table would quietly empty.
        spec.versification = head.versification.clone();
        self.add_with(input, &spec, Some(mid), caps)
    }

    fn add_with(
        &mut self,
        input: &Input<'_>,
        spec: &AddSpec,
        supersedes: Option<Mid>,
        caps: &Caps,
    ) -> Result<Added, LibError> {
        // The licence gate first, before a byte is read. `carry: text` under a licence that
        // does not permit redistribution is `SMY-E402`, and finding that out after writing the
        // objects would mean a library holding text it may not pass on.
        crate::manifest::carry_allowed(&spec.licence, spec.carry)?;

        let reader = reader(&spec.reader).map_err(|e| LibError::BadParam {
            reader: spec.reader.clone(),
            key: "reader".to_string(),
            reason: e.to_string(),
        })?;
        let mut budget = Budget::new(*caps, input.len() as u64)?;
        let mut out = read_with(reader, input, &spec.params, &mut budget)?;

        // Before the structure, the parts or any identity: pseudonymisation rewrites the
        // reading's speakers, and a reading's rdid is taken over the table that holds them. It
        // changes **no byte of the text**, so every tid below is the same as it would have been
        // without it — see [`crate::speaker`] for why that is the point rather than a detail.
        if let Some(key) = &spec.pseudonym_key {
            speaker::pseudonymise(&mut out.rows, key);
        }
        let out = out;

        let structure =
            Structure::build(&out.rows, out.text.len() as u64, &mut budget).map_err(|e| {
                LibError::Unreadable {
                    reader: spec.reader.clone(),
                    at: 0,
                    what: format!("a readable structure ({e})"),
                }
            })?;

        let policy = spec.policy.clone().unwrap_or_else(|| Policy {
            boundary_level: out.top_level.clone(),
            ..Policy::default()
        });
        let boundaries: Vec<Range<u64>> = structure
            .at_level(&policy.boundary_level)
            .map(|n| n.range.clone())
            .collect();
        if boundaries.is_empty() {
            return Err(LibError::NoParts {
                level: policy.boundary_level.to_string(),
                top: out.top_level.to_string(),
                reader: spec.reader.clone(),
            });
        }
        let plans = part::group(&boundaries, &policy, out.text.len() as u64, &mut budget)?;

        let lang = match spec.lang.clone().or_else(|| out.lang.clone()) {
            Some(l) => l,
            None => {
                return Err(LibError::BadParam {
                    reader: spec.reader.clone(),
                    key: "lang".to_string(),
                    reason: "neither the source nor the caller named a language, and `und` is \
                             a claim that nobody knows rather than that nobody said"
                        .to_string(),
                })
            }
        };

        // The manifest records the reader id **with its parameters** (key 3), because a
        // parameter changes a reader's output and a corpus that recorded only the id would mean
        // something else on re-read. Every TX-P1 reader takes none, so this is `spec.reader`
        // today and stops being it at the TX-P2 commit that adds one.
        let reader_field = crate::readers::reader_field(&spec.reader, &spec.params);
        let mut builder =
            ManifestBuilder::new(&spec.alias, lang, &reader_field, &spec.licence, &policy)?
                .carry(spec.carry);
        if let Some(previous) = supersedes {
            builder = builder.supersedes(previous);
        }
        if let Some(title) = spec.title.clone().or_else(|| out.title.clone()) {
            builder = builder.title(title);
        }
        // Only from the caller: no reader infers one. A USFM file names its books in the
        // vocabulary `osis/1` normalises into, and the *numbering* is not something the file
        // states — two Bibles with identical markup can be numbered differently, which is the
        // whole reason the `align` table exists.
        if let Some(scheme) = spec.versification.clone() {
            builder = builder.versification(scheme);
        }
        if !out.creators.is_empty() {
            builder = builder.creators(out.creators.clone());
        }
        if let Some(published) = out.published.clone() {
            builder = builder.published(published);
        }
        for (key, value) in &out.identifiers {
            builder = builder.identifier(key.clone(), value.clone());
        }
        if let Some(raw) = out.raw.clone() {
            builder = builder.raw(raw);
        }
        if out.lossy {
            builder = builder.lossy();
        }

        let mut written = 0usize;
        let mut redacted = 0usize;
        let mut tids = Vec::with_capacity(plans.len());
        let mut entries: Vec<PartEntry> = Vec::with_capacity(plans.len());
        for plan in &plans {
            let text = part::text_of(&out.text, plan.range.clone()).ok_or_else(|| {
                LibError::Unreadable {
                    reader: spec.reader.clone(),
                    at: plan.range.start as usize,
                    what: "a part boundary on a character boundary".to_string(),
                }
            })?;
            let rows = part::local_rows(&out.rows, &plan.range);
            // The **reader field**, not the bare id: record 18 key 1 holds what produced the
            // reading, and a parameter changed it. The format spec says key 1 carries what
            // manifest key 3 carries, and the two being one string is what lets a check compare
            // them. Written with the id alone until TX-P2 step 3, when `whatsapp/1`'s date
            // order made the difference visible.
            let reading = Reading::new(text.tid, &reader_field, rows);
            entries.push(reading.entry(plan.len()));
            tids.push(text.tid);

            // **Rule Z, where text actually rests.** A part this library has redacted is not
            // written back by re-adding the file it came from. Dropped and counted rather than
            // refused, which is what the spec's rule says and what makes it survive a merge: a
            // peer that never heard of the redaction will offer the bytes in good faith, and a
            // refusal would turn one redaction anywhere into a permanent failure for everybody.
            //
            // The manifest is still written. An expression that was read from a text does not
            // stop having been read from it, and a manifest naming a part whose bytes are not
            // held is the honest record of a redacted corpus — `text show` reports the absence
            // rather than inventing the text.
            if self.store.is_redacted(&text.tid) {
                redacted += 2;
            } else {
                if self.objects.put_part(&text)? {
                    written += 1;
                }
                let (_, wrote) = self.objects.put_reading(&reading.to_record())?;
                if wrote {
                    written += 1;
                }
            }
        }

        // An append that changes nothing changes nothing.
        //
        // Re-running a sync before the export has grown is routine, and without this it would
        // write a new manifest whose only difference from the head is that it supersedes it —
        // a version chain of identical versions, which makes `supersedes` mean nothing. So the
        // head's own part entries are compared against the ones just built, and an append that
        // matches returns the head: same mid, no record written, `supersedes: None` to say that
        // no version was created. The objects were already there, so `written` is zero too.
        //
        // Entries and not tids: an entry carries the length, the structure hash and the rdid as
        // well, so this is "the same text read the same way" rather than "the same bytes".
        if let Some(previous) = supersedes {
            if let Some(head) = self.store.manifest(&previous) {
                if head.parts == entries {
                    return Ok(Added {
                        mid: previous,
                        parts: tids,
                        objects_written: written,
                        objects_redacted: redacted,
                        bytes: out.text.len() as u64,
                        lossy: out.lossy,
                        supersedes: None,
                    });
                }
            }
        }

        let manifest = builder.parts(entries).build()?;
        let mid = manifest.mid();
        self.append_records(&[Record::Manifest(manifest)])?;

        Ok(Added {
            mid,
            parts: tids,
            objects_written: written,
            objects_redacted: redacted,
            bytes: out.text.len() as u64,
            lossy: out.lossy,
            supersedes,
        })
    }

    /// Append records to the catalog log and to the in-memory store.
    ///
    /// `SMY-E452` travels out of here untouched: a part text or a reading offered to a log is
    /// refused by `Store::append`, whole batch, and this is the path a caller would otherwise
    /// use to get one in.
    ///
    /// Named `append_records` and not `append` since TX-P2 step 3, where [`Library::append`]
    /// became the growth operation SMYSL-2.4 §3.1 names — `text append <file>`, a new version
    /// of an expression. Both are new in 1.10.0, so nothing released moves; the rename is here
    /// because *append* in this crate's vocabulary now means what the command means, and a
    /// method called `append` that took records would be the other thing.
    pub fn append_records(&mut self, records: &[Record]) -> Result<(), LibError> {
        // The stored view describes the library before this write.
        #[cfg(feature = "substrate-redb")]
        {
            self.stored = std::sync::OnceLock::new();
        }
        // Refused whole before any shard is written, as one log refused it.
        Store::check_loggable(records).map_err(|e| match e {
            smysl_core::Error::Lib(e) => e,
            e => LibError::Io {
                at: CATALOG.to_string(),
                message: e.to_string(),
            },
        })?;
        let routes = crate::shards::route(&self.store, &self.uid_shards, records);
        let batches = crate::shards::encode_by_shard(&self.store, records, &routes);
        if batches.keys().any(|s| !self.shards.contains_key(s)) && self.layout == LAYOUT_1 {
            // The marker first: a layout-1 reader must not see a library with a shard it would
            // not read and report it whole.
            std::fs::write(self.root.join(MARKER), format!("{LAYOUT}\n"))
                .map_err(|e| io(MARKER, &e))?;
            self.layout = LAYOUT;
        }
        let mut was = BTreeMap::new();
        for (shard, (bytes, uids)) in &batches {
            if !self.shards.contains_key(shard) {
                let (log, _) = ShardLog::read(&self.root, shard)?;
                self.shards.insert(shard.clone(), log);
            }
            let log = self.shards.get_mut(shard).expect("inserted above");
            was.insert(shard, log.len());
            log.append(bytes)?;
            if !uids.is_empty() {
                self.uid_shards.extend(shard, uids.iter().copied());
            }
        }
        // The logs hold the batch; the store takes it the way it always has, which dedups
        // and absorbs exactly as before and writes nothing, having no path.
        self.store.append(records).map_err(|e| LibError::Io {
            at: CATALOG.to_string(),
            message: e.to_string(),
        })?;
        // Every shard the batch grew, uids or not: a list stamped with the old length would be
        // stale, and read past at the next open.
        for (shard, (_, uids)) in &batches {
            let now = self.shards[shard].len();
            crate::shards::append_uids(&self.root, shard, was[shard], now, uids, || {
                self.uid_shards.uids_of(shard)
            });
        }
        // Every path that grows a library comes through here - `add`, `append` and `redact`
        // all call it rather than reaching the store - so the index is maintained in one
        // place. `SMY-E452` is refused above and the whole batch with it, so there is no
        // half-appended batch for a table to have half-absorbed.
        self.index_appended(records);
        Ok(())
    }

    /// A part's text, verified against the tid it is stored under (`SMY-E446`).
    pub fn part(&self, tid: &Tid) -> Result<PartText, LibError> {
        self.refuse_if_redacted(tid)?;
        self.objects.get_part(tid)
    }

    /// Rule Z, on the way out: say the part is redacted rather than that a file is missing.
    ///
    /// The objects are already gone — [`Library::open`] sees to that — so without this a caller
    /// asking for a redacted passage got `No such file or directory` and an object path, which
    /// reads as a corrupt library rather than as a corpus doing what it was told.
    fn refuse_if_redacted(&self, tid: &Tid) -> Result<(), LibError> {
        if self.store.is_redacted(tid) {
            return Err(LibError::Redacted {
                tid: tid.canonical(),
            });
        }
        Ok(())
    }

    /// A reading, by rdid.
    pub fn reading(&self, rdid: &Rdid) -> Result<Reading, LibError> {
        let record = self.objects.get_reading(rdid)?;
        Reading::from_record(&record).map_err(|e| LibError::ObjectCorrupt {
            id: format!("{} ({e})", rdid.canonical()),
        })
    }

    /// The reading of one part of one manifest, checked against the entry (`SMY-E401`).
    ///
    /// Beside [`Library::structure`] rather than replacing it: a structure is the tree, and a
    /// reading is the rows — which carry what the tree does not, because a node is a range and
    /// a row is a range *plus* who said it, when, and under what id. A chat corpus is the first
    /// one where the difference is visible to anybody, which is why this exists now and did not
    /// before: `text show --segments` has a speaker to print.
    pub fn part_reading(&self, mid: &Mid, tid: &Tid) -> Result<Reading, LibError> {
        self.refuse_if_redacted(tid)?;
        let entry = self.entry_of(mid, tid)?;
        let reading = self.reading(&entry.rdid)?;
        reading.verify_entry(&entry)?;
        Ok(reading)
    }

    /// The structure of one part of one manifest, checked against the entry (`SMY-E401`).
    pub fn structure(&self, mid: &Mid, tid: &Tid, caps: &Caps) -> Result<Structure, LibError> {
        self.refuse_if_redacted(tid)?;
        let entry = self.entry_of(mid, tid)?;
        let reading = self.reading(&entry.rdid)?;
        reading.verify_entry(&entry)?;
        let mut budget = Budget::new(*caps, entry.length)?;
        Structure::build(&reading.rows, entry.length, &mut budget).map_err(|e| {
            LibError::ObjectCorrupt {
                id: format!("{} ({e})", tid.canonical()),
            }
        })
    }

    fn entry_of(&self, mid: &Mid, tid: &Tid) -> Result<PartEntry, LibError> {
        let manifest = self.store.manifest(mid).ok_or_else(|| LibError::Io {
            at: CATALOG.to_string(),
            message: format!("no manifest `{}`", mid.canonical()),
        })?;
        manifest
            .parts
            .iter()
            .find(|e| &e.tid == tid)
            .cloned()
            .ok_or_else(|| LibError::Io {
                at: CATALOG.to_string(),
                message: format!(
                    "manifest `{}` has no part `{}`",
                    mid.canonical(),
                    tid.canonical()
                ),
            })
    }

    /// Where a locator points, inside one of a manifest's parts.
    ///
    /// Every part is tried in document order and the first that resolves wins. A locator names
    /// at most one place in a text — the reading refuses a table that states one address twice
    /// (step 3's first fuzz finding) — so "first" is "the only", and searching rather than
    /// indexing is what keeps this correct while `text ls` is the only index there is.
    pub fn resolve(
        &self,
        mid: &Mid,
        loc: &Locator,
        caps: &Caps,
    ) -> Result<Option<(Tid, Range<u64>)>, LibError> {
        // The `locators` table, when it can answer for this manifest (§3.8). "Can answer" is a
        // question about the manifest and not about the locator: a mid is in the table only
        // when every one of its parts resolved into it, so a *miss* here is an answer and not
        // an absence. A mid that is not there falls through to the search below, which is the
        // answer this method gave before the table existed - and the one it still has to give,
        // because a redacted part makes the search refuse rather than return nothing, and an
        // index that turned a refusal into `Ok(None)` would have changed a result.
        if self.index.has_locators(mid) {
            return Ok(self
                .index
                .resolve(mid, &loc.to_string())
                .map(|at| (at.tid, at.start..at.end)));
        }
        let manifest = self.store.manifest(mid).ok_or_else(|| LibError::Io {
            at: CATALOG.to_string(),
            message: format!("no manifest `{}`", mid.canonical()),
        })?;
        let tids: Vec<Tid> = manifest.parts.iter().map(|e| e.tid).collect();
        for tid in tids {
            let structure = self.structure(mid, &tid, caps)?;
            if let Some(range) = structure.resolve(loc) {
                return Ok(Some((tid, range)));
            }
        }
        Ok(None)
    }

    /// The manifest an alias or a `m3:` identity names.
    ///
    /// An alias with two heads is refused rather than resolved: `SMY-W418` is a warning on a
    /// corpus, but a command that has to pick one of two texts and print it has no defensible
    /// choice, and printing the lower-sorting mid would make the answer depend on a hash.
    pub fn head(&self, who: &str) -> Result<Mid, LibError> {
        if let Ok(mid) = Mid::parse(who) {
            return if self.store.manifest(&mid).is_some() {
                Ok(mid)
            } else {
                Err(LibError::Io {
                    at: CATALOG.to_string(),
                    message: format!("no manifest `{}`", mid.canonical()),
                })
            };
        }
        let heads = self.store.heads(who);
        match heads.len() {
            // Not `BadAlias`: that one means the string is not an alias at all, and saying
            // `kjv/1769 is not a valid alias` about a perfectly well-formed alias that this
            // library has never heard of sends an operator to check their spelling of the one
            // thing that was right.
            0 => Err(LibError::Io {
                at: CATALOG.to_string(),
                message: format!("no text is catalogued under `{who}`"),
            }),
            1 => Ok(heads[0]),
            n => Err(LibError::Io {
                at: CATALOG.to_string(),
                message: format!(
                    "`{who}` has {n} heads ({}); name one of them instead (SMY-W418)",
                    heads
                        .iter()
                        .map(Mid::canonical)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }),
        }
    }

    /// Resolve `<alias|mid>` and a locator into the text it names.
    pub fn passage(&self, who: &str, loc: &Locator, caps: &Caps) -> Result<Passage, LibError> {
        let mid = self.head(who)?;
        let Some((tid, range)) = self.resolve(&mid, loc, caps)? else {
            return Err(LibError::Io {
                at: CATALOG.to_string(),
                message: format!("`{loc}` names nothing in `{who}`"),
            });
        };
        let part = self.part(&tid)?;
        let corrupt = || LibError::ObjectCorrupt {
            id: tid.canonical(),
        };
        // The bytes hashed to the tid, so they are the part — but a tid is a hash of bytes and
        // says nothing about them being UTF-8 or normalised, which `Normalised` is the type
        // that promises. A part whose bytes are not is a corrupt object, not a decoding
        // question for every caller of `passage` to answer.
        let text = std::str::from_utf8(&part.text).map_err(|_| corrupt())?;
        let text = Normalised::already(text).ok_or_else(corrupt)?;
        let slice = text
            .slice(range.start as usize..range.end as usize)
            .ok_or_else(corrupt)?;
        Ok(Passage {
            mid,
            tid,
            range,
            text: slice.to_string(),
        })
    }

    /// Every alias, with its heads and how many manifests it has.
    ///
    /// What `text ls` prints. A `BTreeMap` rather than an iterator because the caller sorts it
    /// anyway and the interesting row is the one with two heads.
    pub fn catalog(&self) -> BTreeMap<String, Vec<Mid>> {
        self.store
            .aliases()
            .map(|a| (a.to_string(), self.store.heads(a)))
            .collect()
    }
}

/// `impl PartResolver for Library`, so `check --library` can verify objects.
///
/// It delegates to the object store rather than to [`Library::part`]: `part` verifies, which is
/// right for every other caller and useless to a checker, since a verified read can never hand
/// back the bad object that `SMY-E446` exists to report.
impl PartResolver for Library {
    fn part(&self, tid: &Tid) -> Resolved {
        self.objects.part(tid)
    }

    /// Where a locator points, for `SMY-W405` (1.10, TX-P5 step 1).
    ///
    /// Through [`Library::resolve`], so the answer is the index's when the index can give one
    /// and the search's when it cannot — a checker asking this question gets the same answer
    /// `text show` would, which is the only answer worth reporting a disagreement against.
    ///
    /// `None` for a locator that names nothing *and* for a manifest whose parts cannot be read:
    /// the caller reports a disagreement, and "I could not look" is not one. A redacted part is
    /// exactly that case — the bytes are gone by design, and `SMY-W405` is not the diagnostic
    /// for a redaction.
    fn locator(&self, mid: &Mid, locator: &str) -> Option<(Tid, std::ops::Range<u64>)> {
        let loc = crate::locator::parse(locator).ok()?;
        self.resolve(mid, &loc, &Caps::DEFAULT).ok().flatten()
    }
}

/// A digest in hex, for a message.
#[cfg(feature = "store-redb")]
fn hex(d: &[u8; 32]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// A shard's marker, relative to the root, for a message.
#[cfg(feature = "store-redb")]
fn marker_rel(shard: &str) -> String {
    crate::shards::primary_marker(shard).display().to_string()
}
