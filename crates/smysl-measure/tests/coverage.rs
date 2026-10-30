//! §5.1's coverage row for T4: over 100 seeds, the bootstrap interval for α covers the α the
//! text was drawn with in at least 90. Slow, so `--ignored`, as the RFC says:
//!
//! ```text
//! cargo test -p smysl-measure --release --test coverage -- --ignored
//! ```
//!
//! Each seed is a fresh text of 3,000 types whose frequencies are drawn exactly from a discrete
//! power law with `f_min = 1`, and an interval of 200 replicates. The count of seeds covered is
//! printed, so a run records the figure and not only that it passed.

use smysl_graph::Store;
use smysl_measure::num::from_q20;
use smysl_measure::rng::Rng;
use smysl_measure::text::zipf_heaps::hurwitz_zeta;
use smysl_measure::text::{Corpus, Stream};
use smysl_measure::{run, Ci, Level, Num, Outcome, Recipe, Resolved};

fn uniform(rng: &mut Rng) -> f64 {
    (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64
}

/// As `tests/text.rs` draws it: inversion of `ζ(α, x) / ζ(α, 1)`, tokens shuffled, segments of
/// 500.
fn power_law_text(alpha: f64, types: usize, seed: u64) -> Corpus {
    let mut rng = Rng::new(&[4; 32], seed);
    let z1 = hurwitz_zeta(alpha, 1.0);
    let survival = |x: u64| hurwitz_zeta(alpha, x as f64) / z1;
    let mut tokens = Vec::new();
    for t in 0..types {
        let u = uniform(&mut rng);
        let mut hi = 2u64;
        while survival(hi) > u {
            hi *= 2;
        }
        let mut lo = hi / 2;
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if survival(mid) > u {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        for _ in 0..lo {
            tokens.push(format!("t{t}"));
        }
    }
    for i in (1..tokens.len()).rev() {
        tokens.swap(i, rng.index(i + 1));
    }
    let segs = tokens.chunks(500).map(|c| c.to_vec()).collect();
    Corpus::from_terms("smysl/an/1:surface", Stream::Surface, Level::Part, segs)
}

#[test]
#[ignore = "100 seeds × 2 exponents × 200 replicates; run in release"]
fn the_alpha_interval_covers_alpha_in_90_of_100_seeds() {
    let store = Store::new();
    let scope = Resolved::of_uids("synthetic", Default::default());
    for alpha in [2.0, 2.5] {
        let mut covered = 0;
        for seed in 0..100 {
            let c = power_law_text(alpha, 3000, 1000 + seed);
            let mut r = Recipe::new("", 0, "synthetic");
            r.boot_b = 200;
            r.min_n = Some(1000);
            let input = smysl_measure::Input::new(&store, &scope).with_text(&c);
            let res = run(&["T4"], &r, &input).remove(0);
            let Outcome::Value(v) = &res.outcome else {
                panic!("seed {seed}: {:?}", res.outcome);
            };
            let Some(Ci {
                lo: Num::Fixed(lo),
                hi: Num::Fixed(hi),
                ..
            }) = v.ci.get("alpha")
            else {
                panic!("seed {seed}: no interval");
            };
            if from_q20(*lo) <= alpha && alpha <= from_q20(*hi) {
                covered += 1;
            }
        }
        eprintln!("α = {alpha}: the interval covers α in {covered} of 100 seeds");
        assert!(covered >= 90, "α = {alpha}: {covered} of 100");
    }
}
