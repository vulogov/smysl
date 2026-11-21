//! GE-I2 (b) (RFC SMYSL-5 §15, TI-P2): is `mi/1` sound on pairs of autocorrelated series, and
//! are circular-shift surrogates a usable null for them? With the scan cost §12 asks TI-P2 to
//! measure.
//!
//! **What this runs.** Simulated AR(1) pairs, and the report says so. Every number goes through
//! `smysl_tel::info::compute`, the code that writes units: the samples are `dec` values on a
//! one-minute grid, binned by `rank:k`, paired, and stored as §6.0 says.
//!
//! - **Correlated pairs** with known ρ at lag 0: `x = z₁`, `y = ρ·z₁ + √(1 − ρ²)·z₂`, `z₁` and `z₂`
//!   independent AR(1)s with one coefficient φ. The Gaussian mutual information
//!   `−½·log₂(1 − ρ²)` bounds the binned one from above (binning loses information), so the
//!   stored value less its `bias` sits at or below the bound when `bias` is the whole bias. It
//!   is the bias of i.i.d. pairs, so a cell over the bound shows how much autocorrelation adds;
//!   it is reported, and is not a stop condition.
//! - **Independent pairs**, φ from 0 to 0.99: `mi/1` at lag 0 and over the scan `-5..5`, each
//!   with `null` surrogates. A run is a **false alarm** when `null_ge / null_b ≤ 0.05`.
//!
//! **The stop condition** (§15): "if, for independent pairs, `null_ge / null_b ≤ 0.05` occurs in
//! more than 10 % of runs, circular-shift surrogates are not a usable null and D-I7 is
//! redesigned before TI-P2 ships."
//!
//! **Cost.** One `mi/1` with 200 surrogates over 21 lags of a day at one-minute steps (§12's
//! example), timed.
//!
//! ```text
//! cargo run --release -p smysl-tel --example ge_i2b [-- --runs 200 --null 100 --seed 7]
//! ```

// The simulator draws its Gaussians with the platform's `ln` and `cos`. They make the inputs,
// which are then written as `dec` values; nothing the estimator stores comes from them.
#![allow(clippy::disallowed_methods)]

use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};
use smysl_core::{Decimal, Uid};
use smysl_series::Sample;
use smysl_tel::info::{compute, num, Read, Spec, Stream};

const STEP: u64 = 60_000;
/// A day of minutes.
const N: usize = 1440;
/// §15's thresholds.
const ALPHA: f64 = 0.05;
const RATE: f64 = 0.10;

struct Sim(ChaCha20Rng);

impl Sim {
    fn unit(&mut self) -> f64 {
        ((self.0.next_u64() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }

    /// A standard normal, Box–Muller.
    fn normal(&mut self) -> f64 {
        let (u, v) = (self.unit(), self.unit());
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }

    /// A stationary AR(1) of unit variance: `x = φ·x + √(1 − φ²)·e`, started from its
    /// stationary law.
    fn ar1(&mut self, phi: f64, n: usize) -> Vec<f64> {
        let s = (1.0 - phi * phi).sqrt();
        let mut x = self.normal();
        (0..n)
            .map(|_| {
                x = phi * x + s * self.normal();
                x
            })
            .collect()
    }
}

/// `xs` as samples on the minute grid, each a `dec` of six places.
fn samples(xs: &[f64]) -> Vec<Sample> {
    xs.iter()
        .enumerate()
        .map(|(i, x)| {
            let d = Decimal::parse_token(&format!("{x:.6}")).expect("a finite value");
            Sample::new(i as u64 * STEP, None, d, 0).expect("a sample")
        })
        .collect()
}

fn read(samples: &[Sample], to_ms: u64) -> Read<'_> {
    Read {
        from_ms: 0,
        to_ms,
        samples,
        digest: [0; 32],
    }
}

/// `mi/1` of `x` against `y`, as `compute` gives it: (value, bias, null_ge, null_b).
fn mi(spec: &Spec, x: &[Sample], y: &[Sample]) -> Option<(f64, f64, u64, u64)> {
    let to = N as u64 * STEP;
    let stream = |n: u8| Stream {
        series: Uid::from_bytes([n; 32]),
        scheme: "sim",
        key: "ge-i2b",
        class: "gauge",
        instrument: Uid::from_bytes([9; 32]),
    };
    let (sa, sb) = (stream(1), stream(2));
    let (ra, rb) = (read(x, to), read(y, to));
    let c = compute(spec, &sa, &ra, None, Some((&sb, &rb))).ok()?;
    let f = |d: &Decimal| d.to_string().parse::<f64>().unwrap_or(f64::NAN);
    let (ge, b) = c.null.unwrap_or((0, 0));
    Some((f(&c.value), c.bias.as_ref().map_or(0.0, f), ge, b))
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
    let null = arg(&args, "--null", 100);
    let seed = arg(&args, "--seed", 7);
    let mut sim = Sim(ChaCha20Rng::seed_from_u64(seed));

    println!(
        "GE-I2 (b), RFC SMYSL-5 §15. Simulated AR(1) pairs, {N} one-minute points; {runs} runs a \
         cell, seed {seed}."
    );

    println!("\ncorrelated pairs at lag 0: mean stored value, its bias, and the Gaussian bound");
    println!(
        "{:<6} {:<5} {:>9} {:>9} {:>9} {:>9}  value - bias <= bound",
        "phi", "rho", "bins", "value", "bias", "bound"
    );
    let mut over = Vec::new();
    for phi in [0.0, 0.9] {
        for rho in [0.3f64, 0.6, 0.9] {
            for k in [4u32, 8] {
                let spec = Spec::parse(&format!(
                    "{{ stat: mi/1, step: {STEP}, bins: \"rank:{k}\" }}"
                ))
                .expect("a recipe");
                let (mut v, mut b) = (0.0, 0.0);
                for _ in 0..runs {
                    let z1 = sim.ar1(phi, N);
                    let z2 = sim.ar1(phi, N);
                    let s = (1.0 - rho * rho).sqrt();
                    let y: Vec<f64> = z1.iter().zip(&z2).map(|(a, c)| rho * a + s * c).collect();
                    let (value, bias, _, _) = mi(&spec, &samples(&z1), &samples(&y)).unwrap();
                    v += value;
                    b += bias;
                }
                let (v, b) = (v / runs as f64, b / runs as f64);
                let bound = -0.5 * num::log2(1.0 - rho * rho);
                let ok = v - b <= bound;
                if !ok {
                    over.push(format!("phi {phi}, rho {rho}, rank:{k}"));
                }
                println!(
                    "{phi:<6} {rho:<5} {:>9} {v:>9.4} {b:>9.4} {bound:>9.4}  {}",
                    format!("rank:{k}"),
                    if ok { "yes" } else { "over" }
                );
            }
        }
    }

    println!(
        "\nindependent pairs, rank:4, {null} surrogates: share of runs with null_ge/null_b <= {ALPHA}"
    );
    println!("{:<6} {:>10} {:>14}", "phi", "lag 0", "scan -5..5");
    let mut verdicts = Vec::new();
    let mut run_seed = 0u64;
    for phi in [0.0, 0.5, 0.9, 0.99] {
        let mut rates = Vec::new();
        for lag in ["0", "\"-5..5\""] {
            let mut alarms = 0usize;
            for _ in 0..runs {
                run_seed += 1;
                let spec = Spec::parse(&format!(
                    "{{ stat: mi/1, step: {STEP}, bins: \"rank:4\", lag: {lag}, null: {null}, \
                     seed: {run_seed} }}"
                ))
                .expect("a recipe");
                let (x, y) = (sim.ar1(phi, N), sim.ar1(phi, N));
                let (_, _, ge, b) = mi(&spec, &samples(&x), &samples(&y)).unwrap();
                if (ge as f64) <= ALPHA * b as f64 {
                    alarms += 1;
                }
            }
            let rate = alarms as f64 / runs as f64;
            rates.push(rate);
            verdicts.push((format!("phi {phi}, lag {}", lag.trim_matches('"')), rate));
        }
        println!(
            "{phi:<6} {:>9.1}% {:>13.1}%",
            rates[0] * 100.0,
            rates[1] * 100.0
        );
    }

    println!("\ncost: mi/1, rank:4 and width:0.25, lag -10..10, 200 surrogates, {N} points");
    let (x, y) = (sim.ar1(0.9, N), sim.ar1(0.9, N));
    let (x, y) = (samples(&x), samples(&y));
    for bins in ["rank:4", "width:0.25"] {
        let spec = Spec::parse(&format!(
            "{{ stat: mi/1, step: {STEP}, bins: \"{bins}\", lag: \"-10..10\", null: 200, \
             min_n: 80 }}"
        ))
        .expect("a recipe");
        let started = std::time::Instant::now();
        let reps = 5;
        for _ in 0..reps {
            mi(&spec, &x, &y).expect("a value");
        }
        let each = started.elapsed() / reps;
        println!(
            "  {bins:<11} {:>8.1} ms a unit ({} pair counts)",
            each.as_secs_f64() * 1e3,
            201 * 21
        );
    }

    println!("\nverdict");
    if over.is_empty() {
        println!("  value - bias is under the Gaussian bound in every cell");
    } else {
        println!(
            "  value - bias is over the Gaussian bound in {} cell(s), {}: the i.i.d. bias term \
             understates autocorrelated pairs' bias, which the surrogates carry",
            over.len(),
            over.join("; ")
        );
    }
    let mut all = true;
    for (what, rate) in &verdicts {
        let ok = *rate <= RATE;
        all &= ok;
        println!(
            "  {:<22} {:>5.1} % false alarms  {}",
            what,
            rate * 100.0,
            if ok { "holds" } else { "OVER 10 %" }
        );
    }
    println!(
        "\n{}",
        if all {
            "GE-I2 (b): circular-shift surrogates hold as a null; D-I7 stands."
        } else {
            "GE-I2 (b): a cell is over 10 %; D-I7 is redesigned before TI-P2 ships."
        }
    );
}
