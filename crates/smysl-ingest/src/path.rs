//! Path selection (D-9, §22.1).
//!
//! ```text
//! fn choose_path(caps, op, size, cfg) -> Path:
//!     if let Some(p) = cfg.provider_override { return p }
//!     match op {
//!         RelationExtraction | GistRewrite | ThreadRefine  => JsonAst,
//!         ContentIngest if caps.structured == None         => Surface,
//!         ContentIngest if size > SMALL_OUTPUT_THRESHOLD   => Surface,
//!         _                                                => JsonAst,
//!     }
//! ```
//!
//! **Surface is the default for bulk content**, and the reason is asymmetric failure: a
//! malformed unit in surface text is recoverable - the parser reports a span and the repair
//! loop fixes that span - while a truncated JSON object is not. The closing brace is load
//! bearing for the whole document.
//!
//! Small structured operations go the other way. Extracting relations returns a handful of
//! triples, where a schema the provider enforces is worth more than a format that degrades
//! gracefully.

use smysl_provider::{Capabilities, Task};

use crate::IngestPath;

/// Above this many bytes of expected output, bulk content takes the surface path (§22.1).
///
/// Chosen from where truncation starts to matter rather than from a measurement: below a
/// few kilobytes a truncated response is rare and cheap to retry; above it, a single lost
/// brace costs the whole batch.
pub const SMALL_OUTPUT_THRESHOLD: usize = 4096;

/// Why a path was chosen. Reported so `ingest --dry-run` can explain itself rather than
/// leaving a caller to infer the rule from the outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reason {
    /// The caller asked for it.
    Override,
    /// A small structured operation.
    StructuredOperation,
    /// The provider enforces no schema, so JSON would buy nothing and cost robustness.
    NoEnforcement,
    /// Too much output to risk truncation.
    BulkContent,
    /// The default for a small content ingest against an enforcing provider.
    Default,
}

impl Reason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Reason::Override => "caller override",
            Reason::StructuredOperation => "a structured operation",
            Reason::NoEnforcement => "the provider enforces no schema",
            Reason::BulkContent => "output too large to risk truncation",
            Reason::Default => "default for small enforced ingest",
        }
    }
}

/// The chosen path and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Choice {
    pub path: IngestPath,
    pub reason: Reason,
}

/// Choose the ingest path (D-9).
pub fn choose(
    caps: &Capabilities,
    task: Task,
    expected_output: usize,
    override_path: Option<IngestPath>,
) -> Choice {
    if let Some(path) = override_path {
        return Choice {
            path,
            reason: Reason::Override,
        };
    }

    match task {
        // These return a handful of triples or one rewritten string. A schema the provider
        // enforces is worth more here than a format that degrades gracefully.
        Task::RelationExtraction | Task::GistRewrite | Task::ThreadRefine => Choice {
            path: IngestPath::JsonAst,
            reason: Reason::StructuredOperation,
        },
        Task::ContentIngest | Task::Attest => {
            // Asking for JSON from a provider that will not enforce it buys nothing and
            // costs the surface path's recoverability.
            if !caps.structured.is_enforced() {
                return Choice {
                    path: IngestPath::Surface,
                    reason: Reason::NoEnforcement,
                };
            }
            if expected_output > SMALL_OUTPUT_THRESHOLD {
                return Choice {
                    path: IngestPath::Surface,
                    reason: Reason::BulkContent,
                };
            }
            Choice {
                path: IngestPath::JsonAst,
                reason: Reason::Default,
            }
        }
        // A task this table does not know about gets the recoverable path, because the
        // failure mode of guessing wrong is milder in that direction.
        _ => Choice {
            path: IngestPath::Surface,
            reason: Reason::Default,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_provider::StructuredMode;

    fn caps(mode: StructuredMode) -> Capabilities {
        let mut c = Capabilities::default();
        c.structured = mode;
        c
    }

    fn enforcing() -> Capabilities {
        caps(StructuredMode::JsonSchema)
    }

    #[test]
    fn an_override_wins_over_everything() {
        for task in Task::ALL {
            let c = choose(&enforcing(), *task, 1_000_000, Some(IngestPath::JsonAst));
            assert_eq!(c.path, IngestPath::JsonAst);
            assert_eq!(c.reason, Reason::Override);
        }
        assert_eq!(
            choose(
                &caps(StructuredMode::None),
                Task::ContentIngest,
                0,
                Some(IngestPath::JsonAst)
            )
            .path,
            IngestPath::JsonAst
        );
    }

    /// A handful of triples, where an enforced schema is worth more than graceful
    /// degradation.
    #[test]
    fn small_structured_operations_take_the_json_path() {
        for task in [
            Task::RelationExtraction,
            Task::GistRewrite,
            Task::ThreadRefine,
        ] {
            let c = choose(&enforcing(), task, 0, None);
            assert_eq!(c.path, IngestPath::JsonAst, "{task}");
            assert_eq!(c.reason, Reason::StructuredOperation);
        }
    }

    /// ...and they take it even against a provider that will not enforce it, because the
    /// *shape* is what the caller needs; D-9's table is unconditional for these.
    #[test]
    fn structured_operations_do_not_consult_the_capabilities() {
        let c = choose(
            &caps(StructuredMode::None),
            Task::RelationExtraction,
            0,
            None,
        );
        assert_eq!(c.path, IngestPath::JsonAst);
    }

    /// Asking for JSON from a provider that will not enforce it buys nothing and costs the
    /// surface path's recoverability.
    #[test]
    fn content_ingest_falls_back_to_surface_without_enforcement() {
        for mode in [StructuredMode::None, StructuredMode::JsonMode] {
            let c = choose(&caps(mode), Task::ContentIngest, 0, None);
            assert_eq!(c.path, IngestPath::Surface, "{mode}");
            assert_eq!(c.reason, Reason::NoEnforcement);
        }
    }

    /// This is the DeepSeek case, and the one D-9 leaves to measured E9.
    #[test]
    fn json_mode_alone_is_not_enough_to_choose_json() {
        assert!(!StructuredMode::JsonMode.is_enforced());
        assert_eq!(
            choose(
                &caps(StructuredMode::JsonMode),
                Task::ContentIngest,
                10,
                None
            )
            .path,
            IngestPath::Surface
        );
    }

    /// A malformed unit is recoverable; a truncated JSON object is not.
    #[test]
    fn bulk_content_takes_the_surface_path() {
        let c = choose(
            &enforcing(),
            Task::ContentIngest,
            SMALL_OUTPUT_THRESHOLD + 1,
            None,
        );
        assert_eq!(c.path, IngestPath::Surface);
        assert_eq!(c.reason, Reason::BulkContent);
    }

    #[test]
    fn a_small_enforced_content_ingest_takes_the_json_path() {
        let c = choose(
            &enforcing(),
            Task::ContentIngest,
            SMALL_OUTPUT_THRESHOLD,
            None,
        );
        assert_eq!(c.path, IngestPath::JsonAst);
        assert_eq!(c.reason, Reason::Default);
    }

    #[test]
    fn the_threshold_is_a_boundary_not_a_range() {
        let at = choose(
            &enforcing(),
            Task::ContentIngest,
            SMALL_OUTPUT_THRESHOLD,
            None,
        );
        let over = choose(
            &enforcing(),
            Task::ContentIngest,
            SMALL_OUTPUT_THRESHOLD + 1,
            None,
        );
        assert_ne!(at.path, over.path);
    }

    #[test]
    fn every_enforced_mode_is_treated_alike() {
        for mode in [
            StructuredMode::JsonSchema,
            StructuredMode::ToolForce,
            StructuredMode::Grammar,
        ] {
            assert_eq!(
                choose(&caps(mode), Task::ContentIngest, 0, None).path,
                IngestPath::JsonAst,
                "{mode}"
            );
        }
    }

    #[test]
    fn every_reason_can_say_what_it_is() {
        for r in [
            Reason::Override,
            Reason::StructuredOperation,
            Reason::NoEnforcement,
            Reason::BulkContent,
            Reason::Default,
        ] {
            assert!(!r.as_str().is_empty());
        }
    }
}
