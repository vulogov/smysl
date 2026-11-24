//! Signing and verifying (RFC SMYSL-7 §4.4, §6).

use std::collections::BTreeMap;

use smysl_core::{AgentId, Record};
use smysl_graph::Store;

use crate::cose::{protected, sig_structure, DetachedSignature, SignError, TargetKind};
use crate::key::{KeyError, KeyId, KeyResolver, KeyStatus, SigningKey};

/// Sign a record. An attestation is signed only by a key bound to its agent.
///
/// Deterministic: one key and one record give the same bytes every time (RFC 8032 §5.1.6).
pub fn sign_record(key: &SigningKey, record: &Record) -> Result<DetachedSignature, SignError> {
    let (target_kind, subject_uid) = match record {
        Record::Attestation(a) => {
            if &a.agent != key.agent() {
                return Err(SignError::AgentMismatch {
                    key_agent: key.agent().clone(),
                    record_agent: a.agent.clone(),
                });
            }
            (TargetKind::Attestation, Some(a.uid))
        }
        _ => (TargetKind::Record, None),
    };
    let payload = smysl_core::to_cbor(record);
    let target_hash = smysl_core::hash_bytes(&payload);
    let kid = key.kid();
    let p = protected(&kid, key.agent(), target_kind, &target_hash);
    let signature = key.sign(&sig_structure(&p, &payload));
    Ok(DetachedSignature {
        kid,
        agent: key.agent().clone(),
        target_kind,
        target_hash,
        subject_uid,
        signature,
    })
}

/// What one record's signatures come to.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RecordVerdict {
    Verified {
        agent: AgentId,
        kid: KeyId,
    },
    /// No signature names the record.
    Unsigned,
    /// The resolver has no key with this id for this agent.
    UnknownKey {
        agent: AgentId,
        kid: KeyId,
    },
    /// The key was revoked, or the attestation's clock falls outside its bounds.
    RevokedKey {
        agent: AgentId,
        kid: KeyId,
        reason: Option<String>,
    },
    /// The signature's agent is not the attestation's, or the key belongs to another agent.
    AgentMismatch {
        header_agent: AgentId,
        record_agent: AgentId,
    },
    /// The signature does not verify over this record, under this key.
    InvalidSignature {
        agent: AgentId,
        kid: KeyId,
    },
}

impl RecordVerdict {
    /// An error in every mode: a signature that is present and wrong (§6).
    pub fn is_forged(&self) -> bool {
        matches!(
            self,
            RecordVerdict::RevokedKey { .. }
                | RecordVerdict::AgentMismatch { .. }
                | RecordVerdict::InvalidSignature { .. }
        )
    }

    /// How a record with several signatures is judged (R1-11): the worst error, else
    /// verified, else an unknown key.
    fn rank(&self) -> u8 {
        match self {
            RecordVerdict::Unsigned => 0,
            RecordVerdict::UnknownKey { .. } => 1,
            RecordVerdict::Verified { .. } => 2,
            RecordVerdict::RevokedKey { .. } => 3,
            RecordVerdict::AgentMismatch { .. } => 4,
            RecordVerdict::InvalidSignature { .. } => 5,
        }
    }
}

/// Verify one signature over one record.
pub fn verify_record<R: KeyResolver>(
    sig: &DetachedSignature,
    record: &Record,
    resolver: &R,
) -> RecordVerdict {
    let invalid = || RecordVerdict::InvalidSignature {
        agent: sig.agent.clone(),
        kid: sig.kid,
    };
    let payload = smysl_core::to_cbor(record);
    if smysl_core::hash_bytes(&payload) != sig.target_hash {
        return invalid();
    }
    let attestation = match (record, sig.target_kind) {
        (Record::Attestation(a), TargetKind::Attestation) => Some(a),
        (Record::Attestation(_), _) | (_, TargetKind::Attestation) => return invalid(),
        _ => None,
    };
    if let Some(a) = attestation {
        if a.agent != sig.agent {
            return RecordVerdict::AgentMismatch {
                header_agent: sig.agent.clone(),
                record_agent: a.agent.clone(),
            };
        }
    }
    let key = match resolver.resolve(&sig.agent, &sig.kid) {
        Ok(k) => k,
        Err(KeyError::OtherAgent { bound_to, .. }) => {
            return RecordVerdict::AgentMismatch {
                header_agent: sig.agent.clone(),
                record_agent: bound_to,
            }
        }
        Err(_) => {
            return RecordVerdict::UnknownKey {
                agent: sig.agent.clone(),
                kid: sig.kid,
            }
        }
    };
    // A resolver that answers with another key, or another agent's, is not trusted (R1-10).
    if KeyId::of(&key.public_key) != sig.kid || key.kid != sig.kid {
        return invalid();
    }
    if key.agent != sig.agent {
        return RecordVerdict::AgentMismatch {
            header_agent: sig.agent.clone(),
            record_agent: key.agent,
        };
    }
    let Ok(vk) = ed25519_dalek::VerifyingKey::from_bytes(&key.public_key) else {
        return invalid();
    };
    let s = ed25519_dalek::Signature::from_bytes(&sig.signature);
    let msg = sig_structure(&sig.protected_bytes(), &payload);
    if vk.verify_strict(&msg, &s).is_err() {
        return invalid();
    }
    let revoked = |reason: Option<String>| RecordVerdict::RevokedKey {
        agent: sig.agent.clone(),
        kid: sig.kid,
        reason,
    };
    // Revocation and bounds read the attestation's clock; a record with none is judged
    // against "always" (§9).
    let when = attestation.map(|a| a.ts.wall_ms);
    match &key.status {
        KeyStatus::Active => {}
        KeyStatus::Revoked { after_ms, reason } => match (after_ms, when) {
            (Some(t), Some(w)) if w < *t => {}
            _ => return revoked(reason.clone()),
        },
        KeyStatus::Bounded {
            not_before_ms,
            not_after_ms,
        } => match when {
            Some(w) if (*not_before_ms..=*not_after_ms).contains(&w) => {}
            _ => return revoked(Some("outside the key's bounds".into())),
        },
    }
    RecordVerdict::Verified {
        agent: sig.agent.clone(),
        kid: sig.kid,
    }
}

/// How much a verifier demands (§6).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum VerificationMode {
    /// An unsigned attestation, or one signed with a key the resolver does not know, is a
    /// warning. A forged signature is an error.
    #[default]
    Advisory,
    /// Every attestation must carry a signature that verifies.
    Strict,
}

impl VerificationMode {
    /// Whether `v` fails a report under this mode.
    pub fn fails(&self, v: &RecordVerdict) -> bool {
        if v.is_forged() {
            return true;
        }
        match self {
            VerificationMode::Advisory => false,
            VerificationMode::Strict => !matches!(v, RecordVerdict::Verified { .. }),
        }
    }
}

/// A store's verification (§6).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct VerificationReport {
    pub mode: VerificationMode,
    /// No verdict fails under `mode`.
    pub passed: bool,
    /// One verdict per record, by `Store::record_hash`: every attestation, and every other
    /// record a signature names.
    pub verdicts: BTreeMap<[u8; 32], RecordVerdict>,
    pub verified_count: usize,
    pub unsigned_count: usize,
    pub error_count: usize,
    /// Signatures whose record the store does not hold.
    pub unmatched_count: usize,
}

/// Verify a store's attestations, and any other record signed, against `signatures`.
pub fn verify_store<R: KeyResolver>(
    store: &Store,
    signatures: &[DetachedSignature],
    resolver: &R,
    mode: &VerificationMode,
) -> VerificationReport {
    let mut by_target: BTreeMap<[u8; 32], Vec<&DetachedSignature>> = BTreeMap::new();
    for s in signatures {
        by_target.entry(s.target_hash).or_default().push(s);
    }
    let mut verdicts = BTreeMap::new();
    let mut matched = std::collections::BTreeSet::new();
    for r in store.iter() {
        let h = Store::record_hash(r);
        let sigs = by_target.get(&h);
        if !matches!(r, Record::Attestation(_)) && sigs.is_none() {
            continue;
        }
        if verdicts.contains_key(&h) {
            continue;
        }
        let v = sigs
            .into_iter()
            .flatten()
            .map(|s| verify_record(s, r, resolver))
            .max_by_key(RecordVerdict::rank)
            .unwrap_or(RecordVerdict::Unsigned);
        if sigs.is_some() {
            matched.insert(h);
        }
        verdicts.insert(h, v);
    }
    let unmatched_count = by_target
        .iter()
        .filter(|(h, _)| !matched.contains(*h))
        .map(|(_, v)| v.len())
        .sum();
    let count = |f: fn(&RecordVerdict) -> bool| verdicts.values().filter(|v| f(v)).count();
    let error_count = verdicts.values().filter(|v| mode.fails(v)).count();
    VerificationReport {
        mode: mode.clone(),
        passed: error_count == 0,
        verified_count: count(|v| matches!(v, RecordVerdict::Verified { .. })),
        unsigned_count: count(|v| matches!(v, RecordVerdict::Unsigned)),
        error_count,
        unmatched_count,
        verdicts,
    }
}
