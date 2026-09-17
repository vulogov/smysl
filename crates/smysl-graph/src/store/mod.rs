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

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use smysl_core::diag::{Code, Diagnostic, Report, Subject};
use smysl_core::{
    canonical_uid, from_cbor_seq, hash_bytes, to_cbor, AgentId, Attestation, Contention,
    ContentionStatus, DetectionKind, Error, IntegrityError, Record, RelKind, Relation, Resolution,
    ResolutionTarget, Status, Thread, ThreadId, Uid, UidPrefix, Unit, View, ViewId, Withdrawal,
};

use crate::adjacency::{Adjacency, EdgeKind, EdgeSet};
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
#[non_exhaustive]
pub struct OpenReport {
    pub report: Report,
    /// True when the sidecar was missing, stale, or unreadable.
    pub index_rebuilt: bool,
    /// Bytes after the last complete record, left by a writer mid-append.
    pub trailing_bytes: u64,
}

/// What happened during `append`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
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
    /// Withdrawals by the rid they name, whether or not that relation has arrived (1.4).
    withdrawals: BTreeMap<Uid, BTreeSet<Withdrawal>>,
    resolutions: BTreeSet<Resolution>,
    /// Each relation's rid, to its key in `relations`.
    rids: BTreeMap<Uid, (String, Uid, Uid)>,
    /// BLAKE3 of the canonical encoding of every record the log holds: what `contains` answers
    /// from. One 32-byte hash a record, and structural — a record type added later is recognised
    /// as present without anybody remembering to teach `contains` about it (R10).
    record_hashes: BTreeSet<[u8; 32]>,
    /// Units whose effective status under the strict policy is `unfounded`: what liveness
    /// reads (1.4, rule R). Derived on every rebuild, like the adjacency.
    unfounded: BTreeSet<Uid>,
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
            withdrawals: BTreeMap::new(),
            resolutions: BTreeSet::new(),
            rids: BTreeMap::new(),
            record_hashes: BTreeSet::new(),
            unfounded: BTreeSet::new(),
            adjacency: Adjacency::default(),
        }
    }

    /// Build in memory from records, with no file behind it.
    ///
    /// A record given twice is held once, as `append` would hold it. (`open` keeps a log exactly as
    /// it is on disk, duplicates included: the file is the authority, and one written before R10
    /// can hold them.)
    pub fn from_records(records: Vec<Record>) -> Store {
        let mut s = Store::new();
        let mut seen = BTreeSet::new();
        let mut repeated = Vec::new();
        let records: Vec<Record> = records
            .into_iter()
            .filter_map(|r| {
                if seen.insert(Self::record_hash(&r)) {
                    Some(r)
                } else {
                    repeated.push(r);
                    None
                }
            })
            .collect();
        s.absorb(records);
        s.union_edge_attestations(&repeated);
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
        // Also against the batch itself, so the same record twice in one delivery is one record.
        let mut seen: BTreeSet<[u8; 32]> = BTreeSet::new();
        let mut repeated = Vec::new();
        for r in records {
            let encoded = to_cbor(r);
            let hash = hash_bytes(&encoded);
            if self.record_hashes.contains(&hash) || !seen.insert(hash) {
                report.duplicates += 1;
                repeated.push(r.clone());
                continue;
            }
            bytes.extend_from_slice(&encoded);
            fresh.push(r.clone());
            report.added += 1;
        }
        if fresh.is_empty() {
            self.union_edge_attestations(&repeated);
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
        self.union_edge_attestations(&repeated);
        self.log_hash = hash_bytes(&self.log_bytes());
        Ok(report)
    }

    /// A relation held in memory can carry attestations its encoding does not (they travel as
    /// attestation records on the wire), so two relation records can be one record by bytes and
    /// still bring different attestations. Nothing is appended for the repeat; its attestations
    /// join the edge's, as they did when the repeat was appended.
    fn union_edge_attestations(&mut self, repeated: &[Record]) {
        for r in repeated {
            if let Record::Relation(rel) = r {
                if let Some(existing) = self.relations.get_mut(&Self::rel_key(rel)) {
                    existing
                        .attestations
                        .extend(rel.attestations.iter().cloned());
                }
            }
        }
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
                // A bundle is the artifact designed to travel alone - closure exists so it
                // can be handed to a recipient with nothing else to read it against. Without
                // the bindings it arrived with every reference spelled as a bare uid: valid,
                // re-checking clean, and unreadable, in the one case where the reader has no
                // other copy. The catch-all below excluded them silently when the record
                // type was added.
                Record::LabelBinding(b) => keep.contains(&b.uid),
                // A withdrawal travels with its edge, or the recipient would follow an edge the
                // sender does not.
                Record::Withdrawal(w) => self
                    .relation_by_id(&w.relation)
                    .is_some_and(|rel| keep.contains(&rel.from) && keep.contains(&rel.to)),
                Record::Resolution(res) => match &res.target {
                    ResolutionTarget::Relation(rid) => self
                        .relation_by_id(rid)
                        .is_some_and(|rel| keep.contains(&rel.from) && keep.contains(&rel.to)),
                    ResolutionTarget::Contention(id) => self
                        .contentions
                        .iter()
                        .any(|c| &c.id == id && keep.contains(&c.over)),
                    _ => false,
                },
                // Pack manifests and schema declarations are about a whole store rather than
                // any unit in it, so there is no `keep` question to ask; an unknown record
                // cannot be judged at all.
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

    /// What makes two records the same record: the BLAKE3 of the canonical encoding.
    ///
    /// Byte identity, for every record type. Until R10 `append` asked a `contains` that matched
    /// nine types by their own notion of identity and answered "absent" for the rest, so every
    /// merge re-appended label bindings, schema declarations, pack info, unknown records and
    /// attestations on edges: `merge(A, A)` grew a real 157-record batch by 42 each time. Bytes are
    /// also what keeps the record set the same whichever order stores are merged in — a relation
    /// differing only in weight is a different record, and keying it by its endpoints kept
    /// whichever variant arrived first.
    fn record_hash(r: &Record) -> [u8; 32] {
        hash_bytes(&to_cbor(r))
    }

    fn rel_key(r: &Relation) -> (String, Uid, Uid) {
        (r.kind.as_str().to_string(), r.from, r.to)
    }

    /// Fold records into the derived structures. Order-independent by construction: this
    /// is the same fold merge performs (rule U).
    fn absorb(&mut self, records: Vec<Record>) {
        for r in &records {
            self.record_hashes.insert(Self::record_hash(r));
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
                    self.rids.insert(rel.uid(), key.clone());
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
                Record::Withdrawal(w) => {
                    self.withdrawals
                        .entry(w.relation)
                        .or_default()
                        .insert(w.clone());
                }
                Record::Resolution(r) => {
                    self.resolutions.insert(r.clone());
                }
                _ => {}
            }
        }
        self.records.extend(records);
        self.rebuild_adjacency();
    }

    /// Attach an attestation to its unit, or to the relation its uid is the rid of (1.4). An
    /// attestation for something that is not here yet is kept in the log and re-attached on the
    /// next rebuild, so delivery order does not matter (rule U).
    fn attach(&mut self, a: Attestation) {
        if let Some(u) = self.units.get_mut(&a.uid) {
            u.attestations.insert(a);
        } else if let Some(key) = self.rids.get(&a.uid) {
            if let Some(rel) = self.relations.get_mut(key) {
                rel.attestations.insert(a);
            }
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
        // A withdrawn edge is kept and not followed: the adjacency every traversal reads is
        // built without it.
        let relations: Vec<Relation> = self
            .relations
            .values()
            .filter(|r| !self.is_withdrawn(r))
            .cloned()
            .collect();
        self.adjacency = Adjacency::build(&self.units, &relations);

        self.unfounded.clear();
        if self
            .relations
            .keys()
            .any(|(k, _, _)| k == RelKind::Retracts.as_str())
        {
            let eff = crate::merge::effective_status(self, crate::merge::RetractionPolicy::Strict);
            self.unfounded = self
                .units
                .keys()
                .filter(|u| eff.get(u) == Some(Status::Unfounded))
                .copied()
                .collect();
        }
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
        for set in self.withdrawals.values() {
            for w in set {
                bytes.extend_from_slice(&to_cbor(&Record::Withdrawal(w.clone())));
            }
        }
        for r in &self.resolutions {
            bytes.extend_from_slice(&to_cbor(&Record::Resolution(r.clone())));
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

    // -- episodes ----------------------------------------------------------
    //
    // A hop is one handoff in a pipeline: the step that produced a unit. `Attestation.hop`
    // has always carried it and nothing ever read it, so a store could not answer the
    // question a long-running agent most needs to ask - *what did the last step add?* These
    // three methods are that question, and the recency term in `salience` is built on them.

    /// The hop a unit was produced at: the newest one any of its attestations records.
    ///
    /// Newest rather than oldest, because a unit re-attested at a later hop was carried
    /// forward deliberately, and carrying something forward is a statement that it still
    /// matters. A unit with no attestation has no episode and returns `None`.
    pub fn hop_of(&self, uid: &Uid) -> Option<u32> {
        self.get(uid)?.attestations.iter().map(|a| a.hop).max()
    }

    /// Every hop present in the store, in order.
    pub fn hops(&self) -> BTreeSet<u32> {
        self.units().filter_map(|(u, _)| self.hop_of(u)).collect()
    }

    /// The most recent hop anything was produced at.
    pub fn latest_hop(&self) -> Option<u32> {
        self.hops().last().copied()
    }

    /// The units a given hop produced - what one step of a pipeline contributed.
    pub fn at_hop(&self, hop: u32) -> impl Iterator<Item = (&Uid, &Unit)> {
        self.units()
            .filter(move |(u, _)| self.hop_of(u) == Some(hop))
    }

    /// Units whose rebuttal of `uid` is live, which is what rule R pins into a pack.
    ///
    /// Live since 1.4 (the specification's §6): the edge is not withdrawn, and the rebutting
    /// unit is present and not `unfounded` under the strict retraction policy. Until then every
    /// `rebuts` edge counted, so a retracted rebuttal went on pinning its claim into every pack.
    pub fn rebuttals_of(&self, uid: &Uid) -> Vec<Uid> {
        let Some(id) = self.adjacency.id(uid) else {
            return Vec::new();
        };
        traverse::rebuttals_of(&self.adjacency, id)
            .into_iter()
            .filter_map(|n| self.adjacency.uid(n))
            .filter(|a| self.contains_uid(a) && !self.unfounded.contains(a))
            .copied()
            .collect()
    }

    /// Units whose source reference starts with `prefix`, in canonical order (1.5).
    ///
    /// A producer that anchors a unit to a file at a commit — `src/main.rs@90ec2f781421` — asks
    /// this for every file a diff touches, and was iterating the whole store per diff to do it.
    /// Prefix rather than equality because the anchor carries the commit: the question is "units
    /// about this file", whichever revision they were recorded at.
    pub fn units_with_source_prefix(&self, prefix: &str) -> Vec<Uid> {
        self.units
            .iter()
            .filter(|(_, u)| {
                u.core
                    .source
                    .as_ref()
                    .is_some_and(|s| s.reference.starts_with(prefix))
            })
            .map(|(uid, _)| *uid)
            .collect()
    }

    /// Every attestation naming this uid, whether it is a unit or an edge (1.5).
    ///
    /// An attestation names a unit or a relation, by its rid (spec §2.4), and a caller asking
    /// "who stands behind this" should not have to know which it is holding.
    pub fn attestations_of(&self, uid: &Uid) -> &BTreeSet<Attestation> {
        static NONE: std::sync::OnceLock<BTreeSet<Attestation>> = std::sync::OnceLock::new();
        if let Some(u) = self.units.get(uid) {
            return &u.attestations;
        }
        if let Some(r) = self.relation_by_id(uid) {
            return &r.attestations;
        }
        NONE.get_or_init(BTreeSet::new)
    }

    /// The distinct agents that attested it, unit or edge (1.5).
    ///
    /// What a policy counts when it asks for two independent runs to agree before a status is
    /// raised: the same agent attesting twice is one agent, and a store holding one run's
    /// attestation twice says nothing more than it did once.
    pub fn attested_by(&self, uid: &Uid) -> BTreeSet<&AgentId> {
        self.attestations_of(uid).iter().map(|a| &a.agent).collect()
    }

    /// Whether at least `n` distinct agents attested it (1.5).
    pub fn agreement(&self, uid: &Uid, n: usize) -> bool {
        self.attested_by(uid).len() >= n
    }

    /// What this unit rests on, one step, over the edges the caller chooses — `deps` and
    /// `grounds` along their direction, relations against theirs (1.5). `crate::rests_on` is the
    /// transitive form and explains the asymmetry.
    pub fn supports_of(&self, uid: &Uid, edges: &EdgeSet) -> Vec<Uid> {
        let Some(id) = self.adjacency.id(uid) else {
            return Vec::new();
        };
        crate::lineage::one_hop(&self.adjacency, id, edges)
            .into_iter()
            .filter_map(|n| self.adjacency.uid(n))
            .filter(|u| self.contains_uid(u))
            .copied()
            .collect()
    }

    /// Whether this relation is withdrawn: some withdrawal names its rid, and it is not a
    /// lifecycle edge (`retracts`, `supersedes`), which cannot be withdrawn in 1.4 (`SMY-W056`).
    pub fn is_withdrawn(&self, rel: &Relation) -> bool {
        !rel.kind.is_lifecycle() && self.withdrawals.contains_key(&rel.uid())
    }

    /// Whether `rel` is a live rebuttal: a `rebuts` edge, not withdrawn, from a unit that is
    /// present and not `unfounded` under the strict policy.
    pub fn is_live_rebuttal(&self, rel: &Relation) -> bool {
        rel.kind == RelKind::Rebuts
            && !self.is_withdrawn(rel)
            && self.contains_uid(&rel.from)
            && !self.unfounded.contains(&rel.from)
    }

    /// Whether this unit's effective status under the strict policy is `unfounded`.
    pub fn is_unfounded(&self, uid: &Uid) -> bool {
        self.unfounded.contains(uid)
    }

    /// The relation whose rid this is.
    pub fn relation_by_id(&self, rid: &Uid) -> Option<&Relation> {
        self.rids.get(rid).and_then(|k| self.relations.get(k))
    }

    /// Every withdrawal, grouped by the rid it names.
    pub fn withdrawals(&self) -> impl Iterator<Item = &Withdrawal> {
        self.withdrawals.values().flatten()
    }

    pub fn resolutions(&self) -> impl Iterator<Item = &Resolution> {
        self.resolutions.iter()
    }

    /// Whether a resolution names this target.
    pub fn is_resolved(&self, target: &ResolutionTarget) -> bool {
        self.resolutions.iter().any(|r| &r.target == target)
    }

    /// What a contention's status reads as in this store (1.4).
    ///
    /// `resolved` when a resolution names it, whatever its record says. `stale` when it is a
    /// live-rebuttal contention whose rebuttal is no longer live or whose claim is `unfounded`.
    /// Otherwise the status it was recorded with. Only `open` pins positions into a pack.
    pub fn contention_status(&self, c: &Contention) -> ContentionStatus {
        if self.is_resolved(&ResolutionTarget::Contention(c.id.clone())) {
            return ContentionStatus::Resolved;
        }
        if c.detected.kind == DetectionKind::LiveRebuttal {
            let live = c.positions.iter().any(|a| {
                self.relations
                    .get(&(RelKind::Rebuts.as_str().to_string(), *a, c.over))
                    .is_some_and(|r| self.is_live_rebuttal(r))
            });
            if !live || self.unfounded.contains(&c.over) {
                return ContentionStatus::Stale;
            }
        }
        c.status
    }

    /// Recorded contentions that read as open, and so pin their positions (constraint C4).
    pub fn open_contentions(&self) -> impl Iterator<Item = &Contention> {
        self.contentions
            .iter()
            .filter(|c| self.contention_status(c) == ContentionStatus::Open)
    }

    /// Relations of a given kind, in canonical order, leaving out withdrawn ones (1.4).
    pub fn relations_of_kind(&self, kind: &RelKind) -> Vec<&Relation> {
        self.relations
            .values()
            .filter(|r| &r.kind == kind && !self.is_withdrawn(r))
            .collect()
    }
}
