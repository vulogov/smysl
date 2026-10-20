//! D2, uptake (SMYSL-2.6 §3.10): per speaker, how much of what they assert somebody else takes
//! up.
//!
//! An assertion is a unit of the scope whose `text:mode` is `asserted` and which has a speaker.
//! It is **taken up** when a unit of the scope with a *different* speaker elaborates it, rebuts
//! it or answers it — a live kernel `elaborates`, `rebuts` or `answers` edge from that unit to
//! the assertion — or is joined to it by a live `same-as` edge in either direction, which is a
//! restatement (SMYSL-2.4). `uptake:<speaker> = [taken up, asserted]`, and `uptake` is the same
//! over every speaker. A response from a unit with no speaker is nobody else's, and does not
//! count; neither does a speaker taking up their own assertion.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::Code;
use smysl_core::types::RelKind;
use smysl_core::Uid;
use smysl_graph::proposition::SAME_AS;

use crate::dialogue;
use crate::graph;
use crate::{Input, Measure, Needs, Num, Outcome, Purity, Recipe, Refusal, Value};

/// D2.
#[derive(Debug, Clone, Copy, Default)]
pub struct D2;

impl Measure for D2 {
    fn id(&self) -> &'static str {
        "D2"
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
        let store = input.store;
        let run = || -> Result<Value, Refusal> {
            let uids = graph::admitted(recipe, input)?;
            let scope = dialogue::set(&uids);
            let speakers = dialogue::speakers(store, &uids);
            let asserted: BTreeSet<Uid> = speakers
                .keys()
                .filter(|u| dialogue::mode(store, u).as_deref() == Some("asserted"))
                .copied()
                .collect();
            if asserted.is_empty() {
                return Err(Refusal::new(
                    Code::W485,
                    format!(
                        "D2 found no asserted unit with a speaker in `{}`",
                        recipe.scope.trim()
                    ),
                ));
            }
            // (responder, assertion): a directed response, and a restatement both ways.
            let mut responses: Vec<(Uid, Uid)> = Vec::new();
            for kind in [RelKind::Elaborates, RelKind::Rebuts, RelKind::Answers] {
                responses.extend(dialogue::within(store, &kind, &scope));
            }
            for (a, b) in dialogue::within(store, &RelKind::Extension(SAME_AS.into()), &scope) {
                responses.extend([(a, b), (b, a)]);
            }
            let mut taken: BTreeSet<Uid> = BTreeSet::new();
            for (from, to) in responses {
                if !asserted.contains(&to) {
                    continue;
                }
                if speakers.get(&from).is_some_and(|s| *s != speakers[&to]) {
                    taken.insert(to);
                }
            }
            let mut by: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
            for u in &asserted {
                let e = by.entry(speakers[u].as_str()).or_default();
                e.0 += u64::from(taken.contains(u));
                e.1 += 1;
            }
            let mut v = Value::new()
                .with_n("units", uids.len() as u64)
                .with_n("unspoken", (uids.len() - speakers.len()) as u64)
                .with_n("asserted", asserted.len() as u64)
                .with_n("speakers", by.len() as u64)
                .with_value(
                    "uptake",
                    Num::Ratio(taken.len() as u64, asserted.len() as u64),
                )
                .with_headline_in("uptake", "uptake", "");
            for (s, (k, n)) in by {
                v = v.with_value(format!("uptake:{s}"), Num::Ratio(k, n));
            }
            Ok(v)
        };
        match run() {
            Ok(v) => Outcome::Value(v),
            Err(r) => Outcome::refused(r),
        }
    }
}
