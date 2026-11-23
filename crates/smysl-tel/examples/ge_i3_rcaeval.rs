//! GE-I3 on real incidents (RFC SMYSL-5 §15): does an information measure find anything the
//! shipped detectors miss, on RCAEval's labelled faults rather than simulated ones?
//!
//! **The data.** RCAEval RE2-SS (Pham et al., arXiv 2412.17015; Zenodo record 14590730, CC BY
//! 4.0): Sock Shop, 90 cases, six fault types (`cpu`, `mem`, `disk`, `delay`, `loss`,
//! `socket`) injected into five services, three times each. A case is 24 minutes at one sample
//! a second, the fault injected at minute 12 (`inject_time.txt`). The data is not in this
//! repository; the run reads an extracted `RE2-SS/` directory given as `--data`.
//!
//! **Windows.** Each case gives two pairs of six-minute windows, so normal and faulty pairs are
//! read from the same incidents with the same lengths:
//!
//! - **normal**: reference `[t−12m, t−6m)`, test `[t−6m, t)`, both before the injection `t`;
//! - **faulty**: reference `[t−6m, t)`, test `[t, t+6m)`.
//!
//! **Series.** The faulty service's columns of `simple_metrics.csv` (its `cpu`, `mem`, `diskio`,
//! `socket`, `workload`, `latency-50`, `latency-90`, those it has), and its container's
//! `logs.csv` lines for `tent/1`. A window pair is flagged by a detector if any of its series is;
//! a statistic's score for it is the largest over its series (over its pairs, for `mi/1`).
//!
//! **Arms**, each by the shipped code. Every arm gives a window pair a **score**, so that all of
//! them can be held to one false-alarm rate:
//!
//! - `mad/1`: the largest |robust z| in the test window, against the reference window's centre
//!   and scale (`mad::centre_and_scale`). `mad/1` at threshold θ flags the pair exactly when
//!   this exceeds θ.
//! - `cusum/1` and `pelt/1` over the twelve minutes: the largest `h` (CUSUM, its other
//!   parameters at their defaults) or `penalty` (PELT) on a grid of 10^(i/8), i = −8 … 160, at
//!   which a change point falls in the test window; 0 if none does at any.
//! - `jsd/1` (`baseline:4`) against the reference; |Δ `perm/1`| (m = 3); the fall of `mi/1`
//!   (`rank:4`, lag 0); |Δ `tent/1`| over each window's templates by the shipped Drain.
//!
//! **The first reading, superseded.** It held the statistics to MAD's false-alarm rate with the
//! detectors at their shipped defaults, as the simulated run does. On this data the defaults
//! flag the normal pairs too: MAD 90.0 %, CUSUM and PELT 100 %. Every fault type was then
//! "seen" and every threshold sat at the bottom, so the run decided nothing. The shipped
//! defaults' false-alarm rates are still printed.
//!
//! **The reading, amended and fixed before the rerun.** RCAEval does not label the RFC's three
//! kinds (variance, oscillation, decouple), so the stop condition is read on what is labelled:
//!
//! - Every arm, detector or statistic, is held to a **10 % false-alarm rate** on the 90 normal
//!   pairs: its threshold is the least score that at most 9 of them exceed. Its detection rate
//!   per fault type is the share of that type's 15 faulty pairs over its threshold.
//! - A fault type is **unseen by the level detectors** when MAD, CUSUM and PELT each flag fewer
//!   than half of it. A statistic **separates** a type when it flags at least half.
//! - **The real-data arm agrees with the simulated verdict** (the stop condition does not hold)
//!   if some statistic separates a fault type unseen by the level detectors. If none does, the
//!   real data shows nothing the detectors miss at that rate, the simulated verdict rests on
//!   simulated kinds alone, and RFC SMYSL-5 records that against any detector RFC.
//!
//! ```text
//! cargo run --release -p smysl-tel --example ge_i3_rcaeval -- --data PATH/RE2-SS
//! ```

// The detectors' parameter grid is drawn with the platform's `powf`; every number the arms
// compute is the shipped code's.
#![allow(clippy::disallowed_methods)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use smysl_core::{Decimal, Uid};
use smysl_series::Sample;
use smysl_tel::detect::{cusum, mad, pelt, Input};
use smysl_tel::drain;
use smysl_tel::info::{self, logs, Read, Spec, Stream};

const SEC: u64 = 1_000;
const HALF: u64 = 6 * 60 * SEC;
/// The false-alarm rate every arm is held to on the normal pairs.
const RATE: f64 = 0.10;
const FAULTS: [&str; 6] = ["cpu", "mem", "disk", "delay", "loss", "socket"];

/// A minimal RFC 4180 reader: quoted fields, doubled quotes, newlines inside quotes.
fn csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
        } else {
            match c {
                '"' => quoted = true,
                ',' => row.push(std::mem::take(&mut field)),
                '\r' => {}
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                _ => field.push(c),
            }
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn decimal(raw: &str) -> Option<Decimal> {
    if let Ok(d) = Decimal::parse_token(raw) {
        return Some(d);
    }
    let x: f64 = raw.parse().ok()?;
    x.is_finite()
        .then(|| Decimal::parse_token(&format!("{x:.9}")).ok())
        .flatten()
}

/// One case: its injection instant, the faulty service's series, and its container's log lines.
struct Case {
    service: String,
    fault: String,
    inject_ms: u64,
    series: Vec<(String, Vec<Sample>)>,
    /// (timestamp ms, message), in event order.
    lines: Vec<(u64, String)>,
}

fn load(dir: &Path, service: &str, fault: &str) -> Case {
    let read = |f: &str| {
        std::fs::read_to_string(dir.join(f)).unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
    };
    let inject_ms = read("inject_time.txt")
        .trim()
        .parse::<u64>()
        .expect("a time")
        * SEC;
    let rows = csv(&read("simple_metrics.csv"));
    let head = &rows[0];
    let prefix = format!("{service}_");
    let mut series = Vec::new();
    for (j, name) in head.iter().enumerate().skip(1) {
        if !name.starts_with(&prefix) {
            continue;
        }
        let samples: Vec<Sample> = rows[1..]
            .iter()
            .filter_map(|r| {
                let t: f64 = r.first()?.parse().ok()?;
                let v = decimal(r.get(j)?.trim())?;
                Sample::new(t as u64 * SEC, None, v, 0).ok()
            })
            .collect();
        series.push((name.clone(), samples));
    }
    let logs = csv(&read("logs.csv"));
    let col = |n: &str| logs[0].iter().position(|h| h == n).expect("a column");
    let (ts, who, msg) = (col("timestamp"), col("container_name"), col("message"));
    let mut lines: Vec<(u64, String)> = logs[1..]
        .iter()
        .filter(|r| r.get(who).map(String::as_str) == Some(service))
        .filter_map(|r| {
            Some((
                r.get(ts)?.parse::<u64>().ok()? / 1_000_000,
                r.get(msg)?.clone(),
            ))
        })
        .collect();
    lines.sort_by_key(|l| l.0);
    Case {
        service: service.to_string(),
        fault: fault.to_string(),
        inject_ms,
        series,
        lines,
    }
}

fn stream(i: usize, class: &'static str) -> Stream<'static> {
    Stream {
        series: Uid::from_bytes([i as u8 + 1; 32]),
        scheme: "prom",
        key: "ge-i3-rcaeval",
        class,
        instrument: Uid::from_bytes([9; 32]),
    }
}

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

fn value(c: Result<info::Computed, info::Refusal>) -> Option<f64> {
    c.ok()?.value.to_string().parse().ok()
}

/// What one window pair gives: the detectors' flags, and each statistic's score where one ran.
#[derive(Default, Clone)]
struct Row {
    /// Flagged by the detectors at their shipped defaults: MAD, CUSUM, PELT.
    flags: [bool; 3],
    /// The detectors' scores: MAD, CUSUM, PELT.
    det: [f64; 3],
    jsd: Option<f64>,
    perm: Option<f64>,
    mi: Option<f64>,
    tent: Option<f64>,
}

fn max(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

/// An arm's score of a window pair, if it ran.
type Score = dyn Fn(&Row) -> Option<f64>;

/// Counts of refused computations, by statistic.
#[derive(Default)]
struct Refused(BTreeMap<&'static str, usize>);

impl Refused {
    fn note(&mut self, name: &'static str, v: Option<f64>) -> Option<f64> {
        if v.is_none() {
            *self.0.entry(name).or_insert(0) += 1;
        }
        v
    }
}

/// The window pair `[r0, t0)`, `[t0, t1)` of one case.
fn row(case: &Case, r0: u64, t0: u64, t1: u64, refused: &mut Refused) -> Row {
    let mut out = Row::default();
    let jsd = Spec::parse(&format!(
        r#"{{ stat: jsd/1, step: 1000, bins: "baseline:4", ref: "{r0}-{t0}", min_n: 50 }}"#
    ))
    .expect("a recipe");
    let perm = Spec::parse("{ stat: perm/1, step: 1000, m: 3, min_n: 50 }").expect("a recipe");
    let mi =
        Spec::parse(r#"{ stat: mi/1, step: 1000, bins: "rank:4", min_n: 50 }"#).expect("a recipe");
    let tent = Spec::parse("{ stat: tent/1 }").expect("a recipe");

    for (i, (_, s)) in case.series.iter().enumerate() {
        let lo = s.partition_point(|x| x.ts_ms() < r0);
        let hi = s.partition_point(|x| x.ts_ms() < t1);
        let window = &s[lo..hi];
        if window.is_empty() {
            continue;
        }
        let input = Input {
            series: Uid::from_bytes([i as u8 + 1; 32]),
            scheme: "prom",
            key: "ge-i3-rcaeval",
            instrument: Uid::from_bytes([9; 32]),
            samples: window,
        };
        let split = window.partition_point(|x| x.ts_ms() < t0);
        out.flags[0] |= !mad::run(&input, window, t0, t1, &mad::Params::new(r0, t0)).is_empty();
        let points = input.points();
        out.flags[1] |= cusum::change_points(&points, &cusum::Params::default())
            .iter()
            .any(|&i| i >= split);
        let values: Vec<f64> = points.iter().map(|p| p.1).collect();
        out.flags[2] |= pelt::change_points(&values, &pelt::Params::default())
            .iter()
            .any(|&i| i >= split);

        let (base, test) = values.split_at(split);
        if let Some((med, scale)) = mad::centre_and_scale(base) {
            let z = test
                .iter()
                .map(|x| ((x - med) / scale).abs())
                .fold(0.0, f64::max);
            out.det[0] = out.det[0].max(z);
        }
        let in_test = |cps: Vec<usize>| cps.iter().any(|&i| i >= split);
        let largest = |hit: &dyn Fn(Decimal) -> bool| {
            grid().rev().find(|(_, d)| hit(*d)).map_or(0.0, |(g, _)| g)
        };
        let c = largest(&|h| {
            in_test(cusum::change_points(
                &points,
                &cusum::Params {
                    h,
                    ..cusum::Params::default()
                },
            ))
        });
        let p = largest(&|penalty| {
            in_test(pelt::change_points(
                &values,
                &pelt::Params {
                    penalty,
                    ..pelt::Params::default()
                },
            ))
        });
        out.det[1] = out.det[1].max(c);
        out.det[2] = out.det[2].max(p);

        let st = stream(i, "gauge");
        let (r, t) = (read(s, r0, t0), read(s, t0, t1));
        let j = refused.note("jsd/1", value(info::compute(&jsd, &st, &t, Some(&r), None)));
        out.jsd = max(out.jsd, j);
        let pr = value(info::compute(&perm, &st, &r, None, None));
        let pt = value(info::compute(&perm, &st, &t, None, None));
        let p = refused.note("perm/1", pr.zip(pt).map(|(a, b)| (b - a).abs()));
        out.perm = max(out.perm, p);
    }
    for a in 0..case.series.len() {
        for b in a + 1..case.series.len() {
            let (sa, sb) = (&case.series[a].1, &case.series[b].1);
            let (ta, tb) = (stream(a, "gauge"), stream(b, "gauge"));
            let m = |from, to| {
                value(info::compute(
                    &mi,
                    &ta,
                    &read(sa, from, to),
                    None,
                    Some((&tb, &read(sb, from, to))),
                ))
            };
            let drop = refused.note("mi/1", m(r0, t0).zip(m(t0, t1)).map(|(x, y)| x - y));
            out.mi = max(out.mi, drop);
        }
    }
    let templates = |from: u64, to: u64| {
        let lo = case.lines.partition_point(|l| l.0 < from);
        let hi = case.lines.partition_point(|l| l.0 < to);
        logs::Templates {
            instrument: Uid::from_bytes([9; 32]),
            digest: [0; 32],
            counts: drain::run(case.lines[lo..hi].iter().map(|l| l.1.as_str())),
        }
    };
    let st = stream(200, "log");
    let tr = value(logs::compute(&tent, &st, r0, t0, &templates(r0, t0)));
    let tt = value(logs::compute(&tent, &st, t0, t1, &templates(t0, t1)));
    out.tent = refused.note("tent/1", tr.zip(tt).map(|(a, b)| (b - a).abs()));
    out
}

/// The detectors' parameter grid, 10^(i/8) for i = −8 … 160, as (value, the decimal passed).
fn grid() -> impl DoubleEndedIterator<Item = (f64, Decimal)> {
    (-8i32..=160).map(|i| {
        let g = 10f64.powf(f64::from(i) / 8.0);
        let d = Decimal::parse_token(&format!("{g:.6e}")).expect("a decimal");
        (g, d)
    })
}

/// The least threshold that at most `rate` of `normal` scores exceed.
fn threshold(normal: &[f64], rate: f64) -> f64 {
    let mut xs = normal.to_vec();
    xs.sort_by(f64::total_cmp);
    let allowed = (rate * xs.len() as f64).floor() as usize;
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data: PathBuf = args
        .iter()
        .position(|a| a == "--data")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .expect("--data PATH/RE2-SS");

    let mut cases = Vec::new();
    let mut dirs: Vec<_> = std::fs::read_dir(&data)
        .expect("the RE2-SS directory")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for d in dirs {
        let name = d.file_name().expect("a name").to_string_lossy().to_string();
        let Some((service, fault)) = name.rsplit_once('_') else {
            continue;
        };
        let mut reps: Vec<_> = std::fs::read_dir(&d)
            .expect("a case directory")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        reps.sort();
        for rep in reps {
            cases.push(load(&rep, service, fault));
        }
    }

    let mut refused = Refused::default();
    let mut normal = Vec::new();
    let mut faulty: BTreeMap<String, Vec<Row>> = BTreeMap::new();
    for c in &cases {
        let t = c.inject_ms;
        normal.push(row(c, t - 2 * HALF, t - HALF, t, &mut refused));
        faulty.entry(c.fault.clone()).or_default().push(row(
            c,
            t - HALF,
            t,
            t + HALF,
            &mut refused,
        ));
    }
    let services: std::collections::BTreeSet<&str> =
        cases.iter().map(|c| c.service.as_str()).collect();

    println!(
        "GE-I3 on RCAEval RE2-SS (real, CC BY 4.0): {} cases, services {:?}; six-minute \
         windows at one sample a second.",
        cases.len(),
        services
    );
    println!(
        "Normal pairs end at the injection, faulty pairs start at it. Every arm at the \
         threshold that at most {:.0} % of the normal pairs exceed.\n",
        100.0 * RATE
    );
    println!(
        "{:<8} {:>3} {:>7} {:>7} {:>7} | {:>7} {:>7} {:>7} {:>7} | {:>9}",
        "fault", "n", "MAD", "CUSUM", "PELT", "jsd/1", "perm/1", "mi/1", "tent/1", "beyond"
    );
    let cell = |x: Option<f64>| x.map_or("-".to_string(), |v| format!("{:.1} %", 100.0 * v));
    let gets: [(&str, &Score); 7] = [
        ("MAD", &|r| Some(r.det[0])),
        ("CUSUM", &|r| Some(r.det[1])),
        ("PELT", &|r| Some(r.det[2])),
        ("jsd/1", &|r| r.jsd),
        ("perm/1", &|r| r.perm),
        ("mi/1", &|r| r.mi),
        ("tent/1", &|r| r.tent),
    ];
    let thresholds: Vec<Option<f64>> = gets
        .iter()
        .map(|(_, get)| {
            let ns: Vec<f64> = normal.iter().filter_map(get).collect();
            (!ns.is_empty()).then(|| threshold(&ns, RATE))
        })
        .collect();
    let over =
        |k: usize, r: &Row| thresholds[k].is_some_and(|t| gets[k].1(r).is_some_and(|s| s > t));
    let mut separated = Vec::new();
    for fault in FAULTS {
        let rows = faulty.get(fault).map(Vec::as_slice).unwrap_or(&[]);
        let rates: Vec<Option<f64>> = (0..gets.len())
            .map(|k| thresholds[k].map(|_| share(rows.iter().map(|r| over(k, r)))))
            .collect();
        // Faulty pairs some statistic flags and no detector does.
        let beyond = rows
            .iter()
            .filter(|r| (3..7).any(|k| over(k, r)) && !(0..3).any(|k| over(k, r)))
            .count();
        let cells: Vec<String> = rates.iter().map(|v| cell(*v)).collect();
        println!(
            "{:<8} {:>3} {:>7} {:>7} {:>7} | {:>7} {:>7} {:>7} {:>7} | {:>9}",
            fault,
            rows.len(),
            cells[0],
            cells[1],
            cells[2],
            cells[3],
            cells[4],
            cells[5],
            cells[6],
            format!("{beyond}/{}", rows.len())
        );
        let unseen = rates[..3].iter().all(|d| d.is_some_and(|d| d < 0.5));
        for k in 3..7 {
            if unseen && rates[k].is_some_and(|v| v >= 0.5) {
                separated.push(format!("{fault} by {}", gets[k].0));
            }
        }
    }
    let held: Vec<String> = (0..gets.len())
        .map(|k| {
            let n = normal.iter().filter(|r| over(k, r)).count();
            format!("{} {n}", gets[k].0)
        })
        .collect();
    println!(
        "\nnormal pairs over each threshold (of {}): {}",
        normal.len(),
        held.join(", ")
    );
    let nf = |i: usize| 100.0 * share(normal.iter().map(|r| r.flags[i]));
    println!(
        "the shipped defaults' false alarms on them: MAD {:.1} %, CUSUM {:.1} %, PELT {:.1} %",
        nf(0),
        nf(1),
        nf(2)
    );
    println!(
        "refused computations (series or pairs too short or too constant): {:?}",
        refused.0
    );

    println!(
        "\nverdict (at {:.0} % false alarms: a type is unseen when MAD, CUSUM and PELT each flag \
         under half of it; a statistic separates it when it flags at least half)",
        100.0 * RATE
    );
    if separated.is_empty() {
        println!(
            "  no statistic separates a fault type the level detectors miss: on RE2-SS the \
             statistics find nothing the detectors do not at this rate, and the simulated verdict rests on \
             simulated kinds alone"
        );
    } else {
        println!(
            "  separated, unseen by the level detectors: {}",
            separated.join("; ")
        );
        println!("  agrees with the simulated verdict: the stop condition does not hold");
    }
}
