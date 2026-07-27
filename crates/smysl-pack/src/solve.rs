//! The solver (§18.3, §18.4).
//!
//! Greedy by density over closure-augmented items, then bounded local improvement. Every
//! tie-break is total and every enumeration is in dense-id order, so the same graph, budget
//! and thread yield identical bytes (rule D).
//!
//! **Failure is a feature.** If the mandatory floor does not fit, packing returns the
//! minimum feasible budget rather than emitting something smaller. That is rule R doing its
//! job: a budget too small to hold a claim *and* its rebuttals must not quietly yield the
//! claim alone.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::{Code, Diagnostic, Report};
use smysl_core::{DropReason, Lod, Optimality, PackError, PackInfo, PackMode, ThreadId, Uid};
use smysl_graph::{SalienceReport, Store};

use crate::bound;
use crate::closure;
use crate::constraints::{violations, Constraints, Selection, Violation};
use crate::cost::{available_levels, value, Estimator};

/// Default local-improvement passes (§18.3).
pub const IMPROVEMENT_PASSES: usize = 8;
/// Default store size above which exact mode declines to run (§18.3 step 4).
pub const EXACT_THRESHOLD: usize = 256;

/// What to pack.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PackRequest {
    pub budget: u64,
    pub thread: Option<ThreadId>,
    /// Units that must reach L1 (C5).
    pub focus: BTreeSet<Uid>,
    pub mode: PackMode,
    pub estimator: Estimator,
    /// Cap every unit at this level, whatever it was authored at.
    pub max_lod: Option<Lod>,
    /// Restrict packing to these units; empty means the whole store.
    pub scope: BTreeSet<Uid>,
    pub improvement_passes: usize,
    /// Above this many units, `PackMode::Exact` falls back to greedy and says so
    /// (`SMY-W202`). The problem is NP-hard; an unbounded exact search on a large store
    /// would hang rather than answer.
    pub exact_threshold: usize,
}

impl Default for PackRequest {
    fn default() -> PackRequest {
        PackRequest {
            budget: 0,
            thread: None,
            focus: BTreeSet::new(),
            mode: PackMode::Greedy,
            estimator: Estimator::default(),
            max_lod: None,
            scope: BTreeSet::new(),
            improvement_passes: IMPROVEMENT_PASSES,
            exact_threshold: EXACT_THRESHOLD,
        }
    }
}

impl PackRequest {
    pub fn budget(budget: u64) -> PackRequest {
        PackRequest {
            budget,
            ..PackRequest::default()
        }
    }

    pub fn focusing(mut self, f: impl IntoIterator<Item = Uid>) -> PackRequest {
        self.focus = f.into_iter().collect();
        self
    }

    pub fn scoped(mut self, s: impl IntoIterator<Item = Uid>) -> PackRequest {
        self.scope = s.into_iter().collect();
        self
    }

    pub fn capped(mut self, l: Lod) -> PackRequest {
        self.max_lod = Some(l);
        self
    }

    /// Ask for a provably optimal pack.
    pub fn exact(mut self) -> PackRequest {
        self.mode = PackMode::Exact;
        self
    }
}

/// A finished pack.
#[derive(Debug, Clone, PartialEq)]
pub struct Pack {
    /// Which units, at what level.
    pub selection: Selection,
    /// The self-description every pack emits (§8).
    pub info: PackInfo,
    /// Why each unit is in (`--explain`).
    pub why: BTreeMap<Uid, closure::Reason>,
    /// Anything the caller should know that is not a failure - `SMY-W202` when exact mode
    /// declined to run.
    pub report: Report,
}

impl Pack {
    pub fn len(&self) -> usize {
        self.selection.len()
    }

    pub fn is_empty(&self) -> bool {
        self.selection.is_empty()
    }

    pub fn used(&self) -> u64 {
        self.info.used
    }

    /// Units a constraint forced in rather than value earning.
    pub fn forced(&self) -> Vec<(&Uid, &closure::Reason)> {
        self.why.iter().filter(|(_, r)| r.is_forced()).collect()
    }

    /// Whether this pack is provably the best available within its budget.
    pub fn is_optimal(&self) -> bool {
        self.info.optimality.mode == PackMode::Exact && self.info.optimality.gap == 0.0
    }
}

/// Pack a store to a budget (§18.3).
pub fn pack(
    store: &Store,
    salience: &SalienceReport,
    req: &PackRequest,
) -> Result<Pack, PackError> {
    let scope: Vec<Uid> = if req.scope.is_empty() {
        store.units().map(|(u, _)| *u).collect()
    } else {
        req.scope
            .iter()
            .copied()
            .filter(|u| store.contains_uid(u))
            .collect()
    };
    let in_scope: BTreeSet<Uid> = scope.iter().copied().collect();

    for f in &req.focus {
        if !store.contains_uid(f) {
            return Err(PackError::FocusAbsent { uid: *f });
        }
    }

    // Renormalise per thread: a unit that is middling across a corpus may be the most
    // important thing in the selection being packed (§1.5).
    let local = salience.renormalise(&scope);
    let cap = |l: Lod| match req.max_lod {
        Some(m) if l > m => m,
        _ => l,
    };

    // --- 1. the mandatory floor -------------------------------------------
    let mut selection = Selection::new();
    for f in &req.focus {
        let want = cap(highest_available(store, f));
        for (u, l) in closure::required(store, *f, want) {
            raise(&mut selection, u, l);
        }
    }
    let floor_selection = selection.clone();
    let floor_cost = cost_of(store, &selection, &req.estimator);
    if floor_cost > req.budget {
        // Rule R: the alternative is a one-sided pack, which is worse than no pack.
        return Err(PackError::Infeasible {
            budget: req.budget,
            required: floor_cost,
        });
    }

    let mut why: BTreeMap<Uid, closure::Reason> = BTreeMap::new();
    for f in &req.focus {
        why.insert(*f, closure::Reason::Focus);
        for (u, r) in closure::reasons(store, *f, cap(highest_available(store, f))) {
            why.entry(u).or_insert(r);
        }
    }

    // --- 2. greedy by density ---------------------------------------------
    let mut used = floor_cost;
    loop {
        let mut best: Option<(f64, f32, Uid, Lod, u64, Selection)> = None;
        for uid in &scope {
            let Some(unit) = store.get(uid) else { continue };
            for level in available_levels(&unit.core) {
                let level = cap(level);
                if selection.get(uid).is_some_and(|l| *l >= level) {
                    continue;
                }
                let d = closure::delta(store, &selection, *uid, level);
                if d.is_empty() {
                    continue;
                }
                let (dc, dv) = weigh(store, &selection, &d, &local, &req.estimator);
                if dc == 0 || used + dc > req.budget {
                    continue;
                }
                let density = dv / dc as f64;
                let salience_here = local.get(uid).copied().unwrap_or(0.0);
                // Total tie-break: density, then salience, then uid, then level.
                let better = match &best {
                    None => true,
                    Some((bd, bs, bu, bl, _, _)) => {
                        (
                            density,
                            salience_here,
                            std::cmp::Reverse(*uid),
                            std::cmp::Reverse(level),
                        ) > (*bd, *bs, std::cmp::Reverse(*bu), std::cmp::Reverse(*bl))
                    }
                };
                if better {
                    best = Some((density, salience_here, *uid, level, dc, d));
                }
            }
        }

        let Some((_, _, uid, level, dc, d)) = best else {
            break;
        };
        for (u, l) in d {
            raise(&mut selection, u, l);
            why.entry(u).or_insert_with(|| {
                if u == uid {
                    closure::Reason::Density
                } else {
                    closure::reasons(store, uid, level)
                        .remove(&u)
                        .unwrap_or(closure::Reason::Density)
                }
            });
        }
        used += dc;
    }

    // --- 3. local improvement ---------------------------------------------
    let constraints = Constraints {
        pinned: req.focus.clone(),
        budget: req.budget,
    };
    for _ in 0..req.improvement_passes {
        if !improve(
            store,
            &mut selection,
            &mut used,
            &local,
            &scope,
            &req.estimator,
            &constraints,
            cap,
        ) {
            break;
        }
    }

    // --- 4. exact refinement (feature `branch-and-bound`) -------------------
    let mut report = Report::new();
    let mut proven = false;
    if req.mode == PackMode::Exact {
        if scope.len() > req.exact_threshold {
            report.push(Diagnostic::new(Code::W202).with_message(format!(
                "{} units is above the exact threshold of {}; packed greedily",
                scope.len(),
                req.exact_threshold
            )));
        } else {
            #[cfg(feature = "branch-and-bound")]
            {
                let floor = floor_selection.clone();
                let found = crate::exact::solve(
                    store,
                    &scope,
                    &local,
                    &req.estimator,
                    req.budget,
                    &floor,
                    selection.clone(),
                    crate::exact::NODE_LIMIT,
                    cap,
                );
                selection = found.selection;
                used = found.used;
                proven = found.proven;
                if !found.proven {
                    report.push(Diagnostic::new(Code::W202).with_message(
                        "the exact search hit its node limit; the pack is valid but not proven optimal",
                    ));
                }
            }
            #[cfg(not(feature = "branch-and-bound"))]
            report.push(Diagnostic::new(Code::W202).with_message(
                "exact packing is not compiled in; rebuild with the `exact-pack` feature",
            ));
        }
    }

    // --- 5. the manifest ---------------------------------------------------
    let mut info = PackInfo::new(req.budget, used, req.estimator.id());
    info.thread = req.thread.clone();
    for uid in &scope {
        if selection.contains_key(uid) {
            continue;
        }
        info.dropped
            .push((*uid, drop_reason(store, &selection, uid, &local)));
    }
    for (uid, level) in &selection {
        let Some(unit) = store.get(uid) else { continue };
        if *level < cap(unit.core.max_lod()) {
            info.degraded.push((*uid, *level));
        }
    }
    // A gap from the fractional relaxation is a *provable* ceiling, not an estimate. A
    // proven-exhaustive exact search has nothing left to find, so its gap is zero.
    let headroom = if proven {
        0.0
    } else {
        bound::fractional(
            store,
            &selection,
            &scope,
            &local,
            &req.estimator,
            req.budget.saturating_sub(used),
            cap,
        )
    };
    info.optimality = Optimality {
        mode: req.mode,
        gap: bound::gap(bound::achieved(&selection, &local), headroom),
    };

    let _ = in_scope;
    debug_assert!(
        violations(store, &selection, used, &constraints).is_empty(),
        "the solver produced a selection that breaks C1-C7"
    );

    Ok(Pack {
        selection,
        info,
        why,
        report,
    })
}

/// Verify a finished pack against C1-C7. Exposed so a caller can audit rather than trust.
pub fn verify(store: &Store, pack: &Pack, req: &PackRequest) -> Vec<Violation> {
    violations(
        store,
        &pack.selection,
        pack.info.used,
        &Constraints {
            pinned: req.focus.clone(),
            budget: req.budget,
        },
    )
}

fn highest_available(store: &Store, uid: &Uid) -> Lod {
    store
        .get(uid)
        .map(|u| u.core.max_lod())
        .unwrap_or(Lod::L0)
        .max(Lod::L1)
}

fn raise(selection: &mut Selection, uid: Uid, level: Lod) {
    match selection.get(&uid) {
        Some(l) if *l >= level => {}
        _ => {
            selection.insert(uid, level);
        }
    }
}

fn cost_of(store: &Store, selection: &Selection, e: &Estimator) -> u64 {
    selection
        .iter()
        .filter_map(|(u, l)| store.get(u).map(|unit| e.unit(&unit.core, *l)))
        .sum()
}

/// The cost and value a delta adds, charging only the shortfall over what is already held.
fn weigh(
    store: &Store,
    selected: &Selection,
    d: &Selection,
    salience: &BTreeMap<Uid, f32>,
    e: &Estimator,
) -> (u64, f64) {
    let mut cost = 0u64;
    let mut val = 0.0f64;
    for (u, l) in d {
        let Some(unit) = store.get(u) else { continue };
        let from = selected.get(u).copied();
        cost += match from {
            Some(f) => e.upgrade(&unit.core, f, *l),
            None => e.unit(&unit.core, *l),
        };
        let s = salience.get(u).copied().unwrap_or(0.0);
        val += value(s, *l) - from.map(|f| value(s, f)).unwrap_or(0.0);
    }
    (cost, val)
}

/// One local-improvement pass (§18.3 step 3).
///
/// Only moves that leave C1-C7 intact are kept - every candidate is checked and reverted
/// if it breaks anything, so an optimisation can never cost correctness.
#[allow(clippy::too_many_arguments)]
fn improve(
    store: &Store,
    selection: &mut Selection,
    used: &mut u64,
    salience: &BTreeMap<Uid, f32>,
    scope: &[Uid],
    e: &Estimator,
    c: &Constraints,
    cap: impl Fn(Lod) -> Lod,
) -> bool {
    // Downgrade-to-admit: free budget from the least valuable depth, then buy breadth.
    // Deterministic order, so the same graph always tries the same move first.
    let mut candidates: Vec<(f64, Uid, Lod)> = selection
        .iter()
        .filter(|(_, l)| **l > Lod::L0)
        .filter_map(|(u, l)| {
            let unit = store.get(u)?;
            let lower = match *l {
                Lod::L2 => Lod::L1,
                _ => Lod::L0,
            };
            let freed = e.upgrade(&unit.core, lower, *l);
            if freed == 0 {
                return None;
            }
            let s = salience.get(u).copied().unwrap_or(0.0);
            let lost = value(s, *l) - value(s, lower);
            Some((lost / freed as f64, *u, lower))
        })
        .collect();
    candidates.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });

    for (lost_density, uid, lower) in candidates {
        let Some(unit) = store.get(&uid) else {
            continue;
        };
        let current = selection[&uid];
        let freed = e.upgrade(&unit.core, lower, current);

        // Is there breadth worth more than the depth being given up?
        let mut gain: Option<(f64, Uid, Lod, u64, Selection)> = None;
        for other in scope {
            if *other == uid {
                continue;
            }
            let Some(o) = store.get(other) else { continue };
            for level in available_levels(&o.core) {
                let level = cap(level);
                if selection.get(other).is_some_and(|l| *l >= level) {
                    continue;
                }
                let d = closure::delta(store, selection, *other, level);
                if d.is_empty() {
                    continue;
                }
                let (dc, dv) = weigh(store, selection, &d, salience, e);
                if dc == 0 || dc > freed {
                    continue;
                }
                let density = dv / dc as f64;
                if density <= lost_density {
                    continue;
                }
                let better = match &gain {
                    None => true,
                    Some((bd, bu, bl, _, _)) => {
                        (density, std::cmp::Reverse(*other), std::cmp::Reverse(level))
                            > (*bd, std::cmp::Reverse(*bu), std::cmp::Reverse(*bl))
                    }
                };
                if better {
                    gain = Some((density, *other, level, dc, d));
                }
            }
        }

        let Some((_, _, _, dc, d)) = gain else {
            continue;
        };

        let snapshot = selection.clone();
        selection.insert(uid, lower);
        for (u, l) in d {
            raise(selection, u, l);
        }
        let new_used = *used - freed + dc;
        if violations(store, selection, new_used, c).is_empty() {
            *used = new_used;
            return true;
        }
        *selection = snapshot;
    }
    false
}

/// Why a unit did not make it (§8).
fn drop_reason(
    store: &Store,
    selection: &Selection,
    uid: &Uid,
    salience: &BTreeMap<Uid, f32>,
) -> DropReason {
    if salience.get(uid).copied().unwrap_or(0.0) == 0.0 {
        return DropReason::LowValue;
    }
    let d = closure::delta(store, selection, *uid, Lod::L0);
    if d.len() > 1 {
        DropReason::ClosureCost
    } else {
        DropReason::Budget
    }
}
