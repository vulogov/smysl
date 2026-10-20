//! K1, keys (SMYSL-2.6 §3.11): the questions a view or a dossier asks, the answers the store
//! gives them, and how far each answer is from the text.
//!
//! # Items
//!
//! A view's items are the kernel `question` units among its members
//! (`smysl_graph::lineage::membership` of its roots) with at least one live `answers` edge
//! into them; the answers are the units those edges come from. A dossier's view also has, as
//! roots, one saved query per slot (SMYSL-2.5 §3.6), and each slot that found units is an item
//! of its own: the saved query is its question and the units its `x.query/answers` edges name
//! are its answers. A view of neither kind has no item, and is refused (`SMY-W485`).
//!
//! # Difficulty priors
//!
//! `d_path` is the shortest walk along `grounds` from an answer to a unit whose source names a
//! span, over the item's answers — 0 when an answer was itself drawn from the text, and absent
//! when no walk reaches one. `d_spans` is the distinct `(source, span)` places among every unit
//! the walk reaches: how much of the text the answer rests on.
//!
//! # The key unit
//!
//! `@data`, `derived`, grounded on the items' questions; `key:items`, `key:scope`, `key:policy`
//! and `key:view` in the payload. Beside it, a view whose roots are the questions, so that the
//! key is also something `view` and `pack` can read.

use std::collections::{BTreeSet, VecDeque};

use smysl_core::diag::Code;
use smysl_core::schemas::query;
use smysl_core::surface::hjson::HValue;
use smysl_core::types::RelKind;
use smysl_core::{KernelType, SchemaId, Uid, UnitCore, View, ViewId};
use smysl_graph::Store;

use crate::assess::{self, as_array, as_int, as_str, as_uids, get, int, object, text, uids};
use crate::recipe::{at, ClassPolicy};
use crate::Refusal;

/// Whether an item is answered by any of its answers or needs each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Mode {
    #[default]
    Any,
    All,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Any => "any",
            Mode::All => "all",
        }
    }

    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "any" => Some(Mode::Any),
            "all" => Some(Mode::All),
            _ => None,
        }
    }
}

/// One question of a key.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Item {
    pub q: Uid,
    /// The keyed answers, in uid order.
    pub answers: Vec<Uid>,
    pub mode: Mode,
    pub d_path: Option<u64>,
    pub d_spans: u64,
}

/// A key: its items, in question order, and what it was made under.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Key {
    pub items: Vec<Item>,
    /// The sq scope fabrication is judged against; empty for none.
    pub scope: String,
    pub policy: ClassPolicy,
    /// The view the key was made from.
    pub view: String,
}

fn schema_is(store: &Store, uid: &Uid, schema: &SchemaId) -> bool {
    store.get(uid).is_some_and(|u| &u.core.schema == schema)
}

/// `(d_path, d_spans)` of `answers`.
fn difficulty(store: &Store, answers: &[Uid]) -> (Option<u64>, u64) {
    let mut seen: BTreeSet<Uid> = answers.iter().copied().collect();
    let mut queue: VecDeque<(Uid, u64)> = answers.iter().map(|a| (*a, 0)).collect();
    let mut best: Option<u64> = None;
    let mut spans: BTreeSet<(String, (u64, u64))> = BTreeSet::new();
    while let Some((u, d)) = queue.pop_front() {
        let Some(unit) = store.get(&u) else {
            continue;
        };
        if let Some(src) = unit.core.source.as_ref() {
            if let Some(span) = src.span() {
                let part = src
                    .reference
                    .split_once('#')
                    .map_or(src.reference.as_str(), |(p, _)| p);
                spans.insert((part.to_string(), span));
                best = Some(best.map_or(d, |b| b.min(d)));
            }
        }
        for g in &unit.core.grounds {
            if seen.insert(*g) {
                queue.push_back((*g, d + 1));
            }
        }
    }
    (best, spans.len() as u64)
}

/// The items of `view`: its questions with live answers, and its dossier slots with results.
fn items(store: &Store, view: &View, mode: Mode) -> Vec<Item> {
    let members: BTreeSet<Uid> = smysl_graph::lineage::membership(store, &view.roots)
        .into_iter()
        .chain(view.roots.iter().copied())
        .collect();
    let question = SchemaId::Kernel(KernelType::Question);
    let saved = SchemaId::Extension(query::SAVED.to_string());
    let mut out: Vec<(Uid, BTreeSet<Uid>)> = Vec::new();
    let answers_into = |kind: &RelKind, q: &Uid, from_q: bool| -> BTreeSet<Uid> {
        store
            .relations_of_kind(kind)
            .into_iter()
            .filter(|r| store.contains_uid(&r.from) && store.contains_uid(&r.to))
            .filter_map(|r| match from_q {
                false if r.to == *q && r.from != *q => Some(r.from),
                true if r.from == *q && r.to != *q => Some(r.to),
                _ => None,
            })
            .collect()
    };
    for u in &members {
        if schema_is(store, u, &question) {
            let a = answers_into(&RelKind::Answers, u, false);
            if !a.is_empty() {
                out.push((*u, a));
            }
        }
    }
    for u in &view.roots {
        if schema_is(store, u, &saved) {
            let a = answers_into(&RelKind::Extension(query::ANSWERS.to_string()), u, true);
            if !a.is_empty() {
                out.push((*u, a));
            }
        }
    }
    out.sort();
    out.into_iter()
        .map(|(q, a)| {
            let answers: Vec<Uid> = a.into_iter().collect();
            let (d_path, d_spans) = difficulty(store, &answers);
            Item {
                q,
                answers,
                mode,
                d_path,
                d_spans,
            }
        })
        .collect()
}

/// The key `view` asks for, under `policy`, judged later against `scope` (empty for none).
pub fn key_from_view(
    store: &Store,
    view: &View,
    policy: ClassPolicy,
    scope: &str,
    mode: Mode,
) -> Result<Key, Refusal> {
    let items = items(store, view, mode);
    if items.is_empty() {
        return Err(Refusal::new(
            Code::W485,
            format!(
                "view `{}` has no question with a live answer and no dossier slot with results, \
                 so there is no item to key",
                view.id.as_str()
            ),
        ));
    }
    Ok(Key {
        items,
        scope: scope.trim().to_string(),
        policy,
        view: view.id.as_str().to_string(),
    })
}

impl Key {
    /// The key as a `@data` unit, grounded on its questions.
    pub fn to_unit(&self) -> Result<UnitCore, Refusal> {
        let items: Vec<HValue> = self
            .items
            .iter()
            .map(|i| {
                let mut f = vec![
                    ("q", text(i.q.canonical())),
                    ("answers", uids(&i.answers)),
                    ("mode", text(i.mode.as_str())),
                    ("d_spans", int(i.d_spans)),
                ];
                if let Some(d) = i.d_path {
                    f.push(("d_path", int(d)));
                }
                HValue::Object(object(f))
            })
            .collect();
        let mut fields = vec![
            (
                "key:items",
                HValue::Array(items.into_iter().map(at).collect()),
            ),
            ("key:policy", text(self.policy.to_string())),
            ("key:view", text(self.view.clone())),
        ];
        if !self.scope.is_empty() {
            fields.push(("key:scope", text(self.scope.clone())));
        }
        let gist = format!(
            "K1 key of {} item(s) from view {}",
            self.items.len(),
            self.view
        );
        assess::data_unit(&gist, self.items.iter().map(|i| i.q), &object(fields))
    }

    /// The view whose roots are the key's questions.
    pub fn as_view(&self, id: ViewId) -> View {
        View::new(id, "assess-key").with_roots(self.items.iter().map(|i| i.q))
    }

    /// A key read back from its unit.
    pub fn from_unit(store: &Store, uid: &Uid) -> Result<Key, Refusal> {
        let o = assess::payload_of(store, uid, "a key")?;
        let bad = |why: &str| {
            Refusal::new(
                Code::E489,
                format!("`{}` is not a key: {why}", uid.canonical()),
            )
        };
        let policy = as_str(get(&o, "key:policy"))
            .and_then(ClassPolicy::parse)
            .ok_or_else(|| bad("no key:policy"))?;
        let mut items = Vec::new();
        for x in as_array(get(&o, "key:items")).ok_or_else(|| bad("no key:items"))? {
            let HValue::Object(i) = &x.value else {
                return Err(bad("an item is not a map"));
            };
            let q = as_str(get(i, "q"))
                .and_then(|s| Uid::parse(s).ok())
                .ok_or_else(|| bad("an item has no question"))?;
            items.push(Item {
                q,
                answers: as_uids(get(i, "answers")).ok_or_else(|| bad("an item has no answers"))?,
                mode: as_str(get(i, "mode"))
                    .and_then(Mode::parse)
                    .ok_or_else(|| bad("an item has no mode"))?,
                d_path: as_int(get(i, "d_path")),
                d_spans: as_int(get(i, "d_spans")).unwrap_or(0),
            });
        }
        Ok(Key {
            items,
            scope: as_str(get(&o, "key:scope")).unwrap_or("").to_string(),
            policy,
            view: as_str(get(&o, "key:view")).unwrap_or("").to_string(),
        })
    }
}
