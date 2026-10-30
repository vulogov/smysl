//! T1, unigram entropy (SMYSL-2.6 §3.7).
//!
//! `Ĥ = −Σ p̂ᵥ log₂ p̂ᵥ` over the scope's types in term-id order, and Miller–Madow's correction
//! beside it, `Ĥ_MM = Ĥ + (V̂ − 1) / (2N ln 2)` with `V̂` the types observed (Miller 1955). The
//! plug-in estimator is low by about the correction on a sample of `N` tokens; both are stored,
//! and the gist names the corrected one, because it is the one that estimates the source.
//!
//! A grouped jackknife interval over segments is stored for each (version 2, OQ-89): version 1's
//! segment bootstrap ran low in every replicate, because a segment drawn twice adds no new
//! type, and its percentile interval excluded the estimate on every one of five Bibles at every
//! length up to the whole book. The interval is the estimator's sampling spread at this length;
//! it is **not** an interval for the source's entropy, which the plug-in and Miller–Madow
//! estimates both undershoot by more than that spread on a short text (§3.7's OQ-89 note has the
//! figures). `min:n` is 10,000 tokens (§3.7).

use crate::boot;
use crate::num::{self, q20};
use crate::outcome::{Num, Outcome, Refusal, Value};
use crate::recipe::Recipe;
use crate::text::Corpus;
use crate::{Input, Measure, Needs, Purity};

/// T1's `min:n`, in tokens.
pub const MIN_N: u64 = 10_000;

/// T1.
#[derive(Debug, Clone, Copy, Default)]
pub struct T1;

/// Plug-in and Miller–Madow entropy of a count table, in bits; `None` for no tokens.
pub fn entropy(counts: &[u64]) -> Option<(f64, f64)> {
    let n: u64 = counts.iter().sum();
    if n == 0 {
        return None;
    }
    let nf = n as f64;
    let h = num::sum(counts.iter().filter(|&&c| c > 0).map(|&c| {
        let p = c as f64 / nf;
        -(p * num::log2(p))
    }));
    let v = counts.iter().filter(|&&c| c > 0).count() as f64;
    let mm = h + (v - 1.0) / (2.0 * nf * num::ln(2.0));
    Some((h, mm))
}

pub(crate) fn corpus<'a>(input: &Input<'a>) -> Result<&'a Corpus, Refusal> {
    input.text.ok_or_else(|| {
        Refusal::new(
            smysl_core::diag::Code::E489,
            "a text measure was given no text to read",
        )
    })
}

pub(crate) fn min_n(recipe: &Recipe, corpus: &Corpus, id: &str) -> Result<(), Refusal> {
    let want = recipe.min_n.unwrap_or(0);
    let have = corpus.tokens();
    if have < want {
        return Err(Refusal::new(
            smysl_core::diag::Code::W485,
            format!(
                "{id} needs at least {want} tokens and `{}` has {have}",
                recipe.scope.trim()
            ),
        ));
    }
    Ok(())
}

impl Measure for T1 {
    fn id(&self) -> &'static str {
        "T1"
    }

    fn version(&self) -> u16 {
        2
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
        min_n(recipe, corpus(input)?, self.id())
    }

    fn run(&self, recipe: &Recipe, input: &Input<'_>) -> Outcome {
        let c = match corpus(input) {
            Ok(c) => c,
            Err(r) => return Outcome::refused(r),
        };
        let per = c.segment_counts();
        let all: Vec<usize> = (0..per.len()).collect();
        let mut counts = Vec::new();
        Corpus::counts_into(&per, &all, &mut counts, c.types.len());
        let Some((h, mm)) = entropy(&counts) else {
            return not_finite();
        };
        let (Some(hq), Some(mmq)) = (q20(h), q20(mm)) else {
            return not_finite();
        };
        let mut value = Value::new()
            .with_n("tokens", c.tokens())
            .with_n("types", counts.iter().filter(|&&n| n > 0).count() as u64)
            .with_n("segments", per.len() as u64)
            .with_value("H", Num::Fixed(hq))
            .with_value("H:mm", Num::Fixed(mmq))
            .with_headline_in("H:mm", "unigram entropy, Miller–Madow", "bits/token");

        // `boot:b = 0` still asks for no interval; any other value asks for the jackknife, whose
        // replicates are the groups and not `b`.
        if per.len() >= boot::GROUPS && recipe.boot_b > 0 {
            let mut kept = Vec::new();
            let (mut hs, mut mms) = (Vec::new(), Vec::new());
            for g in 0..boot::GROUPS {
                boot::keep(per.len(), g, &mut kept);
                Corpus::counts_into(&per, &kept, &mut counts, c.types.len());
                if let Some((h, mm)) = entropy(&counts) {
                    hs.push(h);
                    mms.push(mm);
                }
            }
            if let Some(ci) = boot::jackknife(h, &hs) {
                value = value.with_ci("H", ci);
            }
            if let Some(ci) = boot::jackknife(mm, &mms) {
                value = value.with_ci("H:mm", ci);
            }
        }
        Outcome::Value(value)
    }
}

fn not_finite() -> Outcome {
    Outcome::NotRun {
        why: "the entropy was not a finite number".into(),
        code: None,
    }
}
