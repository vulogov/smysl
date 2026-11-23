//! PELT (RFC SMYSL-3 §21, TL-P5): the optimal segmentation of a series' level, as
//! `x.tel/segment`s.
//!
//! Killick, Fearnhead and Eckley's pruned exact linear time search: the segmentation minimising
//! the total cost of its segments plus `β` per change, found exactly, with candidates pruned once
//! they can never be optimal again.
//!
//! - **Cost** is L2: a segment's sum of squared deviations from its own mean,
//!   `Σx² − (Σx)²/n`, from prefix sums. The values are centred on the window's mean first, so
//!   the prefix sums stay small and the subtraction does not cancel away the answer.
//! - **Penalty** `β = penalty · σ̂²`, with `σ̂ = median|xᵢ₊₁ − xᵢ| / (0.6745·√2)`: a noise scale
//!   from first differences, which a change in level hardly moves. The usual BIC penalty is
//!   `2σ² ln n`; `ln` is a platform's libm, so here the multiplier is a parameter (default 10,
//!   BIC's value near n = 100) and is recorded in the recipe.
//! - **`min_size`** (default 2) is the shortest segment.
//!
//! Ties go to the earliest candidate, so the segmentation is one function of the samples. A
//! series without noise (`σ̂ = 0`) takes `σ̂² = 10⁻¹⁸·max(1, mean²)`: any change of level is
//! then worth its penalty.

use super::{segment, Input, Options, Recipe};
use smysl_core::{Decimal, UnitCore};

/// The detector and its version.
pub const ID: &str = "pelt/1";

/// PELT's parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    pub penalty: Decimal,
    pub min_size: usize,
}

impl Default for Params {
    fn default() -> Params {
        Params {
            penalty: Decimal::new(10, 0).expect("10"),
            min_size: 2,
        }
    }
}

impl Params {
    pub fn recipe(&self, from_ms: u64, to_ms: u64) -> Recipe {
        Recipe::new(ID)
            .with("penalty", self.penalty)
            .with("min_size", self.min_size)
            .with("from", from_ms)
            .with("to", to_ms)
    }
}

/// The median of `xs`, by total order; `None` for none.
pub fn median(xs: &[f64]) -> Option<f64> {
    if xs.is_empty() {
        return None;
    }
    let mut v = xs.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    })
}

/// The noise variance `σ̂²` from first differences.
#[cfg(test)]
fn noise_variance(xs: &[f64]) -> f64 {
    noise_variance_floored(xs, None)
}

/// [`noise_variance`] with `σ̂` at least `floor`.
fn noise_variance_floored(xs: &[f64], floor: Option<f64>) -> f64 {
    let diffs: Vec<f64> = xs.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
    let sigma = median(&diffs).unwrap_or(0.0) / (0.6745 * std::f64::consts::SQRT_2);
    let var = sigma * sigma;
    let var = if var > 0.0 {
        var
    } else {
        let mean = xs.iter().sum::<f64>() / xs.len().max(1) as f64;
        1e-18 * (mean * mean).max(1.0)
    };
    // The resolution floor only ever raises the shipped variance.
    match floor {
        Some(f) => var.max(f * f),
        None => var,
    }
}

/// The change points of `xs`: indices where a new segment begins, after the first.
pub fn change_points(xs: &[f64], p: &Params) -> Vec<usize> {
    change_points_floored(xs, p, None)
}

/// [`change_points`] under `opts` (§21.2): with `floor: resolution`, `σ̂` is at least the
/// resolution of `xs`.
pub fn change_points_with(xs: &[f64], p: &Params, opts: &Options) -> Vec<usize> {
    change_points_floored(xs, p, opts.floor_of(xs.iter().copied()))
}

fn change_points_floored(xs: &[f64], p: &Params, floor: Option<f64>) -> Vec<usize> {
    let n = xs.len();
    let min = p.min_size.max(1);
    if n < 2 * min {
        return Vec::new();
    }
    let mean = xs.iter().sum::<f64>() / n as f64;
    let (mut s1, mut s2) = (vec![0.0f64; n + 1], vec![0.0f64; n + 1]);
    for (i, x) in xs.iter().enumerate() {
        let c = x - mean;
        s1[i + 1] = s1[i] + c;
        s2[i + 1] = s2[i] + c * c;
    }
    let cost = |a: usize, b: usize| {
        let len = (b - a) as f64;
        let sum = s1[b] - s1[a];
        (s2[b] - s2[a] - sum * sum / len).max(0.0)
    };
    let beta = crate::sketch::to_f64(p.penalty) * noise_variance_floored(xs, floor);

    let mut f = vec![f64::INFINITY; n + 1];
    let mut last = vec![0usize; n + 1];
    f[0] = -beta;
    // Each candidate start, and the time it was found unable to win, if it was. With a minimum
    // segment length the pruning theorem holds only once the candidate that beat it is itself
    // admissible, `min` samples later, so a pruned candidate is kept until then.
    let mut candidates: Vec<(usize, Option<usize>)> = vec![(0, None)];
    for t in min..=n {
        let mut best = f64::INFINITY;
        let mut arg = 0usize;
        for &(s, _) in &candidates {
            if t - s < min {
                continue;
            }
            let v = f[s] + cost(s, t) + beta;
            if v < best {
                best = v;
                arg = s;
            }
        }
        f[t] = best;
        last[t] = arg;
        for (s, pruned) in candidates.iter_mut() {
            if pruned.is_none() && t - *s >= min && f[*s] + cost(*s, t) > best {
                *pruned = Some(t);
            }
        }
        candidates.retain(|(_, pruned)| pruned.is_none_or(|p| t + 1 < p + min));
        candidates.push((t, None));
    }
    let mut out = Vec::new();
    let mut t = n;
    while t > 0 {
        let s = last[t];
        if s > 0 {
            out.push(s);
        }
        t = s;
    }
    out.reverse();
    out
}

/// Run PELT over `input` in `[from_ms, to_ms)`: one segment per regime.
pub fn run(input: &Input<'_>, from_ms: u64, to_ms: u64, p: &Params) -> Vec<UnitCore> {
    run_with(input, from_ms, to_ms, p, &Options::default())
}

/// [`run`] under `opts` (§21.2), recorded in each segment's recipe.
pub fn run_with(
    input: &Input<'_>,
    from_ms: u64,
    to_ms: u64,
    p: &Params,
    opts: &Options,
) -> Vec<UnitCore> {
    let points = input.points();
    if points.is_empty() {
        return Vec::new();
    }
    let xs: Vec<f64> = points.iter().map(|x| x.1).collect();
    let recipe = opts.stamp(p.recipe(from_ms, to_ms));
    let mut bounds = vec![0usize];
    bounds.extend(change_points_with(&xs, p, opts));
    bounds.push(points.len());
    bounds
        .windows(2)
        .map(|b| {
            let seg = &xs[b[0]..b[1]];
            let level = seg.iter().sum::<f64>() / seg.len() as f64;
            let seg_to = points.get(b[1]).map_or(to_ms, |x| x.0);
            segment(input, points[b[0]].0, seg_to, &recipe, Some(level), None)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{canonical_uid, Uid};
    use smysl_series::Sample;

    const NOISE: [f64; 10] = [0.3, -0.2, 0.5, -0.4, 0.1, -0.5, 0.2, 0.0, 0.4, -0.3];

    fn levels(parts: &[(usize, f64)]) -> Vec<f64> {
        let mut out = Vec::new();
        for (n, level) in parts {
            for _ in 0..*n {
                out.push(level + NOISE[out.len() % 10]);
            }
        }
        out
    }

    /// The exact optimum by brute force over every segmentation, for checking the pruning.
    fn brute(xs: &[f64], p: &Params) -> f64 {
        let n = xs.len();
        let beta = crate::sketch::to_f64(p.penalty) * noise_variance(xs);
        let cost = |a: usize, b: usize| {
            let m = xs[a..b].iter().sum::<f64>() / (b - a) as f64;
            xs[a..b].iter().map(|x| (x - m) * (x - m)).sum::<f64>()
        };
        let mut f = vec![f64::INFINITY; n + 1];
        f[0] = -beta;
        for t in 1..=n {
            for s in 0..t {
                if t - s >= p.min_size {
                    f[t] = f[t].min(f[s] + cost(s, t) + beta);
                }
            }
        }
        f[n]
    }

    fn total(xs: &[f64], cps: &[usize], p: &Params) -> f64 {
        let beta = crate::sketch::to_f64(p.penalty) * noise_variance(xs);
        let mut b = vec![0];
        b.extend_from_slice(cps);
        b.push(xs.len());
        b.windows(2)
            .map(|w| {
                let seg = &xs[w[0]..w[1]];
                let m = seg.iter().sum::<f64>() / seg.len() as f64;
                seg.iter().map(|x| (x - m) * (x - m)).sum::<f64>()
            })
            .sum::<f64>()
            + beta * cps.len() as f64
    }

    #[test]
    fn steps_are_found_exactly() {
        let xs = levels(&[(30, 10.0), (25, 14.0), (20, 9.0)]);
        assert_eq!(change_points(&xs, &Params::default()), vec![30, 55]);
        let flat = levels(&[(60, 3.0)]);
        assert!(change_points(&flat, &Params::default()).is_empty());
    }

    #[test]
    fn the_pruned_search_is_the_exact_optimum() {
        let p = Params::default();
        for parts in [
            vec![(12, 1.0), (7, 3.0), (9, 1.5)],
            vec![(5, 0.0), (5, 0.6), (5, 0.0), (5, 0.6)],
            vec![(20, 100.0)],
        ] {
            let xs = levels(&parts);
            let cps = change_points(&xs, &p);
            let got = total(&xs, &cps, &p);
            // F(n) starts at −β and pays β per segment: the cost plus β per change.
            let want = brute(&xs, &p);
            assert!(
                (got - want).abs() <= 1e-9 * want.abs().max(1.0),
                "{parts:?}: {got} vs {want}"
            );
        }
    }

    #[test]
    fn segments_and_their_uids_are_one_function_of_the_samples() {
        let xs = levels(&[(30, 10.0), (30, 14.0)]);
        let s: Vec<Sample> = xs
            .iter()
            .enumerate()
            .map(|(i, v)| {
                Sample::new(1_000 * i as u64, None, Decimal::from_f64(*v).unwrap(), 0).unwrap()
            })
            .collect();
        let input = Input {
            series: Uid::from_bytes([1; 32]),
            scheme: "prom",
            key: "lat",
            instrument: Uid::from_bytes([2; 32]),
            samples: &s,
        };
        let a = run(&input, 0, 60_000, &Params::default());
        assert_eq!(a.len(), 2);
        let b = run(&input, 0, 60_000, &Params::default());
        assert_eq!(
            a.iter().map(canonical_uid).collect::<Vec<_>>(),
            b.iter().map(canonical_uid).collect::<Vec<_>>()
        );
        assert!(run(
            &Input {
                samples: &[],
                ..input
            },
            0,
            1,
            &Params::default()
        )
        .is_empty());
    }

    #[test]
    fn the_pruned_search_is_exact_on_many_series() {
        let mut seed = 11u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            seed >> 33
        };
        for case in 0..200 {
            let n = 6 + (next() % 40) as usize;
            let mut level = 0.0;
            let xs: Vec<f64> = (0..n)
                .map(|_| {
                    if next() % 8 == 0 {
                        level = (next() % 50) as f64 / 10.0;
                    }
                    level + (next() % 100) as f64 / 100.0
                })
                .collect();
            for min_size in [1, 2, 3] {
                let p = Params {
                    min_size,
                    penalty: Decimal::new((1 + next() % 20) as i128, 0).unwrap(),
                };
                let got = total(&xs, &change_points(&xs, &p), &p);
                let want = brute(&xs, &p);
                assert!(
                    (got - want).abs() <= 1e-9 * want.abs().max(1.0),
                    "case {case}, min {min_size}: {got} vs {want}"
                );
            }
        }
    }
}
