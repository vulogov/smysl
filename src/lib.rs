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
//! This is SM-P0 (scaffold). The public surface below grows phase by phase toward the
//! full contract of §12.2; each phase stabilises its library API before the matching CLI
//! subcommand is wired to it.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

// ---- core -----------------------------------------------------------------
pub use smysl_core::{
    format_version_supported, kernel_major, Code, Diagnostic, Error, ExitCode, Group, Report,
    Severity, Span, Subject, Uid, FORMAT_VERSIONS_SUPPORTED, KERNEL_MAJOR, KERNEL_SCHEMA,
};

// ---- check ----------------------------------------------------------------
pub use smysl_check::ConformanceClass;

// ---- pack -----------------------------------------------------------------
pub use smysl_pack::PackError;

// ---- thread ---------------------------------------------------------------
pub use smysl_thread::ThreadSchema;

// ---- render ---------------------------------------------------------------
pub use smysl_render::{RenderError, Target};

// ---- graph ----------------------------------------------------------------
pub use smysl_graph::MergeError;

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
}
