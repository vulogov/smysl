//! §8 of the specification, as far as it can be mechanised.
//!
//! Most of a versioning policy is a promise about future behaviour, which no test can check.
//! Two parts of it are checkable today, and the trap this file was written to catch has now
//! sprung: 0.14 grew `FORMAT_VERSIONS_SUPPORTED` to two entries, and the count assertion that
//! used to sit here has been replaced by the property it was standing in for.

use smysl_core::surface::{parse_surface, write_surface, WriteContext};
use smysl_core::{
    format_version_supported, kernel_major, FORMAT_VERSIONS_SUPPORTED, FORMAT_VERSION_DEFAULT,
    KERNEL_MAJOR, KERNEL_SCHEMA,
};

/// §8.2: a reader must refuse a version absent from its list, and must not infer
/// compatibility from one looking close to a version it knows.
#[test]
fn an_unknown_format_version_is_refused_rather_than_guessed_at() {
    assert!(format_version_supported("smysl/0.1"));
    for near_miss in [
        "smysl/0.2",   // the next one
        "smysl/2.0",   // a major bump
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
    assert_eq!(FORMAT_VERSIONS_SUPPORTED, &["smysl/0.1", "smysl/1.0"]);

    // `smysl/1.0` as of 0.15. The pin here read `smysl/0.1` through 0.14, deliberately: the
    // writer must not emit a version until a release that *reads* it is on the registry, and
    // this line is what made flipping it early a failing test rather than a judgement call.
    assert_eq!(FORMAT_VERSION_DEFAULT, "smysl/1.0");

    // `smysl/0.1` must stay readable forever. That is the promise step 1 was for, and the
    // reason old documents keep working is that nothing removed it from the list above.
    assert!(format_version_supported("smysl/0.1"));

    // The kernel schema is a third axis and did not move with the format version. §8 keeps
    // them separate, and bumping one because the other moved would be the coupling it forbids.
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
    // The tripwire fired in 0.14, which is what it was for. What replaced it is the
    // property it was guarding: a document declaring either supported version must come back
    // declaring the one it declared. A count cannot say that; a round trip can.
    for declared in FORMAT_VERSIONS_SUPPORTED {
        let src = format!(
            "@doc {declared} {{\n  id: v/round-trip\n  lang: en\n}}\n\n\
             @claim c/one {{ status: speculative }}\n~ a claim that survives a round trip\n"
        );
        let out = parse_surface(&src).unwrap_or_else(|e| panic!("`{declared}` must parse: {e}"));
        assert_eq!(
            &out.format_version, declared,
            "the parser dropped the declared version"
        );

        let ctx =
            WriteContext::from_labels(&out.labels).with_format_version(out.format_version.clone());
        let back = write_surface(out.view.as_ref(), &out.records, &ctx);
        assert!(
            back.starts_with(&format!("@doc {declared} ")),
            "a document declaring `{declared}` was rewritten as: {}",
            back.lines().next().unwrap_or("")
        );
    }
}
