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
    quantise, to_cbor, to_cbor_seq, tokens, unit_core_bytes, verify, Admission, AgentId, AgentKind,
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
    check, check_and_fail_on, conformance, fidelity, granularity_distribution, CheckOptions,
    ConformanceClass, ConformanceVerdict, ConsumerProfile, FidelityReport, Pass,
};

// ---- pack -----------------------------------------------------------------
pub use smysl_pack::{
    pack, verify as verify_pack, Constraints, Estimator, Pack, PackError, PackRequest, Reason,
    Selection, Violation,
};

// ---- thread ---------------------------------------------------------------
// `Role` and `ThreadSchema` are wire format and come from the kernel above.
pub use smysl_thread::definition as schema_definition;
pub use smysl_thread::{
    derive_thread, role_weight, role_weights, salience_seed, satisfies_rule_l, DeriveOptions,
    DeriveReport, Matcher, SchemaDef,
};

// ---- render ---------------------------------------------------------------
pub use smysl_render::{
    build as build_ir, emit as emit_artifact, Artifact, Backend, Block, BuildOptions, Connectives,
    Contentions, Ir, LodPlan, Note, NoteKind, Person, Profile, Provenance, Register, RenderError,
    RenderMeta, Show, StatusDisplay, Target, Verbosity,
};

// ---- graph ----------------------------------------------------------------
pub use smysl_graph::{
    closure, cycles, dependents, diff, effective_status, hop_diff, membership, merge,
    plan_retraction, rebuttals_of, reverse_closure, salience, topo, trace, view_roots, Adjacency,
    AgentActivity, AppendReport, Cached, DetectionContext, Edge, EdgeKind, EdgeSet,
    EffectiveStatus, Entry, HopDiff, Index, IndexError, Lineage, LineageNode, MergeError,
    MergeOptions, MergeReport, NodeId, OpenReport, RecipeChange, RecipeChangeKind,
    RetractionAuthority, RetractionPlan, RetractionPolicy, SalienceReport, SalienceRequest,
    SalienceTerms, SalienceWeights, Scratch, Store, StoreDiff, StoreOptions, SupersessionPolicy,
    TopoOrder, TraceKind, Via,
};

// ---- ingest / providers (feature-gated) -----------------------------------
#[cfg(feature = "ingest")]
pub use smysl_ingest::ceiling::ceiling;
#[cfg(feature = "ingest")]
pub use smysl_ingest::path::choose as choose_ingest_path;
#[cfg(feature = "ingest")]
pub use smysl_ingest::recipe::short as recipe_short;
#[cfg(feature = "ingest")]
pub use smysl_ingest::{
    attest, stage, AttestOptions, AttestReport, IngestOptions, IngestPath, IngestReport, Ingestor,
    Judgement, Staged, What, DEFAULT_REPAIR_ATTEMPTS,
};
#[cfg(feature = "providers")]
pub use smysl_provider::usage::{GroupBy, Totals};
#[cfg(feature = "providers")]
pub use smysl_provider::{
    config::ProviderConfig, map::build as build_provider, Capabilities, Completion, Ledger,
    LedgerEntry, Message, Probe, Provider, ProviderConfigFile, ProviderError, ProviderId, Registry,
    Request, StreamMsg, StructuredMode, Task, TokenCount, Usage,
};

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
