//! C2, support (SMYSL-2.6 §3.9): how much of the target the source supports.
//!
//! `[target units in an aligned class, target units]`; the complement, `addition`, is what the
//! target says that the source does not. **When the target was produced by a model** — a unit
//! with an attestation on the `model` rung — an unaligned unit is a fabrication, and
//! `fabrication` counts those: the target units in no aligned class that a model attested. It
//! is written only when some target unit was a model's, so a translation's figure says
//! addition and nothing about models.

use smysl_core::Rung;

use crate::channel;
use crate::graph;
use crate::{Input, Measure, Needs, Num, Outcome, Purity, Recipe, Refusal, Value};

/// C2.
#[derive(Debug, Clone, Copy, Default)]
pub struct C2;

/// Whether a model attested the unit.
pub(crate) fn by_model(store: &smysl_graph::Store, uid: &smysl_core::Uid) -> bool {
    store
        .attestations_of(uid)
        .iter()
        .any(|a| a.rung == Rung::Model)
}

impl Measure for C2 {
    fn id(&self) -> &'static str {
        "C2"
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
        let store = input.store;
        let units = ch.target.len() as u64;
        let (mut aligned, mut model, mut fabricated) = (0u64, 0u64, 0u64);
        for (s, t) in ch.classes.values() {
            for u in t {
                let m = by_model(store, u);
                model += u64::from(m);
                if s.is_empty() {
                    fabricated += u64::from(m);
                } else {
                    aligned += 1;
                }
            }
        }
        let mut v = Value::new()
            .with_n("target:units", units)
            .with_n("source:units", ch.source.len() as u64)
            .with_n("model", model)
            .with_value("support", Num::Ratio(aligned, units))
            .with_value("addition", Num::Ratio(units - aligned, units))
            .with_headline_in("support", "support", "");
        if model > 0 {
            v = v.with_value("fabrication", Num::Ratio(fabricated, units));
        }
        Outcome::Value(v)
    }
}
