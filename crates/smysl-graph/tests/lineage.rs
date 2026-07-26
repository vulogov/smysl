//! The SM-P7 gate.
//!
//! A synthetic five-hop chain with a **known** mutation script, so the expected partition
//! is written down rather than derived from the thing under test. If `diff --hop 0..5`
//! reports anything other than the script, either the script or the implementation is
//! wrong, and the script is the shorter of the two to read.
//!
//! This is F3 made falsifiable. In a prose pipeline, nothing at hop 5 distinguishes what
//! survived from hop 0 from what a model invented at hop 3.

use std::collections::BTreeSet;

use smysl_core::{
    canonical_uid, AgentId, Attestation, Hlc, KernelType, Op, Record, RelKind, Relation, Rung,
    SourceKind, SourceRef, Status, Uid, UnitCore, UnitCoreBuilder,
};
use smysl_graph::{diff, hop_diff, membership, trace, RecipeChangeKind, Store, TraceKind, Via};

fn agent(n: u32) -> AgentId {
    AgentId::new(format!("model:vendor{n}/m")).unwrap()
}

fn evidence(gist: &str) -> UnitCore {
    UnitCoreBuilder::new(KernelType::Evidence, gist, Status::Measured)
        .source(SourceRef::new(SourceKind::Metric, "m"))
        .build()
        .unwrap()
}

fn claim(gist: &str, grounds: Vec<Uid>) -> UnitCore {
    UnitCoreBuilder::new(KernelType::Claim, gist, Status::Inferred)
        .grounds(grounds)
        .build()
        .unwrap()
}

fn attest(uid: Uid, hop: u32) -> Record {
    let a = agent(hop);
    Record::Attestation(
        Attestation::new(
            uid,
            a.clone(),
            Op::Authored,
            Rung::Model,
            Hlc::new(hop as u64, 0, a),
        )
        .at_hop(hop),
    )
}

/// Attest an edge, which is what places a supersession or a retraction in time.
fn attest_edge(rel: Relation, hop: u32) -> Record {
    let a = agent(hop);
    let uid = rel.uid();
    Record::Relation(
        rel.with_attestation(
            Attestation::new(
                uid,
                a.clone(),
                Op::Authored,
                Rung::Model,
                Hlc::new(hop as u64, 0, a),
            )
            .at_hop(hop),
        ),
    )
}

fn attest_with_recipe(uid: Uid, hop: u32, recipe: [u8; 32], family: [u8; 32]) -> Record {
    let a = agent(hop);
    Record::Attestation(
        Attestation::new(
            uid,
            a.clone(),
            Op::Transformed,
            Rung::Model,
            Hlc::new(hop as u64, 0, a),
        )
        .at_hop(hop)
        .with_recipe(recipe, family),
    )
}

/// What the script did, so the assertions can be written against intent rather than
/// against whatever the code happened to produce.
struct Chain {
    store: Store,
    /// hop 0, untouched for all five hops.
    stable: Uid,
    /// hop 0, superseded at hop 2.
    revised: Uid,
    /// the successor, born at hop 2.
    revision: Uid,
    /// hop 0, retracted at hop 3.
    withdrawn: Uid,
    /// hop 0, still standing but grounded on `withdrawn`.
    dependent: Uid,
    /// born at hop 1.
    added_early: Uid,
    /// born at hop 4.
    added_late: Uid,
}

/// The mutation script:
///
/// | hop | what happens |
/// |---|---|
/// | 0 | `stable`, `revised`, `withdrawn`, `dependent` are authored |
/// | 1 | `added_early` appears |
/// | 2 | `revision` appears and supersedes `revised` - and it came from a different vendor running the same prompt |
/// | 3 | `withdrawn` is retracted |
/// | 4 | `added_late` appears |
/// | 5 | nothing |
fn build() -> Chain {
    let stable = evidence("a measurement that nobody touches");
    let s_uid = canonical_uid(&stable);
    let revised = evidence("a measurement that gets revised");
    let r_uid = canonical_uid(&revised);
    let withdrawn = evidence("a measurement that gets withdrawn");
    let w_uid = canonical_uid(&withdrawn);
    let dependent = claim("a claim resting on the withdrawn measurement", vec![w_uid]);
    let d_uid = canonical_uid(&dependent);

    let added_early = claim("a claim added at hop one", vec![s_uid]);
    let e_uid = canonical_uid(&added_early);
    let revision = evidence("a measurement that gets revised, corrected");
    let rev_uid = canonical_uid(&revision);
    let added_late = claim("a claim added at hop four", vec![s_uid]);
    let l_uid = canonical_uid(&added_late);

    let family = [0x5A; 32];
    let records = vec![
        // hop 0
        Record::Unit(stable),
        attest_with_recipe(s_uid, 0, [1; 32], family),
        Record::Unit(revised),
        attest_with_recipe(r_uid, 0, [1; 32], family),
        Record::Unit(withdrawn),
        attest(w_uid, 0),
        Record::Unit(dependent),
        attest(d_uid, 0),
        // hop 1
        Record::Unit(added_early),
        attest(e_uid, 1),
        // hop 2: a different vendor re-ran the same prompt
        Record::Unit(revision),
        attest_with_recipe(rev_uid, 2, [2; 32], family),
        attest_edge(Relation::new(RelKind::Supersedes, rev_uid, r_uid), 2),
        // hop 3
        attest_edge(Relation::new(RelKind::Retracts, w_uid, w_uid), 3),
        // hop 4
        Record::Unit(added_late),
        attest(l_uid, 4),
    ];

    Chain {
        store: Store::from_records(records),
        stable: s_uid,
        revised: r_uid,
        revision: rev_uid,
        withdrawn: w_uid,
        dependent: d_uid,
        added_early: e_uid,
        added_late: l_uid,
    }
}

/// The gate.
#[test]
fn the_five_hop_partition_matches_the_script_exactly() {
    let c = build();
    let d = hop_diff(&c.store, 0, 5, false);

    let mut expect_survived = vec![c.stable, c.dependent];
    expect_survived.sort();
    assert_eq!(d.survived, expect_survived, "survived");

    assert_eq!(d.superseded, vec![(c.revised, c.revision)], "superseded");
    assert_eq!(d.retracted, vec![c.withdrawn], "retracted");

    let mut expect_added = vec![c.added_early, c.revision, c.added_late];
    expect_added.sort();
    assert_eq!(d.added, expect_added, "added");

    // Every hop-0 unit is in exactly one partition, and nothing is double counted.
    assert_eq!(d.survived.len() + d.superseded.len() + d.retracted.len(), 4);
    assert_eq!(d.total(), 7);
}

#[test]
fn the_partitions_are_disjoint() {
    let c = build();
    let d = hop_diff(&c.store, 0, 5, false);
    let mut seen: BTreeSet<Uid> = BTreeSet::new();
    for u in d
        .survived
        .iter()
        .chain(d.retracted.iter())
        .chain(d.added.iter())
        .chain(d.superseded.iter().map(|(old, _)| old))
    {
        assert!(seen.insert(*u), "{u} appears in two partitions");
    }
}

/// The window matters: a change at hop 3 is invisible from hop 0..2.
#[test]
fn a_narrower_window_sees_less() {
    let c = build();
    let d = hop_diff(&c.store, 0, 1, false);
    assert_eq!(d.added, vec![c.added_early]);
    assert!(d.retracted.is_empty(), "the retraction is at hop 3");
    assert!(d.superseded.is_empty(), "the supersession is at hop 2");
    assert_eq!(
        d.survived.len(),
        4,
        "everything from hop 0 is still standing"
    );
}

/// The retraction is dated by its own attestation, so it enters the window at hop 3 and
/// not before.
#[test]
fn the_retraction_enters_the_window_at_its_own_hop() {
    let c = build();
    assert!(hop_diff(&c.store, 0, 2, false).retracted.is_empty());
    assert_eq!(hop_diff(&c.store, 0, 3, false).retracted, vec![c.withdrawn]);
}

#[test]
fn the_supersession_appears_only_once_its_hop_is_in_range() {
    let c = build();
    assert!(hop_diff(&c.store, 0, 1, false).superseded.is_empty());
    assert_eq!(
        hop_diff(&c.store, 0, 2, false).superseded,
        vec![(c.revised, c.revision)]
    );
}

#[test]
fn survival_rate_falls_as_the_window_widens() {
    let c = build();
    let early = hop_diff(&c.store, 0, 1, false).survival_rate();
    let late = hop_diff(&c.store, 0, 5, false).survival_rate();
    assert_eq!(early, 1.0);
    assert_eq!(late, 0.5, "two of four hop-0 units survived");
    assert!(late < early);
}

/// Attribution: who did what, which is the half of F3 that "what changed" does not answer.
#[test]
fn every_change_is_attributed_to_an_agent() {
    let c = build();
    let d = hop_diff(&c.store, 0, 5, true);

    assert_eq!(d.by_agent[&agent(1)].added, 1, "vendor1 added at hop 1");
    assert_eq!(
        d.by_agent[&agent(2)].superseded,
        1,
        "vendor2 revised at hop 2"
    );
    assert_eq!(d.by_agent[&agent(4)].added, 1, "vendor4 added at hop 4");
    assert_eq!(
        d.by_agent[&agent(2)].added,
        1,
        "the revision is also an addition"
    );
    assert!(
        !d.by_agent.contains_key(&agent(3)),
        "hop 3 authored nothing"
    );
}

/// D-8: the revision came from a different vendor running the *same* prompt, so this is a
/// provider change rather than a prompt change. Told apart only by the attestation.
#[test]
fn the_revision_is_reported_as_a_provider_change_not_a_prompt_change() {
    let c = build();
    let d = hop_diff(&c.store, 0, 5, true);
    assert_eq!(d.recipe_changes.len(), 1);
    assert_eq!(d.recipe_changes[0].uid, c.revision);
    assert_eq!(
        d.recipe_changes[0].kind,
        RecipeChangeKind::ProviderChanged,
        "same family, different recipe"
    );
}

// ---------------------------------------------------------------------------
// trace
// ---------------------------------------------------------------------------

/// The question prose cannot answer: at hop 5, where did this come from?
#[test]
fn a_unit_at_hop_five_traces_back_to_hop_zero() {
    let c = build();
    let l = trace(&c.store, c.added_late, TraceKind::Grounds, None);
    assert_eq!(l.len(), 2);
    assert_eq!(l.nodes[0].uid, c.added_late);
    assert_eq!(l.nodes[0].hop, Some(4));

    let root = l.nodes.iter().find(|n| n.uid == c.stable).unwrap();
    assert_eq!(root.hop, Some(0), "the trace reaches back to hop 0");
    assert_eq!(root.via, Via::Grounds);
}

#[test]
fn a_revision_traces_back_to_what_it_replaced() {
    let c = build();
    let l = trace(&c.store, c.revision, TraceKind::Parents, None);
    let old = l.nodes.iter().find(|n| n.uid == c.revised).unwrap();
    assert_eq!(old.via, Via::Supersedes);
    assert_eq!(old.hop, Some(0));
    assert_eq!(old.depth, 1);
}

#[test]
fn a_trace_names_every_agent_in_the_ancestry() {
    let c = build();
    let l = trace(&c.store, c.added_late, TraceKind::Both, None);
    let agents: BTreeSet<String> = l.agents().into_iter().map(ToString::to_string).collect();
    assert!(agents.contains("model:vendor4/m"), "the author");
    assert!(
        agents.contains("model:vendor0/m"),
        "and the source it rests on"
    );
}

#[test]
fn the_dependent_still_traces_to_the_withdrawn_unit() {
    let c = build();
    let l = trace(&c.store, c.dependent, TraceKind::Grounds, None);
    assert!(
        l.nodes.iter().any(|n| n.uid == c.withdrawn),
        "a retraction removes belief, not history"
    );
}

// ---------------------------------------------------------------------------
// diff between stores
// ---------------------------------------------------------------------------

#[test]
fn a_store_diff_finds_what_one_side_is_missing() {
    let c = build();
    let partial = Store::from_records(c.store.iter().take(4).cloned().collect::<Vec<_>>());
    let d = diff(&c.store, &partial);
    assert!(!d.is_identical());
    assert!(d.only_in_b.is_empty(), "the partial store invents nothing");
    assert!(d.only_in_a.contains(&c.added_late));
    assert!(d.common.contains(&c.stable));
}

// ---------------------------------------------------------------------------
// view and bundle
// ---------------------------------------------------------------------------

#[test]
fn membership_is_computed_from_roots_not_stored() {
    let c = build();
    let m = membership(&c.store, &[c.dependent].into_iter().collect());
    assert!(m.contains(&c.dependent));
    assert!(m.contains(&c.withdrawn), "its ground is in the view too");
    assert!(!m.contains(&c.added_late), "an unrelated unit is not");
}

#[test]
fn a_bundle_of_a_view_is_self_contained() {
    use smysl_core::{View, ViewId};
    let c = build();
    let view = View::new(ViewId::new("v/x").unwrap(), "i").with_roots([c.added_late]);
    let bytes = c.store.bundle_with(&view, true);
    let (records, n) = smysl_core::from_cbor_seq(&bytes).unwrap();
    assert_eq!(n, bytes.len());

    let bundled = Store::from_records(records);
    assert!(bundled.contains_uid(&c.added_late));
    assert!(
        bundled.contains_uid(&c.stable),
        "its ground travelled with it"
    );
    let mut report = smysl_core::Report::new();
    bundled.report_dangling(&mut report);
    assert!(report.is_empty(), "{report}");
}

/// A retracted unit is dropped from a bundle - unless something surviving still points at
/// it, because a dangling reference is worse than a unit somebody stopped believing.
#[test]
fn a_bundle_keeps_a_retracted_unit_that_is_still_referenced() {
    use smysl_core::{View, ViewId};
    let c = build();
    let view = View::new(ViewId::new("v/x").unwrap(), "i").with_roots([c.dependent]);

    let lean = Store::from_records(
        smysl_core::from_cbor_seq(&c.store.bundle_with(&view, false))
            .unwrap()
            .0,
    );
    assert!(
        lean.contains_uid(&c.withdrawn),
        "dropping it would leave the dependent's ground dangling"
    );

    let mut report = smysl_core::Report::new();
    lean.report_dangling(&mut report);
    assert!(
        report.is_empty(),
        "a bundle must stay self-contained: {report}"
    );
}

#[test]
fn a_bundle_drops_a_retracted_unit_nothing_needs() {
    use smysl_core::{View, ViewId};
    let e = evidence("a lone measurement");
    let ue = canonical_uid(&e);
    let keep = evidence("an unrelated measurement");
    let uk = canonical_uid(&keep);
    let store = Store::from_records(vec![
        Record::Unit(e),
        attest(ue, 0),
        Record::Unit(keep),
        attest(uk, 0),
        Record::Relation(Relation::new(RelKind::Retracts, ue, ue)),
    ]);
    let view = View::new(ViewId::new("v/x").unwrap(), "i").with_roots([ue, uk]);

    let lean = Store::from_records(
        smysl_core::from_cbor_seq(&store.bundle_with(&view, false))
            .unwrap()
            .0,
    );
    assert!(!lean.contains_uid(&ue), "nothing needed it");
    assert!(lean.contains_uid(&uk));

    let full = Store::from_records(
        smysl_core::from_cbor_seq(&store.bundle_with(&view, true))
            .unwrap()
            .0,
    );
    assert!(full.contains_uid(&ue), "--include-retracted keeps it");
}

#[test]
fn lineage_is_deterministic_over_the_whole_chain() {
    let c = build();
    assert_eq!(
        hop_diff(&c.store, 0, 5, true),
        hop_diff(&c.store, 0, 5, true)
    );
    assert_eq!(
        trace(&c.store, c.added_late, TraceKind::Both, None),
        trace(&c.store, c.added_late, TraceKind::Both, None)
    );
}
