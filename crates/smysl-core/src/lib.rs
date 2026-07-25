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
    quantise, Admission, Attestation, Contention, ContentionStatus, Date, Detected, DetectionKind,
    DropReason, Extra, Fidelity, GranularityProfile, Hlc, Lod, Op, Optimality, PackInfo, PackMode,
    Record, RelKind, Relation, Role, Rung, SchemaDecl, SourceKind, SourceRef, Status, Step, Thread,
    ThreadSchema, Unit, UnitCore, UnitCoreBuilder, View,
};

/// Format versions this implementation accepts in a `@doc` header (§11).
pub const FORMAT_VERSIONS_SUPPORTED: &[&str] = &["smysl/0.1"];

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
        assert_eq!(FORMAT_VERSIONS_SUPPORTED, &["smysl/0.1"]);
        assert_eq!(KERNEL_SCHEMA, "smysl.kernel/0.1");
        assert_eq!(kernel_major(KERNEL_SCHEMA), Some(KERNEL_MAJOR));
    }

    #[test]
    fn accepts_only_declared_format_versions() {
        assert!(format_version_supported("smysl/0.1"));
        assert!(!format_version_supported("smysl/0.2"));
        assert!(!format_version_supported("smysl/1.0"));
        assert!(!format_version_supported("smysl"));
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
