//! The record envelope (§7.2, Appendix B).
//!
//! `[ type_code: uint, payload: map ]`. Integer keys rather than CBOR tags: no IANA
//! registration, more compact, and determinism is easier to guarantee.

use crate::types::aux::{Contention, PackInfo, SchemaDecl};
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

    pub const KNOWN: &[u64] = &[
        UNIT_CORE,
        ATTESTATION,
        RELATION,
        THREAD,
        VIEW,
        CONTENTION,
        PACK_INFO,
        SCHEMA_DECL,
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
        assert_eq!(code::KNOWN.len(), 8);
        assert!(!code::KNOWN.contains(&code::CHECKPOINT));
    }

    #[test]
    fn known_codes_are_contiguous_from_one() {
        assert_eq!(code::KNOWN, &[1, 2, 3, 4, 5, 6, 7, 8]);
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
