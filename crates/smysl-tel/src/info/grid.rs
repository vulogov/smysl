//! From samples to a sequence of exact values on a grid (RFC SMYSL-5 §5.1, §5.2).
//!
//! Every step compares, subtracts and counts `dec` values through the crate's exact decimals;
//! nothing passes through binary64.
//!
//! - **The grid** of `[from_ms, to_ms)` is every `t` in it with `t mod step = 0`, ascending.
//! - **The sample in force at `t`** is the last sample, in sample order, whose `ts_ms` lies in
//!   `(t − hold, t]` and in the window, whatever its flags. `t` is missing when there is none,
//!   or when it is a stale marker (flag 1) or carries no recorded value (flag 4): a marker ends
//!   the value before it (R1-1).
//! - **`of`** maps those values to the sequence a binning reads: the value itself, or its
//!   increase over one step, missing across a reset (flag 2) or when it would be negative.

use smysl_series::{flags, Sample};

use crate::exact::BigDec;

/// What a statistic reads of a series (§5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Of {
    /// The value in force at each grid point.
    Value,
    /// The value at `t` minus the value at `t − step`.
    Increase,
}

impl Of {
    pub fn as_str(self) -> &'static str {
        match self {
            Of::Value => "value",
            Of::Increase => "increase",
        }
    }
}

/// The grid points of `[from_ms, to_ms)` at `step` (§5.1). `step` is positive.
pub fn points(from_ms: u64, to_ms: u64, step: u64) -> Vec<u64> {
    let first = from_ms.div_ceil(step).saturating_mul(step);
    (0..)
        .map(|i: u64| first.saturating_add(i.saturating_mul(step)))
        .take_while(|t| *t < to_ms && *t >= first)
        .collect()
}

/// The value in force at each of `grid`, from `samples` (in sample order, all in the window).
/// `None` where `t` is missing (§5.1).
pub(crate) fn in_force(samples: &[Sample], grid: &[u64], hold: u64) -> Vec<Option<BigDec>> {
    let mut out = Vec::with_capacity(grid.len());
    let mut next = 0usize;
    for &t in grid {
        while next < samples.len() && samples[next].ts_ms() <= t {
            next += 1;
        }
        let value = next
            .checked_sub(1)
            .map(|i| &samples[i])
            .filter(|s| s.ts_ms().saturating_add(hold) > t)
            .filter(|s| s.flags() & (flags::STALE | flags::NO_VALUE) == 0)
            .map(|s| BigDec::from_dec(&s.value_dec()));
        out.push(value);
    }
    out
}

/// Whether a sample with flag 2 (reset) lies in `(a, b]`, for `samples` in sample order.
fn reset_in(samples: &[Sample], a: u64, b: u64) -> bool {
    let start = samples.partition_point(|s| s.ts_ms() <= a);
    samples[start..]
        .iter()
        .take_while(|s| s.ts_ms() <= b)
        .any(|s| s.flags() & flags::RESET != 0)
}

/// The sequence `of` reads at each grid point (§5.2). The first point's increase is missing:
/// the window holds no value one step before it.
pub(crate) fn sequence(
    samples: &[Sample],
    grid: &[u64],
    hold: u64,
    step: u64,
    of: Of,
) -> Vec<Option<BigDec>> {
    let values = in_force(samples, grid, hold);
    match of {
        Of::Value => values,
        Of::Increase => (0..grid.len())
            .map(|i| {
                let (now, before) = (values[i].as_ref()?, values[i.checked_sub(1)?].as_ref()?);
                if reset_in(samples, grid[i].saturating_sub(step), grid[i]) {
                    return None;
                }
                let d = now.sub(before);
                (d.signum() >= 0).then_some(d)
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::Decimal;

    fn s(ts: u64, v: &str, f: u8) -> Sample {
        Sample::new(ts, None, Decimal::parse_token(v).unwrap(), f).unwrap()
    }

    fn text(xs: &[Option<BigDec>]) -> Vec<Option<String>> {
        xs.iter()
            .map(|x| x.as_ref().map(|b| b.to_dec().unwrap().to_string()))
            .collect()
    }

    #[test]
    fn the_grid_is_the_multiples_of_step_in_the_window() {
        assert_eq!(points(0, 50, 10), [0, 10, 20, 30, 40]);
        assert_eq!(points(5, 50, 10), [10, 20, 30, 40]);
        assert_eq!(points(10, 11, 10), [10]);
        assert_eq!(points(11, 19, 10), Vec::<u64>::new());
    }

    #[test]
    fn a_value_holds_for_hold_and_a_marker_ends_it() {
        let samples = [
            s(0, "1", 0),
            s(10, "2", 0),
            s(10, "3", 0), // one instant, two values: the later in sample order is in force
            s(32, "4", 0),
            s(45, "0", flags::STALE),
        ];
        let grid = points(0, 70, 10);
        let got = text(&in_force(&samples, &grid, 10));
        assert_eq!(
            got,
            [
                Some("1".into()),
                Some("3".into()),
                None, // 20: the sample at 10 is not in (10, 20]
                None, // 30: nothing in (20, 30]
                Some("4".into()),
                None, // 50: the stale marker at 45 is in force
                None,
            ]
        );
        // A longer hold carries a value over a gap, and still stops at the marker.
        let got = text(&in_force(&samples, &grid, 30));
        assert_eq!(got[2], Some("3".into()));
        assert_eq!(got[5], None);
    }

    #[test]
    fn an_increase_is_missing_at_a_reset_and_when_negative() {
        let samples = [
            s(0, "10", 0),
            s(10, "15", 0),
            s(20, "22", 0),
            s(30, "3", flags::RESET),
            s(40, "9", 0),
            s(50, "8", 0),
        ];
        let grid = points(0, 60, 10);
        let got = text(&sequence(&samples, &grid, 10, 10, Of::Increase));
        assert_eq!(
            got,
            [
                None,
                Some("5".into()),
                Some("7".into()),
                None, // the reset at 30
                Some("6".into()),
                None, // 8 − 9 is negative
            ]
        );
    }
}
