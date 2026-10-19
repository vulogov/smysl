//! A3, novelty (SMYSL-2.6 §3.8): how much of what a scope says is said for the first time, in
//! the order it is said.
//!
//! Units in order, ties by uid; `new(i) = 1` when unit i's proposition class has not been
//! seen before it. The **curve** is the share of new units in 64 bins of equal count
//! (fewer when there are fewer units than bins), each stored as `[new, units in the bin]`;
//! the **integral**, the share of new units over the whole sequence, is `[classes, units]`,
//! which is what a class count over the same units gives — every class is new exactly once.
//!
//! # Two orders
//!
//! `params.order` is `reading` or `time`, and `reading` is the default: a book's claims have a
//! reading order whether or not anyone dated them, and on a Bible the said-axis time of every
//! verse is the edition's.
//!
//! - **reading**: by the text a unit is drawn from (its alias: the manifest it was read under
//!   when its source names one, else the head manifest listing its part), the part's place in
//!   that manifest, the first byte of the unit's span, and its locator. A unit drawn from no
//!   part has no place to read it at; it is excluded and counted (`n.unplaced`).
//! - **time**: by effective time on the recipe's `axis` (rule E, `smysl_text::time`), the
//!   lower bound and then the upper. An undated unit is excluded and counted (`SMY-W415`), and
//!   a scope whose undated share is above `undated:max` is refused (`SMY-E482`): a curve over
//!   the dated tenth of a scope is a curve about which units happened to be dated.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::Code;
use smysl_core::ids::Tid;
use smysl_core::Uid;
use smysl_graph::Store;
use smysl_text::locator::{self, Locator};
use smysl_text::time::engine::effective;
use smysl_text::time::{Bound, Subject};

use crate::graph::{self, redundancy};
use crate::recipe::{Axis, Param};
use crate::{Input, Measure, Needs, Num, Outcome, Purity, Recipe, Refusal, Value};

/// A3.
#[derive(Debug, Clone, Copy, Default)]
pub struct A3;

/// Bins in the curve.
pub const BINS: usize = 64;

/// The flag a result carries when undated units were left out of it; `SMY-W415` says how many.
pub const UNDATED_EXCLUDED: &str = "undated-excluded";

/// The recipe's axis as rule E's. `known` — when a store came to hold a unit — is not one rule
/// E derives (A-12.2 has three), so a measure over effective time refuses it (`SMY-E489`).
pub(crate) fn axis(recipe: &Recipe) -> Result<smysl_core::types::Axis, Refusal> {
    match recipe.axis {
        Axis::Said => Ok(smysl_core::types::Axis::Said),
        Axis::Composed => Ok(smysl_core::types::Axis::Composed),
        Axis::About => Ok(smysl_core::types::Axis::About),
        other => Err(Refusal::new(
            Code::E489,
            format!(
                "{} reads effective time, which rule E derives on said, composed and about;                  `{other}` is not one of them",
                recipe.id
            ),
        )),
    }
}

/// A reading position: the text, the part's place in it, the span's first byte, the locator.
type Place = (String, usize, Option<u64>, Option<Locator>);

fn places(store: &Store, uids: &[Uid]) -> BTreeMap<Uid, Place> {
    let mut at: BTreeMap<Tid, (String, usize)> = BTreeMap::new();
    for (mid, m) in store.manifests() {
        if store.is_superseded(mid) {
            continue;
        }
        for (k, p) in m.parts.iter().enumerate() {
            // A part two texts share is read where it comes first in alias order.
            match at.get(&p.tid) {
                Some((a, _)) if *a <= m.alias => {}
                _ => {
                    at.insert(p.tid, (m.alias.clone(), k));
                }
            }
        }
    }
    let mut out = BTreeMap::new();
    for u in uids {
        let Some(source) = store.get(u).and_then(|x| x.core.source.as_ref()) else {
            continue;
        };
        let Some(tid) = Store::part_of_reference(&source.reference) else {
            continue;
        };
        // The manifest the unit was read under says which text it is in, when it says.
        let placed = match source.manifest().and_then(|m| store.manifest(m)) {
            Some(m) => m
                .parts
                .iter()
                .position(|p| p.tid == tid)
                .map(|k| (m.alias.clone(), k)),
            None => at.get(&tid).cloned(),
        };
        let Some((alias, k)) = placed else {
            continue;
        };
        let loc = source
            .reference
            .split_once('#')
            .and_then(|(_, l)| locator::parse(l).ok());
        out.insert(*u, (alias, k, source.span().map(|(a, _)| a), loc));
    }
    out
}

/// What ordering gave: the ordered units and how many were left out.
struct Ordered {
    uids: Vec<Uid>,
    excluded: u64,
}

fn order(recipe: &Recipe, store: &Store, uids: &[Uid]) -> Result<Ordered, Refusal> {
    match recipe.params.get("order") {
        Some(Param::Text(t)) if t == "reading" => {
            let p = places(store, uids);
            let mut keyed: Vec<(&Place, Uid)> = p.iter().map(|(u, k)| (k, *u)).collect();
            keyed.sort();
            Ok(Ordered {
                uids: keyed.into_iter().map(|(_, u)| u).collect(),
                excluded: (uids.len() - p.len()) as u64,
            })
        }
        Some(Param::Text(t)) if t == "time" => {
            let eff = effective(store, axis(recipe)?);
            let mut keyed: Vec<((i64, i64), Uid)> = Vec::new();
            for u in uids {
                let iv = eff.interval(&Subject::Unit(*u));
                if iv.is_undated() {
                    continue;
                }
                let lo = match iv.lo {
                    Bound::At(a) => a.ms(),
                    Bound::Open => i64::MIN,
                };
                let hi = match iv.hi {
                    Bound::At(a) => a.ms(),
                    Bound::Open => i64::MAX,
                };
                keyed.push(((lo, hi), *u));
            }
            let undated = (uids.len() - keyed.len()) as u64;
            let (num, den) = recipe.undated_max;
            // undated / units > num / den, in integers.
            if u128::from(undated) * u128::from(den) > u128::from(num) * uids.len() as u128 {
                return Err(Refusal::new(
                    Code::E482,
                    format!(
                        "{} of {} units in `{}` have no time on the {} axis, above the recipe's \
                         undated:max of {num}/{den}",
                        undated,
                        uids.len(),
                        recipe.scope.trim(),
                        recipe.axis
                    ),
                ));
            }
            keyed.sort();
            Ok(Ordered {
                uids: keyed.into_iter().map(|(_, u)| u).collect(),
                excluded: undated,
            })
        }
        other => Err(Refusal::new(
            Code::E489,
            format!(
                "A3's `order` is reading or time, not {}",
                match other {
                    Some(Param::Text(t)) => format!("`{t}`"),
                    Some(p) => format!("{p:?}"),
                    None => "unset".to_string(),
                }
            ),
        )),
    }
}

impl Measure for A3 {
    fn id(&self) -> &'static str {
        "A3"
    }

    fn version(&self) -> u16 {
        1
    }

    fn purity(&self) -> Purity {
        Purity::Pure
    }

    fn needs(&self) -> Needs {
        Needs::UNITS.union(Needs::CLASSES).union(Needs::TIME)
    }

    fn complete(&self, recipe: &mut Recipe) {
        graph::complete(recipe);
        recipe
            .params
            .entry("order".into())
            .or_insert_with(|| Param::Text("reading".into()));
    }

    fn admit(&self, recipe: &Recipe, input: &Input<'_>) -> Result<(), Refusal> {
        redundancy::policy(recipe)?;
        graph::admitted(recipe, input).map(|_| ())
    }

    fn run(&self, recipe: &Recipe, input: &Input<'_>) -> Outcome {
        let store = input.store;
        let run = || -> Result<Value, Refusal> {
            let policy = redundancy::policy(recipe)?;
            let uids = graph::admitted(recipe, input)?;
            let ordered = order(recipe, store, &uids)?;
            let n = ordered.uids.len();
            let reading =
                matches!(recipe.params.get("order"), Some(Param::Text(t)) if t == "reading");
            let mut v = Value::new().with_n("units", uids.len() as u64).with_n(
                if reading { "unplaced" } else { "undated" },
                ordered.excluded,
            );
            if n == 0 {
                return Err(Refusal::new(
                    Code::W485,
                    format!(
                        "A3 found no unit of `{}` it could put in {} order",
                        recipe.scope.trim(),
                        if reading { "reading" } else { "time" }
                    ),
                ));
            }
            if !reading && ordered.excluded > 0 {
                v = v.with_flag(UNDATED_EXCLUDED);
            }
            // Classes over the units that were ordered: a class whose only members were left
            // out is not in the sequence, and would otherwise be new nowhere.
            let class = redundancy::class_of(store, policy, &ordered.uids);
            let mut seen: BTreeSet<Uid> = BTreeSet::new();
            let new: Vec<bool> = ordered.uids.iter().map(|u| seen.insert(class[u])).collect();
            let bins = BINS.min(n);
            let curve: Vec<Num> = (0..bins)
                .map(|j| {
                    let (a, b) = (j * n / bins, (j + 1) * n / bins);
                    let k = new[a..b].iter().filter(|x| **x).count() as u64;
                    Num::Ratio(k, (b - a) as u64)
                })
                .collect();
            Ok(v.with_n("ordered", n as u64)
                .with_n("classes", seen.len() as u64)
                .with_value("novelty", Num::Ratio(seen.len() as u64, n as u64))
                .with_series("curve", curve)
                .with_headline_in("novelty", "novelty", ""))
        };
        match run() {
            Ok(v) => Outcome::Value(v),
            Err(r) => Outcome::refused(r),
        }
    }
}
