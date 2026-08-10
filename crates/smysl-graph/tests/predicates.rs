//! Three predicates that could each be replaced with `true`, with nothing anywhere failing.
//!
//! Found by mutation testing in 0.11, and confirmed against the *whole workspace* rather than
//! just this crate's suite — which matters, because `cargo mutants -p smysl-graph` runs only
//! `smysl-graph`'s tests, and a function exercised downstream reads as a survivor while being
//! well covered. `Store::matching_prefix` was on the same shortlist and turned out to be
//! exactly that. These three are not: replaced with `true`, `cargo test --workspace
//! --all-features` still passes.
//!
//! Each sits directly behind something a user does:
//!
//! * `MergeReport::has_contentions` is what `merge --fail-on-contention` branches on.
//! * `EffectiveStatus::is_retracted` decides whether a retraction actually took.
//! * `TraceKind::follows_parents` picks which way `trace` walks.
//!
//! A predicate stuck at `true` is the worst shape of wrong for all three: a merge that always
//! reports contention, a store where everything reads as retracted, a trace that always follows
//! provenance. Each fails safe-looking and loud, which is precisely why nobody would suspect
//! the test was missing.
//!
//! **Two more of the same shape, found in 1.1 by re-measuring rather than by looking.** The
//! 0.11 sweep produced a shortlist of three and the class was taken as handled; it was not.
//! `Lineage::is_empty` and `RetractionPlan::is_empty` are the same kind of predicate with the
//! same absence behind them, and both sit on a path a user takes — `trace` reporting whether
//! it found any ancestry, and `retract` reporting whether a plan would do anything.
//!
//! The lesson is about the sweep rather than the code: a shortlist is what one pass surfaced,
//! not the population.

use std::collections::BTreeMap;

use smysl_core::{
    canonical_uid, AgentId, Attestation, Hlc, KernelType, Op, Record, RelKind, Relation, Rung,
    Status, Uid, UnitCore, UnitCoreBuilder,
};
use smysl_graph::lineage::{trace, TraceKind};
use smysl_graph::merge::retraction::effective_status;
use smysl_graph::merge::{merge, MergeOptions};
use smysl_graph::Store;
use smysl_graph::{RetractionAuthority, RetractionPolicy};

fn agent() -> AgentId {
    AgentId::new("human:alice").unwrap()
}

fn hlc(ms: u64) -> Hlc {
    Hlc {
        wall_ms: ms,
        counter: 0,
        agent: agent(),
    }
}

fn claim(gist: &str, grounds: Vec<Uid>) -> UnitCore {
    let status = if grounds.is_empty() {
        Status::Speculative
    } else {
        Status::Inferred
    };
    UnitCoreBuilder::new(KernelType::Claim, gist, status)
        .grounds(grounds)
        .build()
        .unwrap()
}

// -- MergeReport::has_contentions ---------------------------------------------------------

/// Two stores that disagree, and two that do not. The predicate must tell them apart.
#[test]
fn has_contentions_distinguishes_a_disagreement_from_agreement() {
    let base = claim("the pool saturated", vec![]);
    let ub = canonical_uid(&base);

    // A supersession *fork*: two units superseding the same target, neither superseding the
    // other. That is a disagreement merge must record rather than adjudicate (§5.4).
    //
    // The first attempt here used a bare `rebuts` relation, and the positive assertion caught
    // that it detects nothing: a live rebuttal is a contention only when both units appear
    // together in a thread. A test asserting only the negative direction would have shipped
    // with a fixture that never contended.
    let one = claim("the pool saturated, per the canary", vec![]);
    let two = claim("the pool saturated, per the shard metrics", vec![]);
    let (u1, u2) = (canonical_uid(&one), canonical_uid(&two));

    let mut disagreeing = Store::from_records(vec![Record::Unit(base.clone())]);
    let incoming = Store::from_records(vec![
        Record::Unit(base.clone()),
        Record::Unit(one),
        Record::Unit(two),
        Record::Relation(Relation::new(RelKind::Supersedes, u1, ub)),
        Record::Relation(Relation::new(RelKind::Supersedes, u2, ub)),
    ]);
    let report = merge(&mut disagreeing, &incoming, MergeOptions::default()).unwrap();

    // The control comes first, because "always true" passes any single positive case.
    let mut agreeing = Store::from_records(vec![Record::Unit(base.clone())]);
    let same = Store::from_records(vec![Record::Unit(base)]);
    let quiet = merge(&mut agreeing, &same, MergeOptions::default()).unwrap();

    assert!(
        !quiet.has_contentions(),
        "merging a store with itself reports contention; `merge --fail-on-contention` would \
         refuse every idempotent merge"
    );
    assert!(
        quiet.contentions.is_empty(),
        "and the underlying list agrees with the predicate"
    );

    // And the positive direction, without which `has_contentions -> false` passes just as
    // happily. Both directions or neither: a predicate stuck at either constant is a bug, and
    // a test that pins only one of them catches only one of them.
    assert!(
        report.has_contentions(),
        "a rebuttal arrived and no contention was reported; `--fail-on-contention` would \
         never fire"
    );
    assert!(!report.contentions.is_empty());
}

// -- EffectiveStatus::is_retracted --------------------------------------------------------

#[test]
fn is_retracted_is_false_for_a_unit_nobody_retracted() {
    let a = claim("a standing claim", vec![]);
    let ua = canonical_uid(&a);
    let store = Store::from_records(vec![Record::Unit(a)]);

    let eff = effective_status(&store, RetractionPolicy::default());
    assert!(
        !eff.is_retracted(&ua),
        "a unit nobody retracted reads as retracted; every consumer would treat a healthy \
         store as withdrawn"
    );
}

/// The control for the one above. Without it, `is_retracted -> false` would pass just as
/// happily as the correct implementation.
#[test]
fn is_retracted_is_true_for_a_unit_that_was() {
    let a = claim("a claim that will be withdrawn", vec![]);
    let ua = canonical_uid(&a);
    // A retraction is a `retracts` relation from a withdrawing unit, not an attestation op.
    let withdrawal = claim("the earlier claim does not hold", vec![]);
    let uw = canonical_uid(&withdrawal);
    let store = Store::from_records(vec![
        Record::Unit(a),
        Record::Unit(withdrawal),
        Record::Relation(Relation::new(RelKind::Retracts, uw, ua)),
    ]);

    let eff = effective_status(&store, RetractionPolicy::default());
    assert!(
        eff.is_retracted(&ua),
        "a retraction was recorded and did not take"
    );
}

// -- TraceKind::follows_parents -----------------------------------------------------------

/// `trace --kind grounds` must not walk provenance, and `--kind parents` must not walk
/// grounds. With either predicate stuck at `true` the two kinds return the same thing, and the
/// flag stops meaning anything.
#[test]
fn the_trace_kinds_walk_different_edges() {
    let ground = claim("the measurement", vec![]);
    let ug = canonical_uid(&ground);
    let derived = claim("the conclusion", vec![ug]);
    let ud = canonical_uid(&derived);

    // A parent by attestation, which is a different edge family from `grounds`.
    let parent = claim("an earlier version", vec![]);
    let up = canonical_uid(&parent);
    let mut att = Attestation::new(ud, agent(), Op::Transformed, Rung::Model, hlc(2));
    att.parents.insert(up);

    let store = Store::from_records(vec![
        Record::Unit(ground),
        Record::Unit(derived),
        Record::Unit(parent),
        Record::Attestation(att),
    ]);

    let by_grounds = trace(&store, ud, TraceKind::Grounds, None);
    let by_parents = trace(&store, ud, TraceKind::Parents, None);
    let by_both = trace(&store, ud, TraceKind::Both, None);

    let uids =
        |l: &smysl_graph::lineage::Lineage| -> Vec<Uid> { l.nodes.iter().map(|n| n.uid).collect() };

    assert!(
        uids(&by_grounds).contains(&ug),
        "`--kind grounds` must reach the ground"
    );
    assert!(
        !uids(&by_grounds).contains(&up),
        "`--kind grounds` reached a provenance parent; the kinds are not distinguished"
    );
    assert!(
        uids(&by_parents).contains(&up),
        "`--kind parents` must reach the parent"
    );
    assert!(
        !uids(&by_parents).contains(&ug),
        "`--kind parents` reached a ground; the kinds are not distinguished"
    );
    assert!(
        uids(&by_both).contains(&ug) && uids(&by_both).contains(&up),
        "`--kind both` must reach both"
    );

    let _ = BTreeMap::<Uid, ()>::new();
}

// -- Lineage::is_empty --------------------------------------------------------------------

/// A trace that found ancestry — and a note on the case that does not exist.
///
/// `Lineage::is_empty` has two mutants and they are not the same kind of thing.
///
/// `-> true` is a real gap and this test closes it: a trace that reached two units is not
/// empty, and nothing said so before.
///
/// `-> false` is **equivalent and cannot be killed**, which is worth recording rather than
/// chasing. `trace` pushes a node for every uid in the frontier and the frontier starts with
/// the root, so every lineage it returns has at least one node. `Lineage` is
/// `#[non_exhaustive]` as of §1.1, so no consumer can construct an empty one either. The
/// method can only ever return `false`, and a test asserting that would be asserting a
/// tautology — the first draft of this test did exactly that, comparing `is_empty()` against
/// `len() == 0` on a lineage where both are trivially false, and it passed under the mutant.
///
/// Recorded the way 0.13 recorded `worse`'s `>=` and `Style::detect`'s `||`: a survivor no
/// test can kill is a fact about the code, and pretending otherwise costs more than it buys.
#[test]
fn a_lineage_that_found_ancestry_is_not_empty() {
    let ground = claim("something measured", vec![]);
    let g = canonical_uid(&ground);
    let derived = claim("something inferred from it", vec![g]);
    let d = canonical_uid(&derived);
    let store = Store::from_records(vec![Record::Unit(ground), Record::Unit(derived)]);

    let found = trace(&store, d, TraceKind::Grounds, None);
    assert!(
        !found.is_empty(),
        "a trace that reached two units is not empty"
    );
    assert_eq!(found.len(), 2);

    // Even a root the store has never heard of yields the root itself, which is the whole
    // reason the empty case is unreachable.
    let absent = trace(
        &store,
        canonical_uid(&claim("never stored", vec![])),
        TraceKind::Both,
        None,
    );
    assert_eq!(absent.len(), 1, "the root is always reported");
    assert!(!absent.is_empty());
}

// -- RetractionPlan::is_empty -------------------------------------------------------------

/// A plan that would remove something, and one that would not.
///
/// `retract` reports "nothing to do" from this predicate, so stuck at `true` it would tell a
/// user their retraction was a no-op while it removed things, and stuck at `false` it would
/// promise a blast radius that is empty.
#[test]
fn a_retraction_plan_is_empty_only_when_nothing_would_go() {
    let ground = claim("the evidence", vec![]);
    let g = canonical_uid(&ground);
    let dependent = claim("a claim resting on it", vec![g]);
    let d = canonical_uid(&dependent);
    let store = Store::from_records(vec![Record::Unit(ground), Record::Unit(dependent)]);

    // `Any` rather than the default `Origin`: this fixture carries no attestations, so
    // origin authority would refuse and the plan would be empty for a reason that has nothing
    // to do with the predicate under test.
    let real = smysl_graph::plan_retraction(
        &store,
        g,
        &[agent()],
        RetractionPolicy::default(),
        RetractionAuthority::Any,
    );
    assert!(
        !real.is_empty(),
        "retracting a ground that something rests on is not a no-op"
    );
    assert!(
        real.blast_radius.contains(&d),
        "the dependent is in the blast radius"
    );

    // A uid the store does not hold: nothing to retract, so nothing to plan.
    let nothing = smysl_graph::plan_retraction(
        &store,
        canonical_uid(&claim("not in the store", vec![])),
        &[agent()],
        RetractionPolicy::default(),
        RetractionAuthority::Any,
    );
    assert!(
        nothing.is_empty(),
        "planning a retraction of an absent unit removes nothing"
    );
    assert!(nothing.blast_radius.is_empty());
}
