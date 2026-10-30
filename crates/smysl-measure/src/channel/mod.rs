//! The channel measures (SMYSL-2.6 §3.9, TX-P9 N4): what survived when a source was carried
//! into a target — a translation, a summary, a model's retelling — and what changed on the way.
//!
//! # The binding
//!
//! A channel is two scopes. The recipe's `scope` is the **source** S and its `channel` the
//! **target** T, each an sq query resolved over the same store; the measures read the units
//! each answers with, the recipe's filter applied to both, and refuse two scopes that share a
//! unit (`SMY-E489`), since a unit in both would align with itself and count as carried.
//!
//! §3.9 spells a channel `from channel kjv-1769 -> syn-1876`, as an sq scope. **sq has no such
//! form, and its `channel:` means something else**: SMYSL-2.5 §3.3 defined `channel:<name>` in
//! TX-P6 as a conversation — the scope of a chat — and 2.9 fixed the grammar. A second meaning
//! for one word in one grammar would make `from channel:general` ambiguous, so the target is
//! its own scope: `measure C1 --scope <S> --channel <T>`, which is §4.4's flag as it was
//! written, and the recipe carries both.
//!
//! **Classes.** The recipe's `policy` forms proposition classes over S ∪ T together
//! (`smysl_graph::proposition`), so a `same-as` edge from a source unit to a target unit is
//! what carries it; without a policy the measures are refused (`SMY-E481`). A class is
//! **aligned** when it has members on both sides.
//!
//! **Partners.** A target unit t in an aligned class is paired with one source member, s*: the
//! one with the **highest** status, counted as A4 counts it (unfounded for a unit whose
//! grounds were withdrawn), ties by the smallest uid. Choosing the strongest source makes
//! inflation conservative: t is inflated only if no source member supports its status.
//!
//! `smysl-eval`'s E2, claim survival, is C1 under an identity policy — every unit its own
//! class, so a unit survives when the target holds it — and E3 is the structural check C3 does
//! not replace; both stay in the harness.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::Code;
use smysl_core::{Status, Uid};
use smysl_graph::proposition::Policy;
use smysl_graph::Store;

use crate::graph::{self, epistemic::status_of, redundancy};
use crate::{Input, Recipe, Refusal};

pub mod c1;
pub mod c2;
pub mod c3;
pub mod c4;
pub mod c5;

/// A source and a target, bound under a policy.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Channel {
    /// The source's units, in uid order.
    pub source: Vec<Uid>,
    /// The target's units, in uid order.
    pub target: Vec<Uid>,
    /// Every class over S ∪ T, by id (its smallest member): its source members and its target
    /// members, each in uid order.
    pub classes: BTreeMap<Uid, (Vec<Uid>, Vec<Uid>)>,
}

impl Channel {
    /// Bind `source` to `target` under `policy`, or `SMY-E489` when they share a unit.
    pub fn bind(
        store: &Store,
        policy: Policy,
        source: &[Uid],
        target: &[Uid],
    ) -> Result<Channel, Refusal> {
        let s: BTreeSet<Uid> = source.iter().copied().collect();
        let shared = target.iter().filter(|u| s.contains(u)).count();
        if shared > 0 {
            return Err(Refusal::new(
                Code::E489,
                format!(
                    "a channel's source and target share {shared} unit(s); a unit on both sides \
                     would be carried into itself"
                ),
            ));
        }
        let all: Vec<Uid> = s.iter().chain(target).copied().collect();
        let mut classes: BTreeMap<Uid, (Vec<Uid>, Vec<Uid>)> = BTreeMap::new();
        for (u, c) in redundancy::class_of(store, policy, &all) {
            let e = classes.entry(c).or_default();
            if s.contains(&u) {
                e.0.push(u);
            } else {
                e.1.push(u);
            }
        }
        Ok(Channel {
            source: s.into_iter().collect(),
            target: {
                let mut t = target.to_vec();
                t.sort();
                t.dedup();
                t
            },
            classes,
        })
    }

    /// The source classes: those with a source member.
    pub fn source_classes(&self) -> impl Iterator<Item = &(Vec<Uid>, Vec<Uid>)> {
        self.classes.values().filter(|(s, _)| !s.is_empty())
    }

    /// Every target unit in an aligned class with its partner s*, in target uid order.
    pub fn pairs(&self, store: &Store) -> Vec<(Uid, Uid)> {
        let mut out = Vec::new();
        for (s, t) in self.classes.values() {
            let Some(partner) = strongest(store, s) else {
                continue;
            };
            out.extend(t.iter().map(|t| (*t, partner)));
        }
        out.sort();
        out
    }
}

/// The member with the highest status, ties by the smallest uid; `None` for none.
pub fn strongest(store: &Store, members: &[Uid]) -> Option<Uid> {
    members
        .iter()
        .map(|u| (status_of(store, u).unwrap_or(Status::Unfounded), *u))
        .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)))
        .map(|(_, u)| u)
}

/// What [`crate::run_with`] stamps on a channel measure's recipe: the target as written and
/// its digest — the target text's when the measure reads text, as a text measure's
/// `scope:digest` is its text's.
pub(crate) fn stamp(recipe: &mut Recipe, input: &Input<'_>) {
    if let Some(t) = input.target {
        recipe.channel = Some(t.text.clone());
        recipe.channel_digest = Some(t.digest);
    }
    if let Some(c) = input.target_text {
        recipe.channel_digest = Some(c.digest);
    }
}

/// The refusal for a channel measure given no target.
pub(crate) fn no_target(recipe: &Recipe) -> Refusal {
    Refusal::new(
        Code::E489,
        format!(
            "{} compares a source with the target it was carried into, and no target was \
             given; name one with --channel",
            recipe.id
        ),
    )
}

/// A channel's units: the scope's and the target's, each under the recipe's filter and each
/// at least `min:n` (`SMY-W485`), bound under the recipe's policy (`SMY-E481`).
pub(crate) fn admitted(recipe: &Recipe, input: &Input<'_>) -> Result<Channel, Refusal> {
    let policy = redundancy::policy(recipe)?;
    let Some(target) = input.target else {
        return Err(no_target(recipe));
    };
    let source = graph::admitted(recipe, input)?;
    let mut other = Input::new(input.store, target);
    other.library = input.library;
    let mut r = recipe.clone();
    r.scope = target.text.clone();
    let target = graph::admitted(&r, &other)?;
    Channel::bind(input.store, policy, &source, &target)
}

/// A channel and its pairs, or `SMY-W485` when no target unit is aligned: a share of pairs
/// over none has nothing under it.
pub(crate) fn paired(
    recipe: &Recipe,
    input: &Input<'_>,
) -> Result<(Channel, Vec<(Uid, Uid)>), Refusal> {
    let ch = admitted(recipe, input)?;
    let pairs = ch.pairs(input.store);
    if pairs.is_empty() {
        return Err(Refusal::new(
            Code::W485,
            format!(
                "{} compares aligned pairs, and no unit of `{}` is in a class with a unit of `{}`",
                recipe.id,
                input.target.map_or("", |t| t.text.trim()),
                recipe.scope.trim()
            ),
        ));
    }
    Ok((ch, pairs))
}
