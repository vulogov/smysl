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
//! The **prose baseline**, in [`prose`]. Each of its hops is a model reading prose and
//! writing prose, so it is never simulated: a baseline produced by guessing what a model
//! would have dropped would measure the guess. [`prose::Summariser`] and [`prose::Judge`]
//! are therefore *ports*, this crate links no HTTP client, and the arm runs only where a
//! provider is wired in.
//!
//! Two things make the baseline honest rather than merely present, and both were wrong in
//! the first version:
//!
//! - **The baseline prose must carry its hedges in words.** A store keeps confidence in a
//!   field; prose has no fields. A renderer that dropped the status would hand the baseline
//!   a passage with no hedges at all, every one would be "lost" before the first hop, and
//!   the experiment would measure that renderer rather than summarisation.
//! - **The judge must be controlled.** The same judge reads the *unsummarised* prose first.
//!   If it already reports certainties there, the post-chain figure is the instrument's
//!   bias and not a finding, and the live test refuses to report it.
//!
//! E8 and E9 remain [`Outcome::NotRun`] with a reason rather than omitted or defaulted to
//! zero - a metric that quietly vanishes from a report is the exact failure this crate
//! exists to detect elsewhere.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub mod chain;
pub mod measure;
pub mod prose;

pub use chain::{run_smysl_arm, Arm, Budget, ChainOptions, Hop, Run};
pub use measure::{measure, Measurement, Outcome};
pub use prose::{
    claims_of, judge, run_prose_arm, Claim, EvalError, Judge, Judged, ProseRun, Summariser, Verdict,
};

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
