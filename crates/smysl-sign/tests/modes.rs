//! RFC SMYSL-7 SG-P2: the modes, status caps, the key file, and `verified_attesters`.
//!
//! The phase's gate: flipping any byte of the record, the protected header or the signature
//! fails in every mode; an attestation signed under another agent's id is `AgentMismatch`; an
//! unsigned `measured` unit passes `Advisory` with a warning and fails `Selective` and `Strict`.

use smysl_core::{AgentId, Attestation, Hlc, Op, Record, Rung, Status, Uid};
use smysl_graph::Store;
use smysl_sign::{
    parse_public_key, sign_record, verified_attesters, verify_record, verify_store,
    DetachedSignature, KeyFile, KeyId, KeyResolver, KeyStatus, MemoryKeyResolver, RecordVerdict,
    ResolvedKey, SigningKey, VerificationMode,
};

fn agent(s: &str) -> AgentId {
    s.parse().unwrap()
}

fn key(name: &str, byte: u8) -> SigningKey {
    SigningKey::from_seed(agent(name), [byte; 32])
}

/// A measured unit and a speculative one.
const DOC: &str = "@doc smysl/0.1 { id: v/m }\n\n\
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

fn modes() -> [VerificationMode; 3] {
    [
        VerificationMode::Advisory,
        VerificationMode::selective(["tool:"], Some(Status::Measured)),
        VerificationMode::Strict,
    ]
}

/// The measured unit, alice's signed attestation on it, and the store holding both.
fn signed_store() -> (Vec<Record>, Record, DetachedSignature, MemoryKeyResolver) {
    let alice = key("human:alice", 1);
    let u = units();
    let a = attest(uid_of(&u[0]), "human:alice", 1_000);
    let sig = sign_record(&alice, &a).unwrap();
    (u, a, sig, resolver(&[&alice]))
}

#[test]
fn flipping_any_byte_of_the_record_fails_every_mode() {
    let (u, a, sig, keys) = signed_store();
    let bytes = smysl_core::to_cbor(&a);
    let (mut decoded, mut changed, mut in_store) = (0, 0, 0);
    for i in 0..bytes.len() {
        for mask in [0x01, 0x80] {
            let mut b = bytes.clone();
            b[i] ^= mask;
            // A record the reader refuses fails before any signature is read.
            let Ok((r, n)) = smysl_core::from_cbor(&b) else {
                continue;
            };
            decoded += 1;
            if n != b.len() || r == a {
                continue;
            }
            changed += 1;
            let v = verify_record(&sig, &r, &keys);
            assert!(v.is_forged(), "byte {i} ^ {mask:#x}: {v:?}");
            // In a store: the tampered attestation, if it still names alice and the unit,
            // meets the signature that names them (R1-14).
            let Record::Attestation(t) = &r else { continue };
            if t.agent != agent("human:alice") || t.uid != uid_of(&u[0]) {
                continue;
            }
            in_store += 1;
            let store = Store::from_records(vec![u[0].clone(), r.clone()]);
            for mode in modes() {
                let rep = verify_store(&store, std::slice::from_ref(&sig), &keys, &mode);
                assert!(!rep.passed, "byte {i} ^ {mask:#x} under {mode:?}: {rep:?}");
            }
        }
    }
    eprintln!("record flips: {decoded} decode, {changed} change it, {in_store} judged in a store");
    assert!(
        changed > 0 && in_store > 0,
        "some flips decode ({decoded}, {changed}, {in_store})"
    );
}

#[test]
fn flipping_any_byte_of_the_envelope_fails_every_mode() {
    let (u, a, sig, keys) = signed_store();
    let store = Store::from_records(vec![u[0].clone(), a.clone()]);
    for mode in modes() {
        assert!(verify_store(&store, std::slice::from_ref(&sig), &keys, &mode).passed);
    }
    let cose = sig.to_cose();
    // The kid's sixteen bytes, inside the protected header: `04 50 <kid>`.
    let kid_at = cose.windows(2).position(|w| w == [0x04, 0x50]).unwrap() + 2;
    let kid = kid_at..kid_at + 16;
    let mut read = 0;
    for i in 0..cose.len() {
        for mask in [0x01, 0x80] {
            let mut b = cose.clone();
            b[i] ^= mask;
            // An envelope the reader refuses fails before it is judged.
            let Ok(t) = DetachedSignature::from_cose(&b) else {
                continue;
            };
            read += 1;
            for mode in modes() {
                let rep = verify_store(&store, std::slice::from_ref(&t), &keys, &mode);
                if kid.contains(&i) {
                    // A kid no resolver knows is an unknown key, which `Advisory` warns of
                    // and the other two refuse (§6; R1-14).
                    let v = &rep.verdicts[&Store::record_hash(&a)];
                    assert!(
                        matches!(v, RecordVerdict::UnknownKey { .. }),
                        "kid byte {i}: {v:?}"
                    );
                    assert_eq!(
                        rep.passed,
                        mode == VerificationMode::Advisory,
                        "kid byte {i}"
                    );
                    assert_eq!(rep.warning_count, rep.passed as usize);
                } else {
                    assert!(!rep.passed, "byte {i} ^ {mask:#x} under {mode:?}: {rep:?}");
                }
            }
        }
    }
    eprintln!("envelope flips: {read} of {} read back", 2 * cose.len());
    assert!(read > 2 * 64, "the signature's flips, at least, read back");
}

#[test]
fn an_attestation_signed_under_another_agents_id_is_agent_mismatch() {
    let alice = key("human:alice", 1);
    let bob = key("human:bob", 2);
    let keys = resolver(&[&alice, &bob]);
    let u = units();
    let by_bob = attest(uid_of(&u[0]), "human:bob", 1_000);
    // Alice's secret, claiming bob's id: the key is alice's, the header and record say bob.
    let posing = SigningKey::from_seed(agent("human:bob"), *alice.seed());
    let sig = sign_record(&posing, &by_bob).unwrap();
    assert_eq!(sig.kid, alice.kid());
    let v = verify_record(&sig, &by_bob, &keys);
    assert!(
        matches!(
            &v,
            RecordVerdict::AgentMismatch { header_agent, record_agent }
                if *header_agent == agent("human:bob") && *record_agent == agent("human:alice")
        ),
        "{v:?}"
    );
    let store = Store::from_records(vec![u[0].clone(), by_bob]);
    for mode in modes() {
        assert!(!verify_store(&store, std::slice::from_ref(&sig), &keys, &mode).passed);
    }
}

#[test]
fn an_unsigned_measured_unit_warns_in_advisory_and_fails_selective_and_strict() {
    let u = units();
    let store = Store::from_records(vec![
        u[0].clone(),
        attest(uid_of(&u[0]), "human:alice", 1_000),
    ]);
    let keys = MemoryKeyResolver::new();
    let adv = verify_store(&store, &[], &keys, &VerificationMode::Advisory);
    assert!(adv.passed);
    assert_eq!(
        (adv.unsigned_count, adv.warning_count, adv.error_count),
        (1, 1, 0)
    );
    for mode in [
        VerificationMode::selective(Vec::<String>::new(), Some(Status::Measured)),
        VerificationMode::Strict,
    ] {
        let rep = verify_store(&store, &[], &keys, &mode);
        assert!(!rep.passed, "{mode:?}");
        assert_eq!((rep.warning_count, rep.error_count), (0, 1));
    }
}

#[test]
fn selective_scope_is_an_agent_prefix_or_a_status_floor() {
    let ci = key("tool:ci", 4);
    let keys = resolver(&[&ci]);
    let u = units();
    let (measured, speculative) = (uid_of(&u[0]), uid_of(&u[1]));
    let mode = VerificationMode::selective(["tool:"], Some(Status::Cited));
    let check = |records: Vec<Record>, sigs: &[DetachedSignature]| {
        let mut all = u.clone();
        all.extend(records);
        verify_store(&Store::from_records(all), sigs, &keys, &mode)
    };
    // Out of scope: a human's attestation on a speculative unit, unsigned. A warning.
    let rep = check(vec![attest(speculative, "human:alice", 1)], &[]);
    assert!(rep.passed && rep.warning_count == 1, "{rep:?}");
    // In scope by status.
    assert!(!check(vec![attest(measured, "human:alice", 1)], &[]).passed);
    // In scope by prefix: unsigned fails, signed passes.
    let by_ci = attest(speculative, "tool:ci", 1);
    assert!(!check(vec![by_ci.clone()], &[]).passed);
    let sig = sign_record(&ci, &by_ci).unwrap();
    assert!(check(vec![by_ci], &[sig]).passed);
    // A prefix is a prefix of the id, not of a word in it.
    assert!(!mode.in_scope(&agent("human:tool"), None));
    assert!(mode.in_scope(&agent("tool:ci"), None));
    assert!(mode.in_scope(&agent("human:a"), Some(Status::Measured)));
    assert!(!mode.in_scope(&agent("human:a"), Some(Status::Derived)));
}

#[test]
fn an_unknown_key_is_in_scope_like_an_absent_one() {
    let stranger = key("tool:ci", 9);
    let u = units();
    let by_ci = attest(uid_of(&u[1]), "tool:ci", 1);
    let sig = sign_record(&stranger, &by_ci).unwrap();
    let store = Store::from_records(vec![u[1].clone(), by_ci]);
    let keys = MemoryKeyResolver::new();
    let run = |m: VerificationMode| verify_store(&store, std::slice::from_ref(&sig), &keys, &m);
    assert!(run(VerificationMode::Advisory).passed);
    assert!(run(VerificationMode::selective(["human:"], None)).passed);
    assert!(!run(VerificationMode::selective(["tool:"], None)).passed);
    assert!(!run(VerificationMode::Strict).passed);
}

#[test]
fn a_key_vouches_only_up_to_its_cap() {
    let ci = key("tool:ci", 4);
    let keys = MemoryKeyResolver::new().with(
        ResolvedKey::active(ci.agent().clone(), ci.public_key()).with_max_status(Status::Derived),
    );
    let u = units();
    let on_measured = attest(uid_of(&u[0]), "tool:ci", 1);
    let on_speculative = attest(uid_of(&u[1]), "tool:ci", 1);
    let sigs = vec![
        sign_record(&ci, &on_measured).unwrap(),
        sign_record(&ci, &on_speculative).unwrap(),
    ];
    let store = Store::from_records(vec![
        u[0].clone(),
        u[1].clone(),
        on_measured.clone(),
        on_speculative.clone(),
    ]);
    for mode in modes() {
        let rep = verify_store(&store, &sigs, &keys, &mode);
        assert!(!rep.passed, "{mode:?}");
        assert!(matches!(
            rep.verdicts[&Store::record_hash(&on_measured)],
            RecordVerdict::StatusOverCap {
                status: Status::Measured,
                max_status: Status::Derived,
                ..
            }
        ));
        assert!(rep.verdicts[&Store::record_hash(&on_speculative)].is_verified());
        assert_eq!(rep.failing.len(), 1);
    }
    // Alone, an attestation has no status to read against the cap (R1-9).
    assert!(verify_record(&sigs[0], &on_measured, &keys).is_verified());
    // A unit signed directly is read against its own status.
    assert!(matches!(
        verify_record(&sign_record(&ci, &u[0]).unwrap(), &u[0], &keys),
        RecordVerdict::StatusOverCap { .. }
    ));
    // An attestation on a unit the store does not hold is never over a cap.
    let elsewhere = attest(Uid::from_bytes([7; 32]), "tool:ci", 1);
    let store = Store::from_records(vec![elsewhere.clone()]);
    let rep = verify_store(
        &store,
        &[sign_record(&ci, &elsewhere).unwrap()],
        &keys,
        &VerificationMode::Strict,
    );
    assert!(rep.passed, "{rep:?}");
}

#[test]
fn a_subject_uid_that_is_not_the_attestations_is_refused() {
    let (u, a, sig, keys) = signed_store();
    let mut other = sig.clone();
    other.subject_uid = Some(uid_of(&u[1]));
    assert!(matches!(
        verify_record(&other, &a, &keys),
        RecordVerdict::InvalidSignature { .. }
    ));
    other.subject_uid = None;
    assert!(matches!(
        verify_record(&other, &a, &keys),
        RecordVerdict::InvalidSignature { .. }
    ));
    let alice = key("human:alice", 1);
    let mut on_unit = sign_record(&alice, &u[1]).unwrap();
    assert!(verify_record(&on_unit, &u[1], &keys).is_verified());
    on_unit.subject_uid = Some(uid_of(&u[1]));
    assert!(matches!(
        verify_record(&on_unit, &u[1], &keys),
        RecordVerdict::InvalidSignature { .. }
    ));
}

#[test]
fn verified_attesters_names_the_agents_whose_signatures_hold() {
    let alice = key("human:alice", 1);
    let bob = key("human:bob", 2);
    let carol = key("human:carol", 3);
    let keys = resolver(&[&alice, &bob]);
    let u = units();
    let (m, s) = (uid_of(&u[0]), uid_of(&u[1]));
    let (am, bm, cm, bs) = (
        attest(m, "human:alice", 1),
        attest(m, "human:bob", 2),
        attest(m, "human:carol", 3),
        attest(s, "human:bob", 4),
    );
    let store = Store::from_records(vec![
        u[0].clone(),
        u[1].clone(),
        am.clone(),
        bm.clone(),
        cm.clone(),
        bs.clone(),
    ]);
    let mut forged = sign_record(&bob, &bs).unwrap();
    forged.signature[5] ^= 1;
    let sigs = vec![
        sign_record(&alice, &am).unwrap(),
        sign_record(&bob, &bm).unwrap(),
        // Carol's key is not in the resolver.
        sign_record(&carol, &cm).unwrap(),
        // A good signature and a forged one: the forged one withdraws it.
        sign_record(&bob, &bs).unwrap(),
        forged,
    ];
    let got = verified_attesters(&store, &sigs, &keys);
    let want: std::collections::BTreeSet<(AgentId, KeyId)> = [
        (agent("human:alice"), alice.kid()),
        (agent("human:bob"), bob.kid()),
    ]
    .into();
    assert_eq!(got.get(&m), Some(&want));
    assert_eq!(got.get(&s), None);
    assert_eq!(got.len(), 1);
}

#[test]
fn a_tampered_attestation_meets_the_signature_that_names_it() {
    let (u, a, sig, keys) = signed_store();
    let Record::Attestation(mut t) = a.clone() else {
        unreachable!()
    };
    t.op = Op::Attested;
    let tampered = Record::Attestation(t);
    let store = Store::from_records(vec![u[0].clone(), tampered.clone()]);
    let rep = verify_store(
        &store,
        std::slice::from_ref(&sig),
        &keys,
        &VerificationMode::Advisory,
    );
    assert!(!rep.passed);
    assert_eq!(rep.unmatched_count, 0, "the signature was judged");
    assert!(matches!(
        rep.verdicts[&Store::record_hash(&tampered)],
        RecordVerdict::InvalidSignature { .. }
    ));
    // Both the signed record and a second, unsigned one by the same agent: the signature
    // matches its own record, and the second is only unsigned.
    let store = Store::from_records(vec![
        u[0].clone(),
        a.clone(),
        attest(uid_of(&u[0]), "human:alice", 2_000),
    ]);
    let rep = verify_store(
        &store,
        std::slice::from_ref(&sig),
        &keys,
        &VerificationMode::Advisory,
    );
    assert!(rep.passed, "{rep:?}");
    assert_eq!((rep.verified_count, rep.unsigned_count), (1, 1));
}

// --- the key file ---------------------------------------------------------------------------

const GOLDEN_HEX: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
const GOLDEN_B64: &str = "11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=";
const GOLDEN_SPKI: &str = "MCowBQYDK2VwAyEA11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=";

#[test]
fn a_public_key_is_hex_raw_base64_or_spki_der() {
    let want = parse_public_key(&format!("hex:{GOLDEN_HEX}")).unwrap();
    assert_eq!(
        parse_public_key(&format!("hex:{}", GOLDEN_HEX.to_uppercase())).unwrap(),
        want
    );
    assert_eq!(
        parse_public_key(&format!("base64:{GOLDEN_B64}")).unwrap(),
        want
    );
    assert_eq!(
        parse_public_key(&format!("base64:{GOLDEN_SPKI}")).unwrap(),
        want
    );
    for bad in [
        GOLDEN_HEX.to_string(),
        format!("hex:{}", &GOLDEN_HEX[2..]),
        format!("base64:{}", &GOLDEN_B64[4..]),
        // An X25519 SPKI: the OID's last byte is 0x6e, not 0x70.
        "base64:MCowBQYDK2VuAyEA11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=".into(),
        // The identity point, of small order.
        format!("hex:01{}", "00".repeat(31)),
    ] {
        assert!(parse_public_key(&bad).is_err(), "{bad}");
    }
}

#[test]
fn the_key_file_reads_statuses_caps_and_comments() {
    let text = format!(
        "{{\n  # who may sign\n  keys: [\n\
         {{ agent: \"human:alice\", ed25519: \"hex:{GOLDEN_HEX}\", status: \"active\" }}\n\
         {{ agent: \"tool:ci\", ed25519: \"hex:{}\", max_status: \"measured\" }}\n\
         {{ agent: \"tool:old-bot\", ed25519: \"hex:{}\", status: \"revoked\",\n\
           revoked_after_ms: 1762000000000, reason: \"rotated in Q4\" }}\n\
         {{ agent: \"tool:temp\", ed25519: \"hex:{}\", status: \"bounded\",\n\
           not_before_ms: 10, not_after_ms: 20 }}\n  ]\n}}\n",
        hex_of(&key("tool:ci", 4)),
        hex_of(&key("tool:old-bot", 5)),
        hex_of(&key("tool:temp", 6)),
    );
    let keys = KeyFile::parse(&text).unwrap();
    assert_eq!(keys.len(), 4);
    let get = |k: &SigningKey| keys.resolve(k.agent(), &k.kid()).unwrap();
    assert_eq!(get(&key("tool:ci", 4)).max_status, Some(Status::Measured));
    assert_eq!(
        get(&key("tool:old-bot", 5)).status,
        KeyStatus::Revoked {
            after_ms: Some(1_762_000_000_000),
            reason: Some("rotated in Q4".into())
        }
    );
    assert_eq!(
        get(&key("tool:temp", 6)).status,
        KeyStatus::Bounded {
            not_before_ms: 10,
            not_after_ms: 20
        }
    );
    // Each entry written back reads back the same.
    let again: Vec<String> = keys.keys().map(KeyFile::entry).collect();
    let reread = KeyFile::parse(&format!("{{ keys: [\n{}\n] }}", again.join("\n"))).unwrap();
    assert_eq!(reread, keys);
}

fn hex_of(k: &SigningKey) -> String {
    k.public_key().iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn the_key_file_refuses_what_it_does_not_understand() {
    let a = format!("agent: \"human:alice\", ed25519: \"hex:{GOLDEN_HEX}\"");
    let b = format!("agent: \"human:bob\", ed25519: \"hex:{GOLDEN_HEX}\"");
    let cases = [
        (
            format!("{{ keys: [ {{ {a}, status: \"revoked\", revoked_afer_ms: 5 }} ] }}"),
            "unknown key `revoked_afer_ms`",
        ),
        (
            format!("{{ keys: [ {{ {a}, revoked_after_ms: 5 }} ] }}"),
            "needs status: \"revoked\"",
        ),
        (
            format!("{{ keys: [ {{ {a}, status: \"bounded\", not_before_ms: 5 }} ] }}"),
            "needs `not_after_ms`",
        ),
        (
            format!(
                "{{ keys: [ {{ {a}, status: \"bounded\", not_before_ms: 5, not_after_ms: 4 }} ] }}"
            ),
            "before",
        ),
        (
            format!("{{ keys: [ {{ {a}, status: \"gone\" }} ] }}"),
            "status `gone`",
        ),
        (
            format!("{{ keys: [ {{ {a}, max_status: \"certain\" }} ] }}"),
            "no status `certain`",
        ),
        (
            format!("{{ keys: [ {{ {a}, kid: \"00\" }} ] }}"),
            "the key's id is",
        ),
        (
            format!("{{ keys: [ {{ {a}, status: \"revoked\", revoked_after_ms: -1 }} ] }}"),
            "milliseconds",
        ),
        (
            format!("{{ keys: [ {{ {a} }}\n{{ {a} }} ] }}"),
            "listed twice",
        ),
        (
            format!("{{ keys: [ {{ {a} }}\n{{ {b} }} ] }}"),
            "bound to human:alice and to human:bob",
        ),
        (
            format!("{{ keys: [ {{ {a} }} ], extra: 1 }}"),
            "unknown key `extra`",
        ),
        (
            format!("{{ keys: [ {{ agent: \"nobody\", ed25519: \"hex:{GOLDEN_HEX}\" }} ] }}"),
            "`agent`",
        ),
        (
            "{ keys: [ { agent: \"human:a\" } ] }".to_string(),
            "no `ed25519`",
        ),
        ("{ }".to_string(), "no `keys`"),
        ("{ keys: [".to_string(), ""),
    ];
    for (text, want) in cases {
        let e = KeyFile::parse(&text).expect_err(&text);
        assert!(e.message.contains(want), "{text}\n  -> {e}");
    }
    let e = KeyFile::parse(&format!("{{ keys: [\n{{ {a} }}\n{{ {a} }} ] }}")).unwrap_err();
    assert_eq!(e.line, 3, "{e}");
}

#[test]
fn a_signing_key_file_reads_back_as_the_key() {
    let k = key("human:alice", 7);
    let text = KeyFile::secret(&k);
    assert!(text.starts_with('#'), "{}", &*text);
    let back = KeyFile::parse_secret(&text).unwrap();
    assert_eq!((back.agent(), back.kid()), (k.agent(), k.kid()));
    assert_eq!(*back.seed(), *k.seed());
    let wrong_kid = text.replace(&k.kid().to_string(), &"0".repeat(32));
    assert!(KeyFile::parse_secret(&wrong_kid)
        .unwrap_err()
        .message
        .contains("the key's id is"));
    assert!(KeyFile::parse_secret("{ agent: \"human:a\", ed25519_secret: \"hex:00\" }").is_err());
    assert!(KeyFile::parse_secret("{ agent: \"human:a\", seed: \"x\" }").is_err());
    // A public key file is not a signing key file.
    let public = KeyFile::entry(&ResolvedKey::active(k.agent().clone(), k.public_key()));
    assert!(KeyFile::parse_secret(&public).is_err());
}
