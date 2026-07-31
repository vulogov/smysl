//! The SM-P10 gate.
//!
//! Exact mode is compared against **brute force** - every level assignment enumerated,
//! filtered to the feasible ones, and the best taken. Brute force is obviously correct and
//! obviously too slow, which is exactly what makes it a good oracle.
//!
//! The search space is `∏(levels(u) + 1)`, so enumeration is only tractable where the
//! store is small or mostly gist-only. Rounds above [`ENUMERATION_LIMIT`] are skipped and
//! counted, so the coverage claim is what actually ran rather than what was hoped for.

#![cfg(feature = "branch-and-bound")]

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::{
    canonical_uid, Contention, ContentionId, Detected, DetectionKind, Hlc, KernelType, Lod,
    PackMode, Record, RelKind, Relation, SourceKind, SourceRef, Status, Uid, UnitCore,
    UnitCoreBuilder,
};
use smysl_graph::{salience, SalienceRequest, Store};
use smysl_pack::{bound, pack, verify, violations, Constraints, Estimator, PackRequest, Selection};

/// Above this many assignments a round is skipped rather than enumerated.
///
/// Brute force is the oracle, so it has to run often enough to mean something and fast
/// enough to stay in CI. This is the line between the two; rounds above it are skipped and
/// counted, so the coverage claims below are about what actually ran.
const ENUMERATION_LIMIT: u64 = 1 << 16;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }
}

/// Mostly gist-only units, so the enumeration stays tractable up to twenty of them.
fn generate(rng: &mut Rng, size: usize) -> Store {
    let mut records: Vec<Record> = Vec::new();
    let mut uids: Vec<Uid> = Vec::new();

    for _ in 0..size {
        let gist = format!("unit {}", rng.next() % 10_000);
        let core: UnitCore = if uids.is_empty() || rng.chance(2) {
            let mut b = UnitCoreBuilder::new(KernelType::Evidence, gist, Status::Measured)
                .source(SourceRef::new(SourceKind::Metric, "m"));
            if rng.chance(4) {
                b = b.body("a body worth buying when there is room".to_string());
                // A detail as well, sometimes, so L2 enters the search space at all. Without
                // one `available_levels` tops out at L1, and this gate — the only place
                // branch-and-bound is checked against brute force — never compared the two
                // on a three-level unit. Gated on the body, because an L2 with no L1 beneath
                // it is not a shape the packer can be handed.
                if rng.chance(2) {
                    b = b.detail("a detail worth buying when there is plenty of room".to_string());
                }
            }
            b.build().unwrap()
        } else {
            let n = 1 + rng.below(uids.len().min(2));
            let grounds: Vec<Uid> = (0..n).map(|_| uids[rng.below(uids.len())]).collect();
            let mut b =
                UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative).grounds(grounds);
            if rng.chance(4) {
                b = b.body("a body worth buying when there is room".to_string());
                // A detail as well, sometimes, so L2 enters the search space at all. Without
                // one `available_levels` tops out at L1, and this gate — the only place
                // branch-and-bound is checked against brute force — never compared the two
                // on a three-level unit. Gated on the body, because an L2 with no L1 beneath
                // it is not a shape the packer can be handed.
                if rng.chance(2) {
                    b = b.detail("a detail worth buying when there is plenty of room".to_string());
                }
            }
            b.build().unwrap()
        };
        uids.push(canonical_uid(&core));
        records.push(Record::Unit(core));
    }

    for _ in 0..size / 2 {
        if uids.len() < 2 {
            break;
        }
        let from = uids[rng.below(uids.len())];
        let to = uids[rng.below(uids.len())];
        if from != to {
            records.push(Record::Relation(Relation::new(RelKind::Rebuts, from, to)));
        }
    }
    if uids.len() >= 2 && rng.chance(2) {
        let a = uids[rng.below(uids.len())];
        let b = uids[rng.below(uids.len())];
        if a != b {
            records.push(Record::Contention(Contention::new(
                ContentionId::new("k/x").unwrap(),
                a,
                vec![a, b],
                Detected {
                    kind: DetectionKind::SupersessionFork,
                    ts: Hlc::new(0, 0, smysl_core::AgentId::new("tool:t").unwrap()),
                },
            )));
        }
    }
    Store::from_records(records)
}

fn setup(store: &Store) -> (Vec<Uid>, BTreeMap<Uid, f32>) {
    let scope: Vec<Uid> = store.units().map(|(u, _)| *u).collect();
    let s = salience(store, &SalienceRequest::default());
    let local = s.renormalise(&scope);
    (scope, local)
}

/// How many assignments a store implies.
fn search_space(store: &Store, scope: &[Uid]) -> u64 {
    scope
        .iter()
        .filter_map(|u| store.get(u))
        .map(|unit| smysl_pack::available_levels(&unit.core).len() as u64 + 1)
        .try_fold(1u64, |a, b| a.checked_mul(b))
        .unwrap_or(u64::MAX)
}

/// The oracle: enumerate every assignment, keep the feasible ones, take the best.
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

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// Exact mode reaches the value brute force proves is available.
#[test]
fn exact_matches_brute_force() {
    let mut rng = Rng(0x2026_0726_2001);
    let mut compared = 0usize;
    let mut skipped = 0usize;

    for round in 0..120 {
        let n = 2 + rng.below(9);
        let store = generate(&mut rng, n);
        let (scope, sal) = setup(&store);
        if scope.is_empty() {
            continue;
        }

        for budget in [6u64, 20, 60] {
            let Some(optimum) = brute_force(&store, &scope, &sal, budget) else {
                skipped += 1;
                continue;
            };
            let s = salience(&store, &SalienceRequest::default());
            let p = pack(&store, &s, &PackRequest::budget(budget).exact()).unwrap();
            let got = bound::achieved(&p.selection, &sal);

            assert!(
                (got - optimum).abs() < 1e-9,
                "round {round} budget {budget}: exact found {got}, brute force {optimum}"
            );
            compared += 1;
        }
    }

    assert!(
        compared > 150,
        "only {compared} comparisons ran ({skipped} skipped)"
    );
}

/// Up to twenty units, where the enumeration is tractable.
#[test]
fn exact_matches_brute_force_on_larger_graphs() {
    let mut rng = Rng(0x2026_0726_2002);
    let mut compared = 0usize;

    for round in 0..40 {
        let n = 12 + rng.below(9); // 12..20
        let store = generate(&mut rng, n);
        let (scope, sal) = setup(&store);
        if search_space(&store, &scope) > ENUMERATION_LIMIT {
            continue;
        }

        for budget in [25u64, 90] {
            let Some(optimum) = brute_force(&store, &scope, &sal, budget) else {
                continue;
            };
            let s = salience(&store, &SalienceRequest::default());
            let p = pack(&store, &s, &PackRequest::budget(budget).exact()).unwrap();
            let got = bound::achieved(&p.selection, &sal);
            assert!(
                (got - optimum).abs() < 1e-9,
                "round {round} n={n} budget {budget}: exact {got}, brute force {optimum}"
            );
            compared += 1;
        }
    }

    assert!(compared > 8, "only {compared} large-graph comparisons ran");
}

/// The other half of the gate: the reported gap must bound the true gap, on graphs too
/// large to enumerate as well as small ones. A gap that understated the shortfall would
/// be worse than none.
#[test]
fn the_reported_gap_bounds_the_true_gap() {
    let mut rng = Rng(0x2026_0726_2003);
    let mut checked = 0usize;

    for round in 0..100 {
        let n = 3 + rng.below(6);
        let store = generate(&mut rng, n);
        let (scope, sal) = setup(&store);
        let Some(optimum) = brute_force(&store, &scope, &sal, 60) else {
            continue;
        };
        if optimum <= 0.0 {
            continue;
        }

        let s = salience(&store, &SalienceRequest::default());
        let greedy = pack(&store, &s, &PackRequest::budget(60)).unwrap();
        let got = bound::achieved(&greedy.selection, &sal);

        let true_gap = ((optimum - got) / optimum) as f32;
        let reported = greedy.info.optimality.gap;
        assert!(
            reported + 1.0 / 1024.0 >= true_gap,
            "round {round}: reported gap {reported} understates the true gap {true_gap}"
        );
        checked += 1;
    }

    assert!(checked > 40, "only {checked} rounds ran");
}

/// Exact is never worse than greedy. That is the only reason to pay for it.
#[test]
fn exact_is_never_worse_than_greedy() {
    let mut rng = Rng(0x2026_0726_2004);
    let mut improved = 0usize;

    for round in 0..80 {
        let n = 3 + rng.below(7);
        let store = generate(&mut rng, n);
        let (_, sal) = setup(&store);
        let s = salience(&store, &SalienceRequest::default());

        for budget in [10u64, 30, 80] {
            let greedy = pack(&store, &s, &PackRequest::budget(budget)).unwrap();
            let exact = pack(&store, &s, &PackRequest::budget(budget).exact()).unwrap();
            let g = bound::achieved(&greedy.selection, &sal);
            let x = bound::achieved(&exact.selection, &sal);
            assert!(
                x >= g - 1e-9,
                "round {round} budget {budget}: exact {x} < greedy {g}"
            );
            if x > g + 1e-9 {
                improved += 1;
            }
        }
    }

    // Not an assertion about how often - only that the fixture is capable of showing a
    // difference at all, or the comparison would be vacuous.
    assert!(
        improved > 0,
        "greedy was optimal every time; the generator never exercises the exact path"
    );
}

/// An exact pack is still a *legal* pack. Optimality never buys its way past a constraint.
#[test]
fn exact_packs_satisfy_c1_through_c7() {
    let mut rng = Rng(0x2026_0726_2005);
    for round in 0..80 {
        let n = 2 + rng.below(8);
        let store = generate(&mut rng, n);
        let s = salience(&store, &SalienceRequest::default());

        for budget in [0u64, 8, 25, 90] {
            let req = PackRequest::budget(budget).exact();
            let p = pack(&store, &s, &req).unwrap();
            let v = verify(&store, &p, &req);
            assert!(
                v.is_empty(),
                "round {round} budget {budget}: {}",
                v.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            );
        }
    }
}

/// A proven-exhaustive exact search reports a gap of exactly zero, and says it is exact.
#[test]
fn a_proven_exact_pack_reports_no_gap() {
    let mut rng = Rng(0x2026_0726_2006);
    let mut proven = 0usize;

    for _ in 0..80 {
        let n = 2 + rng.below(5);
        let store = generate(&mut rng, n);
        let s = salience(&store, &SalienceRequest::default());
        let p = pack(&store, &s, &PackRequest::budget(60).exact()).unwrap();

        assert_eq!(p.info.optimality.mode, PackMode::Exact);
        if p.report.is_empty() {
            assert_eq!(p.info.optimality.gap, 0.0);
            assert!(p.is_optimal());
            proven += 1;
        }
    }
    assert!(proven > 50, "only {proven} searches completed exhaustively");
}

/// Above the threshold, exact declines and says so rather than hanging.
#[test]
fn exact_declines_above_its_threshold_and_reports_w202() {
    let mut rng = Rng(0x2026_0726_2007);
    let store = generate(&mut rng, 12);
    let s = salience(&store, &SalienceRequest::default());

    let mut req = PackRequest::budget(100).exact();
    req.exact_threshold = 2;
    let p = pack(&store, &s, &req).unwrap();

    assert_eq!(p.report.count(smysl_core::Code::W202), 1);
    assert!(
        !p.is_optimal(),
        "a greedy fallback must not claim optimality"
    );
    assert!(p.info.optimality.gap >= 0.0);
}

#[test]
fn exact_packing_is_deterministic() {
    let mut rng = Rng(0x2026_0726_2008);
    for _ in 0..60 {
        let n = 2 + rng.below(7);
        let store = generate(&mut rng, n);
        let s = salience(&store, &SalienceRequest::default());
        let req = PackRequest::budget(45).exact();
        assert_eq!(
            pack(&store, &s, &req).unwrap().selection,
            pack(&store, &s, &req).unwrap().selection
        );
    }
}

/// The oracle has to be exercising the shapes the constraints are about, or matching it
/// proves nothing.
#[test]
fn the_generator_produces_enumerable_graphs_with_constraints() {
    let mut rng = Rng(0x2026_0726_2009);
    let mut rebuttals = 0usize;
    let mut enumerable = 0usize;
    let mut with_bodies = 0usize;
    let mut with_details = 0usize;

    for _ in 0..40 {
        let store = generate(&mut rng, 14);
        let (scope, _) = setup(&store);
        rebuttals += store.relations_of_kind(&RelKind::Rebuts).len();
        if search_space(&store, &scope) <= ENUMERATION_LIMIT {
            enumerable += 1;
        }
        with_bodies += store.units().filter(|(_, u)| u.core.body.is_some()).count();
        with_details += store
            .units()
            .filter(|(_, u)| u.core.detail.is_some())
            .count();
    }

    assert!(rebuttals > 30, "{rebuttals} rebuttals");
    assert!(
        enumerable > 10,
        "only {enumerable} of 40 graphs were enumerable"
    );
    assert!(with_bodies > 10, "{with_bodies} units had bodies");
    // L2 has to be reachable, or this gate compares brute force against branch-and-bound
    // over a search space neither can put a unit at its deepest level in. The generator
    // produced no detail at all until 0.5.0, so every comparison above was over units with
    // at most two levels — true, and a third of the question.
    assert!(
        with_details > 5,
        "only {with_details} units had a detail, so L2 was barely in the search space"
    );
}
