//! RFC SMYSL-8 TR-P1: admission, and what it changes in salience and origin authority.
//!
//! The phase's gate: with no policy every result is what it was; a unit corroborated by four
//! agents, two of them invented, has corroboration 1.0 under no policy and the signed groups'
//! share under `Strict`; a forged attestation claiming an honest group's ancestry collapses
//! nothing; origin authority refuses an agent whose only attestation is forged; and an
//! attestation held with no log record of its own is excluded under `Strict` and admitted under
//! `Advisory` (R1-2).

use std::collections::BTreeSet;

use smysl_core::{AgentId, Attestation, Hlc, Op, Record, RelKind, Relation, Rung, Status, Uid};
use smysl_graph::{
    plan_retraction, plan_retraction_admitted, salience, AttestationAdmission, RetractionAuthority,
    RetractionPolicy, SalienceRequest, Store,
};
use smysl_sign::{
    admission, sign_record, verify_store, DetachedSignature, MemoryKeyResolver, ResolvedKey,
    SigningKey, VerificationMode,
};

fn agent(s: &str) -> AgentId {
    s.parse().unwrap()
}

fn key(name: &str, byte: u8) -> SigningKey {
    SigningKey::from_seed(agent(name), [byte; 32])
}

const DOC: &str = "@doc smysl/0.1 { id: v/t }\n\n\
    @claim c/a { status: speculative }\n~ The pool is small.\n\n\
    @claim c/b { status: speculative }\n~ The cache is cold.\n";

fn units() -> Vec<Record> {
    smysl_core::surface::parse_surface(DOC)
        .unwrap()
        .records
        .into_iter()
        .filter(|r| matches!(r, Record::Unit(_)))
        .collect()
}

fn uid_of(r: &Record) -> Uid {
    smysl_core::canonical_uid(r.as_unit().unwrap())
}

fn att(uid: Uid, who: &str, parents: &[Uid]) -> Attestation {
    let a = agent(who);
    Attestation::new(
        uid,
        a.clone(),
        Op::Authored,
        Rung::Document,
        Hlc::new(1_000, 0, a),
    )
    .with_parents(parents.iter().copied().collect())
}

fn resolver(keys: &[&SigningKey]) -> MemoryKeyResolver {
    keys.iter().fold(MemoryKeyResolver::new(), |r, k| {
        r.with(ResolvedKey::active(k.agent().clone(), k.public_key()))
    })
}

fn sign(k: &SigningKey, a: &Attestation) -> DetachedSignature {
    sign_record(k, &Record::Attestation(a.clone())).unwrap()
}

/// A signature whose last byte is flipped: present, and wrong.
fn forge(k: &SigningKey, a: &Attestation) -> DetachedSignature {
    let mut s = sign(k, a);
    s.signature[63] ^= 0x01;
    s
}

fn corroboration(store: &Store, uid: Uid, adm: AttestationAdmission) -> (f32, Vec<String>) {
    let r = salience(store, &SalienceRequest::default().with_admission(adm));
    let t = &r.terms[&uid];
    (t.corroboration, t.excluded_groups.clone())
}

/// Two signed agents and two invented ones attest one unit.
fn four_attesters() -> (Store, Uid, Vec<DetachedSignature>, MemoryKeyResolver) {
    let (alice, bob) = (key("human:alice", 1), key("human:bob", 2));
    let u = units();
    let uid = uid_of(&u[0]);
    let mut records = u;
    let mut sigs = Vec::new();
    for (who, k) in [("human:alice", Some(&alice)), ("human:bob", Some(&bob))] {
        let a = att(uid, who, &[]);
        sigs.push(sign(k.unwrap(), &a));
        records.push(Record::Attestation(a));
    }
    for who in ["model:fake/one", "model:fake/two"] {
        records.push(Record::Attestation(att(uid, who, &[])));
    }
    (
        Store::from_records(records),
        uid,
        sigs,
        resolver(&[&alice, &bob]),
    )
}

#[test]
fn no_policy_changes_nothing() {
    let (store, uid, _, _) = four_attesters();
    let plain = salience(&store, &SalienceRequest::default());
    let all = salience(
        &store,
        &SalienceRequest::default().with_admission(AttestationAdmission::all()),
    );
    assert_eq!(plain, all);
    assert_eq!(plain.terms[&uid].corroboration, 1.0);
    assert!(plain.terms[&uid].excluded_groups.is_empty());
}

#[test]
fn strict_counts_only_the_signed_groups() {
    let (store, uid, sigs, keys) = four_attesters();
    let strict = admission(&store, &sigs, &keys, &VerificationMode::Strict);
    let (corr, excluded) = corroboration(&store, uid, strict);
    assert_eq!(corr, 0.5, "two signed groups of the cap of four");
    assert_eq!(excluded, ["model:fake/one", "model:fake/two"]);

    // Advisory tolerates the unsigned, as `verify` does.
    let advisory = admission(&store, &sigs, &keys, &VerificationMode::Advisory);
    assert_eq!(corroboration(&store, uid, advisory).0, 1.0);

    // Selective over `model:` excludes exactly the invented model agents.
    let selective = VerificationMode::selective(["model:"], None::<Status>);
    let sel = admission(&store, &sigs, &keys, &selective);
    assert_eq!(corroboration(&store, uid, sel).0, 0.5);
}

/// What `verify --mode M` reports as an error is exactly what `admission` under M excludes.
#[test]
fn admission_is_the_complement_of_verify() {
    let (store, _, sigs, keys) = four_attesters();
    for mode in [
        VerificationMode::Advisory,
        VerificationMode::selective(["model:"], None::<Status>),
        VerificationMode::Strict,
    ] {
        let report = verify_store(&store, &sigs, &keys, &mode);
        let adm = admission(&store, &sigs, &keys, &mode);
        for (h, admitted) in &adm.judged {
            assert_eq!(*admitted, !report.failing.contains(h), "{mode:?}");
        }
    }
}

#[test]
fn a_forged_attestation_cannot_collapse_an_honest_group() {
    let (alice, aaron) = (key("human:alice", 1), key("human:aaron", 3));
    let u = units();
    let (uid, parent) = (uid_of(&u[0]), uid_of(&u[1]));
    // `human:aaron` sorts first, so under no policy his group claims the shared ancestry and
    // alice's honest group is discarded as dependent.
    let forged = att(uid, "human:aaron", &[parent]);
    let honest = att(uid, "human:alice", &[parent]);
    let sigs = vec![forge(&aaron, &forged), sign(&alice, &honest)];
    let mut records = u;
    records.push(Record::Attestation(forged));
    records.push(Record::Attestation(honest));
    let store = Store::from_records(records);
    let keys = resolver(&[&alice, &aaron]);

    let plain = salience(&store, &SalienceRequest::default());
    assert_eq!(plain.terms[&uid].groups, ["human:aaron"]);
    assert_eq!(plain.terms[&uid].dependent_groups, ["human:alice"]);

    let adm = admission(&store, &sigs, &keys, &VerificationMode::Advisory);
    let r = salience(&store, &SalienceRequest::default().with_admission(adm));
    let t = &r.terms[&uid];
    assert_eq!(t.groups, ["human:alice"], "the honest group is counted");
    assert!(t.dependent_groups.is_empty());
    assert_eq!(t.excluded_groups, ["human:aaron"]);
}

#[test]
fn origin_refuses_an_agent_whose_only_attestation_is_forged() {
    let mallory = key("human:mallory", 4);
    let u = units();
    let uid = uid_of(&u[0]);
    let a = att(uid, "human:mallory", &[]);
    let sigs = vec![forge(&mallory, &a)];
    let mut records = u;
    records.push(Record::Attestation(a));
    let store = Store::from_records(records);
    let keys = resolver(&[&mallory]);
    let who = [agent("human:mallory")];
    let (policy, origin) = (RetractionPolicy::default(), RetractionAuthority::Origin);

    assert!(plan_retraction(&store, uid, &who, policy, origin).authorised);

    let adm = admission(&store, &sigs, &keys, &VerificationMode::Advisory);
    let plan = plan_retraction_admitted(&store, uid, &who, policy, origin, &adm);
    assert!(!plan.authorised);
    assert!(
        plan.refusal.as_deref().unwrap().contains("not admitted"),
        "{:?}",
        plan.refusal
    );

    // An agent who never attested is refused as before, with the old reason.
    let stranger = [agent("human:stranger")];
    let plan = plan_retraction_admitted(&store, uid, &stranger, policy, origin, &adm);
    assert!(plan.refusal.as_deref().unwrap().contains("none of the 1"));
}

/// R1-2: an attestation the store holds with no log record of its own is judged too, and a
/// strict policy excludes it rather than failing open.
#[test]
fn a_held_attestation_without_a_record_fails_closed() {
    let u = units();
    let (from, to) = (uid_of(&u[0]), uid_of(&u[1]));
    let mut rel = Relation::new(RelKind::Elaborates, from, to);
    let held = att(rel.uid(), "model:fake/edge", &[]);
    rel.attestations = BTreeSet::from([held.clone()]);
    let mut records = u;
    records.push(Record::Relation(rel));
    let store = Store::from_records(records);
    let h = Store::record_hash(&Record::Attestation(held.clone()));
    assert!(
        !store.contains_record(&h),
        "the precondition: the edge's attestation has no record in the log"
    );
    assert!(
        store.relations().any(|r| r.attestations.contains(&held)),
        "the precondition: the edge holds it"
    );

    let keys = MemoryKeyResolver::new();
    let strict = admission(&store, &[], &keys, &VerificationMode::Strict);
    assert_eq!(strict.judged.get(&h), Some(&false), "judged, and excluded");
    assert!(!strict.admits(&held));
    let advisory = admission(&store, &[], &keys, &VerificationMode::Advisory);
    assert!(advisory.admits(&held));

    // And `verify` now sees it: Strict fails a store whose edge carries an unsigned attestation.
    assert!(!verify_store(&store, &[], &keys, &VerificationMode::Strict).passed);
}
