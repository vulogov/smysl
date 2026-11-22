//! GE-I5 (RFC SMYSL-5 §15, TI-P5): does transfer entropy name the right direction?
//!
//! `te` is reserved (§10) until this releases it. The estimator here is §10's, written in this
//! file and not in the library, because a statistic the experiment does not release is not
//! shipped: the plug-in transfer entropy from a source `x` to a target `y` with history length 1,
//!
//! ```text
//! T = Σ p(y′, y, x) · log₂( p(y′ | y, x) / p(y′ | y) ),   y′ = y[t+1], y = y[t], x = x[t+1−ℓ]
//! ```
//!
//! over `rank:4` symbols, scanned over source lags ℓ = 1 to 5 and maximised, as `mi/1`'s scan is;
//! with 200 surrogates, the source rotated over the whole grid by `mi/1`'s offsets (§6.3), each
//! maximised over the same lags. The sum is the crate's left fold through its `log2`.
//!
//! **What this runs.** No incidents with a known causal direction are on this machine, so the
//! pairs are simulated, and the report says so. A pair is a day of one-minute points, each
//! series an AR(1) with φ = 0.5 and unit noise:
//!
//! - **driven**: `y[t] = 0.5·y[t−1] + c·x[t−d] + e`, c from 0.3 to 0.8, d from 1 to 5 minutes;
//!   which of the pair is presented first is drawn, so order says nothing;
//! - **common driver**: an AR(1) `z` drives both, `x` at delay d₁ and `y` at d₂, each from 0 to
//!   5 minutes and each coupling from 0.3 to 0.8, and neither affects the other;
//! - **none**: two independent AR(1)s.
//!
//! **The stop condition** (§15): "released only if the direction is right in at least 90 % of
//! driven pairs **and** a direction is asserted, at `null_ge / null_b ≤ 0.05`, in at most 10 %
//! of common-driver pairs." Read here as fixed before the run:
//!
//! - each pair is computed both ways; a way is **significant** when its `null_ge / null_b` is at
//!   most 0.05;
//! - a direction is **asserted** when one way is significant, or both are, then the larger;
//! - a driven pair is **right** when the asserted direction is the true one (none asserted is
//!   not right);
//! - a common-driver pair counts against the bound when any direction is asserted.
//!
//! ```text
//! cargo run --release -p smysl-tel --example ge_i5 [-- --pairs 300 --seed 7]
//! ```

// Simulated input, not a stored number: the generators draw Box–Muller normals with the
// platform's `ln`, `sqrt` and `cos`. The estimator's logarithm is the crate's.
#![allow(clippy::disallowed_methods)]

use smysl_tel::info::{mi, num};

const POINTS: usize = 1440;
const K: usize = 4;
const LAGS: usize = 5;
const NULL: u32 = 200;
const GUARD: (u64, u64) = (1, 10);

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn between(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.unit()
    }

    fn normal(&mut self) -> f64 {
        let u = self.unit().max(1e-300);
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * self.unit()).cos()
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// An AR(1) with φ = 0.5, plus `c·drive[t−d]` when a drive is given. A warm-up of 50 points is
/// dropped so that the start carries no transient.
fn ar(r: &mut Rng, drive: Option<(&[f64], f64, usize)>) -> Vec<f64> {
    const WARM: usize = 50;
    let mut v = vec![0.0; POINTS + WARM];
    for t in 1..v.len() {
        let mut x = 0.5 * v[t - 1] + r.normal();
        if let Some((z, c, d)) = drive {
            if t >= d {
                x += c * z[t - d];
            }
        }
        v[t] = x;
    }
    v.split_off(WARM)
}

/// An AR(1) over the warm-up too, for a driver whose past the driven series reads.
fn driver(r: &mut Rng) -> Vec<f64> {
    let mut v = vec![0.0; POINTS + 50];
    for t in 1..v.len() {
        v[t] = 0.5 * v[t - 1] + r.normal();
    }
    v
}

/// `rank:4`: each value's place among the series' values, `floor(4·rank / n)`.
fn ranks(xs: &[f64]) -> Vec<u8> {
    let mut idx: Vec<usize> = (0..xs.len()).collect();
    idx.sort_by(|a, b| xs[*a].total_cmp(&xs[*b]));
    let mut out = vec![0u8; xs.len()];
    for (rank, i) in idx.into_iter().enumerate() {
        out[i] = (K * rank / xs.len()) as u8;
    }
    out
}

/// The plug-in transfer entropy from `x` to `y` at source lag `l`, bits.
fn te(x: &[u8], y: &[u8], l: usize) -> f64 {
    let mut c3 = [0u64; K * K * K];
    let mut cyx = [0u64; K * K];
    let mut cyy = [0u64; K * K];
    let mut cy = [0u64; K];
    let mut n = 0u64;
    for t in (l - 1)..y.len() - 1 {
        let (yn, yc, xs) = (y[t + 1] as usize, y[t] as usize, x[t + 1 - l] as usize);
        c3[(yn * K + yc) * K + xs] += 1;
        cyx[yc * K + xs] += 1;
        cyy[yn * K + yc] += 1;
        cy[yc] += 1;
        n += 1;
    }
    let nf = n as f64;
    num::sum((0..K * K * K).filter(|i| c3[*i] > 0).map(|i| {
        let (yn, yc, xs) = (i / (K * K), (i / K) % K, i % K);
        let c = c3[i];
        let num = (c * cy[yc]) as f64;
        let den = (cyx[yc * K + xs] * cyy[yn * K + yc]) as f64;
        (c as f64 / nf) * num::log2(num / den)
    }))
}

/// The scan's maximum over lags 1 to 5.
fn scan(x: &[u8], y: &[u8]) -> f64 {
    (1..=LAGS).map(|l| te(x, y, l)).fold(f64::MIN, f64::max)
}

/// `x` rotated by `o`, as `mi::rotate` rotates keys.
fn rotate(x: &[u8], o: u64) -> Vec<u8> {
    let keys: Vec<Option<i64>> = x.iter().map(|k| Some(i64::from(*k))).collect();
    mi::rotate(&keys, o)
        .into_iter()
        .map(|k| k.expect("no missing point") as u8)
        .collect()
}

/// The scan from `x` to `y`, and whether it is significant: `null_ge / null_b ≤ 0.05`.
fn way(x: &[u8], y: &[u8], seed: u64) -> (f64, bool) {
    let value = scan(x, y);
    let range = mi::offset_range(x.len() as u64, GUARD).expect("a long grid");
    let ge = mi::offsets("te/1?ge-i5", seed, range, NULL)
        .into_iter()
        .filter(|o| scan(&rotate(x, *o), y) >= value)
        .count();
    (value, ge as f64 / f64::from(NULL) <= 0.05)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Said {
    AtoB,
    BtoA,
    Neither,
}

/// Both ways of a pair, and the direction asserted.
fn assert_direction(a: &[f64], b: &[f64], seed: u64) -> Said {
    assert_both(a, b, seed).0
}

/// The direction asserted, and whether both ways were significant.
fn assert_both(a: &[f64], b: &[f64], seed: u64) -> (Said, bool) {
    let (ka, kb) = (ranks(a), ranks(b));
    let (ab, sab) = way(&ka, &kb, seed);
    let (ba, sba) = way(&kb, &ka, seed.wrapping_add(1));
    let said = match (sab, sba) {
        (true, false) => Said::AtoB,
        (false, true) => Said::BtoA,
        (true, true) if ab >= ba => Said::AtoB,
        (true, true) => Said::BtoA,
        (false, false) => Said::Neither,
    };
    (said, sab && sba)
}

fn arg(args: &[String], name: &str, default: u64) -> u64 {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let pairs = arg(&args, "--pairs", 300);
    let seed = arg(&args, "--seed", 7);
    let mut r = Rng(seed);

    // Driven: the right direction, wrong, or none, by coupling.
    let (mut right, mut wrong, mut none) = (0u64, 0u64, 0u64);
    let mut by_delay = [[0u64; 2]; 6];
    for i in 0..pairs {
        let c = r.between(0.3, 0.8);
        let d = 1 + r.below(5) as usize;
        let xs = driver(&mut r);
        let y = ar(&mut r, Some((&xs, c, d)));
        let x = xs[50..].to_vec();
        let first_is_x = r.below(2) == 0;
        let (a, b, truth) = if first_is_x {
            (&x, &y, Said::AtoB)
        } else {
            (&y, &x, Said::BtoA)
        };
        let said = assert_direction(a, b, seed * 1_000_003 + i);
        by_delay[d][1] += 1;
        if said == truth {
            right += 1;
            by_delay[d][0] += 1;
        } else if said == Said::Neither {
            none += 1;
        } else {
            wrong += 1;
        }
    }
    // Common driver: any direction asserted is against the bound.
    let (mut common_said, mut same_delay, mut same_said, mut common_both) =
        (0u64, 0u64, 0u64, 0u64);
    for i in 0..pairs {
        let z = driver(&mut r);
        let (d1, d2) = (r.below(6) as usize, r.below(6) as usize);
        let (c1, c2) = (r.between(0.3, 0.8), r.between(0.3, 0.8));
        // The drive is read at `t − d` over the warm-up as well, so pass the full driver.
        let x = ar(&mut r, Some((&z, c1, d1)));
        let y = ar(&mut r, Some((&z, c2, d2)));
        let (said, both) = assert_both(&x, &y, seed * 2_000_003 + i);
        let said = said != Said::Neither;
        common_said += u64::from(said);
        common_both += u64::from(both);
        if d1 == d2 {
            same_delay += 1;
            same_said += u64::from(said);
        }
    }
    let mut none_said = 0u64;
    for i in 0..pairs {
        let x = ar(&mut r, None);
        let y = ar(&mut r, None);
        none_said += u64::from(assert_direction(&x, &y, seed * 3_000_003 + i) != Said::Neither);
    }

    let pct = |a: u64, b: u64| 100.0 * a as f64 / b.max(1) as f64;
    println!(
        "GE-I5, RFC SMYSL-5 §15. Simulated pairs, {pairs} of each kind, seed {seed}: a day of \
         one-minute AR(1) points, rank:4, lags 1..5, {NULL} surrogates."
    );
    println!(
        "\ndriven        right {:.1} %, wrong {:.1} %, none asserted {:.1} %",
        pct(right, pairs),
        pct(wrong, pairs),
        pct(none, pairs)
    );
    let delays: Vec<String> = (1..=5)
        .map(|d| format!("{d} min {:.0} %", pct(by_delay[d][0], by_delay[d][1])))
        .collect();
    println!("              right by delay: {}", delays.join(", "));
    println!(
        "common driver a direction asserted in {:.1} % ({:.1} % of the {same_delay} with equal \
         delays)",
        pct(common_said, pairs),
        pct(same_said, same_delay)
    );
    println!(
        "              both ways significant in {:.1} %: with those read as no direction, \
         {:.1} % asserted",
        pct(common_both, pairs),
        pct(common_said - common_both, pairs)
    );
    println!(
        "none          a direction asserted in {:.1} %",
        pct(none_said, pairs)
    );
    let ok_driven = pct(right, pairs) >= 90.0;
    let ok_common = pct(common_said, pairs) <= 10.0;
    println!(
        "\nverdict: right in {:.1} % of driven pairs ({} 90 %); asserted in {:.1} % of \
         common-driver pairs ({} 10 %): te/1 {}",
        pct(right, pairs),
        if ok_driven { "at least" } else { "under" },
        pct(common_said, pairs),
        if ok_common { "at most" } else { "over" },
        if ok_driven && ok_common {
            "is released"
        } else {
            "stays reserved"
        }
    );
}
