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
mod library;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use smysl_core::diag::{Code, Diagnostic, Report, Subject};
use smysl_core::types::record;
use smysl_core::{
    canonical_uid, from_cbor_seq, hash_bytes, to_cbor, AgentId, Attestation, Commit, Commitment,
    Contention, ContentionStatus, DetectionKind, Error, IntegrityError, Record, RelKind, Relation,
    Resolution, ResolutionTarget, Rolling, Status, Thread, ThreadId, Uid, UidPrefix, Unit, View,
    ViewId, Withdrawal,
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
    /// Attestations that named a unit or edge the store did not hold yet (1.8).
    ///
    /// They arrive out of order — an attestation can reach a peer before the unit it vouches
    /// for — so every absorb used to rescan the whole log and retry *every* attestation it
    /// held, cloning each one. Keeping the ones that did not land costs the same correctness
    /// for work proportional to what is still waiting.
    pending_attestations: Vec<Attestation>,
    /// The log's hash, kept open so an append costs the bytes appended rather than the store.
    ///
    /// `log_hash` is the digest of every record's encoding concatenated, and it was recomputed
    /// from scratch on every `append` — re-encoding the whole store to hash it again. One record
    /// appended to a thirty-thousand-record store took 130 ms, and the cost grew with the store.
    /// The digest is unchanged: BLAKE3 over a stream is BLAKE3 over the concatenation.
    log_hasher: Rolling,

    units: BTreeMap<Uid, Unit>,
    relations: BTreeMap<(String, Uid, Uid), Relation>,
    threads: BTreeMap<(ThreadId, AgentId), Thread>,
    views: BTreeMap<ViewId, View>,
    contentions: Vec<Contention>,
    /// Withdrawals by the rid they name, whether or not that relation has arrived (1.4).
    withdrawals: BTreeMap<Uid, BTreeSet<Withdrawal>>,
    resolutions: BTreeSet<Resolution>,
    /// Commitments by the unit they name, whether or not that unit has arrived (1.7).
    commits: BTreeMap<Uid, BTreeSet<Commit>>,
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
    /// How many times the adjacency has been rebuilt.
    ///
    /// Here so that "a manifest-only append does not rebuild the adjacency" is a property a
    /// test can assert rather than a timing a benchmark can suggest. Comparing the adjacency
    /// before and after cannot do it: a rebuild that changes nothing leaves it equal, so the
    /// assertion passes whether or not the work was done. Private, and read only by the test
    /// in this module — a counter in the public surface would be a promise about how often
    /// this crate rebuilds, which is not a promise worth making.
    adjacency_rebuilds: u64,
    /// Manifests, heads and the by-part indexes (RFC SMYSL-2.4 §4.3.2).
    library: library::Library,
}

impl Default for Store {
    fn default() -> Store {
        Store::new()
    }
}

/// What a bundle carried.
///
/// Returned beside the bytes rather than logged, because the caller is the only one who can act
/// on it: a bundle is outbound, and by the time anyone else sees these records the decision to
/// send them has been made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct BundleReport {
    /// Units in the bundle.
    pub units: usize,
    /// How many of those were not reachable by edges, and arrived because a commitment's note,
    /// a withdrawal's reason, a resolution's note or a thread's position named them.
    pub pulled_in_by_reference: usize,
    /// Records of every kind.
    pub records: usize,
    /// Records of a type this build cannot interpret, kept per rule X and counted here so the
    /// sender knows they are forwarding something they could not read.
    ///
    /// Counted whether or not they were kept: under [`UnknownRecords::Drop`] this is what the
    /// bundle *left out*, which the sender needs to know for the same reason. `SMY-W434` says
    /// which of the two happened.
    pub unknown_records: usize,
}

/// Whether a bundle carries records of a type this build cannot interpret (F-16).
///
/// Rule X says keep them: a build that drops what it cannot name silently truncates a peer's
/// store on the way through. But keeping them means forwarding content that cannot be inspected
/// and so cannot be evaluated for redaction either — a 1.9 peer can pass on a future record 15
/// (part text) that a newer peer would have filtered under rule Z. So the default keeps, and
/// this is the escape for a sender who must not forward what they cannot read. SMYSL-2.4
/// replaces the blanket rule with per-record rows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnknownRecords {
    /// Keep and count them (rule X, the default).
    #[default]
    Keep,
    /// Leave them out, and still count them.
    Drop,
}

impl Store {
    /// An empty in-memory store.
    pub fn new() -> Store {
        Store {
            path: None,
            records: Vec::new(),
            log_len: 0,
            log_hash: hash_bytes(&[]),
            log_hasher: Rolling::new(),
            pending_attestations: Vec::new(),
            units: BTreeMap::new(),
            relations: BTreeMap::new(),
            threads: BTreeMap::new(),
            views: BTreeMap::new(),
            contentions: Vec::new(),
            withdrawals: BTreeMap::new(),
            resolutions: BTreeSet::new(),
            commits: BTreeMap::new(),
            rids: BTreeMap::new(),
            record_hashes: BTreeSet::new(),
            unfounded: BTreeSet::new(),
            adjacency_rebuilds: 0,
            library: library::Library::default(),
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
        // Once, over the bytes as they are: `log_bytes` was being built twice here, and the
        // second one only to hash it.
        let bytes = s.log_bytes();
        s.log_len = bytes.len() as u64;
        s.log_hasher.update(&bytes);
        s.log_hash = s.log_hasher.finish();
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
        store.log_hasher = Rolling::new();
        store.log_hasher.update(&bytes[..consumed]);

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
    /// The log itself is `O(new)`: the records are encoded, written to the end of the file, and
    /// the running hash advances over the new bytes only (1.8 — it used to re-encode and rehash
    /// every record in the store, which made an append cost the store).
    ///
    /// **The derived state is rebuilt once per call, not once per record**, and that is the cost
    /// model a caller has to plan around. The adjacency is built from every unit and relation the
    /// store holds, so one call costs `O(store)` however many records it carries. Appending
    /// twenty thousand records to a growing store, measured:
    ///
    /// | records per call | per record |
    /// |---:|---:|
    /// | 1 | 1336 µs |
    /// | 10 | 127 µs |
    /// | 50 | 25 µs |
    /// | 200 | 8 µs |
    /// | 1000 | 5 µs |
    ///
    /// So a producer with a stream of single records — a detector emitting one reading at a time
    /// — should buffer. Fifty is already within a factor of five of the floor.
    ///
    /// **Text does not pay for it** (1.10, TX-P1 step 4). A manifest, a part text and a part
    /// reading name no unit as an endpoint, so they cannot appear in the adjacency or in
    /// `unfounded`; a batch of nothing but those is `O(batch)` whatever the store holds.
    /// Everything else rebuilds, which is the exemption stated as what it skips rather than as
    /// what it catches — a record type added later and forgotten then costs a rebuild it did
    /// not need, instead of leaving a traversal unable to see an edge. Re-measured on the same
    /// harness as the table above (`tests/append_timing.rs`, `--release`), one record per call:
    ///
    /// | store size | a manifest | a unit |
    /// |---:|---:|---:|
    /// | 1,000 | 7.9 µs | 38.0 µs |
    /// | 5,000 | 5.4 µs | 268.6 µs |
    /// | 20,000 | 4.0 µs | ~1123 µs |
    ///
    /// The manifest column does not grow; the unit column is the cost of the store. That is
    /// what makes `text add` affordable one text at a time, and the behaviour is pinned by a
    /// rebuild counter in a test rather than by these figures, which depend on the machine.
    ///
    /// Rebuilding lazily instead, on the first read after an append, was considered and not
    /// done: `adjacency` is `&self`, so deferring the work needs interior mutability, and that
    /// would make `Store` no longer `Sync` — which a pipeline holding one behind an `Arc` would
    /// notice far more than it notices this.
    pub fn append(&mut self, records: &[Record]) -> Result<AppendReport, Error> {
        // Before the duplicate check, before a byte is written: a log does not hold text
        // (OQ-39, `SMY-E452`). A part text or a reading in a log would one day have to be
        // erased to honour a redaction, and rewriting an append-only log resets the very hash
        // chain that would have shown the rewrite. The refusal is the first thing that happens
        // to one, rather than a repair afterwards — and the whole batch is refused, so a
        // caller cannot half-append a delivery and be told about it later.
        for r in records {
            let code = match r {
                Record::PartText(_) => Some(record::code::PART_TEXT as u8),
                Record::PartReading(_) => Some(record::code::PART_READING as u8),
                _ => None,
            };
            if let Some(code) = code {
                return Err(Error::Lib(smysl_core::error::LibError::TextInLog {
                    record: code,
                }));
            }
        }
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
        // The appended bytes only. `absorb` pushes onto `self.records` in order, so the stream
        // the hasher has seen is the log in log order — which is what `log_bytes` concatenates.
        self.log_hasher.update(&bytes);
        self.log_hash = self.log_hasher.finish();
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
        rebuilt.log_hasher = self.log_hasher.clone();
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

    /// Units that carry an observation instant, oldest first (1.8).
    ///
    /// The answer to *what happened, in what order* — which for telemetry is the question, and
    /// which `captured` could not answer at all: it is a date, so everything on one Thursday is
    /// simultaneous. Ties break by uid, so the order is total and two runs agree (rule D).
    ///
    /// Units with no instant are **left out** rather than sorted to one end. A unit with no
    /// observation time is not early or late; it is not on the timeline. A caller that wants
    /// them can iterate `units()` for the rest.
    pub fn units_in_observed_order(&self) -> Vec<Uid> {
        let mut timed: Vec<(u64, Uid)> = self
            .units
            .iter()
            .filter_map(|(uid, u)| {
                u.core
                    .source
                    .as_ref()
                    .and_then(|s| s.observed)
                    .map(|ms| (ms, *uid))
            })
            .collect();
        timed.sort();
        timed.into_iter().map(|(_, uid)| uid).collect()
    }

    /// When this unit was observed, if it says (1.8).
    pub fn observed_at(&self, uid: &Uid) -> Option<u64> {
        self.units
            .get(uid)
            .and_then(|u| u.core.source.as_ref())
            .and_then(|s| s.observed)
    }

    /// Every commitment naming this unit, in record order (1.7).
    pub fn commits_of(&self, uid: &Uid) -> &BTreeSet<Commit> {
        static EMPTY: std::sync::OnceLock<BTreeSet<Commit>> = std::sync::OnceLock::new();
        self.commits
            .get(uid)
            .unwrap_or_else(|| EMPTY.get_or_init(BTreeSet::new))
    }

    /// How settled this unit is: the level of its latest commitment (1.7).
    ///
    /// Latest by `(ts.wall_ms, ts.counter, agent)`, which is a total order — so the answer does
    /// not depend on the order records arrived in, which rule U requires of anything derived from
    /// a store. `None` means nobody has committed to it, which is not the same as `Floated`:
    /// floated is a decision, silence is not.
    ///
    /// Taking the *highest level* anyone ever asserted was the obvious alternative and is wrong:
    /// it makes a commitment impossible to walk back, so `Retconned` could never take effect, and
    /// "the most committed anyone ever was" is not what a ledger means.
    pub fn commitment_of(&self, uid: &Uid) -> Option<Commitment> {
        self.commits
            .get(uid)?
            .iter()
            .max_by(|a, b| {
                (a.ts.wall_ms, a.ts.counter, a.agent.as_str()).cmp(&(
                    b.ts.wall_ms,
                    b.ts.counter,
                    b.agent.as_str(),
                ))
            })
            .map(|c| c.level)
    }

    /// Every unit committed at `level`, in canonical order (1.7).
    pub fn units_at_commitment(&self, level: Commitment) -> Vec<Uid> {
        self.commits
            .keys()
            .filter(|u| self.commitment_of(u) == Some(level))
            .copied()
            .collect()
    }

    /// How many records this log holds more than once (1.7).
    ///
    /// Zero for anything this build wrote: since 1.4's R10 fix `append` refuses a record whose
    /// canonical encoding it already holds. A log written before that grew by its label bindings,
    /// schema declarations and edge attestations on every self-merge, and `open` keeps such a log
    /// exactly as it is on disk — so the only way to learn it was carrying repeats was to run
    /// `compact` and read the number. `check` says it now (`SMY-W111`).
    pub fn duplicate_records(&self) -> usize {
        self.records.len() - self.record_hashes.len()
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
        self.bundle_with_options(view, include_retracted, UnknownRecords::Keep)
            .0
    }

    /// A bundle, with both choices a sender has, and what went into it.
    ///
    /// `bundle_with` answers the retracted question and returns no report; `bundle_with_report`
    /// answers the unknown-record question and does not filter retracted units. The CLI needs
    /// all three at once, and a sender deciding whether to forward records they cannot read
    /// needs the count to decide on.
    pub fn bundle_with_options(
        &self,
        view: &View,
        include_retracted: bool,
        unknown: UnknownRecords,
    ) -> (Vec<u8>, BundleReport) {
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

        let reachable_only = keep.len();
        self.close_keep_set(view, &mut keep);

        let bytes = self.emit_with(view, &keep, unknown);
        // Counted from what `Keep` would have emitted: under `Drop` the sender still has to be
        // told what was left out, which is the whole reason the count exists.
        let all = match unknown {
            UnknownRecords::Keep => bytes.clone(),
            UnknownRecords::Drop => self.emit_with(view, &keep, UnknownRecords::Keep),
        };
        let (records, _) = smysl_core::from_cbor_seq(&bytes).unwrap_or_default();
        let (all_records, _) = smysl_core::from_cbor_seq(&all).unwrap_or_default();
        let report = BundleReport {
            units: keep.len(),
            pulled_in_by_reference: keep.len().saturating_sub(reachable_only),
            records: records.len(),
            unknown_records: all_records
                .iter()
                .filter(|r| matches!(r, Record::Unknown { .. }))
                .count(),
        };
        (bytes, report)
    }

    /// The reachable closure of a view, as a self-contained CBOR sequence.
    ///
    /// A view references rather than owns, so this is the only way to make one portable.
    pub fn bundle(&self, view: &View) -> Vec<u8> {
        self.bundle_with_report(view).0
    }

    /// A bundle, and what went into it.
    ///
    /// The count of records this build cannot interpret is the one a sender needs: a bundle
    /// keeps them, per rule X, which means forwarding content that cannot be inspected — and
    /// under rule Z, cannot be evaluated for redaction either. Keeping them is right; keeping
    /// them *silently* is how a sender comes to believe they have read what they sent.
    pub fn bundle_with_report(&self, view: &View) -> (Vec<u8>, BundleReport) {
        self.bundle_report_with(view, UnknownRecords::Keep)
    }

    /// As [`Store::bundle_with_report`], choosing what happens to records this build cannot
    /// interpret.
    ///
    /// The count in the report is of unknown records in the closure, not of ones emitted, so
    /// `Drop` still reports what it left behind.
    pub fn bundle_report_with(
        &self,
        view: &View,
        unknown: UnknownRecords,
    ) -> (Vec<u8>, BundleReport) {
        let g = &self.adjacency;
        let roots: Vec<_> = view.roots.iter().filter_map(|u| g.id(u)).collect();
        let reachable = traverse::closure(g, &roots, &crate::adjacency::EdgeSet::all());
        let mut keep: std::collections::BTreeSet<Uid> = reachable
            .iter()
            .filter_map(|&n| g.uid(n))
            .copied()
            .collect();
        let reachable_only = keep.len();
        self.close_keep_set(view, &mut keep);

        let bytes = self.emit_with(view, &keep, unknown);
        let (records, _) = smysl_core::from_cbor_seq(&bytes).unwrap_or_default();
        // Counted from what `Keep` would have emitted, not from `bytes`: under `Drop` the
        // sender still has to be told what was left out, which is the whole reason the count
        // exists.
        let kept_all = match unknown {
            UnknownRecords::Keep => bytes.clone(),
            UnknownRecords::Drop => self.emit_with(view, &keep, UnknownRecords::Keep),
        };
        let (all_records, _) = smysl_core::from_cbor_seq(&kept_all).unwrap_or_default();
        let report = BundleReport {
            units: keep.len(),
            pulled_in_by_reference: keep.len() - reachable_only,
            records: records.len(),
            unknown_records: all_records
                .iter()
                .filter(|r| matches!(r, Record::Unknown { .. }))
                .count(),
        };
        (bytes, report)
    }

    /// Grow a keep-set until it is closed under the references records make outside the graph.
    ///
    /// `traverse::closure` follows *edges*. A commitment's `note`, a resolution's `note`, a
    /// withdrawal's `reason` and a thread's step all name a unit without being one, so a single
    /// pass can emit a record pointing at a unit the bundle does not carry — a dangling
    /// reference in the one artifact whose whole purpose is to be readable alone. And the
    /// references compose: pulling in a unit can pull in its own closure, which can carry a
    /// commitment naming another unit again. So this runs to a fixpoint rather than once.
    ///
    /// It terminates because `keep` only grows and is bounded by the store.
    fn close_keep_set(&self, view: &View, keep: &mut std::collections::BTreeSet<Uid>) {
        let g = &self.adjacency;
        loop {
            let mut added: Vec<Uid> = Vec::new();
            let want = |u: Option<Uid>, acc: &mut Vec<Uid>| {
                if let Some(u) = u {
                    if !keep.contains(&u) {
                        acc.push(u);
                    }
                }
            };

            for r in &self.records {
                match r {
                    Record::Commit(c) if keep.contains(&c.unit) => want(c.note, &mut added),
                    Record::Withdrawal(w) => {
                        if self
                            .relation_by_id(&w.relation)
                            .is_some_and(|rel| keep.contains(&rel.from) && keep.contains(&rel.to))
                        {
                            want(w.reason, &mut added);
                        }
                    }
                    Record::Resolution(res) => {
                        let on_a_kept_thing = match &res.target {
                            ResolutionTarget::Relation(rid) => {
                                self.relation_by_id(rid).is_some_and(|rel| {
                                    keep.contains(&rel.from) && keep.contains(&rel.to)
                                })
                            }
                            ResolutionTarget::Contention(id) => self
                                .contentions
                                .iter()
                                .any(|c| &c.id == id && keep.contains(&c.over)),
                            _ => false,
                        };
                        if on_a_kept_thing {
                            want(res.note, &mut added);
                        }
                    }
                    // A thread in the view carries its positions, and a position names a unit.
                    Record::Thread(th) if view.threads.contains(&th.id) => {
                        for s in &th.steps {
                            want(Some(s.unit), &mut added);
                        }
                    }
                    _ => {}
                }
            }

            if added.is_empty() {
                return;
            }
            // Each newly wanted unit arrives with its own closure, or the bundle would carry a
            // unit whose grounds are absent — which is the same failure one level down.
            for u in added {
                keep.insert(u);
                if let Some(n) = g.id(&u) {
                    for m in traverse::closure(g, &[n], &crate::adjacency::EdgeSet::all()) {
                        if let Some(x) = g.uid(m) {
                            keep.insert(*x);
                        }
                    }
                }
            }
        }
    }

    fn emit_with(
        &self,
        view: &View,
        keep: &std::collections::BTreeSet<Uid>,
        unknown: UnknownRecords,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        for r in &self.records {
            let included = match r {
                Record::Unit(u) => keep.contains(&canonical_uid(u)),
                // An attestation names a unit *or* a relation: `attach` resolves it against
                // `rids` when it is not a unit. Testing only `keep` dropped every edge
                // attestation from a bundle, so a recipient saw the edge and not who vouched
                // for it.
                Record::Attestation(a) => {
                    keep.contains(&a.uid)
                        || self
                            .relation_by_id(&a.uid)
                            .is_some_and(|rel| keep.contains(&rel.from) && keep.contains(&rel.to))
                }
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
                // A commitment travels with the unit it settles. 1.7 added the axis and this
                // arm was never written, so every bundle silently dropped how settled anything
                // was — the one thing a ledger exists to carry.
                Record::Commit(c) => keep.contains(&c.unit),
                // A declaration travels when the bundle uses the schema it declares. Without
                // it the recipient holds payloads it cannot interpret, which is rule X failing
                // in the artifact designed to travel alone. Every revision, because a unit
                // written against an earlier one is read against that one.
                Record::SchemaDecl(d) => self.records.iter().any(|r| match r {
                    Record::Unit(u) => keep.contains(&canonical_uid(u)) && u.schema == d.id,
                    _ => false,
                }),
                // Rule X, in the place it is hardest: a record type this build cannot name is
                // kept rather than dropped. The sender is told by `SMY-W434`, because
                // forwarding what you cannot inspect is a decision and should be a visible one
                // — and `--unknown drop` is how a sender who must not forward it says so.
                Record::Unknown { .. } => unknown == UnknownRecords::Keep,
                // An expression manifest travels with the units that came out of its text.
                //
                // SMYSL-2.4 §4.3.2 says record 14 travels *always*. It is narrowed here, and
                // the narrowing is the point: a bundle is outbound, and a manifest names an
                // expression the sender holds — so "always" would put the sender's whole
                // library inventory into every bundle, including texts the bundle has not one
                // unit from. That is a disclosure nobody asked for. The recipient needs the
                // manifest of a text a kept unit *came from* (its licence, its carry rule, the
                // length a span is checked against), and the by-part index is what makes that
                // question answerable — it did not exist when "always" was written.
                //
                // Ancestors do not travel: a superseded manifest is not needed to read a span
                // against the one that superseded it. A recipient reconstructing a chain asks
                // for it.
                Record::Manifest(m) => m.parts.iter().any(|part| {
                    self.library
                        .by_tid
                        .get(&part.tid)
                        .is_some_and(|units| units.iter().any(|u| keep.contains(u)))
                }),
                // A pack manifest is about a whole store rather than any unit in it, so there
                // is no `keep` question to ask.
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
                Record::Attestation(a) => {
                    // Keep it if its subject has not arrived; `rebuild_adjacency` retries.
                    if !self.attach(a.clone()) {
                        self.pending_attestations.push(a.clone());
                    }
                }
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
                Record::Commit(c) => {
                    self.commits.entry(c.unit).or_default().insert(c.clone());
                }
                Record::Manifest(m) => self.library.absorb_manifest(m),
                _ => {}
            }
        }
        // A unit's source is indexed after the units are in, so the pass order does not
        // matter; `absorb_unit` is idempotent on a set.
        for r in &records {
            if let Record::Unit(u) = r {
                self.library
                    .absorb_unit(canonical_uid(u), u.source.as_ref());
            }
        }
        // Rebuild the adjacency only for what can change it.
        //
        // `absorb` used to rebuild on every batch whatever arrived, which charged a manifest
        // the cost of the whole store — 1336 µs a record at 1.8's single-append figure.
        //
        // **A deny-list, and that is the whole design.** The first version of this named the
        // four records that *do* move an edge, which is the shorter list and reads better.
        // It is also the one that fails silently: a record type added in a later release and
        // forgotten here would leave the adjacency stale, and nothing would say so — a
        // traversal would simply not see the edge. Named the other way round, the same
        // omission costs a rebuild nobody needed. SMYSL-2.4 §7's risk table words it this way
        // for that reason, down to `Unknown`: a record this build cannot decode may well be an
        // edge in the release that wrote it, and a build that skipped the rebuild for it would
        // be deciding, on the strength of not understanding the bytes, that they carry none.
        //
        // What is on the list is text — a manifest, a part text, a part reading. None of them
        // names a unit as an endpoint, so none can appear in `adjacency` or in `unfounded`
        // however many of them arrive. A dating (record 17, TX-P3) and a redaction (19, TX-P2)
        // belong here too and are absent because the enum does not hold them yet; each joins
        // the list in the release that adds the variant, and until then it cannot arrive.
        //
        // Pending attestations are retried inside `rebuild_adjacency`, and skipping the
        // rebuild cannot delay one indefinitely: an attestation lands when its subject
        // arrives, and a subject is a unit or a relation — neither of which is on the list.
        let moves_edges = !records.iter().all(|r| {
            matches!(
                r,
                Record::Manifest(_) | Record::PartText(_) | Record::PartReading(_)
            )
        });
        self.records.extend(records);
        if moves_edges {
            self.rebuild_adjacency();
        }
    }

    /// Attach an attestation to its unit, or to the relation its uid is the rid of (1.4). An
    /// attestation for something that is not here yet is kept in the log and re-attached on the
    /// next rebuild, so delivery order does not matter (rule U).
    /// Attach an attestation to the unit or edge it names, and say whether it landed.
    ///
    /// `false` means the subject has not arrived; the caller keeps it and retries on the next
    /// absorb, which is what `pending_attestations` is for.
    fn attach(&mut self, a: Attestation) -> bool {
        if let Some(u) = self.units.get_mut(&a.uid) {
            u.attestations.insert(a);
            return true;
        }
        if let Some(key) = self.rids.get(&a.uid).cloned() {
            if let Some(rel) = self.relations.get_mut(&key) {
                rel.attestations.insert(a);
                return true;
            }
        }
        false
    }

    fn rebuild_adjacency(&mut self) {
        self.adjacency_rebuilds += 1;
        // Attestations may have arrived before their units; retry the ones still waiting.
        // Rescanning the whole log here made every append cost the store: on twenty thousand
        // records it was the largest single term.
        if !self.pending_attestations.is_empty() {
            let waiting = std::mem::take(&mut self.pending_attestations);
            for a in waiting {
                if !self.attach(a.clone()) {
                    self.pending_attestations.push(a);
                }
            }
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
    /// The record-set digest: BLAKE3 over every record the store holds, framed as §3.1
    /// frames it, deduplicated, in ascending hash order.
    ///
    /// `state_hash` covers *derived* state, and derived state is only as complete as the
    /// deriving code. It does not fold commitments, schema declarations, pack infos or
    /// records of a type this build does not understand — so two stores differing in any of
    /// those compared equal, which is the opposite of what rule U needs from a convergence
    /// test. This digest is structural instead: `record_hashes` is filled by `absorb` for
    /// every record that arrives, whatever it is, so a record type added later is covered
    /// without anybody remembering to extend this function.
    pub fn record_set_digest(&self) -> [u8; 32] {
        let mut h = Rolling::new();
        h.update(b"smysl/rsd/1");
        h.update(&[0x00]);
        for rh in &self.record_hashes {
            h.update(rh);
        }
        h.finish()
    }

    /// Whether two stores have converged (rule U).
    ///
    /// Both digests, deliberately. The record-set digest is the answer — equal records are
    /// equal stores. `state_hash` is kept beside it as a self-check: equal records must
    /// derive equal state, so a pair that agrees on the records and disagrees on the derived
    /// state is a bug in deriving, not a pair that has failed to converge. Keeping the
    /// conjunction costs one comparison and makes that case visible instead of silent.
    pub fn converged_with(&self, other: &Store) -> bool {
        self.record_set_digest() == other.record_set_digest()
            && self.state_hash() == other.state_hash()
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
