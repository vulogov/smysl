//! Import: files into a library, each record accepted, staged, waiting or quarantined
//! (SMYSL-2.7 §3.1–§3.3).
//!
//! [`import`] reads its inputs, decodes each file whole or not at all (`SMY-E493`), writes one
//! manifest per file, and classifies every record the library does not already hold. With
//! `accept` the accepted records are appended. Without it they are staged, exact, in
//! `lib/staged/<name>.cbor`, for [`accept`] or [`discard`]. Records that wait go to
//! `lib/waiting/<name>.cbor` and the `waiting` table, and are promoted when what they wait for
//! arrives. Quarantined records go to `lib/quarantine/<name>/` with their findings.
//!
//! Step 4 of TX-P11. Not yet: records 15 and 18, which a log cannot hold and step 6 puts in the
//! object store (they are counted and left out); `--keep-original`, `--attest` and joins
//! (steps 6 and 9); redaction at the door (step 6).

pub mod classify;
mod decode;
mod manifest;

use crate::access::LibraryWrite;
use crate::export::{slug, LibError};
use crate::tables::{write_atomic, Batch, State, Tables, Waiting};
use classify::{classify, provides, Class, Id};
use smysl_core::{from_cbor_seq, to_cbor, Code, Contention, Diagnostic, Mid, Record, Report};
use smysl_graph::merge::contention::detect;
use smysl_graph::{record_set_digest, DetectionContext, Store};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// What to import.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Input {
    /// One file, surface or CBOR.
    File(PathBuf),
    /// Every file in a directory, not recursively, in byte order of name.
    Dir(PathBuf),
    /// An `export-index.cbor`: the collection it lists, each file checked against it first.
    Index(PathBuf),
    /// Bytes the caller holds, under the file name a manifest records.
    Bytes { name: String, bytes: Vec<u8> },
}

/// How to import.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImportOptions {
    /// The import's name: `import:<name>` is its manifests' alias. Defaults to the first
    /// input's stem, slugged.
    pub name: Option<String>,
    /// Append what is accepted, rather than stage it. Staging is the default for every source
    /// (OQ-25): without signatures a library's own export cannot be told from a file that
    /// claims to be one.
    pub accept: bool,
    /// The SPDX id the manifests record. Defaults to `unknown`.
    pub licence: String,
    /// Where the files came from, as a URL; otherwise each manifest records its file name.
    pub origin: Option<String>,
}

impl Default for ImportOptions {
    fn default() -> ImportOptions {
        ImportOptions {
            name: None,
            accept: false,
            licence: "unknown".to_string(),
            origin: None,
        }
    }
}

impl ImportOptions {
    pub fn new() -> ImportOptions {
        ImportOptions::default()
    }

    pub fn name(mut self, name: impl Into<String>) -> ImportOptions {
        self.name = Some(name.into());
        self
    }

    pub fn accept(mut self, accept: bool) -> ImportOptions {
        self.accept = accept;
        self
    }

    pub fn licence(mut self, licence: impl Into<String>) -> ImportOptions {
        self.licence = licence.into();
        self
    }

    pub fn origin(mut self, url: impl Into<String>) -> ImportOptions {
        self.origin = Some(url.into());
        self
    }
}

/// What became of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FileOutcome {
    pub file: String,
    /// Its manifest. `None` when it did not decode.
    pub mid: Option<Mid>,
    /// BLAKE3 of its bytes.
    pub original: [u8; 32],
    /// The record-set digest of what it decoded to.
    pub rsd: [u8; 32],
    /// Records the library did not hold, and records it already did.
    pub added: usize,
    pub duplicates: usize,
    pub accepted: usize,
    pub staged: usize,
    pub waiting: usize,
    pub quarantined: usize,
    /// Records 15 and 18, which a log does not hold; the object store takes them in step 6.
    pub text: usize,
    /// Its manifest was already in the library: the same bytes, under the same options.
    pub already_imported: bool,
    /// It did not decode (`SMY-E493`), and nothing from it was used.
    pub rejected: bool,
}

/// What an import did.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImportReport {
    pub name: String,
    pub files: Vec<FileOutcome>,
    /// The manifests this import wrote.
    pub manifests: Vec<Mid>,
    /// Contentions the appended records gave rise to: detected, reported, not recorded (§2.2).
    pub contentions: Vec<Contention>,
    /// `SMY-E493`, `E494`, `W423` per quarantined record, and `W496` with the waiting count.
    pub report: Report,
}

impl ImportReport {
    fn new(name: &str) -> ImportReport {
        ImportReport {
            name: name.to_string(),
            files: Vec::new(),
            manifests: Vec::new(),
            contentions: Vec::new(),
            report: Report::new(),
        }
    }
}

// --- Where things are kept (§4.1) -----------------------------------------------------------

fn staged_path(root: &Path, name: &str) -> PathBuf {
    root.join("lib").join("staged").join(format!("{name}.cbor"))
}

fn waiting_path(root: &Path, name: &str) -> PathBuf {
    root.join("lib")
        .join("waiting")
        .join(format!("{name}.cbor"))
}

fn quarantine_dir(root: &Path, name: &str) -> PathBuf {
    root.join("lib").join("quarantine").join(name)
}

fn hash(r: &Record) -> [u8; 32] {
    Store::record_hash(r)
}

/// The records of a file this module wrote; none when there is no file.
fn read_records(p: &Path) -> Result<Vec<Record>, LibError> {
    match std::fs::read(p) {
        Ok(b) => from_cbor_seq(&b)
            .map(|(rs, _)| rs)
            .map_err(|e| LibError::Io(p.display().to_string(), e.to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(LibError::Io(p.display().to_string(), e.to_string())),
    }
}

/// Write records exact, each once, in the canonical order (§3.7): by type code, then hash.
fn write_records<'a>(
    p: &Path,
    records: impl IntoIterator<Item = &'a Record>,
) -> Result<(), LibError> {
    let sorted: BTreeMap<(u64, [u8; 32]), &Record> = records
        .into_iter()
        .map(|r| ((r.type_code(), hash(r)), r))
        .collect();
    let mut bytes = Vec::new();
    for r in sorted.values() {
        bytes.extend_from_slice(&to_cbor(r));
    }
    write_atomic(p, &bytes)
}

fn remove(p: &Path) -> Result<(), LibError> {
    match std::fs::remove_file(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(LibError::Io(p.display().to_string(), e.to_string())),
    }
}

/// A name is a layout key's slug, which every alias segment admits.
fn check_name(n: &str) -> Result<String, LibError> {
    match slug(n) {
        (s, true) => Ok(s),
        _ => Err(LibError::BadName(n.to_string())),
    }
}

fn contention_ids(store: &Store) -> BTreeSet<smysl_core::ContentionId> {
    detect(store, &DetectionContext::default())
        .into_iter()
        .map(|k| k.id)
        .collect()
}

// --- import -----------------------------------------------------------------------------------

/// Import `inputs` into the library, as one import named by `opts.name`.
///
/// Every file is decoded first, whole or not at all, and every record the library does not
/// hold is classified once, over the files together: a directory or a collection is checked
/// as one batch. The manifests are appended whatever happens to the content, since they record
/// that a file was read, not that what it holds was accepted (§3.2 step 7).
///
/// The same files under the same options a second time change nothing: their manifests are
/// already in the library, and each file says `already_imported`.
pub fn import(
    lib: &mut impl LibraryWrite,
    inputs: &[Input],
    opts: &ImportOptions,
) -> Result<ImportReport, LibError> {
    let mut report = Report::new();
    let (pieces, default_name) = decode::read(inputs, &mut report)?;
    let name = match &opts.name {
        Some(n) => check_name(n)?,
        None => default_name,
    };
    let root = lib.root().to_path_buf();
    let mut tables = Tables::load(&root)?;
    let library = lib.snapshot().clone();
    let mut out = ImportReport::new(&name);

    // Decode, and name each file's manifest.
    struct Read {
        records: Vec<Record>,
        manifest: Record,
        mid: Mid,
    }
    let several = pieces.len() > 1;
    let mut read: Vec<Option<Read>> = Vec::new();
    for p in &pieces {
        let original = *blake3::hash(&p.bytes).as_bytes();
        let mut outcome = FileOutcome {
            file: p.file.clone(),
            mid: None,
            original,
            rsd: [0; 32],
            added: 0,
            duplicates: 0,
            accepted: 0,
            staged: 0,
            waiting: 0,
            quarantined: 0,
            text: 0,
            already_imported: false,
            rejected: false,
        };
        let decoded = match decode::decode(&p.file, &p.bytes) {
            Ok(rs) => rs,
            Err(d) => {
                report.push(d);
                outcome.rejected = true;
                out.files.push(outcome);
                read.push(None);
                continue;
            }
        };
        let mut seen = BTreeSet::new();
        let mut records = Vec::new();
        for r in decoded {
            if matches!(r, Record::PartText(_) | Record::PartReading(_)) {
                outcome.text += 1;
            } else if seen.insert(hash(&r)) {
                records.push(r);
            }
        }
        outcome.rsd = record_set_digest(&records);
        let alias = if several {
            format!("import:{name}/{}", p.stem)
        } else {
            format!("import:{name}")
        };
        let m = manifest::build(&alias, &p.file, &original, &outcome.rsd, opts)?;
        let mid = m.mid();
        outcome.mid = Some(mid);
        outcome.already_imported = library.manifest(&mid).is_some();
        out.files.push(outcome);
        read.push(Some(Read {
            records,
            manifest: Record::Manifest(m),
            mid,
        }));
    }
    let live: Vec<usize> = (0..read.len())
        .filter(|i| read[*i].is_some() && !out.files[*i].already_imported)
        .collect();
    if live.is_empty() {
        out.report = report;
        return Ok(out);
    }
    if tables.batches.contains_key(&name) {
        return Err(LibError::NameTaken(name));
    }

    // What the library does not hold yet, once each, and which files hold it.
    let mut incoming: BTreeMap<[u8; 32], Record> = BTreeMap::new();
    let mut held_by: BTreeMap<[u8; 32], BTreeSet<Mid>> = BTreeMap::new();
    for &i in &live {
        let r = read[i].as_ref().expect("live");
        for rec in &r.records {
            let h = hash(rec);
            held_by.entry(h).or_default().insert(r.mid);
            if library.contains_record(&h) {
                out.files[i].duplicates += 1;
            } else {
                out.files[i].added += 1;
                incoming.entry(h).or_insert_with(|| rec.clone());
            }
        }
    }
    let classes = classify(&library, &incoming, &BTreeSet::new());

    let accepted_state = if opts.accept {
        State::Accepted
    } else {
        State::Staged
    };
    let mut accepted = Vec::new();
    let mut waiting = Vec::new();
    let mut quarantined: BTreeMap<String, Vec<(&Record, Vec<Diagnostic>)>> = BTreeMap::new();
    for (h, class) in &classes {
        let r = &incoming[h];
        match class {
            Class::Accepted => accepted.push(r.clone()),
            Class::Waiting(missing) => {
                waiting.push(r.clone());
                tables.waiting.insert(
                    *h,
                    Waiting {
                        name: name.clone(),
                        missing: missing.clone(),
                    },
                );
            }
            Class::Quarantined(ds) => {
                // Filed under the first file that holds it, by its manifest's order here.
                let i = live
                    .iter()
                    .copied()
                    .find(|i| held_by[h].contains(&read[*i].as_ref().expect("live").mid))
                    .expect("an incoming record came from a file");
                for d in ds {
                    report.push(Diagnostic::new(Code::W423).with_message(format!(
                        "{} {}: {} {}",
                        r.type_name(),
                        smysl_core::Uid::from_bytes(*h).short(),
                        d.code,
                        d.message
                    )));
                }
                quarantined
                    .entry(pieces[i].stem.clone())
                    .or_default()
                    .push((r, ds.clone()));
            }
        }
    }
    for &i in &live {
        let mid = read[i].as_ref().expect("live").mid;
        for (h, class) in &classes {
            if !held_by[h].contains(&mid) {
                continue;
            }
            let f = &mut out.files[i];
            match class {
                Class::Accepted if opts.accept => f.accepted += 1,
                Class::Accepted => f.staged += 1,
                Class::Waiting(_) => f.waiting += 1,
                Class::Quarantined(_) => f.quarantined += 1,
            }
        }
    }
    if !waiting.is_empty() {
        report.push(Diagnostic::new(Code::W496).with_message(format!(
            "{} record(s) of import:{name} wait for what the library does not hold",
            waiting.len()
        )));
    }

    // Write. The log first, in one append: manifests always, accepted records with `accept`.
    let before = contention_ids(&library);
    let manifests: Vec<Record> = live
        .iter()
        .map(|i| read[*i].as_ref().expect("live").manifest.clone())
        .collect();
    let mut append = manifests;
    if opts.accept {
        append.extend(accepted.iter().cloned());
    }
    lib.append(&append)?;
    if !opts.accept {
        write_records(&staged_path(&root, &name), &accepted)?;
    }
    if !waiting.is_empty() {
        write_records(&waiting_path(&root, &name), &waiting)?;
    }
    for (stem, rs) in &quarantined {
        let dir = quarantine_dir(&root, &name);
        write_records(
            &dir.join(format!("{stem}.cbor")),
            rs.iter().map(|(r, _)| *r),
        )?;
        let mut lines = String::new();
        for (r, ds) in rs {
            for d in ds {
                lines.push_str(&format!(
                    "{} {}: {d}\n",
                    r.type_name(),
                    smysl_core::Uid::from_bytes(hash(r)).short()
                ));
            }
        }
        write_atomic(&dir.join(format!("{stem}.report")), lines.as_bytes())?;
    }

    // The tables: every record each file holds, under its manifest.
    for (h, mids) in &held_by {
        let state = match classes.get(h) {
            None | Some(Class::Accepted) if library.contains_record(h) => State::Accepted,
            None | Some(Class::Accepted) => accepted_state,
            Some(Class::Waiting(_)) => State::Waiting,
            Some(Class::Quarantined(_)) => State::Quarantined,
        };
        tables
            .imported
            .entry(*h)
            .or_default()
            .extend(mids.iter().map(|m| (*m, state)));
    }
    let mids: Vec<Mid> = live
        .iter()
        .map(|i| read[*i].as_ref().expect("live").mid)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    tables.batches.insert(
        name.clone(),
        Batch {
            name: name.clone(),
            mids: mids.clone(),
            state: accepted_state,
            accepted: if opts.accept {
                accepted.len() as u64
            } else {
                0
            },
            staged: if opts.accept {
                0
            } else {
                accepted.len() as u64
            },
            waiting: waiting.len() as u64,
            quarantined: quarantined.values().map(Vec::len).sum::<usize>() as u64,
            tool: crate::TOOL.to_string(),
        },
    );
    if opts.accept {
        promote(lib, &mut tables, &mut report)?;
    }
    tables.save(&root)?;

    out.manifests = mids;
    out.contentions = detect(lib.snapshot(), &DetectionContext::default())
        .into_iter()
        .filter(|k| !before.contains(&k.id))
        .collect();
    out.report = report;
    Ok(out)
}

/// Promote what waits, to a fixpoint: every waiting record whose import is not discarded is
/// classified again over the library as it now is (§3.2 step 9), and goes where its import's
/// content went. An import still staged holds its own staged records as held.
fn promote(
    lib: &mut impl LibraryWrite,
    tables: &mut Tables,
    report: &mut Report,
) -> Result<usize, LibError> {
    let root = lib.root().to_path_buf();
    let mut total = 0;
    loop {
        let mut moved = 0;
        let names: BTreeSet<String> = tables.waiting.values().map(|w| w.name.clone()).collect();
        for name in names {
            let Some(batch) = tables.batches.get(&name).cloned() else {
                continue;
            };
            if batch.state == State::Discarded {
                continue;
            }
            let file = waiting_path(&root, &name);
            let all = read_records(&file)?;
            let library = lib.snapshot().clone();
            let mut already = Vec::new();
            let candidates: BTreeMap<[u8; 32], Record> = all
                .iter()
                .filter(|r| tables.waiting.get(&hash(r)).is_some_and(|w| w.name == name))
                .filter_map(|r| {
                    let h = hash(r);
                    if library.contains_record(&h) {
                        already.push(h);
                        None
                    } else {
                        Some((h, r.clone()))
                    }
                })
                .collect();
            let staged = match batch.state {
                State::Staged => read_records(&staged_path(&root, &name))?,
                _ => Vec::new(),
            };
            let extra: BTreeSet<Id> = staged.iter().flat_map(provides).collect();
            let classes = classify(&library, &candidates, &extra);

            let mut accepted = Vec::new();
            let mut quarantined = Vec::new();
            for (h, c) in &classes {
                match c {
                    Class::Accepted => accepted.push(candidates[h].clone()),
                    Class::Quarantined(ds) => {
                        for d in ds {
                            report.push(Diagnostic::new(Code::W423).with_message(format!(
                                "{} {}: {} {}",
                                candidates[h].type_name(),
                                smysl_core::Uid::from_bytes(*h).short(),
                                d.code,
                                d.message
                            )));
                        }
                        quarantined.push(candidates[h].clone());
                    }
                    Class::Waiting(m) => {
                        if let Some(w) = tables.waiting.get_mut(h) {
                            w.missing = m.clone();
                        }
                    }
                }
            }
            if accepted.is_empty() && quarantined.is_empty() && already.is_empty() {
                continue;
            }
            let mids: BTreeSet<Mid> = batch.mids.iter().copied().collect();
            let to = batch.state;
            match to {
                State::Accepted => lib.append(&accepted)?,
                _ => write_records(
                    &staged_path(&root, &name),
                    staged.iter().chain(accepted.iter()),
                )?,
            }
            if !quarantined.is_empty() {
                let p = quarantine_dir(&root, &name).join("promoted.cbor");
                let mut q = read_records(&p)?;
                q.extend(quarantined.iter().cloned());
                write_records(&p, &q)?;
            }
            for r in &accepted {
                tables.set_state(&hash(r), &mids, to);
            }
            for h in &already {
                tables.set_state(h, &mids, State::Accepted);
            }
            for r in &quarantined {
                tables.set_state(&hash(r), &mids, State::Quarantined);
            }
            let gone: BTreeSet<[u8; 32]> = accepted
                .iter()
                .chain(quarantined.iter())
                .map(hash)
                .chain(already.iter().copied())
                .collect();
            tables.waiting.retain(|h, _| !gone.contains(h));
            let rest: Vec<&Record> = all.iter().filter(|r| !gone.contains(&hash(r))).collect();
            if rest.is_empty() {
                remove(&file)?;
            } else {
                write_records(&file, rest)?;
            }
            let b = tables.batches.get_mut(&name).expect("held above");
            let n = (accepted.len() + already.len()) as u64;
            match to {
                State::Accepted => b.accepted += n,
                _ => b.staged += n,
            }
            b.quarantined += quarantined.len() as u64;
            b.waiting -= gone.len() as u64;
            moved += gone.len();
        }
        total += moved;
        if moved == 0 {
            return Ok(total);
        }
    }
}

// --- status, accept, discard ------------------------------------------------------------------

/// Every import, or the one named: `lib status`.
pub fn status(root: &Path, name: Option<&str>) -> Result<Vec<Batch>, LibError> {
    let tables = Tables::load(root)?;
    Ok(match name {
        Some(n) => vec![tables
            .batches
            .get(n)
            .cloned()
            .ok_or_else(|| LibError::NoImport(n.to_string()))?],
        None => tables.batches.into_values().collect(),
    })
}

/// Merge a staged import into the library: `lib accept`.
///
/// With `fail_on_contention`, an import whose records would give rise to a contention the
/// library does not already have is refused and stays staged (exit 5).
pub fn accept(
    lib: &mut impl LibraryWrite,
    name: &str,
    fail_on_contention: bool,
) -> Result<ImportReport, LibError> {
    let root = lib.root().to_path_buf();
    let mut tables = Tables::load(&root)?;
    let batch = tables
        .batches
        .get(name)
        .cloned()
        .ok_or_else(|| LibError::NoImport(name.to_string()))?;
    if batch.state != State::Staged {
        return Err(LibError::NotStaged(name.to_string()));
    }
    let records = read_records(&staged_path(&root, name))?;
    let before = contention_ids(lib.snapshot());
    let candidate = Store::from_records(
        lib.snapshot()
            .iter()
            .cloned()
            .chain(records.iter().cloned())
            .collect(),
    );
    let new: Vec<Contention> = detect(&candidate, &DetectionContext::default())
        .into_iter()
        .filter(|k| !before.contains(&k.id))
        .collect();
    if fail_on_contention && !new.is_empty() {
        return Err(LibError::Contentions(new.len()));
    }
    lib.append(&records)?;
    let mids: BTreeSet<Mid> = batch.mids.iter().copied().collect();
    for r in &records {
        tables.set_state(&hash(r), &mids, State::Accepted);
    }
    let b = tables.batches.get_mut(name).expect("held above");
    b.state = State::Accepted;
    b.accepted += b.staged;
    b.staged = 0;
    remove(&staged_path(&root, name))?;
    let mut report = Report::new();
    promote(lib, &mut tables, &mut report)?;
    tables.save(&root)?;
    let mut out = ImportReport::new(name);
    out.manifests = batch.mids;
    out.contentions = new;
    out.report = report;
    Ok(out)
}

/// Drop a staged import's staged and waiting records: `lib discard`. Its manifests stay, since
/// the files were read, and so do its quarantined records and their findings. Returns how many
/// records were dropped.
pub fn discard(lib: &mut impl LibraryWrite, name: &str) -> Result<usize, LibError> {
    let root = lib.root().to_path_buf();
    let mut tables = Tables::load(&root)?;
    let batch = tables
        .batches
        .get(name)
        .cloned()
        .ok_or_else(|| LibError::NoImport(name.to_string()))?;
    if batch.state != State::Staged {
        return Err(LibError::NotStaged(name.to_string()));
    }
    let staged = read_records(&staged_path(&root, name))?;
    let waiting = read_records(&waiting_path(&root, name))?;
    let mids: BTreeSet<Mid> = batch.mids.iter().copied().collect();
    for r in staged.iter().chain(waiting.iter()) {
        tables.set_state(&hash(r), &mids, State::Discarded);
    }
    tables.waiting.retain(|_, w| w.name != name);
    remove(&staged_path(&root, name))?;
    remove(&waiting_path(&root, name))?;
    let b = tables.batches.get_mut(name).expect("held above");
    b.state = State::Discarded;
    b.staged = 0;
    b.waiting = 0;
    tables.save(&root)?;
    Ok(staged.len() + waiting.len())
}

/// Every record an import retained, whatever became of it: accepted, staged, waiting or
/// quarantined (§3.9's import-side record set), each once. The manifests it wrote are not
/// among them; they are the import's, not the files'.
pub fn retained(lib: &impl LibraryWrite, name: &str) -> Result<Vec<Record>, LibError> {
    let root = lib.root().to_path_buf();
    let tables = Tables::load(&root)?;
    let batch = tables
        .batches
        .get(name)
        .ok_or_else(|| LibError::NoImport(name.to_string()))?;
    let mids: BTreeSet<&Mid> = batch.mids.iter().collect();
    let wanted: BTreeSet<[u8; 32]> = tables
        .imported
        .iter()
        .filter(|(_, set)| {
            set.iter()
                .any(|(m, st)| mids.contains(m) && *st != State::Discarded)
        })
        .map(|(h, _)| *h)
        .collect();
    let mut found: BTreeMap<[u8; 32], Record> = BTreeMap::new();
    let mut take = |r: &Record| {
        let h = hash(r);
        if wanted.contains(&h) {
            found.entry(h).or_insert_with(|| r.clone());
        }
    };
    lib.snapshot().iter().for_each(&mut take);
    read_records(&staged_path(&root, name))?
        .iter()
        .for_each(&mut take);
    read_records(&waiting_path(&root, name))?
        .iter()
        .for_each(&mut take);
    let q = quarantine_dir(&root, name);
    if let Ok(entries) = std::fs::read_dir(&q) {
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "cbor"))
            .collect();
        paths.sort();
        for p in paths {
            read_records(&p)?.iter().for_each(&mut take);
        }
    }
    Ok(found.into_values().collect())
}
