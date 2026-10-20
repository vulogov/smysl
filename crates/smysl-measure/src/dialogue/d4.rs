//! D4, initiative (SMYSL-2.6 §3.10): per speaker, the share of the conversation's propositions
//! they brought in, against the share of its messages that were theirs.
//!
//! The units are put in **reading order**, A3's: by text, part, the span's first byte and the
//! locator, which for a chat export is the order the export recorded its messages in. D4 is
//! not among §3.10's time-ordered measures, and a conversation has a reading order whether or
//! not its clocks agree. A unit with no place is left out and counted (`n.unplaced`).
//!
//! A class under the recipe's policy (`SMY-E481` without one) is **introduced** by the speaker
//! of its first member in that order. A **message** is the place a unit was drawn from, its
//! source reference — `<tid>#<locator>`, one chat message — and it is the speaker's whose
//! unit it is. So D4 sees the messages the scope's units were drawn from and no others: it is
//! a unit measure, and a message nothing was extracted from said nothing D4 can credit.
//!
//! Per speaker: `intro:<s> = [classes introduced, classes]`, `talk:<s> = [messages, messages]`
//! and `gap:<s>` their difference, in fixed point. A class whose first member has no speaker
//! is introduced by nobody (`n.unspoken:classes`), and stays in the denominator. The headline is
//! the largest gap, `max:gap`, beside `min:gap` — prefixed the other way round, so no speaker
//! named `max` can collide with them.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::Code;
use smysl_core::Uid;

use crate::dialogue;
use crate::graph::{self, novelty, ratio_q20, redundancy};
use crate::{Input, Measure, Needs, Num, Outcome, Purity, Recipe, Refusal, Value};

/// D4.
#[derive(Debug, Clone, Copy, Default)]
pub struct D4;

impl Measure for D4 {
    fn id(&self) -> &'static str {
        "D4"
    }

    fn version(&self) -> u16 {
        1
    }

    fn purity(&self) -> Purity {
        Purity::Pure
    }

    fn needs(&self) -> Needs {
        Needs::UNITS.union(Needs::CLASSES)
    }

    fn complete(&self, recipe: &mut Recipe) {
        graph::complete(recipe);
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
            let places = novelty::places(store, &uids);
            let mut keyed: Vec<(&novelty::Place, Uid)> =
                places.iter().map(|(u, p)| (p, *u)).collect();
            keyed.sort();
            let ordered: Vec<Uid> = keyed.into_iter().map(|(_, u)| u).collect();
            let speakers = dialogue::speakers(store, &ordered);
            let class = redundancy::class_of(store, policy, &ordered);
            let mut seen: BTreeSet<Uid> = BTreeSet::new();
            let mut intro: BTreeMap<&str, u64> = BTreeMap::new();
            let mut unspoken = 0u64;
            for u in &ordered {
                if !seen.insert(class[u]) {
                    continue;
                }
                match speakers.get(u) {
                    Some(s) => *intro.entry(s.as_str()).or_default() += 1,
                    None => unspoken += 1,
                }
            }
            let mut messages: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
            for (u, s) in &speakers {
                if let Some(src) = store.get(u).and_then(|x| x.core.source.as_ref()) {
                    messages
                        .entry(s.as_str())
                        .or_default()
                        .insert(src.reference.as_str());
                }
            }
            let total: u64 = messages.values().map(|m| m.len() as u64).sum();
            let classes = seen.len() as u64;
            if classes == 0 || total == 0 {
                return Err(Refusal::new(
                    Code::W485,
                    format!(
                        "D4 found no message with a speaker in `{}` it could put in reading order",
                        recipe.scope.trim()
                    ),
                ));
            }
            let mut v = Value::new()
                .with_n("units", uids.len() as u64)
                .with_n("unplaced", (uids.len() - ordered.len()) as u64)
                .with_n("classes", classes)
                .with_n("messages", total)
                .with_n("unspoken:classes", unspoken);
            let who: BTreeSet<&str> = intro.keys().chain(messages.keys()).copied().collect();
            let mut gaps = Vec::new();
            for s in &who {
                let i = intro.get(s).copied().unwrap_or(0);
                let m = messages.get(s).map_or(0, |m| m.len() as u64);
                // i/classes − m/total = (i·total − m·classes) / (classes·total).
                let num = i128::from(i) * i128::from(total) - i128::from(m) * i128::from(classes);
                let gap = ratio_q20(num, i128::from(classes) * i128::from(total))
                    .ok_or_else(|| Refusal::new(Code::E489, "D4's gap is past a payload number"))?;
                gaps.push(gap);
                v = v
                    .with_value(format!("intro:{s}"), Num::Ratio(i, classes))
                    .with_value(format!("talk:{s}"), Num::Ratio(m, total))
                    .with_value(format!("gap:{s}"), Num::Fixed(gap));
            }
            let (lo, hi) = (gaps.iter().min(), gaps.iter().max());
            if let (Some(lo), Some(hi)) = (lo, hi) {
                v = v
                    .with_value("min:gap", Num::Fixed(*lo))
                    .with_value("max:gap", Num::Fixed(*hi))
                    .with_headline_in("max:gap", "largest initiative gap", "");
            }
            Ok(v.with_n("speakers", who.len() as u64))
        };
        match run() {
            Ok(v) => Outcome::Value(v),
            Err(r) => Outcome::refused(r),
        }
    }
}
