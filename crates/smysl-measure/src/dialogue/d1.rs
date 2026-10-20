//! D1, answer rate and time to answer (SMYSL-2.6 §3.10): of a conversation's questions, how
//! many were answered, and how long the first answer took.
//!
//! A **question** is a kernel `question` unit of the scope; it is **answered** when a live
//! `answers` edge from another unit of the scope reaches it (the module header says why the
//! answering unit must be in the scope). `answered = [answered questions, questions]`.
//!
//! The **time to answer** is the earliest answering unit's instant minus the question's, over
//! the answered questions whose question and answers are all dated. An unanswered question is
//! in no time and is counted (`n.unanswered`). An answer said *before* its question — a clock
//! that disagrees with the reply graph, which GE-T13 plants and rule E reports — gives no
//! duration either: it is counted (`n.inverted`) and left out, since a negative time to answer
//! is a fact about the clocks and not about the conversation.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::Code;
use smysl_core::types::RelKind;
use smysl_core::{KernelType, SchemaId, Uid};
use smysl_graph::Store;

use crate::dialogue::{self, Clock};
use crate::graph;
use crate::{Input, Measure, Needs, Num, Outcome, Purity, Recipe, Refusal, Value};

/// D1.
#[derive(Debug, Clone, Copy, Default)]
pub struct D1;

fn is_question(store: &Store, uid: &Uid) -> bool {
    store
        .get(uid)
        .is_some_and(|u| u.core.schema == SchemaId::Kernel(KernelType::Question))
}

/// Each question of `uids` and the units of `uids` that answer it, in uid order.
pub fn answered(store: &Store, uids: &[Uid]) -> BTreeMap<Uid, BTreeSet<Uid>> {
    let scope = dialogue::set(uids);
    let mut out: BTreeMap<Uid, BTreeSet<Uid>> = uids
        .iter()
        .filter(|u| is_question(store, u))
        .map(|u| (*u, BTreeSet::new()))
        .collect();
    for (from, to) in dialogue::within(store, &RelKind::Answers, &scope) {
        if let Some(a) = out.get_mut(&to) {
            a.insert(from);
        }
    }
    out
}

impl Measure for D1 {
    fn id(&self) -> &'static str {
        "D1"
    }

    fn version(&self) -> u16 {
        1
    }

    fn purity(&self) -> Purity {
        Purity::Pure
    }

    fn needs(&self) -> Needs {
        Needs::UNITS.union(Needs::TIME)
    }

    fn complete(&self, recipe: &mut Recipe) {
        graph::complete(recipe);
    }

    fn admit(&self, recipe: &Recipe, input: &Input<'_>) -> Result<(), Refusal> {
        dialogue::Clock::of(recipe, input.store, &graph::admitted(recipe, input)?).map(|_| ())
    }

    fn run(&self, recipe: &Recipe, input: &Input<'_>) -> Outcome {
        let store = input.store;
        let run = || -> Result<Value, Refusal> {
            let uids = graph::admitted(recipe, input)?;
            let clock = Clock::of(recipe, store, &uids)?;
            let questions = answered(store, &uids);
            if questions.is_empty() {
                return Err(Refusal::new(
                    Code::W485,
                    format!("D1 found no question unit in `{}`", recipe.scope.trim()),
                ));
            }
            let (mut times, mut inverted, mut untimed) = (Vec::new(), 0u64, 0u64);
            let mut done = 0u64;
            for (q, answers) in &questions {
                if answers.is_empty() {
                    continue;
                }
                done += 1;
                let at = |u: &Uid| clock.at.get(u).copied();
                let first = answers.iter().map(at).collect::<Option<Vec<i64>>>();
                match (at(q), first.and_then(|f| f.into_iter().min())) {
                    (Some(tq), Some(ta)) if ta >= tq => times.push((ta - tq) as u64),
                    (Some(_), Some(_)) => inverted += 1,
                    _ => untimed += 1,
                }
            }
            let n = questions.len() as u64;
            let v = Value::new()
                .with_n("units", uids.len() as u64)
                .with_n("questions", n)
                .with_n("unanswered", n - done)
                .with_n("inverted", inverted)
                .with_n("untimed", untimed)
                .with_n("timed", times.len() as u64)
                .with_value("answered", Num::Ratio(done, n))
                .with_headline_in("answered", "answered", "");
            Ok(clock.note(dialogue::with_durations(v, "time", &mut times)))
        };
        match run() {
            Ok(v) => Outcome::Value(v),
            Err(r) => Outcome::refused(r),
        }
    }
}
