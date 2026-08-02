//! The SM-P11 gate, as a property test.
//!
//! Two claims, neither of which an example can establish:
//!
//! 1. **Derivation is reproducible.** The same store and options produce the same thread,
//!    and so does the same store built from its records in a different order. Rule D says
//!    a pure operation is a function of its inputs; record order is not an input.
//!
//! 2. **The repaired thread always satisfies rule L.** No step may reference a unit whose
//!    deps are absent from the thread. Repair either holds on every graph or it is not a
//!    repair - a thread that is coherent on the graphs someone happened to write down is
//!    not a guarantee, it is a coincidence.
//!
//! The generator therefore builds dependency chains on purpose, including deep ones and
//! diamonds, because a repair that only ever pulls in one level would pass a shallow
//! corpus and fail in the field.

use smysl_core::{
    canonical_uid, AgentId, Hlc, KernelType, Record, RelKind, Relation, Role, SourceKind,
    SourceRef, Status, Step, Thread, ThreadId, ThreadSchema, Uid, UnitCore, UnitCoreBuilder,
};
use smysl_graph::Store;
use smysl_thread::{definition, derive_thread, satisfies_rule_l, DeriveOptions, DeriveReport};

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

fn opts() -> DeriveOptions {
    DeriveOptions::new(
        ThreadId::new("t/derived").unwrap(),
        AgentId::new("tool:test").unwrap(),
    )
}

const TYPES: &[KernelType] = &[
    KernelType::Claim,
    KernelType::Evidence,
    KernelType::Definition,
    KernelType::Question,
    KernelType::Hypothesis,
    KernelType::Finding,
    KernelType::Procedure,
    KernelType::Decision,
    KernelType::Constraint,
    KernelType::Observation,
    KernelType::Data,
];

/// Generate a store with real dependency structure.
///
/// Units are built in dependency order, because content addressing means a reference can
/// only point at something that already exists. Deps are drawn from a *window* of recent
/// units rather than uniformly, so chains genuinely nest instead of all hanging off the
/// first unit ever made.
fn generate(rng: &mut Rng, size: usize) -> (Vec<Record>, Vec<Uid>) {
    let mut records: Vec<Record> = Vec::new();
    let mut uids: Vec<Uid> = Vec::new();

    for i in 0..size {
        let t = TYPES[rng.below(TYPES.len())];
        let gist = format!("unit {i} of a generated store, saying something");
        let mut b = UnitCoreBuilder::new(t, gist, Status::Speculative);

        if !uids.is_empty() && !rng.chance(3) {
            // Draw from the tail, so chains nest rather than fanning out from the root.
            let window = uids.len().min(4);
            let base = uids.len() - window;
            let n = 1 + rng.below(window.min(2));
            let deps: Vec<Uid> = (0..n).map(|_| uids[base + rng.below(window)]).collect();
            b = b.deps(deps);
        }
        if rng.chance(3) {
            b = b.body("a body worth some tokens, at level one");
            // And sometimes a detail, so a step can name a unit that has all three levels.
            // Rendering and packing a thread both care which levels exist; the generator
            // produced no detail until 0.5.0.
            if rng.chance(2) {
                b = b.detail("a detail worth more tokens, at level two");
            }
        }
        if t == KernelType::Evidence {
            b = b.source(SourceRef::new(SourceKind::Metric, "m"));
        }

        let core: UnitCore = b
            .build()
            .expect("the builder shapes are valid by construction");
        let uid = canonical_uid(&core);
        uids.push(uid);
        records.push(Record::Unit(core));
    }

    let kinds = [
        RelKind::Sequences,
        RelKind::Causes,
        RelKind::Rebuts,
        RelKind::Answers,
        RelKind::Elaborates,
    ];
    for _ in 0..size {
        if uids.len() < 2 {
            break;
        }
        // From later to earlier, which is the direction ordering edges point.
        let j = rng.below(uids.len());
        let i = rng.below(uids.len());
        if i >= j {
            continue;
        }
        records.push(Record::Relation(Relation::new(
            kinds[rng.below(kinds.len())].clone(),
            uids[j],
            uids[i],
        )));
    }

    (records, uids)
}

/// The same records in a different order. Rule D says the store, not the log, is the input.
fn shuffled(rng: &mut Rng, records: &[Record]) -> Store {
    let mut records = records.to_vec();
    for i in (1..records.len()).rev() {
        records.swap(i, rng.below(i + 1));
    }
    Store::from_records(records)
}

/// **The gate.** Over generated graphs, every schema, the repaired thread satisfies rule L.
#[test]
fn the_repaired_thread_always_satisfies_rule_l() {
    for seed in 1..=60u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let n = 1 + rng.below(14);
        let (records, _) = generate(&mut rng, n);
        let store = Store::from_records(records.clone());

        for &schema in ThreadSchema::ALL {
            let (thread, report) = derive_thread(&store, schema, &opts());
            let broken = satisfies_rule_l(&store, &thread);
            assert!(
                broken.is_empty(),
                "seed {seed}, {schema}: {} step(s) reference a unit whose deps are absent: \
                 {broken:?} (repaired {})",
                broken.len(),
                report.repaired.len()
            );
        }
    }
}

/// Rule L must hold under a *restricted* scope too, which is the case where repair
/// actually has work to do: a scope that cuts a chain in half leaves every dep outside it.
#[test]
fn rule_l_holds_when_the_scope_cuts_a_chain() {
    for seed in 1..=60u64 {
        let mut rng = Rng(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) | 1);
        let n = 2 + rng.below(12);
        let (records, uids) = generate(&mut rng, n);
        let store = Store::from_records(records);

        // Keep the tail of the chain, which is exactly the half that depends on the rest.
        let cut = uids.len() / 2;
        let scope: Vec<Uid> = uids[cut..].to_vec();
        if scope.is_empty() {
            continue;
        }

        for &schema in ThreadSchema::ALL {
            let o = opts().scoped(scope.clone());
            let (thread, _) = derive_thread(&store, schema, &o);
            assert!(
                satisfies_rule_l(&store, &thread).is_empty(),
                "seed {seed}, {schema}: a cut scope left the thread incoherent"
            );
        }
    }
}

/// Repair terminates. A fixpoint loop over a graph with cycles in `deps` would otherwise
/// hang, and the test suite would report it as a timeout rather than as the bug it is.
#[test]
fn repair_terminates_and_only_adds_units_the_store_holds() {
    for seed in 1..=40u64 {
        let mut rng = Rng(seed.wrapping_mul(0x1234_5678_9ABC_DEF1) | 1);
        let n = 1 + rng.below(12);
        let (records, _) = generate(&mut rng, n);
        let store = Store::from_records(records.clone());

        for &schema in ThreadSchema::ALL {
            let (thread, report) = derive_thread(&store, schema, &opts());
            for (added, _) in &report.repaired {
                assert!(store.contains_uid(added), "repair invented a unit");
                assert!(thread.units().any(|u| u == added));
            }
            assert!(
                thread.steps.len() <= store.units().count(),
                "a thread cannot name more units than exist"
            );
        }
    }
}

/// Rule D: derivation is a function of the store and the options, not of the order the
/// records happened to arrive in.
#[test]
fn derivation_is_reproducible_across_record_order() {
    for seed in 1..=40u64 {
        let mut rng = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03) | 1);
        let n = 1 + rng.below(12);
        let (records, _) = generate(&mut rng, n);
        let store = Store::from_records(records.clone());
        let other = shuffled(&mut rng, &records);

        for &schema in ThreadSchema::ALL {
            let (a, ra) = derive_thread(&store, schema, &opts());
            let (b, rb) = derive_thread(&other, schema, &opts());
            assert_eq!(
                a, b,
                "seed {seed}, {schema}: record order changed the thread"
            );
            assert_eq!(
                ra, rb,
                "seed {seed}, {schema}: record order changed the report"
            );
        }
    }
}

/// The same store derives the same thread every time it is asked.
#[test]
fn derivation_is_idempotent() {
    for seed in 1..=40u64 {
        let mut rng = Rng(seed.wrapping_mul(0xA24B_AED4_963E_E407) | 1);
        let n = 1 + rng.below(12);
        let (records, _) = generate(&mut rng, n);
        let store = Store::from_records(records.clone());
        for &schema in ThreadSchema::ALL {
            assert_eq!(
                derive_thread(&store, schema, &opts()),
                derive_thread(&store, schema, &opts()),
                "seed {seed}, {schema}"
            );
        }
    }
}

/// Selection respects the schema's arity. Repair may push a role past it - coherence wins
/// over length, because an incoherent thread is worse than a long one - so the check is
/// against the selected steps rather than against every step.
#[test]
fn selection_respects_arity() {
    for seed in 1..=40u64 {
        let mut rng = Rng(seed.wrapping_mul(0x7FEB_352D_9E37_79B9) | 1);
        let n = 1 + rng.below(16);
        let (records, _) = generate(&mut rng, n);
        let store = Store::from_records(records.clone());

        for &schema in ThreadSchema::ALL {
            let (thread, report) = derive_thread(&store, schema, &opts());
            let def = definition(schema);
            for &role in def.roles {
                let total = thread.steps.iter().filter(|s| s.role == role).count();
                let repaired = repaired_in_role(&thread, &report, role);
                assert!(
                    total - repaired <= *def.arity_of(role).end(),
                    "seed {seed}, {schema}: {role} holds {} selected units, over its arity",
                    total - repaired
                );
            }
        }
    }
}

fn repaired_in_role(thread: &smysl_thread::Thread, report: &DeriveReport, role: Role) -> usize {
    thread
        .steps
        .iter()
        .filter(|s| s.role == role && report.repaired.iter().any(|(u, _)| *u == s.unit))
        .count()
}

/// A thread never names a unit twice: repair checks what is present before inserting, so a
/// dep that was already selected is not added again.
#[test]
fn no_unit_appears_twice() {
    for seed in 1..=40u64 {
        let mut rng = Rng(seed.wrapping_mul(0x6C07_8965_1234_5679) | 1);
        let n = 1 + rng.below(14);
        let (records, _) = generate(&mut rng, n);
        let store = Store::from_records(records.clone());
        for &schema in ThreadSchema::ALL {
            let (thread, _) = derive_thread(&store, schema, &opts());
            let mut seen = std::collections::BTreeSet::new();
            for step in &thread.steps {
                assert!(
                    seen.insert(step.unit),
                    "seed {seed}, {schema}: {} appears twice",
                    step.unit
                );
            }
        }
    }
}

/// Every step's role is one the schema declares. A thread whose steps use roles from
/// another schema would not survive `check`, and derivation must not produce one.
#[test]
fn every_step_uses_a_role_the_schema_declares() {
    for seed in 1..=40u64 {
        let mut rng = Rng(seed.wrapping_mul(0x3C79_AC49_2BA7_B653) | 1);
        let n = 1 + rng.below(14);
        let (records, _) = generate(&mut rng, n);
        let store = Store::from_records(records.clone());
        for &schema in ThreadSchema::ALL {
            let (thread, _) = derive_thread(&store, schema, &opts());
            for step in &thread.steps {
                assert!(
                    schema.allows(step.role),
                    "seed {seed}, {schema}: {} is not a role of this schema",
                    step.role
                );
            }
        }
    }
}

/// A property test that never exercises the property is worse than no test at all, because
/// it reads as coverage. This asserts the generated corpus actually forces repairs - if a
/// change to the generator makes them stop happening, the gate above becomes vacuous and
/// this fails first.
#[test]
fn the_corpus_actually_forces_repairs() {
    let mut repairs = 0usize;
    for seed in 1..=60u64 {
        let mut rng = Rng(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) | 1);
        let n = 2 + rng.below(12);
        let (records, uids) = generate(&mut rng, n);
        let store = Store::from_records(records);
        let cut = uids.len() / 2;
        let scope: Vec<Uid> = uids[cut..].to_vec();
        for &schema in ThreadSchema::ALL {
            let (_, r) = derive_thread(&store, schema, &opts().scoped(scope.clone()));
            repairs += r.repaired.len();
        }
    }
    assert!(
        repairs > 50,
        "the generated corpus forced only {repairs} repairs, which is too few to trust"
    );
}

/// Rule L's oracle must *report* a broken thread, not merely fail to find one.
///
/// Found by the oracle hunt that followed `verify`: `satisfies_rule_l` is asserted
/// `.is_empty()` in two places and nowhere asserted to say anything. An oracle that always
/// returned `vec![]` would satisfy both, and the repair pass those tests exist to check would
/// be unfalsifiable — the thread could come back with holes and every test would agree it
/// did not.
///
/// This is the same defect `verify` had in `smysl-pack`, in the sibling position. Both are
/// what other tests trust rather than what they test.
#[test]
fn rule_l_reports_a_step_whose_dependency_is_missing() {
    // Two units where the second cannot be read without the first.
    let base = UnitCoreBuilder::new(KernelType::Definition, "the term being used", Status::Cited)
        .source(SourceRef::new(SourceKind::Doc, "handbook"))
        .build()
        .expect("builds");
    let base_uid = canonical_uid(&base);
    let leaning = UnitCoreBuilder::new(
        KernelType::Claim,
        "a claim that leans on that definition",
        Status::Speculative,
    )
    .deps([base_uid])
    .build()
    .expect("builds");
    let leaning_uid = canonical_uid(&leaning);

    let store = Store::from_records(vec![Record::Unit(base), Record::Unit(leaning)]);

    // A thread holding only the dependent unit. Its dependency is in the store and not in
    // the thread, which is exactly what rule L forbids.
    let thread = Thread::new(
        ThreadId::new("t/broken").unwrap(),
        ThreadSchema::Brief,
        AgentId::new("tool:test").unwrap(),
        "a thread with a hole in it",
        Hlc::new(0, 0, AgentId::new("tool:test").unwrap()),
    )
    .with_steps(vec![Step::new(Role::BottomLine, leaning_uid)]);

    let broken = satisfies_rule_l(&store, &thread);
    assert_eq!(
        broken,
        vec![(leaning_uid, base_uid)],
        "rule L's oracle did not report a step whose dependency is absent; every \
         `is_empty()` assertion that trusts it is worth nothing"
    );

    // And it must stay quiet when the dependency is present, or it would report everything
    // and be equally useless in the other direction.
    let whole = thread.clone().with_steps(vec![
        Step::new(Role::BottomLine, leaning_uid),
        Step::new(Role::Support, base_uid),
    ]);
    assert!(
        satisfies_rule_l(&store, &whole).is_empty(),
        "a thread carrying its dependencies must not be reported"
    );
}
