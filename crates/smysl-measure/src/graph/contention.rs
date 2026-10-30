//! A7, contention (SMYSL-2.6 §3.8): how much of a scope is disputed, per hundred units.
//!
//! Two things count, and both are counted once: an **open contention** that touches the scope
//! — its subject or one of its positions is a unit in it (`Store::open_contentions`) — and a
//! **live `x.text/contradicts` relation** with an end in it, live meaning not withdrawn and
//! both ends present. A contention among units of two texts touches both, so a text's figure
//! is over what touches that text's units and the scope's over what touches any.
//!
//! `per100 = (contentions + contradictions) · 100 / units`, rounded once.

use std::collections::BTreeSet;

use smysl_core::schemas::text::CONTRADICTS;
use smysl_core::types::RelKind;
use smysl_core::Uid;
use smysl_graph::Store;

use crate::graph;
use crate::{Input, Measure, Needs, Num, Outcome, Purity, Recipe, Refusal, Value};

/// A7.
#[derive(Debug, Clone, Copy, Default)]
pub struct A7;

/// `(open contentions, live contradictions)` touching `uids`.
pub fn touching(store: &Store, uids: &BTreeSet<Uid>) -> (u64, u64) {
    let contentions = store
        .open_contentions()
        .filter(|c| uids.contains(&c.over) || c.positions.iter().any(|p| uids.contains(p)))
        .count() as u64;
    let contradictions = store
        .relations_of_kind(&RelKind::Extension(CONTRADICTS.to_string()))
        .into_iter()
        .filter(|r| !store.is_withdrawn(r))
        .filter(|r| store.contains_uid(&r.from) && store.contains_uid(&r.to))
        .filter(|r| uids.contains(&r.from) || uids.contains(&r.to))
        .count() as u64;
    (contentions, contradictions)
}

impl Measure for A7 {
    fn id(&self) -> &'static str {
        "A7"
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
        let all: BTreeSet<Uid> = uids.iter().copied().collect();
        let (c, r) = touching(store, &all);
        let units = uids.len() as u64;
        let Some(per100) = graph::scaled(c + r, 100, units) else {
            return Outcome::NotRun {
                why: "the rate does not fit a payload number".into(),
                code: None,
            };
        };
        let mut v = Value::new()
            .with_n("units", units)
            .with_value("contentions", Num::Count(c))
            .with_value("contradictions", Num::Count(r))
            .with_value("per100", Num::Fixed(per100))
            .with_headline_in("per100", "contention", "per 100 units");
        for (alias, members) in graph::by_text(store, &uids) {
            let set: BTreeSet<Uid> = members.iter().copied().collect();
            let (c, r) = touching(store, &set);
            if let Some(x) = graph::scaled(c + r, 100, members.len() as u64) {
                v = v.with_value(format!("per100:{alias}"), Num::Fixed(x));
            }
        }
        Outcome::Value(v)
    }
}
