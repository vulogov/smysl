//! Who holds a claim, and in what mode — from structure, never from the prompt (§3.4, §1.1 item 4).
//!
//! # Why this is not a prompt change
//!
//! S0 measured reported speech extracted **as asserted** in 11.1–24.5% of attribution-bearing
//! units, across every model and every coder pairing, with every cell above the 10% threshold
//! (T8). The arm that *added* a holder paragraph to the prompt did no better — 23.8% and 16.7%,
//! so T9 does not hold — and it introduced `SMY-E022` degradations into a local configuration
//! that had none (MS-9, MS-11). Asking more clearly is therefore not the fix, and the paragraph
//! stays out of the template. "The fool hath said in his heart, There is no God" is a claim the
//! psalm reports and does not make, and a model that writes `God does not exist` as an assertion
//! has produced a false record out of a true text.
//!
//! So `text:mode`, `text:holder` and `text:speaker` are written **here**, from the place the unit
//! was attached to: a message's own speaker, and the spans of a part that are marked as reported
//! speech. Whatever a model said about these three keys is dropped first — they are the tool's,
//! and a model that fills one is answering a question nobody asked.
//!
//! # `asserted` is not a default, so it is not written as one
//!
//! [`smysl_core::schemas::text::MODES`] has no default and §3.4 says it must not be given one.
//! A mode is written only where the structure decided it:
//!
//! - the span falls inside a reported-speech mark → `reported`, and **no holder**, because the
//!   mark says that someone else is speaking and not who;
//! - the node has a speaker and the span is not inside a mark → `asserted`, with that speaker as
//!   `text:speaker` and an `x.text/entity` for them as `text:holder`;
//! - neither → **nothing is written**. Not `asserted`: a unit with no structural evidence about
//!   its mode is a unit nobody has asked the right question about yet, and that is a different
//!   record from one asserted by a known holder.
//!
//! # Where the marks come from
//!
//! §3.4 says "reported-speech spans the reader marks", and from TX-P5 step 3b's reader work the
//! readers do: USFM's `\wj ... \wj*` and OSIS's `<q who="...">` become
//! [`smysl_text::reading::Mark`]s on the row that holds them (reading key 9), and
//! [`marks_of`] takes them off the rows for one part. The marks remain an **input** to this
//! module ([`crate::text::window::Pane::with_reported`]) rather than something it discovers,
//! because a mark is a fact about a *source* and this module only reads structure.
//!
//! [`quoted_spans`] — paired quotation marks — is still the supplier for everything else, and
//! still the supplier of last resort: a part whose reader marked nothing gets the scan, because
//! an empty mark list means "nothing is marked" and not "nothing is reported". What has changed
//! is which texts need it. A punctuating edition was always handled; an edition that marks the
//! words of Jesus is now handled by its own markup; and a 1769 Bible with neither is still not,
//! because Psalm 14:1 marks its reported clause with a comma and a capital letter and nothing
//! else. The probe-set gate in §6 TX-P5 step 3b measures what is left of that gap.
//!
//! A mark's `who` is **not** read here, and §3.4 is the reason: a mark establishes `reported`
//! and *no holder*, because the mark says somebody else is speaking and not that the edition's
//! attribution of it is a fact this corpus asserts. The field is carried in the reading for a
//! later phase.

use std::collections::BTreeSet;
use std::ops::Range;

use smysl_core::ids::LangTag;
use smysl_core::schemas::text;
use smysl_core::{canonical_uid, Uid, UnitCore};

use super::window::Placed;

/// `text:mode`, for the two values structure can establish.
pub const ASSERTED: &str = "asserted";
pub const REPORTED: &str = "reported";

/// The payload keys this module owns. A model's own values for them are dropped before these
/// are written (`SMY-W444`, TX-P5 step 4, reports the drop).
pub const OWNED: &[&str] = &["text:holder", "text:mode", "text:speaker"];

/// What the structure says about one attached unit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Attribution {
    /// `text:mode`, or `None` where the structure established none.
    pub mode: Option<&'static str>,
    /// `text:speaker` — the speaker of the node, as the reading recorded them. The sixth field
    /// of the D-1 copy set, and the one `provenance::stamp` leaves to this step because only an
    /// extractor knows which node a unit came from.
    pub speaker: Option<String>,
    /// The `x.text/entity` unit for the holder, when there is one. Returned rather than its uid
    /// alone because it has to be **staged**: `SMY-W406` wants the holder in `deps`, and a uid in
    /// `deps` pointing at a unit nobody wrote is a dangling reference.
    pub holder: Option<UnitCore>,
}

impl Attribution {
    /// The holder's uid, for `text:holder` and for `deps`.
    pub fn holder_uid(&self) -> Option<Uid> {
        self.holder.as_ref().map(canonical_uid)
    }

    /// Whether anything was established at all.
    pub fn is_empty(&self) -> bool {
        self.mode.is_none() && self.speaker.is_none() && self.holder.is_none()
    }
}

/// Read the structure around an attached span.
///
/// `span` is in the part's coordinates, as the attachment wrote it; `node` is the node it landed
/// in, carrying the reading's speaker and whatever reported-speech marks the pane supplied.
pub fn attribution(
    span: &Range<u64>,
    node: &Placed,
    alias: &str,
    lang: Option<&LangTag>,
) -> Attribution {
    // Containment, not intersection. A span that merely *touches* a reported clause is a claim
    // partly about the reporting and partly about what was reported, and this module does not
    // know which half the gist came from — so it establishes nothing rather than calling it
    // reported. The quote the model gave is still on the unit for a reviewer either way.
    let reported = node
        .reported
        .iter()
        .any(|m| m.start <= span.start && span.end <= m.end);

    if reported {
        return Attribution {
            mode: Some(REPORTED),
            speaker: node.speaker.clone(),
            holder: None,
        };
    }

    match node.speaker.as_deref() {
        Some(speaker) => Attribution {
            mode: Some(ASSERTED),
            speaker: Some(speaker.to_string()),
            // Rule 7's identity is name, expression and language, so two windows of one
            // conversation give one entity for one speaker and `merge` collapses them without
            // being asked. An entity that cannot be built — an alias the grammar refuses — is
            // no holder rather than a refusal: the claim is still a claim.
            holder: text::entity(speaker, alias, lang).ok(),
        },
        None => Attribution::default(),
    }
}

/// The reported-speech marks of one part: the reader's own and the punctuation scan's, joined.
///
/// **Both suppliers are positive evidence and neither claims completeness, so they are joined
/// rather than ranked.** This was decided the other way first — the reader's marks winning
/// outright, with the scan kept only for a part that had no marks at all — on the reasoning that
/// an edition marking its reported speech has already said which clauses are reported, so adding
/// the scan would re-admit the heuristic the marks replace.
///
/// Measuring it killed it. USFM's `\wj` does not mark reported speech: it marks the **words of
/// Jesus**, which is one holder out of however many a passage has. Over Mark 14-15 in the World
/// English Bible the reader finds 30 marks and the scan finds 54 — Pilate, the high priest, the
/// maid in the courtyard, the centurion and the crowd are all punctuated by that edition and
/// none of them is `\wj`. Ranking the suppliers therefore *lost* every speaker but one in an
/// edition that had marked and punctuated them both, and lost them precisely because the edition
/// had marked anything at all. The same holds for OSIS: `<q who>` is used per edition, and
/// nothing in either format asserts that every reported clause carries a marker.
///
/// What the join costs is the scan's false positives — a quoted title, a scare quote — in parts
/// that also carry marks. That cost was already accepted everywhere else: the scan was the sole
/// supplier for every unmarked part before the readers learned either marker, and a mode is only
/// ever written *because* some supplier says `reported`, never because none does.
///
/// Overlap is expected and harmless rather than merged: an edition that marks `\wj` *and* puts
/// quotation marks round the same clause yields two spans over nearly the same bytes, and
/// [`attribution`] asks whether a claim's span is contained in **any** mark. Exact duplicates
/// are dropped, because they say nothing twice.
///
/// The reader's marks come off the rows, where [`smysl_text::reading::Mark`] put them (key 9).
/// Both sets are part-local byte ranges, and the result is sorted into part order.
pub fn marks_of(rows: &[smysl_text::reading::Segment], text: &str) -> Vec<Range<u64>> {
    let mut out: Vec<Range<u64>> = rows
        .iter()
        .flat_map(|r| r.reported.iter().map(|m| m.start..m.end))
        .chain(quoted_spans(text))
        .collect();
    out.sort_by_key(|a| (a.start, a.end));
    out.dedup();
    out
}

/// The spans of `text` that lie inside paired quotation marks.
///
/// The supplier of last resort, and the only one that needs no reader support. Returns part-
/// coordinate ranges **of the quoted content**, excluding the marks themselves, so a span
/// containing the opening mark is not inside the quotation — which is right: a claim that quotes
/// the punctuation is a claim about the sentence that does the reporting.
///
/// Nesting is not tracked and unbalanced marks close nothing. An apostrophe is not a quotation
/// mark here for the reason the quote normaliser keeps apostrophes inside words: `don't` is one
/// word in every edition, and a scan that opened a quotation on it would mark the rest of a
/// paragraph as reported speech.
pub fn quoted_spans(text: &str) -> Vec<Range<u64>> {
    let mut out = Vec::new();
    let mut open: Option<(char, usize)> = None;
    for (at, c) in text.char_indices() {
        match open {
            Some((want, from)) if closes(want, c) => {
                if at > from {
                    out.push(from as u64..at as u64);
                }
                open = None;
            }
            Some(_) => {}
            None => {
                if let Some(after) = opens(c) {
                    open = Some((after, at + c.len_utf8()));
                }
            }
        }
    }
    out
}

/// The mark that closes a quotation this one opens.
fn opens(c: char) -> Option<char> {
    match c {
        '"' => Some('"'),
        '\u{201c}' => Some('\u{201d}'), // “ ”
        '\u{201e}' => Some('\u{201c}'), // „ “, German
        '\u{00ab}' => Some('\u{00bb}'), // « », French and Russian
        '\u{2018}' => Some('\u{2019}'), // ‘ ’, British inner
        _ => None,
    }
}

fn closes(want: char, c: char) -> bool {
    // `„…“` is also written `„…”` in some editions, and a straight mark closes a curly one in
    // text that has been through a tool. Accepting the near neighbour costs nothing: the only
    // thing a wrong close does is end a quotation early, where a missed close swallows a
    // paragraph.
    c == want || (want == '\u{201c}' && c == '\u{201d}') || (want == '\u{201d}' && c == '\u{201c}')
}

/// Clip the marks of a part to one node's range.
///
/// A mark that straddles a node boundary is kept as the part of it inside the node, because a
/// quotation that runs over two verses is reported speech in both.
pub(crate) fn clip(marks: &[Range<u64>], node: &Range<u64>) -> Vec<Range<u64>> {
    let mut out: Vec<Range<u64>> = marks
        .iter()
        .filter_map(|m| {
            let start = m.start.max(node.start);
            let end = m.end.min(node.end);
            (start < end).then_some(start..end)
        })
        .collect();
    out.dedup();
    out
}

/// The uids a batch's attributions add to `deps`.
///
/// Separate from the attribution because `deps` is inside the uid: a caller that added a holder
/// to a unit after hashing it would have two uids for one unit.
pub(crate) fn holder_deps(a: &Attribution) -> BTreeSet<Uid> {
    a.holder_uid().into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::Tid;
    use smysl_text::Locator;

    /// One reported-speech mark, as a list. A helper and not a literal, because a
    /// `vec![a..b]` reads to clippy as an attempt to build a vector *of* the range.
    fn marks(from: u64, to: u64) -> Vec<Range<u64>> {
        std::iter::once(from..to).collect()
    }

    fn node(speaker: Option<&str>, reported: Vec<Range<u64>>) -> Placed {
        Placed {
            tid: Tid::of(b"a part"),
            row: 0,
            locator: Locator::canonical("v", &[1]).expect("a locator"),
            part: 0..100,
            at: 0..100,
            split: false,
            observed: None,
            speaker: speaker.map(str::to_string),
            reported,
        }
    }

    #[test]
    fn a_span_inside_a_reported_mark_is_reported_and_has_no_holder() {
        let a = attribution(&(10..20), &node(Some("alice"), marks(5, 40)), "chat", None);
        assert_eq!(a.mode, Some(REPORTED));
        assert_eq!(a.speaker.as_deref(), Some("alice"));
        assert_eq!(a.holder, None, "a mark says someone else spoke, not who");
    }

    #[test]
    fn a_message_with_a_speaker_is_asserted_by_an_entity_for_them() {
        let a = attribution(&(10..20), &node(Some("alice"), Vec::new()), "chat", None);
        assert_eq!(a.mode, Some(ASSERTED));
        let holder = a.holder.as_ref().expect("an entity");
        assert_eq!(holder.gist, "alice");
        assert_eq!(holder.schema.as_str(), "x.text/entity");
        // Rule 7: the same speaker in the same expression is one entity, so two windows
        // produce one uid and `merge` collapses them.
        let again = attribution(&(50..60), &node(Some("alice"), Vec::new()), "chat", None);
        assert_eq!(a.holder_uid(), again.holder_uid());
    }

    #[test]
    fn nothing_is_written_where_the_structure_establishes_nothing() {
        let a = attribution(&(10..20), &node(None, Vec::new()), "kjv", None);
        assert!(a.is_empty(), "{a:?}");
        assert_eq!(
            a.mode, None,
            "`asserted` is not a default and must not be written as one",
        );
    }

    #[test]
    fn a_span_only_touching_a_reported_clause_establishes_nothing_about_mode() {
        // The span runs from the reporting clause into the reported one.
        let a = attribution(&(0..20), &node(None, marks(10, 40)), "kjv", None);
        assert_eq!(a.mode, None);
    }

    #[test]
    fn the_quotation_scan_finds_content_and_not_marks() {
        let text = "He said, \u{201c}there is no God\u{201d}, and left.";
        let spans = quoted_spans(text);
        assert_eq!(spans.len(), 1);
        let s = &spans[0];
        assert_eq!(
            &text[s.start as usize..s.end as usize],
            "there is no God",
            "the marks are not part of the quoted content",
        );
    }

    #[test]
    fn the_scan_handles_the_marks_four_traditions_use() {
        for (text, inner) in [
            ("a \"quoted\" word", "quoted"),
            ("a \u{201c}quoted\u{201d} word", "quoted"),
            ("ein \u{201e}zitiertes\u{201c} Wort", "zitiertes"),
            ("un \u{00ab}mot\u{00bb} cité", "mot"),
        ] {
            let spans = quoted_spans(text);
            assert_eq!(spans.len(), 1, "{text}");
            assert_eq!(&text[spans[0].start as usize..spans[0].end as usize], inner);
        }
    }

    #[test]
    fn an_apostrophe_does_not_open_a_quotation() {
        assert!(quoted_spans("don't open a quotation here").is_empty());
        assert!(quoted_spans("it\u{2019}s not one either").is_empty());
    }

    #[test]
    fn an_unbalanced_mark_closes_nothing() {
        assert!(quoted_spans("he said \u{201c}and never stopped").is_empty());
    }

    #[test]
    fn a_1769_bible_verse_has_no_marks_at_all_which_is_the_point_of_the_gate() {
        // The standing reported-speech case (S0 P5, Psalm 14:1 / Synodal 13:1). A punctuation
        // scan finds nothing here, and no reader in this build marks it either, which is what
        // the probe set measures rather than what this module can fix.
        let verse = "The fool hath said in his heart, There is no God.";
        assert!(quoted_spans(verse).is_empty());
    }

    #[test]
    fn marks_are_clipped_to_the_node_they_are_read_against() {
        let marks = [5..40, 60..70, 200..210];
        let got = clip(&marks, &(10..65));
        assert_eq!(got, vec![10..40, 60..65]);
    }

    /// A reader's marks and the scan's are joined, and a duplicate is said once.
    ///
    /// The decision this reverses, and why: `\wj` marks the words of one speaker, so an edition
    /// that marks and punctuates its dialogue reports more speech through its punctuation than
    /// through its markers, and letting the markers win lost the rest of the cast.
    #[test]
    fn a_readers_marks_are_joined_with_the_punctuation_scan_rather_than_replacing_it() {
        use smysl_text::reading::{Level, Mark, Segment};
        // Boaz is punctuated and not marked; the reapers' answer is marked and not punctuated.
        let text = "And Boaz said, \u{201c}The LORD be with you.\u{201d} And they answered \
                    him, The LORD bless thee.";
        let quoted = quoted_spans(text);
        assert_eq!(quoted.len(), 1, "the scan finds the punctuated clause");
        let answer = text.find("The LORD bless thee.").expect("the clause") as u64;

        let row = Segment::new(
            0,
            text.len() as u64,
            Level::new("verse").expect("a level"),
            "Ruth.2.4".parse().expect("a locator"),
        )
        .with_mark(Mark::new(answer, text.len() as u64));
        let marks = marks_of(&[row], text);
        assert_eq!(
            marks.len(),
            2,
            "one clause from each supplier, and neither one dropped: {marks:?}"
        );
        assert_eq!(marks[0], quoted[0]);
        assert_eq!(marks[1].start, answer);
    }

    /// The same bytes from both suppliers are one mark, because they say the same thing twice.
    #[test]
    fn a_clause_both_marked_and_punctuated_is_one_mark() {
        use smysl_text::reading::{Level, Mark, Segment};
        let text = "And Boaz said, \u{201c}The LORD be with you.\u{201d}";
        let quoted = quoted_spans(text);
        assert_eq!(quoted.len(), 1);
        let row = Segment::new(
            0,
            text.len() as u64,
            Level::new("verse").expect("a level"),
            "Ruth.2.4".parse().expect("a locator"),
        )
        .with_mark(Mark::new(quoted[0].start, quoted[0].end));
        assert_eq!(marks_of(&[row], text), quoted);
    }

    /// A part whose reader marked nothing still gets the scan.
    #[test]
    fn a_part_with_no_marks_falls_back_to_the_punctuation() {
        use smysl_text::reading::{Level, Segment};
        let text = "And Boaz said, \u{201c}The LORD be with you.\u{201d}";
        let row = Segment::new(
            0,
            text.len() as u64,
            Level::new("verse").expect("a level"),
            "Ruth.2.4".parse().expect("a locator"),
        );
        assert_eq!(marks_of(&[row], text), quoted_spans(text));
    }
}
