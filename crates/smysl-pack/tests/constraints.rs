//! The SM-P9 gate, as a property test.
//!
//! C1-C7 must hold on **every** pack, over generated graphs and every budget from nothing
//! to plenty. These are the constraints that make a pack safe to hand to a model without
//! reading it: the budget is respected, the closure is complete, and no claim ever travels
//! without its rebuttals.
//!
//! The last of those is the one worth generating for. A packer that gets C3 right on a
//! hand-written example and wrong at some awkward budget would hand a model a one-sided
//! argument, which is exactly the failure prose transport already has (F7).

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::{
    canonical_uid, AgentId, Contention, ContentionId, Detected, DetectionKind, Hlc, KernelType,
    Lod, PackError, Record, RelKind, Relation, SourceKind, SourceRef, Status, Uid, UnitCore,
    UnitCoreBuilder,
};
use smysl_graph::{salience, SalienceRequest, Store};
use smysl_pack::{pack, verify, Estimator, PackRequest};

/// A seeded xorshift, so a failure is reproducible from its seed alone.
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

fn body(rng: &mut Rng) -> String {
    "word ".repeat(4 + rng.below(24))
}

fn detail(rng: &mut Rng) -> String {
    "detail ".repeat(4 + rng.below(40))
}

/// A store with rebuttals, contentions, warrants and mixed levels of detail - the shapes
/// the constraints are about.
fn generate(rng: &mut Rng, size: usize) -> Store {
    let mut records: Vec<Record> = Vec::new();
    let mut uids: Vec<Uid> = Vec::new();

    for i in 0..size {
        let gist = format!("unit {} of a generated store", rng.next() % 10_000);
        let core: UnitCore = if uids.is_empty() || rng.chance(3) {
            let mut b = UnitCoreBuilder::new(KernelType::Evidence, gist, Status::Measured)
                .source(SourceRef::new(SourceKind::Metric, "m"));
            if rng.chance(2) {
                let t = body(rng);
                b = b.body(t);
                if rng.chance(3) {
                    let d = detail(rng);
                    b = b.detail(d);
                }
            }
            b.build().unwrap()
        } else {
            let n = 1 + rng.below(uids.len().min(3));
            let grounds: Vec<Uid> = (0..n).map(|_| uids[rng.below(uids.len())]).collect();
            let mut b =
                UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative).grounds(grounds);
            if rng.chance(2) {
                let t = body(rng);
                b = b.body(t);
                if rng.chance(3) {
                    let d = detail(rng);
                    b = b.detail(d);
                }
            }
            b.build().unwrap()
        };
        let uid = canonical_uid(&core);
        uids.push(uid);
        records.push(Record::Unit(core));
        let _ = i;
    }

    // Rebuttals and warrants: the edges C3 and C6 are about.
    for _ in 0..size {
        if uids.len() < 2 {
            break;
        }
        let from = uids[rng.below(uids.len())];
        let to = uids[rng.below(uids.len())];
        if from == to {
            continue;
        }
        let kind = if rng.chance(2) {
            RelKind::Rebuts
        } else if rng.chance(2) {
            RelKind::Warrant
        } else {
            RelKind::Elaborates
        };
        records.push(Record::Relation(Relation::new(kind, from, to)));
    }

    // Open contentions: what C4 is about.
    for i in 0..(size / 4) {
        if uids.len() < 2 {
            break;
        }
        let a = uids[rng.below(uids.len())];
        let b = uids[rng.below(uids.len())];
        if a == b {
            continue;
        }
        records.push(Record::Contention(Contention::new(
            ContentionId::new(format!("k/c{i}")).unwrap(),
            a,
            vec![a, b],
            Detected {
                kind: DetectionKind::SupersessionFork,
                ts: Hlc::new(0, 0, AgentId::new("tool:test").unwrap()),
            },
        )));
    }

    Store::from_records(records)
}

fn sal(store: &Store) -> smysl_graph::SalienceReport {
    salience(store, &SalienceRequest::default())
}

/// Total cost of a selection, computed independently of the solver.
fn independent_cost(store: &Store, selection: &BTreeMap<Uid, Lod>) -> u64 {
    let e = Estimator::default();
    selection
        .iter()
        .filter_map(|(u, l)| store.get(u).map(|unit| e.unit(&unit.core, *l)))
        .sum()
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// C1-C7 hold on every pack over generated graphs and budgets.
#[test]
fn every_pack_satisfies_c1_through_c7() {
    let mut rng = Rng(0x2026_0726_1001);
    let mut packed_something = 0usize;

    for round in 0..300 {
        let n = 2 + rng.below(10);
        let store = generate(&mut rng, n);
        let s = sal(&store);

        for budget in [0u64, 5, 15, 40, 100, 250, 600, 5_000] {
            let req = PackRequest::budget(budget);
            let p = pack(&store, &s, &req).expect("no focus, so the floor is empty");
            let v = verify(&store, &p, &req);
            assert!(
                v.is_empty(),
                "round {round} budget {budget}: {}",
                v.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            );
            if !p.is_empty() {
                packed_something += 1;
            }
        }
    }

    assert!(
        packed_something > 500,
        "only {packed_something} packs had anything in them; the generator is too tame"
    );
}

/// C7 specifically, checked against a cost computed outside the solver - so a bug in the
/// solver's own accounting cannot hide itself.
#[test]
fn a_pack_never_exceeds_its_budget() {
    let mut rng = Rng(0x2026_0726_1002);
    for round in 0..200 {
        let n = 2 + rng.below(8);
        let store = generate(&mut rng, n);
        let s = sal(&store);
        let budget = (rng.below(400) + 1) as u64;

        let p = pack(&store, &s, &PackRequest::budget(budget)).unwrap();
        let actual = independent_cost(&store, &p.selection);
        assert!(actual <= budget, "round {round}: {actual} over {budget}");
        assert_eq!(
            p.used(),
            actual,
            "round {round}: the manifest misreports use"
        );
    }
}

/// Rule R, generated. No selected claim ever appears without a rebuttal that is in the
/// store.
#[test]
fn no_claim_is_ever_packed_unopposed() {
    let mut rng = Rng(0x2026_0726_1003);
    let mut with_rebuttals = 0usize;

    for round in 0..300 {
        let n = 3 + rng.below(8);
        let store = generate(&mut rng, n);
        let s = sal(&store);

        for budget in [8u64, 25, 60, 150, 400] {
            let p = pack(&store, &s, &PackRequest::budget(budget)).unwrap();
            for uid in p.selection.keys() {
                for r in store.rebuttals_of(uid) {
                    if !store.contains_uid(&r) {
                        continue;
                    }
                    with_rebuttals += 1;
                    assert!(
                        p.selection.contains_key(&r),
                        "round {round} budget {budget}: {uid} packed without {r}"
                    );
                }
            }
        }
    }

    assert!(
        with_rebuttals > 100,
        "only {with_rebuttals} rebuttal obligations were exercised"
    );
}

/// An infeasible floor fails with a minimum budget that is correct in both directions:
/// it succeeds, and one token less does not.
#[test]
fn an_infeasible_floor_reports_a_tight_minimum() {
    let mut rng = Rng(0x2026_0726_1004);
    let mut checked = 0usize;

    for round in 0..200 {
        let n = 3 + rng.below(8);
        let store = generate(&mut rng, n);
        let s = sal(&store);
        let units: Vec<Uid> = store.units().map(|(u, _)| *u).collect();
        if units.is_empty() {
            continue;
        }
        let focus = units[rng.below(units.len())];

        // A budget of 1 cannot hold anything, so the floor is always infeasible.
        let err = pack(&store, &s, &PackRequest::budget(1).focusing([focus])).unwrap_err();
        let PackError::Infeasible { required, .. } = err else {
            panic!("round {round}: expected Infeasible, got {err:?}");
        };

        let ok = PackRequest::budget(required).focusing([focus]);
        let p = pack(&store, &s, &ok)
            .unwrap_or_else(|e| panic!("round {round}: the reported minimum did not fit: {e:?}"));
        assert!(verify(&store, &p, &ok).is_empty(), "round {round}");

        assert!(
            pack(
                &store,
                &s,
                &PackRequest::budget(required - 1).focusing([focus])
            )
            .is_err(),
            "round {round}: the reported minimum was not tight"
        );
        checked += 1;
    }

    assert!(checked > 150, "only {checked} rounds ran");
}

/// Packing never degrades silently. A pack that dropped anything says so, and a complete
/// one says that too.
#[test]
fn truncation_is_always_self_describing() {
    let mut rng = Rng(0x2026_0726_1005);
    for round in 0..200 {
        let n = 2 + rng.below(8);
        let store = generate(&mut rng, n);
        let s = sal(&store);
        let total = store.units().count();

        for budget in [10u64, 50, 200, 10_000] {
            let p = pack(&store, &s, &PackRequest::budget(budget)).unwrap();
            let dropped = total - p.len();
            assert_eq!(
                p.info.dropped.len(),
                dropped,
                "round {round} budget {budget}: the manifest miscounts what was dropped"
            );
            if dropped > 0 {
                assert!(!p.info.is_complete());
                assert!(!p.info.drop_histogram().is_empty());
            }
            assert_eq!(p.info.budget, budget);
            assert_eq!(p.info.estimator, "smysl/utf8-div4");
        }
    }
}

/// More budget never buys less. A packer that went backwards would make budgets
/// meaningless.
#[test]
fn value_is_monotone_in_budget() {
    let mut rng = Rng(0x2026_0726_1006);
    for round in 0..150 {
        let n = 3 + rng.below(8);
        let store = generate(&mut rng, n);
        let s = sal(&store);

        let mut previous = 0.0f64;
        for budget in [10u64, 30, 80, 200, 500, 2_000] {
            let p = pack(&store, &s, &PackRequest::budget(budget)).unwrap();
            let value: f64 = p
                .selection
                .iter()
                .map(|(u, l)| smysl_pack::value(s.get(u), *l))
                .sum();
            assert!(
                value >= previous - 1e-9,
                "round {round}: budget {budget} bought less than a smaller one"
            );
            previous = value;
        }
    }
}

/// The whole point of the phase: identical inputs, identical bytes.
#[test]
fn packing_is_deterministic_over_generated_stores() {
    let mut rng = Rng(0x2026_0726_1007);
    for round in 0..150 {
        let n = 2 + rng.below(8);
        let store = generate(&mut rng, n);
        let s = sal(&store);
        let budget = (rng.below(300) + 1) as u64;
        let req = PackRequest::budget(budget);

        let a = pack(&store, &s, &req).unwrap();
        let b = pack(&store, &s, &req).unwrap();
        assert_eq!(a.selection, b.selection, "round {round}");
        assert_eq!(a.info.used, b.info.used);
        assert_eq!(a.why, b.why);
    }
}

/// Arrival order must not reach the selection.
#[test]
fn record_order_does_not_change_a_generated_pack() {
    let mut rng = Rng(0x2026_0726_1008);
    for round in 0..100 {
        let n = 2 + rng.below(8);
        let store = generate(&mut rng, n);
        let mut reversed: Vec<Record> = store.iter().cloned().collect();
        reversed.reverse();
        let other = Store::from_records(reversed);

        let budget = (rng.below(300) + 1) as u64;
        let req = PackRequest::budget(budget);
        let a = pack(&store, &sal(&store), &req).unwrap();
        let b = pack(&other, &sal(&other), &req).unwrap();
        assert_eq!(a.selection, b.selection, "round {round}");
    }
}

/// Every unit in a pack can say why it is there, and a forced one names its constraint.
#[test]
fn every_packed_unit_is_explained() {
    let mut rng = Rng(0x2026_0726_1009);
    for round in 0..150 {
        let n = 3 + rng.below(8);
        let store = generate(&mut rng, n);
        let s = sal(&store);
        let units: Vec<Uid> = store.units().map(|(u, _)| *u).collect();
        if units.is_empty() {
            continue;
        }
        let focus = units[rng.below(units.len())];

        let required = match pack(&store, &s, &PackRequest::budget(1).focusing([focus])) {
            Err(PackError::Infeasible { required, .. }) => required,
            _ => continue,
        };
        let req = PackRequest::budget(required * 4).focusing([focus]);
        let p = pack(&store, &s, &req).unwrap();

        for uid in p.selection.keys() {
            assert!(
                p.why.contains_key(uid),
                "round {round}: {uid} is unexplained"
            );
        }
        assert_eq!(p.why.get(&focus), Some(&smysl_pack::Reason::Focus));
    }
}

/// The generator has to produce the shapes the constraints are about.
#[test]
fn the_generator_exercises_every_constraint() {
    let mut rng = Rng(0x2026_0726_100A);
    let mut rebuttals = 0usize;
    let mut warrants = 0usize;
    let mut contentions = 0usize;
    let mut levels: BTreeSet<Lod> = BTreeSet::new();

    for _ in 0..60 {
        let store = generate(&mut rng, 8);
        rebuttals += store.relations_of_kind(&RelKind::Rebuts).len();
        warrants += store.relations_of_kind(&RelKind::Warrant).len();
        contentions += store.contentions().len();
        for (_, u) in store.units() {
            levels.insert(u.core.max_lod());
        }
    }

    assert!(rebuttals > 20, "{rebuttals} rebuttals");
    assert!(warrants > 10, "{warrants} warrants");
    assert!(contentions > 20, "{contentions} contentions");
    assert_eq!(levels.len(), 3, "all three levels of detail must appear");
}
