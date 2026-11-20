//! A DDSketch whose buckets are the same on every machine (RFC SMYSL-3 §9.3, §21, TL-P4).
//!
//! **Why not `sketches-ddsketch`.** §21 names it. Its bucket index is `ceil(ln(x) / ln(γ))`, and
//! `f64::ln` is the platform's libm: IEEE 754 does not require it to be correctly rounded, so
//! two machines can put one value in two buckets, and GE-M3 replays every detector on x86_64
//! and aarch64 and requires the same bytes. This sketch uses no transcendental function:
//!
//! - **The mapping.** A positive `x` is `f × 2^E` with `f` in `[1, 2)`, which is exact — it is
//!   `x`'s own exponent and mantissa. Each octave is cut into `K` buckets at `g^0, g^1, …,
//!   g^K = 2`, so the index is `E·K + j` for the `j` with `g^j ≤ f < g^(j+1)`. `g = 2^(1/K)` is
//!   found by bisection, and `K` is the smallest for which `g ≤ γ = (1 + α)/(1 − α)`: the buckets
//!   are at least as fine as DDSketch's, so the relative accuracy is at least `α`.
//! - **Every float operation is a multiplication, a division or an addition**, which IEEE 754
//!   rounds correctly everywhere, and Rust does not fuse. The table `g^j` is built by repeated
//!   multiplication, and so is every value the sketch returns.
//! - **A bucket's value** is `2·lo·hi/(lo + hi)` of its bounds, the point whose relative
//!   distance to both is `(g − 1)/(g + 1) ≤ α`.
//!
//! **Merging is adding integer counts**, so sketches of shards merge to the sketch of the whole,
//! bit for bit, in any order (§9.4: "DDSketch merges are bit-exact in any order"). That is the
//! sharded run TL-P4's exit criterion asks for.
//!
//! **Values** are `dec`s, taken to the nearest binary64. One beyond binary64's range is taken
//! as the largest finite binary64 of its sign, and one below its smallest subnormal as zero:
//! a sketch of such values is still within `α` of everything binary64 can tell apart.
//!
//! **Quantiles** follow DDSketch's: the rank of `q` over `n` values is `q·(n − 1)`, and the
//! answer is the value of the first bucket, in value order, whose cumulative count exceeds it.

use std::collections::BTreeMap;

use smysl_core::Decimal;

/// The relative accuracies a sketch may be built with: coarser than 50 % is not a quantile,
/// finer than 0.1 % is a table of over 300 buckets an octave for no reason evidence needs.
pub const ALPHA_MIN: f64 = 0.001;
pub const ALPHA_MAX: f64 = 0.5;

/// The octave table for one accuracy.
#[derive(Debug, Clone, PartialEq)]
pub struct Mapping {
    /// The accuracy asked for.
    pub alpha: Decimal,
    /// Buckets per octave.
    pub k: u32,
    /// `g^0 … g^K`, with `g^K = 2` exactly.
    bounds: Vec<f64>,
    /// Each bucket's value in its octave: `2·lo·hi/(lo + hi)`.
    values: Vec<f64>,
}

/// Why a sketch could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SketchError {
    /// `alpha` is outside `[ALPHA_MIN, ALPHA_MAX]`.
    Alpha(String),
    /// A quantile rank outside `[0, 1]`.
    Rank(String),
}

impl core::fmt::Display for SketchError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SketchError::Alpha(a) => write!(
                f,
                "a sketch's relative accuracy is between {ALPHA_MIN} and {ALPHA_MAX}, not {a}"
            ),
            SketchError::Rank(q) => write!(f, "a quantile rank is between 0 and 1, not {q}"),
        }
    }
}

impl std::error::Error for SketchError {}

/// `g^k` by repeated multiplication.
fn power(g: f64, k: u32) -> f64 {
    let mut p = 1.0;
    for _ in 0..k {
        p *= g;
    }
    p
}

/// The `g` in `[1, 2]` with `g^k` closest to 2 from below, by bisection.
fn root2(k: u32) -> f64 {
    let (mut lo, mut hi) = (1.0f64, 2.0f64);
    for _ in 0..200 {
        let mid = (lo + hi) / 2.0;
        if mid == lo || mid == hi {
            break;
        }
        if power(mid, k) <= 2.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// A `dec` as the nearest binary64, clamped to the finite range.
pub fn to_f64(d: Decimal) -> f64 {
    let x: f64 = format!("{}e{}", d.mantissa(), d.exponent())
        .parse()
        .unwrap_or(0.0);
    if x.is_infinite() {
        f64::MAX.copysign(x)
    } else {
        x
    }
}

/// `x × 2^e`, exactly while the result is normal.
fn scale2(mut x: f64, mut e: i32) -> f64 {
    let two = |n: i32| f64::from_bits(((n + 1023) as u64) << 52);
    while e > 1000 {
        x *= two(1000);
        e -= 1000;
    }
    while e < -1000 {
        x *= two(-1000);
        e += 1000;
    }
    x * two(e)
}

/// A positive finite `x` as `(f, E)`, `x = f × 2^E`, `f` in `[1, 2)`. Exact.
fn split(x: f64) -> (f64, i32) {
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    if exp == 0 {
        // Subnormal: normalise by 2^64, which is exact, and account for it.
        let (f, e) = split(x * f64::from_bits(((64 + 1023) as u64) << 52));
        return (f, e - 64);
    }
    let f = f64::from_bits((bits & ((1u64 << 52) - 1)) | (1023u64 << 52));
    (f, exp - 1023)
}

impl Mapping {
    /// The mapping for relative accuracy `alpha`.
    pub fn new(alpha: Decimal) -> Result<Mapping, SketchError> {
        let a = to_f64(alpha);
        if !(ALPHA_MIN..=ALPHA_MAX).contains(&a) {
            return Err(SketchError::Alpha(alpha.to_string()));
        }
        let gamma = (1.0 + a) / (1.0 - a);
        let mut k = 1u32;
        let mut g = root2(k);
        while g > gamma {
            k += 1;
            g = root2(k);
        }
        let mut bounds = Vec::with_capacity(k as usize + 1);
        let mut p = 1.0f64;
        for _ in 0..k {
            bounds.push(p);
            p *= g;
        }
        bounds.push(2.0);
        let values = bounds
            .windows(2)
            .map(|b| 2.0 * b[0] * b[1] / (b[0] + b[1]))
            .collect();
        Ok(Mapping {
            alpha,
            k,
            bounds,
            values,
        })
    }

    /// The bucket of a positive finite `x`.
    pub fn index(&self, x: f64) -> i64 {
        let (f, e) = split(x);
        // The last j with g^j ≤ f; f < 2 = g^K, so j < K.
        let j = self.bounds[..self.k as usize].partition_point(|b| *b <= f) - 1;
        i64::from(e) * i64::from(self.k) + j as i64
    }

    /// The value a bucket stands for.
    pub fn value(&self, index: i64) -> f64 {
        let k = i64::from(self.k);
        let (e, j) = (index.div_euclid(k), index.rem_euclid(k));
        scale2(self.values[j as usize], e as i32)
    }
}

/// A DDSketch: counts per bucket, positive and negative, and zeros.
#[derive(Debug, Clone, PartialEq)]
pub struct Sketch {
    pub mapping: Mapping,
    positive: BTreeMap<i64, u64>,
    negative: BTreeMap<i64, u64>,
    zeros: u64,
    count: u64,
}

impl Sketch {
    pub fn new(mapping: Mapping) -> Sketch {
        Sketch {
            mapping,
            positive: BTreeMap::new(),
            negative: BTreeMap::new(),
            zeros: 0,
            count: 0,
        }
    }

    pub fn count(&self) -> u64 {
        self.count
    }

    pub fn add(&mut self, v: Decimal) {
        let x = to_f64(v);
        self.count += 1;
        if x == 0.0 {
            self.zeros += 1;
        } else if x > 0.0 {
            *self.positive.entry(self.mapping.index(x)).or_default() += 1;
        } else {
            *self.negative.entry(self.mapping.index(-x)).or_default() += 1;
        }
    }

    /// Add `other`'s counts. Both must have one mapping; the result does not depend on order.
    pub fn merge(&mut self, other: &Sketch) {
        assert_eq!(self.mapping.k, other.mapping.k, "one mapping");
        for (i, c) in &other.positive {
            *self.positive.entry(*i).or_default() += c;
        }
        for (i, c) in &other.negative {
            *self.negative.entry(*i).or_default() += c;
        }
        self.zeros += other.zeros;
        self.count += other.count;
    }

    /// The value at rank `q` in `[0, 1]`, or `None` for an empty sketch.
    pub fn quantile(&self, q: f64) -> Result<Option<Decimal>, SketchError> {
        if !(0.0..=1.0).contains(&q) {
            return Err(SketchError::Rank(q.to_string()));
        }
        if self.count == 0 {
            return Ok(None);
        }
        let rank = q * (self.count - 1) as f64;
        let mut cum = 0u64;
        // In value order: the most negative first.
        for (i, c) in self.negative.iter().rev() {
            cum += c;
            if cum as f64 > rank {
                return Ok(Decimal::from_f64(-self.mapping.value(*i)).ok());
            }
        }
        cum += self.zeros;
        if cum as f64 > rank {
            return Ok(Some(Decimal::ZERO));
        }
        for (i, c) in &self.positive {
            cum += c;
            if cum as f64 > rank {
                return Ok(Decimal::from_f64(self.mapping.value(*i)).ok());
            }
        }
        // Unreachable: the last bucket's cumulative count is `count > rank`.
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        Decimal::parse_token(s).unwrap()
    }

    #[test]
    fn the_buckets_are_at_least_as_fine_as_ddsketchs() {
        for (a, k) in [("0.01", 35), ("0.05", 7), ("0.001", 347), ("0.5", 1)] {
            let m = Mapping::new(d(a)).unwrap();
            assert_eq!(m.k, k, "alpha {a}");
            let g = m.bounds[1];
            let alpha = to_f64(d(a));
            assert!((g - 1.0) / (g + 1.0) <= alpha);
            assert_eq!(*m.bounds.last().unwrap(), 2.0);
        }
        assert!(Mapping::new(d("0.0001")).is_err());
        assert!(Mapping::new(d("0.9")).is_err());
    }

    #[test]
    fn every_value_is_within_alpha_of_its_bucket() {
        let m = Mapping::new(d("0.01")).unwrap();
        let mut x = 1e-310;
        while x < 1e308 {
            let v = m.value(m.index(x));
            assert!((v - x).abs() / x <= 0.01 + 1e-12, "{x} -> {v}");
            x *= 1.37;
        }
        // Bucket edges: g^j itself is in bucket j.
        for j in 0..m.k as usize {
            assert_eq!(m.index(m.bounds[j]), j as i64);
            assert_eq!(m.index(m.bounds[j] * 4.0), 2 * i64::from(m.k) + j as i64);
        }
    }

    #[test]
    fn quantiles_are_within_alpha_of_the_true_ones() {
        let m = Mapping::new(d("0.02")).unwrap();
        let mut s = Sketch::new(m);
        let mut xs = Vec::new();
        let mut seed = 7u64;
        for _ in 0..5000 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let v = ((seed >> 33) % 1_000_000) as i64 - 200_000;
            xs.push(v as f64 / 100.0);
            s.add(Decimal::new(i128::from(v), -2).unwrap());
        }
        xs.sort_by(f64::total_cmp);
        for q in [0.0, 0.1, 0.5, 0.9, 0.99, 1.0] {
            let want = xs[(q * (xs.len() - 1) as f64) as usize];
            let got = to_f64(s.quantile(q).unwrap().unwrap());
            assert!(
                (got - want).abs() <= 0.02 * want.abs() + 1e-9,
                "q {q}: {got} vs {want}"
            );
        }
        assert!(s.quantile(1.5).is_err());
        assert_eq!(
            Sketch::new(Mapping::new(d("0.02")).unwrap()).quantile(0.5),
            Ok(None)
        );
    }

    #[test]
    fn shards_merge_to_the_whole_in_any_order() {
        let m = Mapping::new(d("0.01")).unwrap();
        let values: Vec<Decimal> = (0..600)
            .map(|i| Decimal::new(i128::from((i * 7919) % 1000 - 300), -1).unwrap())
            .collect();
        let mut whole = Sketch::new(m.clone());
        values.iter().for_each(|v| whole.add(*v));
        let shards: Vec<Sketch> = values
            .chunks(77)
            .map(|c| {
                let mut s = Sketch::new(m.clone());
                c.iter().for_each(|v| s.add(*v));
                s
            })
            .collect();
        let mut fwd = Sketch::new(m.clone());
        shards.iter().for_each(|s| fwd.merge(s));
        let mut rev = Sketch::new(m);
        shards.iter().rev().for_each(|s| rev.merge(s));
        assert_eq!(fwd, whole);
        assert_eq!(rev, whole);
        for q in [0.05, 0.5, 0.95] {
            assert_eq!(fwd.quantile(q), whole.quantile(q));
        }
    }

    #[test]
    fn extremes_are_clamped_not_lost() {
        let mut s = Sketch::new(Mapping::new(d("0.01")).unwrap());
        s.add(d("1e400"));
        s.add(d("-1e400"));
        s.add(d("1e-400"));
        s.add(d("5e-324"));
        assert_eq!(s.count(), 4);
        let top = to_f64(s.quantile(1.0).unwrap().unwrap());
        assert!(top > 1.7e308);
        assert_eq!(
            s.quantile(0.0).unwrap().map(|d| d.mantissa() < 0),
            Some(true)
        );
    }
}
