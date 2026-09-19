//! Commitment: record type 13 (1.7), from inkhaven's SMYSL-1 RFC.
//!
//! A second axis beside `Status`. `Status` says how true a thing is and its order *is* rule M;
//! commitment says how settled it is, which for a body of work settled by authorial fiat is the
//! quantity that matters. The two are deliberately independent.
//!
//! The RFC proposed a field on `Unit`, "the same place `salience` already lives". `salience` does
//! not survive a store write — it has no record — so a commitment modelled on it would be lost the
//! moment a ledger was written to disk. Hence a record, which also says who settled it and when.

use smysl_core::{
    canonical_uid, AgentId, Commit, Commitment, Hlc, KernelType, Record, Status, Uid,
    UnitCoreBuilder,
};
use smysl_graph::Store;

fn agent(name: &str) -> AgentId {
    AgentId::new(name).unwrap()
}

fn hlc(ms: u64, agent: &AgentId) -> Hlc {
    Hlc::new(ms, 0, agent.clone())
}

fn claim(gist: &str) -> smysl_core::UnitCore {
    UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
        .build()
        .unwrap()
}

fn store_with(level: Commitment) -> (Store, Uid) {
    let u = claim("the villain's motive is grief");
    let uid = canonical_uid(&u);
    let a = agent("human:vu");
    (
        Store::from_records(vec![
            Record::Unit(u),
            Record::Commit(Commit::new(uid, level, a.clone(), hlc(1, &a))),
        ]),
        uid,
    )
}

/// Committing does not move a uid: that is the whole point of a record beside the unit.
#[test]
fn a_commitment_does_not_change_identity() {
    let u = claim("the villain's motive is grief");
    let uid = canonical_uid(&u);
    let (store, committed) = store_with(Commitment::Canonical);
    assert_eq!(uid, committed);
    assert!(store.contains_uid(&uid));
    assert_eq!(store.commitment_of(&uid), Some(Commitment::Canonical));
}

/// A unit nobody has committed to has no commitment — which is not `Floated`. Floated is a
/// decision; silence is not.
#[test]
fn silence_is_not_a_commitment() {
    let u = claim("an uncommitted thought");
    let uid = canonical_uid(&u);
    let store = Store::from_records(vec![Record::Unit(u)]);
    assert_eq!(store.commitment_of(&uid), None);
    assert!(store.commits_of(&uid).is_empty());
}

/// The latest commitment wins, by a total order, so the answer does not depend on arrival order.
#[test]
fn the_latest_commitment_wins_whatever_order_it_arrives_in() {
    let u = claim("the ending is a reconciliation");
    let uid = canonical_uid(&u);
    let vu = agent("human:vu");
    let early = Commit::new(uid, Commitment::Canonical, vu.clone(), hlc(1, &vu));
    let late = Commit::new(uid, Commitment::Retconned, vu.clone(), hlc(2, &vu));

    for order in [
        vec![early.clone(), late.clone()],
        vec![late.clone(), early.clone()],
    ] {
        let mut records = vec![Record::Unit(claim("the ending is a reconciliation"))];
        records.extend(order.into_iter().map(Record::Commit));
        let store = Store::from_records(records);
        assert_eq!(
            store.commitment_of(&uid),
            Some(Commitment::Retconned),
            "a later commitment replaces an earlier one, in either arrival order"
        );
        assert_eq!(store.commits_of(&uid).len(), 2, "both are kept as history");
    }
}

/// Two agents at the same instant are ordered by agent, so the answer is still a function of the
/// set rather than of the sequence.
#[test]
fn a_tie_is_broken_by_agent_not_by_order() {
    let u = claim("the villain survives");
    let uid = canonical_uid(&u);
    let (a, b) = (agent("human:aa"), agent("human:zz"));
    let one = Commit::new(uid, Commitment::Drafted, a.clone(), hlc(7, &a));
    let two = Commit::new(uid, Commitment::Canonical, b.clone(), hlc(7, &b));

    let forward = Store::from_records(vec![
        Record::Unit(claim("the villain survives")),
        Record::Commit(one.clone()),
        Record::Commit(two.clone()),
    ]);
    let backward = Store::from_records(vec![
        Record::Unit(claim("the villain survives")),
        Record::Commit(two),
        Record::Commit(one),
    ]);
    assert_eq!(forward.commitment_of(&uid), backward.commitment_of(&uid));
    assert_eq!(forward.commitment_of(&uid), Some(Commitment::Canonical));
}

/// Retraction of a commitment has to be possible, which is why the *latest* wins rather than the
/// highest: taking the maximum would make `Retconned` unreachable.
#[test]
fn a_commitment_can_be_walked_back() {
    let u = claim("the sister is the traitor");
    let uid = canonical_uid(&u);
    let vu = agent("human:vu");
    let store = Store::from_records(vec![
        Record::Unit(u),
        Record::Commit(Commit::new(
            uid,
            Commitment::Canonical,
            vu.clone(),
            hlc(1, &vu),
        )),
        Record::Commit(Commit::new(
            uid,
            Commitment::Drafted,
            vu.clone(),
            hlc(2, &vu),
        )),
    ]);
    assert_eq!(store.commitment_of(&uid), Some(Commitment::Drafted));
}

/// The wire is where this differs from `salience`, so it is asserted rather than assumed.
#[test]
fn a_commitment_survives_a_store_write() {
    use smysl_core::{from_cbor_seq, to_cbor_seq};

    let (store, uid) = store_with(Commitment::Canonical);
    let records: Vec<Record> = store.iter().cloned().collect();
    let bytes = to_cbor_seq(&records);
    let (back, _) = from_cbor_seq(&bytes).unwrap();
    assert_eq!(back, records, "the records round-trip");

    let reopened = Store::from_records(back);
    assert_eq!(
        reopened.commitment_of(&uid),
        Some(Commitment::Canonical),
        "and the store built from them still knows how settled it is"
    );
}

/// `units_at_commitment` is what a caller filters a ledger with.
#[test]
fn units_can_be_listed_by_how_settled_they_are() {
    let (a, b) = (claim("settled"), claim("still floating"));
    let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
    let vu = agent("human:vu");
    let store = Store::from_records(vec![
        Record::Unit(a),
        Record::Unit(b),
        Record::Commit(Commit::new(
            ua,
            Commitment::Canonical,
            vu.clone(),
            hlc(1, &vu),
        )),
        Record::Commit(Commit::new(
            ub,
            Commitment::Floated,
            vu.clone(),
            hlc(1, &vu),
        )),
    ]);
    assert_eq!(store.units_at_commitment(Commitment::Canonical), vec![ua]);
    assert_eq!(store.units_at_commitment(Commitment::Floated), vec![ub]);
    assert!(store.units_at_commitment(Commitment::Retconned).is_empty());
}

/// The two axes do not constrain each other: a floated idea may be measured, and a canonical
/// decision may be speculative.
#[test]
fn commitment_and_status_are_independent() {
    let measured = UnitCoreBuilder::new(
        KernelType::Evidence,
        "p95 tripled after the rollout",
        Status::Cited,
    )
    .source(smysl_core::SourceRef::new(
        smysl_core::SourceKind::Doc,
        "postmortems/92",
    ))
    .build()
    .unwrap();
    let uid = canonical_uid(&measured);
    let vu = agent("human:vu");
    let store = Store::from_records(vec![
        Record::Unit(measured),
        Record::Commit(Commit::new(
            uid,
            Commitment::Floated,
            vu.clone(),
            hlc(1, &vu),
        )),
    ]);
    assert_eq!(store.commitment_of(&uid), Some(Commitment::Floated));
    assert_eq!(store.get(&uid).unwrap().core.status, Status::Cited);
}

// --- CommitmentFork (SMY-W058) ------------------------------------------------------------

use smysl_core::DetectionKind;
use smysl_graph::merge::contention::{detect, DetectionContext};

fn forks(store: &Store) -> Vec<smysl_core::Contention> {
    detect(store, &DetectionContext::default())
        .into_iter()
        .filter(|c| c.detected.kind == DetectionKind::CommitmentFork)
        .collect()
}

/// Two agents whose latest words differ is a disagreement a person should see.
#[test]
fn two_agents_disagreeing_is_a_fork() {
    let u = claim("the sister is the traitor");
    let uid = canonical_uid(&u);
    let (vu, ed) = (agent("human:vu"), agent("human:ed"));
    let store = Store::from_records(vec![
        Record::Unit(u),
        Record::Commit(Commit::new(
            uid,
            Commitment::Canonical,
            vu.clone(),
            hlc(1, &vu),
        )),
        Record::Commit(Commit::new(
            uid,
            Commitment::Floated,
            ed.clone(),
            hlc(1, &ed),
        )),
    ]);
    let found = forks(&store);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].over, uid);
    assert!(found[0].is_open());

    // And merge still answers, deterministically, for machinery that needs one value.
    assert!(store.commitment_of(&uid).is_some());
}

/// One agent changing their mind is a revision, not a disagreement. Reporting it would make the
/// queue useless to the person doing the work.
#[test]
fn one_agent_changing_their_mind_is_not_a_fork() {
    let u = claim("the ending is a reconciliation");
    let uid = canonical_uid(&u);
    let vu = agent("human:vu");
    let store = Store::from_records(vec![
        Record::Unit(u),
        Record::Commit(Commit::new(
            uid,
            Commitment::Floated,
            vu.clone(),
            hlc(1, &vu),
        )),
        Record::Commit(Commit::new(
            uid,
            Commitment::Drafted,
            vu.clone(),
            hlc(2, &vu),
        )),
        Record::Commit(Commit::new(
            uid,
            Commitment::Canonical,
            vu.clone(),
            hlc(3, &vu),
        )),
    ]);
    assert!(forks(&store).is_empty());
}

/// Two agents who agree are not a fork, however many times they said it.
#[test]
fn agreement_is_not_a_fork() {
    let u = claim("the villain survives");
    let uid = canonical_uid(&u);
    let (vu, ed) = (agent("human:vu"), agent("human:ed"));
    let store = Store::from_records(vec![
        Record::Unit(u),
        Record::Commit(Commit::new(
            uid,
            Commitment::Canonical,
            vu.clone(),
            hlc(1, &vu),
        )),
        Record::Commit(Commit::new(
            uid,
            Commitment::Canonical,
            ed.clone(),
            hlc(2, &ed),
        )),
        Record::Commit(Commit::new(
            uid,
            Commitment::Canonical,
            vu.clone(),
            hlc(3, &vu),
        )),
    ]);
    assert!(forks(&store).is_empty());
}

/// An agent who came round later is not a fork either: only the latest word per agent counts.
#[test]
fn a_disagreement_that_was_settled_is_not_a_fork() {
    let u = claim("the map is a forgery");
    let uid = canonical_uid(&u);
    let (vu, ed) = (agent("human:vu"), agent("human:ed"));
    let store = Store::from_records(vec![
        Record::Unit(u),
        Record::Commit(Commit::new(
            uid,
            Commitment::Canonical,
            vu.clone(),
            hlc(1, &vu),
        )),
        Record::Commit(Commit::new(
            uid,
            Commitment::Floated,
            ed.clone(),
            hlc(2, &ed),
        )),
        // ed comes round.
        Record::Commit(Commit::new(
            uid,
            Commitment::Canonical,
            ed.clone(),
            hlc(9, &ed),
        )),
    ]);
    assert!(forks(&store).is_empty());
}

/// Detection is a function of the merged set, so it cannot depend on arrival order (rule U).
#[test]
fn the_fork_is_found_whatever_order_the_records_arrive_in() {
    let (vu, ed) = (agent("human:vu"), agent("human:ed"));
    let build = |flip: bool| {
        let u = claim("the sister is the traitor");
        let uid = canonical_uid(&u);
        let a = Record::Commit(Commit::new(
            uid,
            Commitment::Canonical,
            vu.clone(),
            hlc(1, &vu),
        ));
        let b = Record::Commit(Commit::new(
            uid,
            Commitment::Drafted,
            ed.clone(),
            hlc(1, &ed),
        ));
        let mut rs = vec![Record::Unit(u)];
        if flip {
            rs.extend([b, a]);
        } else {
            rs.extend([a, b]);
        }
        Store::from_records(rs)
    };
    let one = forks(&build(false));
    let two = forks(&build(true));
    assert_eq!(one.len(), 1);
    assert_eq!(
        one.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
        two.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
        "the same union gives the same contention, id included"
    );
}
