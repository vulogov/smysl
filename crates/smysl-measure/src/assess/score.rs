//! K3, scoring (SMYSL-2.6 §3.11): a reader's responses against a key.
//!
//! # What is scored
//!
//! A **response** is a unit the reader attested with a live `answers` edge to an item's
//! question. A unit the reader attested that answers no item is **unscorable**: counted and
//! skipped (`SMY-W487`). An item whose question or any answer has drifted — gone, retracted,
//! superseded or unfounded since the key was made ([`crate::assess::drift`]) — is excluded and
//! counted (`SMY-W488`), and its responses with it.
//!
//! # Matching
//!
//! Classes are formed under the policy over the item's answers and responses together. A
//! response **matches** a keyed answer when the two share a class, or when the judge proposes
//! the response with a member of that answer's class (the module header of [`crate::assess`]
//! says why a proposal is not written). The judge is asked once, over every live item's
//! answers and responses: SMYSL-2.4's engines rank neighbours and have no threshold, so within
//! one item a response's nearest unit could only be that item's own answer, and every
//! response would match. Across the key, a wrong response's nearest answer is usually another
//! item's, and a proposal pairing a response with another item's answer matches nothing. An `any` item is **correct** when some answer is
//! matched; an `all` item when every answer is, and it records `[answers matched, answers]`
//! either way. An item with no response is **unanswered**, counted apart from a wrong one.
//!
//! # Fidelity and fabrication
//!
//! Each matched response is paired with the strongest keyed answer it matched (status, then
//! the smaller uid). **Inflation** is `[pairs where the response's effective status is above
//! the answer's, pairs]`, C3's comparison; **holder change** is C4's over the same pairs.
//! **Fabrication** is `[responses whose class has no member in the key's scope, responses]`,
//! the classes formed over the scope and the responses — relative to the texts the key names,
//! never to the world (draft 3 §24); a matched response is in its answer's class and is not
//! fabricated. A key with no scope has no fabrication figure.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::{Code, Diagnostic};
use smysl_core::surface::hjson::HValue;
use smysl_core::types::RelKind;
use smysl_core::{AgentId, Status, Uid, UnitCore};
use smysl_graph::Store;

use crate::assess::{self, int, object, ratio, text, uids, Judge, Key};
use crate::channel::{self, c4};
use crate::graph::epistemic::status_of;
use crate::recipe::{at, ClassPolicy};
use crate::Refusal;

/// One item's score.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ItemScore {
    pub q: Uid,
    /// The reader's responses to it, in uid order.
    pub responses: Vec<Uid>,
    /// `[answers matched, answers]`.
    pub matched: (u64, u64),
    pub correct: bool,
}

/// A reader's score against a key.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Score {
    pub reader: AgentId,
    pub policy: ClassPolicy,
    pub judge: &'static str,
    /// The items scored, in question order; drifted items are not here.
    pub items: Vec<ItemScore>,
    /// `[correct items, items scored]`.
    pub accuracy: (u64, u64),
    pub unanswered: u64,
    /// Answered and not correct.
    pub wrong: u64,
    /// `[inflated pairs, matched pairs]`.
    pub inflation: (u64, u64),
    /// `[pairs whose holder changed, matched pairs]`.
    pub holder: (u64, u64),
    /// `[fabricated responses, responses]`; `None` for a key with no scope.
    pub fabrication: Option<(u64, u64)>,
    /// The reader's units that answer no item (`SMY-W487`).
    pub unscorable: u64,
    /// Items excluded because their question or an answer drifted (`SMY-W488`).
    pub drifted: u64,
    /// `SMY-W487` and `SMY-W488`, when there is anything to say.
    pub diagnostics: Vec<Diagnostic>,
}

/// What matching found for one item: each matched response and its strongest partner.
pub(crate) struct Matched {
    pub pairs: Vec<(Uid, Uid)>,
    /// The keyed answers some response matched.
    pub answered: BTreeSet<Uid>,
}

/// The responses of `reader` to each of `key`'s live items, and the counts W487 and W488 say.
pub(crate) struct Responses {
    pub by_item: Vec<(usize, Vec<Uid>)>,
    pub unscorable: u64,
    pub drifted: u64,
}

pub(crate) fn gather(store: &Store, key: &Key, reader: &AgentId) -> Responses {
    let mine: BTreeSet<Uid> = assess::responses(store, reader).into_iter().collect();
    let mut answering: BTreeMap<Uid, BTreeSet<Uid>> = BTreeMap::new();
    for r in store.relations_of_kind(&RelKind::Answers) {
        if mine.contains(&r.from) {
            answering.entry(r.to).or_default().insert(r.from);
        }
    }
    let mut used: BTreeSet<Uid> = BTreeSet::new();
    let mut by_item = Vec::new();
    let mut drifted = 0u64;
    for (k, item) in key.items.iter().enumerate() {
        let to_item: Vec<Uid> = answering
            .get(&item.q)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default();
        used.extend(to_item.iter().copied());
        let gone = std::iter::once(&item.q)
            .chain(&item.answers)
            .any(|u| assess::drift(store, u).is_some());
        if gone {
            drifted += 1;
            continue;
        }
        by_item.push((k, to_item));
    }
    Responses {
        by_item,
        unscorable: mine.difference(&used).count() as u64,
        drifted,
    }
}

/// What `judge` proposes over every live item's answers and responses at once: a judge that
/// ranks neighbours always finds one, so it is asked across the whole key, where a response's
/// nearest unit can be another item's answer, and not within one item, where it could only be
/// its own.
pub(crate) fn proposals(
    store: &Store,
    key: &Key,
    got: &Responses,
    judge: &Judge<'_>,
) -> BTreeSet<(Uid, Uid)> {
    let mut universe: BTreeSet<Uid> = BTreeSet::new();
    for (k, responses) in &got.by_item {
        universe.extend(key.items[*k].answers.iter().copied());
        universe.extend(responses.iter().copied());
    }
    judge.propose(store, &universe)
}

/// Match `responses` against `answers` under `policy`, with what a judge `proposed`.
pub(crate) fn match_item(
    store: &Store,
    policy: smysl_graph::proposition::Policy,
    proposed: &BTreeSet<(Uid, Uid)>,
    answers: &[Uid],
    responses: &[Uid],
) -> Matched {
    let all: BTreeSet<Uid> = answers.iter().chain(responses).copied().collect();
    let class = assess::classes(store, policy, &all);
    let joined = |r: &Uid, a: &Uid| {
        class[r] == class[a]
            || answers.iter().any(|x| {
                class[x] == class[a] && proposed.contains(&if r < x { (*r, *x) } else { (*x, *r) })
            })
    };
    let mut pairs = Vec::new();
    let mut answered = BTreeSet::new();
    for r in responses {
        let hit: Vec<Uid> = answers.iter().filter(|a| joined(r, a)).copied().collect();
        answered.extend(hit.iter().copied());
        if let Some(best) = channel::strongest(store, &hit) {
            pairs.push((*r, best));
        }
    }
    Matched { pairs, answered }
}

/// Score `reader` against `key` under `policy` (the key's own when `None`), with `judge`;
/// `scope` is the key's scope resolved, for fabrication.
pub fn score(
    store: &Store,
    key: &Key,
    reader: &AgentId,
    policy: Option<ClassPolicy>,
    judge: &Judge<'_>,
    scope: Option<&BTreeSet<Uid>>,
) -> Result<Score, Refusal> {
    let policy = policy.unwrap_or(key.policy);
    let graph_policy = assess::policy(Some(policy))?;
    let got = gather(store, key, reader);
    let proposed = proposals(store, key, &got, judge);
    let level = |u: &Uid| status_of(store, u).unwrap_or(Status::Unfounded).as_u8();
    let mut items = Vec::new();
    let (mut correct, mut unanswered, mut wrong) = (0u64, 0u64, 0u64);
    let mut pairs: Vec<(Uid, Uid)> = Vec::new();
    let mut matched_responses: BTreeSet<Uid> = BTreeSet::new();
    let mut all_responses: BTreeSet<Uid> = BTreeSet::new();
    for (k, responses) in &got.by_item {
        let item = &key.items[*k];
        all_responses.extend(responses.iter().copied());
        let m = match_item(store, graph_policy, &proposed, &item.answers, responses);
        let n = item.answers.len() as u64;
        let hit = m.answered.len() as u64;
        let ok = match item.mode {
            assess::Mode::Any => hit > 0,
            assess::Mode::All => hit == n,
        };
        if responses.is_empty() {
            unanswered += 1;
        } else if ok {
            correct += 1;
        } else {
            wrong += 1;
        }
        matched_responses.extend(m.pairs.iter().map(|(r, _)| *r));
        pairs.extend(m.pairs);
        items.push(ItemScore {
            q: item.q,
            responses: responses.clone(),
            matched: (hit, n),
            correct: ok && !responses.is_empty(),
        });
    }
    let inflated = pairs.iter().filter(|(r, a)| level(r) > level(a)).count() as u64;
    // C4 reads `(target, source)`: the response is what the answer was carried into.
    let held = c4::changes(store, graph_policy, &pairs);
    let fabrication = scope.map(|s| {
        let over: BTreeSet<Uid> = s.iter().chain(&all_responses).copied().collect();
        let class = assess::classes(store, graph_policy, &over);
        let grounded: BTreeSet<Uid> = s.iter().map(|u| class[u]).collect();
        let made_up = all_responses
            .iter()
            .filter(|r| !matched_responses.contains(r) && !grounded.contains(&class[r]))
            .count() as u64;
        (made_up, all_responses.len() as u64)
    });
    let mut diagnostics = Vec::new();
    if got.unscorable > 0 {
        diagnostics.push(Diagnostic::new(Code::W487).with_message(format!(
            "{} unit(s) {} attested answer no item of the key, and are not scored",
            got.unscorable,
            reader.as_str()
        )));
    }
    if got.drifted > 0 {
        diagnostics.push(Diagnostic::new(Code::W488).with_message(format!(
            "{} item(s) of the key have a question or an answer that has since been withdrawn, \
             retracted, superseded or become unfounded; they are left out",
            got.drifted
        )));
    }
    let scored = items.len() as u64;
    Ok(Score {
        reader: reader.clone(),
        policy,
        judge: judge.id(),
        items,
        accuracy: (correct, scored),
        unanswered,
        wrong,
        inflation: (inflated, pairs.len() as u64),
        holder: (held.changed, pairs.len() as u64),
        fabrication,
        unscorable: got.unscorable,
        drifted: got.drifted,
        diagnostics,
    })
}

impl Score {
    /// The score as a `@data` unit, grounded on the key and the responses scored.
    pub fn to_unit(&self, key: &Uid) -> Result<UnitCore, Refusal> {
        let items: Vec<HValue> = self
            .items
            .iter()
            .map(|i| {
                HValue::Object(object(vec![
                    ("q", text(i.q.canonical())),
                    ("responses", uids(&i.responses)),
                    ("matched", ratio(i.matched)),
                    ("correct", HValue::Bool(i.correct)),
                ]))
            })
            .collect();
        let mut summary = vec![
            ("accuracy", ratio(self.accuracy)),
            ("inflation", ratio(self.inflation)),
            ("holder", ratio(self.holder)),
            ("unanswered", int(self.unanswered)),
            ("wrong", int(self.wrong)),
            ("unscorable", int(self.unscorable)),
            ("drifted", int(self.drifted)),
        ];
        if let Some(f) = self.fabrication {
            summary.push(("fabrication", ratio(f)));
        }
        let payload = object(vec![
            ("score:key", text(key.canonical())),
            ("score:reader", text(self.reader.as_str())),
            ("score:policy", text(self.policy.to_string())),
            ("score:judge", text(self.judge)),
            (
                "score:items",
                HValue::Array(items.into_iter().map(at).collect()),
            ),
            ("score:summary", HValue::Object(object(summary))),
        ]);
        let (c, n) = self.accuracy;
        let gist = format!(
            "K3 score of {}: {c}/{n} correct, {} unanswered",
            self.reader.as_str(),
            self.unanswered
        );
        let grounds =
            std::iter::once(*key).chain(self.items.iter().flat_map(|i| i.responses.clone()));
        assess::data_unit(&gist, grounds, &payload)
    }
}
