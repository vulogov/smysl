//! GE-M6 (RFC SMYSL-3 §7, D-T10): do independent instruments change conclusions?
//!
//! D-T10 keeps witness counts out of every status rule, and in dossiers only, "unless GE-M6 shows
//! it changes a conclusion". The conclusion this tests is the one the analysis layer draws: the
//! BARO ranking (`detect::baro`), whose top five become hypotheses. "If witness counting never
//! changes a ranking, it stays a report."
//!
//! **What this runs.** No cluster with duplicated scrapers and independent sources is on this
//! machine, so the incidents are simulated, and the report says so:
//!
//! - `--quantities` gauges, one sample a minute, for an hour before the anomaly and ten minutes
//!   after. Each has its own level and Gaussian noise.
//! - **The fault.** One quantity, the root cause, shifts by 3 to 10 noise widths at the anomaly.
//!   Two others, its symptoms, shift 1 to 5 minutes later by 0.4 to 1.1 of that: a symptom can
//!   move more than its cause, as in a real incident, so plain BARO is not always right.
//! - **The instruments.** Each quantity reaches smysl as one or more series, joined by
//!   `x.tel/same-quantity`, with real `x.tel/instrument` units and `measured-by`/`via` edges, and
//!   `LineageGraph` (§7) counts its witnesses:
//!   - *single*: one exporter, one scraper, one series.
//!   - *duplicated*: one exporter, two scrapers, and two series (a federation or remote-write
//!     copy with a replica label): the same values twice, one witness.
//!   - *independent*: two exporters (node_exporter and the application's own metric, on another
//!     scale with its own noise), two series, two witnesses.
//! - **A blind witness.** One of the root cause's two instruments does not see the fault (it
//!   measures from a cache, or upstream of where the fault is). The counter-case to the
//!   artifact: here the instrument that moved alone is right.
//! - **An instrument artifact.** One origin of a quantity that is not the root cause reports a
//!   shift of 3 to 12 noise widths that the quantity never made (a broken exporter, a unit
//!   change). Its independent twin, if it has one, does not see it.
//!
//! Six arms, `--incidents` each:
//!
//! - *control*: every quantity single.
//! - *duplicated*: each quantity duplicated with probability ½.
//! - *independent*: every quantity measured twice.
//! - *artifact*: independent, plus one artifact per incident.
//! - *blind*: independent, and one witness of the root cause blind in every incident.
//! - *mixed*: per quantity, ½ single, ¼ duplicated, ¼ independent; an artifact in 30 % of
//!   incidents, and a blind witness of the root cause (if it has two) in 30 %.
//!
//! **Three rankings** of the same scores, each read as the list of quantities its top five name:
//!
//! - **plain**: BARO as shipped, one entry per series.
//! - **collapse**: one entry per witness (a class of dependent series, at its highest score).
//!   Duplicates collapse; independent witnesses stay two entries.
//! - **corroborate**: one entry per quantity, at the lowest score among its witnesses: a quantity
//!   ranks high only if every independent instrument saw it move.
//!
//! For each it reports how often the root cause is first (hit@1) and in the top three (hit@3),
//! and how often it differs from plain's:
//!
//! - **top-1 Δ**: the first quantity named.
//! - **order Δ**: the conclusion: the first three distinct quantities, in order. Three quantities
//!   move in each incident (the root cause and two symptoms); past them the order is noise.
//! - **slots Δ**: the five entries as written, duplicates included. A duplicate that only takes
//!   a slot changes this and not the conclusion.
//!
//! Exit 0 when the run completes (it decides, it does not pass or fail); the verdict is printed.
//!
//! `make ge-m6` runs it with the defaults.

// Simulated input, not a stored number: the generators draw Box–Muller normals and exponential
// gaps with the platform's `ln` and `exp`, as they did before `clippy.toml` (SMYSL-5 D-I4)
// reached this crate. Routing them through `libm` would move every simulated sample and every
// result recorded from them.
#![allow(clippy::disallowed_methods)]

use std::collections::BTreeMap;

use smysl_core::schemas::tel;
use smysl_core::{canonical_uid, Relation, Uid, UnitCore};
use smysl_tel::detect::baro;

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

    /// Uniform in `[0, 1)`.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// Standard normal, by Box–Muller.
    fn normal(&mut self) -> f64 {
        let u = self.unit().max(f64::MIN_POSITIVE);
        let v = self.unit();
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

struct Args {
    incidents: usize,
    quantities: usize,
    seed: u64,
}

fn args() -> Args {
    let mut a = Args {
        incidents: 2_000,
        quantities: 12,
        seed: 6,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let v = it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        let n = || v.parse::<u64>().unwrap_or_else(|_| panic!("{flag}: `{v}`"));
        match flag.as_str() {
            "--incidents" => a.incidents = n() as usize,
            "--quantities" => a.quantities = n() as usize,
            "--seed" => a.seed = n(),
            other => panic!("unknown argument {other}"),
        }
    }
    assert!(a.quantities >= 6, "--quantities: at least 6");
    a
}

const MIN: u64 = 60_000;
/// The anomaly instant: an hour of baseline before it.
const AT: u64 = 60 * MIN;
/// Samples run to ten minutes after.
const END: u64 = AT + 10 * MIN;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Single,
    Duplicated,
    Independent,
}

#[derive(Clone, Copy)]
enum Arm {
    Control,
    Duplicated,
    Independent,
    Artifact,
    Blind,
    Mixed,
}

impl Arm {
    const ALL: [Arm; 6] = [
        Arm::Control,
        Arm::Duplicated,
        Arm::Independent,
        Arm::Artifact,
        Arm::Blind,
        Arm::Mixed,
    ];

    fn name(self) -> &'static str {
        match self {
            Arm::Control => "control",
            Arm::Duplicated => "duplicated",
            Arm::Independent => "independent",
            Arm::Artifact => "artifact",
            Arm::Blind => "blind",
            Arm::Mixed => "mixed",
        }
    }

    fn shape(self, r: &mut Rng) -> Shape {
        match self {
            Arm::Control => Shape::Single,
            Arm::Duplicated if r.unit() < 0.5 => Shape::Duplicated,
            Arm::Duplicated => Shape::Single,
            Arm::Independent | Arm::Artifact | Arm::Blind => Shape::Independent,
            Arm::Mixed => match r.unit() {
                x if x < 0.5 => Shape::Single,
                x if x < 0.75 => Shape::Duplicated,
                _ => Shape::Independent,
            },
        }
    }

    fn artifact(self, r: &mut Rng) -> bool {
        match self {
            Arm::Artifact => true,
            Arm::Mixed => r.unit() < 0.3,
            _ => false,
        }
    }

    /// Whether one witness of the root cause misses the fault.
    fn blind(self, r: &mut Rng) -> bool {
        match self {
            Arm::Blind => true,
            Arm::Mixed => r.unit() < 0.3,
            _ => false,
        }
    }
}

/// A series' samples, `(ts_ms, value)`.
type Points = Vec<(u64, f64)>;

/// One incident's lineage: the units and relations `LineageGraph` reads, and each series'
/// quantity.
#[derive(Default)]
struct World {
    units: Vec<(Uid, UnitCore)>,
    rels: Vec<Relation>,
    /// series uid → quantity index, and its points.
    series: Vec<(Uid, usize, Points)>,
}

impl World {
    fn unit(&mut self, u: UnitCore) -> Uid {
        let id = canonical_uid(&u);
        self.units.push((id, u));
        id
    }

    fn instrument(&mut self, role: &str, key: &str) -> Uid {
        self.unit(tel::instrument(role, key).expect("a well-formed instrument"))
    }

    /// A series of quantity `q`, measured by `origin`, carried through `scraper`.
    fn series(&mut self, key: &str, q: usize, origin: Uid, scraper: Uid, pts: Vec<(u64, f64)>) {
        let s = self.unit(tel::series("prom", key).expect("a well-formed series"));
        self.rels.push(tel::measured_by(s, origin));
        self.rels.push(tel::via(origin, scraper));
        self.series.push((s, q, pts));
    }
}

/// A quantity's true path: level, noise, and its shift (size, start).
struct Truth {
    level: f64,
    shift: Option<(f64, u64)>,
}

/// What one instrument reports of a quantity: the truth (without its shift, if the instrument is
/// `blind` to it), its own noise, its own scale, and an artifact shift of its own if it has one.
fn observe(
    t: &Truth,
    scale: f64,
    artifact: Option<f64>,
    blind: bool,
    r: &mut Rng,
) -> Vec<(u64, f64)> {
    (0..=END / MIN)
        .map(|i| {
            let ts = i * MIN;
            let mut x = t.level + r.normal();
            if let Some((d, from)) = t.shift.filter(|_| !blind) {
                if ts >= from {
                    x += d;
                }
            }
            if let Some(d) = artifact {
                if ts >= AT {
                    x += d;
                }
            }
            (ts, x * scale)
        })
        .collect()
}

/// One incident: its world and its root-cause quantity.
fn incident(arm: Arm, nq: usize, r: &mut Rng) -> (World, usize) {
    let root = r.below(nq);
    let mut symptoms = Vec::new();
    while symptoms.len() < 2 {
        let s = r.below(nq);
        if s != root && !symptoms.contains(&s) {
            symptoms.push(s);
        }
    }
    let size = r.range(3.0, 10.0) * if r.unit() < 0.5 { 1.0 } else { -1.0 };
    let truths: Vec<Truth> = (0..nq)
        .map(|q| Truth {
            level: r.range(10.0, 1000.0),
            shift: if q == root {
                Some((size, AT))
            } else if symptoms.contains(&q) {
                let lag = 1 + r.below(5) as u64;
                Some((size * r.range(0.4, 1.1), AT + lag * MIN))
            } else {
                None
            },
        })
        .collect();
    let shapes: Vec<Shape> = (0..nq).map(|_| arm.shape(r)).collect();
    // The artifact: one origin of a quantity that is not the root cause.
    let artifact = if arm.artifact(r) {
        let mut q = r.below(nq);
        while q == root {
            q = r.below(nq);
        }
        let d = r.range(3.0, 12.0) * if r.unit() < 0.5 { 1.0 } else { -1.0 };
        Some((q, r.below(2), d))
    } else {
        None
    };
    // The blind witness: one origin of the root cause that does not see the fault.
    let blind = arm.blind(r).then(|| r.below(2));
    let mut w = World::default();
    let (prom_a, prom_b) = (
        w.instrument("scraper", "prom-scrape:a"),
        w.instrument("scraper", "prom-scrape:b"),
    );
    for (q, t) in truths.iter().enumerate() {
        let art = |o: usize| artifact.and_then(|(aq, ao, d)| (aq == q && ao == o).then_some(d));
        // Only a quantity with a second witness can have one witness blind.
        let blind = |o: usize| q == root && shapes[q] == Shape::Independent && blind == Some(o);
        let ne = w.instrument("exporter", &format!("prom-target:node/q{q}"));
        let pts = observe(t, 1.0, art(0), blind(0), r);
        w.series(&format!("q{q}"), q, ne, prom_a, pts.clone());
        match shapes[q] {
            Shape::Single => {}
            Shape::Duplicated => {
                // The copy: the same origin and values, a second scraper, another label set.
                w.series(&format!("q{q}{{replica=\"b\"}}"), q, ne, prom_b, pts);
            }
            Shape::Independent => {
                let app = w.instrument("exporter", &format!("prom-target:app/q{q}"));
                let scale = r.range(0.001, 1000.0);
                let pts = observe(t, scale, art(1), blind(1), r);
                w.series(&format!("app_q{q}"), q, app, prom_a, pts);
            }
        }
    }
    // Every series of one quantity, joined to the first.
    let mut first: BTreeMap<usize, Uid> = BTreeMap::new();
    let mut joins = Vec::new();
    for (s, q, _) in &w.series {
        match first.get(q) {
            Some(f) => joins.push(tel::same_quantity(*f, *s)),
            None => {
                first.insert(*q, *s);
            }
        }
    }
    w.rels.extend(joins);
    // The lineage says what the shapes meant to build.
    let g = tel::LineageGraph::of(w.units.iter().map(|(u, c)| (u, c)), &w.rels);
    for (q, f) in &first {
        let want = if shapes[*q] == Shape::Independent {
            2
        } else {
            1
        };
        assert_eq!(g.witnesses(f).count(), want, "quantity {q}'s witnesses");
    }
    (w, root)
}

/// The three rankings, each as the quantities its entries name, best first.
fn rankings(w: &World) -> [Vec<usize>; 3] {
    let p = baro::Params::new(AT);
    let scored: Vec<(Uid, usize, f64)> = w
        .series
        .iter()
        .filter_map(|(s, q, pts)| baro::score(pts, &p).map(|x| (*s, *q, x)))
        .collect();
    let order = |mut v: Vec<(f64, Uid, usize)>| -> Vec<usize> {
        v.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        v.into_iter().map(|e| e.2).collect()
    };
    let plain = order(scored.iter().map(|(s, q, x)| (*x, *s, *q)).collect());
    let score: BTreeMap<Uid, f64> = scored.iter().map(|(s, _, x)| (*s, *x)).collect();
    let g = tel::LineageGraph::of(w.units.iter().map(|(u, c)| (u, c)), &w.rels);
    let mut collapse = Vec::new();
    let mut corroborate = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (s, q, _) in &scored {
        if !seen.insert(*q) {
            continue;
        }
        let wit = g.witnesses(s);
        let classes: Vec<(f64, Uid)> = wit
            .witnesses
            .iter()
            .map(|c| {
                let best = c
                    .iter()
                    .filter_map(|m| score.get(m).copied())
                    .fold(f64::NEG_INFINITY, f64::max);
                (best, c[0])
            })
            .collect();
        for (x, u) in &classes {
            collapse.push((*x, *u, *q));
        }
        let (lo, u) = classes
            .iter()
            .copied()
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .expect("a quantity has a witness");
        corroborate.push((lo, u, *q));
    }
    [plain, order(collapse), order(corroborate)]
}

const TOP: usize = 5;

#[derive(Default, Clone, Copy)]
struct Tally {
    hit1: usize,
    hit3: usize,
    top1_changed: usize,
    top5_changed: usize,
    slots_changed: usize,
}

fn pct(n: usize, of: usize) -> String {
    format!("{:5.1} %", 100.0 * n as f64 / of as f64)
}

fn main() {
    let a = args();
    println!("GE-M6 (RFC SMYSL-3 §7, D-T10): do independent instruments change conclusions?");
    println!("SIMULATED: no cluster with duplicated scrapers and independent sources here.");
    println!(
        "{} incidents per arm, {} quantities, seed {}; BARO pre 60 min, post 10 min, top {TOP}.",
        a.incidents, a.quantities, a.seed
    );
    println!();
    println!(
        "{:<12} {:<12} {:>8} {:>8} {:>10} {:>10} {:>10}",
        "arm", "ranking", "hit@1", "hit@3", "top-1 Δ", "order Δ", "slots Δ"
    );
    let names = ["plain", "collapse", "corroborate"];
    let mut changed = [0usize; 3];
    let mut better = [(0i64, 0i64); 3];
    let mut moved: [Vec<(&str, i64)>; 3] = Default::default();
    for (k, arm) in Arm::ALL.into_iter().enumerate() {
        let mut r = Rng(a.seed.wrapping_mul(0x1000).wrapping_add(k as u64));
        let mut t = [Tally::default(); 3];
        for _ in 0..a.incidents {
            let (w, root) = incident(arm, a.quantities, &mut r);
            let rs = rankings(&w);
            // The hypotheses' slots: the top five entries, as written.
            let slots = |v: &[usize]| v.iter().take(TOP).copied().collect::<Vec<_>>();
            // The conclusion: the first three distinct quantities, in order.
            let named = |v: &[usize]| {
                let mut out: Vec<usize> = Vec::new();
                for q in v {
                    if !out.contains(q) {
                        out.push(*q);
                    }
                }
                out.truncate(3);
                out
            };
            for (i, rk) in rs.iter().enumerate() {
                let tally = &mut t[i];
                tally.hit1 += usize::from(rk.first() == Some(&root));
                tally.hit3 += usize::from(rk.iter().take(3).any(|q| *q == root));
                tally.top1_changed += usize::from(rk.first() != rs[0].first());
                tally.top5_changed += usize::from(named(rk) != named(&rs[0]));
                tally.slots_changed += usize::from(slots(rk) != slots(&rs[0]));
            }
        }
        for (i, name) in names.iter().enumerate() {
            println!(
                "{:<12} {:<12} {:>8} {:>8} {:>10} {:>10} {:>10}",
                if i == 0 { arm.name() } else { "" },
                name,
                pct(t[i].hit1, a.incidents),
                pct(t[i].hit3, a.incidents),
                pct(t[i].top1_changed, a.incidents),
                pct(t[i].top5_changed, a.incidents),
                pct(t[i].slots_changed, a.incidents),
            );
            changed[i] += t[i].top5_changed;
            better[i].0 += t[i].hit1 as i64 - t[0].hit1 as i64;
            better[i].1 += t[i].hit3 as i64 - t[0].hit3 as i64;
            moved[i].push((arm.name(), t[i].hit1 as i64 - t[0].hit1 as i64));
        }
    }
    println!();
    for i in 1..3 {
        println!(
            "{:<12} changed {} of {} conclusions; hit@1 {:+}, hit@3 {:+} incidents against plain",
            names[i],
            changed[i],
            a.incidents * Arm::ALL.len(),
            better[i].0,
            better[i].1
        );
    }
    // Where a ranking moved hit@1, by arm: the evidence a status rule would have to stand on.
    for (i, name) in names.iter().enumerate().skip(1) {
        let (up, down): (Vec<&str>, Vec<&str>) = (
            moved[i].iter().filter(|m| m.1 > 0).map(|m| m.0).collect(),
            moved[i].iter().filter(|m| m.1 < 0).map(|m| m.0).collect(),
        );
        println!(
            "{name:<12} hit@1 up in [{}], down in [{}]",
            up.join(", "),
            down.join(", ")
        );
    }
    if changed[1] + changed[2] == 0 {
        println!("verdict: witness counting never changed a conclusion: D-T10 stands");
    } else {
        println!("verdict: witness counting changed conclusions; D-T10 is for a person to decide");
    }
}
