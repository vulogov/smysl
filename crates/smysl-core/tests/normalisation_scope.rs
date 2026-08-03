//! Every text field that reaches a uid, not just the one that had a test.
//!
//! `normalise` carried the claim "every text field is normalised exactly once, on
//! construction". Two tests backed it, and both used `gist`. `SourceRef::reference` is also
//! inside the unit core, and therefore inside the uid, and it was stored exactly as typed.
//!
//! Nothing reached the wire wrong — `Enc::text` normalises on the way out, which is the 0.6
//! fix §3 constraint 6 describes. But it left `PartialEq` disagreeing with identity: two
//! cores differing only in the normalisation form of a source reference compared **unequal**
//! while hashing to the **same uid**. Anything deduplicating by value rather than by uid kept
//! two copies of one unit.
//!
//! The claim said "every" and one field was checked. This checks every field, so the next one
//! added has somewhere obvious to go.

use smysl_core::hash::canonical_uid;
use smysl_core::ids::KernelType;
use smysl_core::types::*;

/// `e` + U+0301 and the composed U+00E9 are the same text in two normalisation forms.
const DECOMPOSED: &str = "cafe\u{301}";
const COMPOSED: &str = "caf\u{e9}";

/// Each entry builds a core with the given text in one field and everything else fixed.
/// Adding a text field to `UnitCore` means adding a line here; the alternative is a claim
/// that says "every" and a test that checks one.
fn cores_differing_only_in(field: &str, text: &str) -> UnitCore {
    let base = || UnitCoreBuilder::new(KernelType::Claim, "a stable gist", Status::Cited);
    let with_source = |b: UnitCoreBuilder, r: &str| b.source(SourceRef::new(SourceKind::File, r));
    match field {
        "gist" => with_source(
            UnitCoreBuilder::new(KernelType::Claim, text, Status::Cited),
            "r.csv",
        )
        .build()
        .unwrap(),
        "body" => with_source(base().body(text), "r.csv").build().unwrap(),
        "detail" => with_source(base().body("a body").detail(text), "r.csv")
            .build()
            .unwrap(),
        "source.reference" => with_source(base(), text).build().unwrap(),
        other => panic!("unknown field {other}"),
    }
}

const TEXT_FIELDS_IN_THE_UID: [&str; 4] = ["gist", "body", "detail", "source.reference"];

#[test]
fn every_text_field_in_the_uid_is_normalised_on_construction() {
    for field in TEXT_FIELDS_IN_THE_UID {
        let a = cores_differing_only_in(field, DECOMPOSED);
        let b = cores_differing_only_in(field, COMPOSED);
        assert_eq!(
            a, b,
            "{field}: two normalisation forms of one text produced unequal cores; \
             `PartialEq` must agree with identity or value-keyed deduplication keeps both"
        );
        assert_eq!(
            canonical_uid(&a),
            canonical_uid(&b),
            "{field}: the same text in two forms hashed to two uids"
        );
    }
}

/// The control. Without it the test above would pass if every core compared equal to every
/// other — which is exactly how a normalisation test stops meaning anything.
#[test]
fn genuinely_different_text_still_differs() {
    for field in TEXT_FIELDS_IN_THE_UID {
        let a = cores_differing_only_in(field, COMPOSED);
        let b = cores_differing_only_in(field, "something else entirely");
        assert_ne!(a, b, "{field}: different text compared equal");
        assert_ne!(
            canonical_uid(&a),
            canonical_uid(&b),
            "{field}: different text hashed to one uid"
        );
    }
}

/// The far side of the same rule, and the reason it is not redundant with the near side.
///
/// §3 constraint 6 says to normalise *at the encoder*, "not only in the constructors that
/// happen to be remembered" — 0.6 found six fields reaching the encoder unchecked. If someone
/// deletes the encoder-side pass because construction now covers the unit core, this fails.
#[test]
fn the_encoder_normalises_too() {
    use smysl_core::cbor::Enc;
    let mut e = Enc::new();
    e.text(DECOMPOSED);
    let bytes = e.into_bytes();
    let mut expected = Enc::new();
    expected.text(COMPOSED);
    assert_eq!(
        bytes,
        expected.into_bytes(),
        "the encoder must normalise regardless of what construction did"
    );
}
