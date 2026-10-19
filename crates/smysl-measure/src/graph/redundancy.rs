//! A2, redundancy (SMYSL-2.6 §3.8): how often a scope says what it has already said.
//!
//! `1 − classes/units`, stored as the ratio `[units − classes, units]`, over proposition
//! classes formed under the recipe's policy (`smysl_graph::proposition`, SMYSL-2.4 §3.6). A
//! unit no live `x.text/same-as` edge joins to another is a class of one, so a scope with no
//! edges is 0 exactly, and k units all claimed the same as each other are `[k − 1, k]`.
//!
//! **Across the scope and within each text.** The across figure is over the classes of the
//! whole scope; a text's is over the classes formed among that text's units alone, so a
//! sentence two translations share makes the scope redundant and neither translation.
//!
//! A recipe without a policy is refused (`SMY-E481`): classes are what the ratio is divided
//! by, and the three policies give three numbers.

use std::collections::BTreeSet;

use smysl_core::diag::Code;
use smysl_core::Uid;
use smysl_graph::proposition::{self, Policy};
use smysl_graph::Store;

use crate::graph;
use crate::recipe::ClassPolicy;
use crate::{Input, Measure, Needs, Num, Outcome, Purity, Recipe, Refusal, Value};

/// A2.
#[derive(Debug, Clone, Copy, Default)]
pub struct A2;

/// The recipe's policy as the graph's, or `SMY-E481`.
pub(crate) fn policy(recipe: &Recipe) -> Result<Policy, Refusal> {
    match recipe.policy {
        Some(ClassPolicy::Strict) => Ok(Policy::Strict),
        Some(ClassPolicy::Component) => Ok(Policy::Component),
        Some(ClassPolicy::Attested(n)) => Ok(Policy::Attested(n)),
        None => Err(Refusal::new(
            Code::E481,
            format!(
                "{} counts proposition classes, and its recipe names no class policy; give one \
                 of strict, component or attested:<n>",
                recipe.id
            ),
        )),
    }
}

/// Each unit's class, by the class's id: the smallest member (A-12.4), or the unit itself when
/// no edge joins it to anything.
pub(crate) fn class_of(
    store: &Store,
    policy: Policy,
    uids: &[Uid],
) -> std::collections::BTreeMap<Uid, Uid> {
    let scope: BTreeSet<Uid> = uids.iter().copied().collect();
    let mut out: std::collections::BTreeMap<Uid, Uid> = uids.iter().map(|u| (*u, *u)).collect();
    for c in proposition::classes(store, policy, Some(&scope)) {
        for m in &c.members {
            out.insert(*m, c.id);
        }
    }
    out
}

/// `(units, classes)` over `uids`.
pub(crate) fn tally(store: &Store, policy: Policy, uids: &[Uid]) -> (u64, u64) {
    let classes: BTreeSet<Uid> = class_of(store, policy, uids).into_values().collect();
    (uids.len() as u64, classes.len() as u64)
}

impl Measure for A2 {
    fn id(&self) -> &'static str {
        "A2"
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
        policy(recipe)?;
        graph::admitted(recipe, input).map(|_| ())
    }

    fn run(&self, recipe: &Recipe, input: &Input<'_>) -> Outcome {
        let (policy, uids) =
            match policy(recipe).and_then(|p| Ok((p, graph::admitted(recipe, input)?))) {
                Ok(x) => x,
                Err(r) => return Outcome::refused(r),
            };
        let (units, classes) = tally(input.store, policy, &uids);
        let mut v = Value::new()
            .with_n("units", units)
            .with_n("classes", classes)
            .with_value("redundancy", Num::Ratio(units - classes, units))
            .with_headline_in("redundancy", "redundancy", "");
        for (alias, members) in graph::by_text(input.store, &uids) {
            let (u, c) = tally(input.store, policy, &members);
            v = v.with_value(format!("redundancy:{alias}"), Num::Ratio(u - c, u));
        }
        Outcome::Value(v)
    }
}
