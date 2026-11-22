//! The info half of the `tel-evidence` pass (RFC SMYSL-5 §7): each `x.tel/info` unit recomputed
//! from its recipe and what it read, by the code that writes units, and compared by uid.
//!
//! - `SMY-E604` when what it read is here and the unit recomputed is another one, or is refused.
//!   The message names the fields that differ.
//! - `SMY-W605` when what it read is not all here, and nothing else: a statistic's status does
//!   not depend on the substrate being present (rule TL). For `tent/1` what it read is its
//!   template units; only when those are missing are the events looked for.
//! - `SMY-W704` once per recipe id this build does not have; those units are kept and not
//!   recomputed.
//!
//! A unit whose payload does not read, or whose recipe is not one this build writes, is rule TI's
//! (`SMY-E603`) or recomputation's: the second is `SMY-E604`, since recomputing it would compute
//! something else.

use std::collections::BTreeMap;

use smysl_core::diag::{Code, Diagnostic};
use smysl_core::schemas::tel::{self, InfoFields};
use smysl_core::{canonical_uid, Uid, UnitCore};
use smysl_series::{Backend, Coverage, Substrate};

use super::{logs as tent, Outcome, Spec, With, ENT, JSD, MI, PERM, TENT};
use crate::logs::template_fields;

/// Recompute each info unit of `infos`; `templates` are the store's `x.tel/log-template` units,
/// which `tent/1` units are recomputed from.
pub fn verify<'a, B: Backend>(
    infos: impl IntoIterator<Item = &'a UnitCore>,
    templates: &[&'a UnitCore],
    substrate: &Substrate<B>,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    // Recipe ids this build does not have: the first unit of each, and how many.
    let mut unknown: BTreeMap<String, (Uid, usize)> = BTreeMap::new();
    for unit in infos {
        if unit.schema.as_str() != tel::INFO {
            continue;
        }
        let uid = canonical_uid(unit);
        let Some(f) = unit
            .payload
            .as_deref()
            .and_then(|p| tel::info_fields(p).ok())
        else {
            continue;
        };
        let Some(recipe) = f.recipe.as_deref() else {
            continue;
        };
        let id = recipe.split_once('?').map_or(recipe, |(id, _)| id);
        if ![ENT, PERM, MI, JSD, TENT].contains(&id) {
            unknown.entry(id.to_string()).or_insert((uid, 0)).1 += 1;
            continue;
        }
        let e604 = |why: String| Diagnostic::on(Code::E604, uid).with_message(why);
        let spec = match Spec::from_recipe(recipe) {
            Ok(s) => s,
            Err(r) => {
                out.push(e604(format!("its recipe cannot be run: {}", r.message)));
                continue;
            }
        };
        match recompute(&spec, &f, templates, substrate) {
            Again::Absent(why) => {
                out.push(Diagnostic::on(Code::W605, uid).with_message(why));
            }
            Again::Failed(why) => out.push(e604(format!("it could not be recomputed: {why}"))),
            Again::Outcomes(outcomes) => {
                if let Some(why) = mismatch(uid, &f, &outcomes) {
                    out.push(e604(why));
                }
            }
            Again::Skip => {}
        }
    }
    for (id, (uid, n)) in unknown {
        out.push(Diagnostic::on(Code::W704, uid).with_message(format!(
            "{n} info unit(s) by `{id}`, which this build does not have: kept, not recomputed"
        )));
    }
    out
}

enum Again {
    /// What the unit read is not all here.
    Absent(String),
    /// The substrate could not be read, or the statistic could not be run.
    Failed(String),
    /// The outcomes of running the recipe over what it read.
    Outcomes(Vec<Outcome>),
    /// A field the recomputation needs is missing: rule TI's to report.
    Skip,
}

fn uid_of(s: Option<&str>) -> Option<Uid> {
    s?.parse().ok()
}

/// Whether `c` holds every sample of its window: `None` when it does.
fn gap(
    c: Result<Coverage, impl std::fmt::Display>,
    what: &str,
    (a, b): (u64, u64),
) -> Option<String> {
    match c {
        Err(e) => Some(format!("the substrate could not be read: {e}")),
        Ok(c) if c.is_absent() => Some(format!(
            "the substrate holds nothing of {what} [{a}, {b}): not re-derivable"
        )),
        Ok(c) if !c.evicted.is_empty() => Some(format!(
            "{} span(s) of {what} [{a}, {b}) were evicted: not re-derivable",
            c.evicted.len()
        )),
        Ok(_) => None,
    }
}

fn recompute<B: Backend>(
    spec: &Spec,
    f: &InfoFields,
    templates: &[&UnitCore],
    sub: &Substrate<B>,
) -> Again {
    let (Some(series), Some(inst), Some(from), Some(to)) = (
        uid_of(f.series.as_deref()),
        uid_of(f.instrument.as_deref()),
        f.from_ms,
        f.to_ms,
    ) else {
        return Again::Skip;
    };
    if let super::Stat::Tent { templates: hash } = &spec.stat {
        let stored: Vec<&UnitCore> = templates
            .iter()
            .copied()
            .filter(|u| {
                u.payload
                    .as_deref()
                    .and_then(template_fields)
                    .is_some_and(|t| {
                        (t.stream, t.inst, t.from, t.to) == (series, inst, from, to)
                            && tent::recipe_hash(&t.recipe) == *hash
                    })
            })
            .collect();
        if stored.is_empty() {
            if *hash != tent::shipped() {
                return Again::Absent(
                    "its templates are not in the store, and this build runs another Drain: \
                     not re-derivable"
                        .to_string(),
                );
            }
            let events = sub.events_present(&series, &inst, from, to);
            if let Some(why) = gap(events, "its templates, and the events", (from, to)) {
                return Again::Absent(why);
            }
        }
        return match tent::run_on(spec, sub, stored, &[series], Some(inst), from, to) {
            Ok(run) => Again::Outcomes(run.outcomes),
            Err(e) => Again::Failed(e),
        };
    }
    let mut reads = vec![(series, inst, (from, to), "the window")];
    if let Some(r) = spec.reference() {
        reads.push((series, inst, r, "the reference window"));
    }
    let with = if spec.pairs() {
        let (Some(w), Some(wi)) = (
            uid_of(f.with.as_deref()),
            uid_of(f.with_instrument.as_deref()),
        ) else {
            return Again::Skip;
        };
        reads.push((w, wi, (from, to), "the second series' window"));
        Some(With {
            series: w,
            instrument: Some(wi),
        })
    } else {
        None
    };
    for (s, i, (a, b), what) in reads {
        if let Some(why) = gap(sub.present(&s, &i, a, b), what, (a, b)) {
            return Again::Absent(why);
        }
    }
    match super::run_on(spec, sub, &[series], Some(inst), with, from, to) {
        Ok(o) => Again::Outcomes(o),
        Err(e) => Again::Failed(e),
    }
}

/// `None` when one outcome is the unit; otherwise what differs in the nearest one: the one that
/// read the same samples, or else the first.
fn mismatch(uid: Uid, f: &InfoFields, outcomes: &[Outcome]) -> Option<String> {
    let computed: Vec<&UnitCore> = outcomes
        .iter()
        .filter_map(|o| o.result.as_ref().ok().map(|c| &c.unit))
        .collect();
    if computed.iter().any(|u| canonical_uid(u) == uid) {
        return None;
    }
    let fields = |u: &UnitCore| {
        u.payload
            .as_deref()
            .and_then(|p| tel::info_fields(p).ok())
            .unwrap_or_default()
    };
    let near = computed
        .iter()
        .find(|u| fields(u).digest == f.digest)
        .or(computed.first());
    let Some(u) = near else {
        return Some(match outcomes.first().map(|o| &o.result) {
            Some(Err(r)) => format!("recomputed, it is refused: {r}"),
            _ => "recomputed, nothing is computed".to_string(),
        });
    };
    let g = fields(u);
    let mut differ = Vec::new();
    for (same, name) in [
        (g.digest == f.digest, "digest"),
        (g.with_digest == f.with_digest, "with_digest"),
        (g.ref_digest == f.ref_digest, "ref_digest"),
        (g.n == f.n, "n"),
        (g.lag == f.lag, "lag"),
    ] {
        if !same {
            differ.push(name);
        }
    }
    if differ.is_empty() {
        differ.push("value or another statistic field");
    }
    Some(format!(
        "recomputed from what it read, the unit's {} differ(s)",
        differ.join(", ")
    ))
}
