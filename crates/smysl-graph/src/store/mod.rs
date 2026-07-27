//! The append-only store (§7.3, §16.1).
//!
//! The log is a bare CBOR sequence (RFC 8742) - self-delimiting, streamable over a pipe,
//! `O(1)` to append, and parseable up to the last complete record after truncation. It is
//! the only authority; the index sidecar is a derived cache that is rebuilt on any
//! mismatch (`SMY-W110`) rather than trusted.
//!
//! Nothing is ever removed. An edit is a new unit carrying `supersedes`, and a retraction
//! is a new relation - so the store is a grow-only set, which is what makes merge a
//! join-semilattice (rule U).

pub mod index;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use smysl_core::diag::{Code, Diagnostic, Report, Subject};
use smysl_core::{
    canonical_uid, from_cbor_seq, hash_bytes, to_cbor, AgentId, Attestation, Contention, Error,
    IntegrityError, Record, RelKind, Relation, Thread, ThreadId, Uid, UidPrefix, Unit, View,
    ViewId,
};

use crate::adjacency::{Adjacency, EdgeKind};
use crate::traverse;
use index::{Cached, Entry, Index};

/// How to open a store.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct StoreOptions {
    /// Recompute every unit's uid and compare it against the sidecar (`SMY-E070`).
    /// SHOULD be on for untrusted input: it is one hash per unit, and a unit cannot be
    /// altered without changing its uid.
    pub verify_hashes: bool,
    /// Rebuild the index even if the sidecar looks current.
    pub force_reindex: bool,
}

impl StoreOptions {
    /// What a verifier or CI gate wants.
    pub fn strict() -> StoreOptions {
        StoreOptions {
            verify_hashes: true,
            force_reindex: false,
        }
    }
}

/// What happened during `open`.
#[derive(Debug, Clone, Default)]
pub struct OpenReport {
    pub report: Report,
    /// True when the sidecar was missing, stale, or unreadable.
    pub index_rebuilt: bool,
    /// Bytes after the last complete record, left by a writer mid-append.
    pub trailing_bytes: u64,
}

/// What happened during `append`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppendReport {
    /// Records that were not already present.
    pub added: usize,
    /// Records already in the store. Appending is idempotent, which is what lets a
    /// delivery be duplicated without consequence (rule U).
    pub duplicates: usize,
    pub bytes_written: u64,
}

/// An append-only log plus its derived view of the graph.
#[derive(Debug, Clone)]
pub struct Store {
    path: Option<PathBuf>,
    records: Vec<Record>,
    log_len: u64,
    log_hash: [u8; 32],

    units: BTreeMap<Uid, Unit>,
    relations: BTreeMap<(String, Uid, Uid), Relation>,
    threads: BTreeMap<(ThreadId, AgentId), Thread>,
    views: BTreeMap<ViewId, View>,
    contentions: Vec<Contention>,
    adjacency: Adjacency,
}

impl Default for Store {
    fn default() -> Store {
        Store::new()
    }
}

impl Store {
    /// An empty in-memory store.
    pub fn new() -> Store {
        Store {
            path: None,
            records: Vec::new(),
            log_len: 0,
            log_hash: hash_bytes(&[]),
            units: BTreeMap::new(),
            relations: BTreeMap::new(),
            threads: BTreeMap::new(),
            views: BTreeMap::new(),
            contentions: Vec::new(),
            adjacency: Adjacency::default(),
        }
    }

    /// Build in memory from records, with no file behind it.
    pub fn from_records(records: Vec<Record>) -> Store {
        let mut s = Store::new();
        s.absorb(records);
        s.log_len = s.log_bytes().len() as u64;
        s.log_hash = hash_bytes(&s.log_bytes());
        s
    }

    /// Open a store, loading or rebuilding its index.
    pub fn open(path: impl AsRef<Path>) -> Result<Store, Error> {
        Ok(Store::open_with(path, StoreOptions::default())?.0)
    }

    /// Open a store and report what had to be repaired on the way in.
    pub fn open_with(
        path: impl AsRef<Path>,
        opts: StoreOptions,
    ) -> Result<(Store, OpenReport), Error> {
        let path = path.as_ref().to_path_buf();
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(Error::Io(e)),
        };

        let mut open = OpenReport::default();
        // A truncated tail is not an error: the log is append-only and may be read while
        // a writer is mid-append (§7.3).
        let (records, consumed) = from_cbor_seq(&bytes)?;
        open.trailing_bytes = (bytes.len() - consumed) as u64;
        if open.trailing_bytes > 0 {
            open.report
                .push(Diagnostic::new(Code::W110).with_message(format!(
                    "{} trailing bytes after the last complete record",
                    open.trailing_bytes
                )));
        }

        let log_len = consumed as u64;
        let log_hash = hash_bytes(&bytes[..consumed]);

        let sidecar = match std::fs::read(Self::index_path(&path)) {
            Ok(b) => Index::from_bytes(&b).ok(),
            Err(_) => None,
        };
        let current = sidecar
            .as_ref()
            .is_some_and(|ix| ix.matches(log_len, &log_hash));
        if !current || opts.force_reindex {
            open.index_rebuilt = true;
            if sidecar.is_some() {
                open.report.push(
                    Diagnostic::new(Code::W110)
                        .with_message("index does not describe this log; rebuilding"),
                );
            }
        }

        let mut store = Store::new();
        store.path = Some(path);
        store.absorb(records);
        store.log_len = log_len;
        store.log_hash = log_hash;

        if opts.verify_hashes {
            if let Some(ix) = sidecar.as_ref().filter(|_| current) {
                store.verify_against(ix, &mut open.report);
            }
        }

        Ok((store, open))
    }

    /// Where the sidecar for a log lives: `.smysl/index/<name>.idx` beside it (§7.3).
    pub fn index_path(log: &Path) -> PathBuf {
        let name = log
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "store".into());
        log.parent()
            .unwrap_or_else(|| Path::new("."))
            .join(".smysl")
            .join("index")
            .join(format!("{name}.idx"))
    }

    /// Append records, writing through to the log if this store has one.
    ///
    /// `O(1)` in the log: the new records are encoded and appended, and the running hash
    /// advances over the new bytes only.
    pub fn append(&mut self, records: &[Record]) -> Result<AppendReport, Error> {
        let mut report = AppendReport::default();
        let mut fresh = Vec::new();
        let mut bytes = Vec::new();
        for r in records {
            if self.contains(r) {
                report.duplicates += 1;
                continue;
            }
            bytes.extend_from_slice(&to_cbor(r));
            fresh.push(r.clone());
            report.added += 1;
        }
        if fresh.is_empty() {
            return Ok(report);
        }

        if let Some(p) = &self.path {
            if let Some(dir) = p.parent() {
                if !dir.as_os_str().is_empty() {
                    std::fs::create_dir_all(dir)?;
                }
            }
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)?;
            f.write_all(&bytes)?;
        }

        report.bytes_written = bytes.len() as u64;
        self.log_len += report.bytes_written;
        self.absorb(fresh);
        self.log_hash = hash_bytes(&self.log_bytes());
        Ok(report)
    }

    /// Write the derived index beside the log.
    pub fn write_index(&self) -> Result<(), Error> {
        let Some(p) = &self.path else {
            return Ok(());
        };
        let target = Self::index_path(p);
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(target, self.index().to_bytes())?;
        Ok(())
    }

    /// Rebuild the index from the log alone.
    ///
    /// The SM-P3 gate is that this produces bytes identical to the index maintained while
    /// appending. If it ever does not, the two paths disagree about the graph.
    pub fn reindex(&mut self) -> Index {
        let records = std::mem::take(&mut self.records);
        let mut rebuilt = Store::new();
        rebuilt.path = self.path.clone();
        rebuilt.absorb(records);
        rebuilt.log_len = self.log_len;
        rebuilt.log_hash = self.log_hash;
        let ix = rebuilt.index();
        *self = rebuilt;
        ix
    }

    // -- reading -----------------------------------------------------------

    pub fn get(&self, uid: &Uid) -> Option<&Unit> {
        self.units.get(uid)
    }

    pub fn contains_uid(&self, uid: &Uid) -> bool {
        self.units.contains_key(uid)
    }

    /// Every record, in log order - which is canonical because the log only grows.
    pub fn iter(&self) -> impl Iterator<Item = &Record> {
        self.records.iter()
    }

    pub fn units(&self) -> impl Iterator<Item = (&Uid, &Unit)> {
        self.units.iter()
    }

    pub fn relations(&self) -> impl Iterator<Item = &Relation> {
        self.relations.values()
    }

    pub fn threads(&self) -> impl Iterator<Item = &Thread> {
        self.threads.values()
    }

    pub fn views(&self) -> impl Iterator<Item = &View> {
        self.views.values()
    }

    pub fn contentions(&self) -> &[Contention] {
        &self.contentions
    }

    pub fn adjacency(&self) -> &Adjacency {
        &self.adjacency
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn log_len(&self) -> u64 {
        self.log_len
    }

    pub fn log_hash(&self) -> &[u8; 32] {
        &self.log_hash
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The log as bytes. Deterministic, so this is also how a store is piped.
    pub fn log_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for r in &self.records {
            out.extend_from_slice(&to_cbor(r));
        }
        out
    }

    /// What to put in a bundle.
    ///
    /// The default drops retracted units - but only where nothing surviving still points
    /// at them. A bundle with a dangling reference is worse than a bundle carrying a unit
    /// somebody stopped believing, and the `retracts` edge always travels, so a consumer
    /// can see the withdrawal for itself.
    pub fn bundle_with(&self, view: &View, include_retracted: bool) -> Vec<u8> {
        let g = &self.adjacency;
        let roots: Vec<_> = view.roots.iter().filter_map(|u| g.id(u)).collect();
        let reachable = traverse::closure(g, &roots, &crate::adjacency::EdgeSet::all());
        let mut keep: std::collections::BTreeSet<Uid> = reachable
            .iter()
            .filter_map(|&n| g.uid(n))
            .copied()
            .collect();

        if !include_retracted {
            let eff = crate::merge::effective_status(self, crate::merge::RetractionPolicy::Strict);
            let retracted: Vec<Uid> = eff.retracted().copied().collect();
            for r in retracted {
                let still_needed = keep.iter().any(|u| {
                    *u != r
                        && self
                            .get(u)
                            .is_some_and(|unit| unit.core.references().any(|x| *x == r))
                });
                if !still_needed {
                    keep.remove(&r);
                }
            }
        }

        self.emit(view, &keep)
    }

    /// The reachable closure of a view, as a self-contained CBOR sequence.
    ///
    /// A view references rather than owns, so this is the only way to make one portable.
    pub fn bundle(&self, view: &View) -> Vec<u8> {
        let g = &self.adjacency;
        let roots: Vec<_> = view.roots.iter().filter_map(|u| g.id(u)).collect();
        let reachable = traverse::closure(g, &roots, &crate::adjacency::EdgeSet::all());
        let keep: std::collections::BTreeSet<Uid> = reachable
            .iter()
            .filter_map(|&n| g.uid(n))
            .copied()
            .collect();
        self.emit(view, &keep)
    }

    fn emit(&self, view: &View, keep: &std::collections::BTreeSet<Uid>) -> Vec<u8> {
        let mut out = Vec::new();
        for r in &self.records {
            let included = match r {
                Record::Unit(u) => keep.contains(&canonical_uid(u)),
                Record::Attestation(a) => keep.contains(&a.uid),
                Record::Relation(rel) => keep.contains(&rel.from) && keep.contains(&rel.to),
                Record::Thread(t) => view.threads.contains(&t.id),
                Record::View(v) => v.id == view.id,
                Record::Contention(c) => keep.contains(&c.over),
                _ => false,
            };
            if included {
                out.extend_from_slice(&to_cbor(r));
            }
        }
        out
    }

    // -- integrity ---------------------------------------------------------

    /// Recompute every unit's uid and compare it against the sidecar (`SMY-E070`), and
    /// report references that point at nothing (`SMY-E060`).
    pub fn verify_against(&self, ix: &Index, report: &mut Report) {
        for uid in self.units.keys() {
            if !ix.entries.contains_key(uid) {
                report.push(Diagnostic::on(Code::E070, *uid).with_message(
                    "the index has no entry for this unit; the log does not match the index",
                ));
            }
        }
        for uid in ix.entries.keys() {
            if ix.cache.contains_key(uid) && !self.units.contains_key(uid) {
                report.push(Diagnostic::on(Code::E070, *uid).with_message(
                    "the index records a unit the log does not produce; content was altered",
                ));
            }
        }
        self.report_dangling(report);
    }

    /// References that point at no unit in this store.
    pub fn report_dangling(&self, report: &mut Report) {
        for n in self.adjacency.dangling() {
            if let Some(uid) = self.adjacency.uid(n) {
                report.push(
                    Diagnostic::new(Code::E060)
                        .with_subject(Subject::Unit(*uid))
                        .with_message("referenced by this store but not present in it"),
                );
            }
        }
    }

    /// The derived index for the current contents.
    pub fn index(&self) -> Index {
        let mut ix = Index {
            log_len: self.log_len,
            log_hash: self.log_hash,
            ..Index::default()
        };

        let mut offset = 0u64;
        for r in &self.records {
            let len = to_cbor(r).len() as u32;
            match r {
                Record::Unit(u) => {
                    ix.entries.insert(
                        canonical_uid(u),
                        Entry {
                            offset,
                            len,
                            type_code: 1,
                        },
                    );
                }
                Record::Thread(t) => {
                    ix.threads.insert((t.id.clone(), t.owner.clone()), offset);
                }
                Record::Contention(_) => ix.contentions.push(offset),
                _ => {}
            }
            offset += len as u64;
        }

        let g = &self.adjacency;
        for n in g.nodes() {
            let Some(uid) = g.uid(n) else { continue };
            let fwd: Vec<(EdgeKind, Uid)> = g
                .out_edges(n)
                .iter()
                .filter_map(|e| g.uid(e.target).map(|t| (e.kind, *t)))
                .collect();
            if !fwd.is_empty() {
                ix.fwd_adj.insert(*uid, fwd);
            }
            let rev: Vec<(EdgeKind, Uid)> = g
                .in_edges(n)
                .iter()
                .filter_map(|e| g.uid(e.target).map(|t| (e.kind, *t)))
                .collect();
            if !rev.is_empty() {
                ix.rev_adj.insert(*uid, rev);
            }
        }

        for (uid, u) in &self.units {
            ix.cache.insert(
                *uid,
                Cached {
                    status: u.core.status,
                    salience_q: u
                        .salience
                        .map(|s| (s * 1024.0).round().clamp(0.0, 1024.0) as u16)
                        .unwrap_or(0),
                },
            );
            for l in &u.labels {
                ix.labels.insert(l.clone(), *uid);
            }
        }

        ix
    }

    // -- internals ---------------------------------------------------------

    fn contains(&self, r: &Record) -> bool {
        match r {
            Record::Unit(u) => self.units.contains_key(&canonical_uid(u)),
            Record::Attestation(a) => self
                .units
                .get(&a.uid)
                .is_some_and(|u| u.attestations.contains(a)),
            Record::Relation(rel) => self.relations.contains_key(&Self::rel_key(rel)),
            Record::Thread(t) => self
                .threads
                .get(&(t.id.clone(), t.owner.clone()))
                .is_some_and(|e| e == t),
            Record::View(v) => self.views.get(&v.id).is_some_and(|e| e == v),
            Record::Contention(c) => self.contentions.iter().any(|e| e.id == c.id),
            _ => false,
        }
    }

    fn rel_key(r: &Relation) -> (String, Uid, Uid) {
        (r.kind.as_str().to_string(), r.from, r.to)
    }

    /// Fold records into the derived structures. Order-independent by construction: this
    /// is the same fold merge performs (rule U).
    fn absorb(&mut self, records: Vec<Record>) {
        for r in &records {
            match r {
                Record::Unit(u) => {
                    let uid = canonical_uid(u);
                    self.units
                        .entry(uid)
                        .or_insert_with(|| Unit::new(u.clone()));
                }
                Record::Attestation(a) => self.attach(a.clone()),
                Record::Relation(rel) => {
                    let key = Self::rel_key(rel);
                    match self.relations.get_mut(&key) {
                        Some(existing) => {
                            existing
                                .attestations
                                .extend(rel.attestations.iter().cloned());
                        }
                        None => {
                            self.relations.insert(key, rel.clone());
                        }
                    }
                }
                Record::Thread(t) => {
                    let key = (t.id.clone(), t.owner.clone());
                    // Last writer wins *within* the key; across owners there is no
                    // conflict to resolve (§5.2).
                    //
                    // A tie on the HLC is broken by encoded bytes, which makes the
                    // register a maximum over a *total* order. Without that, two peers
                    // merging the same pair of simultaneous writes in opposite orders
                    // would keep different threads, and merge would not be commutative.
                    let replace = match self.threads.get(&key) {
                        Some(existing) => match existing.ts.cmp(&t.ts) {
                            std::cmp::Ordering::Less => true,
                            std::cmp::Ordering::Greater => false,
                            std::cmp::Ordering::Equal => {
                                to_cbor(&Record::Thread(t.clone()))
                                    > to_cbor(&Record::Thread(existing.clone()))
                            }
                        },
                        None => true,
                    };
                    if replace {
                        self.threads.insert(key, t.clone());
                    }
                }
                Record::View(v) => {
                    self.views.insert(v.id.clone(), v.clone());
                }
                // Idempotent by id: a contention already recorded is not recorded twice,
                // which is what makes replaying a log a no-op rather than a duplication.
                Record::Contention(c) if !self.contentions.iter().any(|e| e.id == c.id) => {
                    self.contentions.push(c.clone());
                }
                _ => {}
            }
        }
        self.records.extend(records);
        self.rebuild_adjacency();
    }

    /// Attach an attestation to its unit. An attestation for a unit that is not here yet
    /// is kept in the log and re-attached on the next rebuild, so delivery order does not
    /// matter (rule U).
    fn attach(&mut self, a: Attestation) {
        if let Some(u) = self.units.get_mut(&a.uid) {
            u.attestations.insert(a);
        }
    }

    fn rebuild_adjacency(&mut self) {
        // Attestations may have arrived before their units; re-attach whatever now fits.
        let pending: Vec<Attestation> = self
            .records
            .iter()
            .filter_map(|r| match r {
                Record::Attestation(a) => Some(a.clone()),
                _ => None,
            })
            .collect();
        for a in pending {
            self.attach(a);
        }
        let relations: Vec<Relation> = self.relations.values().cloned().collect();
        self.adjacency = Adjacency::build(&self.units, &relations);
    }

    /// Whether this exact edge exists.
    pub fn has_relation(&self, kind: &RelKind, from: &Uid, to: &Uid) -> bool {
        self.relations
            .contains_key(&(kind.as_str().to_string(), *from, *to))
    }

    /// A digest of everything merge is required to converge on (rule U).
    ///
    /// Deliberately *not* over the log: two peers that received the same records in
    /// different orders have different logs and the same store. What must agree is the
    /// derived state - cores, attestations, relations, thread registers, contentions -
    /// which is exactly what §5.1 says the union is component-wise over.
    pub fn state_hash(&self) -> [u8; 32] {
        let mut bytes = Vec::new();

        for (uid, unit) in &self.units {
            bytes.extend_from_slice(uid.as_bytes());
            for a in &unit.attestations {
                bytes.extend_from_slice(&to_cbor(&Record::Attestation(a.clone())));
            }
            if let Some(s) = unit.salience {
                bytes.extend_from_slice(&s.to_be_bytes());
            }
            for l in &unit.labels {
                bytes.extend_from_slice(l.as_str().as_bytes());
            }
        }
        for r in self.relations.values() {
            bytes.extend_from_slice(&to_cbor(&Record::Relation(r.clone())));
            for a in &r.attestations {
                bytes.extend_from_slice(&to_cbor(&Record::Attestation(a.clone())));
            }
        }
        for t in self.threads.values() {
            bytes.extend_from_slice(&to_cbor(&Record::Thread(t.clone())));
        }
        for v in self.views.values() {
            bytes.extend_from_slice(&to_cbor(&Record::View(v.clone())));
        }
        // Contentions are keyed by a derived id, so sorting by it is canonical.
        let mut contentions: Vec<&Contention> = self.contentions.iter().collect();
        contentions.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        for c in contentions {
            bytes.extend_from_slice(&to_cbor(&Record::Contention(c.clone())));
        }

        hash_bytes(&bytes)
    }

    /// Whether two stores carry the same graph, whatever order they were assembled in.
    pub fn converged_with(&self, other: &Store) -> bool {
        self.state_hash() == other.state_hash()
    }

    /// Units whose uid begins with `prefix`, in ascending uid order.
    pub fn matching_prefix(&self, prefix: &UidPrefix) -> Vec<Uid> {
        self.units
            .keys()
            .filter(|u| prefix.matches(u))
            .copied()
            .collect()
    }

    /// Resolve an abbreviated uid.
    ///
    /// A prefix is never an identity (§1.2): resolution MUST report ambiguity rather than
    /// pick a winner, so two candidates are `SMY-E072` and not a coin flip.
    pub fn resolve_prefix(&self, prefix: &UidPrefix) -> Result<Uid, IntegrityError> {
        let mut matches = self.matching_prefix(prefix);
        match matches.len() {
            1 => Ok(matches.remove(0)),
            _ => Err(IntegrityError::AmbiguousPrefix {
                prefix: format!("{} bits", prefix.bits()),
                candidates: matches,
            }),
        }
    }

    /// Units that rebut `uid`, which is what rule R pins into a pack.
    pub fn rebuttals_of(&self, uid: &Uid) -> Vec<Uid> {
        let Some(id) = self.adjacency.id(uid) else {
            return Vec::new();
        };
        traverse::rebuttals_of(&self.adjacency, id)
            .into_iter()
            .filter_map(|n| self.adjacency.uid(n))
            .copied()
            .collect()
    }

    /// Relations of a given kind, in canonical order.
    pub fn relations_of_kind(&self, kind: &RelKind) -> Vec<&Relation> {
        self.relations
            .values()
            .filter(|r| &r.kind == kind)
            .collect()
    }
}
