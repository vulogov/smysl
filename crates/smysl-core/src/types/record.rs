//! The record envelope (§7.2, Appendix B).
//!
//! `[ type_code: uint, payload: map ]`. Integer keys rather than CBOR tags: no IANA
//! registration, more compact, and determinism is easier to guarantee.

use crate::types::aux::{Contention, LabelBinding, PackInfo, SchemaDecl};
use crate::types::provenance::Attestation;
use crate::types::relation::Relation;
use crate::types::thread::Thread;
use crate::types::unit::UnitCore;
use crate::types::view::View;

/// Envelope type codes.
pub mod code {
    pub const UNIT_CORE: u64 = 1;
    pub const ATTESTATION: u64 = 2;
    pub const RELATION: u64 = 3;
    pub const THREAD: u64 = 4;
    pub const VIEW: u64 = 5;
    pub const CONTENTION: u64 = 6;
    pub const PACK_INFO: u64 = 7;
    pub const SCHEMA_DECL: u64 = 8;
    /// Reserved for checkpointing (D-11). Not implemented: the format interacts with
    /// content addressing and must not be retrofitted, but 0.1 does not need it.
    pub const CHECKPOINT: u64 = 9;
    /// A label bound to the uid it names (0.2.0).
    ///
    /// Additive by construction: a 0.1 reader decodes this as `Record::Unknown`, preserves
    /// the payload verbatim and re-emits it identically, so a 0.2 store round-trips through
    /// an older build without loss. Verified rather than assumed.
    pub const LABEL_BINDING: u64 = 10;

    pub const KNOWN: &[u64] = &[
        UNIT_CORE,
        ATTESTATION,
        RELATION,
        THREAD,
        VIEW,
        CONTENTION,
        PACK_INFO,
        SCHEMA_DECL,
        LABEL_BINDING,
    ];
}

/// One record in a store.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Record {
    Unit(UnitCore),
    Attestation(Attestation),
    Relation(Relation),
    Thread(Thread),
    View(View),
    Contention(Contention),
    PackInfo(PackInfo),
    SchemaDecl(SchemaDecl),
    /// A label bound to the uid it names. Not identity: never hashed.
    LabelBinding(LabelBinding),
    /// A record type this build does not know (`SMY-W014`).
    ///
    /// Preserved verbatim - payload bytes exactly as they arrived - and skipped
    /// semantically. Dropping it would break rule X at the record level and silently
    /// corrupt a store written by a later minor version.
    Unknown {
        code: u64,
        payload: Vec<u8>,
    },
}

impl Record {
    pub fn type_code(&self) -> u64 {
        match self {
            Record::Unit(_) => code::UNIT_CORE,
            Record::Attestation(_) => code::ATTESTATION,
            Record::Relation(_) => code::RELATION,
            Record::Thread(_) => code::THREAD,
            Record::View(_) => code::VIEW,
            Record::Contention(_) => code::CONTENTION,
            Record::PackInfo(_) => code::PACK_INFO,
            Record::SchemaDecl(_) => code::SCHEMA_DECL,
            Record::LabelBinding(_) => code::LABEL_BINDING,
            Record::Unknown { code, .. } => *code,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Record::Unit(_) => "unit",
            Record::Attestation(_) => "attestation",
            Record::Relation(_) => "relation",
            Record::Thread(_) => "thread",
            Record::View(_) => "view",
            Record::Contention(_) => "contention",
            Record::PackInfo(_) => "packinfo",
            Record::SchemaDecl(_) => "schemadecl",
            Record::LabelBinding(_) => "labelbinding",
            Record::Unknown { .. } => "unknown",
        }
    }

    pub const fn is_unknown(&self) -> bool {
        matches!(self, Record::Unknown { .. })
    }

    /// The only record type whose bytes determine an identity.
    pub const fn is_hashed(&self) -> bool {
        matches!(self, Record::Unit(_))
    }

    pub const fn as_unit(&self) -> Option<&UnitCore> {
        match self {
            Record::Unit(u) => Some(u),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::KernelType;
    use crate::types::epistemics::Status;
    use crate::types::unit::UnitCoreBuilder;

    fn unit() -> UnitCore {
        UnitCoreBuilder::new(KernelType::Claim, "a claim", Status::Speculative)
            .build()
            .unwrap()
    }

    #[test]
    fn type_codes_match_appendix_b() {
        assert_eq!(code::UNIT_CORE, 1);
        assert_eq!(code::ATTESTATION, 2);
        assert_eq!(code::RELATION, 3);
        assert_eq!(code::THREAD, 4);
        assert_eq!(code::VIEW, 5);
        assert_eq!(code::CONTENTION, 6);
        assert_eq!(code::PACK_INFO, 7);
        assert_eq!(code::SCHEMA_DECL, 8);
        assert_eq!(code::CHECKPOINT, 9);
    }

    #[test]
    fn checkpoint_is_reserved_not_known() {
        assert!(!code::KNOWN.contains(&code::CHECKPOINT));
    }

    /// Ascending, and with a hole. Codes 1-8 are 0.1's records; 9 stays reserved for
    /// checkpointing, whose format interacts with content addressing and must not be
    /// retrofitted; 10 is 0.2's label binding. The list was contiguous until the hole
    /// became real, and contiguity was never the property that mattered - being ascending
    /// and free of duplicates is, since a code is a permanent wire commitment.
    #[test]
    fn known_codes_ascend_and_skip_the_reserved_slot() {
        assert_eq!(code::KNOWN, &[1, 2, 3, 4, 5, 6, 7, 8, 10]);
        assert!(code::KNOWN.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn records_report_their_own_type_code() {
        assert_eq!(Record::Unit(unit()).type_code(), 1);
        assert_eq!(
            Record::Unknown {
                code: 99,
                payload: vec![]
            }
            .type_code(),
            99
        );
    }

    #[test]
    fn only_the_unit_record_is_hashed() {
        assert!(Record::Unit(unit()).is_hashed());
        assert!(!Record::Unknown {
            code: 99,
            payload: vec![]
        }
        .is_hashed());
    }

    #[test]
    fn unknown_records_are_recognisable_as_such() {
        let r = Record::Unknown {
            code: 99,
            payload: vec![0xA0],
        };
        assert!(r.is_unknown());
        assert_eq!(r.type_name(), "unknown");
        assert!(!Record::Unit(unit()).is_unknown());
    }

    #[test]
    fn as_unit_projects_only_unit_records() {
        assert!(Record::Unit(unit()).as_unit().is_some());
        assert!(Record::Unknown {
            code: 9,
            payload: vec![]
        }
        .as_unit()
        .is_none());
    }
}
