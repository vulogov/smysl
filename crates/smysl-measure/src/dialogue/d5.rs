//! D5, lexical drift (SMYSL-2.6 §3.10): how far the words of a conversation move from one
//! window to the next.
//!
//! The text is read at `level = window:<n>` (`SMY-E489` at any other level), and each pair of
//! consecutive windows within one text gives the Jensen–Shannon divergence of their term
//! counts, in bits ([`crate::text::divergence::jsd`], over the two windows' own vocabulary,
//! which gives the same value as over the whole one: a term neither window has adds nothing).
//! `jsd:mean` is the headline, beside `jsd:max` and the series of every pair in reading order.
//!
//! **Only full windows are compared.** A text's last window is whatever was left over, and a
//! window of three terms against one of two hundred is far apart for the reason that it is
//! short; it is counted (`n.partial`) and left out. A window does not run across two texts
//! (M2), so neither does a pair. `min:n` is in full windows, two by default — one pair.
//!
//! This is the lexical form, pure. The embedding form compares window centroids, is
//! `ModelDependent`, and waits for TX-P10.

use std::collections::BTreeMap;

use smysl_core::diag::Code;

use crate::num::{self, q20};
use crate::recipe::Level;
use crate::text::divergence::jsd;
use crate::text::entropy::corpus;
use crate::{Input, Measure, Needs, Num, Outcome, Purity, Recipe, Refusal, Value};

/// D5.
#[derive(Debug, Clone, Copy, Default)]
pub struct D5;

/// D5's `min:n`, in full windows.
pub const MIN_N: u64 = 2;

fn width(recipe: &Recipe) -> Result<usize, Refusal> {
    match recipe.level {
        Level::Window(n) if n > 0 => Ok(n as usize),
        other => Err(Refusal::new(
            Code::E489,
            format!("D5 compares windows, and the recipe's level is `{other}`; give window:<n>"),
        )),
    }
}

/// The two windows' counts over their union, in term-id order.
fn counts(a: &[u32], b: &[u32]) -> (Vec<u64>, Vec<u64>) {
    let mut t: BTreeMap<u32, (u64, u64)> = BTreeMap::new();
    for x in a {
        t.entry(*x).or_default().0 += 1;
    }
    for x in b {
        t.entry(*x).or_default().1 += 1;
    }
    t.into_values().unzip()
}

impl Measure for D5 {
    fn id(&self) -> &'static str {
        "D5"
    }

    fn version(&self) -> u16 {
        1
    }

    fn purity(&self) -> Purity {
        Purity::Pure
    }

    fn needs(&self) -> Needs {
        Needs::TEXT
    }

    fn complete(&self, recipe: &mut Recipe) {
        recipe.min_n.get_or_insert(MIN_N);
    }

    fn admit(&self, recipe: &Recipe, input: &Input<'_>) -> Result<(), Refusal> {
        width(recipe)?;
        corpus(input).map(|_| ())
    }

    fn run(&self, recipe: &Recipe, input: &Input<'_>) -> Outcome {
        let run = || -> Result<Value, Refusal> {
            let w = width(recipe)?;
            let c = corpus(input)?;
            let mut bounds = c.starts.clone();
            bounds.push(c.segments.len());
            let (mut full, mut partial) = (0u64, 0u64);
            let mut series: Vec<f64> = Vec::new();
            for pair in bounds.windows(2) {
                let text = &c.segments[pair[0]..pair[1]];
                let kept: Vec<&Vec<u32>> = text.iter().filter(|s| s.len() == w).collect();
                full += kept.len() as u64;
                partial += (text.len() - kept.len()) as u64;
                for ab in kept.windows(2) {
                    let (p, q) = counts(ab[0], ab[1]);
                    series.extend(jsd(&p, &q));
                }
            }
            let want = recipe.min_n.unwrap_or(0);
            if full < want || series.is_empty() {
                return Err(Refusal::new(
                    Code::W485,
                    format!(
                        "D5 needs at least {} full windows of {w} terms in one text and `{}` \
                         has {full}",
                        want.max(2),
                        recipe.scope.trim()
                    ),
                ));
            }
            let unwritable = || Refusal::new(Code::E489, "a divergence is past a payload number");
            let mean = q20(num::sum(series.iter().copied()) / series.len() as f64)
                .ok_or_else(unwritable)?;
            let max = q20(series.iter().copied().fold(0.0, f64::max)).ok_or_else(unwritable)?;
            let points = series
                .iter()
                .map(|x| q20(*x).map(Num::Fixed).ok_or_else(unwritable))
                .collect::<Result<Vec<Num>, Refusal>>()?;
            Ok(Value::new()
                .with_n("tokens", c.tokens())
                .with_n("windows", full)
                .with_n("partial", partial)
                .with_n("pairs", points.len() as u64)
                .with_value("jsd:mean", Num::Fixed(mean))
                .with_value("jsd:max", Num::Fixed(max))
                .with_series("jsd", points)
                .with_headline_in("jsd:mean", "mean window drift", "bits"))
        };
        match run() {
            Ok(v) => Outcome::Value(v),
            Err(r) => Outcome::refused(r),
        }
    }
}
