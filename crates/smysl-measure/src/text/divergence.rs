//! T7, divergences and collocations (SMYSL-2.6 §3.7).
//!
//! # Jensen–Shannon, between every two texts in scope
//!
//! `JSD(P, Q) = H(M) − ½H(P) − ½H(Q)` with `M = ½(P + Q)`, in bits, in [0, 1], over the texts'
//! term counts, and `√JSD`, a metric, beside it. A scope's texts are read under one analyzer
//! and stream by construction — a corpus has one — so the RFC's `SMY-E489` for two is the
//! corpus loader's refusal of a scope whose texts choose two chains. The pairs are the scope's
//! texts in alias order, keyed `jsd:<a>|<b>` and `jsd:sqrt:<a>|<b>` as T6 keys its pairs.
//!
//! It is computed in the equivalent form
//!
//! ```text
//! JSD = ½ (1/N_P) Σ c_P log₂(2 c_P N_Q / (c_P N_Q + c_Q N_P))
//!     + ½ (1/N_Q) Σ c_Q log₂(2 c_Q N_P / (c_P N_Q + c_Q N_P))
//! ```
//!
//! over the union of the supports in term-id order, with every ratio a ratio of integer
//! products. So **two texts with the same distribution give 0 exactly** (each ratio is `x/x`,
//! and `log₂ 1` is 0) and **two with disjoint supports give 1 exactly** (each ratio is 2, each
//! sum is the text's own token count times 1, and the division is `N/N`), where the textbook
//! form would subtract three rounded entropies and land near either.
//!
//! # Collocations: G² and PMI over adjacent pairs
//!
//! The pairs are adjacent terms within a segment, never across two. For a pair `x y`, with `N`
//! the pairs in scope, `R` the pairs whose first term is `x` and `C` those whose second is `y`,
//! the 2×2 table is `O₁₁ = c(x y)`, `O₁₂ = R − O₁₁`, `O₂₁ = C − O₁₁`, `O₂₂ = N − R − C + O₁₁`,
//! and its expectations `Eᵢⱼ` are the products of its margins over `N`.
//!
//! - **G²** (Dunning 1993) is `2 Σ Oᵢⱼ ln(Oᵢⱼ / Eᵢⱼ)`, `0 · ln 0 = 0`. It is large for a pair
//!   seen more often than chance and for one seen less, and a collocation is the first, so the
//!   ranking keeps the pairs with `O₁₁ > E₁₁`.
//! - **PMI** is `log₂(c(x y) · N / (R · C))`, over the pairs with `c(x y) ≥ params.min:pair`
//!   (default 5): below that a pair seen once beside two rare words has the largest PMI there is.
//!
//! The top `params.top:k` pairs by each (default 20, highest first, ties by term order) are
//! stored by their terms, `g2:<x> <y>` and `pmi:<x> <y>`, with the pair's count in `measure:n`
//! as `pair:<x> <y>`. `min:n` is in tokens and is 10,000, T1's.

use std::collections::BTreeMap;

use smysl_core::diag::Code;

use crate::num::{self, q20};
use crate::outcome::{Num, Outcome, Refusal, Value};
use crate::recipe::{Param, Recipe};
use crate::text::entropy::{corpus, min_n};
use crate::text::Corpus;
use crate::{Input, Measure, Needs, Purity};

/// T7's `min:n`, in tokens.
pub const MIN_N: u64 = 10_000;

/// `params.min:pair`'s default.
pub const MIN_PAIR: i64 = 5;

/// `params.top:k`'s default.
pub const TOP_K: i64 = 20;

/// The Jensen–Shannon divergence of two count tables over one vocabulary, in bits; `None` if
/// either is empty.
pub fn jsd(p: &[u64], q: &[u64]) -> Option<f64> {
    let np: u64 = p.iter().sum();
    let nq: u64 = q.iter().sum();
    if np == 0 || nq == 0 {
        return None;
    }
    let (npf, nqf) = (np as f64, nq as f64);
    let (mut sp, mut sq) = (Vec::new(), Vec::new());
    for (&cp, &cq) in p.iter().zip(q) {
        if cp == 0 && cq == 0 {
            continue;
        }
        let (a, b) = (cp as f64 * nqf, cq as f64 * npf);
        let m = a + b;
        if cp > 0 {
            sp.push(cp as f64 * num::log2(2.0 * a / m));
        }
        if cq > 0 {
            sq.push(cq as f64 * num::log2(2.0 * b / m));
        }
    }
    Some((num::sum(sp) / npf + num::sum(sq) / nqf) / 2.0)
}

/// Dunning's G² of a 2×2 table, `2 Σ O ln(O/E)` with `0 · ln 0 = 0`; `None` for an empty
/// table.
pub fn g2(o11: u64, o12: u64, o21: u64, o22: u64) -> Option<f64> {
    let n = o11 + o12 + o21 + o22;
    if n == 0 {
        return None;
    }
    let (r1, r2) = (o11 + o12, o21 + o22);
    let (c1, c2) = (o11 + o21, o12 + o22);
    let nf = n as f64;
    let cell = |o: u64, r: u64, c: u64| {
        if o == 0 {
            0.0
        } else {
            o as f64 * num::ln(o as f64 * nf / (r as f64 * c as f64))
        }
    };
    let terms = [
        cell(o11, r1, c1),
        cell(o12, r1, c2),
        cell(o21, r2, c1),
        cell(o22, r2, c2),
    ];
    Some(2.0 * num::sum(terms))
}

/// The adjacent pairs of a corpus: their counts, the counts of each first and each second
/// term, and how many there are.
pub struct Pairs {
    pub counts: BTreeMap<(u32, u32), u64>,
    pub first: Vec<u64>,
    pub second: Vec<u64>,
    pub n: u64,
}

impl Pairs {
    pub fn of(c: &Corpus) -> Pairs {
        let v = c.types.len();
        let mut p = Pairs {
            counts: BTreeMap::new(),
            first: vec![0; v],
            second: vec![0; v],
            n: 0,
        };
        for s in &c.segments {
            for w in s.windows(2) {
                *p.counts.entry((w[0], w[1])).or_default() += 1;
                p.first[w[0] as usize] += 1;
                p.second[w[1] as usize] += 1;
                p.n += 1;
            }
        }
        p
    }

    /// The 2×2 table of `(x, y)`.
    pub fn table(&self, x: u32, y: u32) -> (u64, u64, u64, u64) {
        let o11 = self.counts.get(&(x, y)).copied().unwrap_or(0);
        let r = self.first[x as usize];
        let c = self.second[y as usize];
        (o11, r - o11, c - o11, self.n + o11 - r - c)
    }
}

/// T7.
#[derive(Debug, Clone, Copy, Default)]
pub struct T7;

fn int_param(recipe: &Recipe, name: &str) -> Option<i64> {
    match recipe.params.get(name) {
        Some(Param::Int(i)) => Some(*i),
        _ => None,
    }
}

impl Measure for T7 {
    fn id(&self) -> &'static str {
        "T7"
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
            .entry("min:pair".to_string())
            .or_insert(Param::Int(MIN_PAIR));
        recipe
            .params
            .entry("top:k".to_string())
            .or_insert(Param::Int(TOP_K));
    }

    fn admit(&self, recipe: &Recipe, input: &Input<'_>) -> Result<(), Refusal> {
        if int_param(recipe, "min:pair").is_none_or(|k| k < 1) {
            return Err(Refusal::new(
                Code::E489,
                "T7's `min:pair` is a positive integer",
            ));
        }
        if int_param(recipe, "top:k").is_none_or(|k| k < 0) {
            return Err(Refusal::new(
                Code::E489,
                "T7's `top:k` is a non-negative integer",
            ));
        }
        min_n(recipe, corpus(input)?, self.id())
    }

    fn run(&self, recipe: &Recipe, input: &Input<'_>) -> Outcome {
        let c = match corpus(input) {
            Ok(c) => c,
            Err(r) => return Outcome::refused(r),
        };
        let (Some(min_pair), Some(top_k)) =
            (int_param(recipe, "min:pair"), int_param(recipe, "top:k"))
        else {
            return not_run("T7's parameters were not completed");
        };
        let (min_pair, top_k) = (min_pair as u64, top_k as usize);
        let v = c.types.len();

        // Each text's counts, in alias order.
        let per: Vec<Vec<u64>> = (0..c.starts.len())
            .map(|k| {
                let (from, to) = c.text_range(k);
                let mut counts = vec![0u64; v];
                for s in &c.segments[from..to] {
                    for &t in s {
                        counts[t as usize] += 1;
                    }
                }
                counts
            })
            .collect();
        let pairs = Pairs::of(c);
        let mut value = Value::new()
            .with_n("tokens", c.tokens())
            .with_n("types", v as u64)
            .with_n("texts", per.len() as u64)
            .with_n("pairs", pairs.n)
            .with_n("pairs:distinct", pairs.counts.len() as u64);

        for i in 0..per.len() {
            for j in i + 1..per.len() {
                let Some(d) = jsd(&per[i], &per[j]) else {
                    continue;
                };
                // A sum of rounded terms can stray a unit past [0, 1]; the root of a negative
                // is not a number, and the divergence is not below 0.
                let d = d.clamp(0.0, 1.0);
                let (Some(dq), Some(rq)) = (q20(d), q20(d.sqrt())) else {
                    return not_run("a divergence was not a finite number");
                };
                let (a, b) = (&c.texts[i], &c.texts[j]);
                value = value
                    .with_value(format!("jsd:{a}|{b}"), Num::Fixed(dq))
                    .with_value(format!("jsd:sqrt:{a}|{b}"), Num::Fixed(rq));
            }
        }

        // Collocations, each ranked on its stored integer so the order is the payload's.
        let mut by_g2: Vec<(i64, (u32, u32))> = Vec::new();
        let mut by_pmi: Vec<(i64, (u32, u32))> = Vec::new();
        let nf = pairs.n as f64;
        for (&(x, y), &o11) in &pairs.counts {
            let (r, col) = (pairs.first[x as usize], pairs.second[y as usize]);
            // O₁₁ > E₁₁ = R·C/N, in integers.
            let attracted = u128::from(o11) * u128::from(pairs.n) > u128::from(r) * u128::from(col);
            if attracted {
                let (a, b, cc, d) = pairs.table(x, y);
                let Some(g) = g2(a, b, cc, d).and_then(q20) else {
                    return not_run("a G² was not a finite number");
                };
                by_g2.push((g, (x, y)));
            }
            if o11 >= min_pair {
                let Some(p) = q20(num::log2(o11 as f64 * nf / (r as f64 * col as f64))) else {
                    return not_run("a PMI was not a finite number");
                };
                by_pmi.push((p, (x, y)));
            }
        }
        for (prefix, mut ranked) in [("g2", by_g2), ("pmi", by_pmi)] {
            ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            for &(q, (x, y)) in ranked.iter().take(top_k) {
                let name = format!("{} {}", c.types[x as usize], c.types[y as usize]);
                value = value
                    .with_value(format!("{prefix}:{name}"), Num::Fixed(q))
                    .with_n(format!("pair:{name}"), pairs.counts[&(x, y)]);
            }
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
