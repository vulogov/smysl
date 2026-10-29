//! SMYSL-2.6 OQ-89: what should T1's and Heaps' β's interval be, given that the segment
//! bootstrap's runs low for both?
//!
//! ```text
//! cargo test -p smysl-measure --release --test oq89 -- --ignored --nocapture
//! SMYSL_OQ89_VPL=<dir of *_vpl.txt> cargo test … (adds the real-text table)
//! ```
//!
//! A segment drawn twice adds its tokens twice and no new type, so a replicate has fewer types
//! than the text and both statistics run low in it; over the KJV's Ruth and Jude the 95%
//! percentile interval of T1 is [7.510, 7.726] around Ĥ = 7.813. OQ-89 lists three answers and
//! this file scores four intervals, on text whose truth is known, by **coverage**:
//!
//! - **percentile** — what M2 stores: the 2.5th and 97.5th replicates.
//! - **basic** — `[2θ̂ − q₉₇.₅, 2θ̂ − q₂.₅]`, which reflects the bootstrap's shift back.
//! - **half-sample** — the basic construction over replicates of `n/2` segments drawn
//!   *without* replacement, so no segment is duplicated. At `m = n/2` the finite-population
//!   factor makes the replicate spread the full sample's, so no rescaling is needed.
//! - **jackknife** — twenty groups of segments, by index mod 20 so each spans the text in reading
//!   order; `θ̂ ± 1.96·se`. Symmetric about θ̂ by construction.
//!
//! The text is drawn from a Zipf law over 5,000 types (exponent 1.1), in segments of 25 tokens,
//! either i.i.d. or **bursty**: with probability 0.3 a token comes from its segment's own topic
//! of 20 types chosen uniformly, which makes the marginal `0.7·zipf + 0.3·uniform` exactly, so
//! the source entropy is known either way. T1's target is that entropy, for Ĥ and for
//! Miller–Madow's Ĥ_MM. β's target is β̂'s own mean at that length — Heaps' law is a description
//! of a reading order, and its population value is the one the same process gives on average —
//! taken over 400 fresh texts per row.

use smysl_measure::num;
use smysl_measure::rng::Rng;
use smysl_measure::text::entropy::entropy;
use smysl_measure::text::zipf_heaps::heaps;

const TYPES: usize = 5_000;
const S: f64 = 1.1;
const SEG: usize = 25;
const BURST: f64 = 0.3;
const TOPIC: usize = 20;
const B: usize = 200;
const G: usize = 20;
const Z: f64 = 1.959_963_984_540_054;

fn uniform(rng: &mut Rng) -> f64 {
    (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64
}

struct Source {
    cdf: Vec<f64>,
    bursty: bool,
}

impl Source {
    fn new(bursty: bool) -> Source {
        let w: Vec<f64> = (1..=TYPES).map(|r| num::powf(r as f64, -S)).collect();
        let total: f64 = w.iter().sum();
        let mut acc = 0.0;
        let cdf = w
            .iter()
            .map(|x| {
                acc += x / total;
                acc
            })
            .collect();
        Source { cdf, bursty }
    }

    /// The marginal's entropy in bits.
    fn entropy(&self) -> f64 {
        let mut prev = 0.0;
        let mut h = 0.0;
        for &c in &self.cdf {
            let z = c - prev;
            prev = c;
            let p = if self.bursty {
                (1.0 - BURST) * z + BURST / TYPES as f64
            } else {
                z
            };
            h -= p * num::log2(p);
        }
        h
    }

    fn zipf(&self, rng: &mut Rng) -> u32 {
        let u = uniform(rng);
        self.cdf.partition_point(|&c| c < u).min(TYPES - 1) as u32
    }

    fn text(&self, tokens: usize, rng: &mut Rng) -> Vec<Vec<u32>> {
        (0..tokens / SEG)
            .map(|_| {
                let topic: Vec<u32> = (0..TOPIC).map(|_| rng.index(TYPES) as u32).collect();
                (0..SEG)
                    .map(|_| {
                        if self.bursty && uniform(rng) < BURST {
                            topic[rng.index(TOPIC)]
                        } else {
                            self.zipf(rng)
                        }
                    })
                    .collect()
            })
            .collect()
    }
}

/// The three statistics over the segments `at`, in that order.
fn stats(segs: &[Vec<u32>], at: &[usize], types: usize) -> Option<[f64; 3]> {
    let mut counts = vec![0u64; types];
    let mut n = 0u64;
    for &i in at {
        for &t in &segs[i] {
            counts[t as usize] += 1;
            n += 1;
        }
    }
    let (h, mm) = entropy(&counts)?;
    let (beta, _) = heaps(at.iter().flat_map(|&i| segs[i].iter()), n, types)?;
    Some([h, mm, beta])
}

fn quantiles(v: &mut [f64]) -> (f64, f64) {
    v.sort_by(f64::total_cmp);
    let (lo, hi) = smysl_measure::boot::ranks(v.len(), (95, 100));
    (v[lo - 1], v[hi - 1])
}

/// The four intervals for each statistic: `[stat][method] = (lo, hi)`.
type Intervals = [[(f64, f64); 5]; 3];

fn intervals(segs: &[Vec<u32>], types: usize, rng: &mut Rng) -> Option<([f64; 3], Intervals)> {
    let n = segs.len();
    let all: Vec<usize> = (0..n).collect();
    let est = stats(segs, &all, types)?;
    let mut boot: [Vec<f64>; 3] = Default::default();
    let mut half: [Vec<f64>; 3] = Default::default();
    let mut drawn = Vec::new();
    for _ in 0..B {
        drawn.clear();
        drawn.extend((0..n).map(|_| rng.index(n)));
        if let Some(s) = stats(segs, &drawn, types) {
            for k in 0..3 {
                boot[k].push(s[k]);
            }
        }
        // n/2 without replacement, kept in reading order.
        let mut pool = all.clone();
        for i in 0..n / 2 {
            let j = i + rng.index(n - i);
            pool.swap(i, j);
        }
        let mut pick = pool[..n / 2].to_vec();
        pick.sort_unstable();
        if let Some(s) = stats(segs, &pick, types) {
            for k in 0..3 {
                half[k].push(s[k]);
            }
        }
    }
    let mut jack: [Vec<f64>; 3] = Default::default();
    for g in 0..G {
        let keep: Vec<usize> = all.iter().copied().filter(|i| i % G != g).collect();
        let s = stats(segs, &keep, types)?;
        for k in 0..3 {
            jack[k].push(s[k]);
        }
    }
    let mut out: Intervals = [[(0.0, 0.0); 5]; 3];
    for k in 0..3 {
        let t = est[k];
        let (lo, hi) = quantiles(&mut boot[k]);
        let (hlo, hhi) = quantiles(&mut half[k]);
        let mean = jack[k].iter().sum::<f64>() / G as f64;
        let var = jack[k].iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() * (G - 1) as f64
            / G as f64;
        let se = var.sqrt();
        // The grouped jackknife's bias-corrected estimate (Quenouille; Zahl 1977 for entropy).
        let bc = G as f64 * t - (G - 1) as f64 * mean;
        out[k] = [
            (lo, hi),
            (2.0 * t - hi, 2.0 * t - lo),
            (2.0 * t - hhi, 2.0 * t - hlo),
            (t - Z * se, t + Z * se),
            (bc - Z * se, bc + Z * se),
        ];
    }
    Some((est, out))
}

const NAMES: [&str; 3] = ["H", "H:mm", "beta"];
const METHODS: [&str; 5] = [
    "percentile",
    "basic",
    "half-sample",
    "jackknife",
    "jackknife-bc",
];

#[test]
#[ignore = "thousands of texts × 200 replicates; run in release"]
fn coverage_of_four_intervals_on_text_whose_truth_is_known() {
    for bursty in [false, true] {
        let src = Source::new(bursty);
        let h_true = src.entropy();
        for (tokens, seeds) in [(3_000usize, 200u64), (30_000, 100), (300_000, 40)] {
            // β's target: its mean over fresh texts of this length.
            let mut rng = Rng::new(&[89; 32], tokens as u64 + u64::from(bursty));
            let mut sum = 0.0;
            for _ in 0..400 {
                let segs = src.text(tokens, &mut rng);
                let all: Vec<usize> = (0..segs.len()).collect();
                sum += stats(&segs, &all, TYPES).expect("a text")[2];
            }
            let beta_true = sum / 400.0;
            let truth = [h_true, h_true, beta_true];

            let mut covered = [[0usize; 5]; 3];
            let mut off = [0usize; 3];
            let mut width = [[0.0f64; 5]; 3];
            let mut bias = [0.0f64; 3];
            for seed in 0..seeds {
                let mut rng = Rng::new(&[90; 32], seed * 1_000_003 + tokens as u64);
                let segs = src.text(tokens, &mut rng);
                let (est, iv) = intervals(&segs, TYPES, &mut rng).expect("intervals");
                for k in 0..3 {
                    bias[k] += est[k] - truth[k];
                    let (lo, hi) = iv[k][0];
                    if est[k] < lo || est[k] > hi {
                        off[k] += 1;
                    }
                    for m in 0..5 {
                        let (lo, hi) = iv[k][m];
                        width[k][m] += hi - lo;
                        if lo <= truth[k] && truth[k] <= hi {
                            covered[k][m] += 1;
                        }
                    }
                }
            }
            println!(
                "\n{} · {tokens} tokens · {seeds} seeds · H = {h_true:.4}, E[β̂] = {beta_true:.4}",
                if bursty { "bursty" } else { "iid" }
            );
            for k in 0..3 {
                let cells: Vec<String> = (0..5)
                    .map(|m| {
                        format!(
                            "{} {:>5.1}% w {:.4}",
                            METHODS[m],
                            100.0 * covered[k][m] as f64 / seeds as f64,
                            width[k][m] / seeds as f64
                        )
                    })
                    .collect();
                println!(
                    "  {:<5} bias {:+.4}  off-estimate {:>3}  {}",
                    NAMES[k],
                    bias[k] / seeds as f64,
                    off[k],
                    cells.join(" · ")
                );
            }
        }
    }
}

/// The same four intervals over real text: whether the percentile interval excludes the
/// estimate, and by how much, as a Bible's prefix grows. No truth here, so no coverage.
#[test]
#[ignore = "needs SMYSL_OQ89_VPL; run in release"]
fn the_shift_on_real_text_as_it_grows() {
    let Some(dir) = std::env::var_os("SMYSL_OQ89_VPL") else {
        eprintln!("skipped: set SMYSL_OQ89_VPL to a directory of <edition>_vpl.txt files");
        return;
    };
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("the directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with("_vpl.txt"))
        .collect();
    files.sort();
    for path in files {
        let src = std::fs::read_to_string(&path).expect("the text");
        let mut ids = std::collections::HashMap::new();
        let segs: Vec<Vec<u32>> = src
            .lines()
            .filter_map(|l| l.splitn(3, ' ').nth(2))
            .map(|v| {
                v.split(|c: char| !c.is_alphanumeric())
                    .filter(|w| !w.is_empty())
                    .map(|w| {
                        let n = ids.len() as u32;
                        *ids.entry(w.to_lowercase()).or_insert(n)
                    })
                    .collect()
            })
            .collect();
        let types = ids.len();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        println!("\n{name}: {} verses, {types} types", segs.len());
        for verses in [120usize, 1_200, 12_000, segs.len()] {
            let part = &segs[..verses.min(segs.len())];
            let mut rng = Rng::new(&[91; 32], verses as u64);
            let (est, iv) = intervals(part, types, &mut rng).expect("intervals");
            let tokens: usize = part.iter().map(Vec::len).sum();
            println!("  {} verses, {tokens} tokens", part.len());
            for k in 0..3 {
                let cells: Vec<String> = (0..5)
                    .map(|m| format!("{} [{:.4}, {:.4}]", METHODS[m], iv[k][m].0, iv[k][m].1))
                    .collect();
                let (lo, hi) = iv[k][0];
                println!(
                    "    {:<5} {:.4} {} {}",
                    NAMES[k],
                    est[k],
                    if est[k] < lo || est[k] > hi {
                        "OFF"
                    } else {
                        "in "
                    },
                    cells.join(" · ")
                );
            }
        }
    }
}
