//! Map keys, one module per record type (Appendix B).
//!
//! Integer keys rather than CBOR tags: no IANA registration, more compact, and
//! determinism is easier to guarantee. The keys of `unit` are load-bearing in a way the
//! others are not - they are hash input, so renumbering one changes every uid in every
//! store, which §11 classifies as a **major** break.

/// UnitCore (type code 1) - hashed.
pub mod unit {
    pub const SCHEMA: u16 = 0;
    pub const GIST: u16 = 1;
    pub const BODY: u16 = 2;
    pub const DETAIL: u16 = 3;
    pub const DEPS: u16 = 4;
    pub const GROUNDS: u16 = 5;
    pub const STATUS: u16 = 6;
    pub const SOURCE: u16 = 7;
    pub const PAYLOAD: u16 = 8;
    pub const HIGHEST: u16 = PAYLOAD;
}

/// Attestation (type code 2) - not hashed.
pub mod attestation {
    pub const UID: u16 = 0;
    pub const AGENT: u16 = 1;
    pub const HOP: u16 = 2;
    pub const PARENTS: u16 = 3;
    pub const TS: u16 = 4;
    pub const OP: u16 = 5;
    pub const RECIPE: u16 = 6;
    pub const SIG: u16 = 7;
    pub const RUNG: u16 = 8;
    pub const FAMILY: u16 = 9;
    pub const HIGHEST: u16 = FAMILY;
}

/// Relation (type code 3).
pub mod relation {
    pub const KIND: u16 = 0;
    pub const FROM: u16 = 1;
    pub const TO: u16 = 2;
    pub const WEIGHT: u16 = 3;
    pub const NOTE: u16 = 4;
    pub const HIGHEST: u16 = NOTE;
}

/// Thread (type code 4).
pub mod thread {
    pub const ID: u16 = 0;
    pub const SCHEMA: u16 = 1;
    pub const OWNER: u16 = 2;
    pub const GIST: u16 = 3;
    pub const STEPS: u16 = 4;
    pub const TS: u16 = 5;
    pub const HIGHEST: u16 = TS;
}

/// View (type code 5).
pub mod view {
    pub const ID: u16 = 0;
    pub const ROOTS: u16 = 1;
    pub const THREADS: u16 = 2;
    pub const REQUIRES: u16 = 3;
    pub const GRANULARITY: u16 = 4;
    pub const INTENT: u16 = 5;
    pub const LANG: u16 = 6;
    pub const HIGHEST: u16 = LANG;
}

/// Contention (type code 6).
pub mod contention {
    pub const ID: u16 = 0;
    pub const OVER: u16 = 1;
    pub const POSITIONS: u16 = 2;
    pub const DETECTED: u16 = 3;
    pub const STATUS: u16 = 4;
    pub const HIGHEST: u16 = STATUS;
}

/// PackInfo (type code 7).
pub mod packinfo {
    pub const BUDGET: u16 = 0;
    pub const USED: u16 = 1;
    pub const THREAD: u16 = 2;
    pub const DROPPED: u16 = 3;
    pub const DEGRADED: u16 = 4;
    pub const OPTIMALITY: u16 = 5;
    pub const ESTIMATOR: u16 = 6;
    pub const HIGHEST: u16 = ESTIMATOR;
}

/// SchemaDecl (type code 8).
pub mod schema_decl {
    pub const ID: u16 = 0;
    pub const VERSION: u16 = 1;
    pub const TYPES: u16 = 2;
    pub const RELATIONS: u16 = 3;
    pub const PAYLOAD_SHAPE: u16 = 4;
    pub const HIGHEST: u16 = PAYLOAD_SHAPE;
}

/// SourceRef, nested inside a unit under [`unit::SOURCE`].
pub mod source {
    pub const KIND: u16 = 0;
    pub const REFERENCE: u16 = 1;
    pub const CAPTURED: u16 = 2;
    pub const HIGHEST: u16 = CAPTURED;
}

/// GranularityProfile, nested inside a view under [`view::GRANULARITY`].
pub mod granularity {
    pub const PROFILE: u16 = 0;
    pub const L0_MAX: u16 = 1;
    pub const L1_MIN: u16 = 2;
    pub const L1_MAX: u16 = 3;
    pub const ADMISSION: u16 = 4;
    pub const HIGHEST: u16 = ADMISSION;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every key table must be contiguous from zero. A gap would mean a field was
    /// removed, and removing a hashed field is a major break, not a tidy-up.
    #[test]
    fn key_tables_are_contiguous_from_zero() {
        let tables: &[(&str, u16, &[u16])] = &[
            (
                "unit",
                unit::HIGHEST,
                &[
                    unit::SCHEMA,
                    unit::GIST,
                    unit::BODY,
                    unit::DETAIL,
                    unit::DEPS,
                    unit::GROUNDS,
                    unit::STATUS,
                    unit::SOURCE,
                    unit::PAYLOAD,
                ],
            ),
            (
                "attestation",
                attestation::HIGHEST,
                &[
                    attestation::UID,
                    attestation::AGENT,
                    attestation::HOP,
                    attestation::PARENTS,
                    attestation::TS,
                    attestation::OP,
                    attestation::RECIPE,
                    attestation::SIG,
                    attestation::RUNG,
                    attestation::FAMILY,
                ],
            ),
            (
                "relation",
                relation::HIGHEST,
                &[
                    relation::KIND,
                    relation::FROM,
                    relation::TO,
                    relation::WEIGHT,
                    relation::NOTE,
                ],
            ),
            (
                "thread",
                thread::HIGHEST,
                &[
                    thread::ID,
                    thread::SCHEMA,
                    thread::OWNER,
                    thread::GIST,
                    thread::STEPS,
                    thread::TS,
                ],
            ),
            (
                "view",
                view::HIGHEST,
                &[
                    view::ID,
                    view::ROOTS,
                    view::THREADS,
                    view::REQUIRES,
                    view::GRANULARITY,
                    view::INTENT,
                    view::LANG,
                ],
            ),
            (
                "contention",
                contention::HIGHEST,
                &[
                    contention::ID,
                    contention::OVER,
                    contention::POSITIONS,
                    contention::DETECTED,
                    contention::STATUS,
                ],
            ),
            (
                "packinfo",
                packinfo::HIGHEST,
                &[
                    packinfo::BUDGET,
                    packinfo::USED,
                    packinfo::THREAD,
                    packinfo::DROPPED,
                    packinfo::DEGRADED,
                    packinfo::OPTIMALITY,
                    packinfo::ESTIMATOR,
                ],
            ),
            (
                "schema_decl",
                schema_decl::HIGHEST,
                &[
                    schema_decl::ID,
                    schema_decl::VERSION,
                    schema_decl::TYPES,
                    schema_decl::RELATIONS,
                    schema_decl::PAYLOAD_SHAPE,
                ],
            ),
            (
                "source",
                source::HIGHEST,
                &[source::KIND, source::REFERENCE, source::CAPTURED],
            ),
            (
                "granularity",
                granularity::HIGHEST,
                &[
                    granularity::PROFILE,
                    granularity::L0_MAX,
                    granularity::L1_MIN,
                    granularity::L1_MAX,
                    granularity::ADMISSION,
                ],
            ),
        ];

        for (name, highest, keys) in tables {
            let expected: Vec<u16> = (0..keys.len() as u16).collect();
            assert_eq!(*keys, expected.as_slice(), "{name} keys are not 0..n");
            assert_eq!(
                *highest,
                keys.len() as u16 - 1,
                "{name}::HIGHEST disagrees with the table"
            );
        }
    }

    /// The unit key table is hash input. Pinning it here means a renumbering shows up as
    /// a failing test rather than as a store full of unverifiable uids.
    #[test]
    fn unit_keys_are_pinned_to_appendix_b() {
        assert_eq!(unit::SCHEMA, 0);
        assert_eq!(unit::GIST, 1);
        assert_eq!(unit::BODY, 2);
        assert_eq!(unit::DETAIL, 3);
        assert_eq!(unit::DEPS, 4);
        assert_eq!(unit::GROUNDS, 5);
        assert_eq!(unit::STATUS, 6);
        assert_eq!(unit::SOURCE, 7);
        assert_eq!(unit::PAYLOAD, 8);
    }

    /// Appendix B places `rung` and `family` after `sig`, not in field order. That looks
    /// like an accident of drafting but it is wire format, so it is pinned too.
    #[test]
    fn attestation_keys_keep_their_appendix_b_numbering() {
        assert_eq!(attestation::OP, 5);
        assert_eq!(attestation::RECIPE, 6);
        assert_eq!(attestation::SIG, 7);
        assert_eq!(attestation::RUNG, 8);
        assert_eq!(attestation::FAMILY, 9);
    }
}
