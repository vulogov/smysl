//! The SM-P6 gate, as a property test.
//!
//! Rule U claims merge is a join-semilattice union: commutative, associative, idempotent.
//! Everything downstream rests on that claim - no coordination, no vector clocks, no
//! causal-delivery requirement, delivery may be out of order or duplicated or partial.
//! These are exactly the properties that are cheap to test exhaustively and catastrophic
//! to get quietly wrong, so they are tested over generated stores rather than over
//! examples.
//!
//! Rule M is asserted on every result too: it is a local predicate and grounds are never
//! removed, so a union cannot break it. If it ever does, the union is not what §5.1 says.

use std::collections::BTreeMap;

use smysl_core::{
    canonical_uid, AgentId, Attestation, Hlc, KernelType, Op, Record, RelKind, Relation, Role,
    Rung, SourceKind, SourceRef, Status, Step, Thread, ThreadId, ThreadSchema, Uid, UnitCore,
    UnitCoreBuilder,
};
use smysl_graph::{merge, MergeOptions, Store};

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

fn agent(n: usize) -> AgentId {
    AgentId::new(format!("model:vendor{}/m", n % 4)).unwrap()
}

fn opts() -> MergeOptions {
    // A fixed clock keeps merge a pure function of its inputs, so a difference between
    // two merges is a difference in the algebra rather than in the wall clock.
    MergeOptions::default().with_now(Hlc::new(0, 0, AgentId::new("tool:test").unwrap()))
}

/// Generate a store whose units genuinely reference each other.
///
/// Units are built in dependency order and their uids fed forward, because content
/// addressing means a reference can only point at something that already exists.
fn generate(rng: &mut Rng, size: usize) -> Store {
    let mut records: Vec<Record> = Vec::new();
    let mut uids: Vec<Uid> = Vec::new();

    for i in 0..size {
        let gist = format!("unit {} of a generated store", rng.next() % 1000);
        // Bodies and details, because they are part of the unit core and therefore part of
        // the uid — two units identical but for a body are two different units, and merge
        // has to keep both. The generator set neither until 0.5.0, so the join-semilattice
        // laws had only ever been checked over gist-only units.
        let body = if rng.chance(2) {
            Some(format!("a body of {} sentences. ", 1 + rng.below(3)).repeat(2))
        } else {
            None
        };
        let detail = match &body {
            Some(_) if rng.chance(2) => Some("a detail paragraph.".to_string()),
            _ => None,
        };
        let with_levels = |mut b: UnitCoreBuilder| {
            if let Some(t) = body.clone() {
                b = b.body(t);
            }
            if let Some(t) = detail.clone() {
                b = b.detail(t);
            }
            b
        };
        let core = if uids.is_empty() || rng.chance(3) {
            // A rootless unit: measured evidence or a bare speculation.
            if rng.chance(2) {
                with_levels(
                    UnitCoreBuilder::new(KernelType::Evidence, gist, Status::Measured)
                        .source(SourceRef::new(SourceKind::Metric, "m")),
                )
                .build()
                .unwrap()
            } else {
                with_levels(UnitCoreBuilder::new(
                    KernelType::Hypothesis,
                    gist,
                    Status::Speculative,
                ))
                .build()
                .unwrap()
            }
        } else {
            // A grounded unit. The status is chosen freely, so some of these violate
            // rule M - which is the point: merge must not care.
            let n = 1 + rng.below(uids.len().min(3));
            let grounds: Vec<Uid> = (0..n).map(|_| uids[rng.below(uids.len())]).collect();
            let status = if rng.chance(2) {
                Status::Inferred
            } else {
                Status::Derived
            };
            with_levels(UnitCoreBuilder::new(KernelType::Claim, gist, status).grounds(grounds))
                .build()
                .unwrap()
        };

        let uid = canonical_uid(&core);
        uids.push(uid);
        records.push(Record::Unit(core));

        if rng.chance(2) {
            let a = agent(i);
            records.push(Record::Attestation(
                Attestation::new(
                    uid,
                    a.clone(),
                    if rng.chance(3) {
                        Op::Imported
                    } else {
                        Op::Authored
                    },
                    [Rung::Computed, Rung::Document, Rung::Web, Rung::Model][rng.below(4)],
                    Hlc::new(rng.next() % 100, (rng.next() % 4) as u32, a),
                )
                .at_hop((rng.next() % 5) as u32),
            ));
        }
    }

    // Relations, including the ones that make contentions.
    let kinds = [
        RelKind::Rebuts,
        RelKind::Supersedes,
        RelKind::Causes,
        RelKind::Elaborates,
        RelKind::Retracts,
    ];
    for _ in 0..size {
        if uids.len() < 2 {
            break;
        }
        let from = uids[rng.below(uids.len())];
        let to = uids[rng.below(uids.len())];
        if from == to {
            continue;
        }
        records.push(Record::Relation(Relation::new(
            kinds[rng.below(kinds.len())].clone(),
            from,
            to,
        )));
    }

    // Threads, which is what turns a rebuttal into a live contention.
    for i in 0..(size / 4).max(1) {
        if uids.is_empty() {
            break;
        }
        let a = agent(i);
        let steps: Vec<Step> = (0..1 + rng.below(3))
            .map(|_| Step::new(Role::BottomLine, uids[rng.below(uids.len())]))
            .collect();
        records.push(Record::Thread(
            Thread::new(
                ThreadId::new(format!("t/x{}", i % 3)).unwrap(),
                ThreadSchema::Brief,
                a.clone(),
                format!("thread {i}"),
                Hlc::new(rng.next() % 10, (rng.next() % 3) as u32, a),
            )
            .with_steps(steps),
        ));
    }

    Store::from_records(records)
}

/// A store with detection already run over it. Merge reports contentions rather than
/// writing them, so this is the same store - but going through `merge` once makes the
/// comparison honest about what path each side took.
fn normalised(a: &Store) -> Store {
    merged(a, &Store::new())
}

fn merged(a: &Store, b: &Store) -> Store {
    let mut out = a.clone();
    merge(&mut out, b, opts()).expect("merge does not fail without --fail-on-contention");
    out
}

/// Rule M holds on a store: no `derived` or `inferred` unit exceeds its weakest present
/// ground.
fn rule_m_holds(store: &Store) -> Result<(), String> {
    for (uid, unit) in store.units() {
        if !unit.core.status.is_rule_m_constrained() {
            continue;
        }
        let cap = unit
            .core
            .grounds
            .iter()
            .filter_map(|g| store.get(g).map(|u| u.core.status))
            .min();
        if let Some(cap) = cap {
            if unit.core.status > cap {
                return Err(format!("{uid}: {} exceeds {cap}", unit.core.status));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The algebra
// ---------------------------------------------------------------------------

/// `merge(A, B) = merge(B, A)`.
///
/// Two peers gossiping in opposite directions must reach the same store, or the mesh
/// needs coordination - which is the thing rule U exists to avoid.
#[test]
fn merge_is_commutative() {
    let mut rng = Rng(0x2026_0726_0001);
    for round in 0..200 {
        let n = 1 + rng.below(8);
        let a = generate(&mut rng, n);
        let n = 1 + rng.below(8);
        let b = generate(&mut rng, n);
        assert_eq!(
            merged(&a, &b).state_hash(),
            merged(&b, &a).state_hash(),
            "round {round}: merge(A,B) != merge(B,A)"
        );
    }
}

/// `merge(merge(A, B), C) = merge(A, merge(B, C))`.
#[test]
fn merge_is_associative() {
    let mut rng = Rng(0x2026_0726_0002);
    for round in 0..150 {
        let n = 1 + rng.below(6);
        let a = generate(&mut rng, n);
        let n = 1 + rng.below(6);
        let b = generate(&mut rng, n);
        let n = 1 + rng.below(6);
        let c = generate(&mut rng, n);

        let left = merged(&merged(&a, &b), &c);
        let right = merged(&a, &merged(&b, &c));
        assert_eq!(
            left.state_hash(),
            right.state_hash(),
            "round {round}: association changed the result"
        );
    }
}

/// `merge(A, A) = A`, and `merge(merge(A,B), B) = merge(A,B)`.
#[test]
fn merge_is_idempotent() {
    let mut rng = Rng(0x2026_0726_0003);
    for round in 0..200 {
        let n = 1 + rng.below(8);
        let a = generate(&mut rng, n);
        let n = 1 + rng.below(8);
        let b = generate(&mut rng, n);

        assert_eq!(
            merged(&a, &a).state_hash(),
            normalised(&a).state_hash(),
            "round {round}: a store merged with itself changed"
        );

        let once = merged(&a, &b);
        let twice = merged(&once, &b);
        assert_eq!(
            once.state_hash(),
            twice.state_hash(),
            "round {round}: re-delivering B changed the result"
        );
    }
}

/// Delivery may be out of order, duplicated, or partial. All three at once.
#[test]
fn any_delivery_order_converges() {
    let mut rng = Rng(0x2026_0726_0004);
    for round in 0..100 {
        let mut parts: Vec<Store> = Vec::new();
        for _ in 0..4 {
            let n = 1 + rng.below(5);
            parts.push(generate(&mut rng, n));
        }

        let mut straight = Store::new();
        for p in &parts {
            merge(&mut straight, p, opts()).unwrap();
        }

        // The same parts, shuffled and with duplicates thrown in.
        let mut messy = Store::new();
        let mut order: Vec<usize> = (0..parts.len()).collect();
        for i in 0..order.len() {
            let j = rng.below(order.len());
            order.swap(i, j);
        }
        for &i in &order {
            merge(&mut messy, &parts[i], opts()).unwrap();
            if rng.chance(2) {
                merge(&mut messy, &parts[rng.below(parts.len())], opts()).unwrap();
            }
        }
        for &i in &order {
            merge(&mut messy, &parts[i], opts()).unwrap();
        }

        assert_eq!(
            straight.state_hash(),
            messy.state_hash(),
            "round {round}: delivery order changed the store"
        );
    }
}

/// Rule M is a local predicate over grounds, and a union never removes a ground - so a
/// merge cannot turn a compliant store into a non-compliant one.
#[test]
fn merge_never_introduces_a_rule_m_violation() {
    let mut rng = Rng(0x2026_0726_0005);
    let mut checked = 0usize;

    for round in 0..200 {
        let n = 1 + rng.below(8);
        let a = generate(&mut rng, n);
        let n = 1 + rng.below(8);
        let b = generate(&mut rng, n);
        if rule_m_holds(&a).is_err() || rule_m_holds(&b).is_err() {
            // Either input already launders; merge is not obliged to repair it.
            continue;
        }
        checked += 1;
        rule_m_holds(&merged(&a, &b))
            .unwrap_or_else(|e| panic!("round {round}: merge broke rule M: {e}"));
    }

    assert!(
        checked > 20,
        "only {checked} rounds had two compliant inputs"
    );
}

/// Nothing is ever lost. Every unit in either input is in the result - the store is a
/// grow-only set.
#[test]
fn merge_never_loses_a_unit() {
    let mut rng = Rng(0x2026_0726_0006);
    for round in 0..200 {
        let n = 1 + rng.below(8);
        let a = generate(&mut rng, n);
        let n = 1 + rng.below(8);
        let b = generate(&mut rng, n);
        let m = merged(&a, &b);
        for (uid, _) in a.units().chain(b.units()) {
            assert!(m.contains_uid(uid), "round {round}: {uid} was lost");
        }
    }
}

/// Detection is a function of the merged set, so two peers that reach the same store
/// agree about what is contested - down to the identifiers.
#[test]
fn contentions_are_a_function_of_the_union() {
    let mut rng = Rng(0x2026_0726_0007);
    let mut with_contentions = 0usize;

    for round in 0..150 {
        let n = 2 + rng.below(8);
        let a = generate(&mut rng, n);
        let n = 2 + rng.below(8);
        let b = generate(&mut rng, n);

        let mut left = a.clone();
        let one = merge(&mut left, &b, opts()).unwrap();
        let mut right = b.clone();
        let two = merge(&mut right, &a, opts()).unwrap();

        let ids = |r: &smysl_graph::MergeReport| {
            let mut v: Vec<String> = r.contentions.iter().map(|c| c.id.to_string()).collect();
            v.sort();
            v
        };
        assert_eq!(ids(&one), ids(&two), "round {round}: contentions disagree");
        if !one.contentions.is_empty() {
            with_contentions += 1;
        }
    }

    assert!(
        with_contentions > 10,
        "only {with_contentions} rounds produced a contention; the generator is too tame"
    );
}

// ---------------------------------------------------------------------------
// Retraction
// ---------------------------------------------------------------------------

/// The gate: a dry run reports exactly what applying the retraction produces.
#[test]
fn the_retraction_blast_radius_matches_the_dry_run() {
    use smysl_graph::{effective_status, plan_retraction, RetractionAuthority, RetractionPolicy};

    let mut rng = Rng(0x2026_0726_0008);
    let mut checked = 0usize;

    for round in 0..200 {
        let n = 3 + rng.below(8);
        let store = generate(&mut rng, n);
        let targets: Vec<Uid> = store.units().map(|(u, _)| *u).collect();
        if targets.is_empty() {
            continue;
        }
        let target = targets[rng.below(targets.len())];

        let plan = plan_retraction(
            &store,
            target,
            &[AgentId::new("human:v").unwrap()],
            RetractionPolicy::Strict,
            RetractionAuthority::Any,
        );

        let mut applied = store.clone();
        applied
            .append(&[Record::Relation(Relation::new(
                RelKind::Retracts,
                target,
                target,
            ))])
            .unwrap();
        let eff = effective_status(&applied, RetractionPolicy::Strict);
        let actual: Vec<Uid> = eff.blast_radius().into_iter().collect();

        assert_eq!(
            plan.blast_radius, actual,
            "round {round}: the dry run disagreed with the real thing"
        );
        checked += 1;
    }

    assert!(checked > 100, "only {checked} rounds ran");
}

/// Merging a retraction in is the same as having had it all along - retraction is part of
/// the union, not an operation applied to it.
#[test]
fn a_retraction_converges_whichever_side_it_arrives_from() {
    use smysl_graph::{effective_status, RetractionPolicy};

    let mut rng = Rng(0x2026_0726_0009);
    for round in 0..100 {
        let n = 3 + rng.below(6);
        let base = generate(&mut rng, n);
        let targets: Vec<Uid> = base.units().map(|(u, _)| *u).collect();
        if targets.is_empty() {
            continue;
        }
        let target = targets[rng.below(targets.len())];
        let retraction = Store::from_records(vec![Record::Relation(Relation::new(
            RelKind::Retracts,
            target,
            target,
        ))]);

        let forward = merged(&base, &retraction);
        let backward = merged(&retraction, &base);
        assert_eq!(
            forward.state_hash(),
            backward.state_hash(),
            "round {round}: a retraction was order-sensitive"
        );

        let a = effective_status(&forward, RetractionPolicy::Strict);
        let b = effective_status(&backward, RetractionPolicy::Strict);
        assert_eq!(a, b);
    }
}

// ---------------------------------------------------------------------------
// The digest itself
// ---------------------------------------------------------------------------

/// The whole property suite is measured with `state_hash`, so it has to be measuring the
/// right thing: log order must not affect it, and content must.
#[test]
fn the_state_digest_ignores_log_order_and_notices_content() {
    let mut rng = Rng(0x2026_0726_000A);
    for _ in 0..50 {
        let n = 4 + rng.below(6);
        let store = generate(&mut rng, n);
        let mut records: Vec<Record> = store.iter().cloned().collect();
        let forward = Store::from_records(records.clone());
        records.reverse();
        let backward = Store::from_records(records);

        assert_eq!(forward.state_hash(), backward.state_hash());
        assert!(forward.converged_with(&backward));
    }

    let a = Store::from_records(vec![Record::Unit(
        UnitCoreBuilder::new(KernelType::Claim, "a", Status::Speculative)
            .build()
            .unwrap(),
    )]);
    let b = Store::from_records(vec![Record::Unit(
        UnitCoreBuilder::new(KernelType::Claim, "b", Status::Speculative)
            .build()
            .unwrap(),
    )]);
    assert_ne!(a.state_hash(), b.state_hash());
    assert!(!a.converged_with(&b));
}

/// A thread register is a maximum over a total order, so a tie on the HLC still converges.
#[test]
fn simultaneous_thread_writes_still_converge() {
    let core: UnitCore = UnitCoreBuilder::new(KernelType::Claim, "a claim", Status::Speculative)
        .build()
        .unwrap();
    let uid = canonical_uid(&core);
    let owner = AgentId::new("human:v").unwrap();
    let at_the_same_moment = |gist: &str| {
        Record::Thread(
            Thread::new(
                ThreadId::new("t/x").unwrap(),
                ThreadSchema::Brief,
                owner.clone(),
                gist,
                Hlc::new(7, 0, owner.clone()),
            )
            .with_steps([Step::new(Role::BottomLine, uid)]),
        )
    };

    let a = Store::from_records(vec![Record::Unit(core.clone()), at_the_same_moment("mine")]);
    let b = Store::from_records(vec![Record::Unit(core), at_the_same_moment("theirs")]);

    assert_eq!(merged(&a, &b).state_hash(), merged(&b, &a).state_hash());
    assert_eq!(merged(&a, &b).threads().count(), 1);
}

/// The generator has to actually generate something worth testing.
#[test]
fn the_generator_produces_a_varied_corpus() {
    let mut rng = Rng(0x2026_0726_000B);
    let mut units = 0usize;
    let mut relations = 0usize;
    let mut threads = 0usize;
    let mut attestations = 0usize;
    let mut statuses: BTreeMap<Status, usize> = BTreeMap::new();

    for _ in 0..50 {
        let s = generate(&mut rng, 8);
        units += s.units().count();
        relations += s.relations().count();
        threads += s.threads().count();
        for (_, u) in s.units() {
            attestations += u.attestations.len();
            *statuses.entry(u.core.status).or_insert(0) += 1;
        }
    }

    assert!(units > 200, "{units} units");
    assert!(relations > 50, "{relations} relations");
    assert!(threads > 20, "{threads} threads");
    assert!(attestations > 50, "{attestations} attestations");
    assert!(
        statuses.len() >= 3,
        "only {} distinct statuses",
        statuses.len()
    );
}
