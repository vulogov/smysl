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
/// Every error this crate can raise, in one enum, with the CLI's exit code attached.
///
/// Exported as `AnyError` rather than `Error`, which is what it is called in `smysl-core`.
/// Inside a crate that name is idiomatic; through a facade that flattens eleven error types
/// into one namespace it is not — `Error` sitting beside `CodecError` and `ParseError` reads
/// as a twelfth sibling rather than as the enum that wraps the other eleven.
pub use smysl_core::Error as AnyError;
// What this crate promises, and what it merely exposes.
//
// `Documentation/API_CONTRACT.md` has the reasoning; the short version belongs here, because a
// consumer reads the crate and not the repository.
//
// **Contract.** The kernel vocabulary — `Uid`, `UnitCore`, `Status`, `RelKind`, `Record` and
// their neighbours — and the twelve operations over it: `parse_surface`, `write_surface`,
// `to_cbor`, `from_cbor`, `canonical_uid`, `check`, `pack`, `merge`, `salience`,
// `derive_thread`, `trace`, `compact`. Their errors, inputs and outputs go with them. These
// are the format, three other implementations model them, and guarantee A5 already promises
// something stronger than semver about the operations: making one non-reproducible is
// breaking whatever the signature says.
//
// **A seam, not a promise.** The provider machinery (`ProviderConfig`, `Request`,
// `Completion`, `Usage`, `StructuredMode`, `Capabilities`), and render and retrieval (`Bm25`,
// `Hybrid`, `Semantic`, `EmbedModel`, `Query`, `Hit`, `Retriever`). These are public because
// the contract needs them, not because anyone designed them to be built on — `Hybrid` changed
// shape twice inside 0.7 alone. Build on them by all means; expect them to move, and pin a
// version if that matters.
//
// The distinction is not enforced by anything. It is stated so that a consumer who relies on
// the second group has been told, rather than finding out at an upgrade.

pub use smysl_core::surface;
pub use smysl_core::surface::{parse_surface, write_surface, ParseOutcome, WriteContext};
pub use smysl_core::{
    canonical_uid, format_version_supported, from_cbor, from_cbor_seq, hash_bytes, json_escape,
    kernel_major, quantise, to_cbor, to_cbor_seq, tokens, unit_core_bytes, verify, Admission,
    AgentId, AgentKind, Attestation, Code, CodecError, Contention, ContentionId, ContentionStatus,
    Date, Detected, DetectionKind, Diagnostic, DropReason, ExitCode, Extra, Fidelity,
    GranularityProfile, Group, Hlc, IdError, IntegrityError, KernelType, Label, LabelBinding,
    LangTag, Lod, NonDetReason, Op, Optimality, PackInfo, PackMode, ParseError, Record, RelKind,
    Relation, Report, Role, Rung, SchemaDecl, SchemaId, Severity, ShapeError, SourceKind,
    SourceRef, Span, Status, Step, Subject, Thread, ThreadId, ThreadSchema, Uid, UidPrefix, Unit,
    UnitCore, UnitCoreBuilder, View, ViewId, FORMAT_VERSIONS_SUPPORTED, FORMAT_VERSION_DEFAULT,
    KERNEL_MAJOR, KERNEL_SCHEMA,
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
pub use smysl_graph::compact::{compact, Compacted};
pub use smysl_graph::relink::{relink, Relinked};
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

// ---- retrieve -------------------------------------------------------------
// Pure, and deliberately so: the default engine is BM25 with one transitive dependency, no
// model and no runtime, so retrieval is a bit-reproducible function of the store and the
// query. `Retriever` is the seam an impure semantic backend would sit behind.
pub use smysl_retrieve::{tokenize as retrieve_tokenize, Bm25, Hit, Query, Retriever};

// ---- semantic retrieval (feature-gated) -----------------------------------
// Impure, and outside the pure crates on purpose: it needs a model, and a model is something
// outside the format deciding the answer. Same tier as the providers, for the same reason.
#[cfg(feature = "semantic")]
pub use smysl_embed::{Hybrid, Model as EmbedModel, Semantic};

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
// `smysl import` is the only producer of `measured` units and the only unit-producing command
// that consults no model. Until 0.13 `cmd_import` reached into `smysl_ingest::import` directly
// and none of these three names was re-exported, so a consumer holding the facade could not do
// what the command does — a rule A violation that stood because nothing checked rule A.
// `Imported` is here because it is `from_csv`'s return type: without it the function is
// callable and its result unnameable.
#[cfg(feature = "ingest")]
pub use smysl_ingest::import::{from_csv, ImportOptions, Imported};
#[cfg(feature = "providers")]
pub use smysl_provider::usage::{GroupBy, Totals};
#[cfg(feature = "providers")]
pub use smysl_provider::{
    config::ProviderConfig, map::build as build_provider, Capabilities, Completion, Ledger,
    LedgerEntry, Message, Probe, Provider, ProviderConfigFile, ProviderError, ProviderId, Registry,
    Request, StreamMsg, StructuredMode, Task, TokenCount, Usage,
};

/// The terminal browser, whole, under the feature that brings it in.
///
/// Re-exported as a module rather than name by name because the browser is one capability with
/// several entry points — `run`, `App`, `render_to_string` for testing a frame without a
/// terminal — and picking a few of them would be the same rule A gap in a smaller form. The
/// `ratatui` and `crossterm` cost stays behind `--features tui`, exactly as before.
#[cfg(feature = "tui")]
pub use smysl_tui as tui;

/// The crate version, as a convenience for embedders recording provenance.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facade_reexports_the_diagnostic_registry() {
        // 51 as of 0.6.0: `SMY-W306` was deleted rather than emitted. It described a usage
        // threshold that does not exist and never did, and had sat "documented as
        // unreachable" for two releases — which is a holding pattern, not a decision. A code
        // nobody can trigger is worse than a missing one, because a reader waits for it.
        assert_eq!(Code::ALL.len(), 51);
        assert_eq!(Code::E030.severity(), Severity::Error);
    }

    /// Rule A for `smysl import`, expressed as behaviour rather than as a grep.
    ///
    /// `cargo xtask check-purity` proves the CLI names no sibling crate. It cannot prove the
    /// facade can do what the CLI does — a `cmd_import` that stopped importing would satisfy
    /// it just as well. So this does the import through facade names only, which is what rule
    /// A actually promises a library consumer.
    ///
    /// Written when the gate found `cmd_import` reaching into `smysl_ingest::import` for the
    /// CSV reader. `Imported` is named deliberately: it is `from_csv`'s return type, and
    /// re-exporting the function without it would leave the result unnameable.
    #[cfg(feature = "ingest")]
    #[test]
    fn the_import_capability_is_reachable_from_the_facade() {
        let agent = AgentId::new("tool:test").unwrap();
        let opts = ImportOptions::new("latency.csv", agent.clone(), Hlc::zero(agent));
        let out: Imported = from_csv("host,ms\nweb-1,12\nweb-2,31\n", &opts);

        assert_eq!(out.units.len(), 2, "one unit per row");
        assert!(!out.is_empty());
        // The attestation is the licence for `measured`, not a decoration — a unit without
        // one would not be permitted the status at all.
        assert_eq!(
            out.attestations.len(),
            out.units.len(),
            "every imported unit carries its `op: Imported` attestation"
        );
        assert!(
            out.units.iter().all(|u| u.status == Status::Measured),
            "import is the only producer of `measured`; that is what it is for"
        );
    }

    #[test]
    fn facade_reexports_version_constants() {
        // Every declared version must be accepted, not merely the first. The list grew to two
        // in 0.14 and an assertion about `[0]` would have kept passing while the second went
        // unchecked — which is the shape of gap this repository keeps finding.
        for v in FORMAT_VERSIONS_SUPPORTED {
            assert!(
                format_version_supported(v),
                "`{v}` is declared but not accepted"
            );
        }
        assert!(
            FORMAT_VERSIONS_SUPPORTED.contains(&FORMAT_VERSION_DEFAULT),
            "the version we write must be one we can read"
        );
        assert_eq!(kernel_major(KERNEL_SCHEMA), Some(KERNEL_MAJOR));
    }

    /// A tripwire, not a tautology. `VERSION` is `env!("CARGO_PKG_VERSION")`, so it cannot
    /// fail to be the cargo version — what the literal pins is *which release this is
    /// meant to be*, so a bump has to be an edit somebody made on purpose rather than
    /// something that drifted in with a dependency update. Update it when you bump the
    /// manifest, and the diff will say what you decided.
    #[test]
    fn the_crate_version_is_the_one_we_intend_to_ship() {
        assert_eq!(VERSION, "0.14.0");
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
