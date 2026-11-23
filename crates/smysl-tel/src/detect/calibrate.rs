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
//!
//! **Per series** ([`calibrate_each`]). One value for a whole trial is set by its least-moving
//! series: a series constant over its reference scores at the ladder's top for any change, and
//! the value rises until it is quiet, leaving the busy series read far above their noise (RFC
//! SMYSL-5 §15, R8). Calibrating each series, keyed by the caller, gives each its own value. The
//! trial still fires when any series fires at its own value, so the rate is held jointly: for a
//! per-series count `q`, each series takes the least value that at most `q` of its trials score
//! at or above, and `q` is the largest count at which at most `⌊rate · n⌋` trials then fire at
//! all. At `q = 0` a series that scores at the ladder's top takes the top and fires only in those
//! trials; a rate is refused when they are more than `⌊rate · n⌋`. A series' count is capped one
//! below the number of trials it scored in, so its value is never at or below all of its scores
//! (the ladder's foot, a hair trigger no evidence supports); the cap rises with `q`, so the trials
//! firing rise with `q` and the largest `q` is well defined. A series that never fired keeps the
//! recipe's value, marked uncalibrated; it fires in no trial at any value. The bound has the same
//! guarantee as one value's, series by series, and a series absent from a trial cannot fire in
//! it.

use std::collections::BTreeMap;

use super::{cusum, mad, pelt, Options, Spec};
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
    /// What it was calibrated to, the trials, and how many of them may fire at it.
    pub target: Target,
    pub trials: usize,
    pub allowed: usize,
    /// The trials that fire at `value` (at most `allowed`).
    pub over: usize,
}

/// What a calibration is held to: a false-alarm `rate` in (0, 1), and a `margin` of at least 1
/// that divides it, so that at most `⌊rate · n / margin⌋` of `n` trials may fire (§21.1). A
/// margin leaves room for a value carried to data it was not set on, where nothing bounds its
/// rate; a margin of 1 is the rate as stated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    rate: Decimal,
    margin: Decimal,
}

impl Target {
    /// `rate`, with a margin of 1; refused outside (0, 1).
    pub fn new(rate: Decimal) -> Result<Target, String> {
        let (m, e) = (rate.mantissa(), rate.exponent());
        if m <= 0 || e >= 0 || to_f64(rate) >= 1.0 {
            return Err(format!("a false-alarm rate is in (0, 1), not {rate}"));
        }
        Ok(Target {
            rate,
            margin: Decimal::from_i64(1),
        })
    }

    /// The target with `margin`; refused under 1.
    pub fn with_margin(self, margin: Decimal) -> Result<Target, String> {
        if margin.mantissa() <= 0 || to_f64(margin) < 1.0 {
            return Err(format!("a margin is at least 1, not {margin}"));
        }
        Ok(Target { margin, ..self })
    }

    pub fn rate(&self) -> Decimal {
        self.rate
    }

    pub fn margin(&self) -> Decimal {
        self.margin
    }

    /// `⌊rate · n / margin⌋`, exactly, in integers.
    pub fn allowed(&self, n: usize) -> usize {
        let (mr, er) = (self.rate.mantissa(), i64::from(self.rate.exponent()));
        let (mm, em) = (self.margin.mantissa(), i64::from(self.margin.exponent()));
        let pow = |k: i64| 10i128.checked_pow(u32::try_from(k).ok()?);
        let d = er - em;
        let (num, den) = if d >= 0 {
            match pow(d).and_then(|p| mr.checked_mul(n as i128)?.checked_mul(p)) {
                Some(x) => (x, mm),
                None => return n,
            }
        } else {
            match pow(-d).and_then(|p| mm.checked_mul(p)) {
                Some(x) => (mr * n as i128, x),
                None => return 0,
            }
        };
        usize::try_from(num / den).unwrap_or(n).min(n)
    }
}

/// Whether `spec`'s detector, at `value`, finds something in the test window of any of `pairs`:
/// whether the trial fires.
pub fn fires(spec: &Spec, opts: &Options, pairs: &[Pair<'_>], value: Decimal) -> bool {
    pairs.iter().any(|p| fires_on(spec, opts, p, value))
}

fn fires_on(spec: &Spec, opts: &Options, pair: &Pair<'_>, value: Decimal) -> bool {
    let joined = || -> Vec<(u64, f64)> {
        pair.reference
            .iter()
            .chain(pair.test.iter())
            .copied()
            .collect()
    };
    let split = pair.reference.len();
    match spec {
        Spec::Mad(_) => mad_score(opts, pair).is_some_and(|z| z > to_f64(value)),
        Spec::Cusum(p) => {
            let p = cusum::Params { h: value, ..*p };
            cusum::change_points_with(&joined(), &p, opts)
                .iter()
                .any(|&i| i >= split)
        }
        Spec::Pelt(p) => {
            let p = pelt::Params {
                penalty: value,
                ..*p
            };
            let xs: Vec<f64> = joined().iter().map(|x| x.1).collect();
            pelt::change_points_with(&xs, &p, opts)
                .iter()
                .any(|&i| i >= split)
        }
        Spec::Episode(_) | Spec::Baro(_) => false,
    }
}

/// The largest |robust z| of `pair`'s test points against its reference, under `opts` as
/// `mad::run_with` reads them; none for an empty window.
fn mad_score(opts: &Options, pair: &Pair<'_>) -> Option<f64> {
    let base: Vec<f64> = pair.reference.iter().map(|x| x.1).collect();
    let floor = opts.floor_of(base.iter().copied().chain(pair.test.iter().map(|x| x.1)));
    let (med, scale) = mad::centre_and_scale_floored(&base, floor)?;
    pair.test
        .iter()
        .map(|x| ((x.1 - med) / scale).abs())
        .reduce(f64::max)
}

/// A trial's score: the largest ladder value at which `spec` fires on any of `pairs`, or none.
pub fn score(spec: &Spec, opts: &Options, pairs: &[Pair<'_>]) -> Option<Decimal> {
    let ladder = ladder();
    if let Spec::Mad(_) = spec {
        // Monotone in θ: the largest |z| places it on the ladder directly.
        let z = pairs
            .iter()
            .filter_map(|p| mad_score(opts, p))
            .reduce(f64::max)?;
        return ladder.into_iter().rev().find(|v| z > to_f64(*v));
    }
    pairs
        .iter()
        .filter_map(|p| {
            ladder
                .iter()
                .rev()
                .find(|v| fires_on(spec, opts, p, **v))
                .copied()
        })
        .max_by(|a, b| to_f64(*a).total_cmp(&to_f64(*b)))
}

/// Calibrate `spec` under `opts` on `trials`, fault-free, to at most `target`'s share of them
/// firing (§21.1).
///
/// Refused: a detector that has no threshold to set (`episode/1`, `baro/1`), no trials, or a
/// target the ladder's top cannot meet.
pub fn calibrate(
    spec: &Spec,
    opts: &Options,
    trials: &[Vec<Pair<'_>>],
    target: Target,
) -> Result<Calibrated, String> {
    if matches!(spec, Spec::Episode(_) | Spec::Baro(_)) {
        return Err(format!("{} has no threshold to calibrate", spec.id()));
    }
    let scores: Vec<Option<Decimal>> = trials.iter().map(|t| score(spec, opts, t)).collect();
    from_scores(spec, &scores, target)
}

/// [`calibrate`] from the trials' [`score`]s, computed once by a caller that needs them too.
pub fn from_scores(
    spec: &Spec,
    scores: &[Option<Decimal>],
    target: Target,
) -> Result<Calibrated, String> {
    if matches!(spec, Spec::Episode(_) | Spec::Baro(_)) {
        return Err(format!("{} has no threshold to calibrate", spec.id()));
    }
    if scores.is_empty() {
        return Err("calibration needs at least one fault-free trial".into());
    }
    let n = scores.len();
    let allowed = target.allowed(n);
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
    Ok(Calibrated {
        spec: with_value(spec, value),
        value,
        target,
        trials: n,
        allowed,
        over: over(to_f64(value)),
    })
}

/// `spec` with its threshold (`mad/1`), `h` (`cusum/1`) or `penalty` (`pelt/1`) set to `value`.
fn with_value(spec: &Spec, value: Decimal) -> Spec {
    match spec {
        Spec::Mad(p) => Spec::Mad(mad::Params {
            threshold: value,
            ..*p
        }),
        Spec::Cusum(p) => Spec::Cusum(cusum::Params { h: value, ..*p }),
        Spec::Pelt(p) => Spec::Pelt(pelt::Params {
            penalty: value,
            ..*p
        }),
        other => other.clone(),
    }
}

/// One series' calibrated value.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesValue {
    /// The recipe given, its threshold, `h` or `penalty` set to `value`.
    pub spec: Spec,
    pub value: Decimal,
    /// The trials it fires in at `value`.
    pub fired: usize,
    /// False when it fired in no trial at any value, so `value` is the recipe's own.
    pub calibrated: bool,
}

/// The threshold (`mad/1`), `h` (`cusum/1`) or `penalty` (`pelt/1`) `spec` gives.
fn value_of(spec: &Spec) -> Decimal {
    match spec {
        Spec::Mad(p) => p.threshold,
        Spec::Cusum(p) => p.h,
        Spec::Pelt(p) => p.penalty,
        Spec::Episode(_) | Spec::Baro(_) => Decimal::from_i64(0),
    }
}

/// What a per-series calibration found.
#[derive(Debug, Clone, PartialEq)]
pub struct CalibratedEach<K> {
    /// Each series' recipe and value, by key.
    pub series: BTreeMap<K, SeriesValue>,
    /// The per-series count the values were set at.
    pub q: usize,
    /// What it was calibrated to; the trials, how many of them may fire at it, and how many
    /// fire at the values.
    pub target: Target,
    pub trials: usize,
    pub allowed: usize,
    pub over: usize,
}

/// Calibrate `spec` per series (module comment): each trial is its pairs by key, and fires when
/// any key's pair fires at that key's value.
pub fn calibrate_each<K: Ord + Clone>(
    spec: &Spec,
    opts: &Options,
    trials: &[BTreeMap<K, Pair<'_>>],
    target: Target,
) -> Result<CalibratedEach<K>, String> {
    if matches!(spec, Spec::Episode(_) | Spec::Baro(_)) {
        return Err(format!("{} has no threshold to calibrate", spec.id()));
    }
    let scores: Vec<BTreeMap<K, Option<Decimal>>> = trials
        .iter()
        .map(|t| {
            t.iter()
                .map(|(k, p)| (k.clone(), score(spec, opts, std::slice::from_ref(p))))
                .collect()
        })
        .collect();
    each_from_scores(spec, &scores, target)
}

/// [`calibrate_each`] from each trial's per-key [`score`]s.
pub fn each_from_scores<K: Ord + Clone>(
    spec: &Spec,
    scores: &[BTreeMap<K, Option<Decimal>>],
    target: Target,
) -> Result<CalibratedEach<K>, String> {
    if matches!(spec, Spec::Episode(_) | Spec::Baro(_)) {
        return Err(format!("{} has no threshold to calibrate", spec.id()));
    }
    if scores.is_empty() {
        return Err("calibration needs at least one fault-free trial".into());
    }
    let n = scores.len();
    let allowed = target.allowed(n);
    let ladder: Vec<(Decimal, f64)> = ladder().into_iter().map(|v| (v, to_f64(v))).collect();
    // Each key's scores, over the trials it is in.
    let mut by_key: BTreeMap<K, Vec<f64>> = BTreeMap::new();
    for t in scores {
        for (k, s) in t {
            let e = by_key.entry(k.clone()).or_default();
            if let Some(s) = s {
                e.push(to_f64(*s));
            }
        }
    }
    // At count q: each key's least value that at most q of its trials score at or above.
    let values_at = |q: usize| -> BTreeMap<K, (Decimal, f64)> {
        by_key
            .iter()
            .map(|(k, xs)| {
                // A series that never fired keeps the recipe's value: nothing calibrates it, and
                // it fires in no trial at any value. Otherwise its count is capped one below the
                // trials it scored in, so its value is never at or below all its scores (the
                // ladder's foot); the cap rises with q, so firing stays monotone in q.
                let v = if xs.is_empty() {
                    let v = value_of(spec);
                    (v, to_f64(v))
                } else {
                    // A series that scores at the top more than q times takes the top, and
                    // fires there only in those trials.
                    let q = q.min(xs.len() - 1);
                    let top = *ladder.last().expect("a ladder");
                    ladder
                        .iter()
                        .find(|(_, f)| xs.iter().filter(|x| **x >= *f).count() <= q)
                        .copied()
                        .unwrap_or(top)
                };
                (k.clone(), v)
            })
            .collect()
    };
    let firing = |values: &BTreeMap<K, (Decimal, f64)>| {
        scores
            .iter()
            .filter(|t| {
                t.iter()
                    .any(|(k, s)| s.is_some_and(|s| to_f64(s) >= values[k].1))
            })
            .count()
    };
    let q = (0..=n)
        .rev()
        .find(|&q| firing(&values_at(q)) <= allowed)
        .ok_or_else(|| {
            format!(
                "{} fires on more than {allowed} of {n} trials with every series at the \
                 ladder's top, {}",
                spec.id(),
                ladder.last().expect("a ladder").0
            )
        })?;
    let values = values_at(q);
    let over = firing(&values);
    let series = values
        .iter()
        .map(|(k, (v, f))| {
            let xs = &by_key[k];
            let value = SeriesValue {
                spec: with_value(spec, *v),
                value: *v,
                fired: xs.iter().filter(|x| **x >= *f).count(),
                calibrated: !xs.is_empty(),
            };
            (k.clone(), value)
        })
        .collect();
    Ok(CalibratedEach {
        series,
        q,
        target,
        trials: n,
        allowed,
        over,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o() -> Options {
        Options::default()
    }

    fn target(rate: &str) -> Target {
        Target::new(Decimal::parse_token(rate).unwrap()).unwrap()
    }

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
    fn the_rate_and_margin_are_floored_exactly() {
        let d = |s: &str| Decimal::parse_token(s).unwrap();
        assert_eq!(target("0.10").allowed(90), 9);
        assert_eq!(target("0.29").allowed(100), 29);
        assert_eq!(target("0.05").allowed(19), 0);
        assert_eq!(target("0.10").with_margin(d("2")).unwrap().allowed(90), 4);
        assert_eq!(target("0.10").with_margin(d("1.5")).unwrap().allowed(90), 6);
        assert_eq!(target("0.3").with_margin(d("3")).unwrap().allowed(10), 1);
        for bad in ["0", "1", "1.5", "-0.1"] {
            assert!(Target::new(d(bad)).is_err(), "{bad}");
        }
        for bad in ["0.5", "0", "-2"] {
            assert!(target("0.1").with_margin(d(bad)).is_err(), "{bad}");
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
        let rate = target("0.10");
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
            let c = calibrate(&spec, &o(), &trials, rate).unwrap();
            assert_eq!(c.allowed, 4, "{text}");
            assert!(c.over <= 4, "{text}: {c:?}");
            // The value is the least that holds the rate: one step down does not.
            let l = ladder();
            let i = l.iter().position(|v| *v == c.value).unwrap();
            if i > 0 {
                let below = to_f64(l[i - 1]);
                let n = trials
                    .iter()
                    .filter(|t| score(&spec, &o(), t).is_some_and(|s| to_f64(s) >= below))
                    .count();
                assert!(n > 4, "{text}");
            }
            assert!(
                fires(&c.spec, &o(), &fault, c.value),
                "{text}: a 50σ shift is seen at {}",
                c.value
            );
            // Calibrating twice gives one answer.
            assert_eq!(calibrate(&spec, &o(), &trials, rate).unwrap(), c);
        }
    }

    #[test]
    fn per_series_values_follow_each_series_noise_and_hold_the_rate_jointly() {
        type Points = Vec<(u64, f64)>;
        // Two series a trial: "busy" with noise of amplitude 1, "idle" almost constant.
        let busy: Vec<(Points, Points)> = (0..40)
            .map(|s| {
                let xs = noise(120, s + 1, 1.0);
                (points(&xs[..60], 0), points(&xs[60..], 60_000))
            })
            .collect();
        let idle: Vec<(Points, Points)> = (0..40)
            .map(|s| {
                let mut xs = vec![7.0; 120];
                if s % 3 == 0 {
                    xs[90] = 8.0;
                }
                (points(&xs[..60], 0), points(&xs[60..], 60_000))
            })
            .collect();
        let rate = target("0.10");
        for text in ["{ detector: cusum/1 }", "{ detector: pelt/1 }"] {
            let spec = Spec::parse(text).unwrap();
            let each: Vec<BTreeMap<&str, Pair<'_>>> = (0..40)
                .map(|i| {
                    BTreeMap::from([
                        (
                            "busy",
                            Pair {
                                reference: &busy[i].0,
                                test: &busy[i].1,
                            },
                        ),
                        (
                            "idle",
                            Pair {
                                reference: &idle[i].0,
                                test: &idle[i].1,
                            },
                        ),
                    ])
                })
                .collect();
            let c = calibrate_each(&spec, &o(), &each, rate).unwrap();
            assert_eq!(c.allowed, 4);
            assert!(c.over <= 4, "{text}: {c:?}");
            let (b, i) = (c.series["busy"].value, c.series["idle"].value);
            assert!(to_f64(b) < to_f64(i), "{text}: busy {b} below idle {i}");
            // One value for both is set by the idle series, at or above its per-series value.
            let joint: Vec<Vec<Pair<'_>>> =
                each.iter().map(|t| t.values().copied().collect()).collect();
            let one = calibrate(&spec, &o(), &joint, rate).unwrap();
            assert!(
                to_f64(one.value) > to_f64(b),
                "{text}: {} vs {b}",
                one.value
            );
            // The joint bound holds when checked directly.
            let fired = each
                .iter()
                .filter(|t| {
                    t.iter().any(|(k, p)| {
                        fires(&spec, &o(), std::slice::from_ref(p), c.series[k].value)
                    })
                })
                .count();
            assert!(fired <= 4, "{text}: {fired}");
            assert_eq!(
                calibrate_each(&spec, &o(), &each, rate).unwrap(),
                c,
                "twice"
            );
        }
    }

    #[test]
    fn a_series_with_little_evidence_is_not_set_to_a_hair_trigger() {
        let spec = Spec::parse("{ detector: cusum/1 }").unwrap();
        let d = |x: &str| Some(Decimal::parse_token(x).unwrap());
        // "never" scored in no trial; "once" in one, under a q the others allow.
        let scores: Vec<BTreeMap<&str, Option<Decimal>>> = (0..20)
            .map(|i| {
                BTreeMap::from([
                    ("never", None),
                    ("once", if i == 0 { d("6") } else { None }),
                    ("busy", d(["2", "3", "4", "5"][i % 4])),
                ])
            })
            .collect();
        let c = each_from_scores(&spec, &scores, target("0.5")).unwrap();
        assert!(c.q >= 1, "{c:?}");
        let never = &c.series["never"];
        assert!(!never.calibrated);
        assert_eq!(never.value.to_string(), "5", "the recipe's own h");
        let once = &c.series["once"];
        assert!(once.calibrated);
        assert_eq!((once.value.to_string().as_str(), once.fired), ("8", 0));
        // The rate is used, not collapsed to no alarm at all: busy's 2, 3, 4, 5 give 4, and the
        // ten trials scoring 4 or 5 are the ten allowed.
        assert_eq!(c.series["busy"].value.to_string(), "4");
        assert_eq!((c.over, c.allowed), (10, 10));
    }

    #[test]
    fn a_resolution_floor_keeps_an_idle_series_off_the_ladders_top() {
        // Constant 7 over the reference; one sample of 8, one quantum up, in some test windows.
        type Points = Vec<(u64, f64)>;
        let idle: Vec<(Points, Points)> = (0..40)
            .map(|s| {
                let mut xs = vec![7.0; 120];
                if s % 3 == 0 {
                    xs[90] = 8.0;
                }
                (points(&xs[..60], 0), points(&xs[60..], 60_000))
            })
            .collect();
        let trials: Vec<Vec<Pair<'_>>> = idle
            .iter()
            .map(|(r, t)| {
                vec![Pair {
                    reference: r,
                    test: t,
                }]
            })
            .collect();
        let floor = Options::default().with_floor(crate::detect::Floor::Resolution);
        for text in [
            r#"{ detector: mad/1, baseline: "0-60000" }"#,
            "{ detector: cusum/1 }",
            "{ detector: pelt/1 }",
        ] {
            let spec = Spec::parse(text).unwrap();
            let tiny = calibrate(&spec, &o(), &trials, target("0.1")).unwrap();
            let res = calibrate(&spec, &floor, &trials, target("0.1")).unwrap();
            assert!(
                to_f64(tiny.value) > 1e6,
                "{text}: tiny floor {}",
                tiny.value
            );
            assert!(
                to_f64(res.value) < 1e3,
                "{text}: resolution floor {}",
                res.value
            );
            assert!(res.over <= res.allowed);
            // A step of ten with some jitter, held for the test window, is still seen. (A jump
            // between the only two values a series ever took would be one quantum: §21.2.)
            let step: Vec<(u64, f64)> = idle[1]
                .1
                .iter()
                .enumerate()
                .map(|(i, (t, _))| (*t, 17.0 + 0.5 * (i % 3) as f64))
                .collect();
            let fault = [Pair {
                reference: &idle[1].0,
                test: &step,
            }];
            assert!(fires(&res.spec, &floor, &fault, res.value), "{text}");
        }
    }

    /// A series that fires at the ladder's top in a few fault-free trials is held there, those
    /// trials counting against the rate jointly; only more of them than the rate allows refuse
    /// (SMYSL-5 §15, R11: `pelt/1` on RE2-TT).
    #[test]
    fn a_series_at_the_ladders_top_counts_against_the_rate_and_does_not_refuse() {
        let spec = Spec::parse("{ detector: pelt/1 }").unwrap();
        let top = *ladder().last().unwrap();
        let busy = Decimal::parse_token("40").unwrap();
        let trial = |i: usize, hit: bool| -> BTreeMap<&str, Option<Decimal>> {
            BTreeMap::from([
                (
                    "busy",
                    Some(if i % 2 == 0 {
                        busy
                    } else {
                        Decimal::from_i64(4)
                    }),
                ),
                ("flat", hit.then_some(top)),
            ])
        };
        // Two of thirty at the top; at rate 0.1, three may fire.
        let scores: Vec<_> = (0..30).map(|i| trial(i, i < 2)).collect();
        let c = each_from_scores(&spec, &scores, target("0.1")).unwrap();
        assert_eq!(c.series["flat"].value, top);
        assert_eq!(c.series["flat"].fired, 2);
        assert!(c.over <= c.allowed, "{} > {}", c.over, c.allowed);
        // Four at the top cannot be held to three.
        let scores: Vec<_> = (0..30).map(|i| trial(i, i < 4)).collect();
        let e = each_from_scores(&spec, &scores, target("0.1")).unwrap_err();
        assert!(e.contains("more than 3 of 30"), "{e}");
    }

    #[test]
    fn what_has_no_threshold_or_no_trials_is_refused() {
        let rate = target("0.1");
        let e = Spec::parse("{ detector: episode/1 }").unwrap();
        assert!(calibrate(&e, &o(), &[vec![]], rate).is_err());
        let c = Spec::parse("{ detector: cusum/1 }").unwrap();
        assert!(calibrate(&c, &o(), &[], rate).is_err());
    }
}
