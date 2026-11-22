//! `mi/1` (RFC SMYSL-5 §6.3): the plug-in mutual information between two series at a lag, a
//! scan over lags, and how often seeded circular-shift surrogates do as well.
//!
//! Each series arrives as **keys** on one grid: its symbols, or for `rank:k` its values' places
//! among its own distinct values, which are ranked again over the points each pairing keeps
//! (§5.3: "ranks are taken over the points where both series are present at the lag"). From
//! there everything is integers until the final sum:
//!
//! - **pairs** at lag `ℓ`: (key of `series` at grid index `i`, key of `with` at `i + ℓ`), for
//!   every `i` where both are present and `i + ℓ` lies on the grid;
//! - **value** `Σ (c_ab / N) · log₂(c_ab · N / (c_a · c_b))`, a left fold over the pairs in
//!   lexicographic order, and **bias** `(K_a − 1)(K_b − 1) / (2 · N · ln 2)`, stored and never
//!   subtracted; either side with one observed symbol is exactly 0 (§6.0 step 1);
//! - **surrogates**: the second sequence rotated over the whole grid by offsets drawn from
//!   ChaCha20 keyed by `BLAKE3("smysl/tel-info/1\0" ‖ recipe ‖ seed)`, as MinHash's functions are
//!   keyed (RFC SMYSL-3 §21).

use std::collections::BTreeMap;

use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};

use super::{num, symbols};

/// The key that seeds surrogate offsets, before the recipe and the seed (§6.3).
pub const DOMAIN: &[u8] = b"smysl/tel-info/1\0";

/// The longest scan, `-64..64` (§6.3).
pub const MAX_SCAN: u32 = 64;

/// The most surrogates a recipe may ask for (§6.3).
pub const MAX_NULL: u32 = 1000;

/// Which lags a recipe reads, in grid steps. A positive lag means `series` leads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lag {
    /// One lag.
    One(i64),
    /// Every lag from `−L` to `L`.
    Scan(u32),
}

impl Lag {
    /// The recipe's spelling: `3`, `-3`, `-10..10`.
    pub fn text(self) -> String {
        match self {
            Lag::One(l) => l.to_string(),
            Lag::Scan(l) => format!("-{l}..{l}"),
        }
    }

    /// The lags read, ascending.
    pub fn lags(self) -> Vec<i64> {
        match self {
            Lag::One(l) => vec![l],
            Lag::Scan(l) => (-i64::from(l)..=i64::from(l)).collect(),
        }
    }

    /// Parse a scan's spelling, `-L..L` with `1 ≤ L ≤ 64`.
    pub fn parse_scan(s: &str) -> Option<Lag> {
        let (a, b) = s.split_once("..")?;
        let l: u32 = b.parse().ok()?;
        (a.strip_prefix('-')? == b && (1..=MAX_SCAN).contains(&l) && b == l.to_string())
            .then_some(Lag::Scan(l))
    }
}

/// The pair counts at `lag` of two key sequences over one grid, in lexicographic order. `rank`
/// re-ranks each side over the pairs kept, as `rank:k` does (§5.3).
pub fn pairs(
    a: &[Option<i64>],
    b: &[Option<i64>],
    lag: i64,
    rank: Option<u32>,
) -> BTreeMap<(i64, i64), u64> {
    let g = a.len().min(b.len()) as i64;
    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    for i in 0..g {
        let j = i + lag;
        if !(0..g).contains(&j) {
            continue;
        }
        if let (Some(x), Some(y)) = (a[i as usize], b[j as usize]) {
            xs.push(x);
            ys.push(y);
        }
    }
    if let Some(k) = rank {
        if !xs.is_empty() {
            xs = symbols::ranks(&xs, k);
            ys = symbols::ranks(&ys, k);
        }
    }
    let mut c = BTreeMap::new();
    for p in xs.into_iter().zip(ys) {
        *c.entry(p).or_insert(0u64) += 1;
    }
    c
}

/// A plug-in mutual information, before it is stored.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plugin {
    /// Pairs counted.
    pub n: u64,
    /// Distinct symbols of `series` among them.
    pub k_a: u64,
    /// Distinct symbols of `with` among them.
    pub k_b: u64,
    /// Bits.
    pub value: f64,
    /// `(K_a − 1)(K_b − 1) / (2 · N · ln 2)`, bits.
    pub bias: f64,
}

/// The plug-in mutual information of pair `counts` and its bias term. `None` for no pairs.
pub fn plugin(counts: &BTreeMap<(i64, i64), u64>) -> Option<Plugin> {
    let n: u64 = counts.values().sum();
    if n == 0 {
        return None;
    }
    let (mut ca, mut cb) = (BTreeMap::new(), BTreeMap::new());
    for (&(a, b), &c) in counts {
        *ca.entry(a).or_insert(0u64) += c;
        *cb.entry(b).or_insert(0u64) += c;
    }
    let (k_a, k_b) = (ca.len() as u64, cb.len() as u64);
    if k_a == 1 || k_b == 1 {
        return Some(Plugin {
            n,
            k_a,
            k_b,
            value: 0.0,
            bias: 0.0,
        });
    }
    let nf = n as f64;
    let value = num::sum(counts.iter().map(|(&(a, b), &c)| {
        // Both products are integers, exact in binary64 while they are below 2⁵³.
        let num = (u128::from(c) * u128::from(n)) as f64;
        let den = (u128::from(ca[&a]) * u128::from(cb[&b])) as f64;
        (c as f64 / nf) * num::log2(num / den)
    }));
    let bias = ((k_a - 1) * (k_b - 1)) as f64 / (2.0 * nf * num::ln(2.0));
    Some(Plugin {
        n,
        k_a,
        k_b,
        value,
        bias,
    })
}

/// `b` rotated circularly by `o`: the key at index `i` moves to `(i + o) mod G`, a missing point
/// staying missing where it lands (§6.3, R1-6).
pub fn rotate(b: &[Option<i64>], o: u64) -> Vec<Option<i64>> {
    let g = b.len();
    let mut out = vec![None; g];
    if g == 0 {
        return out;
    }
    let o = (o % g as u64) as usize;
    for (i, x) in b.iter().enumerate() {
        out[(i + o) % g] = *x;
    }
    out
}

/// The least offset `guard` admits on a grid of `g` points, `ceil(a·g / b)`; the greatest is
/// `g` minus it. `None` when no offset lies between them.
pub fn offset_range(g: u64, guard: (u64, u64)) -> Option<(u64, u64)> {
    let lo = (u128::from(guard.0) * u128::from(g)).div_ceil(u128::from(guard.1)) as u64;
    let lo = lo.max(1);
    let hi = g.checked_sub(lo)?;
    (lo <= hi).then_some((lo, hi))
}

/// `count` offsets in `[lo, hi]`, drawn by rejection from ChaCha20 keyed by
/// `BLAKE3(DOMAIN ‖ recipe ‖ seed)`, the seed as eight little-endian bytes (§6.3).
pub fn offsets(recipe: &str, seed: u64, (lo, hi): (u64, u64), count: u32) -> Vec<u64> {
    let mut key = Vec::with_capacity(DOMAIN.len() + recipe.len() + 8);
    key.extend_from_slice(DOMAIN);
    key.extend_from_slice(recipe.as_bytes());
    key.extend_from_slice(&seed.to_le_bytes());
    let mut r = ChaCha20Rng::from_seed(smysl_core::hash_bytes(&key));
    let span = hi - lo + 1;
    let limit = u64::MAX - u64::MAX % span;
    (0..count)
        .map(|_| loop {
            let x = r.next_u64();
            if x < limit {
                return lo + x % span;
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(xs: &[i64]) -> Vec<Option<i64>> {
        xs.iter().map(|&x| (x >= 0).then_some(x)).collect()
    }

    #[test]
    fn a_lag_pairs_series_at_t_with_with_at_t_plus_lag() {
        let a = keys(&[0, 1, 2, -1, 4]);
        let b = keys(&[10, 11, 12, 13, 14]);
        let at = |l| pairs(&a, &b, l, None).into_keys().collect::<Vec<_>>();
        assert_eq!(at(0), [(0, 10), (1, 11), (2, 12), (4, 14)]);
        assert_eq!(at(1), [(0, 11), (1, 12), (2, 13)]);
        assert_eq!(at(-2), [(2, 10), (4, 12)]);
        assert!(at(5).is_empty());
    }

    #[test]
    fn identical_series_carry_their_entropy_and_independent_ones_none() {
        // x = y over four equally likely symbols: I = H = 2 bits.
        let x: Vec<Option<i64>> = (0..400).map(|i| Some(i % 4)).collect();
        let p = plugin(&pairs(&x, &x, 0, None)).unwrap();
        assert!((p.value - 2.0).abs() < 1e-12, "{}", p.value);
        assert_eq!((p.n, p.k_a, p.k_b), (400, 4, 4));
        let bias = 9.0 / (2.0 * 400.0 * std::f64::consts::LN_2);
        assert!((p.bias - bias).abs() < 1e-15);
        // y cycles through 0..4 inside each x: every pair once per block, I = 0.
        let a: Vec<Option<i64>> = (0..400).map(|i| Some(i / 4 % 4)).collect();
        let p = plugin(&pairs(&a, &x, 0, None)).unwrap();
        assert!(p.value.abs() < 1e-12, "{}", p.value);
        // One symbol on a side is exactly 0, bias included.
        let c = vec![Some(7); 400];
        let p = plugin(&pairs(&c, &x, 0, None)).unwrap();
        assert_eq!((p.value, p.bias), (0.0, 0.0));
        assert_eq!(plugin(&BTreeMap::new()), None);
    }

    #[test]
    fn ranks_are_taken_over_the_pairs_kept() {
        // At lag 1 the pairs are (5, 0) and (9, 3): each side ranks its own two, k = 2.
        let a = keys(&[5, 9, 1]);
        let b = keys(&[-1, 0, 3]);
        let got: Vec<_> = pairs(&a, &b, 1, Some(2)).into_keys().collect();
        assert_eq!(got, [(0, 0), (1, 1)]);
    }

    #[test]
    fn a_rotation_moves_every_point_and_keeps_missing_ones_missing() {
        let b = keys(&[0, 1, -1, 3]);
        assert_eq!(rotate(&b, 1), [Some(3), Some(0), Some(1), None]);
        assert_eq!(rotate(&b, 4), b);
    }

    #[test]
    fn offsets_stay_inside_the_guard_and_replay() {
        assert_eq!(offset_range(100, (1, 10)), Some((10, 90)));
        assert_eq!(offset_range(101, (1, 10)), Some((11, 90)));
        assert_eq!(offset_range(4, (1, 2)), Some((2, 2)));
        assert_eq!(offset_range(5, (1, 2)), None);
        assert_eq!(offset_range(1, (1, 10)), None);
        let a = offsets("mi/1?x", 7, (10, 90), 500);
        assert!(a.iter().all(|o| (10..=90).contains(o)));
        assert_eq!(a, offsets("mi/1?x", 7, (10, 90), 500));
        assert_ne!(a, offsets("mi/1?x", 8, (10, 90), 500));
        assert_ne!(a, offsets("mi/1?y", 7, (10, 90), 500));
    }

    #[test]
    fn scans_parse_one_way() {
        assert_eq!(Lag::parse_scan("-10..10"), Some(Lag::Scan(10)));
        assert_eq!(Lag::Scan(10).text(), "-10..10");
        assert_eq!(Lag::One(-3).text(), "-3");
        for bad in ["-3..4", "0..0", "-65..65", "10..10", "-03..03", "-3..", "3"] {
            assert_eq!(Lag::parse_scan(bad), None, "{bad}");
        }
        assert_eq!(Lag::Scan(2).lags(), [-2, -1, 0, 1, 2]);
    }
}
