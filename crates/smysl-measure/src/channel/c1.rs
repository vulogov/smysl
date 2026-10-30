//! C1, survival (SMYSL-2.6 §3.9): how much of the source the target still says.
//!
//! `[source classes with a target member, source classes]`, over the classes of the channel
//! ([`super::Channel`]). A source unit claimed the same as nothing on the other side is a class
//! that did not survive; two source units the target says once are one class, and survive
//! together. `n.dropped` is the classes that did not.

use crate::channel;
use crate::graph;
use crate::{Input, Measure, Needs, Num, Outcome, Purity, Recipe, Refusal, Value};

/// C1.
#[derive(Debug, Clone, Copy, Default)]
pub struct C1;

impl Measure for C1 {
    fn id(&self) -> &'static str {
        "C1"
    }

    fn version(&self) -> u16 {
        1
    }

    fn purity(&self) -> Purity {
        Purity::Pure
    }

    fn needs(&self) -> Needs {
        Needs::UNITS.union(Needs::CLASSES).union(Needs::CHANNEL)
    }

    fn complete(&self, recipe: &mut Recipe) {
        graph::complete(recipe);
    }

    fn admit(&self, recipe: &Recipe, input: &Input<'_>) -> Result<(), Refusal> {
        channel::admitted(recipe, input).map(|_| ())
    }

    fn run(&self, recipe: &Recipe, input: &Input<'_>) -> Outcome {
        let ch = match channel::admitted(recipe, input) {
            Ok(c) => c,
            Err(r) => return Outcome::refused(r),
        };
        let classes = ch.source_classes().count() as u64;
        let survived = ch.source_classes().filter(|(_, t)| !t.is_empty()).count() as u64;
        Outcome::Value(
            Value::new()
                .with_n("source:units", ch.source.len() as u64)
                .with_n("target:units", ch.target.len() as u64)
                .with_n("source:classes", classes)
                .with_n("dropped", classes - survived)
                .with_value("survival", Num::Ratio(survived, classes))
                .with_headline_in("survival", "survival", ""),
        )
    }
}
