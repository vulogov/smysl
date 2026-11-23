//! Calibration (RFC SMYSL-3 §21.1): a level detector's threshold, set from fault-free windows
//! to a stated false-alarm rate.
//!
//! The shipped defaults (MAD's 3.5, CUSUM's `h = 5`, PELT's `penalty = 10`) are textbook values
//! for i.i.d. noise. On one-second telemetry they flagged 90 % (MAD) and 100 % (CUSUM, PELT) of
//! fault-free windows (RFC SMYSL-5 §15), so the false-alarm rate a recipe stands for is a
//! property of the data, not of the detector. Calibration states it instead: given windows
//! nobody believes faulty, it returns the least parameter value at which at most a fraction
//! `rate` of them raise an alarm, and that value goes into the recipe like any other.
//!
//! **Trials.** A [`Pair`] is a reference window and the test window after it, as `(ts_ms,
//! value)` points of one stream. A trial is the pairs read together (each series of a service
//! over one window, say), and it **fires** at a value when any of its pairs does:
//!
//! - `mad/1` at threshold θ: some test point's robust z, against the reference's median and MAD
//!   ([`mad::centre_and_scale`]), exceeds θ in absolute value, as [`mad::run`] flags it;
//! - `cusum/1` at `h`, `pelt/1` at `penalty`, their other parameters as given: run over the
//!   reference and test points together, a change point falls in the test window.
//!
//! **The ladder.** Values are drawn from a fixed decimal ladder, ten steps a decade (1, 1.2,
//! 1.5, 2, 2.5, 3, 4, 5, 6, 8 times 10^e) from 10⁻² to 8·10²⁰: written as decimals, so a
//! calibrated recipe reads as written and no `pow` from a platform's libm is involved. A trial's
//! **score** is the largest ladder value at which it fires, or none if it fires at none.
//!
//! **The value** is the least ladder value `v` that at most `⌊rate · n⌋` of the `n` trials score
//! `v` or above. Every trial that fires anywhere at or above `v` scores at least `v`, so on the
//! trials given, the false alarms at `v`, and at every ladder value above it, are at most that
//! many, even where CUSUM's or PELT's alarms are not monotone in their parameter. A rate the
//! ladder's top cannot meet is refused rather than met approximately.

use super::{cusum, mad, pelt, Spec};
use crate::sketch::to_f64;
use smysl_core::Decimal;

/// The ladder's mantissas, in tenths: 1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8.
const STEPS: [i128; 10] = [10, 12, 15, 20, 25, 30, 40, 50, 60, 80];
/// The ladder's decades, `10^LOW` to `10^HIGH`.
const LOW: i64 = -2;
const HIGH: i64 = 20;

/// The calibration ladder, ascending.
pub fn ladder() -> Vec<Decimal> {
    (LOW..=HIGH)
        .flat_map(|e| {
            STEPS
                .iter()
                .map(move |&m| Decimal::new(m, e - 1).expect("on the ladder"))
        })
        .collect()
}

/// A reference window and the test window after it, of one stream.
#[derive(Debug, Clone, Copy)]
pub struct Pair<'a> {
    pub reference: &'a [(u64, f64)],
    pub test: &'a [(u64, f64)],
}

/// What a calibration found.
#[derive(Debug, Clone, PartialEq)]
pub struct Calibrated {
    /// The recipe given, its threshold (`mad/1`), `h` (`cusum/1`) or `penalty` (`pelt/1`) set
    /// to `value`.
    pub spec: Spec,
    pub value: Decimal,
    /// The trials, and how many of them may fire at `rate`.
    pub trials: usize,
    pub allowed: usize,
    /// The trials that fire at `value` (at most `allowed`).
    pub over: usize,
}

/// Whether `spec`'s detector, at `value`, finds something in the test window of any of `pairs`:
/// whether the trial fires.
pub fn fires(spec: &Spec, pairs: &[Pair<'_>], value: Decimal) -> bool {
    pairs.iter().any(|p| fires_on(spec, p, value))
}

fn fires_on(spec: &Spec, pair: &Pair<'_>, value: Decimal) -> bool {
    let joined = || -> Vec<(u64, f64)> {
        pair.reference
            .iter()
            .chain(pair.test.iter())
            .copied()
            .collect()
    };
    let split = pair.reference.len();
    match spec {
        Spec::Mad(_) => mad_score(pair).is_some_and(|z| z > to_f64(value)),
        Spec::Cusum(p) => {
            let p = cusum::Params { h: value, ..*p };
            cusum::change_points(&joined(), &p)
                .iter()
                .any(|&i| i >= split)
        }
        Spec::Pelt(p) => {
            let p = pelt::Params {
                penalty: value,
                ..*p
            };
            let xs: Vec<f64> = joined().iter().map(|x| x.1).collect();
            pelt::change_points(&xs, &p).iter().any(|&i| i >= split)
        }
        Spec::Episode(_) | Spec::Baro(_) => false,
    }
}

/// The largest |robust z| of `pair`'s test points against its reference; none for an empty
/// window.
fn mad_score(pair: &Pair<'_>) -> Option<f64> {
    let base: Vec<f64> = pair.reference.iter().map(|x| x.1).collect();
    let (med, scale) = mad::centre_and_scale(&base)?;
    pair.test
        .iter()
        .map(|x| ((x.1 - med) / scale).abs())
        .reduce(f64::max)
}

/// A trial's score: the largest ladder value at which `spec` fires on any of `pairs`, or none.
pub fn score(spec: &Spec, pairs: &[Pair<'_>]) -> Option<Decimal> {
    let ladder = ladder();
    if let Spec::Mad(_) = spec {
        // Monotone in θ: the largest |z| places it on the ladder directly.
        let z = pairs.iter().filter_map(mad_score).reduce(f64::max)?;
        return ladder.into_iter().rev().find(|v| z > to_f64(*v));
    }
    pairs
        .iter()
        .filter_map(|p| {
            ladder
                .iter()
                .rev()
                .find(|v| fires_on(spec, p, **v))
                .copied()
        })
        .max_by(|a, b| to_f64(*a).total_cmp(&to_f64(*b)))
}

/// `⌊rate · n⌋`, exactly, for `0 < rate < 1`.
fn allowed(rate: Decimal, n: usize) -> Result<usize, String> {
    let (m, e) = (rate.mantissa(), i64::from(rate.exponent()));
    if m <= 0 || e >= 0 || to_f64(rate) >= 1.0 {
        return Err(format!("a false-alarm rate is in (0, 1), not {rate}"));
    }
    let scaled = m * n as i128;
    Ok(match 10i128.checked_pow((-e) as u32) {
        Some(d) => (scaled / d) as usize,
        None => 0,
    })
}

/// Calibrate `spec` on `trials`, fault-free, to at most `rate` of them firing (§21.1).
///
/// Refused: a detector that has no threshold to set (`episode/1`, `baro/1`), no trials, a rate
/// outside (0, 1), or one the ladder's top cannot meet.
pub fn calibrate(
    spec: &Spec,
    trials: &[Vec<Pair<'_>>],
    rate: Decimal,
) -> Result<Calibrated, String> {
    if matches!(spec, Spec::Episode(_) | Spec::Baro(_)) {
        return Err(format!("{} has no threshold to calibrate", spec.id()));
    }
    let scores: Vec<Option<Decimal>> = trials.iter().map(|t| score(spec, t)).collect();
    from_scores(spec, &scores, rate)
}

/// [`calibrate`] from the trials' [`score`]s, computed once by a caller that needs them too.
pub fn from_scores(
    spec: &Spec,
    scores: &[Option<Decimal>],
    rate: Decimal,
) -> Result<Calibrated, String> {
    if matches!(spec, Spec::Episode(_) | Spec::Baro(_)) {
        return Err(format!("{} has no threshold to calibrate", spec.id()));
    }
    if scores.is_empty() {
        return Err("calibration needs at least one fault-free trial".into());
    }
    let n = scores.len();
    let allowed = allowed(rate, n)?;
    let scores: Vec<Option<f64>> = scores.iter().map(|s| s.map(to_f64)).collect();
    let over = |v: f64| scores.iter().filter(|s| s.is_some_and(|s| s >= v)).count();
    let value = ladder()
        .into_iter()
        .find(|v| over(to_f64(*v)) <= allowed)
        .ok_or_else(|| {
            format!(
                "{} fires on more than {allowed} of {} trials at every value up to {}",
                spec.id(),
                n,
                ladder().last().expect("a ladder")
            )
        })?;
    let spec = match spec {
        Spec::Mad(p) => Spec::Mad(mad::Params {
            threshold: value,
            ..*p
        }),
        Spec::Cusum(p) => Spec::Cusum(cusum::Params { h: value, ..*p }),
        Spec::Pelt(p) => Spec::Pelt(pelt::Params {
            penalty: value,
            ..*p
        }),
        Spec::Episode(_) | Spec::Baro(_) => unreachable!("refused above"),
    };
    Ok(Calibrated {
        spec,
        value,
        trials: n,
        allowed,
        over: over(to_f64(value)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic wobble: a small integer pattern, no randomness.
    fn noise(n: usize, seed: u64, amp: f64) -> Vec<f64> {
        let mut x = seed.wrapping_mul(2_654_435_761) | 1;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                amp * ((x % 1000) as f64 / 500.0 - 1.0)
            })
            .collect()
    }

    fn points(xs: &[f64], t0: u64) -> Vec<(u64, f64)> {
        xs.iter()
            .enumerate()
            .map(|(i, x)| (t0 + i as u64 * 1000, *x))
            .collect()
    }

    #[test]
    fn the_ladder_is_ascending_decimals_ten_a_decade() {
        let l = ladder();
        assert_eq!(l.len(), 10 * 23);
        assert_eq!(l[0].to_string(), "0.01");
        assert!(l.windows(2).all(|w| to_f64(w[0]) < to_f64(w[1])));
        assert!(!l.iter().any(|v| v.to_string() == "3.5"));
        assert!(l.iter().any(|v| v.to_string() == "5"));
    }

    #[test]
    fn the_rate_is_floored_exactly() {
        let r = |s: &str| Decimal::parse_token(s).unwrap();
        assert_eq!(allowed(r("0.10"), 90).unwrap(), 9);
        assert_eq!(allowed(r("0.29"), 100).unwrap(), 29);
        assert_eq!(allowed(r("0.05"), 19).unwrap(), 0);
        for bad in ["0", "1", "1.5", "-0.1"] {
            assert!(allowed(r(bad), 10).is_err(), "{bad}");
        }
    }

    #[test]
    fn calibration_holds_the_rate_on_its_trials_and_sees_a_shift() {
        type Points = Vec<(u64, f64)>;
        let series: Vec<(Points, Points)> = (0..40)
            .map(|s| {
                let xs = noise(120, s + 1, 1.0);
                (points(&xs[..60], 0), points(&xs[60..], 60_000))
            })
            .collect();
        let trials: Vec<Vec<Pair<'_>>> = series
            .iter()
            .map(|(r, t)| {
                vec![Pair {
                    reference: r,
                    test: t,
                }]
            })
            .collect();
        let rate = Decimal::parse_token("0.10").unwrap();
        let shifted: Vec<(u64, f64)> = series[0].1.iter().map(|(t, x)| (*t, x + 50.0)).collect();
        let fault = [Pair {
            reference: &series[0].0,
            test: &shifted,
        }];
        for text in [
            r#"{ detector: mad/1, baseline: "0-60000" }"#,
            "{ detector: cusum/1 }",
            "{ detector: pelt/1 }",
        ] {
            let spec = Spec::parse(text).unwrap();
            let c = calibrate(&spec, &trials, rate).unwrap();
            assert_eq!(c.allowed, 4, "{text}");
            assert!(c.over <= 4, "{text}: {c:?}");
            // The value is the least that holds the rate: one step down does not.
            let l = ladder();
            let i = l.iter().position(|v| *v == c.value).unwrap();
            if i > 0 {
                let below = to_f64(l[i - 1]);
                let n = trials
                    .iter()
                    .filter(|t| score(&spec, t).is_some_and(|s| to_f64(s) >= below))
                    .count();
                assert!(n > 4, "{text}");
            }
            assert!(
                fires(&c.spec, &fault, c.value),
                "{text}: a 50σ shift is seen at {}",
                c.value
            );
            // Calibrating twice gives one answer.
            assert_eq!(calibrate(&spec, &trials, rate).unwrap(), c);
        }
    }

    #[test]
    fn what_has_no_threshold_or_no_trials_is_refused() {
        let rate = Decimal::parse_token("0.1").unwrap();
        let e = Spec::parse("{ detector: episode/1 }").unwrap();
        assert!(calibrate(&e, &[vec![]], rate).is_err());
        let c = Spec::parse("{ detector: cusum/1 }").unwrap();
        assert!(calibrate(&c, &[], rate).is_err());
    }
}
