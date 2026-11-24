//! RFC SMYSL-7 SG-P1: signing, the envelope, and verification in Advisory and Strict.

use smysl_core::{AgentId, Attestation, Hlc, Op, Record, Rung, Uid};
use smysl_graph::Store;
use smysl_sign::{
    read_signatures, sign_record, verify_record, verify_store, DetachedSignature, KeyStatus,
    MemoryKeyResolver, RecordVerdict, ResolvedKey, SignError, SigningKey, TargetKind,
    VerificationMode,
};

fn agent(s: &str) -> AgentId {
    s.parse().unwrap()
}

fn key(name: &str, byte: u8) -> SigningKey {
    SigningKey::from_seed(agent(name), [byte; 32])
}

const DOC: &str = "@doc smysl/0.1 { id: v/s }\n\n\
    @evidence e/a { status: measured, source: { kind: metric, ref: \"p95\" } }\n~ Latency rose.\n\n\
    @claim c/b { status: speculative }\n~ The pool is small.\n";

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

fn attest(uid: Uid, who: &str, ms: u64) -> Record {
    let a = agent(who);
    let ts = Hlc {
        wall_ms: ms,
        counter: 0,
        agent: a.clone(),
    };
    Record::Attestation(Attestation::new(uid, a, Op::Authored, Rung::Document, ts))
}

fn resolver(keys: &[&SigningKey]) -> MemoryKeyResolver {
    keys.iter().fold(MemoryKeyResolver::new(), |r, k| {
        r.with(ResolvedKey::active(k.agent().clone(), k.public_key()))
    })
}

#[test]
fn signing_is_deterministic_and_the_envelope_round_trips() {
    let alice = key("human:alice", 1);
    let u = units();
    let r = attest(uid_of(&u[0]), "human:alice", 1_000);
    let a = sign_record(&alice, &r).unwrap();
    let b = sign_record(&key("human:alice", 1), &r).unwrap();
    assert_eq!(
        a.to_cose(),
        b.to_cose(),
        "one key, one record: one signature"
    );
    assert_ne!(
        a.signature,
        sign_record(&SigningKey::from_seed(agent("human:alice"), [2; 32]), &r)
            .unwrap()
            .signature
    );
    assert_eq!(a.target_kind, TargetKind::Attestation);
    assert_eq!(a.subject_uid, Some(uid_of(&u[0])));
    assert_eq!(DetachedSignature::from_cose(&a.to_cose()).unwrap(), a);

    let unit_sig = sign_record(&alice, &u[1]).unwrap();
    assert_eq!(unit_sig.target_kind, TargetKind::Record);
    assert_eq!(unit_sig.subject_uid, None);
    let mut seq = a.to_cose();
    seq.extend(unit_sig.to_cose());
    assert_eq!(read_signatures(&seq).unwrap(), vec![a, unit_sig]);
}

#[test]
fn the_reader_accepts_only_what_the_writer_writes() {
    let alice = key("human:alice", 1);
    let sig = sign_record(&alice, &attest(Uid::from_bytes([3; 32]), "human:alice", 5)).unwrap();
    let good = sig.to_cose();
    let mut trailing = good.clone();
    trailing.push(0);
    assert!(DetachedSignature::from_cose(&trailing).is_err());
    let mut untagged = good.clone();
    untagged[0] = 0xd1; // tag 17, COSE_Mac0
    assert!(DetachedSignature::from_cose(&untagged).is_err());
    // An embedded payload (an empty byte string where null was): not detached.
    let null_at = good.len() - 67;
    assert_eq!(good[null_at], 0xf6);
    let mut embedded = good.clone();
    embedded[null_at] = 0x40;
    assert!(DetachedSignature::from_cose(&embedded).is_err());
    // A truncated file is an error, not a shorter list.
    assert!(read_signatures(&good[..good.len() - 1]).is_err());
}

#[test]
fn a_key_signs_only_its_own_agents_attestations() {
    let alice = key("human:alice", 1);
    let r = attest(Uid::from_bytes([3; 32]), "human:bob", 5);
    assert!(matches!(
        sign_record(&alice, &r),
        Err(SignError::AgentMismatch { .. })
    ));
    let dbg = format!("{alice:?}");
    assert!(dbg.contains("human:alice") && dbg.contains("kid"), "{dbg}");
    assert!(!dbg.contains(&format!("{:?}", &alice.seed()[..])), "{dbg}");
}

#[test]
fn verify_record_judges_each_way_a_signature_can_be_wrong() {
    let alice = key("human:alice", 1);
    let bob = key("human:bob", 2);
    let keys = resolver(&[&alice, &bob]);
    let r = attest(Uid::from_bytes([3; 32]), "human:alice", 5_000);
    let sig = sign_record(&alice, &r).unwrap();
    assert!(matches!(
        verify_record(&sig, &r, &keys),
        RecordVerdict::Verified { .. }
    ));

    // Another record: the hash no longer matches.
    let other = attest(Uid::from_bytes([4; 32]), "human:alice", 5_000);
    assert!(matches!(
        verify_record(&sig, &other, &keys),
        RecordVerdict::InvalidSignature { .. }
    ));
    // Each bit of the signature.
    for i in 0..64 {
        let mut s = sig.clone();
        s.signature[i] ^= 1;
        assert!(
            matches!(
                verify_record(&s, &r, &keys),
                RecordVerdict::InvalidSignature { .. }
            ),
            "byte {i}"
        );
    }
    // A header that claims bob: the attestation is alice's.
    let mut claim = sig.clone();
    claim.agent = agent("human:bob");
    assert!(matches!(
        verify_record(&claim, &r, &keys),
        RecordVerdict::AgentMismatch { .. }
    ));
    // Bob signs a record and names alice's key id.
    let unit = &units()[0];
    let mut by_bob = sign_record(&bob, unit).unwrap();
    by_bob.kid = alice.kid();
    assert!(matches!(
        verify_record(&by_bob, unit, &keys),
        RecordVerdict::AgentMismatch { .. }
    ));
    // A key the resolver does not know.
    assert!(matches!(
        verify_record(&sig, &r, &resolver(&[&bob])),
        RecordVerdict::UnknownKey { .. }
    ));
}

#[test]
fn revocation_and_bounds_read_the_attestations_clock() {
    let alice = key("human:alice", 1);
    let with = |status| {
        MemoryKeyResolver::new().with(
            ResolvedKey::active(alice.agent().clone(), alice.public_key()).with_status(status),
        )
    };
    let early = attest(Uid::from_bytes([3; 32]), "human:alice", 1_000);
    let late = attest(Uid::from_bytes([3; 32]), "human:alice", 9_000);
    let (se, sl) = (
        sign_record(&alice, &early).unwrap(),
        sign_record(&alice, &late).unwrap(),
    );
    let revoked_at_5 = with(KeyStatus::Revoked {
        after_ms: Some(5_000),
        reason: Some("rotated".into()),
    });
    assert!(matches!(
        verify_record(&se, &early, &revoked_at_5),
        RecordVerdict::Verified { .. }
    ));
    assert!(matches!(
        verify_record(&sl, &late, &revoked_at_5),
        RecordVerdict::RevokedKey { .. }
    ));
    let revoked = with(KeyStatus::Revoked {
        after_ms: None,
        reason: None,
    });
    assert!(matches!(
        verify_record(&se, &early, &revoked),
        RecordVerdict::RevokedKey { .. }
    ));
    let bounded = with(KeyStatus::Bounded {
        not_before_ms: 0,
        not_after_ms: 5_000,
    });
    assert!(matches!(
        verify_record(&se, &early, &bounded),
        RecordVerdict::Verified { .. }
    ));
    assert!(matches!(
        verify_record(&sl, &late, &bounded),
        RecordVerdict::RevokedKey { .. }
    ));
}

#[test]
fn a_store_passes_advisory_and_fails_strict_while_an_attestation_is_unsigned() {
    let alice = key("human:alice", 1);
    let bob = key("human:bob", 2);
    let u = units();
    let a1 = attest(uid_of(&u[0]), "human:alice", 1_000);
    let a2 = attest(uid_of(&u[1]), "human:bob", 2_000);
    let store = Store::from_records(vec![u[0].clone(), u[1].clone(), a1.clone(), a2.clone()]);
    let keys = resolver(&[&alice, &bob]);
    let one = vec![sign_record(&alice, &a1).unwrap()];

    let adv = verify_store(&store, &one, &keys, &VerificationMode::Advisory);
    assert!(adv.passed, "{adv:?}");
    assert_eq!(
        (adv.verified_count, adv.unsigned_count, adv.error_count),
        (1, 1, 0)
    );
    let strict = verify_store(&store, &one, &keys, &VerificationMode::Strict);
    assert!(!strict.passed);
    assert_eq!(strict.error_count, 1);

    let mut both = one.clone();
    both.push(sign_record(&bob, &a2).unwrap());
    // A signature for a record the store does not hold is counted, not judged.
    both.push(sign_record(&alice, &attest(Uid::from_bytes([9; 32]), "human:alice", 1)).unwrap());
    let strict = verify_store(&store, &both, &keys, &VerificationMode::Strict);
    assert!(strict.passed, "{strict:?}");
    assert_eq!((strict.verified_count, strict.unmatched_count), (2, 1));
    // Units are not required to be signed; a signed one is judged.
    assert_eq!(strict.verdicts.len(), 2);
    both.push(sign_record(&alice, &u[0]).unwrap());
    assert_eq!(
        verify_store(&store, &both, &keys, &VerificationMode::Strict)
            .verdicts
            .len(),
        3
    );
}

#[test]
fn a_forged_signature_fails_advisory_too_and_wins_over_a_good_one() {
    let alice = key("human:alice", 1);
    let u = units();
    let a1 = attest(uid_of(&u[0]), "human:alice", 1_000);
    let store = Store::from_records(vec![u[0].clone(), a1.clone()]);
    let keys = resolver(&[&alice]);
    let good = sign_record(&alice, &a1).unwrap();
    let mut forged = good.clone();
    forged.signature[0] ^= 0x80;
    let r = verify_store(
        &store,
        &[good.clone(), forged],
        &keys,
        &VerificationMode::Advisory,
    );
    assert!(!r.passed);
    assert!(matches!(
        r.verdicts.values().next().unwrap(),
        RecordVerdict::InvalidSignature { .. }
    ));
    // An unknown co-signer beside a verified signature leaves it verified.
    let carol = key("human:carol", 3);
    let r = verify_store(
        &store,
        &[good, sign_record(&carol, &u[0]).unwrap()],
        &keys,
        &VerificationMode::Strict,
    );
    let a1_hash = Store::record_hash(&a1);
    assert!(matches!(
        r.verdicts[&a1_hash],
        RecordVerdict::Verified { .. }
    ));
}
