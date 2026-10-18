//! T5, n-gram surprisal (SMYSL-2.6 §3.7): interpolated Kneser–Ney (Chen, Goodman 1998,
//! TR-10-98, §3), each segment scored by a model of every other segment.
//!
//! # The model
//!
//! A segment is padded on the left with `order − 1` beginning marks and cut into n-grams of
//! the model's order; the marks are context, never predicted. The **top order counts n-grams**,
//! `c(h w)`. **Each lower order counts continuations**: `N₁₊(• g)`, the number of distinct
//! symbols `x` such that `x g` has a count above zero one order up. A lower-order n-gram is a
//! suffix of a top-order one, so the counts of every order follow from the top order's, and a
//! beginning mark is never the last symbol of any of them.
//!
//! For a context `h` of order `k`, with `a(h w)` that order's count, `A(h) = Σ_w a(h w)` and
//! `T(h) = |{w : a(h w) > 0}|`:
//!
//! ```text
//! P_k(w | h) = max(a(h w) − D_k, 0) / A(h)  +  D_k · T(h) / A(h) · P_{k−1}(w | h′)    if A(h) > 0
//!            = P_{k−1}(w | h′)                                                       if A(h) = 0
//! P_0(w)     = 1 / (V + 1)
//! ```
//!
//! where `h′` is `h` without its first symbol and `V` is the vocabulary the model was trained
//! on, so an unknown word has the `+ 1`'s share at order 0 (§3.7). One absolute discount per
//! order, `D_k = n₁ / (n₁ + 2 n₂)` from that order's counts of counts, and 0 for an order with
//! neither ones nor twos.
//!
//! # Leave one segment out, exactly
//!
//! [`Model`] keeps every order's counts, every context's `A` and `T`, and every order's `n₁` and
//! `n₂` as integers, and [`Model::remove`] subtracts a segment's top-order n-grams one at a time:
//! a count that reaches 0 takes one continuation from the order below, which may reach 0 in
//! turn. So the model without a segment is the model that was never given it — the same
//! integers, and so the same probabilities to the bit, discounts and `V` included — and
//! [`Model::add`] puts it back. `tests/kneser_ney.rs` checks that against retraining from
//! scratch.
//!
//! # The result
//!
//! `H` is the scope's surprisal, `Σ bits / Σ tokens` over every segment scored by the model of
//! the others, in bits per token. The `top:k` segments by mean surprisal (default 10; ties to
//! the earlier segment) are stored by locator, `top:<locator>`, with their tokens in
//! `measure:n`, and in rank order as the series `top`, three numbers each: the segment's
//! ordinal in the scope, its tokens and its mean. The full model's discounts are `D:<k>`.
//! `min:n` is in tokens (default 10,000), and a scope needs two segments, one to score and one
//! to score it by.

use std::collections::BTreeMap;

use smysl_core::diag::Code;

use crate::num::{self, q20};
use crate::outcome::{Num, Outcome, Refusal, Value};
use crate::recipe::{Param, Recipe};
use crate::text::entropy::{corpus, min_n};
use crate::{Input, Measure, Needs, Purity};

/// T5's `min:n`, in tokens.
pub const MIN_N: u64 = 10_000;

/// The highest order this build has.
pub const MAX_ORDER: usize = 5;

/// `params.order`'s default.
pub const ORDER: i64 = 3;

/// `params.top:k`'s default.
pub const TOP_K: i64 = 10;

/// The beginning mark: context only. A term id is below the number of types, which is below
/// `u32::MAX`.
pub const BOS: u32 = u32::MAX;

/// An n-gram or a context, its symbols first and zeros after; each order has its own map, so
/// the length is the order's.
type Key = [u32; MAX_ORDER];

fn key(xs: &[u32]) -> Key {
    let mut k = [0; MAX_ORDER];
    k[..xs.len()].copy_from_slice(xs);
    k
}

/// An interpolated Kneser–Ney model as integers, which a segment can be added to and removed
/// from exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    order: usize,
    /// Order k's counts at `k − 1`: raw at the top order, continuations below.
    grams: Vec<BTreeMap<Key, u64>>,
    /// Order k's contexts at `k − 1`: `(A, T)`.
    contexts: Vec<BTreeMap<Key, (u64, u64)>>,
    /// Order k's counts of counts at `k − 1`: how many n-grams have a count of 1, and of 2.
    ones: Vec<u64>,
    twos: Vec<u64>,
}

impl Model {
    /// An empty model of `order`, 1 to [`MAX_ORDER`].
    pub fn new(order: usize) -> Model {
        assert!((1..=MAX_ORDER).contains(&order), "order {order}");
        Model {
            order,
            grams: vec![BTreeMap::new(); order],
            contexts: vec![BTreeMap::new(); order],
            ones: vec![0; order],
            twos: vec![0; order],
        }
    }

    /// A model of `order` trained on `segments`.
    pub fn train<'s>(order: usize, segments: impl IntoIterator<Item = &'s [u32]>) -> Model {
        let mut m = Model::new(order);
        for s in segments {
            m.add(s);
        }
        m
    }

    pub fn order(&self) -> usize {
        self.order
    }

    /// Add a segment's n-grams.
    pub fn add(&mut self, segment: &[u32]) {
        self.each(segment, 1);
    }

    /// Remove a segment's n-grams, which must have been added.
    pub fn remove(&mut self, segment: &[u32]) {
        self.each(segment, -1);
    }

    fn each(&mut self, segment: &[u32], delta: i64) {
        let n = self.order;
        let mut padded = vec![BOS; n - 1];
        padded.extend_from_slice(segment);
        for g in padded.windows(n) {
            self.bump(n, g, delta);
        }
    }

    /// Order `k`'s count of `g` moves by `delta`; a count that appears or vanishes moves the
    /// continuation of `g`'s suffix one order down.
    fn bump(&mut self, k: usize, g: &[u32], delta: i64) {
        let at = k - 1;
        let gk = key(g);
        let old = self.grams[at].get(&gk).copied().unwrap_or(0);
        let new = old
            .checked_add_signed(delta)
            .expect("a segment removed that was not added");
        if new == 0 {
            self.grams[at].remove(&gk);
        } else {
            self.grams[at].insert(gk, new);
        }
        for (c, sign) in [(old, -1i64), (new, 1)] {
            match c {
                1 => self.ones[at] = self.ones[at].wrapping_add_signed(sign),
                2 => self.twos[at] = self.twos[at].wrapping_add_signed(sign),
                _ => {}
            }
        }
        let hk = key(&g[..k - 1]);
        let (a, t) = self.contexts[at].get(&hk).copied().unwrap_or((0, 0));
        let a = a.wrapping_add_signed(delta);
        let t = t + u64::from(new > 0) - u64::from(old > 0);
        if a == 0 {
            self.contexts[at].remove(&hk);
        } else {
            self.contexts[at].insert(hk, (a, t));
        }
        if (old > 0) != (new > 0) && k > 1 {
            self.bump(k - 1, &g[1..], if new > 0 { 1 } else { -1 });
        }
    }

    /// Order `k`'s discount, `n₁ / (n₁ + 2 n₂)`; 0 with neither.
    pub fn discount(&self, k: usize) -> f64 {
        let (n1, n2) = (self.ones[k - 1], self.twos[k - 1]);
        if n1 + 2 * n2 == 0 {
            0.0
        } else {
            n1 as f64 / (n1 + 2 * n2) as f64
        }
    }

    /// The words the model has seen.
    pub fn vocabulary(&self) -> u64 {
        self.contexts[0].get(&key(&[])).map_or(0, |&(_, t)| t)
    }

    /// Distinct top-order n-grams.
    pub fn ngrams(&self) -> u64 {
        self.grams[self.order - 1].len() as u64
    }

    /// `P(w | history)`, `history` the `order − 1` symbols before `w`, beginning marks included.
    pub fn prob(&self, history: &[u32], w: u32) -> f64 {
        let n = self.order;
        debug_assert_eq!(history.len(), n - 1);
        let mut p = 1.0 / (self.vocabulary() as f64 + 1.0);
        let mut g = [0u32; MAX_ORDER];
        for k in 1..=n {
            let h = &history[n - k..];
            let Some(&(a, t)) = self.contexts[k - 1].get(&key(h)) else {
                continue;
            };
            g[..k - 1].copy_from_slice(h);
            g[k - 1] = w;
            g[k..].fill(0);
            let c = self.grams[k - 1].get(&g).copied().unwrap_or(0);
            let d = self.discount(k);
            let a = a as f64;
            p = (c as f64 - d).max(0.0) / a + d * t as f64 / a * p;
        }
        p
    }

    /// A segment's surprisal, `Σ −log₂ P(wᵢ | history)`, in bits; `None` if a word has
    /// probability 0, which only an order with a zero discount gives.
    pub fn surprisal(&self, segment: &[u32]) -> Option<f64> {
        let n = self.order;
        let mut padded = vec![BOS; n - 1];
        padded.extend_from_slice(segment);
        let mut bits = Vec::with_capacity(segment.len());
        for i in 0..segment.len() {
            let p = self.prob(&padded[i..i + n - 1], segment[i]);
            if p <= 0.0 || !p.is_finite() {
                return None;
            }
            bits.push(-num::log2(p));
        }
        Some(num::sum(bits))
    }
}

/// T5.
#[derive(Debug, Clone, Copy, Default)]
pub struct T5;

fn int_param(recipe: &Recipe, name: &str) -> Option<i64> {
    match recipe.params.get(name) {
        Some(Param::Int(i)) => Some(*i),
        _ => None,
    }
}

impl Measure for T5 {
    fn id(&self) -> &'static str {
        "T5"
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
        recipe
            .params
            .entry("order".to_string())
            .or_insert(Param::Int(ORDER));
        recipe
            .params
            .entry("top:k".to_string())
            .or_insert(Param::Int(TOP_K));
    }

    fn admit(&self, recipe: &Recipe, input: &Input<'_>) -> Result<(), Refusal> {
        if !int_param(recipe, "order").is_some_and(|o| (1..=MAX_ORDER as i64).contains(&o)) {
            return Err(Refusal::new(
                Code::E489,
                format!("T5's `order` is an integer from 1 to {MAX_ORDER}"),
            ));
        }
        if int_param(recipe, "top:k").is_none_or(|k| k < 0) {
            return Err(Refusal::new(
                Code::E489,
                "T5's `top:k` is a non-negative integer",
            ));
        }
        let c = corpus(input)?;
        min_n(recipe, c, self.id())?;
        if c.segments.len() < 2 {
            return Err(Refusal::new(
                Code::W485,
                format!(
                    "T5 scores each segment by a model of the others, and `{}` has {} at `{}`",
                    recipe.scope.trim(),
                    c.segments.len(),
                    recipe.level
                ),
            ));
        }
        Ok(())
    }

    fn run(&self, recipe: &Recipe, input: &Input<'_>) -> Outcome {
        let c = match corpus(input) {
            Ok(c) => c,
            Err(r) => return Outcome::refused(r),
        };
        let (Some(order), Some(top_k)) = (int_param(recipe, "order"), int_param(recipe, "top:k"))
        else {
            return not_run("T5's parameters were not completed");
        };
        let mut model = Model::train(order as usize, c.segments.iter().map(Vec::as_slice));
        let mut value = Value::new()
            .with_n("tokens", c.tokens())
            .with_n("segments", c.segments.len() as u64)
            .with_n("types", model.vocabulary())
            .with_n("ngrams", model.ngrams());
        for k in 1..=model.order() {
            let Some(d) = q20(model.discount(k)) else {
                return not_run("a discount was not a finite number");
            };
            value = value.with_value(format!("D:{k}"), Num::Fixed(d));
        }

        let mut scored: Vec<(usize, u64, f64)> = Vec::with_capacity(c.segments.len());
        for (i, s) in c.segments.iter().enumerate() {
            model.remove(s);
            let bits = model.surprisal(s);
            model.add(s);
            let Some(bits) = bits else {
                return not_run(&format!(
                    "a word of `{}` has probability 0 under the model of the other segments, \
                     which an order with neither ones nor twos among its counts gives",
                    c.locators[i]
                ));
            };
            scored.push((i, s.len() as u64, bits));
        }
        let tokens: u64 = scored.iter().map(|s| s.1).sum();
        let Some(h) = q20(num::sum(scored.iter().map(|s| s.2)) / tokens as f64) else {
            return not_run("the surprisal was not a finite number");
        };
        value = value.with_value("H", Num::Fixed(h)).with_headline_in(
            "H",
            "n-gram surprisal, leave-one-segment-out",
            "bits/token",
        );

        // Rank by mean, highest first, ties to the earlier segment, on the stored integers so
        // that the order is the payload's.
        let mut ranked = Vec::with_capacity(scored.len());
        for &(i, n, bits) in &scored {
            let Some(m) = q20(bits / n as f64) else {
                return not_run("a segment's surprisal was not a finite number");
            };
            ranked.push((i, n, m));
        }
        ranked.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
        let mut series = Vec::new();
        for &(i, n, m) in ranked.iter().take(top_k as usize) {
            let at = &c.locators[i];
            value = value
                .with_n(format!("top:{at}"), n)
                .with_value(format!("top:{at}"), Num::Fixed(m));
            series.extend([Num::Count(i as u64), Num::Count(n), Num::Fixed(m)]);
        }
        if !series.is_empty() {
            value = value.with_series("top", series);
        }
        Outcome::Value(value)
    }
}

fn not_run(why: &str) -> Outcome {
    Outcome::NotRun {
        why: why.into(),
        code: None,
    }
}
