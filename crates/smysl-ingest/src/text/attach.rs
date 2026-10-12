//! Span attachment: turning a quote into a byte range in a part (SMYSL-2.4 §3.4, §4.3.4).
//!
//! The model is asked for a **quote**, never for a span. A quote is checkable against the text
//! that was shown — that is what `SMY-E307` and `SMY-W308` have always been — and a span is what
//! the quote is worth once it has been found. So this module does the finding, and the unit comes
//! out carrying `source.span`, `source.manifest`, the reference with the node's locator, the
//! copy-set dates and the language, all written by one call to
//! [`smysl_text::provenance::stamp`] (D-1).
//!
//! # The search order, and why it is an order
//!
//! A quote may occur in more than one place. Picking the wrong one attaches a claim to the wrong
//! verse, which is worse than attaching it to none, so the scopes are tried narrowest first and
//! the **first scope with a match wins**:
//!
//! 1. the node the model named in `ingest:node`;
//! 2. the owned nodes of the part that node belongs to;
//! 3. the owned nodes of the other parts in the window;
//! 4. the overlap node, which this window does not own.
//!
//! Within a scope `Present` beats `Loose`, and among equals the **earliest** occurrence wins. The
//! hint therefore only ever *narrows*: a model that names a node it did not read loses nothing a
//! model that names none would have had, and `SMY-W419` reports the one case where the narrowest
//! scope is itself ambiguous — the same quote twice in the node named. Psalm 136 is the reason
//! that code exists: twenty-six verses ending "for his mercy endureth for ever".
//!
//! # A span never crosses a node boundary that is not a byte boundary
//!
//! §4.3.4 asks for the search to run "per contiguous same-part run". Contiguity is the operative
//! word, and this module checks it rather than assuming it: a run is a maximal sequence of
//! window nodes of one part whose part ranges are adjacent **with exactly the window's own
//! separator between them**. Inside such a run the map from window offset to part offset is one
//! affine shift, so a quote matched across two verses attaches correctly. Where the nodes are not
//! adjacent in the part — a window filled from a selection, or across parts — the run stops, and
//! a quote spanning the gap is not supported anywhere: it gets no span rather than a span over
//! bytes the part does not hold in that order.
//!
//! # What attachment changes, and therefore what it costs
//!
//! A unit's uid moves. `source` is inside the core and `ingest:quote` is inside the payload, so
//! attaching a span and removing the quote are both identity changes (OQ-41) — which is why they
//! happen here, once, before anything is staged, and never to a stored unit. The quote is removed
//! **only when a span was attached**: a unit with no span keeps it, because then the quote is the
//! only evidence a reviewer has of what was claimed.

use std::ops::Range;

use smysl_core::diag::{Code, Diagnostic};
use smysl_core::ids::LangTag;
use smysl_core::quote::{self, Normaliser, Support};
use smysl_core::types::{Manifest, Status, UnitCore, UnitCoreBuilder};
use smysl_core::{canonical_uid, Tid, Uid};
use smysl_text::provenance::{stamp, Drawn};
use smysl_text::Locator;

use super::attribution::{self, Attribution};
use super::window::{Placed, TextWindow, JOIN};

/// The payload key a model uses to say which node it read (§3.4).
///
/// Defined by the converter that writes it ([`crate::json_ast::NODE_KEY`]) and re-exported here,
/// where the attachment that reads it lives: the converter is in every build and this module is
/// behind a feature, so the key cannot belong to this side of the pair.
///
/// It is removed from the payload beside `ingest:quote` when a span is attached, because it is a
/// question's answer rather than a property of the claim — and it is inside the uid.
pub use crate::json_ast::NODE_KEY;

/// Which scope a quote was found in. Recorded because GE-T2 measures stability of attribution,
/// and "the hint was right" and "the hint was wrong and the part saved it" are different runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Scope {
    /// The node `ingest:node` named.
    Named,
    /// The owned nodes of the part holding the named node — or every owned node, when the answer
    /// named none.
    Owned,
    /// The owned nodes of the window's other parts.
    OtherPart,
    /// The overlap node, which belongs to the previous window.
    Overlap,
}

impl Scope {
    pub const fn id(self) -> &'static str {
        match self {
            Scope::Named => "named",
            Scope::Owned => "owned",
            Scope::OtherPart => "other-part",
            Scope::Overlap => "overlap",
        }
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.id())
    }
}

/// What happened to one unit's quote.
///
/// The journal's row (§4.3.4, step 4) is built from these: the verdict and the quote are what a
/// rerun compares, and `before`/`after` are what makes a staged batch traceable to the answer
/// that produced it, since attachment moves a uid.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Attachment {
    /// The uid the answer's unit had before attachment.
    pub before: Uid,
    /// The uid it has after. Equal to `before` only when nothing changed.
    pub after: Uid,
    pub quote: String,
    pub support: Support,
    pub scope: Option<Scope>,
    pub tid: Option<Tid>,
    pub span: Option<(u64, u64)>,
    /// The locator of the node the span fell in, as the reference carries it.
    pub locator: Option<String>,
    /// What the structure said about who holds the claim and in what mode (step 3b).
    pub attribution: Attribution,
}

/// One window's batch, after attachment.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Attached {
    pub units: Vec<UnitCore>,
    pub diagnostics: Vec<Diagnostic>,
    /// One per unit that attributed a quote, in batch order.
    pub attachments: Vec<Attachment>,
    /// The `x.text/entity` units the attributions produced — holders of what was attached.
    ///
    /// Beside the batch rather than inside it, so a caller can see what attachment *added* as
    /// against what a model said. Both go to the store; `Ingestor::ingest_text` (step 4) stages
    /// the two together.
    pub entities: Vec<UnitCore>,
    pub spanned: usize,
    /// Units that attributed a quote the window does not support (`SMY-W441`).
    pub spanless: usize,
    /// Units that attributed nothing. Not a defect: asking for a quote is a request, and a unit
    /// without one is a thinner record rather than a false one (`quote::verify`'s rule).
    pub unquoted: usize,
}

/// How to stamp what is attached.
#[derive(Debug, Clone, Copy)]
pub struct Opts<'a> {
    /// The manifest the extraction ran under. Its `mid` goes beside every span (rule 1) and its
    /// `published` is the copy rule's source.
    pub manifest: &'a Manifest,
    /// The comparison form the run's recipe records, so a quote is found under the form it will
    /// be checked under.
    pub normaliser: Normaliser,
    /// The language the units are written in — the caller's, never the manifest's (`SMY-W436`).
    pub lang: Option<&'a LangTag>,
}

/// Attach a span to every unit in one window's batch.
///
/// Infallible, and total: every unit comes back, attached or not. That is rule I — a quote the
/// window does not support is a weaker record, not a lost one.
pub fn attach(units: Vec<UnitCore>, w: &TextWindow, o: &Opts<'_>) -> Attached {
    let mut out = Attached::default();
    for unit in units {
        let before = canonical_uid(&unit);
        let Some(q) = quote::quote_of(&unit) else {
            out.unquoted += 1;
            out.units.push(unit);
            continue;
        };
        let hint = payload_str(&unit, NODE_KEY).and_then(|s| smysl_text::locator::parse(&s).ok());
        let found = search(w, &q, o.normaliser, hint.as_ref());

        match found {
            Some(hit) => {
                if hit.scope == Scope::Named && hit.ambiguous {
                    // The narrowest scope cannot narrow: the quote is in the named node more
                    // than once. The earliest occurrence is kept, because an arbitrary choice
                    // between occurrences would be unstable across reruns, and the warning is
                    // what says the choice was arbitrary.
                    out.diagnostics.push(
                        Diagnostic::new(Code::W419)
                            .with_message(format!(
                                "the quote occurs more than once in `{}`; the earliest occurrence \
                                 was taken",
                                hit.locator
                            ))
                            .with_suggestion(
                                "a refrain is the usual cause; a narrower node level gives the \
                                 model a place it can name unambiguously",
                            ),
                    );
                }
                let (attached, a, after) = apply(unit, &hit, o);
                // The holder entity is staged beside the claim. Identical by rule 7 across
                // windows, so the batch's own content addressing collapses the repeats and a
                // conversation of forty messages from one speaker stages one entity.
                if let Some(entity) = a.holder.clone() {
                    let uid = canonical_uid(&entity);
                    if !out.entities.iter().any(|e| canonical_uid(e) == uid) {
                        out.entities.push(entity);
                    }
                }
                out.attachments.push(Attachment {
                    before,
                    after,
                    quote: q,
                    support: hit.support,
                    scope: Some(hit.scope),
                    tid: Some(hit.tid),
                    span: Some((hit.part.start, hit.part.end)),
                    locator: Some(hit.locator.to_string()),
                    attribution: a,
                });
                out.spanned += 1;
                out.units.push(attached);
            }
            None => {
                // §3.4: no span, and the status capped to what the unit can still support —
                // `inferred` where it has grounds, `speculative` where it has none, because
                // `UnitCoreBuilder` refuses `inferred` without grounds and draft 3's plain
                // "capped at inferred" cannot be built.
                let (kept, after) = cap(unit);
                out.diagnostics.push(
                    Diagnostic::on(Code::W441, after)
                        .with_message(format!(
                            "the window does not support the attributed quote; staged with no \
                             span at `{}`",
                            kept.status.as_str()
                        ))
                        .with_suggestion(
                            "the quote is kept in the payload so the claim can be read; a span \
                             is what a later reader cannot reconstruct",
                        ),
                );
                out.attachments.push(Attachment {
                    before,
                    after,
                    quote: q,
                    support: Support::Absent,
                    scope: None,
                    tid: None,
                    span: None,
                    locator: None,
                    attribution: Attribution::default(),
                });
                out.spanless += 1;
                out.units.push(kept);
            }
        }
    }
    out
}

/// A quote found in a window.
#[derive(Debug, Clone)]
struct Hit {
    scope: Scope,
    support: Support,
    tid: Tid,
    part: Range<u64>,
    locator: Locator,
    observed: Option<u64>,
    /// The node the span starts in, for the attribution read off it (step 3b).
    node: Placed,
    /// The quote occurs more than once in the node named (`SMY-W419`); only ever set for
    /// [`Scope::Named`], because that is the only scope whose job was to be unambiguous.
    ambiguous: bool,
}

/// Try the scopes in order and return the first match.
fn search(w: &TextWindow, q: &str, n: Normaliser, hint: Option<&Locator>) -> Option<Hit> {
    let named: Option<usize> = hint.and_then(|l| w.nodes.iter().position(|p| &p.locator == l));

    // Scope 1: the named node alone.
    if let Some(i) = named {
        if let Some(mut hit) = in_runs(w, &[i], n, q, Scope::Named) {
            hit.ambiguous = occurs_again(&w.text[w.nodes[i].at.clone()], q, n);
            return Some(hit);
        }
    }

    let owned: Vec<usize> = (w.owned.start..w.owned.end).collect();
    // Scope 2: the part the named node belongs to — or, with no usable hint, every owned node.
    // "The owned range of the part holding it" has no referent without a hint, and the honest
    // reading of a hintless answer is that every owned node is equally likely.
    let (same, others): (Vec<usize>, Vec<usize>) = match named {
        Some(i) => {
            let tid = w.nodes[i].tid;
            owned.iter().partition(|&&j| w.nodes[j].tid == tid)
        }
        None => (owned.clone(), Vec::new()),
    };
    if let Some(hit) = in_runs(w, &same, n, q, Scope::Owned) {
        return Some(hit);
    }
    // Scope 3: the window's other parts, in window order.
    if let Some(hit) = in_runs(w, &others, n, q, Scope::OtherPart) {
        return Some(hit);
    }
    // Scope 4: the overlap, last. It belongs to the previous window, which already owned it, so
    // a claim anchored here is anchored in text this window is not responsible for — correct,
    // and the reason it is tried only when nothing owned matched.
    let overlap: Vec<usize> = w.overlap.into_iter().collect();
    in_runs(w, &overlap, n, q, Scope::Overlap)
}

/// Search one scope, run by run, and keep the best match: `Present` over `Loose`, earliest over
/// later.
fn in_runs(w: &TextWindow, scope: &[usize], n: Normaliser, q: &str, which: Scope) -> Option<Hit> {
    let mut best: Option<(usize, Hit)> = None;
    for run in runs(&w.nodes, scope) {
        let (support, at) = quote::support_span_with(n, q, &w.text[run.clone()]);
        let Some(at) = at else { continue };
        if support == Support::Absent {
            continue;
        }
        let from = run.start + at.start;
        let to = run.start + at.end;
        let Some(hit) = place(w, from, to, support, which) else {
            continue;
        };
        let better = match &best {
            None => true,
            Some((prev_from, prev)) => match (prev.support, support) {
                (Support::Present, Support::Present) | (Support::Loose, Support::Loose) => {
                    from < *prev_from
                }
                (Support::Loose, Support::Present) => true,
                _ => false,
            },
        };
        if better {
            best = Some((from, hit));
        }
    }
    best.map(|(_, hit)| hit)
}

/// Turn a window range into a part range, through the node it starts in.
fn place(w: &TextWindow, from: usize, to: usize, support: Support, which: Scope) -> Option<Hit> {
    let node = w.node_at(from)?;
    let start = node.into_part(from)?;
    // `to` is exclusive, so the byte before it is the last one matched; that is the offset that
    // has to be inside a node.
    let last = w.node_at(to.saturating_sub(1))?;
    let end = last.into_part(to.saturating_sub(1))? + 1;
    if end <= start {
        return None;
    }
    Some(Hit {
        scope: which,
        support,
        tid: node.tid,
        part: start..end,
        locator: node.locator.clone(),
        observed: node.observed,
        node: node.clone(),
        ambiguous: false,
    })
}

/// The contiguous same-part runs of a scope, as window byte ranges.
///
/// Two nodes continue a run when they are adjacent in the window, belong to one part, and their
/// part ranges are separated by exactly the window's separator — see the module's third section.
fn runs(nodes: &[Placed], scope: &[usize]) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut prev: Option<usize> = None;
    for &i in scope {
        let node = &nodes[i];
        let joined = match prev {
            Some(p) => {
                let before = &nodes[p];
                p + 1 == i
                    && before.tid == node.tid
                    && node.at.start == before.at.end + JOIN.len()
                    && node.part.start == before.part.end + JOIN.len() as u64
            }
            None => false,
        };
        match (joined, out.last_mut()) {
            (true, Some(run)) => run.end = node.at.end,
            _ => out.push(node.at.clone()),
        }
        prev = Some(i);
    }
    out
}

/// Whether the quote occurs a second time in `text`.
fn occurs_again(text: &str, q: &str, n: Normaliser) -> bool {
    let (support, at) = quote::support_span_with(n, q, text);
    if support == Support::Absent {
        return false;
    }
    let Some(at) = at else { return false };
    // From the end of the first occurrence, not from its start plus one: a second *overlapping*
    // match of the same bytes is the same occurrence read twice.
    let rest = &text[at.end.min(text.len())..];
    quote::support_span_with(n, q, rest).0 != Support::Absent
}

/// Write the copy set, drop the tool's own payload keys, and rebuild.
fn apply(unit: UnitCore, hit: &Hit, o: &Opts<'_>) -> (UnitCore, Attribution, Uid) {
    // §3.4, step 3b: who holds the claim and in what mode, from the node the span landed in.
    // Read before the payload is rewritten, because the three keys it owns are dropped whatever
    // the answer put in them — they are the tool's, and a model that fills one is answering a
    // question nobody asked. `SMY-W444` reports that drop and arrives with step 4.
    let a = attribution::attribution(&hit.part, &hit.node, o.manifest.alias.as_str(), o.lang);
    let mut add: Vec<(&str, String)> = Vec::new();
    if let Some(mode) = a.mode {
        add.push(("text:mode", mode.to_string()));
    }
    if let Some(speaker) = a.speaker.as_deref() {
        add.push(("text:speaker", speaker.to_string()));
    }
    if let Some(holder) = a.holder_uid() {
        add.push(("text:holder", holder.canonical()));
    }
    let mut drop: Vec<&str> = vec![quote::QUOTE_KEY, NODE_KEY];
    drop.extend_from_slice(attribution::OWNED);
    let payload = crate::payload::rewrite(unit.payload.as_deref(), &drop, &add);
    let mut b = rebuild(unit.clone());
    b.payload = payload;
    // `SMY-W406` wants a holder uid in `deps`, and `deps` is inside the uid — so it goes on
    // before the unit is hashed, not after. The entity itself is staged beside the claim, or the
    // dep would point at a unit nobody wrote.
    b.deps.extend(attribution::holder_deps(&a));
    // One call writes the reference, the span, the manifest, `published`/`observed` and the
    // language. Nothing here assembles a `SourceRef` by hand — that is the prohibition
    // `provenance` exists to keep (D-1).
    let b = stamp(
        b,
        &Drawn {
            tid: hit.tid,
            locator: Some(&hit.locator.to_string()),
            manifest: o.manifest,
            observed: hit.observed,
            span: Some((hit.part.start, hit.part.end)),
            lang: o.lang,
        },
    );
    match b.build() {
        Ok(core) => {
            let uid = canonical_uid(&core);
            (core, a, uid)
        }
        // Unreachable by construction: attachment adds a source, removes payload keys and never
        // raises a status, and every shape rule the builder enforces is about the fields it does
        // not touch. A library does not panic to prove a point, so the answer's own unit is kept.
        Err(_) => {
            let uid = canonical_uid(&unit);
            (unit, Attribution::default(), uid)
        }
    }
}

/// §3.4's cap for a quote the window does not support.
fn cap(unit: UnitCore) -> (UnitCore, Uid) {
    let ceiling = if unit.grounds.is_empty() {
        Status::Speculative
    } else {
        Status::Inferred
    };
    if unit.status <= ceiling {
        let uid = canonical_uid(&unit);
        return (unit, uid);
    }
    let mut b = rebuild(unit.clone());
    b.status = ceiling;
    // A capped unit may have been `cited` with a source the answer gave, and `cited` is the
    // status that requires one; lowering the status leaves the source where it was, which is
    // what `SMY-W309` and the source policy already decide.
    match b.build() {
        Ok(core) => {
            let uid = canonical_uid(&core);
            (core, uid)
        }
        Err(_) => {
            let uid = canonical_uid(&unit);
            (unit, uid)
        }
    }
}

/// A builder holding exactly what a core holds, so a core can be edited without being re-parsed.
fn rebuild(core: UnitCore) -> UnitCoreBuilder {
    let mut b = UnitCoreBuilder::new(core.schema, core.gist, core.status);
    b.body = core.body;
    b.detail = core.detail;
    b.deps = core.deps;
    b.grounds = core.grounds;
    b.source = core.source;
    b.payload = core.payload;
    b.lang = core.lang;
    b.extra = core.extra;
    b
}

/// A payload string value, by key.
fn payload_str(unit: &UnitCore, key: &str) -> Option<String> {
    let payload = unit.payload.as_deref()?;
    let object = smysl_core::surface::payload::payload_to_object(payload).ok()?;
    object
        .get(key)
        .and_then(|v| v.value.as_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::window::{windows, Pane};
    use smysl_core::ids::KernelType;
    use smysl_core::surface::hjson::{HObject, HValue, Spanned};
    use smysl_core::{Span, TokenEstimator};
    use smysl_text::limits::{Budget, Caps};
    use smysl_text::reading::{Level, Segment};
    use smysl_text::segment::Segmenter;
    use smysl_text::structure::Structure;
    use smysl_text::ManifestBuilder;

    const PSALM: &str =
        "O give thanks unto the LORD; for he is good: for his mercy endureth for ever.\n\
         O give thanks unto the God of gods: for his mercy endureth for ever.\n\
         O give thanks to the Lord of lords: for his mercy endureth for ever.\n";

    fn level() -> Level {
        Level::new("v").expect("a level")
    }

    fn loc(n: u64) -> Locator {
        Locator::canonical("v", &[n]).expect("a locator")
    }

    fn rows_of(text: &str) -> Vec<Segment> {
        let mut rows = Vec::new();
        let mut at = 0u64;
        for (i, line) in text.lines().enumerate() {
            let len = line.len() as u64;
            rows.push(Segment::new(at, at + len, level(), loc(i as u64 + 1)));
            at += len + 1;
        }
        rows
    }

    fn tree(rows: &[Segment], text: &str) -> Structure {
        let mut budget = Budget::new(Caps::default(), text.len() as u64).expect("a budget");
        Structure::build(rows, text.len() as u64, &mut budget).expect("a tree")
    }

    fn manifest() -> Manifest {
        ManifestBuilder::new(
            "psalms",
            LangTag::new("en").expect("a tag"),
            "txt/1",
            "public-domain",
            &smysl_text::Policy::default(),
        )
        .expect("a builder")
        .published("1769")
        .build()
        .expect("a manifest")
    }

    fn unit(gist: &str, quote: &str, node: Option<&str>) -> UnitCore {
        let span = Span::new(0, 0);
        let mut obj = HObject::default();
        obj.insert(
            Spanned::new(quote::QUOTE_KEY.to_string(), span),
            Spanned::new(HValue::Str(quote.to_string()), span),
        );
        if let Some(n) = node {
            obj.insert(
                Spanned::new(NODE_KEY.to_string(), span),
                Spanned::new(HValue::Str(n.to_string()), span),
            );
        }
        let mut b = UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative);
        b.payload = smysl_core::surface::payload::object_to_payload(&obj);
        b.build().expect("a unit")
    }

    fn window(text: &str) -> (TextWindow, Tid) {
        let rows = rows_of(text);
        let structure = tree(&rows, text);
        let tid = smysl_text::norm::Normalised::of(text).tid();
        let panes = [Pane::new(tid, text, &structure, &rows)];
        let (mut ws, _) = windows(
            &panes,
            &level(),
            4096,
            TokenEstimator::Utf8Div4,
            &Segmenter::neutral(),
        );
        assert_eq!(ws.len(), 1, "the fixture is meant to be one window");
        (ws.remove(0), tid)
    }

    fn opts<'a>(m: &'a Manifest) -> Opts<'a> {
        Opts {
            manifest: m,
            normaliser: Normaliser::V1,
            lang: None,
        }
    }

    #[test]
    fn a_quote_becomes_a_span_into_the_part_and_the_quote_leaves_the_payload() {
        let (w, tid) = window(PSALM);
        let m = manifest();
        let out = attach(
            vec![unit("God of gods is thanked", "the God of gods", None)],
            &w,
            &opts(&m),
        );
        assert_eq!(out.spanned, 1);
        assert_eq!(out.spanless, 0);
        let u = &out.units[0];
        let source = u.source.as_ref().expect("a source");
        let (start, end) = source.span().expect("a span");
        assert_eq!(&PSALM[start as usize..end as usize], "the God of gods");
        assert_eq!(source.manifest(), Some(&m.mid()));
        assert!(source.reference.starts_with(&tid.canonical()));
        assert!(source.reference.ends_with("#v.2"), "{}", source.reference);
        assert_eq!(quote::quote_of(u), None, "the quote stayed in the payload");
        assert_eq!(u.payload, None, "an emptied payload is no payload");
        // The copy rule: no `observed` on a verse, so the manifest's `published` is carried.
        assert_eq!(source.published.as_deref(), Some("1769"));
    }

    #[test]
    fn a_refrain_in_the_node_the_model_named_is_reported_and_the_earliest_occurrence_is_taken() {
        let (w, _) = window(PSALM);
        let m = manifest();
        let out = attach(
            vec![unit(
                "mercy endures",
                "for his mercy endureth for ever",
                Some("v.1"),
            )],
            &w,
            &opts(&m),
        );
        assert_eq!(out.spanned, 1);
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
        let first = out.units[0].source.as_ref().expect("a source");
        assert!(first.reference.ends_with("#v.1"));

        // The same refrain with no node named: the owned scope holds it three times, and the
        // earliest wins — the choice is stable, which is what `W419` is about when it is not.
        let out = attach(
            vec![unit(
                "mercy endures",
                "for his mercy endureth for ever",
                None,
            )],
            &w,
            &opts(&m),
        );
        assert_eq!(out.attachments[0].scope, Some(Scope::Owned));
        let (start, _) = out.units[0]
            .source
            .as_ref()
            .expect("a source")
            .span()
            .expect("a span");
        assert_eq!(
            start as usize,
            PSALM.find("for his mercy").expect("the refrain")
        );
    }

    #[test]
    fn a_node_named_twice_over_is_the_w419_case() {
        // One node, two occurrences of the quote inside it.
        let text = "the bell rang, and then the bell rang\nnothing else happened\n";
        let (w, _) = window(text);
        let m = manifest();
        let out = attach(
            vec![unit("the bell rang", "the bell rang", Some("v.1"))],
            &w,
            &opts(&m),
        );
        assert_eq!(out.diagnostics.len(), 1);
        assert_eq!(out.diagnostics[0].code, Code::W419);
        assert_eq!(out.attachments[0].scope, Some(Scope::Named));
        let (start, _) = out.units[0]
            .source
            .as_ref()
            .expect("a source")
            .span()
            .expect("a span");
        assert_eq!(start, 0, "the earliest occurrence");
    }

    #[test]
    fn a_hint_only_narrows_and_a_wrong_one_costs_nothing() {
        let (w, _) = window(PSALM);
        let m = manifest();
        // `v.1` does not contain this quote; the part's owned range does.
        let out = attach(
            vec![unit("lords", "the Lord of lords", Some("v.1"))],
            &w,
            &opts(&m),
        );
        assert_eq!(out.attachments[0].scope, Some(Scope::Owned));
        assert!(out.units[0]
            .source
            .as_ref()
            .expect("a source")
            .reference
            .ends_with("#v.3"));
        // A hint naming a node that is not in the window at all is the same case.
        let out = attach(
            vec![unit("lords", "the Lord of lords", Some("v.99"))],
            &w,
            &opts(&m),
        );
        assert_eq!(out.attachments[0].scope, Some(Scope::Owned));
    }

    #[test]
    fn an_absent_quote_is_staged_with_no_span_and_a_capped_status() {
        let (w, _) = window(PSALM);
        let m = manifest();
        let mut b = rebuild(unit(
            "a claim",
            "a sentence the psalm does not contain",
            None,
        ));
        b.status = Status::Cited;
        b.source = Some(smysl_core::SourceRef::new(
            smysl_core::SourceKind::Doc,
            "psalms",
        ));
        let cited = b.build().expect("a unit");
        let out = attach(vec![cited], &w, &opts(&m));
        assert_eq!(out.spanless, 1);
        assert_eq!(out.spanned, 0);
        let u = &out.units[0];
        assert_eq!(u.status, Status::Speculative, "no grounds, so speculative");
        assert!(u.source.as_ref().expect("a source").span().is_none());
        assert_eq!(
            quote::quote_of(u).as_deref(),
            Some("a sentence the psalm does not contain"),
            "a spanless unit keeps its quote for a reviewer",
        );
        assert_eq!(out.diagnostics.len(), 1);
        assert_eq!(out.diagnostics[0].code, Code::W441);
        assert_eq!(out.attachments[0].support, Support::Absent);
    }

    #[test]
    fn the_cap_is_inferred_where_the_unit_has_grounds() {
        let (w, _) = window(PSALM);
        let m = manifest();
        let ground = unit("a ground", "for he is good", None);
        let ground_uid = canonical_uid(&ground);
        let mut b = rebuild(unit("a claim", "not in the psalm at all", None));
        b.status = Status::Derived;
        b.grounds = [ground_uid].into_iter().collect();
        let derived = b.build().expect("a unit");
        let out = attach(vec![derived], &w, &opts(&m));
        assert_eq!(out.units[0].status, Status::Inferred);
        // And a status already at or under the cap is left exactly where it was: the uid of a
        // speculative unit with no span must not move for a rule that changes nothing.
        let speculative = unit("a claim", "not in the psalm at all", None);
        let before = canonical_uid(&speculative);
        let out = attach(vec![speculative], &w, &opts(&m));
        assert_eq!(canonical_uid(&out.units[0]), before);
    }

    #[test]
    fn a_unit_that_attributes_nothing_is_left_alone() {
        let (w, _) = window(PSALM);
        let m = manifest();
        let bare = UnitCoreBuilder::new(KernelType::Claim, "a bare claim", Status::Speculative)
            .build()
            .expect("a unit");
        let before = canonical_uid(&bare);
        let out = attach(vec![bare], &w, &opts(&m));
        assert_eq!(out.unquoted, 1);
        assert!(out.diagnostics.is_empty());
        assert!(out.attachments.is_empty());
        assert_eq!(canonical_uid(&out.units[0]), before);
    }

    #[test]
    fn the_overlap_node_is_the_last_scope_tried() {
        let rows = rows_of(PSALM);
        let structure = tree(&rows, PSALM);
        let tid = smysl_text::norm::Normalised::of(PSALM).tid();
        let panes = [Pane::new(tid, PSALM, &structure, &rows)];
        // 20 tokens fits one verse per window, so the second window owns verse 2 and carries
        // verse 1 as its overlap.
        let (ws, _) = windows(
            &panes,
            &level(),
            20,
            TokenEstimator::Utf8Div4,
            &Segmenter::neutral(),
        );
        let second = &ws[1];
        assert_eq!(second.overlap, Some(0));
        let m = manifest();
        let out = attach(
            vec![unit("he is good", "for he is good", None)],
            second,
            &opts(&m),
        );
        assert_eq!(out.attachments[0].scope, Some(Scope::Overlap));
        assert!(out.units[0]
            .source
            .as_ref()
            .expect("a source")
            .reference
            .ends_with("#v.1"));
    }

    #[test]
    fn only_the_tool_s_own_payload_keys_are_dropped() {
        let (w, _) = window(PSALM);
        let m = manifest();
        let span = Span::new(0, 0);
        let mut obj = HObject::default();
        obj.insert(
            Spanned::new(quote::QUOTE_KEY.to_string(), span),
            Spanned::new(HValue::Str("for he is good".to_string()), span),
        );
        obj.insert(
            Spanned::new(NODE_KEY.to_string(), span),
            Spanned::new(HValue::Str("v.1".to_string()), span),
        );
        obj.insert(
            Spanned::new("x.text/kind".to_string(), span),
            Spanned::new(HValue::Str("refrain".to_string()), span),
        );
        let mut b = UnitCoreBuilder::new(KernelType::Claim, "a claim", Status::Speculative);
        b.payload = smysl_core::surface::payload::object_to_payload(&obj);
        let out = attach(vec![b.build().expect("a unit")], &w, &opts(&m));
        let kept = smysl_core::surface::payload::payload_to_object(
            out.units[0].payload.as_deref().expect("a payload"),
        )
        .expect("a decodable payload");
        assert_eq!(kept.len(), 1);
        assert_eq!(
            kept.get("x.text/kind").and_then(|v| v.value.as_str()),
            Some("refrain"),
        );
    }

    #[test]
    fn a_quote_crossing_two_adjacent_verses_attaches_to_the_bytes_it_matched() {
        let (w, _) = window(PSALM);
        let m = manifest();
        // The newline between two verses is one byte in the part and one byte in the window, so
        // the run is contiguous and the span is exact.
        let quoted = "for ever.\nO give thanks unto the God of gods";
        let out = attach(vec![unit("two verses", quoted, None)], &w, &opts(&m));
        assert_eq!(out.spanned, 1);
        let (start, end) = out.units[0]
            .source
            .as_ref()
            .expect("a source")
            .span()
            .expect("a span");
        assert_eq!(&PSALM[start as usize..end as usize], quoted);
    }
    /// A chat-shaped pane: one message per line, with a speaker and an instant.
    fn chat_window(speaker: &str, text: &str, reported: &[Range<u64>]) -> (TextWindow, Tid) {
        let mut rows = rows_of(text);
        for r in rows.iter_mut() {
            r.speaker = Some(speaker.to_string());
            r.observed = Some(1_700_000_000_000);
        }
        let structure = tree(&rows, text);
        let tid = smysl_text::norm::Normalised::of(text).tid();
        let panes = [Pane::new(tid, text, &structure, &rows).with_reported(reported)];
        let (mut ws, _) = windows(
            &panes,
            &level(),
            4096,
            TokenEstimator::Utf8Div4,
            &Segmenter::neutral(),
        );
        (ws.remove(0), tid)
    }

    #[test]
    fn a_message_s_speaker_becomes_the_holder_the_mode_and_a_staged_entity() {
        let text = "the pool wait rose to 1.9 seconds\n";
        let (w, _) = chat_window("alice", text, &[]);
        let m = manifest();
        let out = attach(
            vec![unit("the pool wait rose", "the pool wait rose", None)],
            &w,
            &opts(&m),
        );
        let u = &out.units[0];
        let payload = smysl_core::surface::payload::payload_to_object(
            u.payload.as_deref().expect("a payload"),
        )
        .expect("a decodable payload");
        assert_eq!(
            payload.get("text:mode").and_then(|v| v.value.as_str()),
            Some(attribution::ASSERTED),
        );
        assert_eq!(
            payload.get("text:speaker").and_then(|v| v.value.as_str()),
            Some("alice"),
        );
        let entity = out.entities.first().expect("a staged holder");
        assert_eq!(entity.gist, "alice");
        let holder = canonical_uid(entity);
        assert_eq!(
            payload.get("text:holder").and_then(|v| v.value.as_str()),
            Some(holder.canonical().as_str()),
        );
        // `SMY-W406` wants the holder in `deps`, and `deps` is inside the uid.
        assert!(u.deps.contains(&holder));
        assert_eq!(
            out.attachments[0].attribution.mode,
            Some(attribution::ASSERTED)
        );
    }

    #[test]
    fn one_speaker_over_many_messages_stages_one_entity() {
        let text = "first thing\nsecond thing\nthird thing\n";
        let (w, _) = chat_window("alice", text, &[]);
        let m = manifest();
        let out = attach(
            vec![
                unit("one", "first thing", None),
                unit("two", "second thing", None),
                unit("three", "third thing", None),
            ],
            &w,
            &opts(&m),
        );
        assert_eq!(out.spanned, 3);
        assert_eq!(out.entities.len(), 1, "rule 7 gives one entity per speaker");
    }

    #[test]
    fn a_span_inside_a_reported_mark_is_reported_and_gets_no_holder() {
        let text = "he said \u{201c}there is no God\u{201d} and left\n";
        let marks = crate::text::attribution::quoted_spans(text);
        assert_eq!(marks.len(), 1, "the scan found the quotation");
        let (w, _) = chat_window("alice", text, &marks);
        let m = manifest();
        let out = attach(vec![unit("no God", "there is no God", None)], &w, &opts(&m));
        let payload = smysl_core::surface::payload::payload_to_object(
            out.units[0].payload.as_deref().expect("a payload"),
        )
        .expect("a decodable payload");
        assert_eq!(
            payload.get("text:mode").and_then(|v| v.value.as_str()),
            Some(attribution::REPORTED),
        );
        assert_eq!(
            payload.get("text:holder"),
            None,
            "a mark says someone else spoke, not who"
        );
        assert!(out.entities.is_empty());
        assert!(out.units[0].deps.is_empty());
    }

    #[test]
    fn a_verse_with_no_speaker_and_no_mark_gets_no_mode_at_all() {
        let (w, _) = window(PSALM);
        let m = manifest();
        let out = attach(
            vec![unit("he is good", "for he is good", None)],
            &w,
            &opts(&m),
        );
        assert_eq!(out.units[0].payload, None, "no mode, no speaker, no holder");
        assert_eq!(out.attachments[0].attribution.mode, None);
    }

    #[test]
    fn a_model_s_own_holder_and_mode_are_dropped_and_overwritten() {
        let text = "the pool wait rose to 1.9 seconds\n";
        let (w, _) = chat_window("alice", text, &[]);
        let m = manifest();
        let span = Span::new(0, 0);
        let mut obj = HObject::default();
        for (k, v) in [
            (quote::QUOTE_KEY, "the pool wait rose"),
            ("text:mode", "hypothetical"),
            ("text:holder", "b3:deadbeef"),
            ("text:speaker", "mallory"),
        ] {
            obj.insert(
                Spanned::new(k.to_string(), span),
                Spanned::new(HValue::Str(v.to_string()), span),
            );
        }
        let mut b = UnitCoreBuilder::new(KernelType::Claim, "a claim", Status::Speculative);
        b.payload = smysl_core::surface::payload::object_to_payload(&obj);
        let out = attach(vec![b.build().expect("a unit")], &w, &opts(&m));
        let payload = smysl_core::surface::payload::payload_to_object(
            out.units[0].payload.as_deref().expect("a payload"),
        )
        .expect("a decodable payload");
        assert_eq!(
            payload.get("text:mode").and_then(|v| v.value.as_str()),
            Some(attribution::ASSERTED),
            "the structure overrules the answer",
        );
        assert_eq!(
            payload.get("text:speaker").and_then(|v| v.value.as_str()),
            Some("alice"),
        );
        let holder = payload
            .get("text:holder")
            .and_then(|v| v.value.as_str())
            .expect("a holder");
        assert_ne!(holder, "b3:deadbeef");
        assert_eq!(payload.len(), 3, "and nothing else survives: {payload:?}");
    }
}
