//! GE-I2 (a) (RFC SMYSL-5 §15, TI-P1): is plug-in good enough at telemetry sizes?
//!
//! D-I6 chose a plug-in entropy over integer counts, with its Miller–Madow value stored beside
//! it, because a function of counts in a fixed order is replayable and a KSG or NSB estimator is
//! not. This asks whether that choice costs too much accuracy at the sample sizes telemetry has.
//!
//! **What this runs.** Simulated sources whose entropy is known exactly, and the report says so:
//!
//! - **i.i.d. symbols**: uniform over K = 2, 4, 8, 16, 32 and 64 (log₂ K bits), a geometric
//!   source over 8 symbols (p ∝ 2⁻ⁱ) and a Zipf source over 16 (p ∝ 1/i). These are `ent/1`'s
//!   counts once binned, whatever the binning.
//! - **Ordinal patterns of white noise**, m = 3 to 7, τ = 1: every pattern is equally likely, so
//!   `perm/1`'s true value is log₂ m!. Consecutive patterns overlap, so they are not i.i.d.,
//!   which is the case telemetry has.
//!
//! Each cell is `--runs` draws (default 200) at one `n`, with ChaCha20 seeded from `--seed`, and
//! reports the mean plug-in and the mean Miller–Madow value against the truth, as a share of it.
//! The estimators are `smysl_tel::info::entropy::plugin` and `symbols::lehmer`, the code that
//! writes units.
//!
//! **The stop condition** (§15): "if `mm` is more than 5 % off at the default `min_n`, the
//! default rises until it is not." The verdict lines check `ent/1`'s default of 200 against
//! every i.i.d. source, and `perm/1`'s default of 10·m! against each m.
//!
//! ```text
//! cargo run --release -p smysl-tel --example ge_i2 [-- --runs 200 --seed 7]
//! ```

use std::collections::BTreeMap;

use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};
use smysl_tel::info::entropy::plugin;
use smysl_tel::info::num::log2;
use smysl_tel::info::symbols::{factorial, lehmer};

/// The tolerance §15 gives.
const TOLERANCE: f64 = 0.05;

/// A discrete source: its cumulative distribution and its exact entropy.
struct Source {
    name: String,
    cdf: Vec<f64>,
    entropy: f64,
}

impl Source {
    fn weighted(name: &str, weights: &[f64]) -> Source {
        let total: f64 = weights.iter().sum();
        let p: Vec<f64> = weights.iter().map(|w| w / total).collect();
        let entropy = -p.iter().map(|q| q * log2(*q)).sum::<f64>();
        let mut acc = 0.0;
        let cdf = p
            .iter()
            .map(|q| {
                acc += q;
                acc
            })
            .collect();
        Source {
            name: name.to_string(),
            cdf,
            entropy,
        }
    }

    fn draw(&self, rng: &mut ChaCha20Rng) -> i64 {
        let u = unit(rng);
        self.cdf
            .iter()
            .position(|c| u < *c)
            .unwrap_or(self.cdf.len() - 1) as i64
    }
}

/// A uniform draw in [0, 1), 53 bits.
fn unit(rng: &mut ChaCha20Rng) -> f64 {
    (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64
}

/// The mean plug-in and Miller–Madow values over `runs` draws of `n` symbols from `draw`.
fn cell(runs: usize, n: usize, mut draw: impl FnMut() -> Vec<i64>) -> (f64, f64) {
    let (mut pi, mut mm) = (0.0, 0.0);
    for _ in 0..runs {
        let symbols = draw();
        debug_assert_eq!(symbols.len(), n);
        let mut counts = BTreeMap::new();
        for s in symbols {
            *counts.entry(s).or_insert(0u64) += 1;
        }
        let p = plugin(&counts).expect("n > 0");
        pi += p.value;
        mm += p.mm;
    }
    (pi / runs as f64, mm / runs as f64)
}

/// `n` ordinal patterns of length `m` over white noise: `n + m − 1` draws.
fn patterns(rng: &mut ChaCha20Rng, n: usize, m: usize) -> Vec<i64> {
    let noise: Vec<f64> = (0..n + m - 1).map(|_| unit(rng)).collect();
    (0..n)
        .map(|i| {
            let w = &noise[i..i + m];
            let mut order: Vec<usize> = (0..m).collect();
            order.sort_by(|&a, &b| w[a].total_cmp(&w[b]));
            lehmer(&order)
        })
        .collect()
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
    let runs = arg(&args, "--runs", 200) as usize;
    let seed = arg(&args, "--seed", 7);
    let mut rng = ChaCha20Rng::seed_from_u64(seed);
    const SIZES: &[usize] = &[100, 200, 500, 1_000, 10_000, 100_000];

    println!("GE-I2 (a), RFC SMYSL-5 §15. Simulated sources of known entropy; {runs} runs a cell, seed {seed}.");
    println!(
        "Each cell: mean plug-in and mean Miller-Madow, as a share of the truth (0 % is exact).\n"
    );

    let mut sources: Vec<Source> = [2usize, 4, 8, 16, 32, 64]
        .iter()
        .map(|&k| Source::weighted(&format!("uniform K={k}"), &vec![1.0; k]))
        .collect();
    let geometric: Vec<f64> = (0..8u32).map(|i| 1.0 / f64::from(1u32 << i)).collect();
    sources.push(Source::weighted("geometric K=8", &geometric));
    let zipf: Vec<f64> = (1..=16).map(|i| 1.0 / i as f64).collect();
    sources.push(Source::weighted("zipf K=16", &zipf));

    let pct = |x: f64, truth: f64| 100.0 * (x - truth) / truth;
    let mut verdicts = Vec::new();

    println!("i.i.d. symbols (ent/1's counts)");
    print!("{:<16} {:>7}", "source", "H bits");
    for n in SIZES {
        print!(" {:>17}", format!("n={n}"));
    }
    println!();
    for s in &sources {
        print!("{:<16} {:>7.4}", s.name, s.entropy);
        for &n in SIZES {
            let (pi, mm) = cell(runs, n, || (0..n).map(|_| s.draw(&mut rng)).collect());
            print!(" {:>8.2}%/{:>6.2}%", pct(pi, s.entropy), pct(mm, s.entropy));
            if n == 200 {
                verdicts.push((
                    format!("ent/1 min_n 200, {}", s.name),
                    pct(mm, s.entropy).abs() / 100.0,
                ));
            }
        }
        println!();
    }

    println!("\nordinal patterns of white noise (perm/1), H = log2 m!");
    print!("{:<16} {:>7}", "m", "H bits");
    for n in SIZES {
        print!(" {:>17}", format!("n={n}"));
    }
    println!(" {:>17}", "n=10·m!");
    for m in 3..=7usize {
        let truth = log2(factorial(m) as f64);
        print!("{:<16} {:>7.4}", format!("m={m}"), truth);
        for &n in SIZES {
            let r = if n >= 100_000 && m >= 6 {
                runs / 10
            } else {
                runs
            }
            .max(1);
            let (pi, mm) = cell(r, n, || patterns(&mut rng, n, m));
            print!(" {:>8.2}%/{:>6.2}%", pct(pi, truth), pct(mm, truth));
        }
        let n = 10 * factorial(m) as usize;
        let (pi, mm) = cell(runs, n, || patterns(&mut rng, n, m));
        println!(" {:>8.2}%/{:>6.2}%", pct(pi, truth), pct(mm, truth));
        verdicts.push((
            format!("perm/1 min_n {n}, m={m}"),
            pct(mm, truth).abs() / 100.0,
        ));
    }

    println!(
        "\nverdict (stop condition: Miller-Madow within {:.0} % at the default min_n)",
        TOLERANCE * 100.0
    );
    let mut all = true;
    for (what, off) in &verdicts {
        let ok = *off <= TOLERANCE;
        all &= ok;
        println!(
            "  {:<40} {:>6.2} % off  {}",
            what,
            off * 100.0,
            if ok { "holds" } else { "RAISE THE DEFAULT" }
        );
    }
    println!(
        "\n{}",
        if all {
            "GE-I2 (a): every default holds; min_n stays as §6.1 has it."
        } else {
            "GE-I2 (a): a default does not hold; §6.1's min_n rises for it."
        }
    );
}
