//! Which attestations a trust decision counts (RFC SMYSL-8 §3).
//!
//! A signature policy is decided in `smysl-sign`, which sits above this crate; what reaches
//! salience and retraction is its answer, as data. An attestation is *admitted* when the
//! verification mode in force would not fail it. Admission only ever removes attestations from
//! counts — corroboration and origin authority — and never changes a status, a record or a view
//! (SMYSL-8 G4).
//!
//! The type is `AttestationAdmission`, not `Admission`: `smysl_core::Admission` is a view's
//! granularity, and the facade flattens both crates into one namespace.
//!
//! **Fail closed (R1-2).** An admission records the attestations it judged, by record hash, and
//! decides every other attestation by [`AttestationAdmission::unjudged`]: admitted under no
//! policy and under `Advisory`, excluded under `Selective` and `Strict`. A set of exclusions alone would
//! admit whatever the policy never saw — an edge attestation held in memory with no log record of
//! its own, say — and a strict policy would then fail open.

use std::collections::BTreeMap;

use smysl_core::{Attestation, Record};

use crate::store::Store;

/// The attestations a trust decision counts.
///
/// The [`Default`] admits everything, which is what every caller got before SMYSL-8.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AttestationAdmission {
    /// Attestations judged, by record hash ([`Store::record_hash`], the hash a signature
    /// names), and whether each is admitted.
    pub judged: BTreeMap<[u8; 32], bool>,
    /// The answer for an attestation not in `judged`.
    pub unjudged: bool,
}

impl Default for AttestationAdmission {
    fn default() -> AttestationAdmission {
        AttestationAdmission::all()
    }
}

impl AttestationAdmission {
    /// No policy: every attestation is admitted.
    pub fn all() -> AttestationAdmission {
        AttestationAdmission {
            judged: BTreeMap::new(),
            unjudged: true,
        }
    }

    /// An admission from judged attestations, deciding the rest by `unjudged`.
    pub fn new(judged: BTreeMap<[u8; 32], bool>, unjudged: bool) -> AttestationAdmission {
        AttestationAdmission { judged, unjudged }
    }

    /// Whether this admits everything without looking, so a caller can skip hashing.
    pub fn is_all(&self) -> bool {
        self.unjudged && self.judged.values().all(|&ok| ok)
    }

    /// Whether `a` counts.
    pub fn admits(&self, a: &Attestation) -> bool {
        if self.is_all() {
            return true;
        }
        let h = Store::record_hash(&Record::Attestation(a.clone()));
        self.judged.get(&h).copied().unwrap_or(self.unjudged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{AgentId, Hlc, Op, Rung, Uid};

    fn att(agent: &str) -> Attestation {
        let a = AgentId::new(agent).unwrap();
        Attestation::new(
            Uid::from_bytes([7; 32]),
            a.clone(),
            Op::Authored,
            Rung::Document,
            Hlc::new(1, 0, a),
        )
    }

    fn hash(a: &Attestation) -> [u8; 32] {
        Store::record_hash(&Record::Attestation(a.clone()))
    }

    #[test]
    fn the_default_admits_everything() {
        let d = AttestationAdmission::default();
        assert!(d.is_all());
        assert!(d.admits(&att("human:a")));
    }

    #[test]
    fn a_judged_attestation_takes_its_verdict() {
        let (good, bad) = (att("human:a"), att("human:b"));
        let adm =
            AttestationAdmission::new([(hash(&good), true), (hash(&bad), false)].into(), true);
        assert!(!adm.is_all());
        assert!(adm.admits(&good));
        assert!(!adm.admits(&bad));
    }

    /// R1-2: what the policy never saw is decided by `unjudged`, so a strict policy fails closed.
    #[test]
    fn an_unjudged_attestation_takes_the_mode_default() {
        let seen = att("human:a");
        let unseen = att("human:c");
        let strict = AttestationAdmission::new([(hash(&seen), true)].into(), false);
        assert!(strict.admits(&seen));
        assert!(!strict.admits(&unseen));
        let advisory = AttestationAdmission::new([(hash(&seen), true)].into(), true);
        assert!(advisory.admits(&unseen));
    }
}
