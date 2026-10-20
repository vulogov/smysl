//! D6, reversals (SMYSL-2.6 §3.10): who changes their mind, and about what.
//!
//! Per speaker and proposition class under the recipe's policy (`SMY-E481` without one): the
//! speaker's units in the class whose `text:mode` is `asserted` or `negated`, in effective time
//! with ties broken by uid, and a **reversal** each time one is followed by the other. A
//! speaker who asserts, negates and asserts again has reversed twice; one who asserts three
//! times has not reversed. Other modes — `reported`, `hypothetical` — are not a position the
//! speaker takes, and are passed over rather than breaking a run.
//!
//! `reversals` is the total, the headline; `reversals:<speaker>` per speaker; and
//! `reversed = [speaker–class pairs with a reversal, speaker–class pairs with a position]`.
//! An undated unit has no place in the sequence and is left out (`SMY-W415`).

use std::collections::BTreeMap;

use smysl_core::diag::Code;
use smysl_core::Uid;

use crate::dialogue::{self, Clock};
use crate::graph::{self, redundancy};
use crate::{Input, Measure, Needs, Num, Outcome, Purity, Recipe, Refusal, Value};

/// D6.
#[derive(Debug, Clone, Copy, Default)]
pub struct D6;

/// One position a speaker took: when, which unit, and whether it asserted (or negated).
type Position = (i64, Uid, bool);

impl Measure for D6 {
    fn id(&self) -> &'static str {
        "D6"
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
            let speakers = dialogue::speakers(store, &uids);
            let mut runs: BTreeMap<(&str, Uid), Vec<Position>> = BTreeMap::new();
            for (u, s) in &speakers {
                let asserted = match dialogue::mode(store, u).as_deref() {
                    Some("asserted") => true,
                    Some("negated") => false,
                    _ => continue,
                };
                let Some(&t) = clock.at.get(u) else {
                    continue;
                };
                runs.entry((s.as_str(), class[u]))
                    .or_default()
                    .push((t, *u, asserted));
            }
            if runs.is_empty() {
                return Err(Refusal::new(
                    Code::W485,
                    format!(
                        "D6 found no dated asserted or negated unit with a speaker in `{}`",
                        recipe.scope.trim()
                    ),
                ));
            }
            let mut by: BTreeMap<&str, u64> = BTreeMap::new();
            let (mut total, mut reversed) = (0u64, 0u64);
            for ((s, _), seq) in &mut runs {
                seq.sort();
                let k = seq.windows(2).filter(|w| w[0].2 != w[1].2).count() as u64;
                *by.entry(s).or_default() += k;
                total += k;
                reversed += u64::from(k > 0);
            }
            let mut v = Value::new()
                .with_n("units", uids.len() as u64)
                .with_n("unspoken", (uids.len() - speakers.len()) as u64)
                .with_n("speakers", by.len() as u64)
                .with_n("pairs", runs.len() as u64)
                .with_value("reversals", Num::Count(total))
                .with_value("reversed", Num::Ratio(reversed, runs.len() as u64))
                .with_headline_in("reversals", "reversals", "");
            for (s, k) in by {
                v = v.with_value(format!("reversals:{s}"), Num::Count(k));
            }
            Ok(clock.note(v))
        };
        match run() {
            Ok(v) => Outcome::Value(v),
            Err(r) => Outcome::refused(r),
        }
    }
}
