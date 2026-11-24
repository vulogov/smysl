//! Signing and verifying (RFC SMYSL-7 §4.4, §6).

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::{AgentId, Record, Status, Uid};
use smysl_graph::{AttestationAdmission, Store};

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
    /// The signature verifies, and vouches for a unit above the key's cap (R1-9).
    StatusOverCap {
        agent: AgentId,
        kid: KeyId,
        status: Status,
        max_status: Status,
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
            RecordVerdict::StatusOverCap { .. }
                | RecordVerdict::RevokedKey { .. }
                | RecordVerdict::AgentMismatch { .. }
                | RecordVerdict::InvalidSignature { .. }
        )
    }

    pub fn is_verified(&self) -> bool {
        matches!(self, RecordVerdict::Verified { .. })
    }

    /// How a record with several signatures is judged (R1-11): the worst error, else
    /// verified, else an unknown key.
    fn rank(&self) -> u8 {
        match self {
            RecordVerdict::Unsigned => 0,
            RecordVerdict::UnknownKey { .. } => 1,
            RecordVerdict::Verified { .. } => 2,
            RecordVerdict::StatusOverCap { .. } => 3,
            RecordVerdict::RevokedKey { .. } => 4,
            RecordVerdict::AgentMismatch { .. } => 5,
            RecordVerdict::InvalidSignature { .. } => 6,
        }
    }
}

/// Verify one signature over one record.
///
/// A unit's own status is read against the key's cap. An attestation's cap is read against
/// the unit it names, which only a store holds: [`verify_store`] and [`verified_attesters`]
/// apply it, and this function, given the attestation alone, does not.
pub fn verify_record<R: KeyResolver>(
    sig: &DetachedSignature,
    record: &Record,
    resolver: &R,
) -> RecordVerdict {
    let status = record.as_unit().map(|u| u.status);
    verify_with(sig, record, resolver, status)
}

/// [`verify_record`], with the status of the unit the record vouches for.
fn verify_with<R: KeyResolver>(
    sig: &DetachedSignature,
    record: &Record,
    resolver: &R,
    subject_status: Option<Status>,
) -> RecordVerdict {
    let invalid = || RecordVerdict::InvalidSignature {
        agent: sig.agent.clone(),
        kid: sig.kid,
    };
    let payload = smysl_core::to_cbor(record);
    if smysl_core::hash_bytes(&payload) != sig.target_hash {
        return invalid();
    }
    // The subject uid sits in the unprotected header, so it is checked against the record
    // here: a header that names another unit is not this record's.
    let attestation = match (record, sig.target_kind) {
        (Record::Attestation(a), TargetKind::Attestation) if sig.subject_uid == Some(a.uid) => {
            Some(a)
        }
        (Record::Attestation(_), _) | (_, TargetKind::Attestation) => return invalid(),
        _ if sig.subject_uid.is_some() => return invalid(),
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
    if let (Some(max_status), Some(status)) = (key.max_status, subject_status) {
        if status > max_status {
            return RecordVerdict::StatusOverCap {
                agent: sig.agent.clone(),
                kid: sig.kid,
                status,
                max_status,
            };
        }
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
    /// `Strict` for the attestations in scope, `Advisory` for the rest. In scope: an
    /// attestation by an agent whose id starts with one of `agent_prefixes`, or one on a unit
    /// whose status is at or above `require_status_at_least` (R1-13).
    Selective {
        agent_prefixes: Vec<String>,
        require_status_at_least: Option<Status>,
    },
    /// Every attestation must carry a signature that verifies.
    Strict,
}

impl VerificationMode {
    /// `Selective` over these agent prefixes and this status.
    pub fn selective<S: Into<String>>(
        agent_prefixes: impl IntoIterator<Item = S>,
        require_status_at_least: Option<Status>,
    ) -> VerificationMode {
        VerificationMode::Selective {
            agent_prefixes: agent_prefixes.into_iter().map(Into::into).collect(),
            require_status_at_least,
        }
    }

    /// Whether a record by `agent`, vouching for a unit of `subject_status`, must be signed.
    pub fn in_scope(&self, agent: &AgentId, subject_status: Option<Status>) -> bool {
        match self {
            VerificationMode::Advisory => false,
            VerificationMode::Strict => true,
            VerificationMode::Selective {
                agent_prefixes,
                require_status_at_least,
            } => {
                agent_prefixes
                    .iter()
                    .any(|p| agent.as_str().starts_with(p.as_str()))
                    || matches!((require_status_at_least, subject_status),
                        (Some(min), Some(s)) if s >= *min)
            }
        }
    }

    /// Whether `v` fails a report under this mode: a forged signature always, and anything
    /// short of verified when the record is in scope.
    pub fn fails(&self, v: &RecordVerdict, in_scope: bool) -> bool {
        v.is_forged() || (in_scope && !v.is_verified())
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
    /// The records whose verdicts fail under `mode`; every other verdict short of verified
    /// is a warning.
    pub failing: BTreeSet<[u8; 32]>,
    pub verified_count: usize,
    pub unsigned_count: usize,
    pub warning_count: usize,
    pub error_count: usize,
    /// Signatures whose record the store does not hold.
    pub unmatched_count: usize,
}

/// One record, judged.
struct Judged<'a> {
    hash: [u8; 32],
    record: &'a Record,
    /// The agent a mode's scope reads: the attestation's, or the first signer's.
    agent: Option<AgentId>,
    subject_status: Option<Status>,
    /// Each signature's verdict, in the order given.
    each: Vec<RecordVerdict>,
    verdict: RecordVerdict,
}

/// The attestations a store's units and edges hold that have no record of their own in its log
/// (RFC SMYSL-8 R1-2): an edge held in memory can carry attestations its encoding does not.
/// Judged beside the log's, so nothing salience or retraction reads goes unjudged.
fn held_attestations(store: &Store) -> Vec<Record> {
    let mut out = BTreeMap::new();
    let held = store
        .units()
        .flat_map(|(_, u)| u.attestations.iter())
        .chain(store.relations().flat_map(|r| r.attestations.iter()));
    for a in held {
        let r = Record::Attestation(a.clone());
        let h = Store::record_hash(&r);
        if !store.contains_record(&h) {
            out.entry(h).or_insert(r);
        }
    }
    out.into_values().collect()
}

/// Judge every attestation in `store` (its log's, and `held`), and every other record a
/// signature names; and count the signatures left over.
fn judge<'a, R: KeyResolver>(
    store: &'a Store,
    held: &'a [Record],
    signatures: &[DetachedSignature],
    resolver: &R,
) -> (Vec<Judged<'a>>, usize) {
    let mut by_target: BTreeMap<[u8; 32], Vec<&DetachedSignature>> = BTreeMap::new();
    for s in signatures {
        by_target.entry(s.target_hash).or_default().push(s);
    }
    let mut seen = BTreeSet::new();
    let mut matched = BTreeSet::new();
    let mut unsigned = Vec::new();
    let mut out = Vec::new();
    for r in store.iter().chain(held) {
        let h = Store::record_hash(r);
        let sigs = by_target.get(&h);
        if !matches!(r, Record::Attestation(_)) && sigs.is_none() || !seen.insert(h) {
            continue;
        }
        if sigs.is_some() {
            matched.insert(h);
        } else {
            unsigned.push(out.len());
        }
        let (agent, subject_status) = match r {
            Record::Attestation(a) => {
                (Some(a.agent.clone()), store.get(&a.uid).map(|u| u.status()))
            }
            _ => (
                sigs.and_then(|s| s.first()).map(|s| s.agent.clone()),
                r.as_unit().map(|u| u.status),
            ),
        };
        let each: Vec<_> = sigs
            .into_iter()
            .flatten()
            .map(|s| verify_with(s, r, resolver, subject_status))
            .collect();
        out.push(Judged {
            hash: h,
            record: r,
            agent,
            subject_status,
            verdict: worst(&each),
            each,
        });
    }
    // An unsigned attestation, and a signature left over that names its agent and its unit:
    // the record was changed after it was signed, or the signature was. Judged against each
    // other they fail, as they would have had the bytes still matched (R1-14).
    for i in unsigned {
        let j = &mut out[i];
        let Record::Attestation(a) = j.record else {
            continue;
        };
        let suspects: Vec<_> = by_target
            .iter()
            .filter(|(h, _)| !matched.contains(*h))
            .flat_map(|(h, v)| v.iter().map(move |s| (*h, *s)))
            .filter(|(_, s)| s.agent == a.agent && s.subject_uid == Some(a.uid))
            .collect();
        for (h, s) in suspects {
            matched.insert(h);
            j.each
                .push(verify_with(s, j.record, resolver, j.subject_status));
        }
        j.verdict = worst(&j.each);
    }
    let unmatched = by_target
        .iter()
        .filter(|(h, _)| !matched.contains(*h))
        .map(|(_, v)| v.len())
        .sum();
    (out, unmatched)
}

/// The worst of several verdicts (R1-11); `Unsigned` for none.
fn worst(each: &[RecordVerdict]) -> RecordVerdict {
    each.iter()
        .max_by_key(|v| v.rank())
        .cloned()
        .unwrap_or(RecordVerdict::Unsigned)
}

/// Verify a store's attestations, and any other record signed, against `signatures`.
pub fn verify_store<R: KeyResolver>(
    store: &Store,
    signatures: &[DetachedSignature],
    resolver: &R,
    mode: &VerificationMode,
) -> VerificationReport {
    let held = held_attestations(store);
    let (judged, unmatched_count) = judge(store, &held, signatures, resolver);
    let mut verdicts = BTreeMap::new();
    let mut failing = BTreeSet::new();
    for j in judged {
        let in_scope = j
            .agent
            .as_ref()
            .is_some_and(|a| mode.in_scope(a, j.subject_status));
        if mode.fails(&j.verdict, in_scope) {
            failing.insert(j.hash);
        }
        verdicts.insert(j.hash, j.verdict);
    }
    let count = |f: fn(&RecordVerdict) -> bool| verdicts.values().filter(|v| f(v)).count();
    let verified_count = count(RecordVerdict::is_verified);
    VerificationReport {
        mode: mode.clone(),
        passed: failing.is_empty(),
        verified_count,
        unsigned_count: count(|v| matches!(v, RecordVerdict::Unsigned)),
        warning_count: verdicts.len() - verified_count - failing.len(),
        error_count: failing.len(),
        unmatched_count,
        failing,
        verdicts,
    }
}

/// For each unit, the agents whose attestations on it carry a verified signature, and the
/// keys that signed them (§6, R1-8).
///
/// An attestation counts when its verdict, the worst of its signatures', is verified, so a
/// forged signature beside a good one withdraws it. Corroboration and origin authority read
/// this to filter by; `smysl-graph` does not call this crate.
pub fn verified_attesters<R: KeyResolver>(
    store: &Store,
    signatures: &[DetachedSignature],
    resolver: &R,
) -> BTreeMap<Uid, BTreeSet<(AgentId, KeyId)>> {
    let mut out: BTreeMap<Uid, BTreeSet<(AgentId, KeyId)>> = BTreeMap::new();
    let held = held_attestations(store);
    for j in judge(store, &held, signatures, resolver).0 {
        let Record::Attestation(a) = j.record else {
            continue;
        };
        if !j.verdict.is_verified() {
            continue;
        }
        let keys = out.entry(a.uid).or_default();
        for v in &j.each {
            if let RecordVerdict::Verified { agent, kid } = v {
                keys.insert((agent.clone(), *kid));
            }
        }
    }
    out
}

/// Which attestations a trust decision counts under `mode` (RFC SMYSL-8 §3).
///
/// An attestation is admitted when `mode` would not fail it: exactly what [`verify_store`]
/// reports as an error is what this excludes. Every attestation the store holds is judged, and
/// the answer for one it does not hold is `mode`'s own default: admitted under `Advisory`,
/// excluded under `Selective` and `Strict`, so a policy fails closed (R1-2).
pub fn admission<R: KeyResolver>(
    store: &Store,
    signatures: &[DetachedSignature],
    resolver: &R,
    mode: &VerificationMode,
) -> AttestationAdmission {
    let held = held_attestations(store);
    let mut judged = BTreeMap::new();
    for j in judge(store, &held, signatures, resolver).0 {
        if !matches!(j.record, Record::Attestation(_)) {
            continue;
        }
        let in_scope = j
            .agent
            .as_ref()
            .is_some_and(|a| mode.in_scope(a, j.subject_status));
        judged.insert(j.hash, !mode.fails(&j.verdict, in_scope));
    }
    AttestationAdmission::new(judged, matches!(mode, VerificationMode::Advisory))
}
