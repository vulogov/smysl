//! A series at a coarser cadence (RFC SMYSL-9 §4, CC-P1): what a scraper at one sample every
//! `step` would have recorded, or a rollup of each step.
//!
//! Buckets are `[from + k·step, from + (k+1)·step)`, aligned to `from` and never to the first
//! sample, so two reads of one series with one `from` bucket alike. Only counted samples (neither
//! stale nor without a value) are read, and samples before `from` are not. An empty bucket
//! yields no sample, so a gap stays a gap.
//!
//! - [`Agg::Last`] emulates a scrape: each bucket's last sample, itself, with its own instant
//!   and flags. A scraper whose scrapes fall at bucket ends, keeping a sample's own timestamp
//!   and never recording one twice, records exactly these.
//! - [`Agg::Mean`] and [`Agg::Max`] emulate a rollup: one plain sample at the bucket's start,
//!   the mean of the bucket's values (in binary64, summed in sample order, written back as the
//!   shortest decimal that round-trips) or its largest value, exactly as recorded (the first of
//!   equals).

use smysl_core::Decimal;

use crate::Sample;

/// How a bucket becomes one sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Agg {
    /// The bucket's last sample: a scrape.
    #[default]
    Last,
    /// The mean of its values, at the bucket's start.
    Mean,
    /// Its largest value, at the bucket's start.
    Max,
}

impl Agg {
    /// `last`, `mean` or `max`: a recipe's spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Agg::Last => "last",
            Agg::Mean => "mean",
            Agg::Max => "max",
        }
    }

    /// The aggregation a recipe names.
    pub fn parse(s: &str) -> Option<Agg> {
        match s {
            "last" => Some(Agg::Last),
            "mean" => Some(Agg::Mean),
            "max" => Some(Agg::Max),
            _ => None,
        }
    }
}

fn to_f64(d: Decimal) -> f64 {
    format!("{}e{}", d.mantissa(), d.exponent())
        .parse()
        .unwrap_or(0.0)
}

/// `samples`, in sample order, at one sample every `step_ms` from `from_ms` (see the module).
/// A `step_ms` of 0 resamples nothing: the counted samples from `from_ms` on, unchanged.
pub fn resample(samples: &[Sample], from_ms: u64, step_ms: u64, agg: Agg) -> Vec<Sample> {
    let counted = samples
        .iter()
        .filter(|s| s.is_counted() && s.ts_ms() >= from_ms);
    if step_ms == 0 {
        return counted.copied().collect();
    }
    let mut out = Vec::new();
    let mut bucket: Vec<&Sample> = Vec::new();
    let mut at: Option<u64> = None;
    let close = |k: u64, b: &mut Vec<&Sample>, out: &mut Vec<Sample>| {
        let start = from_ms + k * step_ms;
        let one = match agg {
            Agg::Last => b.last().map(|s| **s),
            Agg::Max => b
                .iter()
                .copied()
                .reduce(|m, s| {
                    if to_f64(s.value_dec()) > to_f64(m.value_dec()) {
                        s
                    } else {
                        m
                    }
                })
                .map(|s| Sample::value(start, s.value_dec())),
            Agg::Mean => {
                let sum: f64 = b.iter().map(|s| to_f64(s.value_dec())).sum();
                Decimal::from_f64(sum / b.len() as f64)
                    .ok()
                    .map(|d| Sample::value(start, d))
            }
        };
        out.extend(one);
        b.clear();
    };
    for s in counted {
        let k = (s.ts_ms() - from_ms) / step_ms;
        if at.is_some_and(|a| a != k) {
            close(at.expect("a bucket"), &mut bucket, &mut out);
        }
        at = Some(k);
        bucket.push(s);
    }
    if let Some(k) = at {
        close(k, &mut bucket, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flags;

    fn d(x: i64) -> Decimal {
        Decimal::from_i64(x)
    }

    /// A one-second series with jitter, gaps and stale markers, from a little generator.
    fn generated(seed: u64, n: u64) -> Vec<Sample> {
        let mut x = seed;
        let mut next = || {
            x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        let mut out = Vec::new();
        for i in 0..n {
            let r = next();
            if r % 17 == 0 {
                continue; // a missed sample
            }
            if (i / 40) % 7 == 3 {
                continue; // a gap longer than a step
            }
            let ts = 1_000_000 + i * 1_000 + r % 300; // jitter under a third of a second
            if r % 23 == 0 {
                out.push(Sample::stale(ts));
            } else {
                out.push(Sample::value(ts, d((r % 1_000) as i64 - 500)));
            }
        }
        out
    }

    /// A scraper every `step` from `from`: at each scrape instant it takes the newest counted
    /// sample before that instant, and records it unless it recorded that sample already.
    fn scraped(samples: &[Sample], from: u64, step: u64) -> Vec<Sample> {
        let end = samples.last().map_or(from, |s| s.ts_ms()) + step;
        let mut out: Vec<Sample> = Vec::new();
        let mut at = from + step;
        while at <= end + step {
            let newest = samples
                .iter()
                .rev()
                .find(|s| s.is_counted() && s.ts_ms() >= from && s.ts_ms() < at);
            if let Some(s) = newest {
                if out.last() != Some(s) {
                    out.push(*s);
                }
            }
            at += step;
        }
        out
    }

    #[test]
    fn last_at_fifteen_seconds_is_what_a_fifteen_second_scraper_records() {
        for seed in 0..40 {
            let s = generated(seed, 2_000);
            for (from, step) in [(1_000_000, 15_000), (1_000_000, 5_000), (1_007_500, 15_000)] {
                assert_eq!(
                    resample(&s, from, step, Agg::Last),
                    scraped(&s, from, step),
                    "seed {seed}, from {from}, step {step}"
                );
            }
        }
    }

    #[test]
    fn buckets_align_to_from_and_gaps_stay_gaps() {
        let s: Vec<Sample> = [0u64, 4_000, 9_000, 31_000, 44_000]
            .iter()
            .enumerate()
            .map(|(i, t)| Sample::value(*t, d(i as i64 + 1)))
            .collect();
        // [0,10s) has 1,2,3; [10,20s) nothing; [20,30s) nothing; [30,40s) 4; [40,50s) 5.
        let last = resample(&s, 0, 10_000, Agg::Last);
        assert_eq!(last, vec![s[2], s[3], s[4]]);
        let mean = resample(&s, 0, 10_000, Agg::Mean);
        assert_eq!(
            mean,
            vec![
                Sample::value(0, d(2)),
                Sample::value(30_000, d(4)),
                Sample::value(40_000, d(5))
            ]
        );
        let max = resample(&s, 0, 10_000, Agg::Max);
        assert_eq!(max[0], Sample::value(0, d(3)));
        // From 5 s, the first bucket is [5s, 15s): sample 0 and 4 s are before it.
        assert_eq!(
            resample(&s, 5_000, 10_000, Agg::Last),
            vec![s[2], s[3], s[4]]
        );
        assert_eq!(
            resample(&s, 5_000, 10_000, Agg::Mean)[0],
            Sample::value(5_000, d(3))
        );
    }

    #[test]
    fn stale_markers_are_no_reading_and_a_mean_is_the_shortest_decimal() {
        let s = vec![
            Sample::value(0, d(1)),
            Sample::value(1_000, d(2)),
            Sample::stale(2_000),
            Sample::value(3_000, Decimal::parse_token("0.5").unwrap()),
        ];
        let last = resample(&s, 0, 10_000, Agg::Last);
        assert_eq!(last, vec![s[3]]);
        assert_eq!(
            resample(&s, 0, 10_000, Agg::Mean),
            vec![Sample::value(
                0,
                Decimal::parse_token("1.1666666666666667").unwrap()
            )]
        );
        // Only a stale marker in a bucket is an empty bucket.
        assert!(resample(&[Sample::stale(5)], 0, 10, Agg::Last).is_empty());
        assert_eq!(s[2].flags() & flags::STALE, flags::STALE);
        // Step 0 is no resampling.
        assert_eq!(resample(&s, 1_000, 0, Agg::Max), vec![s[1], s[3]]);
    }

    #[test]
    fn the_aggregations_spell_as_recipes_do() {
        for a in [Agg::Last, Agg::Mean, Agg::Max] {
            assert_eq!(Agg::parse(a.as_str()), Some(a));
        }
        assert_eq!(Agg::parse("min"), None);
        assert_eq!(Agg::default(), Agg::Last);
    }
}
