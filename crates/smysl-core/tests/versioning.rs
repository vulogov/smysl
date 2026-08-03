//! §8 of the specification, as far as it can be mechanised.
//!
//! Most of a versioning policy is a promise about future behaviour, which no test can check.
//! Two parts of it are checkable today, and one of them is a trap that is currently harmless.

use smysl_core::{
    format_version_supported, kernel_major, FORMAT_VERSIONS_SUPPORTED, KERNEL_MAJOR, KERNEL_SCHEMA,
};

/// §8.2: a reader must refuse a version absent from its list, and must not infer
/// compatibility from one looking close to a version it knows.
#[test]
fn an_unknown_format_version_is_refused_rather_than_guessed_at() {
    assert!(format_version_supported("smysl/0.1"));
    for near_miss in [
        "smysl/0.2",   // the next one
        "smysl/1.0",   // a major bump
        "smysl/0.1.0", // longer, same numbers
        "smysl/0.10",  // 0.1 is a prefix of this, which is the trap
        "smysl",       // no version at all
        "SMYSL/0.1",   // case
        " smysl/0.1",  // untrimmed
    ] {
        assert!(
            !format_version_supported(near_miss),
            "{near_miss:?} was accepted; §8.2 says refuse rather than infer"
        );
    }
}

#[test]
fn the_declared_versions_are_what_the_specification_says() {
    assert_eq!(FORMAT_VERSIONS_SUPPORTED, &["smysl/0.1"]);
    assert_eq!(KERNEL_SCHEMA, "smysl.kernel/0.1");
    assert_eq!(kernel_major(KERNEL_SCHEMA), Some(KERNEL_MAJOR));
}

/// A tripwire for the day §8.2's "accept several at once" is actually used.
///
/// The format version lives only in surface syntax — the wire carries no version string at
/// all. `parse` validates the `@doc` version and then discards it: `ParseOutcome` has no
/// field for it. `write_surface` therefore cannot preserve what it was given and emits
/// `FORMAT_VERSIONS_SUPPORTED[0]` unconditionally.
///
/// With one supported version that is correct by coincidence. With two it is a relabelling
/// bug: a document declaring the second version would be read, and written back out claiming
/// to be the first. Content is unaffected — uids are over CBOR, which carries no version —
/// but the header would lie, and the reader downstream trusts the header.
///
/// So this fails the moment the list grows, which is the moment somebody has to decide where
/// the declared version is kept. That is cheaper than discovering it afterwards from a
/// document that misdescribes itself.
#[test]
fn supporting_a_second_format_version_needs_the_writer_fixed_first() {
    assert_eq!(
        FORMAT_VERSIONS_SUPPORTED.len(),
        1,
        "FORMAT_VERSIONS_SUPPORTED has grown. Before this is right, `ParseOutcome` needs to \
         carry the version the document declared and `write_surface` needs to emit that \
         rather than FORMAT_VERSIONS_SUPPORTED[0] (surface/write.rs). Otherwise a document \
         declaring the second version is silently rewritten as the first. See §8.2."
    );
}
