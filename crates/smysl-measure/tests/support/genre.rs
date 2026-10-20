//! GE-T6's statistics (SMYSL-2.6 §5.4), shared by its two arms — `tests/ge_t6.rs` (T2, T3) and
//! `tests/ge_t6_a8.rs` (A8) — so that the arms cannot come to read the kill criterion two ways.
//!
//! The arms differ only in where a book's value and its noise come from: the pure arm's noise
//! is the window-sampling variance under seeds 0–9, A8's is the variance across extraction runs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use smysl_measure::rng::Rng;

pub const SHUFFLES: usize = 10_000;
pub const BOOTSTRAP: usize = 1_000;

pub fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// `fixtures/measure/genres.tsv`: USFM code to genre, `-` for a book §5.4 does not place.
pub fn genres() -> BTreeMap<String, String> {
    std::fs::read_to_string(repo("fixtures/measure/genres.tsv"))
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| {
            let (c, g) = l.split_once('\t').unwrap();
            (c.to_string(), g.to_string())
        })
        .collect()
}

/// The genres a label can take, in order; a genre's index is its place here.
pub fn genre_names(labels: &BTreeMap<String, String>) -> Vec<String> {
    let s: BTreeSet<&String> = labels.values().filter(|g| *g != "-").collect();
    s.into_iter().cloned().collect()
}

pub fn median(xs: &mut [f64]) -> f64 {
    xs.sort_by(f64::total_cmp);
    let n = xs.len();
    if n % 2 == 1 {
        xs[n / 2]
    } else {
        (xs[n / 2 - 1] + xs[n / 2]) / 2.0
    }
}

/// Each book's sample variance over its repetitions, averaged over books.
pub fn mean_variance(by_book: &[&[f64]]) -> f64 {
    by_book
        .iter()
        .map(|v| {
            let m = v.iter().sum::<f64>() / v.len() as f64;
            v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (v.len() - 1) as f64
        })
        .sum::<f64>()
        / by_book.len() as f64
}

/// Kruskal–Wallis H, ties corrected.
pub fn kruskal_wallis(xs: &[f64], groups: &[usize], k: usize) -> f64 {
    let n = xs.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| xs[a].total_cmp(&xs[b]));
    let mut ranks = vec![0.0; n];
    let mut ties = 0.0;
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && xs[order[j + 1]] == xs[order[i]] {
            j += 1;
        }
        let r = (i + j) as f64 / 2.0 + 1.0;
        for &o in &order[i..=j] {
            ranks[o] = r;
        }
        let t = (j - i + 1) as f64;
        ties += t * t * t - t;
        i = j + 1;
    }
    let (mut sum, mut cnt) = (vec![0.0; k], vec![0.0; k]);
    for (r, &g) in ranks.iter().zip(groups) {
        sum[g] += r;
        cnt[g] += 1.0;
    }
    let nf = n as f64;
    let mut h = 0.0;
    for g in 0..k {
        if cnt[g] > 0.0 {
            h += sum[g] * sum[g] / cnt[g];
        }
    }
    h = 12.0 / (nf * (nf + 1.0)) * h - 3.0 * (nf + 1.0);
    let c = 1.0 - ties / (nf * nf * nf - nf);
    if c > 0.0 {
        h / c
    } else {
        0.0
    }
}

pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    let k = ((p * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[k - 1]
}

/// One measure in one language: §5.4's statistic and its kill criterion.
#[derive(Debug)]
pub struct Analysis {
    pub n: usize,
    pub genres: usize,
    pub h: f64,
    pub p: f64,
    pub ms_between: f64,
    pub ms_within: f64,
    pub noise: f64,
    /// The between-genre mean square is not above the within-genre one plus the noise.
    pub against: bool,
    /// `(genre, median, 2.5%, 97.5%, books)` for each genre present.
    pub medians: Vec<(usize, f64, f64, f64, usize)>,
}

/// `x` the books' values, `g` their genres among `k`, `noise` the variance a book's value has
/// that is not the book's: window sampling for a pure measure, extraction for A8.
pub fn analyse(x: &[f64], g: &[usize], k: usize, noise: f64) -> Analysis {
    let present: BTreeSet<usize> = g.iter().copied().collect();
    let h = kruskal_wallis(x, g, k);
    let n = x.len();

    let mut rng = Rng::new(&smysl_core::hash_bytes(b"smysl/ge-t6/1\0shuffle"), 0);
    let mut perm = g.to_vec();
    let mut at_least = 0usize;
    for _ in 0..SHUFFLES {
        for i in (1..perm.len()).rev() {
            let j = rng.index(i + 1);
            perm.swap(i, j);
        }
        if kruskal_wallis(x, &perm, k) >= h - 1e-12 {
            at_least += 1;
        }
    }
    let p = (1 + at_least) as f64 / (1 + SHUFFLES) as f64;

    let of = |gi: usize| -> Vec<f64> {
        x.iter()
            .zip(g)
            .filter(|(_, &q)| q == gi)
            .map(|(v, _)| *v)
            .collect()
    };
    let mean = x.iter().sum::<f64>() / n as f64;
    let (mut ssb, mut ssw) = (0.0, 0.0);
    for &gi in &present {
        let xs = of(gi);
        let m = xs.iter().sum::<f64>() / xs.len() as f64;
        ssb += xs.len() as f64 * (m - mean) * (m - mean);
        ssw += xs.iter().map(|v| (v - m) * (v - m)).sum::<f64>();
    }
    let kk = present.len();
    let ms_between = ssb / (kk as f64 - 1.0);
    let ms_within = ssw / (n - kk) as f64;

    let mut boot = Rng::new(&smysl_core::hash_bytes(b"smysl/ge-t6/1\0bootstrap"), 0);
    let medians = present
        .iter()
        .map(|&gi| {
            let mut xs = of(gi);
            let med = median(&mut xs);
            let mut reps: Vec<f64> = (0..BOOTSTRAP)
                .map(|_| {
                    let mut r: Vec<f64> = (0..xs.len()).map(|_| xs[boot.index(xs.len())]).collect();
                    median(&mut r)
                })
                .collect();
            reps.sort_by(f64::total_cmp);
            (
                gi,
                med,
                percentile(&reps, 0.025),
                percentile(&reps, 0.975),
                xs.len(),
            )
        })
        .collect();
    Analysis {
        n,
        genres: kk,
        h,
        p,
        ms_between,
        ms_within,
        noise,
        against: ms_between <= ms_within + noise,
        medians,
    }
}

impl Analysis {
    /// The report's rows for one edition and measure; `noise_key` names the noise (`var:window`,
    /// `var:run`).
    pub fn rows(
        &self,
        edition: &str,
        id: &str,
        excluded: &[String],
        noise_key: &str,
        genre_names: &[String],
    ) -> String {
        let row = |key: &str, v: String| format!("{edition}\t{id}\t{key}\t{v}\n");
        let mut out = String::new();
        out.push_str(&row("books", self.n.to_string()));
        out.push_str(&row("genres", self.genres.to_string()));
        out.push_str(&row("excluded", excluded.join(" ")));
        out.push_str(&row("H", format!("{:.4}", self.h)));
        out.push_str(&row(
            "epsilon2",
            format!("{:.4}", self.h / (self.n as f64 - 1.0)),
        ));
        out.push_str(&row("p", format!("{:.4}", self.p)));
        out.push_str(&row("ms:between", format!("{:.6}", self.ms_between)));
        out.push_str(&row("ms:within", format!("{:.6}", self.ms_within)));
        out.push_str(&row(noise_key, format!("{:.6}", self.noise)));
        out.push_str(&row("against", self.against.to_string()));
        for (gi, med, lo, hi, n) in &self.medians {
            out.push_str(&row(
                &format!("median:{}", genre_names[*gi]),
                format!("{med:.4} [{lo:.4}, {hi:.4}] n={n}"),
            ));
        }
        out
    }
}

/// The closing rows: a measure counted against in 3 or more languages is dropped from defaults.
pub fn verdict(id: &str, against: usize, languages: usize) -> String {
    format!(
        "all\t{id}\tverdict\t{} ({against} of {languages} languages against)\n",
        if against >= 3 {
            "dropped from defaults"
        } else {
            "kept"
        },
    )
}
