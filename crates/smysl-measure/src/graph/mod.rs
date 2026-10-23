//! The assertion-level measures (SMYSL-2.6 §3.8, TX-P9 N1): what a scope's units say about
//! themselves — how densely a text was extracted, how often it repeats itself, how new each
//! unit is, how its statuses spread, what it rests on, how concentrated its salience is, how
//! much of it is contested, and how well it is dated.
//!
//! # A unit measure counts the scope's units
//!
//! A text measure reads a scope's `from` (§3.7, as built in M2); a unit measure reads what the
//! whole query answered with, [`crate::Resolved::uids`], the units the store still holds, in
//! uid order. That set is already an sq query's answer, so it is already filtered: `from
//! texts where text:mode = asserted` is a scope.
//!
//! **The recipe's `filter` is applied on top of it** when it says something, as an sq `where`
//! predicate over the same fields a scope's filter reads — and a unit measure's own default is
//! the empty filter, which the recipe then does not write (absent is absent, [`crate::recipe`]).
//! Draft 3's default, `holder = text and mode = asserted`, which a text measure still carries,
//! **selects no unit this build writes**: SMYSL-2.4 §3.4 writes `asserted` only with a known
//! holder, an `x.text/entity`, and nothing writes `x.text/v1`'s `text` literal, so the two
//! halves of the default are never true of one unit — and sq reads the bare names `holder` and
//! `mode` as payload keys no unit has. A measure under it would report every scope empty
//! (`SMY-W485`), which is a statement about the filter and not about the text.
//!
//! # Within each text
//!
//! A2, A7 and A10 report per text as well as across the scope. A unit is in a text when its
//! `source.reference` names a part a head manifest lists (`by_text`); a unit drawn from no
//! part is in the scope and in no text.
//!
//! # Exact numbers
//!
//! What a count over units gives is a ratio of counts, stored as one ([`crate::Num::Ratio`]);
//! a rate per hundred or per thousand is the same ratio scaled, rounded once in integer
//! arithmetic (`scaled`). The two entropies and the Gini are the only numbers here that pass
//! through a float, and only A4's through a logarithm.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::Code;
use smysl_core::ids::Tid;
use smysl_core::Uid;
use smysl_graph::Store;
use smysl_query::ast::Expr;
use smysl_query::pred::{self, Lowered};

use crate::recipe::{Recipe, DEFAULT_FILTER};
use crate::{Input, Refusal};

pub mod concentration;
pub mod contention;
pub mod dating;
pub mod density;
pub mod entity;
pub mod epistemic;
pub mod grounding;
pub mod novelty;
pub mod rd;
pub mod redundancy;

/// The smallest scope a unit measure reports on by default: one unit. A ratio over none has
/// nothing under it (§2.1), and a measure says not run (`SMY-W485`) rather than dividing by
/// zero or calling it one.
pub const MIN_N: u64 = 1;

/// A unit measure's defaults: `min:n` in units, and the empty filter in place of the text
/// measures' default (see the module header).
pub(crate) fn complete(recipe: &mut Recipe) {
    recipe.min_n.get_or_insert(MIN_N);
    if recipe.filter == DEFAULT_FILTER {
        recipe.filter.clear();
    }
}

/// The scope's units the store holds, the recipe's filter applied, in uid order.
pub(crate) fn units(recipe: &Recipe, input: &Input<'_>) -> Result<Vec<Uid>, Refusal> {
    // One reader for the whole scope: the store's declarations decoded once, each unit lensed
    // once (SMYSL-2.5 §3.9), so a filter on a declared key reads it as the current revision
    // names it, as `q` does.
    let reader = pred::Reader::new(input.store);
    let filter = lower(&recipe.filter, &reader)?;
    let params = BTreeMap::new();
    Ok(input
        .scope
        .uids
        .iter()
        .filter(|u| input.store.contains_uid(u))
        .filter(|u| {
            filter
                .as_ref()
                .is_none_or(|f| pred::holds_in(&reader, u, f, &params))
        })
        .copied()
        .collect())
}

/// The scope's units, or `SMY-W485` when there are fewer than `min:n`.
pub(crate) fn admitted(recipe: &Recipe, input: &Input<'_>) -> Result<Vec<Uid>, Refusal> {
    let uids = units(recipe, input)?;
    let want = recipe.min_n.unwrap_or(0);
    if (uids.len() as u64) < want {
        return Err(Refusal::new(
            Code::W485,
            format!(
                "{} needs at least {want} units and `{}` has {}",
                recipe.id,
                recipe.scope.trim(),
                uids.len()
            ),
        ));
    }
    Ok(uids)
}

/// A filter as sq lowers a `where`: `None` for the empty one.
fn lower(filter: &str, reader: &pred::Reader<'_>) -> Result<Option<Lowered>, Refusal> {
    if filter.trim().is_empty() {
        return Ok(None);
    }
    let refuse = |why: String| Refusal::new(Code::E489, format!("filter `{filter}`: {why}"));
    let parsed = smysl_query::parse(&format!("where {filter}")).map_err(|d| refuse(d.message))?;
    let Expr::Pipeline(p) = &parsed.query.expr else {
        return Err(refuse("a filter is one predicate".into()));
    };
    let mut diags = Vec::new();
    let mut all = Vec::new();
    for f in &p.filters {
        if let Some(l) = pred::lower_in(&f.pred, Some(reader.schemas()), f.lens_none, &mut diags) {
            all.push(l);
        }
    }
    if let Some(d) = diags.into_iter().next() {
        return Err(refuse(d.message));
    }
    Ok(Some(Lowered::And(all)))
}

/// The texts of a scope's units: each unit under the alias of the manifest it was read under
/// when its source names one, and otherwise under every head manifest that lists its part —
/// a part two expressions share is in each text. A unit drawn from no part is in none.
pub(crate) fn by_text(store: &Store, uids: &[Uid]) -> BTreeMap<String, Vec<Uid>> {
    let mut aliases: BTreeMap<Tid, BTreeSet<&str>> = BTreeMap::new();
    for (mid, m) in store.manifests() {
        if store.is_superseded(mid) {
            continue;
        }
        for p in &m.parts {
            aliases.entry(p.tid).or_default().insert(m.alias.as_str());
        }
    }
    let mut out: BTreeMap<String, Vec<Uid>> = BTreeMap::new();
    for u in uids {
        let Some(source) = store.get(u).and_then(|x| x.core.source.as_ref()) else {
            continue;
        };
        let Some(tid) = Store::part_of_reference(&source.reference) else {
            continue;
        };
        if let Some(m) = source.manifest().and_then(|m| store.manifest(m)) {
            out.entry(m.alias.clone()).or_default().push(*u);
            continue;
        }
        for a in aliases.get(&tid).into_iter().flatten() {
            out.entry((*a).to_string()).or_default().push(*u);
        }
    }
    out
}

/// The part a unit was drawn from, if its source names one.
pub(crate) fn part_of(store: &Store, uid: &Uid) -> Option<Tid> {
    let source = store.get(uid)?.core.source.as_ref()?;
    Store::part_of_reference(&source.reference)
}

/// `num · k / den` as a fixed-point payload number, rounded half to even once, in integers.
/// `None` for a zero denominator or a result past a payload number.
pub(crate) fn scaled(num: u64, k: u64, den: u64) -> Option<i64> {
    ratio_q20(i128::from(num) * i128::from(k), i128::from(den))
}

/// `num / den` as `round_half_even(num · 2²⁰ / den)`, in integers. `den` positive.
pub(crate) fn ratio_q20(num: i128, den: i128) -> Option<i64> {
    if den <= 0 {
        return None;
    }
    let n = num.checked_mul(1 << 20)?;
    let (mut q, r) = (n.div_euclid(den), n.rem_euclid(den));
    if 2 * r > den || (2 * r == den && q % 2 != 0) {
        q += 1;
    }
    i64::try_from(q).ok()
}

/// The lower median of sorted integers: the value at rank ⌈n/2⌉, so that the median of a
/// count is a count. `None` for none.
pub(crate) fn median(sorted: &[u64]) -> Option<u64> {
    nearest_rank(sorted, 1, 2)
}

/// The nearest-rank percentile `num/den` of sorted values: the value at rank ⌈n·num/den⌉.
pub(crate) fn nearest_rank(sorted: &[u64], num: u64, den: u64) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let n = sorted.len() as u64;
    let rank = (n * num).div_ceil(den).max(1);
    sorted.get(rank as usize - 1).copied()
}
