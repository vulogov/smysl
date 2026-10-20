//! K4, knowledge states (SMYSL-2.6 §3.11): what a reader reproduced of a key, and how two such
//! states differ.
//!
//! A **state** lists, per reproduced class, every member it was seen in: the keyed answers a
//! reader's responses matched and the responses that matched them, as K3 matches them. Its
//! view's roots are one representative per class, the smallest uid. The class ids are not
//! stored — they move (draft 3 §9.1) — only the members are.
//!
//! **A diff compares by membership.** Each state's classes are re-formed under the current
//! policy over the members of both, and a class of one state **corresponds** to a class of the
//! other when some member of each is in one current class. So a class whose id moved because a
//! smaller member joined between the two states is kept, not lost and gained (OQ-58). A class
//! with two correspondents was **split**, and one that two correspond to was **merged**; both
//! are reported rather than resolved, which is OQ-58's question left open on purpose. Kept
//! classes compare their strongest member's effective status, and the diff counts the rises
//! and the falls.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::Code;
use smysl_core::surface::hjson::HValue;
use smysl_core::{AgentId, Status, Uid, UnitCore, View, ViewId};
use smysl_graph::Store;

use crate::assess::score::{gather, match_item, proposals};
use crate::assess::{self, as_array, as_str, as_uids, get, object, text, uids, Judge, Key};
use crate::channel;
use crate::graph::epistemic::status_of;
use crate::recipe::{at, ClassPolicy};
use crate::Refusal;

/// A reader's knowledge state against a key.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct State {
    pub key: Uid,
    pub reader: AgentId,
    pub policy: ClassPolicy,
    /// Each reproduced class's members, sorted; the classes in order of their first member.
    pub classes: Vec<Vec<Uid>>,
}

/// The state `reader` is in against `key` (the unit `key_uid`), under `policy` (the key's own
/// when `None`) and `judge`.
pub fn state(
    store: &Store,
    key: &Key,
    key_uid: &Uid,
    reader: &AgentId,
    policy: Option<ClassPolicy>,
    judge: &Judge<'_>,
) -> Result<State, Refusal> {
    let policy = policy.unwrap_or(key.policy);
    let graph_policy = assess::policy(Some(policy))?;
    let got = gather(store, key, reader);
    let proposed = proposals(store, key, &got, judge);
    // Each matched answer and its matching responses, merged by the answers' current class.
    let mut groups: Vec<BTreeSet<Uid>> = Vec::new();
    for (k, responses) in &got.by_item {
        let item = &key.items[*k];
        let m = match_item(store, graph_policy, &proposed, &item.answers, responses);
        let mut by_answer: BTreeMap<Uid, BTreeSet<Uid>> = BTreeMap::new();
        for (r, a) in &m.pairs {
            by_answer.entry(*a).or_default().insert(*r);
        }
        for a in &m.answered {
            let mut g = by_answer.remove(a).unwrap_or_default();
            g.insert(*a);
            groups.push(g);
        }
    }
    let every: BTreeSet<Uid> = groups.iter().flatten().copied().collect();
    let class = assess::classes(store, graph_policy, &every);
    let mut merged: BTreeMap<Uid, BTreeSet<Uid>> = BTreeMap::new();
    for g in groups {
        // A group's answer fixes its class; its members join it whatever their own class is,
        // since a judge's match is not a same-as edge.
        let id = g
            .iter()
            .map(|u| class[u])
            .min()
            .expect("a group is never empty");
        merged.entry(id).or_default().extend(g);
    }
    let mut classes: Vec<Vec<Uid>> = merged
        .into_values()
        .map(|s| s.into_iter().collect())
        .collect();
    classes.sort();
    Ok(State {
        key: *key_uid,
        reader: reader.clone(),
        policy,
        classes,
    })
}

impl State {
    /// The view of the state: one representative per class, its smallest member.
    pub fn as_view(&self, id: ViewId) -> View {
        View::new(id, "assess-state")
            .with_roots(self.classes.iter().filter_map(|c| c.first().copied()))
    }

    /// The state as a `@data` unit, grounded on the key and every member.
    pub fn to_unit(&self) -> Result<UnitCore, Refusal> {
        let classes: Vec<HValue> = self.classes.iter().map(uids).collect();
        let payload = object(vec![
            ("state:key", text(self.key.canonical())),
            ("state:reader", text(self.reader.as_str())),
            ("state:policy", text(self.policy.to_string())),
            (
                "state:classes",
                HValue::Array(classes.into_iter().map(at).collect()),
            ),
        ]);
        let gist = format!(
            "K4 state of {}: {} class(es) reproduced",
            self.reader.as_str(),
            self.classes.len()
        );
        let grounds = std::iter::once(self.key).chain(self.classes.iter().flatten().copied());
        assess::data_unit(&gist, grounds, &payload)
    }

    /// A state read back from its unit.
    pub fn from_unit(store: &Store, uid: &Uid) -> Result<State, Refusal> {
        let o = assess::payload_of(store, uid, "a state")?;
        let bad = |why: &str| {
            Refusal::new(
                Code::E489,
                format!("`{}` is not a state: {why}", uid.canonical()),
            )
        };
        let classes = as_array(get(&o, "state:classes"))
            .ok_or_else(|| bad("no state:classes"))?
            .iter()
            .map(|c| as_uids(Some(&c.value)).ok_or_else(|| bad("a class is not a list of uids")))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(State {
            key: as_str(get(&o, "state:key"))
                .and_then(|s| Uid::parse(s).ok())
                .ok_or_else(|| bad("no state:key"))?,
            reader: as_str(get(&o, "state:reader"))
                .and_then(|s| AgentId::new(s).ok())
                .ok_or_else(|| bad("no state:reader"))?,
            policy: as_str(get(&o, "state:policy"))
                .and_then(ClassPolicy::parse)
                .ok_or_else(|| bad("no state:policy"))?,
            classes,
        })
    }
}

/// What changed between two states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct StateDiff {
    /// Classes of the second state nothing in the first corresponds to.
    pub gained: u64,
    /// Classes of the first nothing in the second corresponds to.
    pub lost: u64,
    /// Classes of the first something in the second corresponds to.
    pub kept: u64,
    /// Classes of the first with two or more correspondents.
    pub split: u64,
    /// Classes of the second with two or more correspondents.
    pub merged: u64,
    /// Corresponding pairs whose strongest member rose, and fell, in effective status.
    pub rose: u64,
    pub fell: u64,
}

/// `b` against `a`, by membership under `policy`.
pub fn diff(
    store: &Store,
    a: &State,
    b: &State,
    policy: ClassPolicy,
) -> Result<StateDiff, Refusal> {
    let graph_policy = assess::policy(Some(policy))?;
    let every: BTreeSet<Uid> = a
        .classes
        .iter()
        .chain(&b.classes)
        .flatten()
        .copied()
        .collect();
    let class = assess::classes(store, graph_policy, &every);
    let now = |c: &Vec<Uid>| -> BTreeSet<Uid> { c.iter().map(|u| class[u]).collect() };
    let (na, nb): (Vec<BTreeSet<Uid>>, Vec<BTreeSet<Uid>>) = (
        a.classes.iter().map(now).collect(),
        b.classes.iter().map(now).collect(),
    );
    let level = |c: &Vec<Uid>| {
        channel::strongest(store, c)
            .and_then(|u| status_of(store, &u))
            .unwrap_or(Status::Unfounded)
            .as_u8()
    };
    let mut d = StateDiff::default();
    let mut hit_b = vec![0u64; nb.len()];
    for (i, x) in na.iter().enumerate() {
        let mut k = 0u64;
        for (j, y) in nb.iter().enumerate() {
            if x.is_disjoint(y) {
                continue;
            }
            k += 1;
            hit_b[j] += 1;
            let (la, lb) = (level(&a.classes[i]), level(&b.classes[j]));
            d.rose += u64::from(lb > la);
            d.fell += u64::from(lb < la);
        }
        match k {
            0 => d.lost += 1,
            1 => d.kept += 1,
            _ => {
                d.kept += 1;
                d.split += 1;
            }
        }
    }
    d.gained = hit_b.iter().filter(|h| **h == 0).count() as u64;
    d.merged = hit_b.iter().filter(|h| **h > 1).count() as u64;
    Ok(d)
}
