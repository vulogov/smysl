//! A store's check, kept as derived state (AC-8, RFC SMYSL-3 §17.1).
//!
//! OKO appends one ledger operation at a time and stages each against the whole store, so
//! [`check`] ran over the store and the batch on every commit: O(store) per append, 30 ms at
//! 6 × 10³ units. [`Checked`] holds the report of the store as it stands, so a batch can be
//! checked against it by recomputing only what the batch can change.
//!
//! **The contract is exact.** For a store `S` that `checked` describes and any batch `B`,
//! `checked.with_batch(S, B, opts)` equals `check(&Overlay::new(S, B), opts)`, byte for byte
//! once sorted. `tests/incremental.rs` holds it over the fixture stores dealt into batches,
//! over generated stores, and `examples/oko_ledger --verify` holds it after every operation.
//! Anything the recomputation cannot bound falls back to the full check, so a fallback costs
//! time and never correctness.
//!
//! **What it is valid for.** One store and one set of options. [`Checked::with_batch`] falls
//! back when the options are not the ones it was built with, and [`Checked::advance`] is the
//! caller's statement that the batch was appended and nothing else was: a store that grows by
//! any other path needs a new [`Checked`].
//!
//! **How a batch is checked.** The passes run over the whole store with the batch, so every
//! lookup a rule makes is answered as the full check answers it, but they iterate only what the
//! batch can change (`passes::Scope`). The kept report is spliced by rule family:
//!
//! - **per unit** (every finding whose subject is the unit judged): dropped and recomputed for
//!   the dirty units: the batch's, units grounding on one of them, the subjects of new
//!   attestations, the endpoints of new relations, and units whose body has a `b3:` token that
//!   is a prefix of a new uid;
//! - **dangling** (`SMY-E060` on the missing uid): dropped for a uid the batch adds, added for
//!   one it references that the store does not hold;
//! - **per record** (`W432`, `W409`, `E612`, and the store-level findings a batch that falls
//!   back would change): kept, and the batch's new records' added;
//! - **aggregates**: `SMY-W013` gains the batch's undeclared kinds, `SMY-W614` is recounted for
//!   the window keys the batch adds a digest to, `SMY-W111` is recomputed (it is a count);
//! - **cycles**: `SMY-W062` is recomputed when the batch adds a `causes` or `sequences` edge.
//!   `SMY-E061` is unreachable (`passes::integrity`) and is kept;
//! - **rule E**: recomputed over the whole store (until AC-8 step 3).
//!
//! A batch falls back to the full check when it changes something global: other options, a
//! View whose profile the store does not have, a schema declaration, an edge a withdrawal
//! already names, or any record other than a unit, a relation, an attestation, a label binding
//! or a View. Their kinds are what [`Fallback`] names.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::{Code, Diagnostic, Report, Subject};
use smysl_core::{GranularityProfile, Record, RelKind, Uid, UidPrefix};
use smysl_graph::{Overlay, StoreRead};

use crate::passes::extension::{w609, w614, window_key, WindowKey};
use crate::passes::{self, Scope};
use crate::{check, CheckOptions, Pass};

/// The options a report depends on, comparable. `parts` is an external resolver, so it is
/// compared by identity: the same resolver, not an equal one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Key {
    options: String,
    parts: Option<usize>,
}

impl Key {
    fn of(opts: &CheckOptions) -> Key {
        Key {
            options: format!(
                "{:?}|{:?}|{:?}|{:?}|{:?}",
                opts.granularity, opts.labels, opts.only, opts.consumer, opts.estimator
            ),
            parts: opts
                .parts
                .as_ref()
                .map(|p| std::sync::Arc::as_ptr(p) as *const () as usize),
        }
    }
}

/// How a batch's report was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Path {
    /// Recomputed where the batch can reach.
    Incremental,
    /// The full check, and why.
    Full(Fallback),
}

/// Why a batch was checked in full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Fallback {
    /// The options are not the ones the cache was built with.
    Options,
    /// A View whose granularity profile the store has no view with: every unit's profile can
    /// move.
    Profile,
    /// A schema declaration the store does not hold: every relation of its kinds can change.
    Declaration,
    /// An edge a withdrawal in the store already names.
    Withdrawn,
    /// A record of this kind, whose findings are the store's rather than one unit's.
    Record(&'static str),
    /// The kept report holds a finding no family claims (a code added since).
    Unclassified,
}

/// A batch checked against a store: the report of the store with the batch, and how it was
/// reached.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BatchReport {
    pub report: Report,
    pub path: Path,
    /// The options it was reached under, which the cache takes on when it advances.
    key: Key,
    /// What the cache's indexes become.
    next: Next,
}

#[derive(Debug, Clone)]
enum Next {
    /// The batch's new records, folded into the indexes.
    Absorb(Vec<Record>),
    /// Indexes rebuilt from the store with the batch.
    Replace(Box<Derived>),
}

/// What the cache keeps beside the report, so that a batch needs no walk of the store.
#[derive(Debug, Clone, Default)]
struct Derived {
    /// The views' profiles, each once, in view order: a View with one of these leaves
    /// [`crate::store_granularity`] as it is.
    profiles: Vec<GranularityProfile>,
    /// The profile the units are checked against.
    granularity: GranularityProfile,
    /// Who grounds on a uid.
    grounded_by: BTreeMap<Uid, BTreeSet<Uid>>,
    /// The `b3:` tokens in unit bodies, with the unit whose body holds each.
    tokens: Vec<(UidPrefix, Uid)>,
    /// `SMY-W614`'s window keys and their digests.
    windows: BTreeMap<WindowKey, BTreeSet<[u8; 32]>>,
    /// Units with an `x.tel/measured-by` from them, withdrawn ones included (`SMY-W609`).
    measured: BTreeSet<Uid>,
    /// Relations a withdrawal names.
    withdrawn: BTreeSet<Uid>,
}

fn measured_by() -> RelKind {
    RelKind::Extension(smysl_core::schemas::tel::MEASURED_BY.to_string())
}

impl Derived {
    fn of(store: &(impl StoreRead + ?Sized), opts: &CheckOptions) -> Derived {
        let profiles = crate::distinct_profiles(store);
        let granularity = crate::effective_granularity(|| crate::widest(&profiles), opts);
        let mut d = Derived {
            profiles,
            granularity,
            ..Derived::default()
        };
        for (uid, unit) in store.units() {
            d.unit(uid, &unit.core);
        }
        let kind = measured_by();
        for r in store.relations() {
            if r.kind == kind {
                d.measured.insert(r.from);
            }
        }
        for w in store.withdrawals() {
            d.withdrawn.insert(w.relation);
        }
        d
    }

    fn unit(&mut self, uid: &Uid, core: &smysl_core::UnitCore) {
        for g in &core.grounds {
            self.grounded_by.entry(*g).or_default().insert(*uid);
        }
        if let Some(body) = &core.body {
            for token in passes::closure::tokenise(body) {
                if token.starts_with("b3:") {
                    if let Ok(p) = UidPrefix::parse(token) {
                        self.tokens.push((p, *uid));
                    }
                }
            }
        }
        if let Some((key, digest)) = window_key(core) {
            self.windows.entry(key).or_default().insert(digest);
        }
    }

    /// The batch's new records, which [`Checked::refusal`] let through.
    fn absorb(&mut self, fresh: &[Record]) {
        let kind = measured_by();
        for r in fresh {
            match r {
                Record::Unit(core) => self.unit(&smysl_core::canonical_uid(core), core),
                Record::Relation(rel) if rel.kind == kind => {
                    self.measured.insert(rel.from);
                }
                _ => {}
            }
        }
    }
}

/// The family a kept finding belongs to, which says how a batch changes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    /// About this unit alone, recomputed when it is dirty.
    Unit(Uid),
    /// `SMY-E060`: this uid is referenced and absent.
    Dangling(Uid),
    /// Kept: what a batch can only add to, or what only a fallback changes.
    Kept,
    /// `SMY-W614`, recounted per key.
    Windows,
    /// `SMY-W111`, recomputed.
    Repeats,
    /// `SMY-W062`, recomputed when a causal edge arrives.
    Causal,
    /// Rule E's, recomputed.
    Time,
}

fn family(d: &Diagnostic) -> Option<Family> {
    let unit = match d.subject {
        Subject::Unit(u) => Some(u),
        _ => None,
    };
    let f = match d.code {
        Code::E060 => match unit {
            Some(u) => Family::Dangling(u),
            None => Family::Time,
        },
        Code::E410 | Code::W449 | Code::W411 | Code::W412 | Code::W413 => Family::Time,
        Code::W111 => Family::Repeats,
        Code::W062 => Family::Causal,
        Code::W614 => Family::Windows,
        Code::E061
        | Code::W056
        | Code::W013
        | Code::E012
        | Code::W014
        | Code::E611
        | Code::E612
        | Code::W432
        | Code::W409
        | Code::E452
        | Code::E446
        | Code::E401
        | Code::W418 => Family::Kept,
        Code::E403 => match unit {
            Some(u) => Family::Unit(u),
            None => Family::Kept,
        },
        Code::E020
        | Code::E072
        | Code::W024
        | Code::W057
        | Code::E030
        | Code::E603
        | Code::E607
        | Code::W010
        | Code::W606
        | Code::W609
        | Code::E040
        | Code::W041
        | Code::E404
        | Code::W405
        | Code::E021
        | Code::E022
        | Code::E023
        | Code::E031
        | Code::E032
        | Code::E034
        | Code::W025
        | Code::E033
        | Code::W035 => Family::Unit(unit?),
        _ => return None,
    };
    Some(f)
}

/// `SMY-E060` on a missing uid, as the store reports it.
fn dangling(uid: Uid) -> Diagnostic {
    Diagnostic::new(Code::E060)
        .with_subject(Subject::Unit(uid))
        .with_message("referenced by this store but not present in it")
}

/// A store's check report, kept so a batch can be checked against it.
#[derive(Debug, Clone)]
pub struct Checked {
    key: Key,
    report: Report,
    derived: Derived,
}

impl Checked {
    /// The full check of `store`.
    pub fn new(store: &(impl StoreRead + ?Sized), opts: &CheckOptions) -> Checked {
        Checked {
            key: Key::of(opts),
            report: check(store, opts.clone()),
            derived: Derived::of(store, opts),
        }
    }

    /// The store's report.
    pub fn report(&self) -> &Report {
        &self.report
    }

    /// The report of `store` with `batch` added, which [`check`] over an [`Overlay`] of the two
    /// would give.
    pub fn with_batch(
        &self,
        store: &(impl StoreRead + ?Sized),
        batch: Vec<Record>,
        opts: &CheckOptions,
    ) -> BatchReport {
        let key = Key::of(opts);
        let union = Overlay::new(store, batch);
        let why = if key != self.key {
            Some(Fallback::Options)
        } else {
            self.refusal(union.batch_records())
        };
        let report = match why {
            None => self.splice(&union, opts),
            Some(_) => None,
        };
        match (report, why) {
            (Some(report), _) => BatchReport {
                report,
                path: Path::Incremental,
                key,
                next: Next::Absorb(union.batch_records().to_vec()),
            },
            (None, why) => BatchReport {
                report: check(&union, opts.clone()),
                path: Path::Full(why.unwrap_or(Fallback::Unclassified)),
                next: Next::Replace(Box::new(Derived::of(&union, opts))),
                key,
            },
        }
    }

    /// Why the batch's new records cannot be checked incrementally, if they cannot.
    fn refusal(&self, fresh: &[Record]) -> Option<Fallback> {
        for r in fresh {
            match r {
                Record::Unit(_) | Record::Attestation(_) | Record::LabelBinding(_) => {}
                Record::Relation(rel) => {
                    if self.derived.withdrawn.contains(&rel.uid()) {
                        return Some(Fallback::Withdrawn);
                    }
                }
                Record::View(v) => {
                    if !self.derived.profiles.contains(&v.granularity) {
                        return Some(Fallback::Profile);
                    }
                }
                Record::SchemaDecl(_) => return Some(Fallback::Declaration),
                other => return Some(Fallback::Record(other.type_name())),
            }
        }
        None
    }

    /// The kept report, spliced with what the batch changes. `None` when a kept finding
    /// belongs to no family.
    fn splice<S: StoreRead + ?Sized>(
        &self,
        union: &Overlay<'_, S>,
        opts: &CheckOptions,
    ) -> Option<Report> {
        let fresh = union.batch_records();
        let d = &self.derived;

        // What the batch adds, and who it can change.
        let mut added_units: BTreeSet<Uid> = BTreeSet::new();
        let mut referenced: BTreeSet<Uid> = BTreeSet::new();
        let mut dirty: BTreeSet<Uid> = BTreeSet::new();
        let mut measured: BTreeSet<Uid> = BTreeSet::new();
        let mut causal = false;
        let kind = measured_by();
        for r in fresh {
            match r {
                Record::Unit(core) => {
                    let uid = smysl_core::canonical_uid(core);
                    added_units.insert(uid);
                    dirty.insert(uid);
                    if let Some(by) = d.grounded_by.get(&uid) {
                        dirty.extend(by.iter().copied());
                    }
                    for (p, holder) in &d.tokens {
                        let (lo, hi) = p.range();
                        if lo <= uid && uid <= hi {
                            dirty.insert(*holder);
                        }
                    }
                    referenced.extend(core.references().copied());
                }
                Record::Attestation(a) => {
                    dirty.insert(a.uid);
                }
                Record::Relation(rel) => {
                    dirty.insert(rel.from);
                    dirty.insert(rel.to);
                    referenced.insert(rel.from);
                    referenced.insert(rel.to);
                    referenced.extend(rel.note);
                    if rel.kind == kind {
                        measured.insert(rel.from);
                    }
                    if matches!(rel.kind, RelKind::Causes | RelKind::Sequences) {
                        causal = true;
                    }
                }
                _ => {}
            }
        }
        let integrity = opts.runs(Pass::Integrity);
        #[cfg(feature = "text")]
        let time = opts.runs(Pass::Time);
        #[cfg(not(feature = "text"))]
        let time = false;

        // The kept findings the batch cannot change.
        let mut out = Report::new();
        let mut dangling_kept: BTreeSet<Uid> = BTreeSet::new();
        for f in &self.report.diagnostics {
            let keep = match family(f)? {
                Family::Unit(u) => !dirty.contains(&u),
                Family::Dangling(u) => {
                    let keep = !added_units.contains(&u);
                    if keep {
                        dangling_kept.insert(u);
                    }
                    keep
                }
                Family::Kept | Family::Windows => true,
                Family::Repeats => false,
                Family::Causal => !causal,
                Family::Time => !time,
            };
            if keep {
                out.push(f.clone());
            }
        }

        // The per-unit and per-record rules, over what the batch can change.
        let mut fresh_findings = Report::new();
        let scope = Scope::Batch {
            units: &dirty,
            records: fresh,
        };
        crate::run_passes(union, scope, opts, &d.granularity, &mut fresh_findings);
        for f in fresh_findings.diagnostics {
            // One per kind, whatever the batch: an undeclared kind already reported stays one.
            if f.code == Code::W013 && out.diagnostics.contains(&f) {
                continue;
            }
            out.push(f);
        }
        if opts.runs(Pass::Extension) {
            let series = smysl_core::schemas::tel::SERIES;
            for u in &dirty {
                let is_series = union
                    .get(u)
                    .is_some_and(|x| x.core.schema.as_str() == series);
                if is_series && !d.measured.contains(u) && !measured.contains(u) {
                    out.push(w609(*u));
                }
            }
            self.windows(fresh, &mut out);
        }

        if integrity {
            for x in referenced {
                if !dangling_kept.contains(&x) && union.get(&x).is_none() {
                    out.push(dangling(x));
                }
            }
            passes::integrity::repeated_records(union, &mut out);
            if causal {
                passes::integrity::causal_cycles(union.adjacency(), &mut out);
            }
        }
        #[cfg(feature = "text")]
        if time {
            passes::time::run(union, &mut out);
        }
        // Counted afresh: a recount removed findings the counts still hold.
        let mut report = Report::new();
        for f in out.diagnostics {
            report.push(f);
        }
        report.sort();
        Some(report)
    }

    /// `SMY-W614`, recounted for the keys the batch adds a digest to.
    fn windows(&self, fresh: &[Record], out: &mut Report) {
        let mut grown: BTreeMap<WindowKey, BTreeSet<[u8; 32]>> = BTreeMap::new();
        for r in fresh {
            if let Record::Unit(core) = r {
                if let Some((key, digest)) = window_key(core) {
                    grown.entry(key).or_default().insert(digest);
                }
            }
        }
        for (key, new) in grown {
            let before = self.derived.windows.get(&key);
            let was = before.map_or(0, BTreeSet::len);
            let now = match before {
                Some(b) => b.union(&new).count(),
                None => new.len(),
            };
            if now == was {
                continue;
            }
            if was > 1 {
                let old = w614(&key, was);
                if let Some(i) = out.diagnostics.iter().position(|f| *f == old) {
                    out.diagnostics.remove(i);
                }
            }
            if now > 1 {
                out.push(w614(&key, now));
            }
        }
    }

    /// The batch was appended: its report is the store's now.
    pub fn advance(&mut self, batch: BatchReport) {
        self.key = batch.key;
        self.report = batch.report;
        match batch.next {
            Next::Absorb(fresh) => self.derived.absorb(&fresh),
            Next::Replace(d) => self.derived = *d,
        }
    }
}
