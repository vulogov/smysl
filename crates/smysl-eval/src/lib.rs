//! `smysl-eval` - the evaluation harness (§28). Not published.
//!
//! Runs a 5-hop chain over fixtures F1-F5 in two arms: a prose baseline where each hop
//! reads prose and emits prose, and a smysl arm where each hop reads a pack and emits
//! units.
//!
//! E3 and E4 should be near-trivially won - they are structural guarantees, not empirical
//! outcomes. They are still measured: a guarantee that never binds indicates an
//! unrepresentative corpus.
//!
//! # What is landed
//!
//! The **smysl arm** and every metric a model-free run can support: E1-E7. The arm is a
//! chain of packs, so the whole run is a pure function of the store and the budget.
//!
//! The **prose baseline is not landed, and is not simulated.** Each of its hops is a model
//! reading prose and emitting prose; a baseline produced by guessing what a model would
//! have dropped would be a measurement of the guess. E8 and E9 are likewise reported as
//! [`Outcome::NotRun`] with the reason, rather than omitted or defaulted to zero - a
//! metric that quietly vanishes from a report is the exact failure this crate exists to
//! detect elsewhere.
//!
//! So E1 and E2 currently describe what the smysl arm costs and keeps. They become a
//! *comparison* only when the baseline runs.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub mod chain;
pub mod measure;

pub use chain::{run_smysl_arm, Arm, Budget, ChainOptions, Hop, Run};
pub use measure::{measure, Measurement, Outcome};

/// The metrics of §28, with the hypothesis each is measured against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Metric {
    /// E1 - token cost per hop.
    TokenCost,
    /// E2 - claim survival across five hops.
    ClaimSurvival,
    /// E3 - epistemic corruption. Structural: rules M and T.
    EpistemicCorruption,
    /// E4 - rebuttal survival under budget pressure. Structural: rule R.
    RebuttalSurvival,
    /// E5 - gist coverage.
    GistCoverage,
    /// E6 - warrant density. Gates D-10.
    WarrantDensity,
    /// E7 - round-trip fidelity.
    RoundTripFidelity,
    /// E8 - ingest overhead amortisation.
    IngestOverhead,
    /// E9 - provider conformance. Gates D-9.
    ProviderConformance,
}

impl Metric {
    pub const ALL: &'static [Metric] = &[
        Metric::TokenCost,
        Metric::ClaimSurvival,
        Metric::EpistemicCorruption,
        Metric::RebuttalSurvival,
        Metric::GistCoverage,
        Metric::WarrantDensity,
        Metric::RoundTripFidelity,
        Metric::IngestOverhead,
        Metric::ProviderConformance,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Metric::TokenCost => "E1",
            Metric::ClaimSurvival => "E2",
            Metric::EpistemicCorruption => "E3",
            Metric::RebuttalSurvival => "E4",
            Metric::GistCoverage => "E5",
            Metric::WarrantDensity => "E6",
            Metric::RoundTripFidelity => "E7",
            Metric::IngestOverhead => "E8",
            Metric::ProviderConformance => "E9",
        }
    }

    /// Whether the metric is a structural guarantee rather than an empirical hope.
    pub const fn is_structural(self) -> bool {
        matches!(self, Metric::EpistemicCorruption | Metric::RebuttalSurvival)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nine_metrics_numbered_e1_through_e9() {
        assert_eq!(Metric::ALL.len(), 9);
        let ids: Vec<&str> = Metric::ALL.iter().map(|m| m.id()).collect();
        assert_eq!(ids, ["E1", "E2", "E3", "E4", "E5", "E6", "E7", "E8", "E9"]);
    }

    #[test]
    fn only_e3_and_e4_are_structural() {
        let structural: Vec<&str> = Metric::ALL
            .iter()
            .filter(|m| m.is_structural())
            .map(|m| m.id())
            .collect();
        assert_eq!(structural, ["E3", "E4"]);
    }
}
