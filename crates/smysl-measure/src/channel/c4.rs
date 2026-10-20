//! C4, attribution (SMYSL-2.6 §3.9): whether the target puts a claim in another mouth.
//!
//! `[pairs whose holder differs, pairs]` over the channel's pairs. A holder is a unit's
//! `text:holder`: the literal `text`, or absent, for the text's own voice, and otherwise an
//! entity's uid. **Entities are compared by class** under the recipe's policy, over the
//! holders the pairs name, so "David" in one language and "Давид" in another are one holder
//! when a `same-as` edge says so; `text` is equal only to `text`. Beside the headline, the
//! three ways a holder changes: `holder:gained` (the text's voice in the source, someone's in
//! the target), `holder:lost` (the reverse), and `holder:moved` (one entity to another).

use std::collections::BTreeMap;

use smysl_core::Uid;
use smysl_graph::proposition::Policy;
use smysl_graph::Store;

use crate::channel;
use crate::graph::{self, density::text_field, redundancy};
use crate::{Input, Measure, Needs, Num, Outcome, Purity, Recipe, Refusal, Value};

/// C4.
#[derive(Debug, Clone, Copy, Default)]
pub struct C4;

/// Who holds a unit's claim.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Holder {
    Text,
    Entity(Uid),
    /// A `text:holder` that is neither `text` nor a uid, compared as written.
    Other(String),
}

pub(crate) fn holder_of(store: &Store, uid: &Uid) -> Holder {
    match text_field(store, uid, "text:holder") {
        None => Holder::Text,
        Some(h) if h == "text" => Holder::Text,
        Some(h) => match Uid::parse(&h) {
            Ok(u) => Holder::Entity(u),
            Err(_) => Holder::Other(h),
        },
    }
}

/// How the holders of `(target, source)` pairs changed, entities compared by class.
pub(crate) struct Changes {
    pub changed: u64,
    pub gained: u64,
    pub lost: u64,
    pub moved: u64,
    /// The distinct entities holding either side.
    pub holders: u64,
}

/// [`Changes`] over `pairs`, each `(target, source)`, entity holders compared by their class
/// under `policy` (K3 reads holder fidelity through this too).
pub(crate) fn changes(store: &Store, policy: Policy, pairs: &[(Uid, Uid)]) -> Changes {
    let held: Vec<(Holder, Holder)> = pairs
        .iter()
        .map(|(t, s)| (holder_of(store, s), holder_of(store, t)))
        .collect();
    let mut entities: Vec<Uid> = held
        .iter()
        .flat_map(|(a, b)| [a, b])
        .filter_map(|h| match h {
            Holder::Entity(u) => Some(*u),
            _ => None,
        })
        .collect();
    entities.sort();
    entities.dedup();
    let class: BTreeMap<Uid, Uid> = redundancy::class_of(store, policy, &entities);
    let canon = |h: &Holder| match h {
        Holder::Entity(u) => Holder::Entity(class.get(u).copied().unwrap_or(*u)),
        other => other.clone(),
    };
    let (mut changed, mut gained, mut lost, mut moved) = (0u64, 0u64, 0u64, 0u64);
    for (s, t) in &held {
        let (s, t) = (canon(s), canon(t));
        if s == t {
            continue;
        }
        changed += 1;
        match (&s, &t) {
            (Holder::Text, _) => gained += 1,
            (_, Holder::Text) => lost += 1,
            _ => moved += 1,
        }
    }
    Changes {
        changed,
        gained,
        lost,
        moved,
        holders: entities.len() as u64,
    }
}

impl Measure for C4 {
    fn id(&self) -> &'static str {
        "C4"
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
        let (policy, (_, pairs)) = match redundancy::policy(recipe)
            .and_then(|p| Ok((p, channel::paired(recipe, input)?)))
        {
            Ok(x) => x,
            Err(r) => return Outcome::refused(r),
        };
        let h = changes(input.store, policy, &pairs);
        let n = pairs.len() as u64;
        Outcome::Value(
            Value::new()
                .with_n("pairs", n)
                .with_n("holders", h.holders)
                .with_value("holder:changed", Num::Ratio(h.changed, n))
                .with_value("holder:gained", Num::Count(h.gained))
                .with_value("holder:lost", Num::Count(h.lost))
                .with_value("holder:moved", Num::Count(h.moved))
                .with_headline_in("holder:changed", "holder changed", ""),
        )
    }
}
