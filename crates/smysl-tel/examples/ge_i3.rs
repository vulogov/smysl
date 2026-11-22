//! GE-I3 (RFC SMYSL-5 §15, TI-P4): does an information measure find anything the shipped
//! detectors miss?
//!
//! **What this runs.** No incident data with these faults is on this machine, so the incidents
//! are simulated, and the report says so. Each incident is a series at one sample a minute: a
//! two-hour **reference window**, then a one-hour **test window**, Gaussian noise of width σ about
//! a level. In a **normal** incident the test window is the reference's process; in a faulty one
//! it is one of:
//!
//! - **level** (GE-M6's fault): the level shifts by 3 to 10 σ at the test window's start;
//! - **variance**: σ grows 2 to 4 times, the median unmoved;
//! - **oscillation**: the noise becomes a sine of period 8 to 20 minutes with a little noise on
//!   it, of about the same variance, the median unmoved;
//! - **decouple**: a pair, `y` following `x` (correlation 0.9) in the reference and independent
//!   of it in the test window, each series' own distribution unmoved. Normal pairs stay coupled;
//! - **mix** (GE-I4, R4-4): a log stream of eight templates at ten events a minute whose shares
//!   are permuted among the templates, so its template entropy is the same in expectation.
//!
//! The level shift is the shipped detectors' own ground, and is here to show they find it. The
//! three after it are the RFC's "three kinds no level detector sees", and decide.
//!
//! **Arms**, each by the shipped code (`smysl_tel::detect`, `smysl_tel::info`):
//!
//! - `mad/1`, the reference window as its baseline: flags the test window if it writes an
//!   anomaly in it. `cusum/1` and `pelt/1` with their defaults over the three hours: flag it if a
//!   change point falls in it. A pair is flagged if either series is.
//! - `jsd/1`: the test window against the reference, binned at the reference's quartiles
//!   (`baseline:4`). Its score is the value.
//! - `perm/1` (m = 3): the score is |test − reference|.
//! - `mi/1` (rank:4, lag 0), on pairs: the score is the reference's value less the test's.
//! - `tent/1`, on the log stream: the score is |test − reference|.
//!
//! **The stop condition** (§15): "if no statistic separates faulty from normal windows on any of
//! the three kinds at the false-alarm rate MAD has on the same data, the statistics stay as
//! described here, a tool for study." Read here as fixed before the run:
//!
//! - MAD's false-alarm rate is the share of the kind's **normal** incidents it flags: single
//!   series for level, variance and oscillation; pairs for decouple. For `mix`, which MAD cannot
//!   read, the single-series rate.
//! - A statistic's threshold is the least score that no more than that share of the same normal
//!   incidents exceed; its **detection rate** is the share of the kind's faulty incidents that
//!   exceed it.
//! - A statistic **separates** a kind when that rate is at least one half.
//! - The stop condition holds, and the statistics stay a tool for study, when no statistic
//!   separates variance, oscillation or decouple.
//!
//! ```text
//! cargo run --release -p smysl-tel --example ge_i3 [-- --incidents 400 --seed 7]
//! ```

// Simulated input, not a stored number: the generators draw Box–Muller normals with the
// platform's `ln`, `sqrt`, `sin` and `cos`. Every number the arms compute is the shipped code's.
#![allow(clippy::disallowed_methods)]

use std::collections::BTreeMap;

use smysl_core::{Decimal, Uid};
use smysl_series::Sample;
use smysl_tel::detect::{cusum, mad, pelt, Input};
use smysl_tel::info::{self, logs, Read, Spec, Stream};

const MIN: u64 = 60_000;
/// 2026-01-16T08:00:00Z: the reference window's start.
const T0: u64 = 1_768_550_400_000;
const REF: u64 = 120;
const TEST: u64 = 60;
const LEVEL: f64 = 50.0;

/// splitmix64, with Box–Muller normals.
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
        let v = self.unit();
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Normal,
    Level,
    Variance,
    Oscillation,
    Decouple,
    Mix,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Normal => "normal",
            Kind::Level => "level",
            Kind::Variance => "variance",
            Kind::Oscillation => "oscillation",
            Kind::Decouple => "decouple",
            Kind::Mix => "mix",
        }
    }
}

/// One series' 180 values: the reference's process, then the test window's.
fn single(r: &mut Rng, kind: Kind) -> Vec<f64> {
    let mut xs: Vec<f64> = (0..REF).map(|_| LEVEL + r.normal()).collect();
    match kind {
        Kind::Level => {
            let shift = r.between(3.0, 10.0) * if r.below(2) == 0 { 1.0 } else { -1.0 };
            xs.extend((0..TEST).map(|_| LEVEL + shift + r.normal()));
        }
        Kind::Variance => {
            let s = r.between(2.0, 4.0);
            xs.extend((0..TEST).map(|_| LEVEL + s * r.normal()));
        }
        Kind::Oscillation => {
            let period = r.between(8.0, 20.0);
            let phase = r.between(0.0, 2.0 * std::f64::consts::PI);
            // Variance A²/2 + 0.3² = 1, so that the spread is the reference's.
            let a = (2.0f64 * (1.0 - 0.09)).sqrt();
            xs.extend((0..TEST).map(|i| {
                let t = 2.0 * std::f64::consts::PI * i as f64 / period + phase;
                LEVEL + a * t.sin() + 0.3 * r.normal()
            }));
        }
        _ => xs.extend((0..TEST).map(|_| LEVEL + r.normal())),
    }
    xs
}

/// A pair: `y` follows `x` at correlation 0.9, through the test window unless it decouples.
fn pair(r: &mut Rng, decouple: bool) -> (Vec<f64>, Vec<f64>) {
    let c: f64 = 0.9;
    let rest = (1.0 - c * c).sqrt();
    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    for i in 0..REF + TEST {
        let x = r.normal();
        let y = if decouple && i >= REF {
            r.normal()
        } else {
            c * x + rest * r.normal()
        };
        xs.push(LEVEL + x);
        ys.push(LEVEL + y);
    }
    (xs, ys)
}

/// A log stream's template counts in the reference window and in the test window.
fn logs_of(r: &mut Rng, mix: bool) -> (BTreeMap<String, u64>, BTreeMap<String, u64>) {
    // Eight templates, shares 30 %, 20 %, 15 %, 10 %, 10 %, 7 %, 5 %, 3 %.
    let shares = [30u64, 20, 15, 10, 10, 7, 5, 3];
    let mut order: Vec<usize> = (0..8).collect();
    if mix {
        // A permutation that moves the commonest template: the mix changes, its entropy not.
        while order[0] == 0 {
            for i in (1..8).rev() {
                order.swap(i, r.below(i as u64 + 1) as usize);
            }
        }
    }
    let draw = |r: &mut Rng, n: u64, order: &[usize]| {
        let mut c: BTreeMap<String, u64> = BTreeMap::new();
        for _ in 0..n {
            let mut p = r.below(100);
            let mut t = 0;
            while p >= shares[t] {
                p -= shares[t];
                t += 1;
            }
            *c.entry(format!("template {}", order[t])).or_insert(0) += 1;
        }
        c
    };
    let base: Vec<usize> = (0..8).collect();
    (draw(r, 10 * REF, &base), draw(r, 10 * TEST, &order))
}

fn samples(xs: &[f64]) -> Vec<Sample> {
    xs.iter()
        .enumerate()
        .map(|(i, x)| {
            let v = Decimal::parse_token(&format!("{x:.3}")).expect("a decimal");
            Sample::new(T0 + i as u64 * MIN, None, v, 0).expect("a sample")
        })
        .collect()
}

fn stream(series: u8) -> Stream<'static> {
    Stream {
        series: Uid::from_bytes([series; 32]),
        scheme: "prom",
        key: "ge-i3",
        class: "gauge",
        instrument: Uid::from_bytes([9; 32]),
    }
}

const TEST_FROM: u64 = T0 + REF * MIN;
const TEST_TO: u64 = T0 + (REF + TEST) * MIN;

fn read(s: &[Sample], from: u64, to: u64) -> Read<'_> {
    let lo = s.partition_point(|x| x.ts_ms() < from);
    let hi = s.partition_point(|x| x.ts_ms() < to);
    Read {
        from_ms: from,
        to_ms: to,
        samples: &s[lo..hi],
        digest: [0; 32],
    }
}

fn value(c: Result<info::Computed, info::Refusal>, what: &str) -> f64 {
    let c = c.unwrap_or_else(|r| panic!("{what}: {r}"));
    c.value.to_string().parse().expect("a decimal")
}

/// Whether the shipped detectors flag the test window: (MAD, CUSUM, PELT).
fn detectors(xs: &[f64]) -> [bool; 3] {
    let s = samples(xs);
    let input = Input {
        series: Uid::from_bytes([1; 32]),
        scheme: "prom",
        key: "ge-i3",
        instrument: Uid::from_bytes([9; 32]),
        samples: &s,
    };
    let mad = !mad::run(
        &input,
        &s,
        TEST_FROM,
        TEST_TO,
        &mad::Params::new(T0, TEST_FROM),
    )
    .is_empty();
    let points = input.points();
    let cusum = cusum::change_points(&points, &cusum::Params::default())
        .iter()
        .any(|&i| i as u64 >= REF);
    let values: Vec<f64> = points.iter().map(|p| p.1).collect();
    let pelt = pelt::change_points(&values, &pelt::Params::default())
        .iter()
        .any(|&i| i as u64 >= REF);
    [mad, cusum, pelt]
}

struct Specs {
    jsd: Spec,
    perm: Spec,
    mi: Spec,
    tent: Spec,
}

/// `jsd/1` and the `perm/1` difference of one series.
fn single_scores(sp: &Specs, xs: &[f64]) -> (f64, f64) {
    let s = samples(xs);
    let st = stream(1);
    let (r, t) = (read(&s, T0, TEST_FROM), read(&s, TEST_FROM, TEST_TO));
    let jsd = value(info::compute(&sp.jsd, &st, &t, Some(&r), None), "jsd/1");
    let pr = value(info::compute(&sp.perm, &st, &r, None, None), "perm/1");
    let pt = value(info::compute(&sp.perm, &st, &t, None, None), "perm/1");
    (jsd, (pt - pr).abs())
}

/// The fall of `mi/1` from the reference window to the test window.
fn mi_drop(sp: &Specs, xs: &[f64], ys: &[f64]) -> f64 {
    let (a, b) = (samples(xs), samples(ys));
    let (sa, sb) = (stream(1), stream(2));
    let mi = |from, to| {
        let (ra, rb) = (read(&a, from, to), read(&b, from, to));
        value(
            info::compute(&sp.mi, &sa, &ra, None, Some((&sb, &rb))),
            "mi/1",
        )
    };
    mi(T0, TEST_FROM) - mi(TEST_FROM, TEST_TO)
}

fn tent_move(sp: &Specs, reference: BTreeMap<String, u64>, test: BTreeMap<String, u64>) -> f64 {
    let st = Stream {
        class: "log",
        ..stream(3)
    };
    let t = |counts, from, to| {
        let ts = logs::Templates {
            instrument: st.instrument,
            digest: [0; 32],
            counts,
        };
        value(logs::compute(&sp.tent, &st, from, to, &ts), "tent/1")
    };
    (t(test, TEST_FROM, TEST_TO) - t(reference, T0, TEST_FROM)).abs()
}

/// What one incident gives: the detectors' flags, and each statistic's score where it applies.
#[derive(Default, Clone)]
struct Row {
    flags: [bool; 3],
    jsd: Option<f64>,
    perm: Option<f64>,
    mi: Option<f64>,
    tent: Option<f64>,
}

/// The least threshold that at most `rate` of `normal` scores exceed.
fn threshold(normal: &[f64], rate: f64) -> f64 {
    let mut xs = normal.to_vec();
    xs.sort_by(f64::total_cmp);
    let allowed = (rate * xs.len() as f64).floor() as usize;
    // The score at rank n − allowed − 1 from the bottom: `allowed` scores lie above it.
    xs[xs.len() - allowed - 1]
}

fn share(xs: impl Iterator<Item = bool>) -> f64 {
    let (mut n, mut k) = (0usize, 0usize);
    for x in xs {
        n += 1;
        k += usize::from(x);
    }
    k as f64 / n.max(1) as f64
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
    let n = arg(&args, "--incidents", 400);
    let seed = arg(&args, "--seed", 7);
    let mut r = Rng(seed);
    let sp = Specs {
        jsd: Spec::parse(&format!(
            r#"{{ stat: jsd/1, step: 60000, bins: "baseline:4", ref: "{T0}-{TEST_FROM}",
                 min_n: 50 }}"#
        ))
        .expect("a recipe"),
        perm: Spec::parse("{ stat: perm/1, step: 60000, m: 3, min_n: 50 }").expect("a recipe"),
        mi: Spec::parse(r#"{ stat: mi/1, step: 60000, bins: "rank:4", min_n: 50 }"#)
            .expect("a recipe"),
        tent: Spec::parse("{ stat: tent/1 }").expect("a recipe"),
    };

    // Per kind, its incidents; normal ones are single series, pairs and log streams.
    let mut singles: BTreeMap<Kind, Vec<Row>> = BTreeMap::new();
    let mut pairs: BTreeMap<bool, Vec<Row>> = BTreeMap::new();
    let mut streams: BTreeMap<bool, Vec<Row>> = BTreeMap::new();
    for kind in [Kind::Normal, Kind::Level, Kind::Variance, Kind::Oscillation] {
        for _ in 0..n {
            let xs = single(&mut r, kind);
            let (jsd, perm) = single_scores(&sp, &xs);
            singles.entry(kind).or_default().push(Row {
                flags: detectors(&xs),
                jsd: Some(jsd),
                perm: Some(perm),
                ..Row::default()
            });
        }
    }
    for decouple in [false, true] {
        for _ in 0..n {
            let (xs, ys) = pair(&mut r, decouple);
            let (fx, fy) = (detectors(&xs), detectors(&ys));
            let (jsd, perm) = single_scores(&sp, &ys);
            pairs.entry(decouple).or_default().push(Row {
                flags: [fx[0] || fy[0], fx[1] || fy[1], fx[2] || fy[2]],
                jsd: Some(jsd),
                perm: Some(perm),
                mi: Some(mi_drop(&sp, &xs, &ys)),
                tent: None,
            });
        }
    }
    for mix in [false, true] {
        for _ in 0..n {
            let (a, b) = logs_of(&mut r, mix);
            streams.entry(mix).or_default().push(Row {
                tent: Some(tent_move(&sp, a, b)),
                ..Row::default()
            });
        }
    }

    println!(
        "GE-I3, RFC SMYSL-5 §15. Simulated incidents, {n} of each kind, seed {seed}: a two-hour \
         reference window and a one-hour test window at one sample a minute."
    );
    println!(
        "Detectors flag the test window as shipped; a statistic at the threshold that holds its \
         false alarms on the kind's normal incidents to MAD's.\n"
    );
    println!(
        "{:<12} {:>7} {:>7} {:>7} {:>7} | {:>7} {:>7} {:>7} {:>7}",
        "kind", "MAD FA", "MAD", "CUSUM", "PELT", "jsd/1", "perm/1", "mi/1", "tent/1"
    );
    let single_normal = &singles[&Kind::Normal];
    let mad_fa_single = share(single_normal.iter().map(|r| r.flags[0]));
    let mut separated: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut report = |kind: Kind, normal: &[Row], faulty: &[Row], fa: f64| {
        let rate = |get: &dyn Fn(&Row) -> Option<f64>| -> Option<f64> {
            let ns: Vec<f64> = normal.iter().filter_map(get).collect();
            if ns.is_empty() {
                return None;
            }
            let t = threshold(&ns, fa);
            Some(share(faulty.iter().map(|r| get(r).is_some_and(|s| s > t))))
        };
        let cells = [
            ("jsd/1", rate(&|r| r.jsd)),
            ("perm/1", rate(&|r| r.perm)),
            ("mi/1", rate(&|r| r.mi)),
            ("tent/1", rate(&|r| r.tent)),
        ];
        let cell = |x: Option<f64>| x.map_or("-".to_string(), |v| format!("{:.1} %", 100.0 * v));
        let det = |i: usize| {
            if faulty.iter().any(|r| r.tent.is_some()) {
                "-".to_string()
            } else {
                cell(Some(share(faulty.iter().map(|r| r.flags[i]))))
            }
        };
        println!(
            "{:<12} {:>7} {:>7} {:>7} {:>7} | {:>7} {:>7} {:>7} {:>7}",
            kind.name(),
            format!("{:.1} %", 100.0 * fa),
            det(0),
            det(1),
            det(2),
            cell(cells[0].1),
            cell(cells[1].1),
            cell(cells[2].1),
            cell(cells[3].1)
        );
        for (name, v) in cells {
            if v.is_some_and(|v| v >= 0.5) {
                separated.entry(kind.name()).or_default().push(name);
            }
        }
    };
    for kind in [Kind::Level, Kind::Variance, Kind::Oscillation] {
        report(kind, single_normal, &singles[&kind], mad_fa_single);
    }
    let mad_fa_pairs = share(pairs[&false].iter().map(|r| r.flags[0]));
    report(Kind::Decouple, &pairs[&false], &pairs[&true], mad_fa_pairs);
    report(Kind::Mix, &streams[&false], &streams[&true], mad_fa_single);
    let normal_flags = |rows: &[Row], i: usize| share(rows.iter().map(|r| r.flags[i]));
    println!(
        "\nfalse alarms on normal incidents: single series MAD {:.1} %, CUSUM {:.1} %, PELT \
         {:.1} %; pairs MAD {:.1} %, CUSUM {:.1} %, PELT {:.1} %",
        100.0 * mad_fa_single,
        100.0 * normal_flags(single_normal, 1),
        100.0 * normal_flags(single_normal, 2),
        100.0 * mad_fa_pairs,
        100.0 * normal_flags(&pairs[&false], 1),
        100.0 * normal_flags(&pairs[&false], 2)
    );

    let decisive = ["variance", "oscillation", "decouple"];
    let found: Vec<String> = decisive
        .iter()
        .filter_map(|k| {
            separated
                .get(k)
                .map(|by| format!("{k} by {}", by.join(", ")))
        })
        .collect();
    println!(
        "\nverdict (a statistic separates a kind when it flags at least half of it at MAD's \
         false-alarm rate)"
    );
    if found.is_empty() {
        println!(
            "  no statistic separates variance, oscillation or decouple: the stop condition \
             holds, and the statistics stay a tool for study"
        );
    } else {
        println!("  separated: {}", found.join("; "));
        println!(
            "  the stop condition does not hold: a statistic sees what a level detector misses"
        );
    }
}
