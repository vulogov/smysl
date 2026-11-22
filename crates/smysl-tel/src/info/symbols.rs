//! From a sequence of exact values to symbols (RFC SMYSL-5 §5.3, §5.4).
//!
//! A symbol is an `i64`. Each binning reads the sequence `of` produced ([`super::grid`]); a
//! point the sequence lacks gives no symbol, and neither does a point a binning cannot place.

use std::cmp::Ordering;

use smysl_core::Decimal;

use crate::exact::BigDec;

/// How values become symbols (§5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bins {
    /// `floor(x / w)`, `w > 0`.
    Width(Decimal),
    /// `floor(k · L(x) / n)`: ranks within the window. `mi/1` only.
    Rank(u32),
    /// The number of edges below `x`, the edges being the reference window's `k`-quantiles.
    Baseline(u32),
    /// The sign of the step's change: −1, 0 or 1.
    Diff,
}

impl Bins {
    /// The recipe's spelling: `width:0.5`, `rank:4`, `baseline:4`, `diff`.
    pub fn text(&self) -> String {
        match self {
            Bins::Width(w) => format!("width:{w}"),
            Bins::Rank(k) => format!("rank:{k}"),
            Bins::Baseline(k) => format!("baseline:{k}"),
            Bins::Diff => "diff".into(),
        }
    }

    /// Parse the recipe's spelling. `k` is 2 to 64 and `w` positive.
    pub fn parse(s: &str) -> Result<Bins, String> {
        if s == "diff" {
            return Ok(Bins::Diff);
        }
        let (kind, arg) = s
            .split_once(':')
            .ok_or_else(|| format!("`bins: {s}` is width:W, rank:K, baseline:K or diff"))?;
        let k = || -> Result<u32, String> {
            arg.parse::<u32>()
                .ok()
                .filter(|k| (2..=64).contains(k))
                .ok_or_else(|| format!("`bins: {s}`: K is 2 to 64"))
        };
        match kind {
            "width" => {
                let w = Decimal::parse_token(arg).map_err(|e| format!("`bins: {s}`: {e}"))?;
                if w.mantissa() <= 0 {
                    return Err(format!("`bins: {s}`: W is positive"));
                }
                Ok(Bins::Width(w))
            }
            "rank" => Ok(Bins::Rank(k()?)),
            "baseline" => Ok(Bins::Baseline(k()?)),
            _ => Err(format!(
                "`bins: {s}` is width:W, rank:K, baseline:K or diff"
            )),
        }
    }
}

/// The `k − 1` edges of `baseline:k` (§5.3): the reference values at sorted positions
/// `ceil(i · m / k) − 1`, `i = 1 … k − 1`, `m` the number present. Empty when none is.
pub(crate) fn edges(reference: &[Option<BigDec>], k: u32) -> Vec<BigDec> {
    let mut sorted: Vec<&BigDec> = reference.iter().flatten().collect();
    sorted.sort_by(|a, b| a.cmp_value(b));
    let m = sorted.len() as u64;
    if m == 0 {
        return Vec::new();
    }
    let k = u64::from(k);
    (1..k)
        .map(|i| sorted[((i * m).div_ceil(k) - 1) as usize].clone())
        .collect()
}

/// Symbols of `seq` under `bins`. `edges` are `baseline`'s, from [`edges`]; `Rank` is taken over
/// the points a pairing keeps ([`dense`], [`ranks`]), not over one series alone, and gives none
/// here.
pub(crate) fn symbols(seq: &[Option<BigDec>], bins: &Bins, edges: &[BigDec]) -> Vec<Option<i64>> {
    match bins {
        Bins::Width(w) => seq
            .iter()
            .map(|x| x.as_ref().and_then(|x| x.floor_div(w)))
            .collect(),
        Bins::Baseline(_) => seq
            .iter()
            .map(|x| {
                let x = x.as_ref()?;
                Some(
                    edges
                        .iter()
                        .filter(|e| e.cmp_value(x) == Ordering::Less)
                        .count() as i64,
                )
            })
            .collect(),
        Bins::Diff => (0..seq.len())
            .map(|i| {
                let (now, before) = (seq[i].as_ref()?, seq[i.checked_sub(1)?].as_ref()?);
                Some(i64::from(now.sub(before).signum()))
            })
            .collect(),
        Bins::Rank(_) => vec![None; seq.len()],
    }
}

/// Each present value's place among the distinct present values of `seq`, ascending: an exact
/// stand-in for the value that keeps its order, so that `rank:k` can be taken over any subset
/// of points by comparing integers (§5.3). `None` where `seq` is.
pub(crate) fn dense(seq: &[Option<BigDec>]) -> Vec<Option<i64>> {
    let mut sorted: Vec<&BigDec> = seq.iter().flatten().collect();
    sorted.sort_by(|a, b| a.cmp_value(b));
    sorted.dedup_by(|a, b| a.cmp_value(b) == Ordering::Equal);
    seq.iter()
        .map(|x| {
            let x = x.as_ref()?;
            Some(sorted.partition_point(|e| e.cmp_value(x) == Ordering::Less) as i64)
        })
        .collect()
}

/// `rank:k` over a set of values given by their dense keys (§5.3): `floor(k · L(x) / n)`, `L(x)`
/// the number of the set's values strictly less than `x` and `n` the set's size. Tied values
/// share a symbol.
pub fn ranks(keys: &[i64], k: u32) -> Vec<i64> {
    let mut sorted = keys.to_vec();
    sorted.sort_unstable();
    let n = keys.len() as u64;
    keys.iter()
        .map(|x| {
            let below = sorted.partition_point(|e| e < x) as u64;
            (u64::from(k) * below / n) as i64
        })
        .collect()
}

/// The ordinal pattern at each grid index (§5.4): the order of the `m` values at
/// `i, i + τ, …, i + (m − 1)·τ`, as the index in lexicographic order (0 to `m! − 1`) of the
/// permutation that sorts them ascending, equal values keeping time order. Missing when any of
/// the `m` is missing or falls past the window's last point.
pub(crate) fn ordinal(seq: &[Option<BigDec>], m: usize, tau: usize) -> Vec<Option<i64>> {
    (0..seq.len())
        .map(|i| {
            let values: Vec<&BigDec> = (0..m)
                .map(|j| seq.get(i + j * tau).and_then(Option::as_ref))
                .collect::<Option<_>>()?;
            // The permutation that sorts ascending, ties in time order (a stable sort).
            let mut order: Vec<usize> = (0..m).collect();
            order.sort_by(|&a, &b| values[a].cmp_value(values[b]));
            Some(lehmer(&order))
        })
        .collect()
}

/// The index of a permutation of `0..m` in lexicographic order.
pub fn lehmer(perm: &[usize]) -> i64 {
    let m = perm.len();
    let mut index = 0i64;
    for i in 0..m {
        let smaller_after = perm[i + 1..].iter().filter(|&&x| x < perm[i]).count() as i64;
        index = index * (m - i) as i64 + smaller_after;
    }
    index
}

/// `m!`, for `m` from 2 to 7.
pub fn factorial(m: usize) -> u64 {
    (1..=m as u64).product()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(xs: &[Option<&str>]) -> Vec<Option<BigDec>> {
        xs.iter()
            .map(|x| x.map(|t| BigDec::from_dec(&Decimal::parse_token(t).unwrap())))
            .collect()
    }

    #[test]
    fn bins_parse_and_print_one_way() {
        for s in ["width:0.5", "rank:4", "baseline:64", "diff"] {
            assert_eq!(Bins::parse(s).unwrap().text(), s);
        }
        assert_eq!(Bins::parse("width:0.50").unwrap().text(), "width:0.5");
        for bad in [
            "width:0",
            "width:-1",
            "rank:1",
            "baseline:65",
            "rank:x",
            "deciles",
        ] {
            assert!(Bins::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn width_floors_toward_negative_infinity() {
        let s = seq(&[Some("0.4"), Some("0.5"), Some("-0.1"), None, Some("2.49")]);
        let w = Bins::parse("width:0.5").unwrap();
        assert_eq!(
            symbols(&s, &w, &[]),
            [Some(0), Some(1), Some(-1), None, Some(4)]
        );
    }

    #[test]
    fn baseline_edges_are_the_reference_quantiles() {
        // Ten reference values 1…10, k = 4: positions ceil(10i/4) − 1 = 2, 4, 7: values 3, 5, 8.
        let r: Vec<String> = (1..=10).rev().map(|i| i.to_string()).collect();
        let r = seq(&r.iter().map(|s| Some(s.as_str())).collect::<Vec<_>>());
        let e = edges(&r, 4);
        let text: Vec<String> = e.iter().map(|b| b.to_dec().unwrap().to_string()).collect();
        assert_eq!(text, ["3", "5", "8"]);
        let s = seq(&[Some("3"), Some("3.01"), Some("100"), Some("-1"), None]);
        assert_eq!(
            symbols(&s, &Bins::Baseline(4), &e),
            [Some(0), Some(1), Some(3), Some(0), None]
        );
        assert!(edges(&seq(&[None, None]), 4).is_empty());
    }

    #[test]
    fn diff_is_the_sign_of_the_step() {
        let s = seq(&[
            Some("1"),
            Some("2"),
            Some("2"),
            Some("0.5"),
            None,
            Some("3"),
        ]);
        assert_eq!(
            symbols(&s, &Bins::Diff, &[]),
            [None, Some(1), Some(0), Some(-1), None, None]
        );
    }

    #[test]
    fn ranks_are_taken_over_the_set_and_ties_share_a_symbol() {
        let s = seq(&[
            Some("5"),
            None,
            Some("1.5"),
            Some("5.0"),
            Some("-2"),
            Some("9"),
        ]);
        let d = dense(&s);
        assert_eq!(d, [Some(2), None, Some(1), Some(2), Some(0), Some(3)]);
        let keys: Vec<i64> = d.iter().flatten().copied().collect();
        // n = 5; L = 2, 1, 2, 0, 4; floor(4L/5) = 1, 0, 1, 0, 3.
        assert_eq!(ranks(&keys, 4), [1, 0, 1, 0, 3]);
        // A subset is ranked on its own: (5, 9) gives 0 and 2 at k = 4.
        assert_eq!(ranks(&[2, 3], 4), [0, 2]);
    }

    #[test]
    fn lehmer_numbers_permutations_in_lexicographic_order() {
        assert_eq!(lehmer(&[0, 1, 2]), 0);
        assert_eq!(lehmer(&[0, 2, 1]), 1);
        assert_eq!(lehmer(&[1, 0, 2]), 2);
        assert_eq!(lehmer(&[2, 1, 0]), 5);
        assert_eq!(lehmer(&[6, 5, 4, 3, 2, 1, 0]), 5039);
        assert_eq!(factorial(7), 5040);
    }

    #[test]
    fn ordinal_patterns_keep_time_order_on_ties() {
        let s = seq(&[Some("3"), Some("1"), Some("2"), Some("2"), None, Some("5")]);
        // i=0: (3,1,2) sorts as positions 1,2,0 → [1,2,0] → index 3.
        // i=1: (1,2,2) ties keep time order → [0,1,2] → 0.
        // i=2: (2,2,None) → missing.
        let got = ordinal(&s, 3, 1);
        assert_eq!(got[0], Some(3));
        assert_eq!(got[1], Some(0));
        assert_eq!(got[2], None);
        assert_eq!(got[4], None);
        // τ = 2 at i=0: (3, 2, None) → missing; at i=1: (1, 2, 5) → 0.
        let got = ordinal(&s, 3, 2);
        assert_eq!((got[0], got[1]), (None, Some(0)));
    }
}
