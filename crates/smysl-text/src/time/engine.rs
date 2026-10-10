//! Rule E: effective time, stratified by how well each value is evidenced (A-12.2).
//!
//! # What the rule is for
//!
//! A corpus has clocks in it that are wrong. A chat export ran three hours slow for a week; a
//! title page says 1769 and the translation is from 1611; a reader supplied an instant and a
//! person later found the export header that contradicts it. The naive repair is to correct the
//! record — and a unit's `observed` is inside its uid, so correcting it in place gives every
//! affected unit a new identity, and a store that re-identified its contents whenever a clock
//! turned out to be wrong could not be cited. So corrections are *records beside* the units
//! (record 17), and the time a unit effectively has is **recomputed** from the record set every
//! time the record set changes. Nothing is stored.
//!
//! # No silent override
//!
//! The one property the rule exists to give is that a weakly evidenced value never quietly
//! replaces a strongly evidenced one. It is delivered by **stratification**: the network is
//! solved once per status level, from `measured` down to `speculative`, and a bound moves only
//! if the stratum doing the moving is at least as well evidenced as the value currently there.
//! A move that a stratum would make and may not is not dropped — it is reported, as a position
//! of a detection-kind-4 contention (`SMY-W412`), so that "somebody typed a date that disagrees
//! with the instrument" is visible rather than absent.
//!
//! Because a stratum holds only edges of its own status or better, the weakest link on any path
//! bounds what that path can justify. That is OQ-42, answered in A-12.2, and it falls out of the
//! construction rather than being checked for.
//!
//! # What this release computes, and what it cannot
//!
//! Subjects are units and manifests. Parts and windows are scopes: a dating names them and they
//! resolve to the units drawn from them.
//!
//! - A **manifest-scoped** dating (target kind 2) resolves through the manifest's part list,
//!   because a unit does not name its manifest until `source.manifest` lands (FC-3, TX-P5).
//!   That over-selects exactly when two manifests list the same part — which happens on every
//!   supersession chain, since appending to an expression rewrites one part and keeps the rest.
//!   [`Effective::inexact_scopes`] counts it rather than leaving it to be discovered.
//! - **Nothing dates a manifest.** A-5 gives target kind 2 to "every unit drawn under one
//!   manifest", so the format as written offers no way to correct a catalog entry's own
//!   publication date; the engine follows the wording. Recorded as OQ-72.
//! - The **reply** free constraint has no data (TX-P5), and a derivation bound reaches units
//!   only through FC-3. See [`super::constraints`].

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::types::{Axis, Commitment, Dating, DatingTarget, DatingValue, Manifest};
use smysl_core::{Allen, ContentionId, DetectionKind, Did, Mid, Uid};
use smysl_graph::Store;

use super::constraints::{self, Free, FreeKind};
use super::edtf;
use super::stn::Network;
use super::{Bound, Instant, Interval, Subject, TimeStatus};

/// What set a bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Cause {
    /// The subject's own `published` or `observed`.
    AsRecorded,
    /// A dating's absolute or relative value.
    Dating(Did),
    /// A dating's offset, applied to the as-recorded instant before propagation (step 4).
    Offset(Did),
    /// A free constraint (D-3).
    Free(FreeKind),
}

impl std::fmt::Display for Cause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Cause::AsRecorded => f.write_str("as recorded"),
            Cause::Dating(d) => write!(f, "dating {d}"),
            Cause::Offset(d) => write!(f, "offset {d}"),
            Cause::Free(k) => write!(f, "{k}"),
        }
    }
}

/// One link of a why-chain: what set a bound, and the subject it came through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Why {
    pub cause: Cause,
    /// The subject the bound propagated from, when it came along an ordering rather than from
    /// the subject's own record. Following these is the chain `date show --why` prints.
    pub through: Option<Subject>,
    /// The status the move was justified at.
    pub at: TimeStatus,
}

/// A subject's effective time on one axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dated {
    pub interval: Interval,
    /// How well each bound is evidenced. The two differ often: a `measured` instant with a
    /// `cited` ceiling over it is an ordinary thing for a corpus to contain.
    pub lo_status: TimeStatus,
    pub hi_status: TimeStatus,
    pub why_lo: Option<Why>,
    pub why_hi: Option<Why>,
    /// Nothing was chosen: the constraints over this subject cannot all hold (step 5).
    pub contested: bool,
}

/// A contention rule E derived. **Never written** to a store (A-8.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeContention {
    pub id: ContentionId,
    pub kind: DetectionKind,
    pub over: Subject,
    /// The datings involved, in did order.
    pub datings: Vec<Did>,
    /// The free constraints involved, in kind order.
    pub free: Vec<FreeKind>,
}

/// Effective time over a store, for one axis.
#[derive(Debug, Clone, Default)]
pub struct Effective {
    pub axis_said: bool,
    pub dated: BTreeMap<Subject, Dated>,
    /// `SMY-W412`: a live dating that did not move a bound.
    pub not_applied: Vec<TimeContention>,
    /// `SMY-W413`: constraints that cannot all hold.
    pub inconsistent: Vec<TimeContention>,
    /// Datings a lock is holding (A-12.2, liveness).
    pub held: Vec<Did>,
    /// Datings a withdrawal has made not live, or whose basis is unfounded.
    pub not_live: Vec<Did>,
    /// Manifest-scoped datings resolved through a part two manifests share.
    pub inexact_scopes: usize,
    /// Datings whose target this store cannot resolve to any subject.
    pub unresolved: Vec<Did>,
}

impl Effective {
    pub fn of(&self, s: &Subject) -> Option<&Dated> {
        self.dated.get(s)
    }

    pub fn interval(&self, s: &Subject) -> Interval {
        self.dated.get(s).map_or(Interval::UNDATED, |d| d.interval)
    }

    pub fn contested(&self) -> impl Iterator<Item = &Subject> + '_ {
        self.dated
            .iter()
            .filter(|(_, d)| d.contested)
            .map(|(s, _)| s)
    }
}

/// Compute effective time for one axis.
///
/// Over the whole store rather than over one component, which costs nothing: the network is
/// sparse, disconnected subjects simply keep their seeds, and the alternative — finding
/// components first — would be a second traversal to save a few relaxations.
pub fn effective(store: &Store, axis: Axis) -> Effective {
    let mut out = Effective {
        axis_said: axis == Axis::Said,
        ..Default::default()
    };

    // 1. Subjects, in a fixed order. Units before manifests, each in identity order, so a
    //    variable index is a function of the store's contents and not of its iteration order.
    let mut subjects: Vec<Subject> = store.units().map(|(u, _)| Subject::Unit(*u)).collect();
    subjects.extend(store.manifests().map(|(m, _)| Subject::Manifest(*m)));
    subjects.sort_unstable();
    let index: BTreeMap<Subject, usize> =
        subjects.iter().enumerate().map(|(i, s)| (*s, i)).collect();
    if subjects.is_empty() {
        return out;
    }

    // 2. Seeds: the as-recorded interval and the status of the value that set it.
    let mut state: Vec<Dated> = subjects.iter().map(|s| seed(store, s, axis)).collect();

    // 3. The datings that bear on this axis, with their status and liveness.
    let live = live_datings(store, axis, &mut out);

    // 4. Offsets, applied to the as-recorded instant before propagation (step 4).
    apply_offsets(&live, &index, &mut state, &mut out, store, axis);

    // 5. The free constraints.
    let (frees, _unbuilt) = constraints::free(store);

    // 6. Stratified tightening.
    for s in TimeStatus::STRATA.iter().copied() {
        let mut net = Network::new(subjects.len());
        let mut causes: Vec<(Cause, Option<Subject>)> = Vec::new();
        // The current bounds, so that the solve's answer is implied by what is already known.
        // They carry no cause: a bound that does not move needs no new explanation.
        let current = push_cause(&mut causes, Cause::AsRecorded, None);
        for (i, d) in state.iter().enumerate() {
            if let Bound::At(at) = d.interval.lo {
                net.lower(i, at.0, current);
            }
            if let Bound::At(at) = d.interval.hi {
                net.upper(i, last(at), current);
            }
        }
        for d in &live {
            if d.status < s {
                continue;
            }
            add_dating(&mut net, &mut causes, d, &index, &state);
        }
        for f in &frees {
            if f.status() < s {
                continue;
            }
            add_free(&mut net, &mut causes, f, &index);
        }

        let sol = net.solve();

        // Step 5 first: a subject on a negative cycle has nothing chosen for it, and every
        // subject the cycle can reach is on it.
        for v in &sol.inconsistent {
            if state[*v].contested {
                continue;
            }
            state[*v].contested = true;
            out.inconsistent.push(contention(
                DetectionKind::TemporalInconsistency,
                subjects[*v],
                &live,
                &frees,
                s,
            ));
        }

        // Step 3: apply what this stratum justifies, report what it does not.
        for (i, d) in state.iter_mut().enumerate() {
            if d.contested {
                continue;
            }
            if let Some(lo) = sol.lower[i] {
                let tighter = match d.interval.lo {
                    Bound::Open => true,
                    Bound::At(at) => lo > at.0,
                };
                if tighter {
                    let why = sol.lower_by[i].map(|b| causes[b as usize]);
                    if s >= d.lo_status {
                        d.interval.lo = Bound::At(Instant(lo));
                        d.lo_status = s;
                        d.why_lo = why.map(|(cause, through)| Why {
                            cause,
                            through,
                            at: s,
                        });
                    } else if let Some((Cause::Dating(did) | Cause::Offset(did), _)) = why {
                        out.not_applied.push(one_dating_contention(
                            subjects[i],
                            did,
                            DetectionKind::DatingNotApplied,
                        ));
                    }
                }
            }
            if let Some(hi) = sol.upper[i] {
                // Back from the network's **inclusive** last instant to the interval's
                // half-open end. Everything inside a network is inclusive, because a
                // difference constraint is an inequality over instants and not over intervals;
                // mixing the two is an off-by-one that only shows up as a date a day wide.
                let hi = hi.saturating_add(1);
                let tighter = match d.interval.hi {
                    Bound::Open => true,
                    Bound::At(at) => hi < at.0,
                };
                if tighter {
                    let why = sol.upper_by[i].map(|b| causes[b as usize]);
                    if s >= d.hi_status {
                        d.interval.hi = Bound::At(Instant(hi));
                        d.hi_status = s;
                        d.why_hi = why.map(|(cause, through)| Why {
                            cause,
                            through,
                            at: s,
                        });
                    } else if let Some((Cause::Dating(did) | Cause::Offset(did), _)) = why {
                        out.not_applied.push(one_dating_contention(
                            subjects[i],
                            did,
                            DetectionKind::DatingNotApplied,
                        ));
                    }
                }
            }
            // An interval that has closed to nothing is step 5's other half.
            if d.interval.is_empty() {
                d.contested = true;
                out.inconsistent.push(contention(
                    DetectionKind::TemporalInconsistency,
                    subjects[i],
                    &live,
                    &frees,
                    s,
                ));
            }
        }
    }

    out.not_applied.sort_by(|a, b| a.id.cmp(&b.id));
    out.not_applied.dedup_by(|a, b| a.id == b.id);
    out.inconsistent.sort_by(|a, b| a.id.cmp(&b.id));
    out.inconsistent.dedup_by(|a, b| a.id == b.id);
    out.held.sort_unstable();
    out.held.dedup();
    out.not_live.sort_unstable();
    out.not_live.dedup();
    out.unresolved.sort_unstable();
    out.unresolved.dedup();
    out.dated = subjects.into_iter().zip(state).collect();
    out
}

/// The last instant a half-open end admits: a network speaks in inclusive instants.
fn last(end: Instant) -> i64 {
    end.0.saturating_sub(1)
}

fn push_cause(
    causes: &mut Vec<(Cause, Option<Subject>)>,
    cause: Cause,
    through: Option<Subject>,
) -> u32 {
    causes.push((cause, through));
    (causes.len() - 1) as u32
}

/// A dating with everything the engine needs to place it, resolved once.
#[derive(Debug, Clone)]
struct Live {
    did: Did,
    dating: Dating,
    status: TimeStatus,
    /// The subjects it dates.
    scope: Vec<Subject>,
}

/// Every live dating on this axis, and the ones that are not live and why.
///
/// Liveness is A-12.2: a dating is live unless a withdrawal names its did, its basis is
/// unfounded (rule R), or a lock holds it. The first two are questions a store answers
/// directly. The third is the interesting one — see [`held_by_lock`].
fn live_datings(store: &Store, axis: Axis, out: &mut Effective) -> Vec<Live> {
    let withdrawn: BTreeSet<Uid> = store.withdrawals().map(|w| w.relation).collect();
    let mut live = Vec::new();
    for (did, dating) in store.datings() {
        if dating.axis != axis {
            continue;
        }
        let as_uid = Uid::from_bytes(*did.as_bytes());
        if withdrawn.contains(&as_uid) {
            out.not_live.push(*did);
            continue;
        }
        if let Some(basis) = dating.basis {
            if store.is_unfounded(&basis) {
                out.not_live.push(*did);
                continue;
            }
        }
        if held_by_lock(store, did, dating) {
            out.held.push(*did);
            continue;
        }
        let scope = resolve(store, &dating.target, out);
        if scope.is_empty() {
            out.unresolved.push(*did);
            continue;
        }
        live.push(Live {
            did: *did,
            dating: dating.clone(),
            // D-4: a dating takes the status of its basis unit, and `speculative` without one.
            // That is the whole mechanism by which "the export header says so" outranks "it
            // must have been about then" without anybody ranking them by hand.
            status: dating
                .basis
                .and_then(|b| store.units().find(|(u, _)| **u == b).map(|(_, u)| u))
                .map_or(TimeStatus::Speculative, |u| {
                    constraints::of_status(u.core.status)
                }),
            scope,
        });
    }
    live
}

/// Whether a `canonical` commitment is holding this dating for review (A-12.2, locks).
///
/// Two cases. A commitment on the dating's own did holds it outright. A commitment on *another*
/// dating of the same target holds this one, because the point of a lock is that the value
/// under it does not move until somebody has looked.
///
/// The second case is narrowed by comparing the two datings' **written values**: two datings
/// that say the same thing cannot disagree, so one is not held by a lock on the other. A-12.2
/// words the condition as "the new dating would change the effective value", which is a
/// question about the result; comparing the values is a cheap sufficient test for the answer
/// being *no*, and erring towards holding is what a lock is for.
fn held_by_lock(store: &Store, did: &Did, dating: &Dating) -> bool {
    let as_uid = Uid::from_bytes(*did.as_bytes());
    if store.commitment_of(&as_uid) == Some(Commitment::Canonical) {
        // Unless a resolution has named the contention the hold derives.
        return !resolved(store, did, dating);
    }
    for (other_did, other) in store.datings() {
        if other_did == did || other.target != dating.target || other.axis != dating.axis {
            continue;
        }
        let other_uid = Uid::from_bytes(*other_did.as_bytes());
        if store.commitment_of(&other_uid) != Some(Commitment::Canonical) {
            continue;
        }
        if other.value == dating.value {
            continue;
        }
        if !resolved(store, did, dating) {
            return true;
        }
    }
    false
}

/// Whether a resolution names the kind-4 contention a hold on this dating derives.
fn resolved(store: &Store, did: &Did, dating: &Dating) -> bool {
    let over = match &dating.target {
        DatingTarget::Unit(u) => *u,
        DatingTarget::Part(t) => Uid::from_bytes(*t.as_bytes()),
        DatingTarget::Manifest(m) => Uid::from_bytes(*m.as_bytes()),
        DatingTarget::Window { tid, .. } => Uid::from_bytes(*tid.as_bytes()),
        _ => return false,
    };
    let id = ContentionId::derive(
        DetectionKind::DatingNotApplied,
        &over,
        &[Uid::from_bytes(*did.as_bytes())],
    );
    store.resolutions().any(|r| match &r.target {
        smysl_core::ResolutionTarget::Contention(c) => *c == id,
        _ => false,
    })
}

/// The subjects a target names.
fn resolve(store: &Store, target: &DatingTarget, out: &mut Effective) -> Vec<Subject> {
    let mut v: Vec<Subject> = match target {
        DatingTarget::Unit(u) => vec![Subject::Unit(*u)],
        DatingTarget::Part(t) => store
            .units_with_tid(t)
            .into_iter()
            .map(Subject::Unit)
            .collect(),
        DatingTarget::Manifest(m) => {
            let Some(manifest) = store.manifest(m) else {
                return Vec::new();
            };
            if shares_a_part(store, m, manifest) {
                out.inexact_scopes += 1;
            }
            manifest
                .parts
                .iter()
                .flat_map(|p| store.units_with_tid(&p.tid))
                .map(Subject::Unit)
                .collect()
        }
        DatingTarget::Window {
            tid,
            from_ms,
            to_ms,
        } => store
            .units_with_tid(tid)
            .into_iter()
            .filter(|u| {
                // The **as-recorded** instant, never the effective one: a window that moved as
                // the datings it selects took effect would select a different set on every
                // pass, and the solve would not converge.
                store
                    .units()
                    .find(|(x, _)| *x == u)
                    .and_then(|(_, unit)| unit.core.source.as_ref())
                    .and_then(|s| s.observed)
                    .is_some_and(|ms| ms >= *from_ms && ms < *to_ms)
            })
            .map(Subject::Unit)
            .collect(),
        _ => Vec::new(),
    };
    v.sort_unstable();
    v.dedup();
    v
}

/// Whether another manifest lists one of this one's parts.
fn shares_a_part(store: &Store, mid: &Mid, m: &Manifest) -> bool {
    store.manifests().any(|(other, om)| {
        other != mid
            && om
                .parts
                .iter()
                .any(|p| m.parts.iter().any(|q| q.tid == p.tid))
    })
}

/// The as-recorded interval and status of a subject (step 1).
fn seed(store: &Store, s: &Subject, axis: Axis) -> Dated {
    let undated = |lo_status: TimeStatus| Dated {
        interval: Interval::UNDATED,
        lo_status,
        hi_status: lo_status,
        why_lo: None,
        why_hi: None,
        contested: false,
    };
    // Only the *said* axis has an as-recorded value. `composed` is reached through datings on a
    // work's catalog entity and `about` through datings alone, so both start undated — which is
    // not a gap: a unit that nobody has dated on those axes has no composed or about time, and
    // inventing one from `observed` would be asserting that a message is about the moment it
    // was sent.
    if axis != Axis::Said {
        return undated(TimeStatus::Speculative);
    }
    match s {
        Subject::Unit(uid) => {
            let Some(unit) = store.units().find(|(u, _)| *u == uid).map(|(_, u)| u) else {
                return undated(TimeStatus::Speculative);
            };
            let Some(source) = unit.core.source.as_ref() else {
                return undated(TimeStatus::Speculative);
            };
            // `published` first: a value a source stated beats one a reader derived, and A-2.4
            // rule 2 means a unit carrying both is one whose instant is its own.
            if let Some(published) = source.published.as_deref() {
                if let Ok(v) = edtf::parse(published) {
                    let status = TimeStatus::Cited;
                    return Dated {
                        interval: edtf::to_interval(&v),
                        lo_status: status,
                        hi_status: status,
                        why_lo: Some(Why {
                            cause: Cause::AsRecorded,
                            through: None,
                            at: status,
                        }),
                        why_hi: Some(Why {
                            cause: Cause::AsRecorded,
                            through: None,
                            at: status,
                        }),
                        contested: false,
                    };
                }
            }
            if let Some(ms) = source.observed.and_then(|ms| i64::try_from(ms).ok()) {
                // Measured only when an instrument put it there: `Imported` at rung
                // `computed` is rule T's own test, and a reader's export is `cited`.
                let measured = store.attestations_of(uid).iter().any(|a| {
                    a.op == smysl_core::Op::Imported && a.rung == smysl_core::Rung::Computed
                });
                let status = if measured {
                    TimeStatus::Measured
                } else {
                    TimeStatus::Cited
                };
                return Dated {
                    interval: Interval::at(Instant(ms)),
                    lo_status: status,
                    hi_status: status,
                    why_lo: Some(Why {
                        cause: Cause::AsRecorded,
                        through: None,
                        at: status,
                    }),
                    why_hi: Some(Why {
                        cause: Cause::AsRecorded,
                        through: None,
                        at: status,
                    }),
                    contested: false,
                };
            }
            undated(TimeStatus::Speculative)
        }
        Subject::Manifest(mid) => {
            let Some(m) = store.manifest(mid) else {
                return undated(TimeStatus::Speculative);
            };
            let Some(published) = m.published.as_deref() else {
                return undated(TimeStatus::Speculative);
            };
            let Ok(v) = edtf::parse(published) else {
                return undated(TimeStatus::Speculative);
            };
            let status = TimeStatus::Cited;
            Dated {
                // A manifest is the one subject whose calendar is known: key 18 says whether
                // key 8 is Julian, and the conversion happens here and is never written back.
                interval: edtf::to_interval_in(&v, m.calendar),
                lo_status: status,
                hi_status: status,
                why_lo: Some(Why {
                    cause: Cause::AsRecorded,
                    through: None,
                    at: status,
                }),
                why_hi: Some(Why {
                    cause: Cause::AsRecorded,
                    through: None,
                    at: status,
                }),
                contested: false,
            }
        }
    }
}

/// Step 4: an offset corrects the as-recorded instant before anything propagates.
///
/// Outside the stratified loop, which is what A-12.2 step 4 says and is the only reading that
/// makes the feature work: an export header saying the clock was three hours slow is `cited`,
/// and the instant it corrects is `measured`, so a correction inside the loop could never
/// apply. What keeps it from being a silent override is the **status of the result**: a bound an
/// offset moved is no better evidenced than the offset, so it takes the lower of the two. That
/// is OQ-42's rule — a chain tightens at its weakest link — applied to the seed.
fn apply_offsets(
    live: &[Live],
    index: &BTreeMap<Subject, usize>,
    state: &mut [Dated],
    out: &mut Effective,
    store: &Store,
    axis: Axis,
) {
    // Group by subject so that two offsets disagreeing about one subject is a finding rather
    // than two shifts applied one after the other.
    let mut by_subject: BTreeMap<Subject, Vec<(&Live, i64)>> = BTreeMap::new();
    for l in live {
        let DatingValue::Offset(ms) = l.dating.value else {
            continue;
        };
        for s in &l.scope {
            by_subject.entry(*s).or_default().push((l, ms));
        }
    }
    for (subject, offsets) in by_subject {
        let Some(i) = index.get(&subject).copied() else {
            continue;
        };
        let distinct: BTreeSet<i64> = offsets.iter().map(|(_, ms)| *ms).collect();
        if distinct.len() > 1 {
            // Two corrections that cannot both be right. Nothing is chosen, which is step 5's
            // outcome reached by a different route from a negative cycle.
            state[i].contested = true;
            let mut datings: Vec<Did> = offsets.iter().map(|(l, _)| l.did).collect();
            datings.sort_unstable();
            datings.dedup();
            out.inconsistent.push(TimeContention {
                id: derive_id(DetectionKind::TemporalInconsistency, subject, &datings),
                kind: DetectionKind::TemporalInconsistency,
                over: subject,
                datings,
                free: Vec::new(),
            });
            continue;
        }
        let (l, ms) = offsets[0];
        // Re-seeded rather than shifted from the current state, because `apply_offsets` runs
        // before any propagation and the current state *is* the seed. Stating it this way means
        // a later caller cannot accidentally offset an already-offset bound.
        let base = seed(store, &subject, axis);
        if base.interval.is_undated() {
            // An offset has nothing to correct. Not an error and not silent: the dating is live
            // and did not move a bound, which is exactly what `SMY-W412` is for.
            out.not_applied.push(one_dating_contention(
                subject,
                l.did,
                DetectionKind::DatingNotApplied,
            ));
            continue;
        }
        let at = l.status.min(base.lo_status);
        state[i] = Dated {
            interval: base.interval.shift(ms),
            lo_status: at,
            hi_status: at,
            why_lo: Some(Why {
                cause: Cause::Offset(l.did),
                through: None,
                at,
            }),
            why_hi: Some(Why {
                cause: Cause::Offset(l.did),
                through: None,
                at,
            }),
            contested: false,
        };
    }
}

/// Add a dating's constraints to a stratum's network.
fn add_dating(
    net: &mut Network,
    causes: &mut Vec<(Cause, Option<Subject>)>,
    l: &Live,
    index: &BTreeMap<Subject, usize>,
    state: &[Dated],
) {
    match &l.dating.value {
        // An offset is step 4's and is already in the seed.
        DatingValue::Offset(_) => {}
        DatingValue::Absolute(text) => {
            let Ok(v) = edtf::parse(text) else { return };
            let extent = edtf::to_interval(&v);
            let by = push_cause(causes, Cause::Dating(l.did), None);
            for s in &l.scope {
                let Some(i) = index.get(s).copied() else {
                    continue;
                };
                if let Bound::At(at) = extent.lo {
                    net.lower(i, at.0, by);
                }
                if let Bound::At(at) = extent.hi {
                    net.upper(i, last(at), by);
                }
            }
        }
        DatingValue::Relative { allen, target } => {
            let other = relative_target(target, index, state);
            for s in &l.scope {
                let Some(i) = index.get(s).copied() else {
                    continue;
                };
                let by = push_cause(causes, Cause::Dating(l.did), other.subject);
                match (allen, other.variable) {
                    // Against another subject, the relation is an edge and propagates: that is
                    // what makes "A before B, B before C" bound A by C's ceiling.
                    (Allen::Before, Some(j)) => net.no_later(i, j, 0, by),
                    (Allen::After, Some(j)) => net.no_later(j, i, 0, by),
                    // With a point subject on each side, the other five collapse to equality:
                    // `during`, `overlaps`, `contains` and `equals` all say "at the same time
                    // as", and `meets` says the same of a point's single boundary. A collapse
                    // rather than a refusal because every one of these readings is *weaker*
                    // than the relation it stands for, so none of them can exclude a truth the
                    // stricter reading admits. Recorded as OQ-71.
                    (_, Some(j)) => net.same(i, j, by),
                    // Against a scope, the relation is a bound against that scope's
                    // as-recorded extent — static, which is what keeps the solve convergent.
                    (Allen::Before, None) => {
                        if let Bound::At(at) = other.extent.lo {
                            net.upper(i, at.0, by);
                        }
                    }
                    (Allen::After, None) => {
                        if let Bound::At(at) = other.extent.hi {
                            net.lower(i, last(at), by);
                        }
                    }
                    (Allen::Meets, None) => {
                        if let Bound::At(at) = other.extent.lo {
                            net.lower(i, at.0, by);
                            net.upper(i, at.0, by);
                        }
                    }
                    (_, None) => {
                        if let Bound::At(at) = other.extent.lo {
                            net.lower(i, at.0, by);
                        }
                        if let Bound::At(at) = other.extent.hi {
                            net.upper(i, last(at), by);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// What a relative dating's second target is: one variable, or a static extent.
struct Other {
    variable: Option<usize>,
    subject: Option<Subject>,
    extent: Interval,
}

fn relative_target(
    target: &DatingTarget,
    index: &BTreeMap<Subject, usize>,
    state: &[Dated],
) -> Other {
    match target {
        DatingTarget::Unit(u) => {
            let s = Subject::Unit(*u);
            let variable = index.get(&s).copied();
            Other {
                variable,
                subject: Some(s),
                extent: variable.map_or(Interval::UNDATED, |i| state[i].interval),
            }
        }
        DatingTarget::Window { from_ms, to_ms, .. } => Other {
            variable: None,
            subject: None,
            extent: Interval::of(
                i64::try_from(*from_ms).unwrap_or(i64::MAX),
                i64::try_from(*to_ms).unwrap_or(i64::MAX),
            ),
        },
        // A part or a manifest: the union of the as-recorded intervals of the subjects under
        // it. From the seeds and not from the current state, so the extent is the same in every
        // stratum — an extent that tightened as the solve proceeded would make the answer
        // depend on the order the strata ran in, which is the one thing rule U forbids.
        DatingTarget::Part(_) | DatingTarget::Manifest(_) => Other {
            variable: None,
            subject: None,
            extent: Interval::UNDATED,
        },
        _ => Other {
            variable: None,
            subject: None,
            extent: Interval::UNDATED,
        },
    }
}

fn add_free(
    net: &mut Network,
    causes: &mut Vec<(Cause, Option<Subject>)>,
    f: &Free,
    index: &BTreeMap<Subject, usize>,
) {
    match f {
        Free::Order {
            kind,
            earlier,
            later,
            ..
        } => {
            let (Some(a), Some(b)) = (index.get(earlier).copied(), index.get(later).copied())
            else {
                return;
            };
            let by = push_cause(causes, Cause::Free(*kind), Some(*later));
            net.no_later(a, b, 0, by);
        }
        Free::NoLater {
            kind,
            subject,
            than,
            ..
        } => {
            let Some(i) = index.get(subject).copied() else {
                return;
            };
            let by = push_cause(causes, Cause::Free(*kind), None);
            net.upper(i, last(*than), by);
        }
    }
}

fn derive_id(kind: DetectionKind, over: Subject, datings: &[Did]) -> ContentionId {
    let over = match over {
        Subject::Unit(u) => u,
        Subject::Manifest(m) => Uid::from_bytes(*m.as_bytes()),
    };
    let positions: Vec<Uid> = datings
        .iter()
        .map(|d| Uid::from_bytes(*d.as_bytes()))
        .collect();
    ContentionId::derive(kind, &over, &positions)
}

fn one_dating_contention(over: Subject, did: Did, kind: DetectionKind) -> TimeContention {
    TimeContention {
        id: derive_id(kind, over, &[did]),
        kind,
        over,
        datings: vec![did],
        free: Vec::new(),
    }
}

/// The contention for an inconsistency: every dating and free constraint of this stratum that
/// touches the subject.
///
/// Named by what was in the stratum rather than by a minimal unsatisfiable core. A minimal core
/// would be better to read and is NP-hard to find in general; naming the stratum's constraints
/// over the subject is what a reviewer needs to look at, and it is a function of the record set.
fn contention(
    kind: DetectionKind,
    over: Subject,
    live: &[Live],
    frees: &[Free],
    stratum: TimeStatus,
) -> TimeContention {
    let mut datings: Vec<Did> = live
        .iter()
        .filter(|l| l.status >= stratum && l.scope.contains(&over))
        .map(|l| l.did)
        .collect();
    datings.sort_unstable();
    datings.dedup();
    let mut free: Vec<FreeKind> = frees
        .iter()
        .filter(|f| {
            f.status() >= stratum
                && match f {
                    Free::Order { earlier, later, .. } => *earlier == over || *later == over,
                    Free::NoLater { subject, .. } => *subject == over,
                }
        })
        .map(|f| f.kind())
        .collect();
    free.sort_unstable();
    free.dedup();
    TimeContention {
        id: derive_id(kind, over, &datings),
        kind,
        over,
        datings,
        free,
    }
}
