//! Windows over a reading: whole nodes, filled to a budget, with one node of overlap.
//!
//! A chunk of a document (`crate::chunk`) is a slice of prose with no structure behind it: it
//! splits at paragraphs because paragraphs are all it can see. A **window** over a library part
//! is different in kind — the reading says where the verses, the messages or the sections are, so
//! a window is a whole number of them. That is not tidiness. A node is the unit a locator names
//! and the unit a span is attached inside, so a window that cut one in half would offer the model
//! half a verse and then have nowhere to anchor what it said about it.
//!
//! # The three rules
//!
//! 1. **A window is whole nodes.** Filled in order up to `budget` tokens, measured with the
//!    caller's estimator, never splitting a node.
//! 2. **One node of overlap.** Each window after the first opens with the last node the previous
//!    window owned, so a claim spanning a boundary is visible whole to one call. The overlap node
//!    is *not* owned: [`TextWindow::owned`] is the index range this window is responsible for, and
//!    the owned ranges partition the text exactly once.
//! 3. **The one exception.** A node bigger than the whole budget cannot be a window, so it is cut
//!    at sentence boundaries and `SMY-W447` says so. Nothing else splits a node, and nothing cuts
//!    inside a sentence — a piece whose single sentence exceeds the budget goes over it rather
//!    than being cut mid-clause, because a half-sentence is not a quotable place.
//!
//! # The budget is counted conservatively
//!
//! A window's token count is the **sum of its nodes' counts**, not the count of the rendered
//! text. For `ceil(len/4)` those differ: `ceil(a/4) + ceil(b/4) >= ceil((a+b)/4)`, so the sum is
//! an over-estimate by at most one token per node. That direction is the safe one — a window
//! never exceeds the budget it was given — and it makes the fill cost linear instead of
//! re-counting the whole accumulated string for every node.
//!
//! # Document order is the caller's
//!
//! Panes are windowed in the order they are handed in, and the nodes inside a pane in the order
//! the reading gives. §3.4's "chats follow reply threads" is therefore a property of the sequence
//! `ingest --text` builds (step 4), not a second ordering here: a windower that re-sorted its
//! input would be deciding reading order twice, in two places, from less information than the
//! command has.

use std::ops::Range;

use smysl_core::diag::{Code, Diagnostic};
use smysl_core::{Tid, TokenEstimator};
use smysl_text::reading::{Level, Segment};
use smysl_text::segment::Segmenter;
use smysl_text::structure::Structure;
use smysl_text::Locator;

/// One part, as windowing needs to see it.
///
/// The rows as well as the structure, and not for the tree's sake: [`Structure`] drops a
/// segment's `observed`, because the tree is derived from byte ranges alone, and the copy set
/// (D-1 rule 3) needs that instant for a chat message. A pane that could not supply it would
/// make every message's `observed` the caller's guess.
#[derive(Debug, Clone, Copy)]
pub struct Pane<'a> {
    pub tid: Tid,
    /// The part's normalised bytes. Spans are offsets into exactly these.
    pub text: &'a str,
    pub structure: &'a Structure,
    /// The reading's rows, indexed by [`smysl_text::structure::Node::row`].
    pub rows: &'a [Segment],
    /// Byte ranges of this part that are **reported speech**, in part coordinates.
    ///
    /// An input rather than something windowing discovers, because the thing that knows is
    /// whatever marked them: a reader that reads USFM's `\wj` or OSIS's `<q who>` (none does
    /// yet), or [`crate::text::attribution::quoted_spans`] over the text's own punctuation.
    /// Empty is the honest default — "nothing is marked", not "nothing is reported".
    pub reported: &'a [Range<u64>],
}

impl<'a> Pane<'a> {
    pub fn new(tid: Tid, text: &'a str, structure: &'a Structure, rows: &'a [Segment]) -> Pane<'a> {
        Pane {
            tid,
            text,
            structure,
            rows,
            reported: &[],
        }
    }

    /// The part's reported-speech marks (§3.4, step 3b).
    pub fn with_reported(mut self, marks: &'a [Range<u64>]) -> Pane<'a> {
        self.reported = marks;
        self
    }
}

/// A node as a window holds it: where it came from, and where it sits in the window's text.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Placed {
    pub tid: Tid,
    /// The row this came from, so a caller can find the segment again.
    pub row: u32,
    /// The locator of the node, which is what `ingest:node` names and what the copy set carries
    /// as the informative half of the reference.
    pub locator: Locator,
    /// The byte range **in the part**. Narrower than the node when the node was split.
    pub part: Range<u64>,
    /// The byte range **in the window's text**.
    pub at: Range<usize>,
    /// Whether this is one piece of a node too large for a window (`SMY-W447`).
    pub split: bool,
    /// The segment's `observed`, when the reading gave one.
    pub observed: Option<u64>,
    /// The segment's speaker, when the reading gave one. A chat reader gives one per message;
    /// a Bible reader gives none. This is what `text:speaker` and the holder are read from
    /// (§3.4, step 3b).
    pub speaker: Option<String>,
    /// The part's reported-speech marks, clipped to this node.
    pub reported: Vec<Range<u64>>,
}

impl Placed {
    /// A window offset inside this node, as an offset into the part.
    pub fn into_part(&self, at: usize) -> Option<u64> {
        if !self.at.contains(&at) && at != self.at.end {
            return None;
        }
        Some(self.part.start + (at - self.at.start) as u64)
    }
}

/// One call's worth of text.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TextWindow {
    /// The overlap node first, when there is one, then the owned nodes in order.
    pub nodes: Vec<Placed>,
    /// Which of [`TextWindow::nodes`] this window owns. The owned ranges of all windows
    /// partition the text.
    pub owned: Range<usize>,
    /// The index in [`TextWindow::nodes`] of the one node carried over from the previous window.
    pub overlap: Option<usize>,
    /// The nodes' text, joined by newlines. This is what a quote is searched in; the prompt's
    /// per-node rendering with locator markers is step 4's and is not this.
    pub text: String,
}

impl TextWindow {
    /// The window as a model is shown it: one node per line, each prefixed with its locator
    /// (§3.9.2).
    ///
    /// Not [`TextWindow::text`], and the difference is load-bearing. This is what goes inside the
    /// fence, because a model asked which node it read a claim in needs the names in front of it;
    /// `text` is what a quote is **checked** against, so that a quote is not required to contain
    /// `[v.3]` in order to be `Present`. Two strings for two jobs, and the attachment maps
    /// offsets in the second.
    pub fn rendered(&self) -> String {
        let mut out = String::new();
        for node in &self.nodes {
            out.push('[');
            out.push_str(&node.locator.to_string());
            out.push_str("] ");
            out.push_str(&self.text[node.at.clone()]);
            out.push('\n');
        }
        out
    }

    /// The nodes this window is responsible for.
    pub fn owned_nodes(&self) -> &[Placed] {
        &self.nodes[self.owned.start..self.owned.end]
    }

    /// The node a window offset falls in.
    pub fn node_at(&self, at: usize) -> Option<&Placed> {
        self.nodes.iter().find(|n| n.at.contains(&at))
    }

    /// A window offset as `(part, offset in that part)`.
    ///
    /// The `map` of §4.3.4, as a method rather than a field: the mapping is already stated
    /// exactly once, by the nodes' `at` and `part` ranges, and a second copy of it beside them
    /// would be a second thing to keep true.
    pub fn map(&self, at: usize) -> Option<(Tid, u64)> {
        let n = self.node_at(at)?;
        Some((n.tid, n.into_part(at)?))
    }
}

/// The separator between nodes in a window's text.
///
/// One byte, and it is in the text rather than implied, because a span found across two nodes
/// would otherwise be a span over text that is not contiguous in any part. A newline cannot be
/// matched by a quote that did not contain one, which is what keeps `support_span` from gluing
/// two verses together.
pub(crate) const JOIN: &str = "\n";

/// The level a window should be filled with when the caller names none.
///
/// The **leaves**: the level of the most numerous nodes that have no children. A window of whole
/// leaf nodes is the finest division the reading offers, and a leaf is the unit a locator names
/// and a span is attached inside — so a coarser default would hand the model a chapter and then
/// have nowhere narrower to anchor what it said about one verse of it.
///
/// Most numerous rather than "the deepest", because a reading may have one odd leaf at another
/// level — a chapter with no verses marked, a day with no messages — and the odd one must not
/// decide how the whole expression is windowed. `None` for a structure with no nodes at all.
pub fn leaf_level(s: &Structure) -> Option<Level> {
    let mut counts: std::collections::BTreeMap<&Level, usize> = std::collections::BTreeMap::new();
    for node in s.nodes() {
        if node.children.is_empty() {
            *counts.entry(&node.level).or_default() += 1;
        }
    }
    // Ties go to the level that sorts first, so the answer is the same on every run and on
    // every machine: a default that depended on a hash map's order would make the recipe of an
    // unparameterised run unreproducible.
    counts
        .into_iter()
        .max_by_key(|(level, n)| (*n, std::cmp::Reverse((*level).clone())))
        .map(|(level, _)| level.clone())
}

/// Fill windows over `panes` with the nodes at `level`.
///
/// Returns the windows in order, and the diagnostics windowing itself produced — today exactly
/// the `SMY-W447` of rule 3, one per node that had to be cut.
pub fn windows(
    panes: &[Pane<'_>],
    level: &Level,
    budget: u32,
    est: TokenEstimator,
    seg: &Segmenter,
) -> (Vec<TextWindow>, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    let mut pieces: Vec<Piece> = Vec::new();

    for pane in panes {
        for node in pane.structure.at_level(level) {
            let Some(text) = slice(pane.text, &node.range) else {
                continue;
            };
            if text.trim().is_empty() {
                continue;
            }
            let row = pane.rows.get(node.row as usize);
            let observed = row.and_then(|r| r.observed);
            let speaker = row.and_then(|r| r.speaker.clone());
            let whole = Piece {
                tid: pane.tid,
                row: node.row,
                locator: node.locator.clone(),
                part: node.range.clone(),
                text: text.to_string(),
                tokens: est.count(text),
                split: false,
                observed,
                speaker,
                reported: super::attribution::clip(pane.reported, &node.range),
            };
            if whole.tokens <= budget {
                pieces.push(whole);
                continue;
            }
            // Rule 3. The cut is reported against the node, with the two numbers that explain
            // it: a reader asking why one verse arrived in four windows wants the budget it did
            // not fit in, not just the fact that it did not.
            let cut = split_at_sentences(&whole, budget, est, seg);
            diagnostics.push(
                Diagnostic::new(Code::W447)
                    .with_message(format!(
                        "`{}` of {} counts {} tokens against a window budget of {budget}; split \
                         into {} pieces at sentence boundaries",
                        whole.locator,
                        whole.tid.canonical(),
                        whole.tokens,
                        cut.len()
                    ))
                    .with_suggestion(
                        "a span still names bytes of the whole part; raise the budget with a \
                         larger context window if the node should be seen at once",
                    ),
            );
            pieces.extend(cut);
        }
    }

    (fill(pieces, budget), diagnostics)
}

/// A node, or one piece of one, before it is placed in a window.
#[derive(Debug, Clone)]
struct Piece {
    tid: Tid,
    row: u32,
    locator: Locator,
    part: Range<u64>,
    text: String,
    tokens: u32,
    split: bool,
    observed: Option<u64>,
    speaker: Option<String>,
    reported: Vec<Range<u64>>,
}

/// Cut one oversized node at sentence boundaries.
///
/// The pieces **partition** the node's range: a boundary is the start of the sentence that would
/// have overflowed, so no byte of the node belongs to two pieces or to none. A single sentence
/// over the budget is kept whole; see rule 3.
fn split_at_sentences(
    node: &Piece,
    budget: u32,
    est: TokenEstimator,
    seg: &Segmenter,
) -> Vec<Piece> {
    let sentences = seg.sentences(&node.text);
    // A node with no sentence boundary in it is one sentence, and rule 3 does not cut inside
    // one. It travels whole and over budget, which the `W447` beside it reports.
    if sentences.len() < 2 {
        let mut whole = node.clone();
        whole.split = true;
        return vec![whole];
    }

    let mut out = Vec::new();
    let mut start = 0usize;
    let mut tokens = 0u32;
    for s in &sentences {
        let next = est.count(&node.text[s.start..s.end]);
        // Cut *before* this sentence when it would overflow and something is already held: the
        // boundary is the sentence's start, so the whitespace between the two pieces goes with
        // the piece that follows and nothing is dropped.
        if tokens > 0 && tokens + next > budget {
            out.push(piece_of(node, start, s.start, est));
            start = s.start;
            tokens = 0;
        }
        tokens += next;
    }
    out.push(piece_of(node, start, node.text.len(), est));
    out
}

fn piece_of(node: &Piece, from: usize, to: usize, est: TokenEstimator) -> Piece {
    let text = node.text[from..to].to_string();
    Piece {
        tid: node.tid,
        row: node.row,
        locator: node.locator.clone(),
        part: (node.part.start + from as u64)..(node.part.start + to as u64),
        tokens: est.count(&text),
        text,
        split: true,
        observed: node.observed,
        speaker: node.speaker.clone(),
        reported: super::attribution::clip(
            &node.reported,
            &((node.part.start + from as u64)..(node.part.start + to as u64)),
        ),
    }
}

/// Place the pieces into windows: fill to the budget, carry one node over.
fn fill(pieces: Vec<Piece>, budget: u32) -> Vec<TextWindow> {
    let mut out: Vec<TextWindow> = Vec::new();
    let mut i = 0usize;
    // The piece the next window opens with, as an index into `pieces`.
    let mut carry: Option<usize> = None;

    while i < pieces.len() {
        let mut held: Vec<usize> = Vec::new();
        let mut tokens = 0u32;
        if let Some(c) = carry {
            held.push(c);
            tokens += pieces[c].tokens;
        }
        let owned_from = held.len();
        // At least one owned piece per window, whatever the budget says: a window that owned
        // nothing would make no progress, and the overlap it carried is already another
        // window's. That is rule I in the small.
        while i < pieces.len() {
            let next = pieces[i].tokens;
            if held.len() > owned_from && tokens + next > budget {
                break;
            }
            held.push(i);
            tokens += next;
            i += 1;
        }
        let owned_to = held.len();
        out.push(render(&pieces, &held, owned_from..owned_to));
        carry = held.last().copied();
    }

    out
}

/// Build the window's text and the nodes' placement in it.
fn render(pieces: &[Piece], held: &[usize], owned: Range<usize>) -> TextWindow {
    let mut text = String::new();
    let mut nodes = Vec::with_capacity(held.len());
    for (slot, &p) in held.iter().enumerate() {
        if slot > 0 {
            text.push_str(JOIN);
        }
        let start = text.len();
        text.push_str(&pieces[p].text);
        nodes.push(Placed {
            tid: pieces[p].tid,
            row: pieces[p].row,
            locator: pieces[p].locator.clone(),
            part: pieces[p].part.clone(),
            at: start..text.len(),
            split: pieces[p].split,
            observed: pieces[p].observed,
            speaker: pieces[p].speaker.clone(),
            reported: pieces[p].reported.clone(),
        });
    }
    TextWindow {
        nodes,
        overlap: (owned.start > 0).then_some(0),
        owned,
        text,
    }
}

fn slice<'t>(text: &'t str, range: &Range<u64>) -> Option<&'t str> {
    let start = usize::try_from(range.start).ok()?;
    let end = usize::try_from(range.end).ok()?;
    text.get(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_text::limits::Budget;
    use smysl_text::reading::Segment;

    fn level(s: &str) -> Level {
        Level::new(s).expect("a level")
    }

    fn loc(n: u64) -> Locator {
        Locator::canonical("v", &[n]).expect("a locator")
    }

    fn tid(text: &str) -> Tid {
        smysl_text::norm::Normalised::of(text).tid()
    }

    fn tree(rows: &[Segment], text: &str) -> Structure {
        let mut budget =
            Budget::new(smysl_text::Caps::default(), text.len() as u64).expect("a budget");
        Structure::build(rows, text.len() as u64, &mut budget).expect("a tree")
    }

    /// A part whose nodes are the lines of `text`, one node per line, at level `v`.
    fn pane(text: &str) -> (Tid, Vec<Segment>, Structure) {
        let mut rows = Vec::new();
        let mut at = 0u64;
        for (i, line) in text.lines().enumerate() {
            let len = line.len() as u64;
            rows.push(Segment::new(at, at + len, level("v"), loc(i as u64 + 1)));
            at += len + 1;
        }
        let structure = tree(&rows, text);
        (tid(text), rows, structure)
    }

    #[test]
    fn a_window_is_whole_nodes_and_the_owned_ranges_partition_the_text() {
        let text = "alpha one\nbeta two\ngamma three\ndelta four\n";
        let (t, rows, structure) = pane(text);
        let panes = [Pane::new(t, text, &structure, &rows)];
        // Each line counts 3 tokens under utf8-div4, so two nodes fit and the third does not.
        let (ws, diags) = windows(
            &panes,
            &level("v"),
            6,
            TokenEstimator::Utf8Div4,
            &Segmenter::neutral(),
        );
        assert!(diags.is_empty(), "{diags:?}");
        assert!(ws.len() > 1, "one window held everything");
        let owned: Vec<String> = ws
            .iter()
            .flat_map(|w| w.owned_nodes().iter().map(|n| n.locator.to_string()))
            .collect();
        let all: Vec<String> = (1..=4).map(|n| loc(n).to_string()).collect();
        assert_eq!(owned, all, "the owned nodes are every node, once, in order");
    }

    #[test]
    fn every_window_after_the_first_opens_with_the_previous_one_s_last_node() {
        let text = "alpha one\nbeta two\ngamma three\ndelta four\n";
        let (t, rows, structure) = pane(text);
        let panes = [Pane::new(t, text, &structure, &rows)];
        let (ws, _) = windows(
            &panes,
            &level("v"),
            6,
            TokenEstimator::Utf8Div4,
            &Segmenter::neutral(),
        );
        assert_eq!(ws[0].overlap, None);
        for pair in ws.windows(2) {
            let (before, after) = (&pair[0], &pair[1]);
            assert_eq!(after.overlap, Some(0));
            assert_eq!(
                after.nodes[0].part,
                before.owned_nodes().last().expect("an owned node").part,
            );
            assert!(
                after.owned.start == 1,
                "the carried node is not owned twice"
            );
        }
    }

    #[test]
    fn a_node_larger_than_the_window_is_split_at_sentence_boundaries_and_reported() {
        let text = "One sentence here. Another sentence follows. A third one closes it.\n";
        let (t, rows, structure) = pane(text);
        let panes = [Pane::new(t, text, &structure, &rows)];
        let (ws, diags) = windows(
            &panes,
            &level("v"),
            6,
            TokenEstimator::Utf8Div4,
            &Segmenter::new(&"en".parse().expect("a tag")),
        );
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, Code::W447);
        let pieces: Vec<&Placed> = ws.iter().flat_map(|w| w.owned_nodes()).collect();
        assert!(pieces.len() > 1, "the node was not split");
        assert!(pieces.iter().all(|p| p.split));
        // The pieces partition the node: contiguous, and covering it end to end.
        assert_eq!(pieces[0].part.start, 0);
        assert_eq!(
            pieces.last().expect("a piece").part.end,
            text.lines().next().expect("a line").len() as u64
        );
        for pair in pieces.windows(2) {
            assert_eq!(pair[0].part.end, pair[1].part.start);
        }
    }

    #[test]
    fn a_node_that_is_one_long_sentence_is_not_cut_inside_it() {
        let text = "a sentence with no boundary in it at all and quite a lot of words\n";
        let (t, rows, structure) = pane(text);
        let panes = [Pane::new(t, text, &structure, &rows)];
        let (ws, diags) = windows(
            &panes,
            &level("v"),
            4,
            TokenEstimator::Utf8Div4,
            &Segmenter::new(&"en".parse().expect("a tag")),
        );
        assert_eq!(diags.len(), 1, "the split is still reported");
        let pieces: Vec<&Placed> = ws.iter().flat_map(|w| w.owned_nodes()).collect();
        assert_eq!(pieces.len(), 1, "a sentence was cut in half");
        assert!(pieces[0].split);
    }

    #[test]
    fn a_window_offset_maps_back_to_the_part_it_came_from() {
        let text = "alpha one\nbeta two\n";
        let (t, rows, structure) = pane(text);
        let panes = [Pane::new(t, text, &structure, &rows)];
        let (ws, _) = windows(
            &panes,
            &level("v"),
            64,
            TokenEstimator::Utf8Div4,
            &Segmenter::neutral(),
        );
        let w = &ws[0];
        let at = w.text.find("beta").expect("the second node");
        let (part, offset) = w.map(at).expect("a mapping");
        assert_eq!(part, t);
        assert_eq!(
            &text[offset as usize..offset as usize + 4],
            "beta",
            "the offset is into the part, not the window"
        );
        assert_eq!(w.map(w.text.len() + 1), None);
    }

    #[test]
    fn a_window_may_cross_parts_and_a_node_never_does() {
        let (one, rows_one, structure_one) = pane("alpha one\n");
        let (two, rows_two, structure_two) = pane("beta two\n");
        let panes = [
            Pane::new(one, "alpha one\n", &structure_one, &rows_one),
            Pane::new(two, "beta two\n", &structure_two, &rows_two),
        ];
        let (ws, _) = windows(
            &panes,
            &level("v"),
            64,
            TokenEstimator::Utf8Div4,
            &Segmenter::neutral(),
        );
        assert_eq!(ws.len(), 1);
        let tids: Vec<Tid> = ws[0].owned_nodes().iter().map(|n| n.tid).collect();
        assert_eq!(tids, vec![one, two]);
    }

    #[test]
    fn a_chat_message_s_observed_instant_travels_with_its_node() {
        let text = "hello there\n";
        let (t, mut rows, _) = pane(text);
        rows[0].observed = Some(1_700_000_000_000);
        let structure = tree(&rows, text);
        let panes = [Pane::new(t, text, &structure, &rows)];
        let (ws, _) = windows(
            &panes,
            &level("v"),
            64,
            TokenEstimator::Utf8Div4,
            &Segmenter::neutral(),
        );
        assert_eq!(ws[0].owned_nodes()[0].observed, Some(1_700_000_000_000));
    }
    #[test]
    fn the_rendered_window_names_every_node_and_the_searched_text_does_not() {
        let text = "alpha one\nbeta two\n";
        let (t, rows, structure) = pane(text);
        let panes = [Pane::new(t, text, &structure, &rows)];
        let (ws, _) = windows(
            &panes,
            &level("v"),
            64,
            TokenEstimator::Utf8Div4,
            &Segmenter::neutral(),
        );
        let w = &ws[0];
        assert_eq!(w.rendered(), "[v.1] alpha one\n[v.2] beta two\n");
        assert!(
            !w.text.contains('['),
            "a quote would have to contain a locator to be present: {:?}",
            w.text
        );
    }

    #[test]
    fn the_default_window_level_is_the_leaves() {
        let text = "alpha one\nbeta two\n";
        let (_t, rows, structure) = pane(text);
        assert_eq!(leaf_level(&structure), Some(level("v")));
        assert!(!rows.is_empty());
    }

    #[test]
    fn one_odd_leaf_does_not_decide_how_an_expression_is_windowed() {
        // Three verses at `v` and one section with no verses in it: the section is a leaf too,
        // and it must not become the level every window is filled with.
        let text = "one\ntwo\nthree\na section with nothing marked inside it\n";
        let mut rows = Vec::new();
        let mut at = 0u64;
        for (i, line) in text.lines().enumerate() {
            let len = line.len() as u64;
            let lv = if i == 3 { level("section") } else { level("v") };
            rows.push(Segment::new(at, at + len, lv, loc(i as u64 + 1)));
            at += len + 1;
        }
        let structure = tree(&rows, text);
        assert_eq!(leaf_level(&structure), Some(level("v")));
    }
}
