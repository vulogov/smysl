//! The library handle: a catalog log and an object store, opened together (SMYSL-2.4 §4.2).
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
//! <root>/log/catalog.cbor        records 14 and 19, and the units ingest stages later
//! <root>/objects/t3/ab/cd/…      part texts   (record 15 envelopes)
//! <root>/objects/r3/ab/cd/…      readings     (record 18 envelopes)
//! <root>/objects/tmp/            where an object is written before it is renamed into place
//! <root>/lock                    the library lock; `log/<shard>.lock` are the shard locks
//! ```
//!
//! **One log, not one per shard.** §14.2's layout is per-shard and SMYSL-2.8 owns the sharded
//! virtual union; what TX-P1 needs is a catalog, and a sharding scheme written now would be a
//! scheme chosen before anything can measure it. The marker file carries a layout version so
//! that the release which shards can tell an unsharded library apart from a corrupt one.
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
use smysl_core::ids::{LangTag, Mid, Rdid, Tid};
use smysl_core::types::library::{PartResolver, Resolved};
use smysl_core::types::{Carry, PartEntry, PartText, Record};
use smysl_graph::Store;

use crate::limits::{Budget, Caps};
use crate::locator::Locator;
use crate::manifest::ManifestBuilder;
use crate::norm::Normalised;
use crate::objects::ObjectStore;
use crate::part::{self, Policy};
use crate::readers::{read_with, reader, Input, Params};
use crate::reading::{Reading, Segment};
use crate::structure::Structure;

/// The marker file that says a directory is a library.
///
/// A file rather than a naming convention, because `-s/--store` has meant "a path to a log"
/// since 1.0 and a directory passed to it has to be recognised rather than guessed at. Its
/// contents are the layout version: a library written by a later release can then be refused
/// by name instead of half-read.
pub const MARKER: &str = "LIBRARY";

/// The layout this release writes and reads.
pub const LAYOUT: &str = "smysl/library/1";

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
        }
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
    /// The normalised length of the whole text.
    pub bytes: u64,
    /// The reader dropped something the source carried.
    pub lossy: bool,
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
    store: Store,
    objects: ObjectStore,
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
        if version != LAYOUT {
            // Named rather than guessed at. A library written by a later layout may differ in
            // where the logs are, and half-reading it would report an empty catalog for a
            // library that is full.
            return Err(LibError::Io {
                at: MARKER.to_string(),
                message: format!("layout `{version}` is not `{LAYOUT}`; this library was written by another release"),
            });
        }
        let log = root.join(CATALOG);
        let store = Store::open(&log).map_err(|e: smysl_core::Error| LibError::Io {
            at: CATALOG.to_string(),
            message: e.to_string(),
        })?;
        let objects = ObjectStore::open(&root.join("objects"))?;
        Ok(Library {
            root: root.to_path_buf(),
            store,
            objects,
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

    /// The shard names a library-wide write locks, for [`crate::lock::acquire_all`].
    ///
    /// One, while there is one log. It is a method rather than a constant so that the release
    /// which shards changes the lock set in one place.
    pub fn shards(&self) -> Vec<String> {
        vec!["catalog".to_string()]
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
        let out = read_with(reader, input, &spec.params, &mut budget)?;

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
        if let Some(title) = spec.title.clone().or_else(|| out.title.clone()) {
            builder = builder.title(title);
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
            let rows = part_local_rows(&out.rows, &plan.range);
            let reading = Reading::new(text.tid, &spec.reader, rows);
            entries.push(reading.entry(plan.len()));
            tids.push(text.tid);

            if self.objects.put_part(&text)? {
                written += 1;
            }
            let (_, wrote) = self.objects.put_reading(&reading.to_record())?;
            if wrote {
                written += 1;
            }
        }

        let manifest = builder.parts(entries).build()?;
        let mid = manifest.mid();
        self.append(&[Record::Manifest(manifest)])?;

        Ok(Added {
            mid,
            parts: tids,
            objects_written: written,
            bytes: out.text.len() as u64,
            lossy: out.lossy,
        })
    }

    /// Append records to the catalog log and to the in-memory store.
    ///
    /// `SMY-E452` travels out of here untouched: a part text or a reading offered to a log is
    /// refused by `Store::append`, whole batch, and this is the path a caller would otherwise
    /// use to get one in.
    pub fn append(&mut self, records: &[Record]) -> Result<(), LibError> {
        self.store.append(records).map_err(|e| LibError::Io {
            at: CATALOG.to_string(),
            message: e.to_string(),
        })?;
        Ok(())
    }

    /// A part's text, verified against the tid it is stored under (`SMY-E446`).
    pub fn part(&self, tid: &Tid) -> Result<PartText, LibError> {
        self.objects.get_part(tid)
    }

    /// A reading, by rdid.
    pub fn reading(&self, rdid: &Rdid) -> Result<Reading, LibError> {
        let record = self.objects.get_reading(rdid)?;
        Reading::from_record(&record).map_err(|e| LibError::ObjectCorrupt {
            id: format!("{} ({e})", rdid.canonical()),
        })
    }

    /// The structure of one part of one manifest, checked against the entry (`SMY-E401`).
    pub fn structure(&self, mid: &Mid, tid: &Tid, caps: &Caps) -> Result<Structure, LibError> {
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
}

/// A reader's rows, restricted to one part and shifted to the part's own offsets.
///
/// The shift is the whole point and the reason this is a named function with a test. A row's
/// `start`/`end` are offsets into **the part**, and a reader produces one table over the whole
/// text; a part that begins at byte 70,000 would otherwise carry rows pointing past its own
/// end, `Structure::build` would refuse the table, and the structure hash — which is over the
/// table — would be a hash of the wrong thing in the manifest entry.
///
/// Nothing caught this until now because every reader fixture in the tree produces exactly one
/// part, and for one part the shift is zero.
fn part_local_rows(rows: &[Segment], range: &Range<u64>) -> Vec<Segment> {
    rows.iter()
        .filter(|r| r.start >= range.start && r.end <= range.end)
        .map(|r| {
            let mut row = r.clone();
            row.start -= range.start;
            row.end -= range.start;
            row
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reading::Level;

    fn level(s: &str) -> Level {
        Level::new(s).expect("a level")
    }

    fn row(start: u64, end: u64, loc: &str) -> Segment {
        Segment::new(
            start,
            end,
            level("line"),
            crate::locator::parse(loc).expect("a locator"),
        )
    }

    /// The shift, stated as the property the manifest entry depends on: a part's rows start at
    /// zero and end at the part's length, whatever the part's place in the text.
    #[test]
    fn a_parts_rows_are_its_own_offsets() {
        let rows = vec![row(0, 10, "L1"), row(10, 25, "L2"), row(25, 40, "L3")];
        let second = part_local_rows(&rows, &(10..40));
        assert_eq!(second.len(), 2);
        assert_eq!((second[0].start, second[0].end), (0, 15));
        assert_eq!((second[1].start, second[1].end), (15, 30));

        // The first part is the case that hid this: its shift is zero.
        let first = part_local_rows(&rows, &(0..10));
        assert_eq!((first[0].start, first[0].end), (0, 10));
    }

    /// A row straddling a boundary belongs to neither part.
    ///
    /// It cannot happen from `group`, which cuts on whole nodes, and the filter is written to
    /// be total anyway: a row half in a part would otherwise be shifted to a negative offset
    /// and panic in a debug build.
    #[test]
    fn a_straddling_row_is_in_no_part() {
        let rows = vec![row(0, 20, "L1")];
        assert!(part_local_rows(&rows, &(10..20)).is_empty());
        assert!(part_local_rows(&rows, &(0..10)).is_empty());
    }
}
