//! `smysl-core` - kernel types, deterministic codec, surface syntax, diagnostics.
//!
//! This crate is the bottom of the workspace. It is synchronous, performs no I/O beyond
//! what its callers hand it, and links no runtime, HTTP client, or argument parser
//! (rule B). The only wall-clock read in the pure crates is `Hlc::now`, at record
//! creation time (rule D).
//!
//! SM-P1 delivers the kernel types, the deterministic codec, and identity. The surface
//! syntax lands in SM-P2.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub mod cbor;
pub mod diag;
pub mod error;
pub mod hash;
pub mod ids;
pub mod surface;
pub mod types;

pub use cbor::envelope::unit_core_bytes;
pub use cbor::{from_cbor, from_cbor_seq, to_cbor, to_cbor_seq};
pub use diag::{Code, Diagnostic, Group, Report, Severity, Span, Subject};
pub use error::{
    CodecError, Error, ExitCode, IdError, IntegrityError, MergeError, NonDetReason, PackError,
    ParseError, ProviderError, RenderError, ShapeError,
};
pub use hash::{canonical_uid, hash_bytes, verify};
pub use ids::{
    AgentId, AgentKind, ContentionId, KernelType, Label, LangTag, SchemaId, ThreadId, Uid,
    UidPrefix, ViewId,
};
pub use types::{
    json_escape, quantise, tokens, Admission, Attestation, Contention, ContentionStatus, Date,
    Detected, DetectionKind, DropReason, Extra, Fidelity, GranularityProfile, Hlc, LabelBinding,
    Lod, Op, Optimality, PackInfo, PackMode, Record, RelKind, Relation, Role, Rung, SchemaDecl,
    SourceKind, SourceRef, Status, Step, Thread, ThreadSchema, Unit, UnitCore, UnitCoreBuilder,
    View,
};

/// Format versions this implementation accepts in a `@doc` header (§11).
///
/// Two, as of 0.14, and the order is load-bearing: `[0]` is what a document declares when
/// nothing tells the writer otherwise, so **this list grows at the end until the writer is
/// flipped deliberately**. §0.1 of `ROAD_TO_1.0.md` sequences that: readers learn `smysl/1.0`
/// and are *released* first, and only a later cycle makes it what new documents say. Flip the
/// writer before the field has a reader for it and every other implementation refuses the
/// output — §8.2 requires a reader to reject a version absent from its list rather than guess.
pub const FORMAT_VERSIONS_SUPPORTED: &[&str] = &["smysl/0.1", "smysl/1.0"];

/// What a document declares when the writer has nothing better to go on.
///
/// Separate from `FORMAT_VERSIONS_SUPPORTED[0]` because the two stopped meaning the same thing
/// the moment the list grew: the first is *what we emit*, the list is *what we accept*. They
/// are equal today and will not be after the flip.
pub const FORMAT_VERSION_DEFAULT: &str = FORMAT_VERSIONS_SUPPORTED[0];

/// The kernel schema this implementation implements (§11).
pub const KERNEL_SCHEMA: &str = "smysl.kernel/0.1";

/// The major component of [`KERNEL_SCHEMA`]. A consumer MUST refuse - never silently
/// degrade - when a store requires a kernel major it does not implement (rule X,
/// `SMY-E002`).
pub const KERNEL_MAJOR: u32 = 0;

/// True if this implementation can read a store declaring `version`.
pub fn format_version_supported(version: &str) -> bool {
    FORMAT_VERSIONS_SUPPORTED.contains(&version)
}

/// The major number of a `smysl.kernel/MAJOR[.MINOR]` schema id, if it is well-formed.
///
/// Only the major is load-bearing: a consumer refuses an unknown kernel major
/// (`SMY-E002`) but degrades gracefully across minors (rule X).
pub fn kernel_major(schema: &str) -> Option<u32> {
    let version = schema.strip_prefix("smysl.kernel/")?;
    let major = version.split('.').next()?;
    major.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_format_and_kernel_versions() {
        assert_eq!(FORMAT_VERSIONS_SUPPORTED, &["smysl/0.1", "smysl/1.0"]);
        assert_eq!(
            FORMAT_VERSION_DEFAULT, "smysl/0.1",
            "the writer has not been flipped yet"
        );
        assert_eq!(KERNEL_SCHEMA, "smysl.kernel/0.1");
        assert_eq!(kernel_major(KERNEL_SCHEMA), Some(KERNEL_MAJOR));
    }

    /// Accepted is exactly the declared list — no more, and no fewer.
    ///
    /// The "no fewer" half is new. This asserted `smysl/0.1` and then three rejections, one of
    /// which was `smysl/1.0`; when 0.14 added that version the test failed, which was right,
    /// and it would have kept passing had the list grown by something it did not happen to
    /// name. Driving it from the list itself makes it grow with the list.
    #[test]
    fn accepts_only_declared_format_versions() {
        for v in FORMAT_VERSIONS_SUPPORTED {
            assert!(
                format_version_supported(v),
                "`{v}` is declared but not accepted"
            );
        }
        // Close to a supported version is not supported: §8.2 says refuse rather than infer.
        for v in [
            "smysl/0.2",
            "smysl/2.0",
            "smysl/1.1",
            "smysl",
            "",
            "SMYSL/0.1",
        ] {
            assert!(
                !format_version_supported(v),
                "`{v}` is not declared and must not be accepted"
            );
        }
    }

    #[test]
    fn parses_kernel_majors() {
        assert_eq!(kernel_major("smysl.kernel/0"), Some(0));
        assert_eq!(kernel_major("smysl.kernel/0.1"), Some(0));
        assert_eq!(kernel_major("smysl.kernel/12.7"), Some(12));
        assert_eq!(kernel_major("x.sre/1"), None);
        assert_eq!(kernel_major("smysl.kernel/x"), None);
        assert_eq!(kernel_major("smysl.kernel/"), None);
    }

    /// Only the major matters for the refuse-versus-degrade decision of rule X: a minor
    /// bump must stay readable.
    #[test]
    fn kernel_minor_does_not_change_the_major() {
        assert_eq!(
            kernel_major("smysl.kernel/0.9"),
            kernel_major(KERNEL_SCHEMA)
        );
        assert_ne!(
            kernel_major("smysl.kernel/1.0"),
            kernel_major(KERNEL_SCHEMA)
        );
    }
}
