//! A6, concentration (SMYSL-2.6 §3.8): the Gini coefficient of a scope's salience.
//!
//! `G = 2 Σᵢ i·s₍ᵢ₎ / (n Σ s) − (n + 1)/n` over the scope's scores sorted ascending, i from 1:
//! 0 when every unit is as salient as every other, approaching 1 when one unit holds it all.
//!
//! **Exact.** `smysl_graph::salience` quantises every score to 1/1024 in `[0, 1]`, so a score
//! is an integer `qᵢ = 1024 sᵢ` and `G = (2 Σ i·qᵢ − (n + 1) Σ q) / (n Σ q)` is a ratio of
//! integers, rounded once into the payload. No float is summed, so the order of a sum cannot
//! move the last bit, and equal scores give 0 exactly rather than a rounding residue.
//!
//! The scores are salience over the whole store under the default request, read for the units
//! in scope: a unit's salience is its place in the store's graph, and the scope says which
//! units are asked about. A scope whose scores are all zero has no concentration to report and
//! is not run.

use smysl_graph::salience::{salience, SalienceRequest};

use crate::graph;
use crate::{Input, Measure, Needs, Num, Outcome, Purity, Recipe, Refusal, Value};

/// A6.
#[derive(Debug, Clone, Copy, Default)]
pub struct A6;

/// The Gini coefficient of integer scores, as a payload number; `None` when they sum to 0.
pub fn gini(scores: &mut [u64]) -> Option<i64> {
    scores.sort_unstable();
    let n = scores.len() as i128;
    let total: i128 = scores.iter().map(|&q| i128::from(q)).sum();
    if n == 0 || total == 0 {
        return None;
    }
    let weighted: i128 = scores
        .iter()
        .enumerate()
        .map(|(i, &q)| (i as i128 + 1) * i128::from(q))
        .sum();
    graph::ratio_q20(2 * weighted - (n + 1) * total, n * total)
}

impl Measure for A6 {
    fn id(&self) -> &'static str {
        "A6"
    }

    fn version(&self) -> u16 {
        1
    }

    fn purity(&self) -> Purity {
        Purity::Pure
    }

    fn needs(&self) -> Needs {
        Needs::UNITS.union(Needs::SALIENCE)
    }

    fn complete(&self, recipe: &mut Recipe) {
        graph::complete(recipe);
    }

    fn admit(&self, recipe: &Recipe, input: &Input<'_>) -> Result<(), Refusal> {
        graph::admitted(recipe, input).map(|_| ())
    }

    fn run(&self, recipe: &Recipe, input: &Input<'_>) -> Outcome {
        let uids = match graph::admitted(recipe, input) {
            Ok(u) => u,
            Err(r) => return Outcome::refused(r),
        };
        let report = salience(input.store, &SalienceRequest::default());
        let mut q: Vec<u64> = Vec::with_capacity(uids.len());
        for u in &uids {
            let s = f64::from(report.get(u)) * 1024.0;
            if s.fract() != 0.0 || s < 0.0 {
                return Outcome::NotRun {
                    why: format!("a salience score of {u} is not on the 1/1024 grid"),
                    code: None,
                };
            }
            q.push(s as u64);
        }
        let Some(g) = gini(&mut q) else {
            return Outcome::NotRun {
                why: format!(
                    "every unit of `{}` has salience 0, so there is nothing to be concentrated",
                    recipe.scope.trim()
                ),
                code: None,
            };
        };
        Outcome::Value(
            Value::new()
                .with_n("units", uids.len() as u64)
                .with_value("gini", Num::Fixed(g))
                .with_headline_in("gini", "salience Gini", ""),
        )
    }
}
