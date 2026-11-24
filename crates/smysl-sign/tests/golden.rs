//! RFC SMYSL-7 SG-P1's golden vector: `fixtures/sign/`.
//!
//! RFC 8032 §7.1's first test key signs one attestation. The envelope and the record are
//! committed, and checked here three ways: the crate makes the same bytes again; a
//! `Sig_structure` built here by hand, byte by byte from RFC 9052 §4.4, verifies under
//! `ed25519-dalek` directly; and the RFC's own signature of the empty message comes out of the
//! same key. The Go and Node conformance tests verify the same files with their standard
//! libraries. `SMYSL_REGEN_SIGN=1` rewrites the files; nothing else should change them.

use std::path::PathBuf;

use smysl_core::{AgentId, Attestation, Hlc, Op, Record, Rung, Uid};
use smysl_sign::{sign_record, DetachedSignature, SigningKey};

const SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
/// RFC 8032 §7.1 TEST 1: the signature of the empty message.
const EMPTY_SIG: &str = "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b";

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sign")
}

fn record() -> Record {
    let agent: AgentId = "human:alice".parse().unwrap();
    let uid = Uid::from_bytes(smysl_core::hash_bytes(b"smysl.sign/1 golden vector"));
    let ts = Hlc {
        wall_ms: 1_762_000_000_000,
        counter: 0,
        agent: agent.clone(),
    };
    Record::Attestation(Attestation::new(
        uid,
        agent,
        Op::Authored,
        Rung::Document,
        ts,
    ))
}

fn key() -> SigningKey {
    let seed: [u8; 32] = hex(SEED).try_into().unwrap();
    SigningKey::from_seed("human:alice".parse().unwrap(), seed)
}

/// A CBOR byte-string head, written here rather than by the crate's encoder.
fn bstr(out: &mut Vec<u8>, b: &[u8]) {
    match b.len() {
        n @ 0..=23 => out.push(0x40 | n as u8),
        n @ 24..=255 => out.extend([0x58, n as u8]),
        n => {
            out.push(0x59);
            out.extend((n as u16).to_be_bytes());
        }
    }
    out.extend_from_slice(b);
}

#[test]
fn the_golden_vector_is_what_the_crate_makes_and_verifies_by_hand() {
    let key = key();
    assert_eq!(
        key.public_key().to_vec(),
        hex(PUBLIC),
        "RFC 8032 TEST 1's public key"
    );
    let dalek = ed25519_dalek::SigningKey::from_bytes(&hex(SEED).try_into().unwrap());
    assert_eq!(
        ed25519_dalek::Signer::sign(&dalek, b"").to_bytes().to_vec(),
        hex(EMPTY_SIG),
        "RFC 8032 TEST 1's signature"
    );

    let record = record();
    let payload = smysl_core::to_cbor(&record);
    let cose = sign_record(&key, &record).unwrap().to_cose();
    let (rec_path, sig_path) = (dir().join("attestation.cbor"), dir().join("signature.cose"));
    if std::env::var_os("SMYSL_REGEN_SIGN").is_some() {
        std::fs::create_dir_all(dir()).unwrap();
        std::fs::write(&rec_path, &payload).unwrap();
        std::fs::write(&sig_path, &cose).unwrap();
    }
    assert_eq!(
        std::fs::read(&rec_path).unwrap(),
        payload,
        "attestation.cbor"
    );
    assert_eq!(std::fs::read(&sig_path).unwrap(), cose, "signature.cose");

    // Take the envelope apart by position: tag 18, array of 4, protected bstr.
    assert_eq!(&cose[..3], &[0xd2, 0x84, 0x58]);
    let plen = cose[3] as usize;
    let protected = &cose[4..4 + plen];
    let rest = &cose[4 + plen..];
    // Unprotected: { -65540: h'<uid>' }, then null, then the 64-byte signature.
    assert_eq!(
        &rest[..8],
        &[0xa1, 0x3a, 0x00, 0x01, 0x00, 0x03, 0x58, 0x20]
    );
    assert_eq!(rest[40], 0xf6);
    assert_eq!(&rest[41..43], &[0x58, 0x40]);
    let signature: [u8; 64] = rest[43..].try_into().unwrap();

    // The protected header, as RFC 9052 and §4.2 give it, written out here.
    let mut want = vec![0xa5, 0x01, 0x27, 0x04, 0x50];
    want.extend(&key.kid().0);
    want.extend([0x3a, 0x00, 0x01, 0x00, 0x00, 0x6b]);
    want.extend(b"human:alice");
    want.extend([0x3a, 0x00, 0x01, 0x00, 0x01, 0x6b]);
    want.extend(b"attestation");
    want.extend([0x3a, 0x00, 0x01, 0x00, 0x02, 0x58, 0x20]);
    want.extend(smysl_core::hash_bytes(&payload));
    assert_eq!(protected, &want[..]);

    // Sig_structure = ["Signature1", protected, external_aad, payload].
    let mut msg = vec![0x84, 0x6a];
    msg.extend(b"Signature1");
    bstr(&mut msg, protected);
    bstr(&mut msg, b"smysl.sign/1");
    bstr(&mut msg, &payload);
    let vk = ed25519_dalek::VerifyingKey::from_bytes(&hex(PUBLIC).try_into().unwrap()).unwrap();
    vk.verify_strict(&msg, &ed25519_dalek::Signature::from_bytes(&signature))
        .expect("the golden signature verifies over a hand-built Sig_structure");

    assert_eq!(
        DetachedSignature::from_cose(&cose).unwrap().signature,
        signature
    );
}
