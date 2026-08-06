//! Exact packing by branch and bound (§18.3 step 4, feature `branch-and-bound`).
//!
//! Greedy is fast and usually close; exact is slow and provably right. The difference
//! matters where a budget is tight enough that one wrong swap loses a whole cluster, which
//! is precisely where a caller most wants to know the answer is not merely plausible.
//!
//! The search is over **anchors**, not over the selection directly. Choosing a unit at a
//! level commits its whole closure, so a node's selection is always closure-complete by
//! construction and the constraints never need repairing. Because closure only ever grows a
//! selection, deciding units in canonical order can never conflict with an earlier
//! decision.
//!
//! Pruning uses the fractional relaxation, which is a sound upper bound - so a pruned
//! branch provably cannot contain the optimum.

use std::collections::BTreeMap;

use smysl_core::{Lod, Uid};
use smysl_graph::Store;

use crate::bound;
use crate::closure;
use crate::constraints::Selection;
use crate::cost::{available_levels, Estimator};

/// How hard to look before giving up.
///
/// The problem is NP-hard, so an unbounded search can run for ever on an adversarial
/// graph. Hitting the limit is not a failure: the incumbent is still a valid pack, and the
/// reported gap still bounds how far from optimal it might be.
pub const NODE_LIMIT: usize = 250_000;

/// What the search found.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Exact {
    pub selection: Selection,
    pub used: u64,
    pub value: f64,
    /// Nodes explored.
    pub nodes: usize,
    /// True when the search completed rather than hitting the node limit - which is what
    /// makes the result *proven* optimal rather than merely the best seen.
    pub proven: bool,
}

/// Search for the highest-value closure-complete selection within a budget.
///
/// `incumbent` seeds the search with a known-good selection, usually greedy's. A good
/// incumbent prunes hard, so seeding is worth far more than it costs.
#[allow(clippy::too_many_arguments)]
pub fn solve(
    store: &Store,
    scope: &[Uid],
    salience: &BTreeMap<Uid, f32>,
    e: &Estimator,
    budget: u64,
    floor: &Selection,
    incumbent: Selection,
    node_limit: usize,
    cap: impl Fn(Lod) -> Lod + Copy,
) -> Exact {
    let floor_cost = cost_of(store, floor, e);
    let mut best = Exact {
        used: cost_of(store, &incumbent, e),
        value: bound::achieved(&incumbent, salience),
        selection: incumbent,
        nodes: 0,
        proven: false,
    };

    let mut nodes = 0usize;
    let hit_limit = descend(
        store,
        scope,
        salience,
        e,
        budget,
        0,
        floor.clone(),
        floor_cost,
        &mut best,
        &mut nodes,
        node_limit,
        cap,
    );

    best.nodes = nodes;
    best.proven = !hit_limit;
    best
}

/// Returns true if the node limit was reached, meaning the search is not exhaustive.
#[allow(clippy::too_many_arguments)]
fn descend(
    store: &Store,
    scope: &[Uid],
    salience: &BTreeMap<Uid, f32>,
    e: &Estimator,
    budget: u64,
    index: usize,
    selection: Selection,
    used: u64,
    best: &mut Exact,
    nodes: &mut usize,
    node_limit: usize,
    cap: impl Fn(Lod) -> Lod + Copy,
) -> bool {
    *nodes += 1;
    if *nodes > node_limit {
        return true;
    }

    let value = bound::achieved(&selection, salience);
    // Strictly better only: on a tie the first found in canonical order wins, which keeps
    // the answer a function of the graph rather than of the search.
    if value > best.value {
        best.value = value;
        best.used = used;
        best.selection = selection.clone();
    }

    if index >= scope.len() {
        return false;
    }

    // Prune: even filling the rest of the budget fractionally cannot beat the incumbent.
    let headroom = bound::fractional(
        store,
        &selection,
        &scope[index..],
        salience,
        e,
        budget.saturating_sub(used),
        cap,
    );
    if value + headroom <= best.value {
        return false;
    }

    let uid = scope[index];
    let Some(unit) = store.get(&uid) else {
        return descend(
            store,
            scope,
            salience,
            e,
            budget,
            index + 1,
            selection,
            used,
            best,
            nodes,
            node_limit,
            cap,
        );
    };

    // Deepest first: a high-value branch found early prunes everything shallower.
    let mut levels: Vec<Lod> = available_levels(&unit.core).into_iter().map(cap).collect();
    levels.sort();
    levels.dedup();
    levels.reverse();

    let mut limited = false;
    for level in levels {
        if selection.get(&uid).is_some_and(|l| *l >= level) {
            continue;
        }
        let d = closure::delta(store, &selection, uid, level);
        if d.is_empty() {
            continue;
        }
        let mut next = selection.clone();
        for (u, l) in d {
            match next.get(&u) {
                Some(existing) if *existing >= l => {}
                _ => {
                    next.insert(u, l);
                }
            }
        }
        let next_cost = cost_of(store, &next, e);
        if next_cost > budget {
            continue;
        }
        limited |= descend(
            store,
            scope,
            salience,
            e,
            budget,
            index + 1,
            next,
            next_cost,
            best,
            nodes,
            node_limit,
            cap,
        );
    }

    // And the branch where this unit is not anchored at all. It may still arrive later as
    // somebody else's closure, which is why this is not the same as excluding it.
    limited |= descend(
        store,
        scope,
        salience,
        e,
        budget,
        index + 1,
        selection,
        used,
        best,
        nodes,
        node_limit,
        cap,
    );
    limited
}

fn cost_of(store: &Store, selection: &Selection, e: &Estimator) -> u64 {
    selection
        .iter()
        .filter_map(|(u, l)| store.get(u).map(|unit| e.unit(&unit.core, *l)))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{
        canonical_uid, KernelType, Record, RelKind, Relation, SourceKind, SourceRef, Status,
        UnitCore, UnitCoreBuilder,
    };
    use smysl_graph::{salience as compute, SalienceRequest};

    fn evidence(gist: &str, body: Option<&str>) -> UnitCore {
        let mut b = UnitCoreBuilder::new(KernelType::Evidence, gist, Status::Measured)
            .source(SourceRef::new(SourceKind::Metric, "m"));
        if let Some(t) = body {
            b = b.body(t);
        }
        b.build().unwrap()
    }

    fn identity(l: Lod) -> Lod {
        l
    }

    fn setup(store: &Store) -> (Vec<Uid>, BTreeMap<Uid, f32>) {
        let scope: Vec<Uid> = store.units().map(|(u, _)| *u).collect();
        let s = compute(store, &SalienceRequest::default());
        let local = s.renormalise(&scope);
        (scope, local)
    }

    fn run(store: &Store, budget: u64) -> Exact {
        let (scope, sal) = setup(store);
        solve(
            store,
            &scope,
            &sal,
            &Estimator::default(),
            budget,
            &Selection::new(),
            Selection::new(),
            NODE_LIMIT,
            identity,
        )
    }

    #[test]
    fn a_generous_budget_takes_everything() {
        let a = evidence("unit a", Some("a body"));
        let b = evidence("unit b", None);
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
        let store = Store::from_records(vec![Record::Unit(a), Record::Unit(b)]);

        let r = run(&store, 10_000);
        assert!(r.proven);
        assert_eq!(r.selection[&ua], Lod::L1);
        assert_eq!(r.selection[&ub], Lod::L0);
    }

    #[test]
    fn a_zero_budget_selects_nothing() {
        let store = Store::from_records(vec![Record::Unit(evidence("a", None))]);
        let r = run(&store, 0);
        assert!(r.selection.is_empty());
        assert!(r.proven);
    }

    #[test]
    fn the_result_never_exceeds_the_budget() {
        let a = evidence("unit a", Some("a moderately long body here"));
        let b = evidence("unit b", Some("another moderately long body"));
        let store = Store::from_records(vec![Record::Unit(a), Record::Unit(b)]);
        for budget in [3u64, 6, 9, 12, 20, 40] {
            let r = run(&store, budget);
            assert!(r.used <= budget, "{} over {budget}", r.used);
        }
    }

    /// Rule R survives the exact path too - closure is applied at every anchor, so an
    /// optimal pack is still a closed one.
    #[test]
    fn exact_selection_respects_rule_r() {
        let c = evidence("the claim", None);
        let uc = canonical_uid(&c);
        let r = evidence("the rebuttal", None);
        let ur = canonical_uid(&r);
        let store = Store::from_records(vec![
            Record::Unit(c),
            Record::Unit(r),
            Record::Relation(Relation::new(RelKind::Rebuts, ur, uc)),
        ]);

        for budget in [1u64, 4, 6, 8, 20, 100] {
            let out = run(&store, budget);
            if out.selection.contains_key(&uc) {
                assert!(
                    out.selection.contains_key(&ur),
                    "budget {budget}: the claim was packed unopposed"
                );
            }
        }
    }

    /// A good incumbent is never lost: the search can only improve on what it is seeded
    /// with.
    #[test]
    fn the_incumbent_is_a_floor_on_the_result() {
        let a = evidence("unit a", Some("a body"));
        let ua = canonical_uid(&a);
        let store = Store::from_records(vec![Record::Unit(a)]);
        let (scope, sal) = setup(&store);

        let seeded = Selection::from([(ua, Lod::L0)]);
        let seed_value = bound::achieved(&seeded, &sal);
        let r = solve(
            &store,
            &scope,
            &sal,
            &Estimator::default(),
            10_000,
            &Selection::new(),
            seeded,
            NODE_LIMIT,
            identity,
        );
        assert!(r.value >= seed_value);
    }

    #[test]
    fn a_floor_is_always_included() {
        let a = evidence("unit a", None);
        let ua = canonical_uid(&a);
        let b = evidence("unit b", None);
        let ub = canonical_uid(&b);
        let store = Store::from_records(vec![Record::Unit(a), Record::Unit(b)]);
        let (scope, sal) = setup(&store);

        let floor = Selection::from([(ua, Lod::L0)]);
        let r = solve(
            &store,
            &scope,
            &sal,
            &Estimator::default(),
            10_000,
            &floor,
            floor.clone(),
            NODE_LIMIT,
            identity,
        );
        assert!(r.selection.contains_key(&ua));
        assert!(r.selection.contains_key(&ub));
    }

    #[test]
    fn a_node_limit_yields_an_unproven_but_valid_result() {
        let mut records = Vec::new();
        for i in 0..12 {
            records.push(Record::Unit(evidence(
                &format!("unit {i}"),
                Some("a body worth buying"),
            )));
        }
        let store = Store::from_records(records);
        let (scope, sal) = setup(&store);

        let r = solve(
            &store,
            &scope,
            &sal,
            &Estimator::default(),
            200,
            &Selection::new(),
            Selection::new(),
            5,
            identity,
        );
        assert!(!r.proven, "a limit of five nodes cannot be exhaustive");
        assert!(r.used <= 200, "but the result is still a valid pack");
    }

    #[test]
    fn the_search_is_deterministic() {
        let a = evidence("unit a", Some("a body"));
        let b = evidence("unit b", Some("another body"));
        let store = Store::from_records(vec![Record::Unit(a), Record::Unit(b)]);
        assert_eq!(run(&store, 15), run(&store, 15));
    }

    #[test]
    fn node_count_is_reported() {
        let store = Store::from_records(vec![Record::Unit(evidence("a", None))]);
        let r = run(&store, 100);
        assert!(r.nodes > 0);
    }
}
