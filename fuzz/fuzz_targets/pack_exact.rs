//! Exact packing against brute force.
//!
//! Two implementations of the same question, checked against each other. Brute force
//! enumerates every level assignment, drops the ones that break a constraint or the budget,
//! and takes the best remaining. It is obviously correct and obviously too slow, which is
//! what makes it an oracle rather than a second opinion.
//!
//! This matters more than "exact is at least as good as greedy". That comparison can only
//! catch exact being *worse*; it says nothing about whether either reaches the optimum, so
//! two implementations sharing one wrong idea about feasibility agree with each other
//! perfectly. Enumeration shares nothing with the branch-and-bound search except the
//! constraint checker.
//!
//! `smysl-pack/tests/exact.rs` already does this over a fixed seed. What changes here is
//! that coverage feedback picks the graphs.

#![no_main]
use std::collections::{BTreeMap, BTreeSet};

use libfuzzer_sys::fuzz_target;
use smysl_core::{Lod, PackMode, Uid};
use smysl_fuzz::{generate, Choices};
use smysl_graph::{salience, SalienceRequest, Store};
use smysl_pack::{bound, pack, verify, violations, Constraints, Estimator, PackRequest, Selection};

/// Above this many assignments the case is skipped rather than enumerated.
///
/// Lower than the `1 << 16` the seeded gate uses. That one runs 120 rounds and can afford a
/// slow oracle; this runs thousands per second, and a fuzzer that spends its budget inside
/// the oracle explores nothing. The generator is bounded to match, so most inputs land
/// under the limit rather than being generated and thrown away.
const ENUMERATION_LIMIT: u64 = 1 << 12;

fn search_space(store: &Store, scope: &[Uid]) -> u64 {
    scope
        .iter()
        .filter_map(|u| store.get(u))
        .map(|unit| smysl_pack::available_levels(&unit.core).len() as u64 + 1)
        .try_fold(1u64, |a, b| a.checked_mul(b))
        .unwrap_or(u64::MAX)
}

/// The best value any feasible selection can reach, found by looking at all of them.
fn brute_force(
    store: &Store,
    scope: &[Uid],
    salience: &BTreeMap<Uid, f32>,
    budget: u64,
) -> Option<f64> {
    let e = Estimator::default();
    let choices: Vec<Vec<Option<Lod>>> = scope
        .iter()
        .map(|u| {
            let mut v: Vec<Option<Lod>> = vec![None];
            if let Some(unit) = store.get(u) {
                v.extend(
                    smysl_pack::available_levels(&unit.core)
                        .into_iter()
                        .map(Some),
                );
            }
            v
        })
        .collect();

    let total: u64 = choices
        .iter()
        .map(|c| c.len() as u64)
        .try_fold(1u64, |a, b| a.checked_mul(b))?;
    if total > ENUMERATION_LIMIT {
        return None;
    }

    let constraints = Constraints {
        pinned: BTreeSet::new(),
        budget,
    };
    let mut best = 0.0f64;
    for mut n in 0..total {
        let mut selection = Selection::new();
        for (i, options) in choices.iter().enumerate() {
            let pick = (n % options.len() as u64) as usize;
            n /= options.len() as u64;
            if let Some(level) = options[pick] {
                selection.insert(scope[i], level);
            }
        }
        let used: u64 = selection
            .iter()
            .filter_map(|(u, l)| store.get(u).map(|unit| e.unit(&unit.core, *l)))
            .sum();
        if used > budget {
            continue;
        }
        if !violations(store, &selection, used, &constraints).is_empty() {
            continue;
        }
        let v = bound::achieved(&selection, salience);
        if v > best {
            best = v;
        }
    }
    Some(best)
}

fuzz_target!(|data: &[u8]| {
    let mut c = Choices::new(data);
    // Small, so the oracle stays affordable. The interesting behaviour is in how
    // constraints interact, not in how many units there are.
    let store = generate(&mut c, 6);

    let scope: Vec<Uid> = store.units().map(|(u, _)| *u).collect();
    if scope.is_empty() {
        return;
    }
    let report = salience(&store, &SalienceRequest::default());
    let local = report.renormalise(&scope);
    if search_space(&store, &scope) > ENUMERATION_LIMIT {
        return;
    }

    for budget in [10u64, 25, 90, 250] {
        let req = PackRequest::budget(budget).exact();
        let p = pack(&store, &report, &req).expect("no focus, so the floor is empty");

        // Whatever mode it settled on, the result has to be a legal pack.
        let v = verify(&store, &p, &req);
        assert!(
            v.is_empty(),
            "budget {budget}: {}",
            v.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")
        );

        // Exact may decline — above its own threshold it reports `SMY-W202` and falls back.
        // Comparing a greedy result against an optimality oracle would be a false failure.
        if p.info.optimality.mode != PackMode::Exact {
            continue;
        }

        let Some(best) = brute_force(&store, &scope, &local, budget) else {
            continue;
        };
        let got = bound::achieved(&p.selection, &local);

        assert!(
            got >= best - 1e-9,
            "budget {budget}: exact reached {got}, brute force proves {best} is available"
        );
        // And it cannot exceed the true optimum — that would mean the two disagree about
        // what counts as feasible, which is the more dangerous direction: a pack that looks
        // better than possible is one that broke a constraint the verifier also missed.
        assert!(
            got <= best + 1e-9,
            "budget {budget}: exact claims {got}, above the true optimum {best}"
        );

        // A proven-optimal pack reports no gap.
        assert!(
            p.info.optimality.gap.abs() < 1e-6,
            "budget {budget}: exact proved optimality but reports a gap of {}",
            p.info.optimality.gap
        );
    }
});
