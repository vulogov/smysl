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
use smysl_core::ids::{AgentId, LangTag, Mid, Rdid, Tid, Uid};
use smysl_core::types::library::{PartResolver, Redaction, Resolved};
use smysl_core::types::provenance::Hlc;
use smysl_core::types::{Carry, PartEntry, PartText, Record};
use smysl_graph::Store;

use crate::limits::{Budget, Caps};
use crate::locator::Locator;
use crate::manifest::ManifestBuilder;
use crate::norm::Normalised;
use crate::objects::ObjectStore;
use crate::part::{self, Policy};
use crate::readers::{read_with, reader, Input, Params};
use crate::reading::Reading;
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
        let mut library = Library {
            root: root.to_path_buf(),
            store,
            objects,
        };
        library.enforce_z()?;
        Ok(library)
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
        self.store.append(records).map_err(|e| LibError::Io {
            at: CATALOG.to_string(),
            message: e.to_string(),
        })?;
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
