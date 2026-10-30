//! The dialogue measures (SMYSL-2.6 §3.10, TX-P9 N5): what a conversation's units say about the
//! conversation — how often a question is answered and how soon (D1), whose assertions others
//! take up (D2), how long a proposition stays in the conversation (D3), who brings new
//! propositions in against how much they talk (D4), how far the vocabulary drifts from one
//! window to the next (D5), and who changes their mind (D6).
//!
//! # Speakers, modes and times
//!
//! A speaker is a unit's `text:speaker`, the pseudonym SMYSL-2.4 §3.4 writes from the message a
//! unit was attached to; a unit with none is in no speaker's figure and is counted
//! (`n.unspoken`). A mode is `text:mode`. A unit's time is its effective time on the recipe's
//! `axis` (rule E, `smysl_text::time`), read as an **instant**: the interval's lower bound, or
//! its upper when the lower is open. A chat message's said time is an instant already, and a
//! wider interval is placed where it starts, which is the order A3 sorts in.
//!
//! D1, D3 and D6 are time-ordered: an undated unit is left out of the figure and counted
//! (`SMY-W415`), and a scope whose undated share is above `undated:max` is refused
//! (`SMY-E482`), over the scope's units, as A3 does.
//!
//! # A response is a unit of the scope
//!
//! The relations these measures read — `answers`, `elaborates`, `rebuts`, `same-as` and, for
//! D3, any relation into a member — count only when **both ends are in the scope**, live and
//! present. A conversation is the scope, and an edge from outside it is somebody else's
//! conversation about this one. D3's "any relation" leaves out one family: a saved scope's
//! `x.query/answers` edges to the units its query found (§3.1), which a scope over every unit
//! can contain on a rerun, and which are a query's record and not anybody's reply. Kernel
//! `answers` is a different kind (OQ-49 kept them apart), so D1 never reads one.
//!
//! # Seconds
//!
//! A duration is the difference of two instants in milliseconds, stored as seconds in fixed
//! point (`ms · 2²⁰ / 1000`, rounded once); its median is the lower median and its p90 the
//! nearest rank, so each is a duration that was measured.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::schemas::query;
use smysl_core::types::RelKind;
use smysl_core::Uid;
use smysl_graph::Store;
use smysl_text::time::engine::effective;
use smysl_text::time::{Bound, Subject};

use crate::graph::density::text_field;
use crate::graph::novelty::{self, within_undated_max};
use crate::graph::ratio_q20;
use crate::{Num, Recipe, Refusal, Value};

pub mod d1;
pub mod d2;
pub mod d3;
pub mod d4;
pub mod d5;
pub mod d6;

/// A unit's speaker, its `text:speaker`.
pub fn speaker(store: &Store, uid: &Uid) -> Option<String> {
    text_field(store, uid, "text:speaker")
}

/// A unit's `text:mode`.
pub(crate) fn mode(store: &Store, uid: &Uid) -> Option<String> {
    text_field(store, uid, "text:mode")
}

/// The instants of a scope's dated units, and how many were not.
pub(crate) struct Clock {
    pub at: BTreeMap<Uid, i64>,
    pub undated: u64,
}

impl Clock {
    /// `units` on the recipe's axis, or `SMY-E482` when too few of them are dated.
    pub(crate) fn of(recipe: &Recipe, store: &Store, units: &[Uid]) -> Result<Clock, Refusal> {
        let eff = effective(store, novelty::axis(recipe)?);
        let mut at = BTreeMap::new();
        for u in units {
            let iv = eff.interval(&Subject::Unit(*u));
            let t = match (iv.lo, iv.hi) {
                (Bound::At(a), _) | (Bound::Open, Bound::At(a)) => a.ms(),
                (Bound::Open, Bound::Open) => continue,
            };
            at.insert(*u, t);
        }
        let undated = (units.len() - at.len()) as u64;
        within_undated_max(recipe, undated, units.len() as u64)?;
        Ok(Clock { at, undated })
    }

    /// `v` with the undated count, flagged when anything was left out, which `run_with`
    /// reports as `SMY-W415`.
    pub(crate) fn note(&self, v: Value) -> Value {
        let v = v.with_n("undated", self.undated);
        if self.undated > 0 {
            v.with_flag(novelty::UNDATED_EXCLUDED)
        } else {
            v
        }
    }
}

/// Live relations of `kind` with both ends in `scope`, as `(from, to)`, sorted.
pub(crate) fn within(store: &Store, kind: &RelKind, scope: &BTreeSet<Uid>) -> Vec<(Uid, Uid)> {
    let mut out: Vec<(Uid, Uid)> = store
        .relations_of_kind(kind)
        .into_iter()
        .filter(|r| r.from != r.to && scope.contains(&r.from) && scope.contains(&r.to))
        .map(|r| (r.from, r.to))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Every live relation with both ends in `scope`, whatever its kind but a saved query's, as
/// `(from, to)`, sorted.
pub(crate) fn all_within(store: &Store, scope: &BTreeSet<Uid>) -> Vec<(Uid, Uid)> {
    let mut out: Vec<(Uid, Uid)> = store
        .relations()
        .filter(|r| !store.is_withdrawn(r))
        .filter(|r| !matches!(&r.kind, RelKind::Extension(k) if k == query::ANSWERS))
        .filter(|r| r.from != r.to && scope.contains(&r.from) && scope.contains(&r.to))
        .map(|r| (r.from, r.to))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The scope's units as a set.
pub(crate) fn set(uids: &[Uid]) -> BTreeSet<Uid> {
    uids.iter().copied().collect()
}

/// Each unit's speaker, for the units that have one.
pub(crate) fn speakers(store: &Store, uids: &[Uid]) -> BTreeMap<Uid, String> {
    uids.iter()
        .filter_map(|u| speaker(store, u).map(|s| (*u, s)))
        .collect()
}

/// A duration in milliseconds as seconds in fixed point.
pub(crate) fn seconds(ms: u64) -> Option<Num> {
    ratio_q20(i128::from(ms), 1000).map(Num::Fixed)
}

/// `v` with the lower median and the p90 of `durations` (milliseconds, any order) under
/// `<prefix>:median` and `<prefix>:p90`; nothing when there are none.
pub(crate) fn with_durations(mut v: Value, prefix: &str, durations: &mut [u64]) -> Value {
    durations.sort_unstable();
    for (name, rank) in [("median", (1, 2)), ("p90", (9, 10))] {
        if let Some(n) = crate::graph::nearest_rank(durations, rank.0, rank.1).and_then(seconds) {
            v = v.with_value(format!("{prefix}:{name}"), n);
        }
    }
    v
}
