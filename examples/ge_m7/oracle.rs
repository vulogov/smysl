//! The oracle (RFC SMYSL-10 §5.4) and arm (c), BARO's ranking (§4.1).
//!
//! The oracle reads the case's files as loaded (`Raw`), never the substrate or the store, and
//! recomputes what a quote names: an exact field over the window's raw values, a quantile against
//! the exact order statistics within the sketch's `alpha`, a template's count by Drain over the
//! window's renamed lines. It is independent of rule TQ by construction: TQ compares a quote with
//! a unit's payload; the oracle never reads a payload's numbers.

use std::collections::BTreeMap;

use smysl::tel::detect::{baro, Input};
use smysl::{canonical_uid, UnitCore};

use crate::world::{mean_ms, Known, Origin, Quote, World, ALPHA, HALF_S, QS};

/// What the oracle says of one quote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// By the tolerant match: a correct rounding is not a fabrication.
    pub fabricated: bool,
    /// By TQ's spelling: the value is the unit's canonical text.
    pub strict: bool,
    pub why: String,
}

fn digits(v: &str) -> i32 {
    match v.split_once('.') {
        Some((_, f)) => f.chars().take_while(char::is_ascii_digit).count() as i32,
        None => 0,
    }
}

/// `claim` matches `truth` to the digits it gives.
fn close(claim: f64, places: i32, truth: f64) -> bool {
    let half = 0.5 * 10f64.powi(-places);
    (claim - truth).abs() <= half * (1.0 + 1e-9) + 1e-12 * truth.abs().max(1.0)
}

/// The window's raw values, in time order.
fn values(world: &World, origin: &Origin, from: u64, to: u64) -> Vec<f64> {
    let (a, b) = (world.raw.t0 + from, world.raw.t0 + to);
    match origin {
        Origin::Column(c) => world
            .raw
            .metrics
            .get(c)
            .map(|pts| {
                pts.iter()
                    .filter(|(ts, _)| (a..b).contains(ts))
                    .filter_map(|(_, v)| v.parse::<f64>().ok())
                    .filter(|x| x.is_finite())
                    .collect()
            })
            .unwrap_or_default(),
        Origin::Calls(svc, op) | Origin::Latency(svc, op) => {
            let mut per: BTreeMap<u64, (u64, u64)> = BTreeMap::new();
            for (ms, s, o, d) in world.raw.spans.as_deref().unwrap_or(&[]) {
                if s == svc && o == op && (a..b).contains(&(ms / 1000)) {
                    let e = per.entry(ms / 1000).or_default();
                    e.0 += 1;
                    e.1 += d;
                }
            }
            per.values()
                .map(|(c, sum)| match origin {
                    Origin::Calls(..) => *c as f64,
                    _ => mean_ms(*sum, *c).parse().unwrap_or(f64::NAN),
                })
                .collect()
        }
    }
}

fn field_index(field: &str) -> (&str, Option<usize>) {
    match field.strip_suffix(']').and_then(|f| f.split_once('[')) {
        Some((f, i)) => (f, i.parse().ok()),
        None => (field, None),
    }
}

/// Judge one quote.
pub fn judge(world: &World, q: &Quote) -> Verdict {
    let fab = |why: &str| Verdict {
        fabricated: true,
        strict: false,
        why: why.to_string(),
    };
    let Ok(uid) = q.id.parse::<smysl::Uid>() else {
        return fab("not an id");
    };
    let Some((known, unit)) = world.known.get(&uid) else {
        return fab("no tool returned this id");
    };
    let (field, index) = field_index(&q.field);
    let strict = unit
        .payload
        .as_deref()
        .and_then(|p| smysl::schemas::tel::quoted_value(p, field, index))
        .is_some_and(|v| v == q.value);
    let Ok(claim) = q.value.trim().parse::<f64>() else {
        return fab("the value is not a number");
    };
    let places = digits(q.value.trim());
    let ok = match known {
        Known::Window {
            series,
            from,
            to,
            sketched,
        } => {
            let Some(s) = world.series.get(series) else {
                return fab("no such series");
            };
            let v = values(world, &s.origin, *from, *to);
            if v.is_empty() {
                return fab("the window is empty");
            }
            let mut sorted = v.clone();
            sorted.sort_by(f64::total_cmp);
            let n = v.len() as f64;
            let sum: f64 = v.iter().sum();
            match (field, index, sketched) {
                ("count", None, _) => close(claim, places, n),
                ("min", None, false) => close(claim, places, sorted[0]),
                ("max", None, false) => close(claim, places, sorted[v.len() - 1]),
                ("first", None, false) => close(claim, places, v[0]),
                ("last", None, false) => close(claim, places, v[v.len() - 1]),
                ("sum", None, false) => close(claim, places, sum),
                ("mean", None, false) => close(claim, places, sum / n),
                ("q", Some(i), true) if i < QS.len() => {
                    let rank: f64 = QS[i].parse().unwrap();
                    let alpha: f64 = ALPHA.parse().unwrap();
                    let pos = rank * (n - 1.0);
                    let (lo, hi) = (pos.floor() as usize, pos.ceil() as usize);
                    // Within alpha of an order statistic at that rank, or a rounding of such a value.
                    (lo.saturating_sub(1)..=(hi + 1).min(v.len() - 1)).any(|k| {
                        let x = sorted[k];
                        (claim - x).abs() <= alpha * x.abs() * (1.0 + 1e-9)
                            || close(claim, places, x * (1.0 - alpha))
                            || close(claim, places, x * (1.0 + alpha))
                    })
                }
                _ => return fab("the unit has no such field"),
            }
        }
        Known::Template {
            service,
            from,
            to,
            template,
        } => {
            if field != "count" || index.is_some() {
                return fab("a template quotes only its count");
            }
            let Some(orig) = world.anon.from.get(service) else {
                return fab("no such service");
            };
            let (a, b) = (world.ms(*from), world.ms(*to));
            // A line repeated at the same millisecond in the same container is one event: the
            // substrate stores events by content, and the tool counts what it stores. A quote
            // copied from the tool is not a fabrication because the file repeats a line (M7-P2).
            let lines: Vec<String> = world
                .raw
                .logs
                .iter()
                .filter(|(ms, who, _)| who == orig && (a..b).contains(ms))
                .map(|(ms, _, m)| (*ms, m))
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .map(|(_, m)| world.anon.text(m))
                .collect();
            let t = smysl::tel::drain::run(lines.iter().map(String::as_str));
            let n = t.get(template).copied().unwrap_or(0);
            if !close(claim, places, n as f64) {
                return fab(&format!(
                    "Drain over the window's {} lines counts {n}",
                    lines.len()
                ));
            }
            true
        }
        Known::Hypothesis | Known::Ranked => return fab("a hypothesis is not evidence"),
    };
    Verdict {
        fabricated: !ok,
        strict: strict && ok,
        why: if ok {
            String::new()
        } else {
            "the value does not match".into()
        },
    }
}

/// Arm (c): BARO over every metric column, six minutes each side of the alert, every series
/// ranked (R1-2). Its hypotheses, and the first five distinct services in rank order.
pub fn baro(world: &World) -> (Vec<UnitCore>, Vec<String>) {
    let mut data = Vec::new();
    for (name, s) in &world.series {
        if !matches!(s.origin, Origin::Column(_)) {
            continue;
        }
        let read = world
            .sub
            .window(&s.uid, &world.metric_inst, 0, u64::MAX)
            .expect("a window");
        data.push((name.clone(), s.uid, read.samples));
    }
    let inputs: Vec<Input<'_>> = data
        .iter()
        .map(|(name, uid, samples)| Input {
            series: *uid,
            scheme: "prom",
            key: name,
            instrument: world.metric_inst,
            samples,
        })
        .collect();
    let mut p = baro::Params::new(world.ms(world.alert_s()));
    p.pre_ms = HALF_S * 1000;
    p.post_ms = HALF_S * 1000;
    p.top = inputs.len();
    let units = baro::run(&inputs, None, &p);
    let by_uid: BTreeMap<smysl::Uid, &str> = data
        .iter()
        .map(|(n, u, _)| (*u, n.split('/').next().unwrap_or("")))
        .collect();
    let mut ranking: Vec<String> = Vec::new();
    for u in &units {
        let target = u
            .grounds
            .first()
            .and_then(|g| by_uid.get(g))
            .map(|s| s.to_string());
        if let Some(t) = target {
            if !ranking.contains(&t) {
                ranking.push(t);
            }
        }
        if ranking.len() == 5 {
            break;
        }
    }
    (units, ranking)
}

/// The ranking as (d)'s first message lists it: services in order, each with its hypothesis id.
pub fn ranking_text(world: &World, units: &[UnitCore], ranking: &[String]) -> String {
    let mut out = String::from(
        "A ranking computed from the metrics (how far each series moved after the alert, \
         against its median before) is in the ledger:\n",
    );
    for (i, svc) in ranking.iter().enumerate() {
        let first = units.iter().find(|u| {
            u.grounds.first().is_some_and(|g| {
                world
                    .series
                    .iter()
                    .any(|(n, s)| s.uid == *g && n.split('/').next() == Some(svc.as_str()))
            })
        });
        if let Some(u) = first {
            out.push_str(&format!(
                "  {}. {svc}  hypothesis id={}\n",
                i + 1,
                canonical_uid(u).canonical()
            ));
        }
    }
    out
}
