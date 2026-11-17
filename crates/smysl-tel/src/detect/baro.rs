//! A BARO-style ranking (RFC SMYSL-3 §21, TL-P5): which series moved most when something went
//! wrong, as `x.tel/hypothesis` units.
//!
//! BARO (Pham et al., 2024) ranks root-cause candidates by a robust score of how far each metric
//! moved after the anomaly, against its own behaviour before it. This is that scorer, without
//! BARO's Bayesian change-point step: the instant `t` is given (an anomaly's start, an episode's,
//! an alert's), not estimated.
//!
//! ```text
//! score(s) = max over [t, t + post) of |x − median(pre)| / IQR(pre),    pre = [t − pre, t)
//! ```
//!
//! - The IQR is `Q3 − Q1` with linear interpolation between order statistics. An IQR of 0 falls
//!   back to the robust z's scale (mean absolute deviation, then a tiny constant), so a constant
//!   series that moved at all ranks high.
//! - A series with no sample before `t` or none after is not ranked.
//! - Ranks are by score, highest first, then series uid; the top `top` become hypotheses.
//!
//! Each hypothesis is `derived` (§21's table: a computed ranking, `Rung::Computed`), targets its
//! series, and is grounded on the series and on the trigger, when one is given. Its claim states
//! the rank, the score and the windows, so it is a sentence a person can check against the
//! samples.

use smysl_core::schemas::tel;
use smysl_core::surface::hjson::HValue;
use smysl_core::{
    canonical_uid, Decimal, SchemaId, SourceKind, SourceRef, Status, Uid, UnitCore, UnitCoreBuilder,
};

use super::{dec, Input, Recipe};
use crate::scrape::payload;

/// The detector and its version.
pub const ID: &str = "baro/1";

/// The ranking's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Params {
    /// The anomaly instant.
    pub at_ms: u64,
    /// How far before `at_ms` the baseline reaches, ms.
    pub pre_ms: u64,
    /// How far after `at_ms` a move is looked for, ms.
    pub post_ms: u64,
    /// How many ranked series become hypotheses.
    pub top: usize,
}

impl Params {
    pub fn new(at_ms: u64) -> Params {
        Params {
            at_ms,
            pre_ms: 3_600_000,
            post_ms: 600_000,
            top: 5,
        }
    }

    pub fn recipe(&self) -> Recipe {
        Recipe::new(ID)
            .with("at", self.at_ms)
            .with("pre", self.pre_ms)
            .with("post", self.post_ms)
            .with("top", self.top)
    }
}

/// `q` in `[0, 1]` of `sorted`, by linear interpolation between order statistics.
fn quantile(sorted: &[f64], q: f64) -> f64 {
    let pos = q * (sorted.len() - 1) as f64;
    let lo = pos as usize;
    let frac = pos - lo as f64;
    match sorted.get(lo + 1) {
        Some(hi) => sorted[lo] + (hi - sorted[lo]) * frac,
        None => sorted[lo],
    }
}

/// A series' robust score, or `None` if it has nothing before or after `at`.
pub fn score(points: &[(u64, f64)], p: &Params) -> Option<f64> {
    let pre_from = p.at_ms.saturating_sub(p.pre_ms);
    let mut pre: Vec<f64> = points
        .iter()
        .filter(|x| (pre_from..p.at_ms).contains(&x.0))
        .map(|x| x.1)
        .collect();
    let post: Vec<f64> = points
        .iter()
        .filter(|x| (p.at_ms..p.at_ms.saturating_add(p.post_ms)).contains(&x.0))
        .map(|x| x.1)
        .collect();
    if pre.is_empty() || post.is_empty() {
        return None;
    }
    pre.sort_by(f64::total_cmp);
    let med = quantile(&pre, 0.5);
    let mut scale = quantile(&pre, 0.75) - quantile(&pre, 0.25);
    if scale <= 0.0 {
        scale = super::mad::centre_and_scale(&pre).map_or(1.0, |c| c.1);
    }
    post.iter()
        .map(|x| (x - med).abs() / scale)
        .max_by(f64::total_cmp)
}

/// Rank `inputs` around `p.at_ms`; the top `p.top` as hypotheses, grounded also on `trigger`.
pub fn run(inputs: &[Input<'_>], trigger: Option<Uid>, p: &Params) -> Vec<UnitCore> {
    let mut ranked: Vec<(f64, &Input<'_>)> = inputs
        .iter()
        .filter_map(|i| score(&i.points(), p).map(|s| (s, i)))
        .filter(|(s, _)| s.is_finite())
        .collect();
    ranked.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| a.1.series.cmp(&b.1.series))
    });
    let n = ranked.len();
    let recipe = p.recipe();
    ranked
        .into_iter()
        .take(p.top)
        .enumerate()
        .filter_map(|(rank, (s, input))| {
            let s: Decimal = dec(s)?;
            let claim = format!(
                "rank {} of {n}: {} moved {s} IQR from its median over the {} ms before {}, \
                 within {} ms after",
                rank + 1,
                input.key,
                p.pre_ms,
                p.at_ms,
                p.post_ms
            );
            let mut grounds = vec![input.series];
            grounds.extend(trigger);
            Some(
                UnitCoreBuilder::new(
                    SchemaId::Extension(tel::HYPOTHESIS.to_string()),
                    tel::gist_of(&claim),
                    Status::Derived,
                )
                .source(
                    SourceRef::new(SourceKind::Tool, format!("detector:{}", recipe.text()))
                        .observed_at(p.at_ms.saturating_add(p.post_ms)),
                )
                .grounds(grounds)
                .payload(payload(&[
                    ("tel:claim", HValue::Str(claim)),
                    ("tel:target", HValue::Str(input.series.canonical())),
                ]))
                .build()
                .expect("a hypothesis unit is well formed by construction"),
            )
        })
        .collect()
}

/// The ranked hypotheses' uids, for comparing runs.
pub fn ranking_uids(units: &[UnitCore]) -> Vec<Uid> {
    units.iter().map(canonical_uid).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_series::Sample;

    fn series(n: u8, f: impl Fn(u64) -> f64) -> Vec<Sample> {
        (0..120u64)
            .map(|i| {
                let t = i * 60_000;
                Sample::new(
                    t,
                    None,
                    Decimal::from_f64(f(t) + (n as f64) * 0.001).unwrap(),
                    0,
                )
                .unwrap()
            })
            .collect()
    }

    fn input(n: u8, s: &[Sample]) -> Input<'_> {
        Input {
            series: Uid::from_bytes([n; 32]),
            scheme: "prom",
            key: ["cpu", "latency", "errors"][n as usize],
            instrument: Uid::from_bytes([9; 32]),
            samples: s,
        }
    }

    #[test]
    fn the_series_that_moved_most_ranks_first() {
        let at = 90 * 60_000;
        let noise = |t: u64| [0.0, 0.2, -0.1, 0.3, -0.2][(t / 60_000 % 5) as usize];
        let cpu = series(0, |t| 50.0 + noise(t) + if t >= at { 1.0 } else { 0.0 });
        let lat = series(1, |t| {
            0.2 + noise(t) / 10.0 + if t >= at { 3.0 } else { 0.0 }
        });
        let err = series(2, |t| 1.0 + noise(t));
        let inputs = [input(0, &cpu), input(1, &lat), input(2, &err)];
        let trigger = Uid::from_bytes([7; 32]);
        let p = Params::new(at);
        let hyps = run(&inputs, Some(trigger), &p);
        assert_eq!(hyps.len(), 3);
        let claim = |u: &UnitCore| {
            let o = smysl_core::surface::payload_to_object(u.payload.as_deref().unwrap()).unwrap();
            match &o.get("tel:claim").unwrap().value {
                HValue::Str(s) => s.clone(),
                _ => unreachable!(),
            }
        };
        assert!(
            claim(&hyps[0]).starts_with("rank 1 of 3: latency"),
            "{}",
            claim(&hyps[0])
        );
        assert!(claim(&hyps[2]).contains("errors"));
        assert!(hyps
            .iter()
            .all(|h| h.status == Status::Derived && h.grounds.contains(&trigger)));
        // Input order does not matter.
        let rev = [input(2, &err), input(0, &cpu), input(1, &lat)];
        assert_eq!(
            ranking_uids(&run(&rev, Some(trigger), &p)),
            ranking_uids(&hyps)
        );
        // `top` cuts it.
        assert_eq!(run(&inputs, None, &Params { top: 1, ..p }).len(), 1);
    }

    #[test]
    fn a_series_with_nothing_before_or_after_is_not_ranked() {
        let s = series(0, |_| 1.0);
        let p = Params::new(500 * 60_000);
        assert!(score(&input(0, &s).points(), &p).is_none());
    }

    #[test]
    fn quantiles_interpolate() {
        let v = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(quantile(&v, 0.5), 2.5);
        assert_eq!(quantile(&v, 0.25), 1.75);
        assert_eq!(quantile(&[5.0], 0.75), 5.0);
    }
}
