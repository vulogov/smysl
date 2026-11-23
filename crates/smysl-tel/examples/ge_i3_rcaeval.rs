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
//! **Arms**, each by the shipped code, so that all of them can be held to one false-alarm rate:
//!
//! - `mad/1`, `cusum/1`, `pelt/1`: calibrated by `detect::calibrate` (RFC SMYSL-3 §21.1) on the
//!   normal pairs, each case's series one trial; a pair is flagged when the detector, at its
//!   calibrated threshold, `h` or `penalty`, fires on any of its series.
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
//!   pairs: a statistic's threshold is the least score that at most 9 of them exceed, a
//!   detector's the least ladder value at which at most 9 of them fire. Its detection rate
//!   per fault type is the share of that type's 15 faulty pairs over its threshold.
//! - A fault type is **unseen by the level detectors** when MAD, CUSUM and PELT each flag fewer
//!   than half of it. A statistic **separates** a type when it flags at least half.
//! - **The real-data arm agrees with the simulated verdict** (the stop condition does not hold)
//!   if some statistic separates a fault type unseen by the level detectors. If none does, the
//!   real data shows nothing the detectors miss at that rate, the simulated verdict rests on
//!   simulated kinds alone, and RFC SMYSL-5 records that against any detector RFC.
//!
//! **Held out** (R8). With `--held PATH/RE2-OB` the detectors' values calibrated on `--data`
//! are run on a second system's normal and faulty pairs unchanged, which is what a calibrated
//! recipe is for; that system is then read at its own calibration as above.
//!
//! **Per series** (R9). Each detector is also calibrated per metric kind (`cpu`, `mem`,
//! `latency-90`, …, the column with its service's prefix taken off) by
//! `calibrate::each_from_scores`, the rate held over the trials jointly, so the kinds carry
//! from one system's services to another's. A kind that fired in no fault-free window, or that
//! the calibrating system does not have, is run at no value.
//!
//! **Floor and margin** (R10). Every calibration is also made under `floor: resolution`
//! (RFC SMYSL-3 §21.2), which keeps a flat reference from scaling to `10⁻⁹`, and at a transfer
//! margin of 2 (`calibrate::Target::with_margin`, at most `⌊rate · n / 2⌋` normal pairs) beside
//! 1. Each is run on the calibrating set and, unchanged, on the held-out one.
//!
//! **A third system** (R11). `--held` takes any RCAEval RE2 system; RE2-TT (Train Ticket) is
//! read as RE2-OB is.
//!
//! ```text
//! cargo run --release -p smysl-tel --example ge_i3_rcaeval -- --data PATH/RE2-SS [--held PATH/RE2-OB]
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use smysl_core::{Decimal, Uid};
use smysl_series::Sample;
use smysl_tel::detect::{
    calibrate, cusum, mad, pelt, Floor, Input, Options as DetOptions, Spec as DetSpec,
};
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
    /// The detectors' calibration scores under each of `opts()`.
    scores: Vec<Scores>,
    /// Whether each detector fires, at each [`Ask`].
    fired: Vec<[bool; 3]>,
    jsd: Option<f64>,
    perm: Option<f64>,
    mi: Option<f64>,
    tent: Option<f64>,
}

/// The detectors' calibration scores (`calibrate::score`) under one `Options`: MAD, CUSUM,
/// PELT, for the trial and by metric kind (`cpu`, `latency-90`, …).
#[derive(Default, Clone)]
struct Scores {
    det: [Option<Decimal>; 3],
    each: [BTreeMap<String, Option<Decimal>>; 3],
}

/// Values to run the detectors at: under which of `opts()`, one a trial or one per kind.
#[derive(Clone)]
enum Ask {
    Trial(usize, [Decimal; 3]),
    Each(usize, Values),
}

/// The detector options compared: the shipped floor, and `floor: resolution` (R10).
fn opts() -> [DetOptions; 2] {
    [
        DetOptions::default(),
        DetOptions::default().with_floor(Floor::Resolution),
    ]
}

fn max(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

/// Per-kind values, one map per detector.
type Values = [BTreeMap<String, Decimal>; 3];

fn cmp_dec(a: &Decimal, b: &Decimal) -> std::cmp::Ordering {
    let f = |d: &Decimal| d.to_string().parse::<f64>().unwrap_or(f64::NAN);
    f(a).total_cmp(&f(b))
}

/// One stream's `(ts_ms, value)` points in a window.
type Points = Vec<(u64, f64)>;

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
fn row(case: &Case, (r0, t0, t1): (u64, u64, u64), asks: &[Ask], refused: &mut Refused) -> Row {
    let mut out = Row::default();
    let mut pairs: Vec<(String, Points, Points)> = Vec::new();
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

        let (base, test) = points.split_at(split);
        let kind = case.series[i]
            .0
            .strip_prefix(&format!("{}_", case.service))
            .unwrap_or(&case.series[i].0)
            .to_string();
        pairs.push((kind, base.to_vec(), test.to_vec()));

        let st = stream(i, "gauge");
        let (r, t) = (read(s, r0, t0), read(s, t0, t1));
        let j = refused.note("jsd/1", value(info::compute(&jsd, &st, &t, Some(&r), None)));
        out.jsd = max(out.jsd, j);
        let pr = value(info::compute(&perm, &st, &r, None, None));
        let pt = value(info::compute(&perm, &st, &t, None, None));
        let p = refused.note("perm/1", pr.zip(pt).map(|(a, b)| (b - a).abs()));
        out.perm = max(out.perm, p);
    }
    let trial: Vec<(&str, calibrate::Pair<'_>)> = pairs
        .iter()
        .map(|(kind, r, t)| {
            (
                kind.as_str(),
                calibrate::Pair {
                    reference: r,
                    test: t,
                },
            )
        })
        .collect();
    let specs = specs();
    // A trial's score is the largest of its series' scores, so one pass gives both.
    out.scores = opts()
        .iter()
        .map(|o| {
            let each: [BTreeMap<String, Option<Decimal>>; 3] = std::array::from_fn(|k| {
                trial
                    .iter()
                    .map(|(kind, p)| {
                        (
                            kind.to_string(),
                            calibrate::score(&specs[k], o, std::slice::from_ref(p)),
                        )
                    })
                    .collect()
            });
            let det = std::array::from_fn(|k| each[k].values().flatten().copied().max_by(cmp_dec));
            Scores { det, each }
        })
        .collect();
    let all: Vec<calibrate::Pair<'_>> = trial.iter().map(|(_, p)| *p).collect();
    out.fired = asks
        .iter()
        .map(|a| match a {
            Ask::Trial(o, v) => {
                std::array::from_fn(|k| calibrate::fires(&specs[k], &opts()[*o], &all, v[k]))
            }
            Ask::Each(o, v) => std::array::from_fn(|k| {
                trial.iter().any(|(kind, p)| {
                    v[k].get(*kind).is_some_and(|x| {
                        calibrate::fires(&specs[k], &opts()[*o], std::slice::from_ref(p), *x)
                    })
                })
            }),
        })
        .collect();
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

/// The level detectors at their shipped defaults, for calibration: MAD (its baseline is each
/// pair's reference), CUSUM, PELT.
fn specs() -> [DetSpec; 3] {
    [
        DetSpec::Mad(mad::Params::new(0, 1)),
        DetSpec::Cusum(cusum::Params::default()),
        DetSpec::Pelt(pelt::Params::default()),
    ]
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

/// Every case under an extracted RE2 directory: `SERVICE_FAULT/REP/`.
fn load_all(data: &Path) -> Vec<Case> {
    let mut cases = Vec::new();
    let mut dirs: Vec<_> = std::fs::read_dir(data)
        .unwrap_or_else(|e| panic!("{}: {e}", data.display()))
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
            // The numbered repetitions; RE2-OB ships a stray `multi-source-data/` beside them.
            .filter(|p| {
                p.is_dir()
                    && p.file_name()
                        .is_some_and(|n| n.to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
            })
            .collect();
        reps.sort();
        for rep in reps {
            cases.push(load(&rep, service, fault));
        }
    }
    cases
}

/// One data set's rows: the normal pairs, and the faulty pairs by fault type.
struct Reading {
    normal: Vec<Row>,
    faulty: BTreeMap<String, Vec<Row>>,
}

fn normal_rows(cases: &[Case], asks: &[Ask], refused: &mut Refused) -> Vec<Row> {
    cases
        .iter()
        .map(|c| {
            let t = c.inject_ms;
            row(c, (t - 2 * HALF, t - HALF, t), asks, refused)
        })
        .collect()
}

fn faulty_rows(cases: &[Case], asks: &[Ask], refused: &mut Refused) -> BTreeMap<String, Vec<Row>> {
    let mut out: BTreeMap<String, Vec<Row>> = BTreeMap::new();
    for c in cases {
        let t = c.inject_ms;
        out.entry(c.fault.clone()).or_default().push(row(
            c,
            (t - HALF, t, t + HALF),
            asks,
            refused,
        ));
    }
    out
}

/// A calibration compared: which of `opts()`, and one value a trial or one per metric kind.
#[derive(Clone, Copy)]
struct Config {
    opt: usize,
    each: bool,
}

const CONFIGS: [Config; 4] = [
    Config {
        opt: 0,
        each: false,
    },
    Config { opt: 0, each: true },
    Config {
        opt: 1,
        each: false,
    },
    Config { opt: 1, each: true },
];

/// The transfer margins compared (R10).
const MARGINS: [i64; 2] = [1, 2];

impl Config {
    fn label(&self) -> String {
        format!(
            "{}, {}",
            ["tiny floor", "resolution floor"][self.opt],
            if self.each { "per kind" } else { "one a trial" }
        )
    }
}

/// One calibration of `normal` at `RATE` over `margin`: what to ask rows for, how many normal
/// pairs fire at it, and its values as text.
fn calibrate_config(normal: &[Row], cfg: Config, margin: i64) -> (Ask, [usize; 3], String) {
    let target = calibrate::Target::new(Decimal::parse_token(&format!("{RATE}")).expect("a rate"))
        .and_then(|t| t.with_margin(Decimal::from_i64(margin)))
        .expect("a target");
    let specs = specs();
    if cfg.each {
        let c: [calibrate::CalibratedEach<String>; 3] = std::array::from_fn(|k| {
            let scores: Vec<BTreeMap<String, Option<Decimal>>> = normal
                .iter()
                .map(|r| r.scores[cfg.opt].each[k].clone())
                .collect();
            calibrate::each_from_scores(&specs[k], &scores, target).expect("a calibration")
        });
        let values: Values = std::array::from_fn(|k| {
            c[k].series
                .iter()
                .filter(|(_, v)| v.calibrated)
                .map(|(kind, v)| (kind.clone(), v.value))
                .collect()
        });
        let text = ["MAD", "CUSUM", "PELT"]
            .iter()
            .enumerate()
            .map(|(k, d)| {
                let vs: Vec<String> = c[k]
                    .series
                    .iter()
                    .map(|(kind, v)| {
                        if v.calibrated {
                            format!("{kind} {}", v.value)
                        } else {
                            format!("{kind} -")
                        }
                    })
                    .collect();
                format!("    {d} (q = {}): {}", c[k].q, vs.join(", "))
            })
            .collect::<Vec<_>>()
            .join("\n");
        (
            Ask::Each(cfg.opt, values),
            std::array::from_fn(|k| c[k].over),
            text,
        )
    } else {
        let c: [calibrate::Calibrated; 3] = std::array::from_fn(|k| {
            let scores: Vec<Option<Decimal>> =
                normal.iter().map(|r| r.scores[cfg.opt].det[k]).collect();
            calibrate::from_scores(&specs[k], &scores, target).expect("a calibration")
        });
        let text = format!(
            "    MAD {}, CUSUM h {}, PELT penalty {}",
            c[0].value, c[1].value, c[2].value
        );
        (
            Ask::Trial(cfg.opt, std::array::from_fn(|k| c[k].value)),
            std::array::from_fn(|k| c[k].over),
            text,
        )
    }
}

/// A statistic's score of a window pair, if it ran.
type Stat = fn(&Row) -> Option<f64>;

const STATS: [(&str, Stat); 4] = [
    ("jsd/1", |r| r.jsd),
    ("perm/1", |r| r.perm),
    ("mi/1", |r| r.mi),
    ("tent/1", |r| r.tent),
];

/// The statistics' thresholds: the least score that at most `RATE` of the normal pairs exceed.
fn stat_thresholds(reading: &Reading) -> Vec<Option<f64>> {
    STATS
        .iter()
        .map(|(_, get)| {
            let ns: Vec<f64> = reading.normal.iter().filter_map(get).collect();
            (!ns.is_empty()).then(|| threshold(&ns, RATE))
        })
        .collect()
}

fn stat_over(th: &[Option<f64>], k: usize, r: &Row) -> bool {
    th[k].is_some_and(|t| STATS[k].1(r).is_some_and(|s| s > t))
}

fn pct(x: f64) -> String {
    format!("{:.1} %", 100.0 * x)
}

fn rows_of<'a>(reading: &'a Reading, fault: &str) -> &'a [Row] {
    reading.faulty.get(fault).map(Vec::as_slice).unwrap_or(&[])
}

/// One table per detector: a line per calibration, its normal-pair column, then the share of
/// each fault type it flags.
fn detector_tables(reading: &Reading, lines: &[(String, [String; 3], usize)]) {
    for (k, det) in ["MAD", "CUSUM", "PELT"].iter().enumerate() {
        println!(
            "  {det:<34} {:>8} | {}",
            "normal",
            FAULTS.map(|f| format!("{f:>7}")).join(" ")
        );
        for (label, normal, ask) in lines {
            let cells: Vec<String> = FAULTS
                .iter()
                .map(|f| {
                    format!(
                        "{:>7}",
                        pct(share(rows_of(reading, f).iter().map(|r| r.fired[*ask][k])))
                    )
                })
                .collect();
            println!("    {label:<32} {:>8} | {}", normal[k], cells.join(" "));
        }
    }
}

/// The statistics' table, and per calibration its `beyond` and verdict.
fn statistics_and_verdicts(name: &str, reading: &Reading, configs: &[(String, usize)]) {
    let th = stat_thresholds(reading);
    println!(
        "  {:<34} {:>8} | {}",
        "statistic",
        "normal",
        FAULTS.map(|f| format!("{f:>7}")).join(" ")
    );
    for (k, (stat, _)) in STATS.iter().enumerate() {
        let normal = reading
            .normal
            .iter()
            .filter(|r| stat_over(&th, k, r))
            .count();
        let cells: Vec<String> = FAULTS
            .iter()
            .map(|f| match th[k] {
                Some(_) => format!(
                    "{:>7}",
                    pct(share(
                        rows_of(reading, f).iter().map(|r| stat_over(&th, k, r))
                    ))
                ),
                None => format!("{:>7}", "-"),
            })
            .collect();
        println!(
            "    {stat:<32} {:>8} | {}",
            format!("{normal}/{}", reading.normal.len()),
            cells.join(" ")
        );
    }
    for (label, ask) in configs {
        let mut separated = Vec::new();
        let mut beyond = 0;
        for f in FAULTS {
            let rows = rows_of(reading, f);
            beyond += rows
                .iter()
                .filter(|r| {
                    (0..4).any(|k| stat_over(&th, k, r)) && !r.fired[*ask].iter().any(|x| *x)
                })
                .count();
            let unseen = (0..3).all(|k| share(rows.iter().map(|r| r.fired[*ask][k])) < 0.5);
            for (k, (stat, _)) in STATS.iter().enumerate() {
                if unseen && share(rows.iter().map(|r| stat_over(&th, k, r))) >= 0.5 {
                    separated.push(format!("{f} by {stat}"));
                }
            }
        }
        println!(
            "  verdict on {name}, {label}: {} faulty pair(s) flagged by a statistic and no detector; {}",
            beyond,
            if separated.is_empty() {
                "no statistic separates a type the level detectors miss".to_string()
            } else {
                format!("separated: {}", separated.join("; "))
            }
        );
    }
}

fn defaults_line(reading: &Reading) {
    let nf = |i: usize| pct(share(reading.normal.iter().map(|r| r.flags[i])));
    println!(
        "the shipped defaults' false alarms on the normal pairs: MAD {}, CUSUM {}, PELT {}",
        nf(0),
        nf(1),
        nf(2)
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .map(PathBuf::from)
    };
    let data = arg("--data").expect("--data PATH/RE2-SS");
    let held = arg("--held");
    let label = |p: &Path| {
        p.file_name()
            .map_or("data".into(), |n| n.to_string_lossy().to_string())
    };

    let mut refused = Refused::default();
    let cases = load_all(&data);
    let services: std::collections::BTreeSet<&str> =
        cases.iter().map(|c| c.service.as_str()).collect();
    let name = label(&data);
    println!(
        "GE-I3 on RCAEval {name} (real, CC BY 4.0): {} cases, services {:?}; six-minute \
         windows at one sample a second.",
        cases.len(),
        services
    );
    println!(
        "Normal pairs end at the injection, faulty pairs start at it. The detectors are \
         calibrated to {:.0} % of the normal pairs over a margin, under each floor, one value a \
         trial or one per metric kind; the statistics are held to {:.0} %.\n",
        100.0 * RATE,
        100.0 * RATE
    );
    let normal = normal_rows(&cases, &[], &mut refused);
    // Every calibration of the calibrating set: by config, then margin.
    let mut asks = Vec::new();
    let mut lines = Vec::new();
    let mut texts = Vec::new();
    for cfg in CONFIGS {
        for m in MARGINS {
            let (ask, over, text) = calibrate_config(&normal, cfg, m);
            let label = format!("{}, margin {m}", cfg.label());
            lines.push((
                label.clone(),
                over.map(|n| format!("{n}/{}", normal.len())),
                asks.len(),
            ));
            texts.push(format!("  {label}:\n{text}"));
            asks.push(ask);
        }
    }
    let reading = Reading {
        faulty: faulty_rows(&cases, &asks, &mut refused),
        normal,
    };
    defaults_line(&reading);
    println!("\n{name}'s calibrated values:");
    for t in &texts {
        println!("{t}");
    }
    println!("\n{name} at its own calibrations (normal: pairs over, of the calibrating set):");
    detector_tables(&reading, &lines);
    let own: Vec<(String, usize)> = lines
        .iter()
        .filter(|(l, _, _)| l.ends_with("margin 1"))
        .map(|(l, _, a)| (l.trim_end_matches(", margin 1").to_string(), *a))
        .collect();
    statistics_and_verdicts(&name, &reading, &own);
    println!(
        "refused computations (series or pairs too short or too constant): {:?}",
        refused.0
    );

    let Some(held) = held else {
        return;
    };
    // The held-out set: every calibration above run unchanged, and its own at margin 1.
    let mut refused = Refused::default();
    let hcases = load_all(&held);
    let hname = label(&held);
    let hservices: std::collections::BTreeSet<&str> =
        hcases.iter().map(|c| c.service.as_str()).collect();
    println!(
        "\nheld out: RCAEval {hname}, {} cases, services {:?}",
        hcases.len(),
        hservices
    );
    let normal = normal_rows(&hcases, &asks, &mut refused);
    let carried = asks.len();
    let mut all = asks.clone();
    let mut own_lines = Vec::new();
    let mut own_texts = Vec::new();
    for cfg in CONFIGS {
        let (ask, over, text) = calibrate_config(&normal, cfg, 1);
        own_lines.push((
            cfg.label(),
            over.map(|n| format!("{n}/{}", normal.len())),
            all.len(),
        ));
        own_texts.push(format!("  {}:\n{text}", cfg.label()));
        all.push(ask);
    }
    let reading = Reading {
        faulty: faulty_rows(&hcases, &all, &mut refused),
        normal,
    };
    defaults_line(&reading);
    let transfer: Vec<(String, [String; 3], usize)> = lines
        .iter()
        .map(|(l, _, a)| {
            let fa: [String; 3] =
                std::array::from_fn(|k| pct(share(reading.normal.iter().map(|r| r.fired[*a][k]))));
            (l.clone(), fa, *a)
        })
        .collect();
    debug_assert!(transfer.iter().all(|t| t.2 < carried));
    println!(
        "\n{name}'s calibrations carried to {hname} (normal: its false alarms there, calibrated for \
         {:.0} % / margin):",
        100.0 * RATE
    );
    detector_tables(&reading, &transfer);
    println!("\n{hname}'s own calibrated values (margin 1):");
    for t in &own_texts {
        println!("{t}");
    }
    println!("\n{hname} at its own calibrations, margin 1 (normal: pairs over):");
    detector_tables(&reading, &own_lines);
    let own: Vec<(String, usize)> = own_lines.iter().map(|(l, _, a)| (l.clone(), *a)).collect();
    statistics_and_verdicts(&hname, &reading, &own);
    println!(
        "refused computations (series or pairs too short or too constant): {:?}",
        refused.0
    );
}
