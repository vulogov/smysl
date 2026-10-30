//! D3, persistence (SMYSL-2.6 §3.10): how long a proposition stays in the conversation.
//!
//! Per proposition class under the recipe's policy (`SMY-E481` without one): the **last
//! reference** minus the **first utterance**. The first utterance is the earliest instant of a
//! member; a reference is a member, or a unit of the scope with a live relation into a member,
//! of any kind — an answer, a rebuttal, a quote — at that unit's instant. So a class said once
//! and never returned to lives 0 s, and one a later message elaborates lives until that message.
//!
//! Over the classes with a dated member: `life:median` (the lower median) and `life:p90`, in
//! seconds, and `returned = [classes whose life is above 0, classes]`. A class with no dated
//! member has no first utterance and is counted (`n.undated:classes`); an undated reference
//! is not a reference in time and is passed over.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::Code;
use smysl_core::Uid;

use crate::dialogue::{self, Clock};
use crate::graph::{self, redundancy};
use crate::{Input, Measure, Needs, Num, Outcome, Purity, Recipe, Refusal, Value};

/// D3.
#[derive(Debug, Clone, Copy, Default)]
pub struct D3;

impl Measure for D3 {
    fn id(&self) -> &'static str {
        "D3"
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
    }

    fn admit(&self, recipe: &Recipe, input: &Input<'_>) -> Result<(), Refusal> {
        redundancy::policy(recipe)?;
        Clock::of(recipe, input.store, &graph::admitted(recipe, input)?).map(|_| ())
    }

    fn run(&self, recipe: &Recipe, input: &Input<'_>) -> Outcome {
        let store = input.store;
        let run = || -> Result<Value, Refusal> {
            let policy = redundancy::policy(recipe)?;
            let uids = graph::admitted(recipe, input)?;
            let clock = Clock::of(recipe, store, &uids)?;
            let class = redundancy::class_of(store, policy, &uids);
            // (first utterance, last reference) per class, over dated members.
            let mut span: BTreeMap<Uid, (i64, i64)> = BTreeMap::new();
            for (u, c) in &class {
                if let Some(&t) = clock.at.get(u) {
                    let e = span.entry(*c).or_insert((t, t));
                    e.0 = e.0.min(t);
                    e.1 = e.1.max(t);
                }
            }
            for (from, to) in dialogue::all_within(store, &dialogue::set(&uids)) {
                if let (Some(c), Some(&t)) = (class.get(&to), clock.at.get(&from)) {
                    if let Some(e) = span.get_mut(c) {
                        e.1 = e.1.max(t);
                    }
                }
            }
            let classes: BTreeSet<Uid> = class.values().copied().collect();
            if span.is_empty() {
                return Err(Refusal::new(
                    Code::W485,
                    format!(
                        "D3 found no class of `{}` with a dated member",
                        recipe.scope.trim()
                    ),
                ));
            }
            let mut lives: Vec<u64> = span.values().map(|(a, b)| (b - a) as u64).collect();
            let returned = lives.iter().filter(|l| **l > 0).count() as u64;
            let n = span.len() as u64;
            let v = Value::new()
                .with_n("units", uids.len() as u64)
                .with_n("classes", n)
                .with_n("undated:classes", classes.len() as u64 - n)
                .with_value("returned", Num::Ratio(returned, n));
            let v = dialogue::with_durations(v, "life", &mut lives).with_headline_in(
                "life:median",
                "median class life",
                "s",
            );
            Ok(clock.note(v))
        };
        match run() {
            Ok(v) => Outcome::Value(v),
            Err(r) => Outcome::refused(r),
        }
    }
}
