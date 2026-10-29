//! T4, Zipf and Heaps (SMYSL-2.6 §3.7).
//!
//! # Zipf
//!
//! A discrete power law `P(f) = f^{−α} / ζ(α, f_min)` fitted to the type **frequencies**
//! `cᵥ ≥ f_min` by maximum likelihood (Clauset, Shalizi, Newman 2009, §3). The log-likelihood
//! over `m` frequencies is `−m ln ζ(α, f_min) − α Σ ln cᵥ`, maximised by golden-section search
//! on `[1.01, 6]` for 64 fixed iterations — fixed so that the search is the same sequence of
//! evaluations on every machine, not a tolerance that stops one iteration apart. `f_min` is the
//! candidate whose fit has the smallest Kolmogorov–Smirnov distance to its tail, candidates in
//! increasing order and the first kept on a tie. The rank–frequency exponent is
//! `s = 1/(α − 1)`.
//!
//! **A candidate needs a tail** of at least `params.tail:min` types (default 50). Without a
//! floor the search finds the largest frequencies — "the", "and", "of" — and fits a line through
//! three points, whose distance to its own fit is small because there is nothing to be far from.
//! Clauset et al. warn about exactly this; the floor is the recipe's, so it is recorded.
//!
//! **The KS distance is exact over the integers.** Both CDFs are step functions of an integer
//! frequency, the empirical one constant between two observed values `uⱼ < uⱼ₊₁`, the model's
//! increasing. The supremum over `[uⱼ, uⱼ₊₁)` is therefore at one of its ends, and evaluating at
//! `uⱼ` and `uⱼ₊₁ − 1` is the supremum over every integer at twice the observed values' cost.
//!
//! **Hurwitz ζ** by Euler–Maclaurin: ten terms summed directly, the integral of the rest, half
//! the first omitted term, and four Bernoulli corrections (B₂ to B₈), every power through
//! `libm`. At `q + 10 ≥ 11` the first neglected correction, B₁₀'s, is at most 3·10⁻¹³ relative
//! over `[1.01, 6]` — largest near α = 2, where it is 2.7·10⁻¹³ — which is five orders below a
//! fixed-point quantum.
//!
//! # Heaps
//!
//! `V(n) = K nᵝ` by ordinary least squares of `ln V` on `ln n` at 64 geometric checkpoints in
//! reading order. **The checkpoints run from `⌈√N⌉` to `N`, not from 1** — an as-built
//! decision. A text's first few hundred tokens are nearly all new types, because `V(n) ≤ n`
//! binds before Heaps' law does: for §5.1's own synthetic `V(n) = 10·n^0.6` the bound binds
//! below `n = 10^2.5`, and checkpoints from 1 put 42% of the fit there, at slope 1, and return
//! β ≈ 0.8 for a text built to have 0.6. Starting at `√N` is scale-free and leaves the first
//! decade out of a text of a million tokens and less of a short one.
//!
//! # Intervals: a bootstrap and a jackknife, because the two fits have different observations
//!
//! **α resamples types**, an as-built deviation from §3.7's segment bootstrap. Clauset et al.,
//! the paper §3.7 cites for the fit, estimate the uncertainty of `f_min` and α by a
//! nonparametric bootstrap over the *measurements* — here, the scope's type frequencies —
//! drawn uniformly with replacement and refitted, `f_min` included. The segment bootstrap answers a
//! different question and answers it with a bias: a segment drawn twice adds its tokens
//! twice and **no new type**, so a replicate's rare types are fewer and more frequent than
//! the text's and its α is high. Measured on §5.1's own synthetic text at α = 2.5 — frequencies
//! drawn exactly from the law — the segment interval was [2.502, 2.831] around α̂ = 2.484, and
//! did not contain the estimate it was an interval for; resampling types, it does.
//!
//! **β leaves segments out** (version 2, OQ-89), since a vocabulary curve is a fact about
//! reading order and segments are what a reader reads in order. Version 1 resampled them with
//! replacement, as §3.7 first said, and had the same bias — a duplicated segment adds no type,
//! so β\* ran low, and over five Bibles the percentile interval excluded the estimate at every
//! length up to the whole book. No resampling *with replacement* avoids that, because a
//! duplicate is its mechanism. The grouped jackknife ([`boot::jackknife`]) duplicates nothing:
//! each of twenty replicates drops one interleaved group and refits the rest in order, and
//! `β̂ ± z·se` covered `E[β̂]` in 90–97.5% of seeds across every length and both texts OQ-89's
//! simulation drew, where the percentile interval ranged from 0% to 100%.
//!
//! A replicate with no candidate tail is not counted, and the interval's `b` says how many were.

use crate::boot;
use crate::num::{self, q20};
use crate::outcome::{Num, Outcome, Refusal, Value};
use crate::recipe::{Param, Recipe};
use crate::rng::Rng;
use crate::text::entropy::{corpus, min_n};
use crate::text::Corpus;
use crate::{Input, Measure, Needs, Purity};

/// T4's `min:n`, in tokens: T1's, since the two are read from the same counts.
pub const MIN_N: u64 = 10_000;

/// The smallest tail a candidate `f_min` may leave, in types.
pub const TAIL_MIN: i64 = 50;

/// The search interval and its iterations (§3.7).
pub const ALPHA_LO: f64 = 1.01;
pub const ALPHA_HI: f64 = 6.0;
pub const ITERATIONS: u32 = 64;

/// Heaps checkpoints.
pub const CHECKPOINTS: u32 = 64;

/// T4.
#[derive(Debug, Clone, Copy, Default)]
pub struct T4;

/// `ζ(s, q) = Σ_{k≥0} (q + k)^{−s}`, for `s > 1` and `q ≥ 1`.
pub fn hurwitz_zeta(s: f64, q: f64) -> f64 {
    const N: u32 = 10;
    // B₂ⱼ / (2j)!, for j = 1..4.
    const B: [f64; 4] = [1.0 / 12.0, -1.0 / 720.0, 1.0 / 30_240.0, -1.0 / 1_209_600.0];
    let direct = num::sum((0..N).map(|k| num::powf(q + f64::from(k), -s)));
    let a = q + f64::from(N);
    let a_s = num::powf(a, -s);
    let mut tail = a * a_s / (s - 1.0) + a_s / 2.0;
    // s(s+1)…(s+2j−2) · a^{−s−2j+1}, built up one j at a time.
    let mut fac = s * a_s / a;
    let mut j = 1.0;
    for b in B {
        tail += b * fac;
        fac *= (s + 2.0 * j - 1.0) * (s + 2.0 * j) / (a * a);
        j += 1.0;
    }
    direct + tail
}

/// A power-law fit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub alpha: f64,
    pub f_min: u64,
    pub ks: f64,
    /// Types at or above `f_min`.
    pub tail: u64,
}

/// The distinct frequencies of a tail and how many types have each, ascending.
fn histogram(sorted: &[u64]) -> Vec<(u64, u64)> {
    let mut h: Vec<(u64, u64)> = Vec::new();
    for &c in sorted {
        match h.last_mut() {
            Some((v, n)) if *v == c => *n += 1,
            _ => h.push((c, 1)),
        }
    }
    h
}

/// α for one `f_min`: golden-section search of the log-likelihood.
fn alpha_for(f_min: u64, m: f64, sum_ln: f64) -> f64 {
    let q = f_min as f64;
    let ll = |a: f64| -m * num::ln(hurwitz_zeta(a, q)) - a * sum_ln;
    // (√5 − 1) / 2, written out: a square root here would be the one float in the search a
    // port could round differently, and this is its correctly rounded value.
    const INV_PHI: f64 = 0.618_033_988_749_894_9;
    let (mut lo, mut hi) = (ALPHA_LO, ALPHA_HI);
    let mut c = hi - INV_PHI * (hi - lo);
    let mut d = lo + INV_PHI * (hi - lo);
    let (mut fc, mut fd) = (ll(c), ll(d));
    for _ in 0..ITERATIONS {
        if fc > fd {
            hi = d;
            d = c;
            fd = fc;
            c = hi - INV_PHI * (hi - lo);
            fc = ll(c);
        } else {
            lo = c;
            c = d;
            fc = fd;
            d = lo + INV_PHI * (hi - lo);
            fd = ll(d);
        }
    }
    (lo + hi) / 2.0
}

/// The KS distance between a tail's histogram and the law at `alpha`, exact over the integers.
fn ks(h: &[(u64, u64)], m: f64, alpha: f64, f_min: u64) -> f64 {
    let z0 = hurwitz_zeta(alpha, f_min as f64);
    // P(X ≤ x) = 1 − ζ(α, x+1) / ζ(α, f_min).
    let model = |x: u64| 1.0 - hurwitz_zeta(alpha, (x + 1) as f64) / z0;
    let mut seen = 0u64;
    let mut d: f64 = 0.0;
    for (j, &(u, n)) in h.iter().enumerate() {
        seen += n;
        let emp = seen as f64 / m;
        d = d.max((emp - model(u)).abs());
        if let Some(&(next, _)) = h.get(j + 1) {
            if next - 1 > u {
                d = d.max((emp - model(next - 1)).abs());
            }
        }
    }
    d
}

/// The power-law fit of a count table: every candidate `f_min` leaving at least `tail_min`
/// types, the one with the smallest KS distance kept. `None` when no candidate has that tail.
pub fn fit(counts: &[u64], tail_min: u64) -> Option<Fit> {
    let mut sorted: Vec<u64> = counts.iter().copied().filter(|&c| c > 0).collect();
    sorted.sort_unstable();
    let h = histogram(&sorted);
    // Suffix sums: types and Σ ln c at or above each distinct value.
    let mut types_above = vec![0u64; h.len() + 1];
    let mut ln_above = vec![0.0f64; h.len() + 1];
    for j in (0..h.len()).rev() {
        let (u, n) = h[j];
        types_above[j] = types_above[j + 1] + n;
        ln_above[j] = ln_above[j + 1] + n as f64 * num::ln(u as f64);
    }
    let mut best: Option<Fit> = None;
    for j in 0..h.len() {
        let m = types_above[j];
        if m < tail_min.max(2) {
            break;
        }
        let f_min = h[j].0;
        let mf = m as f64;
        let alpha = alpha_for(f_min, mf, ln_above[j]);
        let d = ks(&h[j..], mf, alpha, f_min);
        if best.is_none_or(|b| d < b.ks) {
            best = Some(Fit {
                alpha,
                f_min,
                ks: d,
                tail: m,
            });
        }
    }
    best
}

/// The 64 geometric checkpoints on `[⌈√N⌉, N]`, increasing and distinct.
pub fn checkpoints(n: u64) -> Vec<u64> {
    if n == 0 {
        return Vec::new();
    }
    let lo = n.isqrt() + u64::from(n.isqrt() * n.isqrt() < n);
    let (l0, l1) = (num::ln(lo as f64), num::ln(n as f64));
    // 63 by the formula and the 64th is `N` itself, exactly: `exp(ln N)` may floor to `N − 1`.
    let mut out: Vec<u64> = (0..CHECKPOINTS - 1)
        .map(|k| {
            let t = f64::from(k) / f64::from(CHECKPOINTS - 1);
            let x = num::exp(l0 + t * (l1 - l0)).floor() as u64;
            x.clamp(lo, n)
        })
        .collect();
    out.push(n);
    out.dedup();
    out
}

/// Heaps' `(β, K)` over a token stream in reading order; `None` with fewer than two
/// checkpoints.
pub fn heaps<'a>(
    stream: impl Iterator<Item = &'a u32>,
    n: u64,
    types: usize,
) -> Option<(f64, f64)> {
    let at = checkpoints(n);
    if at.len() < 2 {
        return None;
    }
    let mut seen = vec![false; types];
    let (mut v, mut i, mut k) = (0u64, 0u64, 0usize);
    let mut points: Vec<(f64, f64)> = Vec::with_capacity(at.len());
    for &t in stream {
        if !std::mem::replace(&mut seen[t as usize], true) {
            v += 1;
        }
        i += 1;
        if k < at.len() && i == at[k] {
            points.push((num::ln(i as f64), num::ln(v as f64)));
            k += 1;
        }
    }
    let m = points.len() as f64;
    let mx = num::sum(points.iter().map(|p| p.0)) / m;
    let my = num::sum(points.iter().map(|p| p.1)) / m;
    let sxy = num::sum(points.iter().map(|p| (p.0 - mx) * (p.1 - my)));
    let sxx = num::sum(points.iter().map(|p| (p.0 - mx) * (p.0 - mx)));
    if sxx <= 0.0 {
        return None;
    }
    let beta = sxy / sxx;
    Some((beta, num::exp(my - beta * mx)))
}

fn tail_min(recipe: &Recipe) -> u64 {
    match recipe.params.get("tail:min") {
        Some(Param::Int(n)) if *n > 0 => *n as u64,
        _ => TAIL_MIN as u64,
    }
}

impl Measure for T4 {
    fn id(&self) -> &'static str {
        "T4"
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
        recipe
            .params
            .entry("tail:min".to_string())
            .or_insert(Param::Int(TAIL_MIN));
    }

    fn admit(&self, recipe: &Recipe, input: &Input<'_>) -> Result<(), Refusal> {
        if !matches!(recipe.params.get("tail:min"), Some(Param::Int(n)) if *n > 0) {
            return Err(Refusal::new(
                smysl_core::diag::Code::E489,
                "T4's `tail:min` is a positive integer",
            ));
        }
        min_n(recipe, corpus(input)?, self.id())
    }

    fn run(&self, recipe: &Recipe, input: &Input<'_>) -> Outcome {
        let c = match corpus(input) {
            Ok(c) => c,
            Err(r) => return Outcome::refused(r),
        };
        let tmin = tail_min(recipe);
        let per = c.segment_counts();
        let all: Vec<usize> = (0..per.len()).collect();
        let mut counts = Vec::new();
        Corpus::counts_into(&per, &all, &mut counts, c.types.len());
        let n = c.tokens();
        let Some(z) = fit(&counts, tmin) else {
            return Outcome::refused(Refusal::new(
                smysl_core::diag::Code::W485,
                format!(
                    "T4 needs at least {tmin} types sharing a tail, and `{}` has {} types",
                    recipe.scope.trim(),
                    c.types.len()
                ),
            ));
        };
        let Some((beta, k)) = heaps(c.segments.iter().flatten(), n, c.types.len()) else {
            return not_finite();
        };
        let fixed = |v: f64| q20(v).map(Num::Fixed);
        let (Some(alpha), Some(s), Some(ks), Some(beta_q), Some(k_q)) = (
            fixed(z.alpha),
            fixed(1.0 / (z.alpha - 1.0)),
            fixed(z.ks),
            fixed(beta),
            fixed(k),
        ) else {
            return not_finite();
        };
        let mut value = Value::new()
            .with_n("tokens", n)
            .with_n("types", c.types.len() as u64)
            .with_n("tail", z.tail)
            .with_n("segments", per.len() as u64)
            .with_value("alpha", alpha)
            .with_value("s", s)
            .with_value("ks", ks)
            .with_value("f:min", Num::Count(z.f_min))
            .with_value("heaps:beta", beta_q)
            .with_value("heaps:K", k_q)
            .with_headline_in("alpha", "Zipf exponent", "");

        if per.len() >= 2 && recipe.boot_b > 0 {
            let Ok(hash) = recipe.hash() else {
                return not_finite();
            };
            let mut rng = Rng::new(&hash, recipe.seed);
            let observed: Vec<u64> = counts.iter().copied().filter(|&n| n > 0).collect();
            let mut drawn = Vec::new();
            let mut resampled = Vec::with_capacity(observed.len());
            let mut alphas = Vec::new();
            // The generator draws types only: version 2's β takes nothing from it.
            for _ in 0..recipe.boot_b {
                boot::draw(&mut rng, observed.len(), &mut drawn);
                resampled.clear();
                resampled.extend(drawn.iter().map(|&i| observed[i]));
                if let Some(f) = fit(&resampled, tmin) {
                    alphas.push(f.alpha);
                }
            }
            if let Some(ci) = boot::interval(&mut alphas) {
                value = value.with_ci("alpha", ci);
            }
            if per.len() >= boot::GROUPS {
                let mut betas = Vec::new();
                for g in 0..boot::GROUPS {
                    boot::keep(per.len(), g, &mut drawn);
                    let len: u64 = drawn.iter().map(|&i| c.segments[i].len() as u64).sum();
                    let stream = drawn.iter().flat_map(|&i| c.segments[i].iter());
                    if let Some((b, _)) = heaps(stream, len, c.types.len()) {
                        betas.push(b);
                    }
                }
                if let Some(ci) = boot::jackknife(beta, &betas) {
                    value = value.with_ci("heaps:beta", ci);
                }
            }
        }
        Outcome::Value(value)
    }
}

fn not_finite() -> Outcome {
    Outcome::NotRun {
        why: "a fit was not a finite number".into(),
        code: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ζ(2, 1) = π²/6 and ζ(4, 1) = π⁴/90, and the shift rule ζ(s, q) = q^{−s} + ζ(s, q+1).
    #[test]
    fn hurwitz_zeta_matches_its_closed_forms() {
        let pi = std::f64::consts::PI;
        // The error is B₁₀'s term, 2.7·10⁻¹³ at s = 2: the module note's bound, asserted.
        assert!((hurwitz_zeta(2.0, 1.0) - pi * pi / 6.0).abs() < 3e-13);
        assert!((hurwitz_zeta(4.0, 1.0) - pi * pi * pi * pi / 90.0).abs() < 3e-13);
        for (s, q) in [(1.01, 1.0), (1.5, 3.0), (2.5, 7.0), (6.0, 1.0)] {
            let lhs = hurwitz_zeta(s, q);
            let rhs = num::powf(q, -s) + hurwitz_zeta(s, q + 1.0);
            assert!(((lhs - rhs) / lhs).abs() < 6e-13, "ζ({s}, {q})");
        }
        // ζ(1.01, 1) ≈ 100.5779, the pole's neighbourhood.
        assert!((hurwitz_zeta(1.01, 1.0) - 100.577_943_338_497).abs() < 1e-9);
    }

    #[test]
    fn checkpoints_are_geometric_from_the_square_root() {
        let c = checkpoints(1_000_000);
        assert_eq!(c.first(), Some(&1000));
        assert_eq!(c.last(), Some(&1_000_000));
        assert!(c.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(c.len(), 64);
        assert_eq!(checkpoints(10), vec![4, 5, 6, 7, 8, 9, 10]);
        assert_eq!(checkpoints(1), vec![1]);
    }
}
