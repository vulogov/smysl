//! GE-M8 (RFC SMYSL-3 §21, §26) on real incidents: do RCAEval's repeated faults recur
//! detectably by episode similarity? The simulated run is `examples/ge_m8`.
//!
//! **The data.** RCAEval RE2 (Pham et al., arXiv 2412.17015; Zenodo record 14590730, CC BY 4.0):
//! RE2-SS (Sock Shop), RE2-OB (Online Boutique) and RE2-TT (Train Ticket), 90 cases each, six
//! fault types (`cpu`, `mem`, `disk`, `delay`, `loss`, `socket`) injected into five services,
//! **three times each**. The repetitions are the recurrences: one (service, fault) three times.
//! The data is not in this repository; each `--data PATH/RE2-XX` is an extracted directory.
//!
//! As fixed before the run (RFC SMYSL-3 §26):
//!
//! - **A case's episode** is what its faulty window `[t, t+6m)` shows, `t` the injection:
//!   - anomalies by `mad/1` under `floor: resolution` on **every** column of `simple_metrics.csv`,
//!     every service's and not only the faulty one's, since a search does not know the root
//!     cause. The baseline is `[t−6m, t)`. Each column's threshold is calibrated on its own
//!     fault-free pairs (`[t−12m, t−6m)` against `[t−6m, t)`, every case of the system) by
//!     `calibrate::from_scores`, to 10 % false alarms; a column whose rate cannot be met is not
//!     read;
//!   - the log templates **new** in the faulty window: per container, one Drain fed the
//!     reference window's lines and then the faulty window's, and the templates that the second
//!     added;
//!   - grouped by `detect::episode` with a gap longer than the window, so one case is at most
//!     one episode, and its features read by `similar::episodes`, as `tel similar` does.
//! - **Three arms** rank the other episodes of the same system: MinHash (128 functions), exact
//!   Jaccard, and Model2Vec over a text rendering of the same features by cosine
//!   (`SMYSL_EMBED_MODEL`, `potion-base-8M`).
//! - **Two readings:**
//!   - **recurrence**: of a case's two sibling repetitions, the share in its top five (a case
//!     with no episode finds none);
//!   - **type**: the simulated run's recall@5, the share of the top five of the same fault type,
//!     any service.
//! - **The stop condition** is the RFC's, read on the type reading in each system: MinHash below
//!   0.6 where embeddings exceed 0.8 makes similarity model-dependent by default.
//!
//! **Without templates** (after the run). `--no-templates` reads the anomalies alone. It was
//! added once the run showed hundreds of "new" templates a case on RE2-SS and RE2-TT, Drain's
//! templates drifting between windows rather than new behaviour, to see what they cost.
//!
//! ```text
//! cargo run --release --features tel,semantic --example ge_m8_rcaeval -- [--no-templates] \
//!     --data PATH/RE2-SS --data PATH/RE2-OB --data PATH/RE2-TT
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use smysl::schemas::tel;
use smysl::series::Sample;
use smysl::tel::detect::calibrate::{self, Pair};
use smysl::tel::detect::{episode, mad, Floor, Input, Options, Spec};
use smysl::tel::drain::Drain;
use smysl::tel::logs::Template;
use smysl::tel::similar;
use smysl::{canonical_uid, Decimal, Uid, UnitCore};

const SEC: u64 = 1_000;
const HALF: u64 = 6 * 60 * SEC;

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

fn to_f64(d: Decimal) -> f64 {
    smysl::tel::sketch::to_f64(d)
}

/// One case after its first pass: what calibration and its episode need, and nothing else.
struct Case {
    service: String,
    fault: String,
    inject_ms: u64,
    /// Each column's fault-free score (`[t−12m, t−6m)` against `[t−6m, t)`).
    normal: BTreeMap<String, Option<Decimal>>,
    /// Each column's samples in `[t−6m, t+6m)`.
    window: BTreeMap<String, Vec<(u64, Decimal)>>,
    /// The templates new in the faulty window, as `container: template`.
    templates: BTreeSet<String>,
}

fn opts() -> Options {
    Options::default().with_floor(Floor::Resolution)
}

fn mad_spec() -> Spec {
    Spec::parse(r#"{ detector: mad/1, baseline: "0-1" }"#).expect("a recipe")
}

fn load(dir: &Path, service: &str, fault: &str) -> Case {
    let read = |f: &str| {
        std::fs::read_to_string(dir.join(f)).unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
    };
    let t = read("inject_time.txt")
        .trim()
        .parse::<u64>()
        .expect("a time")
        * SEC;
    let rows = csv(&read("simple_metrics.csv"));
    let head = &rows[0];
    let (mut normal, mut window) = (BTreeMap::new(), BTreeMap::new());
    let spec = mad_spec();
    for (j, name) in head.iter().enumerate().skip(1) {
        if name == "timestamp" {
            continue;
        }
        let pts: Vec<(u64, Decimal)> = rows[1..]
            .iter()
            .filter_map(|r| {
                let ts: f64 = r.first()?.parse().ok()?;
                Some((ts as u64 * SEC, decimal(r.get(j)?.trim())?))
            })
            .collect();
        let range = |a: u64, b: u64| -> Vec<(u64, f64)> {
            pts.iter()
                .filter(|(ts, _)| (a..b).contains(ts))
                .map(|(ts, v)| (*ts, to_f64(*v)))
                .collect()
        };
        let (r, s) = (range(t - 2 * HALF, t - HALF), range(t - HALF, t));
        if r.len() < 2 || s.is_empty() {
            continue;
        }
        let pair = [Pair {
            reference: &r,
            test: &s,
        }];
        normal.insert(name.clone(), calibrate::score(&spec, &opts(), &pair));
        window.insert(
            name.clone(),
            pts.into_iter()
                .filter(|(ts, _)| (t - HALF..t + HALF).contains(ts))
                .collect(),
        );
    }
    // Logs: per container, the templates the faulty window adds to the reference window's.
    let logs = csv(&read("logs.csv"));
    let col = |n: &str| logs[0].iter().position(|h| h == n).expect("a column");
    let (ts, who, msg) = (col("timestamp"), col("container_name"), col("message"));
    let mut lines: Vec<(u64, &str, &str)> = logs[1..]
        .iter()
        .filter_map(|r| {
            Some((
                r.get(ts)?.parse::<u64>().ok()? / 1_000_000,
                r.get(who)?.as_str(),
                r.get(msg)?.as_str(),
            ))
        })
        .filter(|(ms, _, _)| (t - HALF..t + HALF).contains(ms))
        .collect();
    lines.sort_by_key(|l| l.0);
    let mut drains: BTreeMap<&str, Drain> = BTreeMap::new();
    for (_, c, m) in lines.iter().filter(|l| l.0 < t) {
        drains.entry(c).or_default().add(m);
    }
    let before: BTreeMap<&str, BTreeSet<String>> = drains
        .iter()
        .map(|(c, d)| (*c, d.templates().into_keys().collect()))
        .collect();
    for (_, c, m) in lines.iter().filter(|l| l.0 >= t) {
        drains.entry(c).or_default().add(m);
    }
    let mut templates = BTreeSet::new();
    for (c, d) in &drains {
        for k in d.templates().into_keys() {
            if !before.get(c).is_some_and(|b| b.contains(&k)) {
                templates.insert(format!("{c}: {k}"));
            }
        }
    }
    Case {
        service: service.to_string(),
        fault: fault.to_string(),
        inject_ms: t,
        normal,
        window,
        templates,
    }
}

fn load_all(data: &Path) -> Vec<Case> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(data)
        .unwrap_or_else(|e| panic!("{}: {e}", data.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    let mut cases = Vec::new();
    for d in dirs {
        let name = d.file_name().expect("a name").to_string_lossy().to_string();
        let Some((service, fault)) = name.rsplit_once('_') else {
            continue;
        };
        let mut reps: Vec<PathBuf> = std::fs::read_dir(&d)
            .expect("a case directory")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.is_dir()
                    && p.file_name()
                        .is_some_and(|n| n.to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
            })
            .collect();
        reps.sort();
        for rep in reps {
            eprintln!("  {}", rep.display());
            cases.push(load(&rep, service, fault));
        }
    }
    cases
}

/// One episode: its case, features, and the text the embeddings arm reads.
struct Ep {
    case: usize,
    features: BTreeSet<String>,
    text: String,
}

/// Each case's episode, if its faulty window shows anything a detector or a template marks.
fn episodes(cases: &[Case], thresholds: &BTreeMap<String, Decimal>, logs: bool) -> Vec<Ep> {
    let inst = canonical_uid(
        &tel::instrument("scraper", "rcaeval-simple-metrics").expect("an instrument"),
    );
    let stream = tel::series("journald", r#"{"unit":"rcaeval"}"#).expect("a stream");
    let stream_uid = canonical_uid(&stream);
    let mut out = Vec::new();
    for (i, c) in cases.iter().enumerate() {
        let t = c.inject_ms;
        let mut members: Vec<UnitCore> = Vec::new();
        let mut text: Vec<String> = Vec::new();
        for (col, pts) in &c.window {
            let Some(theta) = thresholds.get(col) else {
                continue;
            };
            let samples: Vec<Sample> = pts
                .iter()
                .filter_map(|(ts, v)| Sample::new(*ts, None, *v, 0).ok())
                .collect();
            let series = canonical_uid(&tel::series("prom", col).expect("a series"));
            let input = Input {
                series,
                scheme: "prom",
                key: col,
                instrument: inst,
                samples: &samples,
            };
            let mut p = mad::Params::new(t - HALF, t);
            p.threshold = *theta;
            let found = mad::run_with(&input, &samples, t, t + HALF, &p, &opts());
            let mut dirs = BTreeSet::new();
            for u in &found {
                let o = smysl::surface::payload_to_object(u.payload.as_deref().expect("a payload"))
                    .expect("an object");
                if let Some(d) = o.get("tel:direction") {
                    if let smysl::surface::hjson::HValue::Str(s) = &d.value {
                        dirs.insert(s.clone());
                    }
                }
            }
            for d in dirs {
                text.push(format!("anomaly {d} on {col}"));
            }
            members.extend(found);
        }
        let grouping = episode::Params {
            gap_ms: 2 * HALF,
            min_members: 1,
        };
        let grouped = episode::run(&members, t, t + HALF, &grouping);
        if grouped.is_empty() {
            continue;
        }
        assert_eq!(grouped.len(), 1, "one case is one episode");
        let mut units: BTreeMap<Uid, UnitCore> = BTreeMap::new();
        for u in &members {
            units.insert(canonical_uid(u), u.clone());
        }
        for tpl in c.templates.iter().filter(|_| logs) {
            let u = Template {
                stream: stream_uid,
                scheme: "journald".into(),
                key: r#"{"unit":"rcaeval"}"#.into(),
                inst,
                from_ms: t,
                to_ms: t + HALF,
                template: tpl.clone(),
                count: 1,
                digest: [0; 32],
            }
            .unit();
            units.insert(canonical_uid(&u), u);
            text.push(format!("log {tpl}"));
        }
        let g = &grouped[0];
        units.insert(canonical_uid(&g.unit), g.unit.clone());
        let feats = similar::episodes(units.iter(), g.relations.iter());
        let (_, features) = feats.into_iter().next().expect("the episode");
        text.sort();
        text.dedup();
        out.push(Ep {
            case: i,
            features,
            text: text.join(". "),
        });
    }
    out
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum();
    let na: f64 = a.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    let nb: f64 = b.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// The two readings of one arm, its ranking given as each episode's others in order.
fn readings(cases: &[Case], eps: &[Ep], rank: impl Fn(usize) -> Vec<usize>) -> (f64, f64) {
    let (mut rec, mut ty) = (0.0, 0.0);
    for (k, e) in eps.iter().enumerate() {
        let me = &cases[e.case];
        let top: Vec<&Case> = rank(k)
            .into_iter()
            .take(5)
            .map(|j| &cases[eps[j].case])
            .collect();
        let siblings = top
            .iter()
            .filter(|c| c.service == me.service && c.fault == me.fault)
            .count();
        rec += siblings as f64 / 2.0;
        ty += top.iter().filter(|c| c.fault == me.fault).count() as f64 / 5.0;
    }
    // A case with no episode finds none of its siblings and is in no one's type reading.
    (rec / cases.len() as f64, ty / eps.len().max(1) as f64)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data: Vec<PathBuf> = args
        .windows(2)
        .filter(|w| w[0] == "--data")
        .map(|w| PathBuf::from(&w[1]))
        .collect();
    assert!(!data.is_empty(), "--data PATH/RE2-XX, once or more");
    let started = std::time::Instant::now();
    let model = std::env::var_os("SMYSL_EMBED_MODEL")
        .map(|dir| smysl::EmbedModel::from_dir(&dir).expect("a Model2Vec model directory"));
    println!("GE-M8 on RCAEval (real, CC BY 4.0): do repeated faults recur detectably?");
    for d in &data {
        let name = d.file_name().expect("a name").to_string_lossy().to_string();
        eprintln!("{name}:");
        let cases = load_all(d);
        // Each column's threshold from its own fault-free pairs, to 10 %.
        let target =
            calibrate::Target::new(Decimal::parse_token("0.1").expect("0.1")).expect("a target");
        let mut by_col: BTreeMap<&str, Vec<Option<Decimal>>> = BTreeMap::new();
        for c in &cases {
            for (col, s) in &c.normal {
                by_col.entry(col).or_default().push(*s);
            }
        }
        let spec = mad_spec();
        let mut refused = 0;
        let thresholds: BTreeMap<String, Decimal> = by_col
            .iter()
            .filter_map(|(col, s)| match calibrate::from_scores(&spec, s, target) {
                Ok(c) => Some((col.to_string(), c.value)),
                Err(_) => {
                    refused += 1;
                    None
                }
            })
            .collect();
        let eps = episodes(
            &cases,
            &thresholds,
            !args.iter().any(|a| a == "--no-templates"),
        );
        // MinHash ranks by uid on ties; an episode's stand-in uid is its index, big-endian.
        let uid_of = |k: usize| {
            let mut b = [0u8; 32];
            b[..8].copy_from_slice(&(k as u64).to_be_bytes());
            Uid::from_bytes(b)
        };
        let fmap: BTreeMap<Uid, BTreeSet<String>> = eps
            .iter()
            .enumerate()
            .map(|(k, e)| (uid_of(k), e.features.clone()))
            .collect();
        let idx_of: BTreeMap<Uid, usize> = (0..eps.len()).map(|k| (uid_of(k), k)).collect();
        let mh = readings(&cases, &eps, |k| {
            similar::nearest(&fmap, &uid_of(k), 5, 0)
                .into_iter()
                .map(|n| idx_of[&n.uid])
                .collect()
        });
        let jac = readings(&cases, &eps, |k| {
            let mut v: Vec<(usize, (usize, usize))> = (0..eps.len())
                .filter(|j| *j != k)
                .map(|j| (j, similar::jaccard(&eps[k].features, &eps[j].features)))
                .collect();
            v.sort_by(|x, y| {
                (y.1 .0 * x.1 .1)
                    .cmp(&(x.1 .0 * y.1 .1))
                    .then(x.0.cmp(&y.0))
            });
            v.into_iter().map(|(j, _)| j).collect()
        });
        let emb = model.as_ref().map(|m| {
            let vecs: Vec<Vec<f32>> = eps.iter().map(|e| m.embed(&e.text)).collect();
            readings(&cases, &eps, |k| {
                let mut v: Vec<(usize, f64)> = (0..eps.len())
                    .filter(|j| *j != k)
                    .map(|j| (j, cosine(&vecs[k], &vecs[j])))
                    .collect();
                v.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(&y.0)));
                v.into_iter().map(|(j, _)| j).collect()
            })
        });
        let feats: Vec<usize> = eps.iter().map(|e| e.features.len()).collect();
        let mut sorted = feats.clone();
        sorted.sort_unstable();
        let tpl: Vec<usize> = cases.iter().map(|c| c.templates.len()).collect();
        let others = eps.len().saturating_sub(1).max(1) as f64;
        println!(
            "\n{name}: {} cases, {} with an episode; {} columns calibrated, {refused} refused",
            cases.len(),
            eps.len(),
            thresholds.len()
        );
        println!(
            "  features per episode: median {}, {}–{}; new templates per case: median {}",
            sorted.get(sorted.len() / 2).unwrap_or(&0),
            sorted.first().unwrap_or(&0),
            sorted.last().unwrap_or(&0),
            {
                let mut t = tpl.clone();
                t.sort_unstable();
                t.get(t.len() / 2).copied().unwrap_or(0)
            }
        );
        let per_type = cases.iter().filter(|c| c.fault == cases[0].fault).count() as f64;
        println!(
            "  chance: recurrence {:.3}, type {:.3}",
            (5.0 / others).min(1.0),
            (per_type - 1.0) / others
        );
        println!("                      recurrence   type");
        println!("  MinHash (128)         {:.3}     {:.3}", mh.0, mh.1);
        println!("  exact Jaccard         {:.3}     {:.3}", jac.0, jac.1);
        match emb {
            Some(e) => println!(
                "  embeddings            {:.3}     {:.3}  (Model2Vec)",
                e.0, e.1
            ),
            None => println!("  embeddings            not run: SMYSL_EMBED_MODEL is not set"),
        }
        let verdict = match emb {
            Some(e) if mh.1 < 0.6 && e.1 > 0.8 => {
                "MinHash < 0.6 where embeddings > 0.8: model-dependent by default"
            }
            Some(_) => "the stop condition does not fire",
            None => "undecided without the embeddings arm",
        };
        println!("  verdict               {verdict}");
    }
    println!("\nrun {:.1} s", started.elapsed().as_secs_f64());
}
