//! `x.text/replies`: the edges a conversation's own reply ids imply, between the units drawn
//! from its messages (SMYSL-2.4 §3.4, SMYSL-2.5 §6 TX-P8 step 2).
//!
//! A chat reader records what a message answers as an id on its row — Telegram's
//! `reply_to_message_id` as `reply`, Slack's `thread_ts` as `thread` — and a row is not a unit.
//! `x.text/v1` declares the edge, rule E reads it as a free constraint and `dialogue` derives
//! by it; until this module nothing turned a row's id into one. It does that and nothing else,
//! as a function of the readings and of where units sit in them, so that the ingest that drew
//! the units and a test that drew them by hand get the same edges.
//!
//! # The rule
//!
//! 1. **What a message answers.** A row's `reply` id, or failing that its `thread` id, names the
//!    message whose `msg` id it is, among the rows of the same expression. Slack's thread is
//!    flat — every reply in it names the message that opened it — so a Slack reply answers the
//!    opening message, which is what the export says and no more. A row that names itself
//!    answers nothing.
//! 2. **Where a unit sits.** A unit belongs to the message whose row holds its whole span, in
//!    the part the span is an offset into. A unit with no span sits nowhere, and the caller
//!    leaves out what is not the message's own claim — an entity has no span, and a translation
//!    says what another unit says, which already carries the edge.
//! 3. **The edges.** From every unit of the answering message to every unit of the message it
//!    answers. A message with no units gives none and takes none; nothing is invented to stand
//!    for it.
//! 4. **What is not there.** An answered id that names no row of the expression is reported
//!    (`SMY-W448`) and skipped: the message was deleted, or lives in a part this expression
//!    does not hold.
//!
//! The edges are content-addressed relations, so producing them twice is producing them once,
//! and they depend on the units the store holds rather than on the order windows ran in — a
//! reply ingested before the message it answers is linked as soon as both are there.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::ids::Tid;
use smysl_core::Uid;

use crate::reading::Segment;

/// The row id a reader records a message's own identifier under.
pub const MSG: &str = "msg";
/// The row id Telegram's reader records the message a reply answers under.
pub const REPLY: &str = "reply";
/// The row id Slack's reader records a threaded reply's opening message under.
pub const THREAD: &str = "thread";

/// The message a row answers, by the reader's own id for it (rule 1).
pub fn answered(row: &Segment) -> Option<&str> {
    let named = row.ids.get(REPLY).or_else(|| row.ids.get(THREAD))?;
    (row.ids.get(MSG) != Some(named)).then_some(named.as_str())
}

/// A unit and where its span lies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sited {
    pub uid: Uid,
    /// The part the span is an offset into.
    pub tid: Tid,
    /// Half-open, in part coordinates.
    pub span: (u64, u64),
}

/// What [`link`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Linked {
    /// `(answering, answered)`, sorted and without repeats: the `from` and `to` of each edge.
    pub pairs: Vec<(Uid, Uid)>,
    /// Answered ids that name no message of the expression (rule 4), sorted and without
    /// repeats.
    pub unresolved: Vec<String>,
}

/// The reply edges an expression's readings imply between `units` (rules 1 to 4).
///
/// `parts` are the expression's parts with their reading rows. Rows that carry no `msg` id are
/// not messages — a sentence row inside one, or a reading with no messages at all — and take no
/// part.
pub fn link(parts: &[(Tid, &[Segment])], units: &[Sited]) -> Linked {
    // Every message, in part order and by its start within the part, so a span finds its row by
    // one search.
    let mut rows: Vec<(Tid, &Segment)> = Vec::new();
    for (tid, segs) in parts {
        rows.extend(
            segs.iter()
                .filter(|s| s.ids.contains_key(MSG))
                .map(|s| (*tid, s)),
        );
    }
    let mut by_start: BTreeMap<(Tid, u64), usize> = BTreeMap::new();
    let mut by_msg: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, (tid, s)) in rows.iter().enumerate() {
        by_start.insert((*tid, s.start), i);
        by_msg.entry(s.ids[MSG].as_str()).or_default().push(i);
    }

    let mut held: Vec<BTreeSet<Uid>> = vec![BTreeSet::new(); rows.len()];
    for u in units {
        let (start, end) = u.span;
        let Some((&(tid, _), &i)) = by_start.range(..=(u.tid, start)).next_back() else {
            continue;
        };
        if tid == u.tid && end <= rows[i].1.end {
            held[i].insert(u.uid);
        }
    }

    let mut pairs = BTreeSet::new();
    let mut unresolved = BTreeSet::new();
    for (i, (_, row)) in rows.iter().enumerate() {
        let Some(id) = answered(row) else { continue };
        let Some(targets) = by_msg.get(id) else {
            unresolved.insert(id.to_string());
            continue;
        };
        for &t in targets {
            for from in &held[i] {
                for to in &held[t] {
                    if from != to {
                        pairs.insert((*from, *to));
                    }
                }
            }
        }
    }
    Linked {
        pairs: pairs.into_iter().collect(),
        unresolved: unresolved.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locator;
    use crate::reading::Level;

    fn tid(n: u8) -> Tid {
        Tid::from_bytes([n; 32])
    }

    fn uid(n: u8) -> Uid {
        Uid::from_bytes([n; 32])
    }

    fn row(start: u64, end: u64, ids: &[(&str, &str)]) -> Segment {
        let mut s = Segment::new(
            start,
            end,
            Level::new("message").unwrap(),
            locator::parse(&format!("m{start}")).unwrap(),
        );
        for (k, v) in ids {
            s = s.with_id(*k, *v);
        }
        s
    }

    fn at(n: u8, part: u8, start: u64, end: u64) -> Sited {
        Sited {
            uid: uid(n),
            tid: tid(part),
            span: (start, end),
        }
    }

    #[test]
    fn a_reply_links_every_unit_of_the_answer_to_every_unit_of_what_it_answers() {
        let rows = [
            row(0, 10, &[("msg", "1")]),
            row(10, 20, &[("msg", "2"), ("reply", "1")]),
        ];
        let units = [at(1, 1, 0, 4), at(2, 1, 5, 10), at(3, 1, 12, 18)];
        let l = link(&[(tid(1), &rows)], &units);
        assert_eq!(l.pairs, vec![(uid(3), uid(1)), (uid(3), uid(2))]);
        assert!(l.unresolved.is_empty());
    }

    #[test]
    fn a_slack_thread_answers_the_message_that_opened_it() {
        let rows = [
            row(0, 10, &[("msg", "100.1")]),
            row(10, 20, &[("msg", "100.2"), ("thread", "100.1")]),
            row(20, 30, &[("msg", "100.3"), ("thread", "100.1")]),
        ];
        let units = [at(1, 1, 0, 10), at(2, 1, 10, 20), at(3, 1, 20, 30)];
        let l = link(&[(tid(1), &rows)], &units);
        assert_eq!(l.pairs, vec![(uid(2), uid(1)), (uid(3), uid(1))]);
    }

    #[test]
    fn a_reply_across_parts_is_found_and_a_missing_message_is_reported() {
        let first = [row(0, 10, &[("msg", "1")])];
        let second = [
            row(0, 10, &[("msg", "2"), ("reply", "1")]),
            row(10, 20, &[("msg", "3"), ("reply", "9")]),
        ];
        let units = [at(1, 1, 0, 10), at(2, 2, 0, 10), at(3, 2, 10, 20)];
        let l = link(&[(tid(1), &first), (tid(2), &second)], &units);
        assert_eq!(l.pairs, vec![(uid(2), uid(1))]);
        assert_eq!(l.unresolved, vec!["9".to_string()]);
    }

    #[test]
    fn a_span_that_leaves_its_message_sits_nowhere_and_links_nothing() {
        let rows = [
            row(0, 10, &[("msg", "1")]),
            row(10, 20, &[("msg", "2"), ("reply", "1")]),
        ];
        // The second unit runs across both messages, and the third is in another part.
        let units = [at(1, 1, 0, 10), at(2, 1, 5, 15), at(3, 2, 12, 18)];
        let l = link(&[(tid(1), &rows)], &units);
        assert!(l.pairs.is_empty(), "{:?}", l.pairs);
    }

    #[test]
    fn a_message_that_names_itself_or_has_no_units_gives_no_edge() {
        let rows = [
            row(0, 10, &[("msg", "1"), ("reply", "1")]),
            row(10, 20, &[("msg", "2"), ("reply", "1")]),
            row(20, 30, &[("msg", "3"), ("reply", "2")]),
        ];
        // Message 2 has no unit: 3 answers nothing that is there, and 2 links nothing to 1.
        let units = [at(1, 1, 0, 10), at(3, 1, 20, 30)];
        let l = link(&[(tid(1), &rows)], &units);
        assert!(l.pairs.is_empty(), "{:?}", l.pairs);
        assert!(l.unresolved.is_empty());
    }

    #[test]
    fn rows_without_a_message_id_are_not_messages() {
        let rows = [
            row(0, 10, &[]),
            row(10, 20, &[("msg", "2"), ("reply", "1")]),
        ];
        let units = [at(1, 1, 0, 10), at(2, 1, 10, 20)];
        let l = link(&[(tid(1), &rows)], &units);
        assert!(l.pairs.is_empty());
        assert_eq!(l.unresolved, vec!["1".to_string()]);
    }
}
