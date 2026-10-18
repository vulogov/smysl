//! The segment bootstrap and its percentile interval (SMYSL-2.6 §3.5, §3.7).
//!
//! A replicate resamples the scope's segments with replacement — as many draws as there are
//! segments, each an index from [`Rng::index`] — and the statistic is recomputed over the
//! segments drawn, in the order drawn. Segments and not tokens, because a text's tokens are not
//! independent and its segments are closer to it: a verse, a chapter, a message is the unit a
//! reader would call one observation, and resampling words would pretend the text has as many
//! observations as it has words.
//!
//! **One generator per result, drawn in replicate order.** The generator is keyed by the
//! recipe's hash and seed ([`Rng::new`]), so a recipe that differs in anything — `boot:b`
//! included — draws a different stream, and two runs of one recipe draw the same one.
//!
//! **Nearest-rank percentiles, in integers.** The interval at coverage `num/den` is the
//! replicates at ranks `⌈B · (den − num) / (2·den)⌉` and `⌈B · (den + num) / (2·den)⌉` of the
//! sorted values, 1-based — for 95% over 1000 replicates, the 25th and the 975th. Nearest rank
//! is chosen over interpolation because it returns a replicate rather than a number between two
//! of them, and the rank is computed without a float, so a port has nothing to round.

use crate::num::q20;
use crate::outcome::{Ci, Num};
use crate::rng::Rng;

/// The coverage every interval in this crate is computed at.
pub const LEVEL: (u32, u32) = (95, 100);

/// Fill `out` with one replicate's draws: `segments` indices in `[0, segments)`.
pub fn draw(rng: &mut Rng, segments: usize, out: &mut Vec<usize>) {
    out.clear();
    out.extend((0..segments).map(|_| rng.index(segments)));
}

/// The 1-based nearest ranks of the interval at coverage `level` over `b` replicates.
pub fn ranks(b: usize, level: (u32, u32)) -> (usize, usize) {
    let (num, den) = (level.0 as u128, level.1 as u128);
    let b = b as u128;
    let rank = |p: u128| -> usize {
        // ⌈b · p / (2·den)⌉, at least 1.
        let r = (b * p).div_ceil(2 * den);
        r.max(1) as usize
    };
    (rank(den - num), rank(den + num))
}

/// The percentile interval of `values` at [`LEVEL`], as fixed-point numbers.
///
/// `None` when there is nothing to take an interval of, or when a replicate is not finite —
/// a replicate that produced NaN is a replicate whose statistic was undefined, and an interval
/// that sorted it somewhere would be an interval over a different set of replicates than the
/// one reported.
pub fn interval(values: &mut [f64]) -> Option<Ci> {
    if values.is_empty() || values.iter().any(|v| !v.is_finite()) {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let (lo, hi) = ranks(values.len(), LEVEL);
    let at = |r: usize| values[(r - 1).min(values.len() - 1)];
    Some(Ci {
        lo: Num::Fixed(q20(at(lo))?),
        hi: Num::Fixed(q20(at(hi))?),
        level: LEVEL,
        b: u32::try_from(values.len()).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_are_nearest_rank_and_never_zero() {
        assert_eq!(ranks(1000, LEVEL), (25, 975));
        assert_eq!(ranks(200, LEVEL), (5, 195));
        assert_eq!(ranks(10, LEVEL), (1, 10));
        assert_eq!(ranks(1, LEVEL), (1, 1));
        assert_eq!(ranks(999, (90, 100)), (50, 950));
    }

    #[test]
    fn an_interval_is_two_replicates_and_refuses_a_nan() {
        let mut v: Vec<f64> = (1..=1000).rev().map(f64::from).collect();
        let ci = interval(&mut v).unwrap();
        assert_eq!(ci.lo, Num::Fixed(q20(25.0).unwrap()));
        assert_eq!(ci.hi, Num::Fixed(q20(975.0).unwrap()));
        assert_eq!(ci.b, 1000);
        assert!(interval(&mut []).is_none());
        assert!(interval(&mut [1.0, f64::NAN]).is_none());
    }

    #[test]
    fn a_draw_is_as_long_as_the_scope_and_inside_it() {
        let mut rng = Rng::new(&[7; 32], 0);
        let mut out = Vec::new();
        draw(&mut rng, 13, &mut out);
        assert_eq!(out.len(), 13);
        assert!(out.iter().all(|&i| i < 13));
    }
}
