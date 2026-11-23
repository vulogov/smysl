//! CUSUM (RFC SMYSL-3 §21, TL-P5): change points in a series' level, as `x.tel/segment`s.
//!
//! Page's two-sided cumulative sum, over standardised values:
//!
//! ```text
//! z_t  = (x_t − μ) / σ
//! S⁺_t = max(0, S⁺_{t−1} + z_t − k)      S⁻_t = max(0, S⁻_{t−1} − z_t − k)
//! ```
//!
//! An alarm is `S⁺ > h` or `S⁻ > h`; the change point is the sample where the sum that alarmed
//! last left zero, which is where the drift began rather than where it became certain. `μ` and
//! `σ` are the mean and standard deviation of the first `baseline` samples of each regime, so the
//! detector restarts after each change, against the new level.
//!
//! - `k` (default 0.5) is the slack in units of `σ`: half the shift it is tuned to find.
//! - `h` (default 5) is the alarm threshold, in units of `σ`.
//! - `baseline` (default 10) is how many samples estimate a regime's `μ` and `σ`.
//!
//! A regime whose baseline has no spread (`σ = 0`) takes `σ = 10⁻⁹·max(1, |μ|)`: any departure
//! from a constant is then a change, which is what a constant series means.
//!
//! Each regime is a segment `[first sample, next regime's first sample)`, the last one ending at
//! the window's end, with `level` the mean of its samples. A window with no change is one
//! segment. The arithmetic is `+ − × ÷` and one `sqrt` per regime, in sample order.

use super::{segment, Input, Options, Recipe};
use smysl_core::{Decimal, UnitCore};

/// The detector and its version.
pub const ID: &str = "cusum/1";

/// CUSUM's parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    pub k: Decimal,
    pub h: Decimal,
    pub baseline: usize,
}

impl Default for Params {
    fn default() -> Params {
        Params {
            k: Decimal::new(5, -1).expect("0.5"),
            h: Decimal::new(5, 0).expect("5"),
            baseline: 10,
        }
    }
}

impl Params {
    pub fn recipe(&self, from_ms: u64, to_ms: u64) -> Recipe {
        Recipe::new(ID)
            .with("k", self.k)
            .with("h", self.h)
            .with("baseline", self.baseline)
            .with("from", from_ms)
            .with("to", to_ms)
    }
}

/// The change points of `points`: indices where a new regime begins, after the first.
pub fn change_points(points: &[(u64, f64)], p: &Params) -> Vec<usize> {
    change_points_floored(points, p, None)
}

/// [`change_points`] under `opts` (§21.2): with `floor: resolution`, each regime's `σ` is at
/// least the resolution of `points`.
pub fn change_points_with(points: &[(u64, f64)], p: &Params, opts: &Options) -> Vec<usize> {
    change_points_floored(points, p, opts.floor_of(points.iter().map(|x| x.1)))
}

fn change_points_floored(points: &[(u64, f64)], p: &Params, floor: Option<f64>) -> Vec<usize> {
    let k = crate::sketch::to_f64(p.k);
    let h = crate::sketch::to_f64(p.h);
    let base = p.baseline.max(2);
    let mut out = Vec::new();
    let mut start = 0usize;
    'regime: while start + base <= points.len() {
        let xs = &points[start..start + base];
        let mean = xs.iter().map(|x| x.1).sum::<f64>() / base as f64;
        let var = xs.iter().map(|x| (x.1 - mean) * (x.1 - mean)).sum::<f64>() / (base - 1) as f64;
        let mut sigma = var.sqrt();
        if sigma == 0.0 {
            sigma = 1e-9 * mean.abs().max(1.0);
        }
        // The resolution floor only ever raises the shipped scale.
        if let Some(f) = floor {
            sigma = sigma.max(f);
        }
        let (mut up, mut down) = (0.0f64, 0.0f64);
        let (mut up_from, mut down_from) = (start, start);
        for (i, x) in points.iter().enumerate().skip(start + base) {
            let z = (x.1 - mean) / sigma;
            if up == 0.0 {
                up_from = i;
            }
            if down == 0.0 {
                down_from = i;
            }
            up = (up + z - k).max(0.0);
            down = (down - z - k).max(0.0);
            if up > h || down > h {
                let cp = if up > h { up_from } else { down_from };
                out.push(cp);
                start = cp;
                continue 'regime;
            }
        }
        break;
    }
    out
}

/// Run CUSUM over `input` in `[from_ms, to_ms)`: one segment per regime.
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
    let recipe = opts.stamp(p.recipe(from_ms, to_ms));
    let mut bounds = vec![0usize];
    bounds.extend(change_points_with(&points, p, opts));
    bounds.push(points.len());
    bounds
        .windows(2)
        .map(|b| {
            let xs = &points[b[0]..b[1]];
            let level = xs.iter().map(|x| x.1).sum::<f64>() / xs.len() as f64;
            let seg_to = points.get(b[1]).map_or(to_ms, |x| x.0);
            segment(input, xs[0].0, seg_to, &recipe, Some(level), None)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{canonical_uid, Uid};
    use smysl_series::Sample;

    fn samples(values: &[f64]) -> Vec<Sample> {
        values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                Sample::new(1_000 * i as u64, None, Decimal::from_f64(*v).unwrap(), 0).unwrap()
            })
            .collect()
    }

    fn input(s: &[Sample]) -> Input<'_> {
        Input {
            series: Uid::from_bytes([1; 32]),
            scheme: "prom",
            key: "lat",
            instrument: Uid::from_bytes([2; 32]),
            samples: s,
        }
    }

    /// A level of 10 with ±0.5 noise, then 14 from sample 40.
    fn step() -> Vec<f64> {
        (0..80)
            .map(|i| {
                let noise = [0.3, -0.2, 0.5, -0.4, 0.1, -0.5, 0.2, 0.0, 0.4, -0.3][i % 10];
                (if i < 40 { 10.0 } else { 14.0 }) + noise
            })
            .collect()
    }

    #[test]
    fn a_step_is_found_where_it_began() {
        let cps = change_points(
            &step()
                .iter()
                .enumerate()
                .map(|(i, v)| (i as u64, *v))
                .collect::<Vec<_>>(),
            &Params::default(),
        );
        assert_eq!(cps, vec![40]);
    }

    #[test]
    fn two_regimes_two_segments_with_their_levels() {
        let s = samples(&step());
        let units = run(&input(&s), 0, 80_000, &Params::default());
        assert_eq!(units.len(), 2);
        assert!(units
            .iter()
            .all(|u| u.status == smysl_core::Status::Derived));
        let text = format!("{:?}", units[1].payload);
        assert!(!text.is_empty());
        // Same samples, same recipe: the same uids.
        let again = run(&input(&s), 0, 80_000, &Params::default());
        assert_eq!(
            units.iter().map(canonical_uid).collect::<Vec<_>>(),
            again.iter().map(canonical_uid).collect::<Vec<_>>()
        );
        // Another parameter is another recipe, and other units.
        let p = Params {
            h: Decimal::new(8, 0).unwrap(),
            ..Params::default()
        };
        assert_ne!(
            canonical_uid(&run(&input(&s), 0, 80_000, &p)[0]),
            canonical_uid(&units[0])
        );
    }

    #[test]
    fn a_flat_series_is_one_segment_and_a_constant_one_breaks_on_any_change() {
        let flat: Vec<f64> = step().into_iter().take(40).collect();
        assert_eq!(
            run(&input(&samples(&flat)), 0, 40_000, &Params::default()).len(),
            1
        );
        let mut constant = vec![5.0; 30];
        constant.extend([5.001; 10]);
        let units = run(&input(&samples(&constant)), 0, 40_000, &Params::default());
        assert_eq!(units.len(), 2);
    }

    #[test]
    fn too_few_samples_is_one_segment_and_none_is_none() {
        assert_eq!(
            run(&input(&samples(&[1.0, 2.0])), 0, 2_000, &Params::default()).len(),
            1
        );
        assert!(run(&input(&[]), 0, 1, &Params::default()).is_empty());
    }
}
