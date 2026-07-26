//! `smysl` - an AI<->AI<->Human data interchange format, library, and CLI.
//!
//! The library is the product; the CLI is its first consumer (principle P8). Rule A
//! holds throughout: no CLI capability may be unreachable from here, and no code path may
//! be CLI-only.
//!
//! # Feature shape
//!
//! With `default-features = false` this crate is a fully synchronous library with no
//! async runtime, no HTTP client, and no argument parser in its dependency tree
//! (rule B, verified by `xtask check-purity` in CI). Model access is opt-in behind
//! `local` or `remote`.
//!
//! # Guarantees to embedders (§12.3)
//!
//! - **A1** No panics on untrusted input.
//! - **A2** No global state, no implicit I/O.
//! - **A3** No hidden async - only `smysl-provider` needs a runtime.
//! - **A4** Typed, `#[non_exhaustive]` errors.
//! - **A5** Determinism is part of the API: making `pack`, `merge`, `derive_thread`,
//!   `salience`, or `render` non-reproducible is a breaking change regardless of
//!   signature.
//! - **A6** No hidden allocation cliffs.
//!
//! # Build status
//!
//! This is SM-P1 (deterministic codec). The public surface below grows phase by phase
//! toward the full contract of §12.2; each phase stabilises its library API before the
//! matching CLI subcommand is wired to it.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

// ---- core: identifiers, kernel types, codec, identity ----------------------
pub use smysl_core::surface;
pub use smysl_core::surface::{parse_surface, write_surface, ParseOutcome, WriteContext};
pub use smysl_core::{
    canonical_uid, format_version_supported, from_cbor, from_cbor_seq, hash_bytes, kernel_major,
    quantise, to_cbor, to_cbor_seq, unit_core_bytes, verify, Admission, AgentId, AgentKind,
    Attestation, Code, CodecError, Contention, ContentionId, ContentionStatus, Date, Detected,
    DetectionKind, Diagnostic, DropReason, Error, ExitCode, Extra, Fidelity, GranularityProfile,
    Group, Hlc, IdError, IntegrityError, KernelType, Label, LangTag, Lod, NonDetReason, Op,
    Optimality, PackInfo, PackMode, ParseError, Record, RelKind, Relation, Report, Role, Rung,
    SchemaDecl, SchemaId, Severity, ShapeError, SourceKind, SourceRef, Span, Status, Step, Subject,
    Thread, ThreadId, ThreadSchema, Uid, UidPrefix, Unit, UnitCore, UnitCoreBuilder, View, ViewId,
    FORMAT_VERSIONS_SUPPORTED, KERNEL_MAJOR, KERNEL_SCHEMA,
};

// ---- check ----------------------------------------------------------------
pub use smysl_check::{
    check, check_and_fail_on, granularity_distribution, CheckOptions, ConformanceClass, Pass,
};

// ---- pack -----------------------------------------------------------------
pub use smysl_pack::PackError;

// ---- render ---------------------------------------------------------------
pub use smysl_render::{RenderError, Target};

// ---- graph ----------------------------------------------------------------
pub use smysl_graph::{
    closure, cycles, rebuttals_of, reverse_closure, topo, Adjacency, AppendReport, Cached, Edge,
    EdgeKind, EdgeSet, Entry, Index, IndexError, MergeError, NodeId, OpenReport, Scratch, Store,
    StoreOptions, TopoOrder,
};

// ---- ingest / providers (feature-gated) -----------------------------------
#[cfg(feature = "ingest")]
pub use smysl_ingest::{IngestPath, DEFAULT_REPAIR_ATTEMPTS};
#[cfg(feature = "providers")]
pub use smysl_provider::{ProviderError, StructuredMode};

/// The crate version, as a convenience for embedders recording provenance.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facade_reexports_the_diagnostic_registry() {
        assert_eq!(Code::ALL.len(), 49);
        assert_eq!(Code::E030.severity(), Severity::Error);
    }

    #[test]
    fn facade_reexports_version_constants() {
        assert!(format_version_supported(FORMAT_VERSIONS_SUPPORTED[0]));
        assert_eq!(kernel_major(KERNEL_SCHEMA), Some(KERNEL_MAJOR));
    }

    #[test]
    fn crate_version_is_the_cargo_version() {
        assert_eq!(VERSION, "0.1.0");
    }

    /// A crate major bump MUST NOT imply a format break, and vice versa (§11). The two
    /// axes are independent, so they are asserted independently.
    #[test]
    fn crate_and_format_versions_are_independent_axes() {
        assert_ne!(VERSION, FORMAT_VERSIONS_SUPPORTED[0]);
    }

    /// The canonical consumer shape of §12.2, as far as SM-P1 reaches: build a core,
    /// hash it, encode it, read it back.
    #[test]
    fn a_unit_round_trips_through_the_facade_alone() {
        let core = UnitCoreBuilder::new(
            KernelType::Claim,
            "p95 auth latency tripled",
            Status::Speculative,
        )
        .build()
        .unwrap();
        let uid = canonical_uid(&core);
        let bytes = to_cbor(&Record::Unit(core.clone()));
        let (decoded, n) = from_cbor(&bytes).unwrap();
        assert_eq!(n, bytes.len());
        assert_eq!(decoded.as_unit(), Some(&core));
        assert!(verify(decoded.as_unit().unwrap(), &uid).is_ok());
    }
}
