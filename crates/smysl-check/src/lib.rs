//! `smysl-check` - the ten-pass check pipeline (§17).
//!
//! Passes run in dependency order and MUST NOT short-circuit: a full diagnostic set is
//! more useful than a first error, and the ingest repair loop (§22.3) needs all of them
//! at once.
//!
//! Filled by SM-P4 (structural passes 2-5), SM-P5 (rule M, rule T, conformance classes).

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub use smysl_core::diag::{Code, Diagnostic, Report, Severity, Span, Subject};

/// The conformance classes of §11. A minimal downstream agent needs `Consume`; the
/// reference implementation targets `Full`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ConformanceClass {
    Read,
    Consume,
    Produce,
    Merge,
    Full,
}

impl ConformanceClass {
    pub const ALL: &'static [ConformanceClass] = &[
        ConformanceClass::Read,
        ConformanceClass::Consume,
        ConformanceClass::Produce,
        ConformanceClass::Merge,
        ConformanceClass::Full,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            ConformanceClass::Read => "C-Read",
            ConformanceClass::Consume => "C-Consume",
            ConformanceClass::Produce => "C-Produce",
            ConformanceClass::Merge => "C-Merge",
            ConformanceClass::Full => "C-Full",
        }
    }

    /// Every class builds on C-Read (§11).
    pub const fn requires_read(self) -> bool {
        true
    }
}

impl core::fmt::Display for ConformanceClass {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_conformance_classes_with_stable_names() {
        assert_eq!(ConformanceClass::ALL.len(), 5);
        assert_eq!(ConformanceClass::Read.to_string(), "C-Read");
        assert_eq!(ConformanceClass::Full.to_string(), "C-Full");
        for c in ConformanceClass::ALL {
            assert!(c.as_str().starts_with("C-"));
            assert!(c.requires_read());
        }
    }
}
