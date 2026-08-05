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
#[non_exhaustive]
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
/// The greedy's choice, as one orderable value.
///
/// Extracted so that the scan and the ordered structure that replaces it cannot disagree.
/// The order used to live inline in a comparison, and the risk in replacing the scan was
/// that a heap would reproduce three of its four terms — producing packs that are legal,
/// deterministic and monotone in budget, and *different*. Nothing in the suite covered the
/// salience term, because no corpus fixture ties on density without also tying on salience.
/// One implementation of the order removes that question rather than testing around it.
///
/// `Ord` is derived, so the field order **is** the tie-break: density, then salience, then
/// uid descending, then level descending, then candidate index ascending. The last term is
/// what makes the order total — the scan kept the first candidate it met among equals,
/// because it replaced `best` only on a strictly-greater key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Choice {
    density: Ordered,
    salience: Ordered,
    uid: std::cmp::Reverse<Uid>,
    level: std::cmp::Reverse<Lod>,
    index: std::cmp::Reverse<usize>,
}

/// A finite float that can live in an ordered structure.
///
/// `total_cmp` rather than `partial_cmp().unwrap()`: densities are finite by construction
/// here, and a panic inside a packer because one was not is a worse failure than an
/// arbitrary-but-consistent order.
#[derive(Debug, Clone, Copy)]
struct Ordered(f64);

impl PartialEq for Ordered {
    fn eq(&self, other: &Self) -> bool {
        self.0.total_cmp(&other.0) == core::cmp::Ordering::Equal
    }
}
impl Eq for Ordered {}
impl PartialOrd for Ordered {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Ordered {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl Choice {
    fn new(density: f64, salience: f32, uid: Uid, level: Lod, index: usize) -> Choice {
        Choice {
            density: Ordered(density),
            salience: Ordered(salience as f64),
            uid: core::cmp::Reverse(uid),
            level: core::cmp::Reverse(level),
            index: core::cmp::Reverse(index),
        }
    }
}

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
    // Only the exact path needs the floor kept separately, so without that feature this
    // clone would be dead code rather than merely unused.
    #[cfg(feature = "branch-and-bound")]
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

    // Hoisted above the selection so the whole-scope path below can be *verified* rather
    // than argued for. It depends only on the request, so there was never a reason for it to
    // be built after the fact.
    let constraints = Constraints {
        pinned: req.focus.clone(),
        budget: req.budget,
    };

    // --- 2a. everything fits ----------------------------------------------
    //
    // The greedy below is O(n^2) *by construction*: it runs one round per unit admitted and
    // re-evaluates every remaining candidate each round to pick the best. Measured on a
    // synthetic store, `closure::delta` was called 7.5 million times for 4 000 units, scaling
    // exactly 4.0x per doubling.
    //
    // That is inherent to picking a global best each round, and worth paying when the budget
    // actually binds. It is worth nothing at all when the budget admits the whole scope: the
    // greedy then spends quadratic time arriving at the answer this block reaches in one
    // pass. Which is why the pathology showed up in the *easy* case — ample budget quadratic,
    // binding budget linear.
    //
    // Taking everything at its top level is not a heuristic here, it is optimal: value is
    // monotonic in level, so no unit is worth less for being fuller, and every closure
    // constraint (C1-C7, including rule R) is trivially satisfied by a selection that omits
    // nothing. If it fits, there is nothing left to trade.
    let everything: Selection = scope
        .iter()
        .filter_map(|u| {
            let unit = store.get(u)?;
            let top = available_levels(&unit.core).into_iter().map(&cap).max()?;
            Some((*u, top))
        })
        .collect();
    let everything_cost = cost_of(store, &everything, &req.estimator);

    let mut used = floor_cost;
    if everything_cost <= req.budget
        && violations(store, &everything, everything_cost, &constraints).is_empty()
    {
        // Every unit reads `earned on density`, and that is the honest answer here rather
        // than a shortcut. The C-reasons mean "this was dragged in by something else's
        // obligation under budget pressure" — and under this path there was no pressure and
        // nothing was dragged: the whole scope fit. Attributing `C3 rebuts …` would describe
        // a trade that did not happen.
        //
        // The greedy's attribution cannot be reproduced without the greedy in any case: it
        // depends on admission *order*, giving `Density` to whichever unit it chose and a
        // C-reason to what that choice pulled along, first writer winning. Order is exactly
        // what this path does not compute.
        for (u, l) in &everything {
            raise(&mut selection, *u, *l);
            why.entry(*u).or_insert(closure::Reason::Density);
        }
        used = everything_cost;
    }

    // --- 2b. greedy by density --------------------------------------------
    //
    // One memo for the whole run. The greedy is O(n^2) in *candidate evaluations* by
    // construction — one round per unit admitted, every remaining candidate re-weighed each
    // round — and that is what makes it pick a global best. What it does not need is to
    // re-walk the graph for each of those evaluations: an obligation is a pure function of
    // `(uid, level)`, so there are at most two distinct answers per unit and the greedy was
    // computing 7.5 million of them for 4 001 units.
    //
    // Memoising leaves every choice identical and removes the walk from the inner loop.
    let mut needs = closure::Needs::new();

    // The candidate set, fixed for the run: every (unit, level) the greedy could ever weigh.
    let candidates: Vec<(Uid, Lod)> = scope
        .iter()
        .filter_map(|u| store.get(u).map(|unit| (*u, unit)))
        .flat_map(|(u, unit)| {
            available_levels(&unit.core)
                .into_iter()
                .map(move |l| (u, cap(l)))
        })
        .collect();

    // Which candidates a unit's level affects.
    //
    // This is what makes invalidation *exact* rather than a heuristic. A candidate's cost and
    // value depend on the selection only through the units in its own obligation: `delta`
    // filters the obligation by what is already held, and `weigh` charges each member against
    // the level currently held for it. Nothing outside the obligation can change either
    // number. So raising unit `u` can only disturb candidates whose obligation mentions `u`,
    // and every other cached figure stays exactly as valid as it was.
    let mut affected: BTreeMap<Uid, Vec<usize>> = BTreeMap::new();
    for (i, (u, l)) in candidates.iter().enumerate() {
        for x in needs.required(store, *u, *l).keys() {
            affected.entry(*x).or_default().push(i);
        }
    }

    // `(dc, dv)` per candidate; `None` means it must be recomputed before use. A zero cost
    // stands for "nothing left to buy", which is what both an empty delta and a free one meant
    // to the original loop — each simply moved on.
    let mut weighed: Vec<Option<(u64, f64)>> = vec![None; candidates.len()];

    // The scan is gone. What replaced it, and why each piece is needed:
    //
    // `ready` holds every candidate whose cached pricing is current *and* affordable, keyed
    // by `Choice`, so the round's winner is `ready.pop_last()` rather than a walk over
    // everything. `dirty` is the set whose pricing a selection change invalidated — exactly
    // `affected[u]` for each unit whose level moved. `parked` holds candidates priced
    // correctly but unaffordable at the current `used`.
    //
    // Parking is the subtle part, and it is why a naive lazy greedy is unsound here. `used`
    // only grows, so a candidate that cannot be afforded now can never be afforded later —
    // *unless its marginal cost falls*, which happens when something in its obligation gets
    // selected and is therefore already paid for. That is precisely a dirty event. So a
    // parked candidate is reconsidered when, and only when, it is dirtied, and nothing has to
    // sweep the parked set per round.
    //
    // Affordability is checked at pop rather than eagerly, because `used` changes every round
    // and re-testing every ready entry would reintroduce the scan. Each pop that parks is
    // charged against the insert or dirty event that put the entry there, so the work is
    // amortised rather than per-round.
    let mut ready: BTreeSet<Choice> = BTreeSet::new();
    let mut parked: BTreeSet<usize> = BTreeSet::new();
    let mut placed: Vec<Option<Choice>> = vec![None; candidates.len()];
    let mut dirty: Vec<usize> = (0..candidates.len()).collect();

    loop {
        // Reprice what a selection change invalidated, and place it.
        for i in std::mem::take(&mut dirty) {
            if let Some(old) = placed[i].take() {
                ready.remove(&old);
            }
            parked.remove(&i);

            let (uid, level) = candidates[i];
            // Satisfied candidates leave the running entirely. A unit's own candidates are
            // in `affected[uid]`, because a candidate's obligation always contains its own
            // unit, so raising it dirties them and they land here.
            if selection.get(&uid).is_some_and(|l| *l >= level) {
                continue;
            }
            let d = needs.delta(store, &selection, uid, level);
            let w = if d.is_empty() {
                (0, 0.0)
            } else {
                weigh(store, &selection, &d, &local, &req.estimator)
            };
            weighed[i] = Some(w);
            let (dc, dv) = w;
            if dc == 0 {
                continue;
            }
            if used + dc > req.budget {
                parked.insert(i);
                continue;
            }
            let c = Choice::new(
                dv / dc as f64,
                local.get(&uid).copied().unwrap_or(0.0),
                uid,
                level,
                i,
            );
            placed[i] = Some(c);
            ready.insert(c);
        }

        // Take the best affordable candidate, parking anything the growing `used` has put
        // out of reach since it was placed.
        let winner = loop {
            let Some(c) = ready.pop_last() else {
                break None;
            };
            let i = c.index.0;
            placed[i] = None;
            let (uid, level) = candidates[i];
            let (dc, _) = weighed[i].expect("placed candidates are priced");
            if used + dc > req.budget {
                parked.insert(i);
                continue;
            }
            break Some((uid, level, dc, i));
        };

        let Some((uid, level, dc, _i)) = winner else {
            break;
        };
        let d = needs.delta(store, &selection, uid, level);
        for (u, l) in d {
            // Only a level that actually moved can invalidate anything. `raise` is a no-op
            // when the selection already holds `u` at or above `l`, and treating that as a
            // change would dirty candidates whose figures are still perfectly good.
            let moved = selection.get(&u).map(|held| *held < l).unwrap_or(true);
            raise(&mut selection, u, l);
            if moved {
                // Dirty rather than merely uncached: the entry must also leave `ready`,
                // because its key is now wrong and a stale key at the top of the order would
                // be chosen on figures that no longer hold.
                for i in affected.get(&u).map(Vec::as_slice).unwrap_or(&[]) {
                    weighed[*i] = None;
                    dirty.push(*i);
                }
            }
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

    // Step 3 of §18.3 was a local-improvement pass — downgrade the least valuable depth,
    // spend what that frees on breadth. Removed in 0.8.0 because it was measured and it
    // lost: over 28 000 generated packs it changed 26, and 22 of those 26 were *worse* by
    // the value function it exists to maximise. It fired rarely enough (0.09%) to escape
    // every fixture, which is why two earlier measurements read it as harmless — 0.3.0
    // found turning it off changed runtime by under 1%, and mutation testing found
    // `improve -> false` survives. Neither could see that the packs got better.

    // The search that found it lived in `tests/constraints.rs` and went with it: with the
    // pass gone it would compare two identical configurations, which is the shape of test
    // this project keeps deleting. The numbers are in the 0.8.0 changelog.

    // --- 4. exact refinement (feature `branch-and-bound`) -------------------
    let mut report = Report::new();
    // Nothing reassigns this unless the exact search runs.
    #[cfg_attr(not(feature = "branch-and-bound"), allow(unused_mut))]
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
    info.optimality = Optimality::new(
        req.mode,
        bound::gap(bound::achieved(&selection, &local), headroom),
    );

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

#[cfg(test)]
mod ordering_tests {
    use super::*;

    /// `Choice` is ordered by density, then salience, then uid descending, then level
    /// descending, then index ascending — and the derived `Ord` makes the field order *be*
    /// that rule. Mutation testing found this untested: `Ordered::eq`, `partial_cmp` and
    /// `cmp` could each be replaced with a constant and every test still passed.
    ///
    /// That is the sharper half of the finding. The type was introduced in 0.6.0 with the
    /// argument that one implementation of the order "removes the question rather than
    /// testing around it" — which was right about consistency and wrong about correctness.
    /// One implementation used by both callers is still an implementation nothing checks.
    #[test]
    fn choices_order_by_density_first() {
        let u = Uid::from_bytes([1; 32]);
        let lo = Choice::new(1.0, 0.5, u, Lod::L0, 0);
        let hi = Choice::new(2.0, 0.1, u, Lod::L0, 0);
        assert!(hi > lo, "a denser candidate wins regardless of salience");
    }

    #[test]
    fn salience_breaks_a_density_tie() {
        let u = Uid::from_bytes([1; 32]);
        let dull = Choice::new(1.0, 0.1, u, Lod::L0, 0);
        let keen = Choice::new(1.0, 0.9, u, Lod::L0, 0);
        assert!(
            keen > dull,
            "the salience term never decides in the corpus, so only this says it works"
        );
    }

    #[test]
    fn a_lower_uid_wins_when_density_and_salience_tie() {
        let low = Choice::new(1.0, 0.5, Uid::from_bytes([1; 32]), Lod::L0, 0);
        let high = Choice::new(1.0, 0.5, Uid::from_bytes([9; 32]), Lod::L0, 0);
        assert!(
            low > high,
            "uid descends, so the smaller uid is the greater Choice"
        );
    }

    #[test]
    fn an_earlier_candidate_wins_when_everything_else_ties() {
        let u = Uid::from_bytes([1; 32]);
        let first = Choice::new(1.0, 0.5, u, Lod::L0, 0);
        let later = Choice::new(1.0, 0.5, u, Lod::L0, 7);
        assert!(
            first > later,
            "the scan kept the first candidate among equals; the heap must agree"
        );
    }

    #[test]
    fn ordered_compares_by_value_and_not_by_constant() {
        assert!(Ordered(2.0) > Ordered(1.0));
        assert!(Ordered(1.0) < Ordered(2.0));
        assert_eq!(Ordered(1.0), Ordered(1.0));
        assert_ne!(Ordered(1.0), Ordered(2.0));
        // `total_cmp` rather than `partial_cmp().unwrap()`: -0.0 and 0.0 are distinct to it,
        // which is fine, and no input can panic.
        assert!(Ordered(f64::MAX) > Ordered(0.0));
    }

    /// `is_optimal` could be replaced with `true`, with `false`, or have its `&&` flipped,
    /// and nothing noticed — for the function that tells a caller whether their pack is
    /// provably the best available.
    #[test]
    fn optimality_needs_both_exact_mode_and_no_gap() {
        let mk = |mode, gap| Optimality::new(mode, gap);
        // Built by hand rather than by packing something: the question is what
        // `is_optimal` reads off `info`, and reaching it through a real pack would test the
        // packer instead.
        let pack = |o| {
            let mut info = PackInfo::new(0, 0, "");
            info.optimality = o;
            Pack {
                selection: Selection::new(),
                info,
                why: BTreeMap::new(),
                report: Report::default(),
            }
        };
        assert!(pack(mk(PackMode::Exact, 0.0)).is_optimal());
        assert!(
            !pack(mk(PackMode::Greedy, 0.0)).is_optimal(),
            "a greedy pack with no measured gap is not *proved* optimal"
        );
        assert!(
            !pack(mk(PackMode::Exact, 0.1)).is_optimal(),
            "exact mode that gave up with a gap has not proved anything"
        );
        assert!(!pack(mk(PackMode::Greedy, 0.1)).is_optimal());
    }
}
