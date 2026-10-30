//! C3, inflation (SMYSL-2.6 §3.9): whether the target states more firmly than its source.
//!
//! Over the channel's pairs — each target unit in an aligned class with its partner s*
//! ([`super::Channel::pairs`]) — `Δ = status(t) − status(s*)` on the ladder `unfounded` 0 …
//! `measured` 5, counted as A4 counts a status. Stored as eleven counts `delta:-5` …
//! `delta:5`, every one written so the shape does not depend on the data; `inflation` is
//! `[Δ > 0, pairs]` and `deflation` `[Δ < 0, pairs]`. Because s* is the strongest source
//! member, a pair is inflated only if no source member supports t's status.
//!
//! **The mode matrix** counts each pair's `text:mode` change, `mode:<s*>><t>`, over the seven
//! modes of `x.text/v1` and `none` for a unit the structure gave no mode — an eighth row and
//! column §3.9's 7×7 does not have, because a unit this build drew from a Bible has none, and
//! leaving those out would make the matrix empty where the channel is not. Only cells with a
//! count are written; `mode:changed` is `[pairs whose mode differs, pairs]`.

use std::collections::BTreeMap;

use smysl_core::Status;

use crate::channel;
use crate::graph::{self, density::text_field, epistemic::status_of};
use crate::{Input, Measure, Needs, Num, Outcome, Purity, Recipe, Refusal, Value};

/// C3.
#[derive(Debug, Clone, Copy, Default)]
pub struct C3;

/// The mode a unit is counted under: its `text:mode`, or `none`.
pub(crate) fn mode_of(store: &smysl_graph::Store, uid: &smysl_core::Uid) -> String {
    text_field(store, uid, "text:mode").unwrap_or_else(|| "none".into())
}

impl Measure for C3 {
    fn id(&self) -> &'static str {
        "C3"
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
        channel::paired(recipe, input).map(|_| ())
    }

    fn run(&self, recipe: &Recipe, input: &Input<'_>) -> Outcome {
        let (ch, pairs) = match channel::paired(recipe, input) {
            Ok(c) => c,
            Err(r) => return Outcome::refused(r),
        };
        let store = input.store;
        let level = |u| i64::from(status_of(store, u).unwrap_or(Status::Unfounded).as_u8());
        let mut delta = [0u64; 11];
        let mut modes: BTreeMap<String, u64> = BTreeMap::new();
        let mut changed = 0u64;
        for (t, s) in &pairs {
            let d = level(t) - level(s);
            delta[(d + 5) as usize] += 1;
            let (ms, mt) = (mode_of(store, s), mode_of(store, t));
            changed += u64::from(ms != mt);
            *modes.entry(format!("mode:{ms}>{mt}")).or_default() += 1;
        }
        let n = pairs.len() as u64;
        let up: u64 = delta[6..].iter().sum();
        let down: u64 = delta[..5].iter().sum();
        let mut v = Value::new()
            .with_n("pairs", n)
            .with_n("target:units", ch.target.len() as u64)
            .with_value("inflation", Num::Ratio(up, n))
            .with_value("deflation", Num::Ratio(down, n))
            .with_value("mode:changed", Num::Ratio(changed, n))
            .with_headline_in("inflation", "inflation", "");
        for (i, c) in delta.iter().enumerate() {
            v = v.with_value(format!("delta:{}", i as i64 - 5), Num::Count(*c));
        }
        for (k, c) in modes {
            v = v.with_value(k, Num::Count(c));
        }
        Outcome::Value(v)
    }
}
