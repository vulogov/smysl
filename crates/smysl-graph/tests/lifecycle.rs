//! The lifecycle of edges and disagreements (1.4 specification draft, §1–§5).
//!
//! Each test states the rule it holds, because the rules are new and the draft is where they
//! are argued: withdrawal, edge attestation, liveness under rule R, and resolution.

use smysl_core::{
    canonical_uid, AgentId, Attestation, Contention, ContentionId, ContentionStatus, Detected,
    DetectionKind, Hlc, KernelType, Op, Record, RelKind, Relation, Resolution, ResolutionTarget,
    Role, Rung, Status, Step, Thread, ThreadId, ThreadSchema, Uid, UnitCore, UnitCoreBuilder,
    Withdrawal,
};
use smysl_graph::merge::contention::detect;
use smysl_graph::{DetectionContext, Store};

fn reviewer() -> AgentId {
    AgentId::new("human:reviewer").unwrap()
}

fn model() -> AgentId {
    AgentId::new("model:gemini/flash-lite").unwrap()
}

fn at(ms: u64, a: AgentId) -> Hlc {
    Hlc::new(ms, 0, a)
}

fn claim(gist: &str) -> UnitCore {
    UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
        .build()
        .unwrap()
}

/// A claim, a unit rebutting it, and the edge between them.
fn rebutted() -> (Vec<Record>, Uid, Uid, Relation) {
    let c = claim("the pool saturated");
    let r = claim("the pool never exceeded half its size");
    let (uc, ur) = (canonical_uid(&c), canonical_uid(&r));
    let edge = Relation::new(RelKind::Rebuts, ur, uc);
    (
        vec![
            Record::Unit(c),
            Record::Unit(r),
            Record::Relation(edge.clone()),
        ],
        uc,
        ur,
        edge,
    )
}

fn retract(uid: Uid) -> Record {
    Record::Relation(Relation::new(RelKind::Retracts, uid, uid))
}

// -- §2 withdrawal -------------------------------------------------------------------------

/// A withdrawn edge is kept and not followed.
#[test]
fn a_withdrawn_rebuttal_no_longer_binds_and_its_record_stays() {
    let (mut records, uc, ur, edge) = rebutted();
    let before = Store::from_records(records.clone());
    assert_eq!(before.rebuttals_of(&uc), vec![ur]);

    records.push(Record::Withdrawal(Withdrawal::new(
        edge.uid(),
        reviewer(),
        at(10, reviewer()),
    )));
    let after = Store::from_records(records);

    assert!(after.rebuttals_of(&uc).is_empty(), "rule R still binds");
    assert!(after.relations_of_kind(&RelKind::Rebuts).is_empty());
    assert!(after.is_withdrawn(&edge));
    // Preserved: the relation is still a record of this store, and still a relation of it.
    assert!(after
        .iter()
        .any(|r| matches!(r, Record::Relation(e) if e == &edge)));
    assert_eq!(after.relation_by_id(&edge.uid()), Some(&edge));
}

/// Delivery order does not matter: a withdrawal that arrives before its edge takes effect when
/// the edge does, and the two orders converge (rule U).
#[test]
fn a_withdrawal_delivered_before_its_edge_takes_effect_and_converges() {
    let (records, uc, _, edge) = rebutted();
    let w = Record::Withdrawal(Withdrawal::new(edge.uid(), reviewer(), at(10, reviewer())));

    let mut first = vec![w.clone()];
    first.extend(records.clone());
    let mut last = records;
    last.push(w);

    let a = Store::from_records(first);
    let b = Store::from_records(last);
    assert!(a.rebuttals_of(&uc).is_empty());
    assert!(a.converged_with(&b));

    // And appending to a store that already holds the edge.
    let (records, uc, _, edge) = rebutted();
    let mut s = Store::from_records(records);
    s.append(&[Record::Withdrawal(Withdrawal::new(
        edge.uid(),
        reviewer(),
        at(10, reviewer()),
    ))])
    .unwrap();
    assert!(s.rebuttals_of(&uc).is_empty());
}

/// Withdrawing a retraction would un-retract a unit, which needs rules 1.4 does not have. The
/// withdrawal is preserved and changes nothing.
#[test]
fn a_retraction_cannot_be_withdrawn() {
    let c = claim("a claim somebody retracted");
    let uc = canonical_uid(&c);
    let retraction = Relation::new(RelKind::Retracts, uc, uc);
    let s = Store::from_records(vec![
        Record::Unit(c),
        Record::Relation(retraction.clone()),
        Record::Withdrawal(Withdrawal::new(
            retraction.uid(),
            reviewer(),
            at(10, reviewer()),
        )),
    ]);
    assert!(!s.is_withdrawn(&retraction));
    assert!(s.is_unfounded(&uc), "still retracted");
    assert_eq!(s.withdrawals().count(), 1, "and the withdrawal is kept");
}

// -- §3 who asserted an edge ---------------------------------------------------------------

/// An attestation whose uid is a rid attaches to that edge, whichever arrives first, and
/// attestations from different stores union on it.
#[test]
fn an_edge_carries_the_attestations_that_name_its_rid() {
    let (records, _, _, edge) = rebutted();
    let proposed = Attestation::new(
        edge.uid(),
        model(),
        Op::Imported,
        Rung::Model,
        at(1, model()),
    );
    let confirmed = Attestation::new(
        edge.uid(),
        reviewer(),
        Op::Authored,
        Rung::Document,
        at(2, reviewer()),
    );

    let mut early = vec![Record::Attestation(proposed.clone())];
    early.extend(records.clone());
    let mut s = Store::from_records(early);
    s.append(&[Record::Attestation(confirmed.clone())]).unwrap();

    let attached = &s.relation_by_id(&edge.uid()).unwrap().attestations;
    assert!(attached.contains(&proposed), "delivered before the edge");
    assert!(attached.contains(&confirmed), "appended after");
    assert_eq!(attached.len(), 2, "one edge, two assertions of it");
}

// -- §4 live rebuttal ----------------------------------------------------------------------

/// A retracted rebuttal no longer pins its claim. This is the finding that started 1.4: pack
/// went on pinning a claim as `C3` after the unit rebutting it was retracted.
#[test]
fn a_retracted_rebuttal_is_not_live() {
    let (mut records, uc, ur, edge) = rebutted();
    records.push(retract(ur));
    let s = Store::from_records(records);
    assert!(!s.is_live_rebuttal(s.relation_by_id(&edge.uid()).unwrap()));
    assert!(s.rebuttals_of(&uc).is_empty());
}

/// Orphaned is unfounded too: a rebuttal whose every ground was retracted is not live.
#[test]
fn an_orphaned_rebuttal_is_not_live() {
    let ground = claim("the dashboard read 50%");
    let ug = canonical_uid(&ground);
    let c = claim("the pool saturated");
    let uc = canonical_uid(&c);
    let r = UnitCoreBuilder::new(
        KernelType::Claim,
        "the pool never exceeded half its size",
        Status::Inferred,
    )
    .grounds([ug])
    .build()
    .unwrap();
    let ur = canonical_uid(&r);
    let s = Store::from_records(vec![
        Record::Unit(ground),
        Record::Unit(c),
        Record::Unit(r),
        Record::Relation(Relation::new(RelKind::Rebuts, ur, uc)),
        retract(ug),
    ]);
    assert!(s.is_unfounded(&ur));
    assert!(s.rebuttals_of(&uc).is_empty());
}

/// Supersession is not disbelief: a superseded rebuttal still stands.
#[test]
fn a_superseded_rebuttal_is_still_live() {
    let (mut records, uc, ur, _) = rebutted();
    let better = claim("the pool peaked at 48% of its size");
    let ub = canonical_uid(&better);
    records.push(Record::Unit(better));
    records.push(Record::Relation(Relation::new(RelKind::Supersedes, ub, ur)));
    let s = Store::from_records(records);
    assert_eq!(s.rebuttals_of(&uc), vec![ur]);
}

// -- §5 resolution -------------------------------------------------------------------------

/// A thread presenting both units makes the rebuttal a contention (detection kind 1).
fn threaded(records: &mut Vec<Record>, uc: Uid, ur: Uid) {
    records.push(Record::Thread(
        Thread::new(
            ThreadId::new("t/brief").unwrap(),
            ThreadSchema::Brief,
            reviewer(),
            "the incident",
            at(1, reviewer()),
        )
        .with_steps([Step::new(Role::BottomLine, uc), Step::new(Role::Risk, ur)]),
    ));
}

/// The id detection derives is the one in the core, so a resolution written against it names
/// what detection finds.
#[test]
fn a_resolution_names_a_detected_contention_by_its_derived_id() {
    let (mut records, uc, ur, _) = rebutted();
    threaded(&mut records, uc, ur);
    let open = Store::from_records(records.clone());
    let found = detect(&open, &DetectionContext::default());
    assert_eq!(found.len(), 1);
    let id = ContentionId::derive(DetectionKind::LiveRebuttal, &uc, &[ur, uc]);
    assert_eq!(found[0].id, id);
    assert_eq!(found[0].status, ContentionStatus::Open);

    records.push(Record::Resolution(Resolution::new(
        ResolutionTarget::Contention(id),
        reviewer(),
        at(20, reviewer()),
    )));
    let reviewed = Store::from_records(records);
    let found = detect(&reviewed, &DetectionContext::default());
    assert_eq!(found[0].status, ContentionStatus::Resolved);
}

/// A recorded contention stops pinning once resolved, and reads stale once its rebuttal is
/// withdrawn — whatever its own record says.
#[test]
fn a_recorded_contention_reads_resolved_or_stale_and_only_open_pins() {
    let (records, uc, ur, edge) = rebutted();
    let k = Contention::new(
        ContentionId::derive(DetectionKind::LiveRebuttal, &uc, &[uc, ur]),
        uc,
        vec![uc, ur],
        Detected::new(DetectionKind::LiveRebuttal, at(5, reviewer())),
    );
    let mut base = records;
    base.push(Record::Contention(k.clone()));

    let s = Store::from_records(base.clone());
    assert_eq!(s.contention_status(&k), ContentionStatus::Open);
    assert_eq!(s.open_contentions().count(), 1);

    let mut resolved = base.clone();
    resolved.push(Record::Resolution(Resolution::new(
        ResolutionTarget::Contention(k.id.clone()),
        reviewer(),
        at(20, reviewer()),
    )));
    let s = Store::from_records(resolved);
    assert_eq!(s.contention_status(&k), ContentionStatus::Resolved);
    assert_eq!(s.open_contentions().count(), 0);
    // A resolution decides nothing: the rebuttal is still live and still travels.
    assert_eq!(s.rebuttals_of(&uc), vec![ur]);

    let mut withdrawn = base;
    withdrawn.push(Record::Withdrawal(Withdrawal::new(
        edge.uid(),
        reviewer(),
        at(20, reviewer()),
    )));
    let s = Store::from_records(withdrawn);
    assert_eq!(s.contention_status(&k), ContentionStatus::Stale);
    assert_eq!(s.open_contentions().count(), 0);
}

/// A rebuttal nobody threaded can be resolved by its rid: it leaves the review queue and still
/// binds rule R.
#[test]
fn an_unthreaded_rebuttal_is_resolved_by_its_rid_and_still_binds() {
    let (mut records, uc, ur, edge) = rebutted();
    records.push(Record::Resolution(Resolution::new(
        ResolutionTarget::Relation(edge.uid()),
        reviewer(),
        at(20, reviewer()),
    )));
    let s = Store::from_records(records);
    assert!(s.is_resolved(&ResolutionTarget::Relation(edge.uid())));
    assert_eq!(s.rebuttals_of(&uc), vec![ur]);
}

/// Detection over a withdrawn edge or a retracted claim finds nothing.
#[test]
fn no_contention_is_detected_over_a_dead_rebuttal_or_a_retracted_claim() {
    let (mut records, uc, ur, edge) = rebutted();
    threaded(&mut records, uc, ur);

    let mut withdrawn = records.clone();
    withdrawn.push(Record::Withdrawal(Withdrawal::new(
        edge.uid(),
        reviewer(),
        at(20, reviewer()),
    )));
    assert!(detect(
        &Store::from_records(withdrawn),
        &DetectionContext::default()
    )
    .is_empty());

    let mut claim_retracted = records;
    claim_retracted.push(retract(uc));
    assert!(detect(
        &Store::from_records(claim_retracted),
        &DetectionContext::default()
    )
    .is_empty());
}

/// Withdrawals and resolutions are part of what merge converges on: the same records in any
/// order give the same state, and a store missing one does not.
#[test]
fn withdrawals_and_resolutions_are_in_the_converged_state() {
    let (records, _, _, edge) = rebutted();
    let w = Record::Withdrawal(Withdrawal::new(edge.uid(), reviewer(), at(10, reviewer())));
    let r = Record::Resolution(Resolution::new(
        ResolutionTarget::Relation(edge.uid()),
        reviewer(),
        at(11, reviewer()),
    ));

    let mut forward = records.clone();
    forward.extend([w.clone(), r.clone()]);
    let mut backward = vec![r.clone(), w.clone()];
    backward.extend(records.clone());
    let a = Store::from_records(forward);
    let b = Store::from_records(backward);
    assert!(a.converged_with(&b));

    let mut duplicated = records.clone();
    duplicated.extend([w.clone(), r.clone(), w, r]);
    assert!(a.converged_with(&Store::from_records(duplicated)));

    assert!(!a.converged_with(&Store::from_records(records)));
}
