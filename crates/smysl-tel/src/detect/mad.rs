//! Robust z (RFC SMYSL-3 §21, TL-P5): points far from a baseline's median, as `x.tel/anomaly`s.
//!
//! Iglewicz and Hoaglin's modified z-score, against a **declared** baseline window:
//!
//! ```text
//! z = 0.6745 · (x − median) / MAD          MAD = median |b − median|, over the baseline
//! ```
//!
//! - `threshold` (default 3.5, Iglewicz and Hoaglin's) is the `|z|` above which a sample is
//!   anomalous.
//! - The baseline is a window of the same series and instrument, `[baseline_from, baseline_to)`,
//!   recorded in the recipe: a score means nothing without what it was scored against.
//!
//! A run of consecutive anomalous samples with one direction is one anomaly, `[its first sample,
//! the next sample after it)`, the last one ending at the window's end. Its score is the run's
//! largest `|z|`, and its direction `up` or `down`.
//!
//! A baseline with `MAD = 0` (more than half its values equal) scales by the mean absolute
//! deviation instead, `1.2533 · mean|b − median|`, as Iglewicz and Hoaglin do; one with no
//! spread at all takes `10⁻⁹·max(1, |median|)`, so any departure from a constant is anomalous.
//! An empty baseline scores nothing.

use super::{anomaly, Direction, Input, Recipe};
use crate::detect::pelt::median;
use crate::evidence::counted;
use crate::sketch::to_f64;
use smysl_core::{Decimal, UnitCore};
use smysl_series::Sample;

/// The detector and its version.
pub const ID: &str = "mad/1";

/// The robust z's parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    pub threshold: Decimal,
    pub baseline_from: u64,
    pub baseline_to: u64,
}

impl Params {
    /// The default threshold, against `[baseline_from, baseline_to)`.
    pub fn new(baseline_from: u64, baseline_to: u64) -> Params {
        Params {
            threshold: Decimal::new(35, -1).expect("3.5"),
            baseline_from,
            baseline_to,
        }
    }

    pub fn recipe(&self, from_ms: u64, to_ms: u64) -> Recipe {
        Recipe::new(ID)
            .with("threshold", self.threshold)
            .with(
                "baseline",
                format!("{}-{}", self.baseline_from, self.baseline_to),
            )
            .with("from", from_ms)
            .with("to", to_ms)
    }
}

/// The baseline's centre and scale: its median, and the MAD (or its fallbacks).
pub fn centre_and_scale(baseline: &[f64]) -> Option<(f64, f64)> {
    let med = median(baseline)?;
    let dev: Vec<f64> = baseline.iter().map(|b| (b - med).abs()).collect();
    let mad = median(&dev).unwrap_or(0.0);
    if mad > 0.0 {
        return Some((med, mad / 0.6745));
    }
    let mean_ad = dev.iter().sum::<f64>() / dev.len() as f64;
    if mean_ad > 0.0 {
        return Some((med, 1.2533 * mean_ad));
    }
    Some((med, 1e-9 * med.abs().max(1.0)))
}

/// Score `input` in `[from_ms, to_ms)` against `baseline` (the same series' samples in the
/// baseline window): one anomaly per run of samples beyond the threshold.
pub fn run(
    input: &Input<'_>,
    baseline: &[Sample],
    from_ms: u64,
    to_ms: u64,
    p: &Params,
) -> Vec<UnitCore> {
    let base: Vec<f64> = baseline
        .iter()
        .filter(|s| counted(s) && (p.baseline_from..p.baseline_to).contains(&s.ts_ms()))
        .map(|s| to_f64(s.value_dec()))
        .collect();
    let Some((med, scale)) = centre_and_scale(&base) else {
        return Vec::new();
    };
    let threshold = to_f64(p.threshold);
    let recipe = p.recipe(from_ms, to_ms);
    let points: Vec<(u64, f64)> = input
        .points()
        .into_iter()
        .filter(|(t, _)| (from_ms..to_ms).contains(t))
        .collect();

    let mut out = Vec::new();
    // The open run: its first index, direction and largest |z|.
    let mut open: Option<(usize, Direction, f64)> = None;
    let close = |run: (usize, Direction, f64), end: usize, out: &mut Vec<UnitCore>| {
        let (first, dir, score) = run;
        let to = points.get(end).map_or(to_ms, |x| x.0);
        if let Some(u) = anomaly(input, points[first].0, to, &recipe, score, Some(dir)) {
            out.push(u);
        }
    };
    for (i, (_, x)) in points.iter().enumerate() {
        let z = (x - med) / scale;
        let dir = if z > 0.0 {
            Direction::Up
        } else {
            Direction::Down
        };
        let beyond = z.abs() > threshold;
        match (open, beyond) {
            (Some((first, d, score)), true) if d == dir => {
                open = Some((first, d, score.max(z.abs())));
            }
            (Some(run), _) => {
                close(run, i, &mut out);
                open = beyond.then_some((i, dir, z.abs()));
            }
            (None, true) => open = Some((i, dir, z.abs())),
            (None, false) => {}
        }
    }
    if let Some(run) = open {
        close(run, points.len(), &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::schemas::tel;
    use smysl_core::{canonical_uid, Uid};

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

    fn payload(u: &UnitCore) -> smysl_core::surface::hjson::HObject {
        smysl_core::surface::payload_to_object(u.payload.as_deref().unwrap()).unwrap()
    }

    #[test]
    fn a_spike_and_a_dip_are_two_anomalies_with_their_directions() {
        // Baseline: 0..20, values 10 ± 1. Then a two-sample spike and a one-sample dip.
        let mut v: Vec<f64> = (0..20)
            .map(|i| 10.0 + [0.5, -0.5, 1.0, -1.0, 0.0][i % 5])
            .collect();
        v.extend([10.2, 30.0, 31.0, 10.1, 9.9, -5.0, 10.0]);
        let s = samples(&v);
        let p = Params::new(0, 20_000);
        let units = run(&input(&s), &s, 20_000, 27_000, &p);
        assert_eq!(units.len(), 2);
        let a = payload(&units[0]);
        assert_eq!(
            a.get("tel:direction").unwrap().value,
            smysl_core::surface::hjson::HValue::Str("up".into())
        );
        assert_eq!(
            a.get("tel:from_ms").unwrap().value,
            smysl_core::surface::hjson::HValue::Int(21_000)
        );
        assert_eq!(
            a.get("tel:to_ms").unwrap().value,
            smysl_core::surface::hjson::HValue::Int(23_000)
        );
        let b = payload(&units[1]);
        assert_eq!(
            b.get("tel:direction").unwrap().value,
            smysl_core::surface::hjson::HValue::Str("down".into())
        );
        assert!(units.iter().all(|u| u.schema.as_str() == tel::ANOMALY));
        assert!(units
            .iter()
            .all(|u| u.status == smysl_core::Status::Derived));
        // The recipe names the baseline it was scored against.
        assert!(format!("{:?}", a.get("tel:recipe").unwrap().value).contains("baseline=0-20000"));
        let again = run(&input(&s), &s, 20_000, 27_000, &p);
        assert_eq!(
            units.iter().map(canonical_uid).collect::<Vec<_>>(),
            again.iter().map(canonical_uid).collect::<Vec<_>>()
        );
    }

    #[test]
    fn mad_falls_back_when_half_the_baseline_is_one_value() {
        assert_eq!(
            centre_and_scale(&[5.0, 5.0, 5.0, 6.0]).map(|c| c.0),
            Some(5.0)
        );
        let (_, scale) = centre_and_scale(&[5.0, 5.0, 5.0, 6.0]).unwrap();
        assert!((scale - 1.2533 * 0.25).abs() < 1e-12);
        let (_, tiny) = centre_and_scale(&[7.0; 4]).unwrap();
        assert!(tiny > 0.0 && tiny < 1e-7);
        assert!(centre_and_scale(&[]).is_none());
    }

    #[test]
    fn an_empty_baseline_scores_nothing() {
        let s = samples(&[1.0, 100.0]);
        assert!(run(&input(&s), &s, 0, 2_000, &Params::new(50_000, 60_000)).is_empty());
    }
}
