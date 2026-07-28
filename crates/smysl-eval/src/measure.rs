//! Turning a [`Run`](crate::Run) into the numbers of §28.
//!
//! Every measurement carries whether it was *observed* or *not run*. A harness that
//! reported 0.0 for a metric it never measured would be indistinguishable from one that
//! measured it and found nothing, and the two mean opposite things.

use std::collections::BTreeSet;

use smysl_check::{check, CheckOptions};
use smysl_core::{Code, Severity, Uid};
use smysl_graph::Store;

use crate::chain::Run;
use crate::Metric;

/// One metric's result.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Outcome {
    /// Measured, with the value in the metric's own units.
    Observed(f64),
    /// Not measured, and why. A missing provider is the usual reason.
    NotRun(&'static str),
}

impl Outcome {
    pub fn value(&self) -> Option<f64> {
        match self {
            Outcome::Observed(v) => Some(*v),
            Outcome::NotRun(_) => None,
        }
    }
}

/// A metric, its outcome, and what the number is of.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Measurement {
    pub metric: Metric,
    pub outcome: Outcome,
    /// Human-readable units, so a report cannot present a ratio as a count.
    pub unit: &'static str,
}

impl Measurement {
    fn observed(metric: Metric, value: f64, unit: &'static str) -> Measurement {
        Measurement {
            metric,
            outcome: Outcome::Observed(value),
            unit,
        }
    }

    fn not_run(metric: Metric, why: &'static str) -> Measurement {
        Measurement {
            metric,
            outcome: Outcome::NotRun(why),
            unit: "",
        }
    }
}

fn ratio(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        return 1.0;
    }
    part as f64 / whole as f64
}

/// **E1** - tokens handed to the receiving system at the final hop, against the whole input.
///
/// Below 1.0 means the chain costs less to pass on than to pass whole. It is a cost rather
/// than a price because no model was called to produce it.
fn token_cost(run: &Run) -> Measurement {
    let final_tokens = run
        .hops
        .last()
        .map(|h| h.tokens)
        .unwrap_or(run.initial_tokens);
    Measurement::observed(
        Metric::TokenCost,
        final_tokens as f64 / run.initial_tokens.max(1) as f64,
        "fraction of full-detail input tokens",
    )
}

/// **E2** - claims present at the start that are still present at the end.
fn claim_survival(run: &Run) -> Measurement {
    let survivors = run.survivors();
    let kept = run.initial.iter().filter(|u| survivors.contains(u)).count();
    Measurement::observed(
        Metric::ClaimSurvival,
        ratio(kept, run.initial.len()),
        "fraction of initial units",
    )
}

/// **E3** - epistemic corruption, structural. Rules M and T over what survived.
///
/// Expected to be zero, and measured anyway: §28 is explicit that a guarantee which never
/// binds indicates an unrepresentative corpus rather than a strong guarantee.
fn epistemic_corruption(store: &Store, run: &Run) -> Measurement {
    let report = check(
        store,
        CheckOptions::default().only([smysl_check::Pass::Epistemics, smysl_check::Pass::Trust]),
    );
    let survivors = run.survivors();
    let violations = report
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .filter(|d| d.uid().is_some_and(|u| survivors.contains(u)))
        .count();
    Measurement::observed(
        Metric::EpistemicCorruption,
        violations as f64,
        "rule M/T violations among survivors",
    )
}

/// **E4** - rebuttal survival under budget pressure, structural (rule R).
///
/// The fraction of surviving rebutted units whose rebuttal survived with them, at the
/// tightest point of the chain. Taking the worst hop rather than the last is deliberate:
/// rule R holding at the end after an intermediate hop dropped a rebuttal would be rule R
/// not holding.
fn rebuttal_survival(run: &Run) -> Measurement {
    let worst = run
        .hops
        .iter()
        .filter(|h| h.rebuttals_possible > 0)
        .map(|h| ratio(h.rebuttals_honoured, h.rebuttals_possible))
        .fold(f64::INFINITY, f64::min);
    let value = if worst.is_finite() { worst } else { 1.0 };
    Measurement::observed(
        Metric::RebuttalSurvival,
        value,
        "fraction of rebutted survivors keeping their rebuttal",
    )
}

/// **E5** - gist coverage: input gist tokens still reachable at the end.
fn gist_coverage(store: &Store, run: &Run) -> Measurement {
    let gist_tokens = |set: &BTreeSet<Uid>| -> u32 {
        set.iter()
            .filter_map(|u| store.get(u))
            .map(|u| smysl_core::tokens(&u.core.gist))
            .sum()
    };
    Measurement::observed(
        Metric::GistCoverage,
        f64::from(gist_tokens(run.survivors())) / f64::from(gist_tokens(&run.initial).max(1)),
        "fraction of input gist tokens",
    )
}

/// **E6** - warrant density: warranted survivors per survivor carrying grounds. Gates D-10.
fn warrant_density(store: &Store, run: &Run) -> Measurement {
    let survivors = run.survivors();
    let mut grounded = 0usize;
    let mut warranted = 0usize;
    for uid in survivors {
        let Some(unit) = store.get(uid) else { continue };
        if unit.core.grounds.is_empty() {
            continue;
        }
        grounded += 1;
        if store
            .relations_of_kind(&smysl_core::RelKind::Warrant)
            .iter()
            .any(|r| &r.to == uid)
        {
            warranted += 1;
        }
    }
    Measurement::observed(
        Metric::WarrantDensity,
        ratio(warranted, grounded),
        "fraction of grounded survivors carrying a warrant",
    )
}

/// **E7** - round-trip fidelity over the surviving set: 1.0 when records survive
/// surface -> records and CBOR -> records unchanged.
fn round_trip_fidelity(store: &Store, run: &Run) -> Measurement {
    let survivors = run.survivors();
    let mut checked = 0usize;
    let mut intact = 0usize;
    for uid in survivors {
        let Some(unit) = store.get(uid) else { continue };
        checked += 1;
        let bytes = smysl_core::to_cbor(&smysl_core::Record::Unit(unit.core.clone()));
        if let Ok((smysl_core::Record::Unit(back), n)) = smysl_core::from_cbor(&bytes) {
            if n == bytes.len() && back == unit.core {
                intact += 1;
            }
        }
    }
    Measurement::observed(
        Metric::RoundTripFidelity,
        ratio(intact, checked),
        "fraction of survivors surviving a CBOR round trip",
    )
}

/// Measure every metric that a model-free run can support.
///
/// E8 and E9 are reported as [`Outcome::NotRun`] rather than omitted: the list of metrics
/// is the list of claims, and a claim that quietly disappears from a report is the failure
/// mode this whole crate exists to measure in someone else's pipeline.
pub fn measure(store: &Store, run: &Run) -> Vec<Measurement> {
    vec![
        token_cost(run),
        claim_survival(run),
        epistemic_corruption(store, run),
        rebuttal_survival(run),
        gist_coverage(store, run),
        warrant_density(store, run),
        round_trip_fidelity(store, run),
        Measurement::not_run(
            Metric::IngestOverhead,
            "needs a provider: ingest is a model call",
        ),
        Measurement::not_run(
            Metric::ProviderConformance,
            "needs a provider: measured by the live ingest gate",
        ),
    ]
}

/// Every code E3 would report, for a caller that wants the diagnostics rather than a count.
pub fn corruption_codes() -> [Code; 2] {
    [Code::E030, Code::E033]
}
