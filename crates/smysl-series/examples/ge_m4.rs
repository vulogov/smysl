//! GE-M4 (RFC SMYSL-3 §14, D-T6): is a hold record needed?
//!
//! Record 16 (`hold`) is reserved for "value v held from t1 to t2". The question GE-M4 puts is
//! whether the substrate already answers "value in force at t" — correctly, and in at most 50 ms
//! at the 95th percentile over six months of data. If it does, the record number is released.
//!
//! **What this runs.** GE-M1's data (a real cluster for seven days, extended to six months) is
//! not on this machine, so the data here is simulated, and says so in its report:
//!
//! - `--series` gauges, scraped every `--interval` seconds with up to 999 ms of jitter, for
//!   `--days` days, into a fresh `series.redb` under `--dir` (as `<dir>/index/series.redb`, so
//!   `smysl tel at -s <dir>` reads it).
//! - Each gauge holds its value for runs of scrapes and then changes (probability `--change`
//!   per scrape), with a stale marker now and then (a target gone for a while). That is the
//!   shape holds are for: a hold record would store one record per run.
//! - **The oracle is the holds.** Every run is also kept as a hold `(from, value)`, exactly as
//!   record 16 would carry it, and "in force at t" is a binary search over them.
//!
//! Then `--queries` instants, uniformly over the six months and over every series, plus every
//! hold's first instant and the millisecond before it, are asked of both. Any disagreement is a
//! failure; the substrate's latency per query is measured, and its percentiles reported.
//!
//! Exit 0 when every answer agrees and p95 ≤ 50 ms; 1 otherwise.
//!
//! `make ge-m4` runs it with the RFC's parameters.

use std::path::PathBuf;
use std::time::Instant;

use smysl_core::schemas::tel;
use smysl_core::{canonical_uid, Decimal, Uid};
use smysl_series::redb::RedbBackend;
use smysl_series::{flags, Sample, SeriesInfo, Substrate};

/// splitmix64: a fixed, seedable sequence, so a run is reproducible from its arguments.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, n)`.
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }
}

struct Args {
    dir: PathBuf,
    series: u64,
    days: u64,
    interval_s: u64,
    change: f64,
    queries: u64,
    seed: u64,
}

fn args() -> Args {
    let mut a = Args {
        dir: PathBuf::from("target/ge-m4"),
        series: 20,
        days: 182,
        interval_s: 60,
        change: 0.02,
        queries: 20_000,
        seed: 4,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let v = it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        let n = || v.parse::<u64>().unwrap_or_else(|_| panic!("{flag}: `{v}`"));
        match flag.as_str() {
            "--dir" => a.dir = PathBuf::from(&v),
            "--series" => a.series = n(),
            "--days" => a.days = n(),
            "--interval" => a.interval_s = n(),
            "--queries" => a.queries = n(),
            "--seed" => a.seed = n(),
            "--change" => a.change = v.parse().unwrap_or_else(|_| panic!("--change: `{v}`")),
            other => panic!("unknown argument {other}"),
        }
    }
    a
}

/// 2026-01-01T00:00:00Z.
const START: u64 = 1_767_225_600_000;
const DAY: u64 = 86_400_000;

/// One series: its uid, key, and holds `(from_ms, value or None for stale)`.
struct Truth {
    uid: Uid,
    key: String,
    holds: Vec<(u64, Option<Decimal>)>,
}

impl Truth {
    fn at(&self, t: u64) -> Option<Option<Decimal>> {
        let i = self.holds.partition_point(|(from, _)| *from <= t);
        (i > 0).then(|| self.holds[i - 1].1)
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[i]
}

fn main() {
    let a = args();
    let _ = std::fs::remove_dir_all(&a.dir);
    std::fs::create_dir_all(a.dir.join("index")).expect("the output directory");
    let path = a.dir.join("index").join("series.redb");
    let mut sub = Substrate::new(RedbBackend::open(&path).expect("a fresh series.redb"));

    let inst = canonical_uid(&tel::instrument("scraper", "prom-scrape:ge-m4/sim:9100").unwrap());
    let mut rng = Rng(a.seed);
    let interval = a.interval_s * 1000;
    let per_day = DAY / interval;
    let mut truth = Vec::new();
    let (mut samples, mut runs) = (0u64, 0u64);
    let written = Instant::now();
    for s in 0..a.series {
        let key = format!(r#"ge_m4_gauge{{instance="sim:9100",job="ge-m4",n="{s}"}}"#);
        let uid = canonical_uid(&tel::series("prom", &key).unwrap());
        let info = SeriesInfo {
            scheme: "prom".into(),
            key: key.clone(),
            class: "gauge".into(),
        };
        let mut holds: Vec<(u64, Option<Decimal>)> = Vec::new();
        let mut value = Some(Decimal::new(rng.below(10_000) as i128, -2).unwrap());
        let mut gone = 0u64;
        for day in 0..a.days {
            let mut batch = Vec::with_capacity(per_day as usize);
            for k in 0..per_day {
                let t = START + day * DAY + k * interval + rng.below(1000);
                if gone > 0 {
                    gone -= 1;
                    continue;
                }
                if rng.chance(a.change) {
                    value = Some(Decimal::new(rng.below(10_000) as i128, -2).unwrap());
                } else if rng.chance(0.0005) {
                    // The target goes away for up to an hour: one stale marker, then nothing.
                    value = None;
                    gone = rng.below(3_600_000 / interval) + 1;
                }
                let sample = match value {
                    Some(v) => Sample::new(t, None, v, 0).unwrap(),
                    None => Sample::stale(t),
                };
                if holds.last().is_none_or(|(_, v)| *v != value) {
                    holds.push((t, value));
                    runs += 1;
                }
                batch.push(sample);
                if value.is_none() {
                    // Back with a fresh value after the gap.
                    value = Some(Decimal::new(rng.below(10_000) as i128, -2).unwrap());
                }
            }
            samples += batch.len() as u64;
            sub.add(&uid, &inst, &info, &batch).expect("add");
        }
        truth.push(Truth { uid, key, holds });
    }
    let write_s = written.elapsed().as_secs_f64();
    drop(sub);
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

    // Reopened, as `tel at` opens it.
    let sub = Substrate::new(RedbBackend::open(&path).expect("reopen"));
    let end = START + a.days * DAY;
    let mut asks: Vec<(usize, u64)> = (0..a.queries)
        .map(|_| {
            (
                rng.below(a.series) as usize,
                START - DAY + rng.below(end - START + DAY),
            )
        })
        .collect();
    for (i, t) in truth.iter().enumerate() {
        for (from, _) in &t.holds {
            asks.push((i, *from));
            asks.push((i, from - 1));
        }
    }
    let mut lat = Vec::with_capacity(asks.len());
    let mut wrong = Vec::new();
    for (i, t) in &asks {
        let tr = &truth[*i];
        let q = Instant::now();
        let got = sub.at(&tr.uid, &inst, *t).expect("at");
        lat.push(q.elapsed().as_secs_f64() * 1000.0);
        let got = got.map(|s| (s.flags() & flags::STALE == 0).then(|| s.value_dec()));
        if got != tr.at(*t) {
            wrong.push((tr.key.clone(), *t, got, tr.at(*t)));
        }
    }
    let mut sorted = lat.clone();
    sorted.sort_by(f64::total_cmp);
    let (p50, p95, p99) = (
        percentile(&sorted, 0.50),
        percentile(&sorted, 0.95),
        percentile(&sorted, 0.99),
    );
    let max = sorted[sorted.len() - 1];

    println!("GE-M4 (RFC SMYSL-3 §14): value in force at t, substrate against holds");
    println!("data        SIMULATED: GE-M1's cluster data is not available here");
    println!(
        "shape       {} series x {} days, every {} s with jitter; change p {}; seed {}",
        a.series, a.days, a.interval_s, a.change, a.seed
    );
    println!(
        "substrate   {samples} samples in {} ({:.1} MB, {:.1} bytes/sample), written in {write_s:.1} s",
        path.display(),
        bytes as f64 / 1e6,
        bytes as f64 / samples as f64
    );
    println!(
        "holds       {runs} runs: a hold record per run would be {:.1} % of the samples",
        100.0 * runs as f64 / samples as f64
    );
    println!(
        "queries     {} ({} random, {} at and before each hold's start)",
        asks.len(),
        a.queries,
        asks.len() as u64 - a.queries
    );
    println!(
        "latency ms  p50 {p50:.3}  p95 {p95:.3}  p99 {p99:.3}  max {max:.3}  (threshold p95 50)"
    );
    println!("answers     {} disagree with the holds", wrong.len());
    for (k, t, got, want) in wrong.iter().take(5) {
        println!("  {k} at {t}: substrate {got:?}, holds {want:?}");
    }
    let pass = wrong.is_empty() && p95 <= 50.0;
    println!(
        "verdict     {}",
        if pass {
            "PASS: the substrate answers it; record 16 is not needed for this"
        } else {
            "FAIL"
        }
    );
    std::process::exit(if pass { 0 } else { 1 });
}
