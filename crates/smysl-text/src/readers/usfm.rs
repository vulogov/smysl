//! `usfm/1` — Unified Standard Format Markers, the way scripture is actually distributed.
//!
//! A USFM file is lines of backslash markers: `\id GEN`, `\c 1`, `\v 1 In the beginning…`,
//! with paragraph markers between them and character markers inside them. This reader keeps
//! the words and throws the typesetting away, which is what makes it **lossy** — a manifest
//! from this reader says so, because a measurement over a text that silently dropped half its
//! markup is a measurement of something nobody chose.
//!
//! # What it emits
//!
//! Three levels, nested by containment: `book` (`Gen`), `chapter` (`Gen.1`), `verse`
//! (`Gen.1.1`), in the canonical vocabulary of [`super::books`]. A verse bridge `\v 1-2`
//! becomes the range locator `Gen.1.1-Gen.1.2`, which is the one place the grammar's range
//! form is produced by a reader rather than typed by a person.
//!
//! # What it refuses
//!
//! A file whose structure cannot mean anything: a verse before any chapter, a chapter before
//! any book, a book code that is not a book, a verse number that is not a number. Those are
//! refusals rather than warnings because each one would otherwise produce a locator that
//! points somewhere real and wrong — and a wrong locator is worse than no text, since
//! everything downstream treats it as an address.
//!
//! Markup it does not know is a different matter: an unknown paragraph marker keeps its text
//! and sets `lossy`, and an unknown character marker keeps what is inside it. Refusing those
//! would mean refusing most real Bibles over one unusual marker, and the information that
//! something was dropped is already carried — once, honestly — by `lossy`.
//!
//! # `\wj`, the one character marker whose *position* is kept
//!
//! Every other character marker here is typesetting: `\add` is a supplied word, `\nd` is a
//! divine name in small capitals, and the reading keeps the words and forgets which. `\wj …
//! \wj*` is different in kind — it says the enclosed words are **reported speech**, which
//! §3.4 writes into `text:mode` and which decides whether a unit asserts a claim or records
//! that somebody made one. "The fool hath said in his heart, There is no God" is a claim the
//! psalm reports, and a corpus that lost the distinction holds a false record drawn from a
//! true text.
//!
//! So the words of Jesus are written as their own fragment and the range they land in becomes
//! a [`crate::reading::Mark`] on the verse row (key 9). USFM carries no attribution with the
//! marker — the mark has no `who`, and §3.4 wants none, since the mark says somebody else is
//! speaking and not who.
//!
//! Two limits worth stating. A `\wj` that is never closed marks to the end of its line, which
//! is what the stripper already does with an unclosed marker's body, and `lossy` is set either
//! way. And the mark is per line, so words of Jesus spanning two verses must carry `\wj` in
//! each — which is what the USFM style guide asks for and what the editions that use the
//! marker do.

use crate::limits::Budget;
use crate::locator::Locator;
use crate::norm::Normalised;
use crate::readers::books;
use crate::readers::build::Doc;
use crate::readers::{Input, Params, ReadOutput, Reader};
use crate::reading::Level;
use crate::LibError;

/// The level this reader cuts parts on: a part is whole chapters.
pub const LEVEL: &str = "chapter";

/// Character markers whose content is a note about the text rather than the text.
///
/// Dropped entirely, content and all. A footnote is not part of the verse — including it would
/// put an editor's cross-reference inside the span a quotation is matched against.
const NOTES: &[&str] = &["f", "fe", "x", "ef", "ex"];

/// The character marker that means *the words of Jesus* — reported speech, in §3.4's sense.
const WJ: &str = "wj";

pub struct Usfm;

impl Reader for Usfm {
    fn id(&self) -> &'static str {
        "usfm/1"
    }

    fn read(
        &self,
        input: &Input<'_>,
        _params: &Params,
        limits: &mut Budget,
    ) -> Result<ReadOutput, LibError> {
        limits.scan(input.len() as u64)?;
        let source = Normalised::new(input.bytes())?;
        Parse::new().run(source.as_str(), limits)
    }
}

/// A verse number as USFM writes it: one number, or a bridge over two.
struct VerseNumber {
    first: u64,
    last: u64,
}

struct Parse {
    doc: Doc,
    book: Option<&'static str>,
    /// Whether the book row has been opened. Tracked here rather than asked of the builder:
    /// it is this reader's question, and a builder accessor that only one reader used was an
    /// accessor that went unused in every build without that reader.
    book_open: bool,
    chapter: Option<u64>,
    title: Option<String>,
    lossy: bool,
}

impl Parse {
    fn new() -> Parse {
        Parse {
            doc: Doc::new("usfm/1"),
            book: None,
            book_open: false,
            chapter: None,
            title: None,
            lossy: false,
        }
    }

    fn run(mut self, source: &str, limits: &mut Budget) -> Result<ReadOutput, LibError> {
        let mut at = 0usize;
        for line in source.split('\n') {
            self.line(line, at, limits)?;
            at += line.len() + 1;
        }
        let (text, rows) = self.doc.finish()?;
        let mut out = ReadOutput::new(text, rows, Level::new(LEVEL).expect("a level"));
        out.lossy = self.lossy;
        out.title = self.title;
        Ok(out)
    }

    fn line(&mut self, line: &str, at: usize, limits: &mut Budget) -> Result<(), LibError> {
        let line = line.trim_end();
        let Some(rest) = line.strip_prefix('\\') else {
            // A continuation line: text belonging to the verse already open.
            self.text(line);
            return Ok(());
        };
        let (marker, value) = split_marker(rest);
        match marker {
            "id" => self.id(value, at),
            "c" => self.chapter(value, at, limits),
            "v" => self.verse(value, at, limits),
            // The book's running header and its major title, in the order USFM prefers them.
            // Taken as metadata, not as text: a title inside the text would be inside a verse
            // span, and no quotation of the book is a quotation of its header.
            "h" | "mt" | "mt1" => {
                if self.title.is_none() && !value.trim().is_empty() {
                    // A title is one string: nothing marks the words of Jesus in a running
                    // header, and a title is metadata rather than a range of the text.
                    let pieces = strip_inline(value, &mut self.lossy);
                    let joined: String = pieces.iter().map(|p| p.text.as_str()).collect();
                    self.title = Some(joined.trim().to_string());
                }
                Ok(())
            }
            // Markers that carry no text of their own.
            "ide" | "rem" | "b" | "nb" | "cl" | "cp" | "sts" | "usfm" => Ok(()),
            _ if marker.starts_with("toc") => Ok(()),
            // Paragraph-level markers: a break, and whatever text follows on the same line.
            _ => {
                if !is_known_paragraph(marker) {
                    // Recorded once per file rather than per marker: `lossy` is a fact about
                    // the text, not a count of markers.
                    self.lossy = true;
                }
                self.text(value);
                Ok(())
            }
        }
    }

    fn id(&mut self, value: &str, at: usize) -> Result<(), LibError> {
        let code = value.split_whitespace().next().unwrap_or("");
        match books::from_usfm(code) {
            Some(osis) => {
                self.book = Some(osis);
                Ok(())
            }
            None => Err(LibError::Unreadable {
                reader: "usfm/1".to_string(),
                at,
                what: format!("a book code in the canonical vocabulary, not `{code}`"),
            }),
        }
    }

    fn chapter(&mut self, value: &str, at: usize, limits: &mut Budget) -> Result<(), LibError> {
        let book = self.book.ok_or_else(|| LibError::Unreadable {
            reader: "usfm/1".to_string(),
            at,
            what: "`\\id` before the first chapter".to_string(),
        })?;
        let number = number(value.split_whitespace().next().unwrap_or(""), at)?;
        // The book row opens once, with the first chapter: a `\id` line alone describes no
        // text, and a row over nothing is not a node.
        if !self.book_open {
            self.book_open = true;
            self.doc
                .open("book", parse_canonical(book, at)?, at, limits)?;
        }
        self.chapter = Some(number);
        self.doc.open(
            "chapter",
            parse_canonical(&format!("{book}.{number}"), at)?,
            at,
            limits,
        )
    }

    fn verse(&mut self, value: &str, at: usize, limits: &mut Budget) -> Result<(), LibError> {
        let book = self.book.ok_or_else(|| LibError::Unreadable {
            reader: "usfm/1".to_string(),
            at,
            what: "`\\id` before the first verse".to_string(),
        })?;
        let chapter = self.chapter.ok_or_else(|| LibError::Unreadable {
            reader: "usfm/1".to_string(),
            at,
            what: "`\\c` before the first verse".to_string(),
        })?;
        let (digits, text) = split_marker(value);
        let verse = verse_number(digits, at)?;
        let locator = if verse.first == verse.last {
            parse_canonical(&format!("{book}.{chapter}.{}", verse.first), at)?
        } else {
            // A bridge: one span of text addressed by the range it covers. `Locator::range`
            // is what refuses a backwards one, so `\v 3-1` is a refusal rather than a span
            // addressed by a locator nothing can resolve.
            let first = parse_canonical(&format!("{book}.{chapter}.{}", verse.first), at)?;
            let last = parse_canonical(&format!("{book}.{chapter}.{}", verse.last), at)?;
            Locator::range(first, last).map_err(|e| LibError::Unreadable {
                reader: "usfm/1".to_string(),
                at,
                what: format!("a verse bridge that runs forwards ({e})"),
            })?
        };
        self.doc.open("verse", locator, at, limits)?;
        self.text(text);
        Ok(())
    }

    /// Text from one line or one marker: a separator is owed before it, because USFM's
    /// line breaks are where its words are separated.
    ///
    /// Written in pieces rather than as one string, because a `\wj` body has to be written on
    /// its own for the builder to say where it landed. A line with no `\wj` is one piece, and
    /// the text it produces is the same either way: [`super::build::Doc::write`] collapses
    /// whitespace runs and treats a fragment's edges as owed separators, so a sentence split
    /// at a marker boundary joins back up exactly as it did when the stripper concatenated it.
    fn text(&mut self, raw: &str) {
        let pieces = strip_inline(raw, &mut self.lossy);
        self.doc.gap();
        for piece in &pieces {
            let at = self.doc.write(&piece.text);
            if let (true, Some(at)) = (piece.reported, at) {
                self.doc.mark_speech(at, None);
            }
        }
    }
}

/// Split `marker rest` into its marker and what follows, tolerating `*` and digits.
///
/// The separator is skipped by its own UTF-8 length and not by one byte. `char::is_whitespace`
/// is true of U+0085 and U+00A0 as well as of a space, and those are two bytes each — so
/// `i + 1` landed inside the character and panicked, in a crate whose whole claim is that it
/// is a set of total functions over its arguments. Found by fuzzing, at five million
/// executions: a `\v` followed by U+0085.
fn split_marker(rest: &str) -> (&str, &str) {
    match rest.char_indices().find(|(_, c)| c.is_whitespace()) {
        Some((i, c)) => (&rest[..i], rest[i + c.len_utf8()..].trim_start()),
        None => (rest, ""),
    }
}

fn number(digits: &str, at: usize) -> Result<u64, LibError> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(LibError::Unreadable {
            reader: "usfm/1".to_string(),
            at,
            what: format!("a number, not `{digits}`"),
        });
    }
    digits.parse().map_err(|_| LibError::Unreadable {
        reader: "usfm/1".to_string(),
        at,
        what: "a number that fits".to_string(),
    })
}

fn verse_number(value: &str, at: usize) -> Result<VerseNumber, LibError> {
    match value.split_once('-') {
        Some((a, b)) => Ok(VerseNumber {
            first: number(a, at)?,
            last: number(b, at)?,
        }),
        None => {
            let n = number(value, at)?;
            Ok(VerseNumber { first: n, last: n })
        }
    }
}

fn parse_canonical(text: &str, at: usize) -> Result<Locator, LibError> {
    text.parse().map_err(|e| LibError::Unreadable {
        reader: "usfm/1".to_string(),
        at,
        what: format!("a locator ({e})"),
    })
}

/// Paragraph markers this reader knows. An unknown one is still read; it sets `lossy`.
fn is_known_paragraph(marker: &str) -> bool {
    let base = marker.trim_end_matches(|c: char| c.is_ascii_digit());
    matches!(
        base,
        "p" | "m"
            | "q"
            | "d"
            | "sp"
            | "li"
            | "pi"
            | "mi"
            | "pc"
            | "pr"
            | "s"
            | "ms"
            | "r"
            | "qa"
            | "qc"
            | "qr"
            | "qm"
            | "cd"
            | "lh"
            | "lf"
            | "lim"
            | "th"
            | "tc"
            | "tr"
    )
}

/// Remove character markers, keeping the words inside the ones that hold words.
///
/// `\add he\add*` keeps `he`; `\w word|strong="H1"\w*` keeps `word`; `\f + note\f*` keeps
/// nothing. An unclosed marker takes the rest of the line, which is what a typesetter's
/// mistake looks like and is still better than printing a backslash into a corpus.
fn strip_inline(raw: &str, lossy: &mut bool) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    let mut plain = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(i) = rest.find('\\') {
        plain.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let (marker, _) = split_marker(after);
        let name = marker.trim_end_matches('*');
        if marker.ends_with('*') {
            // A closing marker with no opening one. Drop it and keep reading.
            *lossy = true;
            rest = &after[marker.len()..];
            continue;
        }
        *lossy = true;
        let body_start = i + 1 + marker.len();
        let body = &rest[body_start..];
        let close = format!("\\{name}*");
        let (inner, tail) = match body.find(&close) {
            Some(j) => (&body[..j], &body[j + close.len()..]),
            None => (body, ""),
        };
        if name == WJ {
            // The one marker whose position survives. Its body may hold markers of its own —
            // `\wj ... \nd LORD\nd* ...\wj*` is ordinary — so it is stripped in turn, and
            // every piece that comes back is reported whether or not the inner marker kept it.
            Piece::flush(&mut out, &mut plain);
            for mut piece in strip_inline(inner, lossy) {
                piece.reported = true;
                out.push(piece);
            }
        } else if !NOTES.contains(&name) {
            // `\w word|strong="H1"\w*`: everything after the pipe is an attribute list, not
            // text. Keeping it would put markup inside a verse's span.
            let kept = inner.split('|').next().unwrap_or("");
            plain.push_str(kept.trim_start());
        }
        rest = tail;
    }
    plain.push_str(rest);
    Piece::flush(&mut out, &mut plain);
    out
}

/// A run of one line's text, and whether the source marked it as reported speech.
struct Piece {
    text: String,
    reported: bool,
}

impl Piece {
    /// Move whatever plain text has accumulated into a piece of its own.
    fn flush(out: &mut Vec<Piece>, plain: &mut String) {
        if !plain.is_empty() {
            out.push(Piece {
                text: std::mem::take(plain),
                reported: false,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::Caps;
    use crate::readers::read_with;
    use crate::reading::Segment;
    use crate::structure::Structure;

    pub(super) const GENESIS: &str = "\\id GEN The Book of Genesis\n\\h Genesis\n\\mt1 Genesis\n\\c 1\n\\p\n\\v 1 In the beginning God created the heaven and the earth.\n\\v 2 And the earth was without form.\n\\c 2\n\\p\n\\v 1 Thus the heavens were finished.\n";

    fn read(source: &str) -> ReadOutput {
        let mut budget = Budget::new(Caps::DEFAULT, source.len() as u64).expect("budget");
        read_with(
            &Usfm,
            &Input::new(source.as_bytes()),
            &Params::new(),
            &mut budget,
        )
        .expect("read")
    }

    fn refuse(source: &str) -> LibError {
        let mut budget = Budget::new(Caps::DEFAULT, source.len() as u64).expect("budget");
        read_with(
            &Usfm,
            &Input::new(source.as_bytes()),
            &Params::new(),
            &mut budget,
        )
        .expect_err("refused")
    }

    fn locators(out: &ReadOutput) -> Vec<String> {
        out.rows.iter().map(|r| r.locator.to_string()).collect()
    }

    #[test]
    fn a_book_two_chapters_and_three_verses_nest_by_containment() {
        let out = read(super::tests::GENESIS);
        assert_eq!(
            locators(&out),
            vec!["Gen", "Gen.1", "Gen.1.1", "Gen.1.2", "Gen.2", "Gen.2.1"]
        );
        let mut budget = Budget::new(Caps::DEFAULT, 1 << 16).expect("budget");
        let structure =
            Structure::build(&out.rows, out.text.len() as u64, &mut budget).expect("a valid table");
        assert_eq!(structure.roots().len(), 1, "one book");
        let book = structure.node(structure.roots()[0]).expect("the book");
        assert_eq!(structure.children(book).len(), 2, "two chapters");
    }

    /// Every verse's range holds that verse's words and no neighbour's.
    #[test]
    fn each_verse_range_is_its_own_text() {
        let out = read(super::tests::GENESIS);
        let text = out.text.as_str();
        let verse = |l: &str| {
            let row = out
                .rows
                .iter()
                .find(|r| r.locator.to_string() == l)
                .unwrap_or_else(|| panic!("{l}"));
            &text[row.start as usize..row.end as usize]
        };
        assert_eq!(
            verse("Gen.1.1"),
            "In the beginning God created the heaven and the earth."
        );
        assert_eq!(verse("Gen.1.2"), "And the earth was without form.");
        assert_eq!(verse("Gen.2.1"), "Thus the heavens were finished.");
    }

    /// No row's range holds the spacing that joins it to its neighbours.
    ///
    /// The separator between two verses belongs to neither of them. A row that took its start
    /// at its `\\v` marker instead of at its first word held one byte of the previous verse's
    /// spacing — for every verse after the first in a chapter, and for every chapter after the
    /// first in a book. Asserted as a property over every row, because the version of this
    /// test that compares one expected string passes while the second verse is wrong.
    #[test]
    fn no_row_range_holds_a_separator() {
        let out = read(super::tests::GENESIS);
        let text = out.text.as_str();
        for row in &out.rows {
            let slice = &text[row.start as usize..row.end as usize];
            assert!(
                !slice.starts_with(char::is_whitespace),
                "{} starts with spacing: {slice:?}",
                row.locator
            );
            assert!(
                !slice.ends_with(char::is_whitespace),
                "{} ends with spacing: {slice:?}",
                row.locator
            );
            assert!(!slice.is_empty(), "{} is empty", row.locator);
        }
    }

    /// The text holds exactly the separators the rows do not: one space per join.
    #[test]
    fn the_separators_are_outside_every_verse() {
        let out = read(super::tests::GENESIS);
        let verses: Vec<&Segment> = out
            .rows
            .iter()
            .filter(|r| r.level.as_str() == "verse")
            .collect();
        let inside: u64 = verses.iter().map(|r| r.len()).sum();
        let joins = (verses.len() - 1) as u64;
        assert_eq!(
            inside + joins,
            out.text.len() as u64,
            "every byte is in one verse or is a join between two"
        );
    }

    #[test]
    fn the_running_header_becomes_the_title_and_is_not_in_the_text() {
        let out = read(super::tests::GENESIS);
        assert_eq!(out.title.as_deref(), Some("Genesis"));
        assert!(
            !out.text.as_str().contains("Genesis"),
            "{}",
            out.text.as_str()
        );
    }

    /// A chapter's range is the hull of its verses, so a locator resolves to the whole chapter.
    #[test]
    fn a_chapter_resolves_to_all_of_its_verses() {
        let out = read(super::tests::GENESIS);
        let mut budget = Budget::new(Caps::DEFAULT, 1 << 16).expect("budget");
        let structure =
            Structure::build(&out.rows, out.text.len() as u64, &mut budget).expect("a valid table");
        let chapter: Locator = "Gen.1".parse().expect("a locator");
        let range = structure.resolve(&chapter).expect("resolved");
        let text = &out.text.as_str()[range.start as usize..range.end as usize];
        assert!(text.starts_with("In the beginning"), "{text}");
        assert!(text.ends_with("without form."), "{text}");
    }

    /// A verse continued on the next line is one run of prose with one space in the join.
    #[test]
    fn a_verse_across_two_lines_is_one_verse() {
        let out = read("\\id GEN\n\\c 1\n\\v 1 In the beginning\nGod created the heaven.\n");
        assert_eq!(locators(&out), vec!["Gen", "Gen.1", "Gen.1.1"]);
        let row = out.rows.last().expect("the verse");
        assert_eq!(
            &out.text.as_str()[row.start as usize..row.end as usize],
            "In the beginning God created the heaven."
        );
    }

    /// A bridge is addressed by the range it covers, which the grammar can already say.
    #[test]
    fn a_verse_bridge_gets_a_range_locator() {
        let out = read("\\id EXO\n\\c 20\n\\v 1-2 And God spake all these words.\n");
        assert_eq!(
            locators(&out),
            vec!["Exod", "Exod.20", "Exod.20.1-Exod.20.2"]
        );
    }

    #[test]
    fn a_backwards_bridge_is_refused() {
        let err = refuse("\\id EXO\n\\c 20\n\\v 3-1 Backwards.\n");
        match err {
            LibError::Unreadable { what, .. } => assert!(what.contains("forwards"), "{what}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn character_markers_keep_their_words_and_notes_keep_none() {
        let out = read("\\id GEN\n\\c 1\n\\v 1 The \\add LORD\\add* spake\\f + \\fr 1.1 \\ft a note\\f* now.\n");
        let row = out.rows.last().expect("the verse");
        assert_eq!(
            &out.text.as_str()[row.start as usize..row.end as usize],
            "The LORD spake now."
        );
        assert!(out.lossy, "markup was dropped");
    }

    #[test]
    fn a_word_marker_drops_its_attributes() {
        let out = read("\\id GEN\n\\c 1\n\\v 1 \\w In|strong=\"H1\"\\w* the beginning.\n");
        let row = out.rows.last().expect("the verse");
        assert_eq!(
            &out.text.as_str()[row.start as usize..row.end as usize],
            "In the beginning."
        );
    }

    /// A file with no markup at all is still read, and says it lost nothing.
    #[test]
    fn a_plain_file_is_not_reported_lossy() {
        let out = read("\\id GEN\n\\c 1\n\\v 1 In the beginning.\n");
        assert!(!out.lossy, "nothing was dropped");
    }

    /// An unknown paragraph marker keeps its text rather than refusing the file.
    #[test]
    fn an_unknown_paragraph_marker_keeps_its_text_and_is_recorded_as_a_loss() {
        let out = read("\\id GEN\n\\c 1\n\\zfuture Something new.\n\\v 1 In the beginning.\n");
        assert!(out.text.as_str().contains("Something new."));
        assert!(out.lossy);
    }

    #[test]
    fn structure_that_cannot_mean_anything_is_refused() {
        for (source, expected) in [
            ("\\c 1\n\\v 1 Text.\n", "`\\id` before the first chapter"),
            ("\\id GEN\n\\v 1 Text.\n", "`\\c` before the first verse"),
            ("\\id ZZZ\n\\c 1\n", "canonical vocabulary"),
            ("\\id GEN\n\\c one\n", "a number"),
            ("\\id GEN\n\\c 1\n\\v 1a Text.\n", "a number"),
        ] {
            match refuse(source) {
                LibError::Unreadable { what, .. } => {
                    assert!(what.contains(expected), "{source:?}: {what}")
                }
                other => panic!("{source:?}: {other:?}"),
            }
        }
    }

    /// Two verses with one number is refused here too, because the rule is one rule.
    ///
    /// The duplicate-address check lives in the shared builder, so a USFM file with `\\v 1`
    /// twice fails the same way a Zefania file with two `vnumber="1"` does. Asserted in both
    /// places because "the readers share the code" is the claim, and a claim about shared code
    /// is worth one test per sharer.
    #[test]
    fn a_repeated_verse_number_is_refused() {
        let err = refuse("\\id GEN\n\\c 1\n\\v 1 First.\n\\v 1 Again.\n");
        match err {
            LibError::Unreadable { what, .. } => {
                assert!(
                    what.contains("Gen.1.1") && what.contains("stated twice"),
                    "{what}"
                )
            }
            other => panic!("{other:?}"),
        }
    }

    /// A marker separated by multi-byte whitespace does not panic.
    ///
    /// `char::is_whitespace` is true of U+0085 (NEL) and U+00A0 (no-break space), which are
    /// two bytes each. Skipping the separator by one byte landed inside the character and
    /// panicked — a panic in a library, which is the one failure mode this crate's
    /// `forbid(unsafe_code)`, total-function style exists to make impossible. Every form of
    /// whitespace Unicode has is tested here, not only the one that was found.
    #[test]
    fn a_marker_followed_by_multibyte_whitespace_does_not_panic() {
        for ws in [
            "\u{85}", "\u{a0}", "\u{2028}", "\u{2029}", "\u{3000}", "\u{2003}",
        ] {
            let source = format!("\\id{ws}GEN\n\\c{ws}1\n\\v{ws}1 In the beginning.\n");
            let mut budget = Budget::new(Caps::DEFAULT, source.len() as u64).expect("budget");
            // Either outcome is fine; a panic is not. Several of these are not the ASCII
            // space USFM is written with, so a refusal is a perfectly good answer.
            let _ = read_with(
                &Usfm,
                &Input::new(source.as_bytes()),
                &Params::new(),
                &mut budget,
            );
        }
        // The one the fuzzer found, in the shape it found it: it must read, because U+0085 is
        // whitespace and the marker is still `v`.
        let out = read("\\id GEN\n\\c 1\n\\v\u{85}1 In the beginning.\n");
        assert_eq!(locators(&out), vec!["Gen", "Gen.1", "Gen.1.1"]);
    }

    /// The node cap is charged per node, book and chapter rows included.
    #[test]
    fn the_node_cap_counts_every_row() {
        let mut caps = Caps::DEFAULT;
        caps.nodes = 3;
        let source = "\\id GEN\n\\c 1\n\\v 1 a\n\\v 2 b\n\\v 3 c\n";
        let mut budget = Budget::new(caps, source.len() as u64).expect("budget");
        let err = read_with(
            &Usfm,
            &Input::new(source.as_bytes()),
            &Params::new(),
            &mut budget,
        )
        .expect_err("refused");
        match err {
            LibError::Limit { cap, .. } => assert_eq!(cap, "nodes"),
            other => panic!("{other:?}"),
        }
    }

    /// The text is the reader's output, not the input, and it is normalised.
    #[test]
    fn the_text_is_normalised_output_not_input_bytes() {
        let out = read("\\id GEN\r\n\\c 1\r\n\\v 1 In the beginning.\r\n");
        assert_eq!(out.text.as_str(), "In the beginning.");
        assert_eq!(out.text.tid(), out.text.tid(), "a tid over the output");
    }
}

#[cfg(test)]
mod speech_tests {
    use super::*;
    use crate::limits::Caps;
    use crate::readers::{read_with, Params};

    fn read(source: &str) -> ReadOutput {
        let mut budget = Budget::new(Caps::DEFAULT, source.len() as u64).expect("budget");
        read_with(
            &Usfm,
            &Input::new(source.as_bytes()),
            &Params::new(),
            &mut budget,
        )
        .expect("read")
    }

    /// What each mark covers, as text, so a test says what it means.
    fn marked(out: &ReadOutput) -> Vec<(String, Option<String>)> {
        let text = out.text.as_str();
        out.rows
            .iter()
            .flat_map(|row| {
                row.reported.iter().map(|m| {
                    (
                        text[m.start as usize..m.end as usize].to_string(),
                        m.who.clone(),
                    )
                })
            })
            .collect()
    }

    /// The words of Jesus are marked; the narration around them is not.
    #[test]
    fn wj_marks_its_words_and_carries_no_attribution() {
        let out = read(
            "\\id MAT\n\\c 4\n\\p\n\\v 4 But he answered and said, \
             \\wj It is written, Man shall not live by bread alone.\\wj*\n",
        );
        assert_eq!(
            marked(&out),
            vec![(
                "It is written, Man shall not live by bread alone.".to_string(),
                // USFM says that somebody else is speaking and not who, which is exactly what
                // §3.4 reads a mark for.
                None
            )]
        );
        assert!(out.text.as_str().starts_with("But he answered and said,"));
    }

    /// A marker inside the quotation does not break it into two quotations.
    #[test]
    fn a_marker_inside_wj_is_one_clause_and_not_three() {
        let out = read(
            "\\id MAT\n\\c 22\n\\p\n\\v 37 \\wj Thou shalt love the \\nd Lord\\nd* thy God.\\wj*\n",
        );
        assert_eq!(
            marked(&out),
            vec![("Thou shalt love the Lord thy God.".to_string(), None)]
        );
    }

    /// Two quotations with narration between them are two marks, which is the case the
    /// whitespace-only merge has to leave alone.
    #[test]
    fn narration_between_two_quotations_keeps_them_apart() {
        let out = read(
            "\\id MAT\n\\c 9\n\\p\n\\v 2 \\wj Be of good cheer.\\wj* And he said, \
             \\wj Thy sins be forgiven thee.\\wj*\n",
        );
        assert_eq!(
            marked(&out),
            vec![
                ("Be of good cheer.".to_string(), None),
                ("Thy sins be forgiven thee.".to_string(), None),
            ]
        );
    }

    /// An unclosed marker marks to the end of its line, and no further.
    #[test]
    fn an_unclosed_wj_stops_at_the_end_of_the_line() {
        let out = read(
            "\\id MAT\n\\c 5\n\\p\n\\v 3 \\wj Blessed are the poor in spirit\n\
             \\v 4 And the narrator speaks again.\n",
        );
        let found = marked(&out);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "Blessed are the poor in spirit");
        assert!(out.text.as_str().contains("And the narrator speaks again."));
    }

    /// A mark lands on the verse row, not on the chapter or the book.
    #[test]
    fn the_mark_is_on_the_row_whose_range_contains_it() {
        let out = read("\\id MAT\n\\c 4\n\\p\n\\v 4 \\wj It is written.\\wj*\n");
        let levels: Vec<&str> = out
            .rows
            .iter()
            .filter(|r| !r.reported.is_empty())
            .map(|r| r.level.as_str())
            .collect();
        assert_eq!(levels, vec!["verse"]);
    }

    /// Text with no `\wj` reads exactly as it did before the marker was understood — which the
    /// bibles fixtures assert over five editions, and this asserts in one line.
    #[test]
    fn a_file_with_no_wj_is_marked_nowhere() {
        let out = read(super::tests::GENESIS);
        assert!(out.rows.iter().all(|r| r.reported.is_empty()));
    }
}
