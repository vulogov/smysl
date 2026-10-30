//! A5, grounding (SMYSL-2.6 §3.8): how much of a scope rests on something, how deep, and how
//! much of it nothing holds up any more — and **E6's warrant density**, moved here from
//! `smysl-eval` (§4.3), which now calls [`warrant_counts`].
//!
//! - **grounded**: `[claims with at least one ground, claims]`, a claim being a unit of the
//!   kernel type `claim`.
//! - **warranted**: E6's ratio, `[units with grounds that a warrant points at, units with
//!   grounds]`, over every unit in scope, as E6 counts it over every survivor.
//! - **unfounded**: `[unfounded units, units]`.
//! - **depth**: the longest chain of `grounds` and `deps` below each claim, its maximum and
//!   lower median.
//!
//! # Depth is the longest chain, which is what memoising can compute
//!
//! §3.8 names `trace(.., TraceKind::Grounds, None)` per claim, memoised in topological order so
//! that the total is O(U + E). The two do not compute the same number. `trace` walks breadth
//! first and marks what it has seen, so its depth is the **shortest** distance to the farthest
//! unit below — and the shortest distance does not compose: a unit's is not one more than its
//! grounds'. The longest chain does compose, `depth(u) = 1 + max depth(g)` over u's grounds and
//! deps, so it is what the memo holds, and over a tree, where there is one path to each unit,
//! the two agree. They differ on a shortcut: `a` grounded on `b` and `c`, `b` on `c`, is depth 2
//! here and 1 to `trace`. A chain of reasons two long with a shortcut beside it is two long.
//!
//! The walk is over the store, not the scope: what a claim rests on is a fact about the store,
//! and a ground outside the scope is still a ground. A ground the store does not hold ends the
//! chain without adding to it, as it does for `trace`. Identity makes the graph acyclic — a
//! unit names its grounds by uid, so a ground cannot name the unit — and the walk is iterative,
//! so a deep chain does not reach the stack's limit.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::types::RelKind;
use smysl_core::{KernelType, SchemaId, Uid};
use smysl_graph::Store;

use crate::graph;
use crate::{Input, Measure, Needs, Num, Outcome, Purity, Recipe, Refusal, Value};

/// A5.
#[derive(Debug, Clone, Copy, Default)]
pub struct A5;

/// E6's two counts over `uids`: `(warranted, grounded)`.
///
/// A unit counts as **grounded** when its core has a ground, and as **warranted** when it is
/// grounded and some `warrant` relation in the store points at it. A uid the store does not
/// hold counts as neither. E6's own loop asked the store for every warrant once per unit; the
/// targets are collected once here, which gives the same two numbers in O(U + R).
pub fn warrant_counts<'a>(
    store: &Store,
    uids: impl IntoIterator<Item = &'a Uid>,
) -> (usize, usize) {
    let targets: BTreeSet<Uid> = store
        .relations_of_kind(&RelKind::Warrant)
        .into_iter()
        .map(|r| r.to)
        .collect();
    let (mut warranted, mut grounded) = (0, 0);
    for uid in uids {
        let Some(unit) = store.get(uid) else {
            continue;
        };
        if unit.core.grounds.is_empty() {
            continue;
        }
        grounded += 1;
        if targets.contains(uid) {
            warranted += 1;
        }
    }
    (warranted, grounded)
}

/// The longest chain of `grounds` and `deps` below each of `roots`, through the store.
pub fn depths(store: &Store, roots: &[Uid]) -> BTreeMap<Uid, u64> {
    let mut memo: BTreeMap<Uid, u64> = BTreeMap::new();
    for root in roots {
        if memo.contains_key(root) {
            continue;
        }
        // Post-order without recursion: a unit is finished when everything below it is.
        let mut stack: Vec<(Uid, bool)> = vec![(*root, false)];
        while let Some((u, expanded)) = stack.pop() {
            if memo.contains_key(&u) {
                continue;
            }
            let Some(unit) = store.get(&u) else {
                continue;
            };
            let below = unit.core.grounds.iter().chain(&unit.core.deps);
            if expanded {
                let d = below
                    .filter_map(|g| match store.get(g) {
                        Some(_) => memo.get(g).map(|d| d + 1),
                        None => None,
                    })
                    .max()
                    .unwrap_or(0);
                memo.insert(u, d);
            } else {
                stack.push((u, true));
                for g in below {
                    if !memo.contains_key(g) && store.contains_uid(g) {
                        stack.push((*g, false));
                    }
                }
            }
        }
    }
    roots
        .iter()
        .map(|r| (*r, memo.get(r).copied().unwrap_or(0)))
        .collect()
}

fn is_claim(store: &Store, uid: &Uid) -> bool {
    store
        .get(uid)
        .is_some_and(|u| u.core.schema == SchemaId::Kernel(KernelType::Claim))
}

impl Measure for A5 {
    fn id(&self) -> &'static str {
        "A5"
    }

    fn version(&self) -> u16 {
        1
    }

    fn purity(&self) -> Purity {
        Purity::Pure
    }

    fn needs(&self) -> Needs {
        Needs::UNITS
    }

    fn complete(&self, recipe: &mut Recipe) {
        graph::complete(recipe);
    }

    fn admit(&self, recipe: &Recipe, input: &Input<'_>) -> Result<(), Refusal> {
        graph::admitted(recipe, input).map(|_| ())
    }

    fn run(&self, recipe: &Recipe, input: &Input<'_>) -> Outcome {
        let uids = match graph::admitted(recipe, input) {
            Ok(u) => u,
            Err(r) => return Outcome::refused(r),
        };
        let store = input.store;
        let claims: Vec<Uid> = uids
            .iter()
            .filter(|u| is_claim(store, u))
            .copied()
            .collect();
        let grounded = claims
            .iter()
            .filter(|u| store.get(u).is_some_and(|x| !x.core.grounds.is_empty()))
            .count() as u64;
        let (warranted, with_grounds) = warrant_counts(store, &uids);
        let unfounded = uids.iter().filter(|u| store.is_unfounded(u)).count() as u64;
        let units = uids.len() as u64;

        let mut v = Value::new()
            .with_n("units", units)
            .with_n("claims", claims.len() as u64)
            .with_n("grounded", with_grounds as u64)
            .with_value("unfounded", Num::Ratio(unfounded, units));
        // A ratio with nothing under it is not written (§2.1): a scope without claims has no
        // grounded share, and one without grounds no warrant density.
        if with_grounds > 0 {
            v = v.with_value(
                "warranted",
                Num::Ratio(warranted as u64, with_grounds as u64),
            );
        }
        if !claims.is_empty() {
            let mut d: Vec<u64> = depths(store, &claims).into_values().collect();
            d.sort_unstable();
            v = v
                .with_value("grounded", Num::Ratio(grounded, claims.len() as u64))
                .with_value("depth:max", Num::Count(*d.last().unwrap_or(&0)))
                .with_value("depth:median", Num::Count(graph::median(&d).unwrap_or(0)))
                .with_headline_in("grounded", "grounded share of claims", "");
        } else {
            v = v.with_headline_in("unfounded", "unfounded share", "");
        }
        Outcome::Value(v)
    }
}
