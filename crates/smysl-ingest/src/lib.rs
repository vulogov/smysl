//! `smysl-ingest` - the ingest boundary (§9, §22).
//!
//! Three rules meet here. Rule S: model output is staged, checked, and confirmed, never
//! written straight to the store. Rule I: ingest always makes progress - an unrepairable
//! span degrades to an opaque `prose` unit rather than failing the run. Rule T: a model
//! asserting from its own priors is capped at `inferred`, however confidently it phrases
//! the claim.
//!
//! Filled by SM-P14.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

/// Default repair attempts before an unrepairable span degrades to opaque `prose`
/// (rule I, `SMY-W304`).
pub const DEFAULT_REPAIR_ATTEMPTS: u8 = 2;

/// Which ingest path a request takes (D-9). Surface is the default for bulk content: a
/// malformed unit is recoverable, a truncated JSON object is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum IngestPath {
    Surface,
    JsonAst,
}

impl IngestPath {
    pub const fn as_str(self) -> &'static str {
        match self {
            IngestPath::Surface => "surface",
            IngestPath::JsonAst => "json-ast",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repair_budget_is_two_attempts() {
        assert_eq!(DEFAULT_REPAIR_ATTEMPTS, 2);
    }

    #[test]
    fn ingest_paths_have_stable_names() {
        assert_eq!(IngestPath::Surface.as_str(), "surface");
        assert_eq!(IngestPath::JsonAst.as_str(), "json-ast");
    }
}
