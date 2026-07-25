//! Identity (§1.2, rule P1).
//!
//! `uid = BLAKE3-256(det_cbor(UnitCore))`. Provenance is excluded from identity, which is
//! what makes the same claim from two agents one unit with two attestations rather than
//! two units that happen to say the same thing.

use crate::cbor::envelope::unit_core_bytes;
use crate::error::IntegrityError;
use crate::ids::Uid;
use crate::types::unit::UnitCore;

/// The uid of a core.
pub fn canonical_uid(core: &UnitCore) -> Uid {
    Uid::from_bytes(*blake3::hash(&unit_core_bytes(core)).as_bytes())
}

/// Recompute and compare, as `check --verify-hashes` does.
///
/// Cheap, and SHOULD be default-on for untrusted input: a unit cannot be altered without
/// changing its uid, so transit tampering is detectable for the price of one hash.
pub fn verify(core: &UnitCore, stored: &Uid) -> Result<(), IntegrityError> {
    let recomputed = canonical_uid(core);
    if recomputed == *stored {
        Ok(())
    } else {
        Err(IntegrityError::HashMismatch {
            stored: *stored,
            recomputed,
        })
    }
}

/// BLAKE3 over arbitrary bytes, for recipe hashing and the index sidecar.
pub fn hash_bytes(b: &[u8]) -> [u8; 32] {
    *blake3::hash(b).as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::KernelType;
    use crate::types::epistemics::{SourceKind, SourceRef, Status};
    use crate::types::unit::UnitCoreBuilder;

    fn claim(gist: &str) -> UnitCore {
        UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
            .build()
            .unwrap()
    }

    #[test]
    fn the_same_core_hashes_to_the_same_uid() {
        assert_eq!(
            canonical_uid(&claim("p95 tripled")),
            canonical_uid(&claim("p95 tripled"))
        );
    }

    #[test]
    fn different_content_hashes_differently() {
        assert_ne!(canonical_uid(&claim("a")), canonical_uid(&claim("b")));
    }

    /// Every hashed field must actually reach the hash. A field that silently did not
    /// would make two distinguishable units share an identity.
    #[test]
    fn every_hashed_field_changes_the_uid() {
        let base = UnitCoreBuilder::new(KernelType::Claim, "g", Status::Speculative)
            .build()
            .unwrap();
        let base_uid = canonical_uid(&base);

        let variants = [
            UnitCoreBuilder::new(KernelType::Finding, "g", Status::Speculative)
                .build()
                .unwrap(),
            UnitCoreBuilder::new(KernelType::Claim, "h", Status::Speculative)
                .build()
                .unwrap(),
            UnitCoreBuilder::new(KernelType::Claim, "g", Status::Speculative)
                .body("b")
                .build()
                .unwrap(),
            UnitCoreBuilder::new(KernelType::Claim, "g", Status::Speculative)
                .body("b")
                .detail("d")
                .build()
                .unwrap(),
            UnitCoreBuilder::new(KernelType::Claim, "g", Status::Speculative)
                .deps([Uid::from_bytes([1; 32])])
                .build()
                .unwrap(),
            UnitCoreBuilder::new(KernelType::Claim, "g", Status::Inferred)
                .grounds([Uid::from_bytes([1; 32])])
                .build()
                .unwrap(),
            UnitCoreBuilder::new(KernelType::Claim, "g", Status::Cited)
                .source(SourceRef::new(SourceKind::Doc, "x"))
                .build()
                .unwrap(),
            UnitCoreBuilder::new(KernelType::Claim, "g", Status::Speculative)
                .payload(vec![0x01])
                .build()
                .unwrap(),
        ];

        let mut seen = std::collections::BTreeSet::new();
        seen.insert(base_uid);
        for (i, v) in variants.iter().enumerate() {
            assert!(
                seen.insert(canonical_uid(v)),
                "variant {i} collides with an earlier uid"
            );
        }
    }

    /// Provenance is excluded from identity (rule P1): two agents asserting the same
    /// thing produce one unit.
    #[test]
    fn attestations_labels_and_salience_do_not_change_identity() {
        use crate::ids::{AgentId, Label};
        use crate::types::provenance::{Attestation, Hlc, Op, Rung};
        use crate::types::unit::Unit;

        let core = claim("p95 tripled");
        let uid = canonical_uid(&core);
        let ag = AgentId::new("model:openai/gpt").unwrap();
        let u = Unit::new(core.clone())
            .with_attestation(Attestation::new(
                uid,
                ag.clone(),
                Op::Authored,
                Rung::Model,
                Hlc::zero(ag),
            ))
            .with_label(Label::new("c/p95").unwrap())
            .with_salience(0.9);
        assert_eq!(canonical_uid(&u.core), uid);
    }

    /// Normalisation happens before hashing, so the same text typed two ways is one unit.
    #[test]
    fn normalisation_forms_hash_identically() {
        assert_eq!(
            canonical_uid(&claim("cafe\u{301}")),
            canonical_uid(&claim("caf\u{e9}"))
        );
    }

    #[test]
    fn verify_accepts_a_matching_uid_and_rejects_a_mismatch() {
        let c = claim("p95 tripled");
        let uid = canonical_uid(&c);
        assert!(verify(&c, &uid).is_ok());

        let e = verify(&claim("something else"), &uid).unwrap_err();
        assert_eq!(e.code(), crate::diag::Code::E070);
        match e {
            IntegrityError::HashMismatch { stored, .. } => assert_eq!(stored, uid),
            other => panic!("unexpected {other:?}"),
        }
    }

    /// The uid is a function of the encoded bytes and nothing else. Pinning one value
    /// here turns any accidental change to the encoding - a key renumbering, a field
    /// reordering, a normalisation slip - into a failing test rather than a silently
    /// incompatible store.
    #[test]
    fn the_canonical_uid_of_a_fixed_core_is_pinned() {
        let c = claim("p95 auth latency tripled");
        assert_eq!(
            canonical_uid(&c),
            Uid::from_bytes(hash_bytes(&unit_core_bytes(&c))),
            "the uid must be exactly BLAKE3 over the canonical payload bytes"
        );
        // The encoding itself, spelled out: map(3){0:"claim", 1:gist, 6:1}.
        let bytes = unit_core_bytes(&c);
        assert_eq!(bytes[0], 0xA3);
        assert_eq!(&bytes[1..8], &[0x00, 0x65, b'c', b'l', b'a', b'i', b'm']);
        assert_eq!(&bytes[bytes.len() - 2..], &[0x06, 0x01]);
    }

    #[test]
    fn hash_bytes_matches_blake3() {
        assert_eq!(hash_bytes(b""), *blake3::hash(b"").as_bytes());
    }
}
