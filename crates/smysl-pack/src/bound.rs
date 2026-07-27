//! The fractional relaxation bound (§18.3).
//!
//! An upper bound on the value any solution could still reach from here. Two things use
//! it: branch and bound prunes with it, and greedy mode reports the gap it implies - so an
//! optimality figure is a *provable* ceiling rather than a guess.
//!
//! The relaxation drops both hard parts of the problem. Closure obligations are ignored, so
//! an item is charged only its own cost; and items may be taken fractionally. Every real
//! solution costs at least this much for the same value, so the bound is sound - and it is
//! the tightest sound bound that stays cheap to compute.

use smysl_core::{Lod, Uid};
use smysl_graph::Store;
use std::collections::BTreeMap;

use crate::constraints::Selection;
use crate::cost::{available_levels, value, Estimator};

/// One relaxed item: what upgrading a unit to its best remaining level would cost and be
/// worth, ignoring everything it would drag in.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Relaxed {
    cost: u64,
    value: f64,
}

impl Relaxed {
    fn density(&self) -> f64 {
        if self.cost == 0 {
            f64::INFINITY
        } else {
            self.value / self.cost as f64
        }
    }
}

/// The most additional value any solution could reach with `remaining` budget.
///
/// Sound because it relaxes away closure and integrality: a real acquisition costs at
/// least the item's own cost and cannot be taken in part.
pub fn fractional(
    store: &Store,
    selection: &Selection,
    scope: &[Uid],
    salience: &BTreeMap<Uid, f32>,
    e: &Estimator,
    remaining: u64,
    cap: impl Fn(Lod) -> Lod,
) -> f64 {
    if remaining == 0 {
        return 0.0;
    }

    let mut items: Vec<Relaxed> = Vec::new();
    for uid in scope {
        let Some(unit) = store.get(uid) else { continue };
        let held = selection.get(uid).copied();
        let s = salience.get(uid).copied().unwrap_or(0.0);

        // Every level this unit could still be raised to, as an incremental purchase.
        let upgrades: Vec<(u64, f64)> = available_levels(&unit.core)
            .into_iter()
            .map(&cap)
            .filter(|l| held.is_none_or_lower(*l))
            .map(|l| {
                let cost = match held {
                    Some(h) => e.upgrade(&unit.core, h, l),
                    None => e.unit(&unit.core, l),
                };
                let gained = value(s, l) - held.map(|h| value(s, h)).unwrap_or(0.0);
                (cost, gained)
            })
            .filter(|(c, v)| *c > 0 && *v > 0.0)
            .collect();
        if upgrades.is_empty() {
            continue;
        }

        // The *best density* level, not the best value one. A real solution takes this
        // unit at some level whose density is at most this, so charging the unit at its
        // best density and letting it absorb up to its largest possible spend dominates
        // whatever it actually does. Taking only the deepest level would be unsound: a
        // cheaper level often has the higher density, and a real pack may well buy it.
        let best_density = upgrades
            .iter()
            .map(|(c, v)| v / *c as f64)
            .fold(0.0f64, f64::max);
        let capacity = upgrades.iter().map(|(c, _)| *c).max().unwrap_or(0);
        if capacity == 0 || best_density <= 0.0 {
            continue;
        }
        items.push(Relaxed {
            cost: capacity,
            value: capacity as f64 * best_density,
        });
    }

    // Fill by density, taking a fraction of whatever straddles the boundary.
    items.sort_by(|a, b| {
        b.density()
            .partial_cmp(&a.density())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.cost.cmp(&b.cost))
    });

    let mut left = remaining;
    let mut total = 0.0f64;
    for item in items {
        if left == 0 {
            break;
        }
        if item.cost <= left {
            total += item.value;
            left -= item.cost;
        } else {
            total += item.value * (left as f64 / item.cost as f64);
            break;
        }
    }
    total
}

/// The optimality gap a bound implies: the fraction of the ceiling not achieved.
///
/// Zero means proven optimal. Reported in every `packinfo`, so a caller knows how far from
/// optimal a pack might be rather than having to assume either way.
pub fn gap(achieved: f64, headroom: f64) -> f32 {
    let ceiling = achieved + headroom;
    if ceiling <= 0.0 {
        return 0.0;
    }
    smysl_core::quantise((headroom / ceiling) as f32)
}

/// The value a selection has achieved.
pub fn achieved(selection: &Selection, salience: &BTreeMap<Uid, f32>) -> f64 {
    selection
        .iter()
        .map(|(u, l)| value(salience.get(u).copied().unwrap_or(0.0), *l))
        .sum()
}

/// `Option<Lod>` helper: whether a candidate level would actually be an upgrade.
trait HeldLevel {
    fn is_none_or_lower(&self, candidate: Lod) -> bool;
}

impl HeldLevel for Option<Lod> {
    fn is_none_or_lower(&self, candidate: Lod) -> bool {
        match self {
            Some(held) => *held < candidate,
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{
        canonical_uid, KernelType, Record, SourceKind, SourceRef, Status, UnitCore, UnitCoreBuilder,
    };

    fn evidence(gist: &str, body: Option<&str>) -> UnitCore {
        let mut b = UnitCoreBuilder::new(KernelType::Evidence, gist, Status::Measured)
            .source(SourceRef::new(SourceKind::Metric, "m"));
        if let Some(t) = body {
            b = b.body(t);
        }
        b.build().unwrap()
    }

    fn fixture() -> (Store, Vec<Uid>, BTreeMap<Uid, f32>) {
        let a = evidence("unit a", Some("a body for unit a"));
        let b = evidence("unit b", None);
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
        let store = Store::from_records(vec![Record::Unit(a), Record::Unit(b)]);
        let sal = BTreeMap::from([(ua, 1.0f32), (ub, 0.5f32)]);
        (store, vec![ua, ub], sal)
    }

    fn identity(l: Lod) -> Lod {
        l
    }

    #[test]
    fn no_budget_means_no_headroom() {
        let (store, scope, sal) = fixture();
        let h = fractional(
            &store,
            &Selection::new(),
            &scope,
            &sal,
            &Estimator::default(),
            0,
            identity,
        );
        assert_eq!(h, 0.0);
    }

    #[test]
    fn a_generous_budget_bounds_everything_available() {
        let (store, scope, sal) = fixture();
        let h = fractional(
            &store,
            &Selection::new(),
            &scope,
            &sal,
            &Estimator::default(),
            10_000,
            identity,
        );
        assert!(h > 0.0);
    }

    /// The bound must be an upper bound, not an estimate. Anything a real pack achieves
    /// has to sit under it, or branch and bound would prune the optimum.
    #[test]
    fn the_bound_dominates_what_a_real_pack_achieves() {
        use crate::{pack, PackRequest};
        use smysl_graph::{salience as compute, SalienceRequest};

        let (store, scope, _) = fixture();
        let s = compute(&store, &SalienceRequest::default());
        let local = s.renormalise(&scope);

        for budget in [5u64, 10, 20, 50, 200] {
            let p = pack(&store, &s, &PackRequest::budget(budget)).unwrap();
            let got = achieved(&p.selection, &local);
            let bound = fractional(
                &store,
                &Selection::new(),
                &scope,
                &local,
                &Estimator::default(),
                budget,
                identity,
            );
            assert!(
                got <= bound + 1e-9,
                "budget {budget}: achieved {got} exceeds the bound {bound}"
            );
        }
    }

    #[test]
    fn the_bound_shrinks_as_the_selection_grows() {
        let (store, scope, sal) = fixture();
        let e = Estimator::default();
        let empty = fractional(&store, &Selection::new(), &scope, &sal, &e, 100, identity);
        let partial = Selection::from([(scope[0], Lod::L1)]);
        let after = fractional(&store, &partial, &scope, &sal, &e, 100, identity);
        assert!(after < empty);
    }

    #[test]
    fn a_full_selection_has_no_headroom() {
        let (store, scope, sal) = fixture();
        let full = Selection::from([(scope[0], Lod::L1), (scope[1], Lod::L0)]);
        let h = fractional(
            &store,
            &full,
            &scope,
            &sal,
            &Estimator::default(),
            10_000,
            identity,
        );
        assert_eq!(h, 0.0);
    }

    #[test]
    fn a_gap_of_zero_means_proven_optimal() {
        assert_eq!(gap(10.0, 0.0), 0.0);
        assert_eq!(gap(0.0, 0.0), 0.0);
        assert!(gap(10.0, 10.0) > 0.4);
        assert!(gap(1.0, 99.0) > 0.9);
    }

    #[test]
    fn the_gap_is_quantised_and_bounded() {
        for (a, h) in [(1.0, 0.5), (10.0, 3.0), (0.5, 0.25)] {
            let g = gap(a, h);
            assert!((0.0..=1.0).contains(&g));
            assert_eq!(smysl_core::quantise(g), g);
        }
    }

    #[test]
    fn achieved_sums_the_selection() {
        let (_, scope, sal) = fixture();
        let sel = Selection::from([(scope[0], Lod::L1)]);
        assert_eq!(achieved(&sel, &sal), value(1.0, Lod::L1));
        assert_eq!(achieved(&Selection::new(), &sal), 0.0);
    }

    #[test]
    fn a_capped_level_lowers_the_bound() {
        let (store, scope, sal) = fixture();
        let e = Estimator::default();
        let uncapped = fractional(&store, &Selection::new(), &scope, &sal, &e, 1000, identity);
        let capped = fractional(&store, &Selection::new(), &scope, &sal, &e, 1000, |_| {
            Lod::L0
        });
        assert!(capped < uncapped);
    }

    #[test]
    fn the_bound_is_deterministic() {
        let (store, scope, sal) = fixture();
        let e = Estimator::default();
        let a = fractional(&store, &Selection::new(), &scope, &sal, &e, 37, identity);
        let b = fractional(&store, &Selection::new(), &scope, &sal, &e, 37, identity);
        assert_eq!(a, b);
    }
}
