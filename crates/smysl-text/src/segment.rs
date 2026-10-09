//! Sentence segmentation: `smysl/seg-uax29+abbr/1`.
//!
//! A reader gives a part its nodes — a verse, a message, a paragraph. Ingest needs something
//! finer and more uniform than that: the unit of a window, of a span, of a proposition is a
//! sentence, and no file format marks one. This module is where a sentence comes from, and the
//! id above is what a recipe records so that two ingests of the same text can be compared at
//! all (SMYSL-2.4 §3.2, §4.3's `Conditions::segmenter`).
//!
//! # What the id promises
//!
//! UAX #29 sentence boundaries, then **three suppressions and one reattachment**. The plan
//! named one of the four — the abbreviation lists — and measuring found the other three:
//!
//! | # | rule | what it fixes |
//! |---|---|---|
//! | 1 | **soft wrap**: a single `\n` that no sentence terminator precedes | UAX #29 breaks at every `\n` (rule SB4, `Sep`), so hard-wrapped prose came out one sentence per *line*. A blank line still ends a sentence. |
//! | 2 | **abbreviation**: the token before the boundary is in `data/abbr/<lang>.txt`, or is a single uppercase initial | `Mr. Smith`, `M. Dupont`, `Abb. 3.`, `ул. Ленина` — UAX #29 breaks on all four, because an uppercase letter or a digit follows a period. |
//! | 3 | **continuation**: the next piece begins with a lowercase letter | `He said "Stop!" and left.` — UAX #29 suppresses this after `.` (rule SB8) but not after `!` or `?`, and a closing quote between them puts it out of reach either way. |
//! | 4 | **closing marks** (a reattachment, not a suppression): a leading run of marks that cannot open a sentence belongs to the sentence before it | French spaces its quotation marks: `« … jamais eu. »` ends at the `»`, and UAX #29 ends it at the period, leaving `» Sa sœur posa…` as one piece. Suppressing that boundary would have joined two sentences; moving the `»` back joins nothing. |
//!
//! Those are the whole algorithm, and version `1` of the id means exactly them over exactly
//! the lists this build carries. [`LIST_HASH`] is what makes "exactly the lists" checkable:
//! edit a list and the pinned hash fails, which is the moment to decide whether the id becomes
//! `2`. Nothing has been written with it yet, so there is no corpus to be compatible with —
//! that is also why the three unplanned rules could be folded into `1` rather than named
//! separately. The RFC records it (§3.2, TX-P2 step 1).
//!
//! # What a list cannot settle
//!
//! Suppression 2 is data, and two Russian rows cost a boundary to buy another:
//!
//! * `г.` holds `в г. Тверь` together and loses the break in `умерла в 1911 г. Ты сама…`;
//! * `мин.` holds `6 ч. 15 мин.` together and loses the break after it.
//!
//! Both are the same shape as German `f.`, which measuring *removed* from `de.txt` — it cost
//! more boundaries than it saved. Whether `г.` pays for itself is a question about how often
//! Russian prose ends a sentence on a year against how often it writes an address, and
//! nothing in this repository can answer it: the 500-sentence gold set the phase exit asks
//! for is what decides, and until then the rows stay and the cost is in the measurement
//! (`tests/segment_f1.rs`, ru 0.96 against 1.00 for en).
//!
//! # What a segment is
//!
//! A byte range into the text handed in, ending at the sentence's **last text byte**. The
//! whitespace between two sentences belongs to neither, exactly as the whitespace between two
//! parts belongs to no part (`part::group`, TX-P1 step 6) — a cut lands in the gap, so an
//! offset is never in two segments at once. Whitespace-only input yields no segments, not one
//! empty one.
//!
//! # What is not here
//!
//! The segmenter does not name levels, mint locators or build rows: [`crate::reading::Segment`]
//! wants a level and a locator, and both are a reader's or ingest's vocabulary, not a
//! sentence's. It also does not detect a language — it is *given* one, because a short row is
//! where detection is least reliable (`lang::MIN_CHARS`, and the measurement behind it), and
//! the manifest already knows.

use std::collections::BTreeSet;
use std::ops::Range;

use smysl_core::ids::LangTag;
use unicode_segmentation::UnicodeSegmentation;

/// The segmenter id a recipe records, and what `smysl/seg-…` names in a journal.
///
/// Changing the algorithm **or** a list changes this. See the module docs for what version 1
/// is, and [`LIST_HASH`] for the half of it that lives in data files.
pub const ID: &str = "smysl/seg-uax29+abbr/1";

/// The languages with an abbreviation list, in the order [`LIST_HASH`] hashes them.
///
/// Tier 1 of SMYSL-2.4 §3.7: the five `lingua` is built for. A language outside this list is
/// not refused — it gets [`Segmenter::neutral`], which is UAX #29 plus suppressions 1 and 3,
/// and is what a `mul` manifest's unidentified stretches fall back to.
pub const LANGUAGES: &[&str] = &["de", "en", "es", "fr", "ru"];

/// The lists, compiled in. `data/abbr/<lang>.txt`, in [`LANGUAGES`] order.
///
/// `include_str!` rather than a runtime read: a segmenter that went to the filesystem for its
/// data would segment differently on two machines with the same version of this crate, and the
/// id would be a claim about the installation rather than about the build.
const LISTS: &[&str] = &[
    include_str!("../data/abbr/de.txt"),
    include_str!("../data/abbr/en.txt"),
    include_str!("../data/abbr/es.txt"),
    include_str!("../data/abbr/fr.txt"),
    include_str!("../data/abbr/ru.txt"),
];

/// BLAKE3 of the five lists concatenated in [`LANGUAGES`] order, hex.
///
/// Pinned so that editing a list is a test failure rather than a silent change of meaning for
/// every corpus taken with `smysl/seg-uax29+abbr/1`. The id is a promise about behaviour and
/// the lists *are* behaviour; a constant is the cheapest way to make forgetting that loud.
pub const LIST_HASH: &str = "147352a2be5268f77641da7a95ca96608c3e881ecc39272bddde5749738e0134";

/// The sentence terminators suppression 1 looks for.
///
/// The five tier-1 languages' own set, not Unicode's `STerm` — `…` is here because prose
/// uses it where a period would go, and `:` and `;` are not, because a clause that continues
/// after them is still the same sentence.
const TERMINATORS: &[char] = &['.', '!', '?', '…'];

/// Characters that may sit between a terminator and the gap, and still leave a sentence ended.
///
/// `He said "Stop!"` ends at the quote, not at the `!`, so suppression 1 has to look past the
/// closing marks to find out whether anything ended at all.
const CLOSERS: &[char] = &['"', '\'', '»', '’', '”', ')', ']', '}', '」'];

/// Closing marks that cannot *open* a sentence, which is a stronger claim than [`CLOSERS`].
///
/// French puts a space before `»`, so UAX #29 ends the sentence at the period and leaves
/// `»` to start the next piece — a piece that is one closing quotation mark and nothing else.
/// Suppression 3 gives it back, and these are the marks it is safe to give back: `"` and `'`
/// are absent because they are both halves of their own pair, and `He left.\n"Stop," she
/// said.` is two sentences that a rule over ASCII quotes would join into one.
const NEVER_OPENS: &[char] = &['»', '’', '”', ')', ']', '}', '」'];

/// One language's segmenter.
///
/// Cheap to build (a list is tens of rows) and cheap to clone, but it is built from data, so
/// `new` parses rather than looking anything up lazily — there is no global state here and no
/// first-call cost to make one run differ from the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segmenter {
    lang: Option<&'static str>,
    abbr: BTreeSet<&'static str>,
}

impl Segmenter {
    /// The segmenter for a language tag, by its primary subtag.
    ///
    /// `en-US`, `en` and `EN` get the English list; `pt` gets [`Segmenter::neutral`]. Matching
    /// on the primary subtag is the whole of the language negotiation this module does: an
    /// abbreviation list is about a language's typography, and `en-GB` writes `Mr.` the same
    /// way `en-US` does.
    pub fn new(lang: &LangTag) -> Segmenter {
        let primary = lang
            .as_str()
            .split('-')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        match LANGUAGES.iter().position(|l| *l == primary) {
            Some(i) => Segmenter {
                lang: Some(LANGUAGES[i]),
                abbr: parse_list(LISTS[i]),
            },
            None => Segmenter::neutral(),
        }
    }

    /// UAX #29 and the two language-independent suppressions, with no abbreviation list.
    ///
    /// What an unidentified stretch of a `mul` manifest gets, and what a language outside
    /// tier 1 gets. It is a weaker segmenter, not a broken one: suppression 2 is the only one
    /// that needs data.
    pub fn neutral() -> Segmenter {
        Segmenter {
            lang: None,
            abbr: BTreeSet::new(),
        }
    }

    /// The id, which is the same for every language — the lists are all inside version 1.
    pub fn id(&self) -> &'static str {
        ID
    }

    /// The language whose list is loaded, or `None` for [`Segmenter::neutral`].
    pub fn lang(&self) -> Option<&'static str> {
        self.lang
    }

    /// The abbreviations loaded, for a caller that wants to show them (`text show --segments`
    /// explains a suppression by naming the token it matched).
    pub fn abbreviations(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.abbr.iter().copied()
    }

    /// The sentences in `text`, as byte ranges that end at each sentence's last text byte.
    ///
    /// The ranges are in document order, do not overlap, and do not cover the whitespace
    /// between them — so they partition the text's *content* and nothing else. An empty or
    /// whitespace-only input gives an empty vector.
    pub fn sentences(&self, text: &str) -> Vec<Range<usize>> {
        let mut out: Vec<Range<usize>> = Vec::new();
        for (at, piece) in text.split_sentence_bound_indices() {
            // A UAX #29 piece carries its trailing whitespace, and the blank-line piece of a
            // paragraph break is whitespace entire. Trimming first is what makes a gap a gap.
            let Some(mut cur) = trim(text, at..at + piece.len()) else {
                continue;
            };
            // Rule 4 first, and it can leave a piece behind: `» Sa sœur posa…` is a closing
            // mark and then a whole sentence, so the mark moves and the sentence goes on to
            // be judged by the other three rules on its own.
            if let Some(prev) = out.last_mut() {
                if let Some(Why::Closer(after)) = self.suppression(text, prev, &cur) {
                    prev.end = after;
                    match trim(text, after..cur.end) {
                        Some(rest) => cur = rest,
                        None => continue,
                    }
                }
            }
            match out.last_mut() {
                Some(prev) if self.joins(text, prev, &cur) => prev.end = cur.end,
                _ => out.push(cur),
            }
        }
        out
    }

    /// Whether the boundary between two pieces is suppressed, and by which rule.
    ///
    /// Separate from [`Segmenter::sentences`] because the three rules are the algorithm and
    /// reading them in one place is the only way to see that they are independent: each looks
    /// at a different thing (the gap, the token before it, the word after it).
    pub fn suppression<'t>(
        &self,
        text: &'t str,
        prev: &Range<usize>,
        cur: &Range<usize>,
    ) -> Option<Why<'t>> {
        let gap = &text[prev.end..cur.start];

        // A blank line ends a sentence whatever precedes it. Checked first, so none of the
        // three can join across a paragraph break — an abbreviation at the end of a paragraph
        // is the end of a sentence, whatever the next paragraph starts with.
        if gap.matches('\n').count() >= 2 {
            return None;
        }

        // Checked before the three, because it is the one rule that does not ask whether the
        // boundary is real: a `»` belongs to the sentence it closes either way.
        let after_closers = closers_end(text, cur);
        if after_closers > cur.start {
            return Some(Why::Closer(after_closers));
        }

        if gap.contains('\n') && !ends_sentence(&text[prev.clone()]) {
            return Some(Why::SoftWrap);
        }
        if let Some(tok) = trailing_token(&text[prev.clone()]) {
            if self.is_abbreviation(tok) {
                return Some(Why::Abbreviation(tok));
            }
        }
        if text[cur.clone()]
            .chars()
            .next()
            .is_some_and(char::is_lowercase)
        {
            return Some(Why::Continuation);
        }
        None
    }

    /// Whether a token holds a boundary: in the list, or a single uppercase initial.
    ///
    /// The initials rule covers `J. R. R. Tolkien` and French `M. Dupont` without 26 rows per
    /// language saying the same thing. It is deliberately narrow — one uppercase letter and a
    /// period — because `I.` is the only English word it could swallow and `I.` is not one.
    pub fn is_abbreviation(&self, token: &str) -> bool {
        if self.abbr.contains(token) {
            return true;
        }
        let mut chars = token.chars();
        match (chars.next(), chars.next(), chars.next()) {
            (Some(c), Some('.'), None) => c.is_uppercase(),
            _ => false,
        }
    }

    fn joins(&self, text: &str, prev: &Range<usize>, cur: &Range<usize>) -> bool {
        self.suppression(text, prev, cur).is_some()
    }
}

/// Which suppression held a boundary, for a caller that has to explain itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why<'t> {
    /// A single `\n` with no terminator before it: one sentence over two source lines.
    SoftWrap,
    /// The token before the boundary, which is an abbreviation or an initial.
    Abbreviation(&'t str),
    /// The word after the boundary starts lowercase.
    Continuation,
    /// The piece after the boundary opens with marks that cannot open a sentence, and the
    /// offset past them. Only that run moves; the boundary itself stays, which is the whole
    /// difference between this and [`Why::Continuation`].
    Closer(usize),
}

/// BLAKE3 of the lists, hex — what [`LIST_HASH`] is pinned to.
///
/// Public because the check belongs to anyone shipping a corpus: a reader who wants to know
/// whether their build's `smysl/seg-uax29+abbr/1` is the one a manifest was taken with can
/// compare this rather than diff five data files.
pub fn list_hash() -> String {
    let mut h = smysl_core::hash::Rolling::new();
    for list in LISTS {
        h.update(list.as_bytes());
    }
    let mut s = String::with_capacity(64);
    for b in h.finish() {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('?'));
        s.push(char::from_digit((b & 0x0f) as u32, 16).unwrap_or('?'));
    }
    s
}

/// One list, as rows: `#` comments and blank lines dropped, each row trimmed.
fn parse_list(src: &str) -> BTreeSet<&str> {
    src.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

/// `range` with leading and trailing whitespace removed, or `None` if nothing is left.
fn trim(text: &str, range: Range<usize>) -> Option<Range<usize>> {
    let slice = &text[range.clone()];
    let trimmed = slice.trim();
    if trimmed.is_empty() {
        return None;
    }
    let start = range.start + (slice.len() - slice.trim_start().len());
    Some(start..start + trimmed.len())
}

/// The offset past the leading run of marks that cannot open a sentence, which is `cur.start`
/// when there is none.
///
/// Whitespace inside the run is taken too (`» )` is one run), because what is being moved is
/// the tail of a sentence and its internal spacing goes with it. A paragraph break stops it:
/// a mark that opens a paragraph closes nothing in the one before.
fn closers_end(text: &str, cur: &Range<usize>) -> usize {
    let mut at = cur.start;
    for (i, c) in text[cur.clone()].char_indices() {
        if NEVER_OPENS.contains(&c) {
            at = cur.start + i + c.len_utf8();
        } else if c == ' ' || c == '\u{a0}' || c == '\u{202f}' {
            continue;
        } else {
            break;
        }
    }
    at
}

/// Whether a piece ends with a sentence terminator, looking past closing quotes and brackets.
fn ends_sentence(piece: &str) -> bool {
    // Whitespace *between* the terminator and the closing mark, not only after it: French
    // writes `jamais eu. »`, and a single `trim_end` then `trim_end_matches` left the space
    // as the last character and reported that nothing had ended. The loop is what makes the
    // check about the sentence rather than about the typography of one language.
    let mut rest = piece;
    loop {
        let next = rest
            .trim_end()
            .trim_end_matches(|c| CLOSERS.contains(&c))
            .trim_end();
        if next.len() == rest.len() {
            break;
        }
        rest = next;
    }
    rest.chars()
        .next_back()
        .is_some_and(|c| TERMINATORS.contains(&c))
}

/// The dotted token a piece ends with: `Mr.`, `т.е.`, `U.S.`, `3.14.` — or `None`.
///
/// Scans back over letters, digits and periods, which is why `т.е.` comes out whole: a scan
/// that stopped at the first period would look up `е.` and find nothing. The token must end
/// in a period, because a boundary that no period produced is not one a list can hold.
fn trailing_token(piece: &str) -> Option<&str> {
    let piece = piece.trim_end();
    if !piece.ends_with('.') {
        return None;
    }
    let start = piece
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '.')
        .last()
        .map(|(i, _)| i)?;
    Some(&piece[start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(lang: &str) -> Segmenter {
        Segmenter::new(&LangTag::new(lang).unwrap())
    }

    fn cut(text: &str, lang: &str) -> Vec<String> {
        seg(lang)
            .sentences(text)
            .into_iter()
            .map(|r| text[r].to_string())
            .collect()
    }

    /// The cases draft 3 §5.3 named, which is where the lists came from.
    ///
    /// Two groups, and only the first needs the lists: UAX #29 already suppresses a boundary
    /// before a lowercase word that follows a period, so `т.е.`, `z.B.` and `p.m.` were never
    /// broken. Testing them anyway is the point — they are free today, and a UAX table bump
    /// that stopped making them free would otherwise change every corpus silently.
    #[test]
    fn the_abbreviations_draft_3_named_hold_a_boundary() {
        assert_eq!(
            cut("Mr. Smith went home. He slept.", "en"),
            ["Mr. Smith went home.", "He slept."]
        );
        assert_eq!(cut("Dr. Who arrived.", "en"), ["Dr. Who arrived."]);
        assert_eq!(
            cut("M. Dupont est arrivé. Il dort.", "fr"),
            ["M. Dupont est arrivé.", "Il dort."]
        );
        assert_eq!(cut("Это т.е. значит так.", "ru"), ["Это т.е. значит так."]);
        assert_eq!(cut("Das ist z.B. gut.", "de"), ["Das ist z.B. gut."]);
        assert_eq!(cut("At 5 p.m. we left.", "en"), ["At 5 p.m. we left."]);
    }

    /// The four languages whose own typography breaks UAX #29, one case each.
    ///
    /// German is the worst of them and is why the list exists at all: `Abb. 3.` and
    /// `Vgl. S. 14 f.` are ordinary prose in which an uppercase word or a digit follows a
    /// period, so UAX #29 breaks three times in two sentences.
    #[test]
    fn a_language_list_fixes_what_uax_29_breaks_in_that_language() {
        assert_eq!(
            cut("Dies ist Abb. 3. Siehe dort.", "de"),
            ["Dies ist Abb. 3.", "Siehe dort."]
        );
        assert_eq!(
            cut("Vgl. S. 14 f. Dort steht es.", "de"),
            ["Vgl. S. 14 f.", "Dort steht es."]
        );
        assert_eq!(
            cut("ул. Ленина, 5. Он там жил.", "ru"),
            ["ул. Ленина, 5.", "Он там жил."]
        );
        assert_eq!(
            cut("Sr. García llegó. Durmió.", "es"),
            ["Sr. García llegó.", "Durmió."]
        );
        assert_eq!(
            cut("Voir chap. 4. Puis arrêter.", "fr"),
            ["Voir chap. 4.", "Puis arrêter."]
        );
    }

    /// Initials are a rule, not rows: `J. R. R. Tolkien` is one sentence in every language.
    #[test]
    fn an_uppercase_initial_holds_without_being_listed() {
        assert_eq!(
            cut("J. R. R. Tolkien wrote it.", "en"),
            ["J. R. R. Tolkien wrote it."]
        );
        // And the rule is narrow: a lowercase token is not an initial, so a list that wants
        // one has to say so — `de.txt` carries `vgl.`, `en.txt` carries nothing like it.
        assert!(!seg("en").is_abbreviation("vgl."));
        assert!(seg("de").is_abbreviation("vgl."));
    }

    /// Suppression 1: a wrapped line is one sentence, a terminated line is not.
    #[test]
    fn a_soft_wrap_joins_but_a_terminator_before_one_does_not() {
        assert_eq!(
            cut("Wrapped line one\nand line two here.", "en"),
            ["Wrapped line one\nand line two here."]
        );
        assert_eq!(
            cut("Ends here.\nNew sentence starts.", "en"),
            ["Ends here.", "New sentence starts."]
        );
        // Past the closing quote, because that is where the sentence ended.
        assert_eq!(
            cut("He shouted \"Go!\"\nShe went.", "en"),
            ["He shouted \"Go!\"", "She went."]
        );
    }

    /// A blank line ends a sentence whatever precedes it — including an abbreviation.
    ///
    /// Checked first in [`Segmenter::suppression`], and this is the test that says why: a
    /// paragraph that ends `… Fig.` is still a paragraph, and joining it to the next one
    /// would make one sentence out of two pieces of prose that a writer separated by hand.
    #[test]
    fn a_blank_line_ends_a_sentence_whatever_precedes_it() {
        assert_eq!(
            cut("Para one.\n\nPara two.", "en"),
            ["Para one.", "Para two."]
        );
        assert_eq!(
            cut("See Fig.\n\nThen stop.", "en"),
            ["See Fig.", "Then stop."]
        );
        assert_eq!(
            cut("Wrapped with no stop\n\nand a new paragraph.", "en"),
            ["Wrapped with no stop", "and a new paragraph."]
        );
    }

    /// Suppression 3: UAX #29 starts a sentence at `and` because a quote sits before it.
    ///
    /// The rule UAX #29 has (SB8) looks past spaces and lowercase letters after a period —
    /// but `!` is `STerm`, not `ATerm`, so the rule does not apply, and the closing quote puts
    /// the lowercase word out of reach anyway.
    #[test]
    fn a_lowercase_word_continues_the_sentence_it_is_in() {
        assert_eq!(
            cut("He said \"Stop!\" and left. She did.", "en"),
            ["He said \"Stop!\" and left.", "She did."]
        );
        assert_eq!(
            cut("«Стой!» — сказал он. Она ушла.", "ru"),
            ["«Стой!» — сказал он.", "Она ушла."]
        );
    }

    /// The ranges partition the text's content: no overlap, in order, and the gaps are
    /// exactly the whitespace between sentences.
    ///
    /// The same property `part::group` has, for the same reason — an offset that fell in two
    /// segments would make a span ambiguous, and a byte that fell in none would make one
    /// unreachable.
    #[test]
    fn segments_end_at_their_last_text_byte_and_leave_the_gaps_to_nobody() {
        let text = "One. Two!\n\nThree?  Four.\n";
        let rows = seg("en").sentences(text);
        assert_eq!(rows.len(), 4);
        let mut at = 0;
        for r in &rows {
            assert!(r.start >= at, "{rows:?}");
            assert!(text[at..r.start].trim().is_empty(), "gap is whitespace");
            assert_eq!(text[r.clone()].trim(), &text[r.clone()], "trimmed");
            at = r.end;
        }
        assert!(text[at..].trim().is_empty());
    }

    #[test]
    fn whitespace_only_text_has_no_sentences() {
        assert!(seg("en").sentences("").is_empty());
        assert!(seg("en").sentences("   \n\n  \t ").is_empty());
    }

    /// A language with no list is not refused: it gets the two rules that need no data.
    #[test]
    fn a_language_outside_tier_1_gets_the_neutral_segmenter() {
        let s = seg("pt-BR");
        assert_eq!(s, Segmenter::neutral());
        assert_eq!(s.lang(), None);
        assert_eq!(s.abbreviations().count(), 0);
        assert_eq!(
            s.sentences("Wrapped line\ncontinues here. Done."),
            vec![0..28, 29..34]
        );
        // And a region subtag does not cost a list.
        assert_eq!(seg("en-GB").lang(), Some("en"));
        assert_eq!(seg("EN").lang(), Some("en"));
    }

    /// Which rule held a boundary, since a caller that explains a segmentation has to say.
    #[test]
    fn the_suppression_names_the_rule_and_the_token() {
        let s = seg("en");
        let text = "Mr. Smith";
        assert_eq!(
            s.suppression(text, &(0..3), &(4..9)),
            Some(Why::Abbreviation("Mr."))
        );
        let text = "one\ntwo";
        assert_eq!(s.suppression(text, &(0..3), &(4..7)), Some(Why::SoftWrap));
        let text = "One! and two";
        assert_eq!(
            s.suppression(text, &(0..4), &(5..12)),
            Some(Why::Continuation)
        );
        let text = "One.\n\nTwo";
        assert_eq!(s.suppression(text, &(0..4), &(6..9)), None);
    }

    /// The lists are data, so the id is a promise about data: pin the hash.
    ///
    /// Editing a list fails here, which is the moment to decide whether the id becomes `2`.
    /// The hash is over the files as shipped — comments included — because a comment that
    /// explained a row that is no longer there is also a change to what the list means.
    #[test]
    fn the_pinned_list_hash_is_the_hash_of_the_lists_this_build_carries() {
        assert_eq!(list_hash(), LIST_HASH, "the lists changed; so must `ID`");
        assert_eq!(LISTS.len(), LANGUAGES.len());
    }

    /// Every row can trigger. A row that cannot is worse than a missing one.
    ///
    /// Two ways a row goes dead: no period, so no boundary it could ever hold, and a
    /// duplicate, which says the list was edited twice without being read.
    #[test]
    fn every_list_row_ends_in_a_period_and_appears_once() {
        for (lang, src) in LANGUAGES.iter().zip(LISTS) {
            let rows: Vec<&str> = src
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .collect();
            assert!(!rows.is_empty(), "{lang}");
            for row in &rows {
                assert!(row.ends_with('.'), "{lang}: `{row}` cannot hold a boundary");
                assert!(!row.contains(char::is_whitespace), "{lang}: `{row}`");
            }
            let parsed = parse_list(src);
            assert_eq!(parsed.len(), rows.len(), "{lang}: a row is listed twice");
        }
    }

    /// The token scan, which is the part of suppression 2 that is not a lookup.
    #[test]
    fn the_trailing_token_is_the_whole_dotted_run() {
        assert_eq!(trailing_token("Hello Mr."), Some("Mr."));
        assert_eq!(trailing_token("Это т.е."), Some("т.е."));
        assert_eq!(trailing_token("in the U.S."), Some("U.S."));
        assert_eq!(trailing_token("(see fig."), Some("fig."));
        assert_eq!(trailing_token("it was 3.14."), Some("3.14."));
        assert_eq!(trailing_token("no period here"), None);
        assert_eq!(trailing_token("a dash -."), Some("."));
    }

    /// A terminator is found past the closers, and `:` is not a terminator.
    ///
    /// The French row is the one that was wrong: `jamais eu. »` has a space *between* the
    /// period and the quote, and a single pass of "trim spaces, then trim closers" left the
    /// space as the last character and concluded that nothing had ended — which turned the
    /// next sentence into a soft wrap and silently joined two sentences in every quoted
    /// French paragraph.
    #[test]
    fn a_sentence_ends_past_its_closing_marks_and_their_spacing() {
        assert!(ends_sentence("He said \"Stop!\""));
        assert!(ends_sentence("(Yes.)"));
        assert!(ends_sentence("Wait… "));
        assert!(ends_sentence("il n'y en a jamais eu. »"));
        assert!(ends_sentence("«Да!» )"));
        assert!(!ends_sentence("as follows:"));
        assert!(!ends_sentence("and then"));
        assert!(!ends_sentence("une saison »"));
    }

    /// Rule 4 moves the marks and leaves the boundary where it was.
    ///
    /// The distinction that matters: suppressing this boundary would have joined
    /// `« … saison. »` to `Il secoua la tête.` — two sentences in one row — because the piece
    /// UAX #29 hands over is the mark *and* the sentence after it.
    #[test]
    fn a_closing_mark_joins_the_sentence_it_closes_without_joining_the_next_one() {
        assert_eq!(
            cut("« Je suis désolé, dit-il. » Il partit.", "fr"),
            ["« Je suis désolé, dit-il. »", "Il partit."]
        );
        assert_eq!(
            cut("« Non. »\nElle partit.", "fr"),
            ["« Non. »", "Elle partit."]
        );
        // And a mark that opens a sentence is not one of these: `\"` is both halves of its
        // own pair, so the ASCII quote is deliberately outside `NEVER_OPENS`.
        assert_eq!(
            cut("He left.\n\"Stop,\" she said.", "en"),
            ["He left.", "\"Stop,\" she said."]
        );
    }

    /// The marks move only within a paragraph: a `»` that opens one closes nothing.
    #[test]
    fn a_closing_mark_after_a_blank_line_stays_where_it_is() {
        assert_eq!(cut("Fin.\n\n» Suite.", "fr"), ["Fin.", "» Suite."]);
    }
}
