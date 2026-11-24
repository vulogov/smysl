//! The envelope (RFC SMYSL-7 §4): a detached `COSE_Sign1` (RFC 9052 §4.2), written with
//! `smysl-core`'s deterministic CBOR encoder and read back strictly.

use std::fmt;

use smysl_core::cbor::{Dec, Enc};
use smysl_core::{AgentId, Uid};

use crate::key::KeyId;

/// CBOR tag 18, `COSE_Sign1`.
pub const COSE_SIGN1_TAG: u64 = 18;
/// `alg`: EdDSA.
pub const ALG_EDDSA: i64 = -8;
/// The external AAD every signature binds: a domain separator.
pub const EXTERNAL_AAD: &[u8] = b"smysl.sign/1";

const H_ALG: i64 = 1;
const H_KID: i64 = 4;
const H_AGENT: i64 = -65537;
const H_TARGET_KIND: i64 = -65538;
const H_TARGET_HASH: i64 = -65539;
const H_SUBJECT_UID: i64 = -65540;

const MAJOR_MAP: u8 = 5;
const MAJOR_TAG: u8 = 6;
const MAJOR_SIMPLE: u8 = 7;
const SIMPLE_NULL: u64 = 22;

/// What a signature covers (§4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum TargetKind {
    /// A `Record::Attestation`; the signing agent must be the attestation's.
    Attestation,
    /// Any other record, signed as it is.
    Record,
}

impl TargetKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            TargetKind::Attestation => "attestation",
            TargetKind::Record => "record",
        }
    }

    pub fn parse(s: &str) -> Option<TargetKind> {
        match s {
            "attestation" => Some(TargetKind::Attestation),
            "record" => Some(TargetKind::Record),
            _ => None,
        }
    }
}

impl fmt::Display for TargetKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

/// One detached signature. Its envelope is built from these fields on demand, so the two cannot
/// disagree (R1-12).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct DetachedSignature {
    pub kid: KeyId,
    pub agent: AgentId,
    pub target_kind: TargetKind,
    /// `Store::record_hash` of the signed record: BLAKE3-256 of its canonical CBOR.
    pub target_hash: [u8; 32],
    /// The uid an attestation names; an index hint, outside the signature.
    pub subject_uid: Option<Uid>,
    pub signature: [u8; 64],
}

/// Why a signature could not be made or read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SignError {
    /// The key signs as one agent, and the attestation is another's.
    AgentMismatch {
        key_agent: AgentId,
        record_agent: AgentId,
    },
    /// Bytes that are not a `COSE_Sign1` this crate writes: what is wrong, and at which byte.
    Malformed { at: usize, reason: String },
}

impl fmt::Display for SignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignError::AgentMismatch {
                key_agent,
                record_agent,
            } => write!(
                f,
                "the key signs as {key_agent}, and the attestation is {record_agent}'s"
            ),
            SignError::Malformed { at, reason } => {
                write!(f, "not a smysl COSE_Sign1 at byte {at}: {reason}")
            }
        }
    }
}

impl std::error::Error for SignError {}

impl DetachedSignature {
    /// The protected header's bytes: the map the signature covers.
    pub fn protected_bytes(&self) -> Vec<u8> {
        protected(&self.kid, &self.agent, self.target_kind, &self.target_hash)
    }

    /// The `COSE_Sign1` envelope.
    pub fn to_cose(&self) -> Vec<u8> {
        let mut e = Enc::new();
        e.head(MAJOR_TAG, COSE_SIGN1_TAG);
        e.array_head(4);
        e.bytes(&self.protected_bytes());
        match &self.subject_uid {
            Some(uid) => {
                e.head(MAJOR_MAP, 1);
                e.int(H_SUBJECT_UID);
                e.bytes(uid.as_bytes());
            }
            None => e.head(MAJOR_MAP, 0),
        }
        e.head(MAJOR_SIMPLE, SIMPLE_NULL);
        e.bytes(&self.signature);
        e.into_bytes()
    }

    /// Read one envelope, which must fill `bytes` exactly.
    pub fn from_cose(bytes: &[u8]) -> Result<DetachedSignature, SignError> {
        let (sig, n) = read_one(bytes)?;
        if n != bytes.len() {
            return Err(malformed(n, "bytes after the envelope"));
        }
        Ok(sig)
    }
}

/// Read a signature file: envelopes one after another, as `<store>.sig` holds them (§4.1).
pub fn read_signatures(seq: &[u8]) -> Result<Vec<DetachedSignature>, SignError> {
    let mut out = Vec::new();
    let mut off = 0;
    while off < seq.len() {
        let (sig, n) = read_one(&seq[off..]).map_err(|e| match e {
            SignError::Malformed { at, reason } => SignError::Malformed {
                at: at + off,
                reason,
            },
            other => other,
        })?;
        out.push(sig);
        off += n;
    }
    Ok(out)
}

/// The bytes Ed25519 signs: RFC 9052 §4.4's `Sig_structure` for a detached payload.
pub fn sig_structure(protected: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut e = Enc::with_capacity(protected.len() + payload.len() + 32);
    e.array_head(4);
    e.text("Signature1");
    e.bytes(protected);
    e.bytes(EXTERNAL_AAD);
    e.bytes(payload);
    e.into_bytes()
}

pub(crate) fn protected(
    kid: &KeyId,
    agent: &AgentId,
    kind: TargetKind,
    target_hash: &[u8; 32],
) -> Vec<u8> {
    // Deterministic key order (RFC 8949 §4.2.1) is the bytewise order of the encoded keys:
    // 0x01, 0x04, then the three 0x3a-headed labels, ascending by their arguments.
    let mut e = Enc::new();
    e.head(MAJOR_MAP, 5);
    e.int(H_ALG);
    e.int(ALG_EDDSA);
    e.int(H_KID);
    e.bytes(&kid.0);
    e.int(H_AGENT);
    e.text(agent.as_str());
    e.int(H_TARGET_KIND);
    e.text(kind.as_str());
    e.int(H_TARGET_HASH);
    e.bytes(target_hash);
    e.into_bytes()
}

fn malformed(at: usize, reason: impl Into<String>) -> SignError {
    SignError::Malformed {
        at,
        reason: reason.into(),
    }
}

fn codec(e: smysl_core::error::CodecError, at: usize) -> SignError {
    malformed(at, e.to_string())
}

/// One envelope from the front of `bytes`, and how many bytes it took. Strict: every field
/// must be the one this crate writes, and the bytes must be exactly the ones it would write.
fn read_one(bytes: &[u8]) -> Result<(DetachedSignature, usize), SignError> {
    let mut d = Dec::new(bytes);
    let at = |d: &Dec| d.position();
    let (m, tag) = d.head().map_err(|e| codec(e, 0))?;
    if (m, tag) != (MAJOR_TAG, COSE_SIGN1_TAG) {
        return Err(malformed(0, "not tag 18 (COSE_Sign1)"));
    }
    if d.array_head().map_err(|e| codec(e, at(&d)))? != 4 {
        return Err(malformed(at(&d), "a COSE_Sign1 is an array of four"));
    }
    let p_at = at(&d);
    let protected_bytes = d.bytes().map_err(|e| codec(e, p_at))?;
    let mut p = Dec::new(protected_bytes);
    let perr = |e: smysl_core::error::CodecError| codec(e, p_at);
    if p.map_head().map_err(perr)? != 5 {
        return Err(malformed(p_at, "the protected header has five entries"));
    }
    let expect = |p: &mut Dec, key: i64| -> Result<(), SignError> {
        if p.int().map_err(perr)? != key {
            return Err(malformed(
                p_at,
                format!("protected header: expected label {key}"),
            ));
        }
        Ok(())
    };
    expect(&mut p, H_ALG)?;
    if p.int().map_err(perr)? != ALG_EDDSA {
        return Err(malformed(p_at, "alg is not EdDSA (-8)"));
    }
    expect(&mut p, H_KID)?;
    let kid: [u8; 16] = p
        .bytes()
        .map_err(perr)?
        .try_into()
        .map_err(|_| malformed(p_at, "kid is not 16 bytes"))?;
    expect(&mut p, H_AGENT)?;
    let agent: AgentId = p
        .text()
        .map_err(perr)?
        .parse()
        .map_err(|e| malformed(p_at, format!("smysl.agent: {e}")))?;
    expect(&mut p, H_TARGET_KIND)?;
    let kind_s = p.text().map_err(perr)?;
    let target_kind = TargetKind::parse(kind_s)
        .ok_or_else(|| malformed(p_at, format!("unknown smysl.target_kind {kind_s:?}")))?;
    expect(&mut p, H_TARGET_HASH)?;
    let target_hash: [u8; 32] = p
        .bytes()
        .map_err(perr)?
        .try_into()
        .map_err(|_| malformed(p_at, "smysl.target_hash is not 32 bytes"))?;
    if !p.is_done() {
        return Err(malformed(p_at, "bytes after the protected header"));
    }

    let u_at = at(&d);
    let subject_uid = match d.map_head().map_err(|e| codec(e, u_at))? {
        0 => None,
        1 => {
            if d.int().map_err(|e| codec(e, u_at))? != H_SUBJECT_UID {
                return Err(malformed(u_at, "unprotected header: unknown label"));
            }
            let b: [u8; 32] = d
                .bytes()
                .map_err(|e| codec(e, u_at))?
                .try_into()
                .map_err(|_| malformed(u_at, "smysl.subject_uid is not 32 bytes"))?;
            Some(Uid::from_bytes(b))
        }
        _ => return Err(malformed(u_at, "unprotected header: one entry at most")),
    };
    let n_at = at(&d);
    if d.head().map_err(|e| codec(e, n_at))? != (MAJOR_SIMPLE, SIMPLE_NULL) {
        return Err(malformed(n_at, "the payload is not null: not detached"));
    }
    let s_at = at(&d);
    let signature: [u8; 64] = d
        .bytes()
        .map_err(|e| codec(e, s_at))?
        .try_into()
        .map_err(|_| malformed(s_at, "the signature is not 64 bytes"))?;
    let n = at(&d);
    let sig = DetachedSignature {
        kid: KeyId(kid),
        agent,
        target_kind,
        target_hash,
        subject_uid,
        signature,
    };
    // The reader accepts exactly what the writer writes, so a file has one encoding.
    if sig.to_cose() != bytes[..n] {
        return Err(malformed(0, "not in deterministic encoding"));
    }
    Ok((sig, n))
}
