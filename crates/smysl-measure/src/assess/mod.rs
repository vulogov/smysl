//! Assessment (SMYSL-2.6 §3.11, TX-P9 N6): a **key** of questions and the answers the texts
//! give them (K1), a reader's **score** against it (K3), the **knowledge state** that score
//! leaves (K4), the **diff** of two states, and Krippendorff's α for the annotators whose
//! agreement every model figure is read against ([`agreement`]).
//!
//! K2, a reader's responses, is not code here: they are units in the store, attested by the
//! reader, each with a live `answers` edge to the question of the item it answers. How they got
//! there — a reader's answer sheet through ingest, a model's turn, a typed import — is the
//! store's business; what a score reads is who attested a unit and what it answers.
//!
//! # Classes are resolved when they are read
//!
//! A key stores the **uids** of its answers, never a class id: a class id is the smallest
//! member and moves when a smaller member joins (draft 3 §9.1). Every function here forms the
//! classes it needs from the store as it is, under the policy the key recorded or the one it
//! is given, so a key made last year scores today's evidence.
//!
//! # A judge proposes a match, and writes nothing
//!
//! K3 step 1 proposes `same-as` between a reader's responses and the keyed answers with one of
//! SMYSL-2.4's engines ([`Judge`]). A proposal counts as a **match for this score** and is not
//! accepted into the store, so scoring changes no class and a second reader is scored against
//! the same key as the first. A pair already in one class matches under any judge; `accepted`
//! is the judge that proposes nothing and reads only that.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::Code;
use smysl_core::surface::hjson::{HObject, HValue, Spanned};
use smysl_core::surface::{object_to_payload, payload_to_object};
use smysl_core::types::RelKind;
use smysl_core::{AgentId, KernelType, Status, Uid, UnitCore, UnitCoreBuilder};
use smysl_graph::proposition::Policy;
use smysl_graph::Store;
use smysl_text::index::align;
use smysl_text::proposition::{Anchored, Engine, Lexical};

use crate::recipe::{at, key, ClassPolicy};
use crate::Refusal;

pub mod agreement;
pub mod key;
pub mod score;
pub mod state;

pub use key::{key_from_view, Item, Key, Mode};
pub use score::{score, ItemScore, Score};
pub use state::{diff, state, State, StateDiff};

/// How a reader's responses are matched to the keyed answers beyond the classes the store
/// already has (K3 step 1).
#[derive(Clone, Copy)]
#[non_exhaustive]
pub enum Judge<'t> {
    /// Only the store's own `same-as`: a response matches an answer in its class.
    Accepted,
    /// SMYSL-2.4's lexical engine, BM25 over gists; a response is proposed with its `k`
    /// nearest units among the key's answers and responses, so `k = 1` with adjacency off is
    /// the judge that asks "is this answer the one it is closest to".
    Lexical(Lexical),
    /// SMYSL-2.4's anchored engine over a library's alignment table.
    Anchored(&'t align::Table),
}

impl Judge<'_> {
    /// `accepted`, `lexical` or `anchored`, as a score records it.
    pub fn id(&self) -> &'static str {
        match self {
            Judge::Accepted => "accepted",
            Judge::Lexical(_) => "lexical",
            Judge::Anchored(_) => "anchored",
        }
    }

    /// The pairs this judge proposes among `units`, each `(smaller, larger)`.
    pub(crate) fn propose(&self, store: &Store, units: &BTreeSet<Uid>) -> BTreeSet<(Uid, Uid)> {
        let proposals = match self {
            Judge::Accepted => return BTreeSet::new(),
            Judge::Lexical(l) => l.propose(store, Some(units)),
            Judge::Anchored(t) => Anchored::new(t).propose(store, Some(units)),
        };
        proposals.into_iter().map(|p| (p.a, p.b)).collect()
    }
}

/// The recipe's policy as the graph's, or `SMY-E481`.
pub(crate) fn policy(p: Option<ClassPolicy>) -> Result<Policy, Refusal> {
    match p {
        Some(ClassPolicy::Strict) => Ok(Policy::Strict),
        Some(ClassPolicy::Component) => Ok(Policy::Component),
        Some(ClassPolicy::Attested(n)) => Ok(Policy::Attested(n)),
        None => Err(Refusal::new(
            Code::E481,
            "an assessment compares proposition classes, and names no class policy; give one \
             of strict, component or attested:<n>",
        )),
    }
}

/// Why a key's question or answer no longer stands (`SMY-W488`), or `None` when it does: it
/// is gone from the store, a live `retracts` or `supersedes` names it, or it has become
/// unfounded.
pub fn drift(store: &Store, uid: &Uid) -> Option<&'static str> {
    if !store.contains_uid(uid) {
        return Some("absent");
    }
    for (kind, why) in [
        (RelKind::Retracts, "retracted"),
        (RelKind::Supersedes, "superseded"),
    ] {
        if store.relations_of_kind(&kind).iter().any(|r| r.to == *uid) {
            return Some(why);
        }
    }
    store.is_unfounded(uid).then_some("unfounded")
}

/// The units `reader` attested, in uid order: a reader's responses, before any of them is
/// found to answer anything.
pub fn responses(store: &Store, reader: &AgentId) -> Vec<Uid> {
    store
        .units()
        .filter(|(u, _)| store.attested_by(u).contains(reader))
        .map(|(u, _)| *u)
        .collect()
}

/// The `@data` unit an assessment result is: `derived`, grounded on `grounds`, its gist cut to
/// the default bound.
pub(crate) fn data_unit(
    gist: &str,
    grounds: impl IntoIterator<Item = Uid>,
    payload: &HObject,
) -> Result<UnitCore, Refusal> {
    let bytes = object_to_payload(payload)
        .ok_or_else(|| Refusal::new(Code::E489, "the payload could not be written"))?;
    let mut grounds: Vec<Uid> = grounds.into_iter().collect();
    grounds.sort();
    grounds.dedup();
    UnitCoreBuilder::new(KernelType::Data, crate::fit(gist), Status::Derived)
        .grounds(grounds)
        .payload(bytes)
        .build()
        .map_err(|e| Refusal::new(Code::E489, format!("the unit could not be built: {e}")))
}

/// Payload building: an object from `(key, value)` pairs.
pub(crate) fn object(fields: Vec<(&str, HValue)>) -> HObject {
    let mut o = HObject::new();
    for (k, v) in fields {
        o.insert(key(k), at(v));
    }
    o
}

pub(crate) fn text(s: impl Into<String>) -> HValue {
    HValue::Str(s.into())
}

pub(crate) fn int(n: u64) -> HValue {
    HValue::Int(i64::try_from(n).unwrap_or(i64::MAX))
}

pub(crate) fn uids<'a>(us: impl IntoIterator<Item = &'a Uid>) -> HValue {
    HValue::Array(us.into_iter().map(|u| at(text(u.canonical()))).collect())
}

pub(crate) fn ratio((n, d): (u64, u64)) -> HValue {
    HValue::Array(vec![at(int(n)), at(int(d))])
}

/// Payload reading, for a key or a state read back: the object of a unit of `schema` data.
pub(crate) fn payload_of(store: &Store, uid: &Uid, what: &str) -> Result<HObject, Refusal> {
    let refuse = |why: &str| {
        Refusal::new(
            Code::E489,
            format!("`{}` is not {what}: {why}", uid.canonical()),
        )
    };
    let unit = store
        .get(uid)
        .ok_or_else(|| refuse("the store does not hold it"))?;
    let payload = unit
        .core
        .payload
        .as_ref()
        .ok_or_else(|| refuse("it has no payload"))?;
    payload_to_object(payload).map_err(|e| refuse(&e.to_string()))
}

pub(crate) fn get<'o>(o: &'o HObject, k: &str) -> Option<&'o HValue> {
    o.get(k).map(|v| &v.value)
}

pub(crate) fn as_str(v: Option<&HValue>) -> Option<&str> {
    match v {
        Some(HValue::Str(s)) => Some(s),
        _ => None,
    }
}

pub(crate) fn as_int(v: Option<&HValue>) -> Option<u64> {
    match v {
        Some(HValue::Int(i)) => u64::try_from(*i).ok(),
        _ => None,
    }
}

pub(crate) fn as_array(v: Option<&HValue>) -> Option<&[Spanned<HValue>]> {
    match v {
        Some(HValue::Array(a)) => Some(a),
        _ => None,
    }
}

pub(crate) fn as_uids(v: Option<&HValue>) -> Option<Vec<Uid>> {
    as_array(v)?
        .iter()
        .map(|x| as_str(Some(&x.value)).and_then(|s| Uid::parse(s).ok()))
        .collect()
}

/// Classes over `units` under `policy`, each unit's class id.
pub(crate) fn classes(store: &Store, policy: Policy, units: &BTreeSet<Uid>) -> BTreeMap<Uid, Uid> {
    let v: Vec<Uid> = units.iter().copied().collect();
    crate::graph::redundancy::class_of(store, policy, &v)
}
