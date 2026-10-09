//! The part of every structured reader that is not about a file format.
//!
//! Three of the six readers produce the same shape — nested levels, text inside the innermost
//! one — out of three unrelated syntaxes. What they share is not parsing but **where a row
//! begins**, and that turned out to be the one thing easy to get wrong: a row that takes its
//! start at the marker which opened it holds the spacing that joins it to its predecessor, for
//! every sibling after the first. It is one byte, it is invisible in a two-verse test, and
//! every span and every alignment downstream is measured against it.
//!
//! So the rule lives here once: a row **starts at its first text and ends at its last**, and
//! the separator between two rows belongs to neither.

use std::collections::BTreeSet;

use crate::limits::Budget;
use crate::locator::Locator;
use crate::norm::Normalised;
use crate::reading::{Level, Segment};
use crate::LibError;

/// A level that has been opened and not yet closed.
struct Open {
    level: String,
    row: usize,
    /// Whether any text has landed in this row yet. Until it has, the row's start is a
    /// placeholder and the next text to arrive claims it.
    started: bool,
}

/// A document under construction: the text a tid will be taken over, and the rows over it.
pub(crate) struct Doc {
    reader: &'static str,
    out: String,
    rows: Vec<Segment>,
    /// Open levels, outermost first. Rows are pushed in the order they open, which is document
    /// order, which is what the segment table requires.
    stack: Vec<Open>,
    /// Every address emitted, so that a source stating one twice is refused rather than
    /// turned into a table nothing can resolve. Found by fuzzing: a Zefania file with two
    /// verses numbered 1 produced two rows with one locator, and `Structure::build` refuses
    /// that table — so the reader was emitting a reading it could not read back.
    seen: BTreeSet<String>,
    pending_space: bool,
}

impl Doc {
    pub(crate) fn new(reader: &'static str) -> Doc {
        Doc {
            reader,
            out: String::new(),
            rows: Vec::new(),
            stack: Vec::new(),
            seen: BTreeSet::new(),
            pending_space: false,
        }
    }

    /// Open a level, closing any open level at or inside it first.
    ///
    /// Charges the node cap: a row is the unit the cap counts, and charging here means no
    /// reader can forget to.
    /// `at` is the byte offset in the *source* that opened this row, for a diagnostic to
    /// point at. It is not the row's range, which comes from the text.
    pub(crate) fn open(
        &mut self,
        level: &str,
        locator: Locator,
        at: usize,
        limits: &mut Budget,
    ) -> Result<(), LibError> {
        // One address, one node. A source that states the same address twice cannot be
        // addressed at all: `Structure::build` refuses the table, so a reader that passed it
        // on would produce a reading nothing downstream could open. Refused here, naming the
        // address and where the second one is, rather than merged — merging two verses into
        // one would be this crate deciding which of them the corpus holds.
        if !self.seen.insert(locator.to_string()) {
            return Err(LibError::Unreadable {
                reader: self.reader.to_string(),
                at,
                what: format!("each address once; `{locator}` is stated twice"),
            });
        }
        self.close_level(level);
        // Opening a row is a boundary: whatever follows is separated from what came before.
        // Declared here rather than left to each reader, because a reader that forgot would
        // glue two verses into one word and the text would still look plausible.
        self.gap();
        limits.node()?;
        // A placeholder range at the current end of the text. The first text to arrive claims
        // it; see `Open::started`.
        let here = self.out.len() as u64;
        let row = self.rows.len();
        self.rows.push(Segment::new(
            here,
            here,
            Level::new(level).ok_or_else(|| LibError::Unreadable {
                reader: self.reader.to_string(),
                at,
                what: format!("a level name, not `{level}`"),
            })?,
            locator,
        ));
        self.stack.push(Open {
            level: level.to_string(),
            row,
            started: false,
        });
        Ok(())
    }

    /// Close this level and everything open inside it.
    ///
    /// Nothing happens if the level is not open, which is what makes a reader's `\c` able to
    /// say "close any chapter" without first asking whether there is one.
    pub(crate) fn close_level(&mut self, level: &str) {
        if let Some(from) = self.stack.iter().position(|o| o.level == level) {
            self.close_from(from);
        }
    }

    /// Close every open level.
    pub(crate) fn close_all(&mut self) {
        self.close_from(0);
    }

    fn close_from(&mut self, from: usize) {
        while self.stack.len() > from {
            let open = self.stack.pop().expect("len > from");
            if !open.started {
                // A level the source declared and no text ever landed in. Its row goes.
                //
                // It used to stay, empty, at the position the parser had reached — "the
                // structure is what the source said". Fuzzing showed what that costs. A
                // markdown document with an empty heading and an empty list item produced an
                // empty row placed *before* the start of a row opened earlier, because an
                // unstarted row is placed when it closes while its unstarted parent is placed
                // when its first text arrives. The rows were then out of document order and
                // `Structure::build` refused the whole table.
                //
                // The deeper reason is the better argument: a node is a range of text, and a
                // zero-length range sits wherever the parser happened to be, which is not a
                // fact about the text at all. A-5 makes the range authoritative, so a row
                // claiming no text claims nothing. That the source declared an empty chapter
                // is a fact about the *source*, and the place to report it is a check over the
                // source — not a node in a corpus that no span can attach to and no locator
                // can usefully resolve.
                //
                // Removal is always from the tail: a started descendant would have started
                // this row too, so by the time an unstarted row closes, every row opened after
                // it has already been removed.
                debug_assert_eq!(
                    open.row,
                    self.rows.len() - 1,
                    "an unstarted row should be the last one"
                );
                if open.row + 1 == self.rows.len() {
                    self.rows.pop();
                }
            }
        }
    }

    /// A boundary where a separator is owed before the next text.
    ///
    /// A line ending, a paragraph marker, the gap between two elements. Owed and not written:
    /// the separator is only materialised when text actually follows, and then it lands
    /// *before* any row's start is taken, so it sits between ranges rather than at the front
    /// of one.
    pub(crate) fn gap(&mut self) {
        self.pending_space = true;
    }

    /// Append a fragment of character data to the innermost open level.
    ///
    /// Fragments, not lines, because that is how text arrives from an XML parser: `Spirit`,
    /// `&apos;`, `s word` are three events of one sentence. A method that trimmed each
    /// fragment and joined them with spaces would write `Spirit ' s word`, so whitespace is
    /// handled the only way that works for both kinds of source — runs of it collapse to one
    /// space, and whitespace at a fragment's edge becomes an *owed* separator rather than a
    /// written one.
    pub(crate) fn write(&mut self, fragment: &str) {
        if fragment.is_empty() {
            return;
        }
        if fragment.chars().all(char::is_whitespace) {
            // Indentation between two elements, or a blank line between two paragraphs. It
            // separates; it is not content.
            self.gap();
            return;
        }
        if fragment.starts_with(char::is_whitespace) {
            self.gap();
        }
        let trailing = fragment.ends_with(char::is_whitespace);

        if self.pending_space && !self.out.is_empty() {
            self.out.push(' ');
        }
        self.pending_space = false;

        let begin = self.out.len() as u64;
        // Internal runs collapse: pretty-printed XML puts a newline and two spaces inside a
        // verse, and no edition prints that.
        let mut spaced = false;
        for c in fragment.trim().chars() {
            if c.is_whitespace() {
                spaced = true;
                continue;
            }
            if spaced {
                self.out.push(' ');
                spaced = false;
            }
            self.out.push(c);
        }
        let end = self.out.len() as u64;
        if trailing {
            self.gap();
        }
        if end == begin {
            return;
        }
        for open in &mut self.stack {
            if !open.started {
                self.rows[open.row].start = begin;
                open.started = true;
            }
            self.rows[open.row].end = end;
        }
    }

    /// Put something on the innermost open row, after its text has landed.
    ///
    /// The metadata half of a chat reading — speaker, timestamp, the platform's own ids — is
    /// known when a message is read and belongs on the row that message produced. The
    /// alternative was to widen [`Doc::open`] with five more parameters that five of the eight
    /// readers would pass `None` for.
    ///
    /// Nothing happens when no level is open, which is the same shape as
    /// [`Doc::close_level`]: a caller that annotates before opening has written no text either,
    /// so there is no row for the annotation to be wrong about.
    ///
    /// Gated on the three chat readers, which are its only callers. `--all-features` cannot
    /// catch a `cfg` list that is missing a feature, which is why `make crate-features` builds
    /// each reader alone — and why this attribute is here rather than an `allow(dead_code)`:
    /// the question "does anything call this?" stays worth asking.
    #[cfg(any(
        feature = "reader-telegram",
        feature = "reader-slack",
        feature = "reader-whatsapp"
    ))]
    pub(crate) fn annotate(&mut self, f: impl FnOnce(&mut Segment)) {
        if let Some(open) = self.stack.last() {
            f(&mut self.rows[open.row]);
        }
    }

    /// The text and the rows, with the text checked for the one thing that could be wrong.
    ///
    /// The ranges were measured against the string this builder assembled. If that string is
    /// not itself normalised, the text a tid would be taken over is not the one the ranges
    /// describe — removing markup can leave a combining mark against a new base, and NFC is
    /// not closed under that. No real document does it; the refusal is here so that a reader
    /// can never emit ranges into text it did not produce.
    pub(crate) fn finish(mut self) -> Result<(Normalised, Vec<Segment>), LibError> {
        self.close_all();
        let text = Normalised::already(&self.out).ok_or_else(|| LibError::Unreadable {
            reader: self.reader.to_string(),
            at: 0,
            what: "text that is still normalised once its markup is removed".to_string(),
        })?;
        Ok((text, self.rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::Caps;
    use crate::structure::Structure;

    fn budget() -> Budget {
        Budget::new(Caps::DEFAULT, 1 << 16).expect("budget")
    }

    fn loc(text: &str) -> Locator {
        text.parse().unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    /// Two siblings, and the space between them is in neither.
    #[test]
    fn a_separator_belongs_to_no_row() {
        let mut doc = Doc::new("test");
        let mut b = budget();
        doc.open("verse", loc("Gen.1.1"), 0, &mut b).expect("open");
        doc.write("First.");
        doc.open("verse", loc("Gen.1.2"), 0, &mut b).expect("open");
        doc.write("Second.");
        let (text, rows) = doc.finish().expect("finished");
        assert_eq!(text.as_str(), "First. Second.");
        let slice = |r: &Segment| text.as_str()[r.start as usize..r.end as usize].to_string();
        assert_eq!(slice(&rows[0]), "First.");
        assert_eq!(slice(&rows[1]), "Second.");
    }

    /// A parent's range is the hull of its children, and the rows are a valid structure.
    #[test]
    fn nesting_gives_a_parent_the_hull_of_its_children() {
        let mut doc = Doc::new("test");
        let mut b = budget();
        doc.open("book", loc("Gen"), 0, &mut b).expect("open");
        doc.open("chapter", loc("Gen.1"), 0, &mut b).expect("open");
        doc.open("verse", loc("Gen.1.1"), 0, &mut b).expect("open");
        doc.write("One.");
        doc.open("verse", loc("Gen.1.2"), 0, &mut b).expect("open");
        doc.write("Two.");
        doc.open("chapter", loc("Gen.2"), 0, &mut b).expect("open");
        doc.open("verse", loc("Gen.2.1"), 0, &mut b).expect("open");
        doc.write("Three.");
        let (text, rows) = doc.finish().expect("finished");
        assert_eq!(text.as_str(), "One. Two. Three.");

        let structure =
            Structure::build(&rows, text.len() as u64, &mut budget()).expect("a valid table");
        let book = structure.node(structure.roots()[0]).expect("the book");
        assert_eq!(book.range, 0..text.len() as u64);
        assert_eq!(structure.children(book).len(), 2);
        let first = structure.resolve(&loc("Gen.1")).expect("Gen.1");
        assert_eq!(
            &text.as_str()[first.start as usize..first.end as usize],
            "One. Two."
        );
    }

    /// A level declared and left empty is not a row at all.
    #[test]
    fn an_empty_level_is_not_a_row() {
        let mut doc = Doc::new("test");
        let mut b = budget();
        doc.open("chapter", loc("Gen.1"), 0, &mut b).expect("open");
        doc.open("chapter", loc("Gen.2"), 0, &mut b).expect("open");
        doc.write("Only text.");
        let (text, rows) = doc.finish().expect("finished");
        assert_eq!(rows.len(), 1, "only the chapter with text is a node");
        assert_eq!(rows[0].locator.to_string(), "Gen.2");
        assert_eq!(rows[0].len(), text.len() as u64);
    }

    /// An empty row would otherwise be placed out of order, which is how this was found.
    ///
    /// The shape is a parent that has not started yet and an empty child inside it: the child
    /// is placed where the parser stands, the parent is placed later when its first text
    /// arrives, and the child then begins before its own parent. `Structure::build` refuses
    /// that table, so the reader would be emitting a reading nothing could open.
    #[test]
    fn an_empty_child_of_an_unstarted_parent_does_not_disorder_the_table() {
        let mut doc = Doc::new("test");
        let mut b = budget();
        doc.write("first");
        doc.open("section", loc("L2"), 0, &mut b).expect("open");
        doc.open("paragraph", loc("L3"), 0, &mut b).expect("open");
        doc.close_level("paragraph");
        doc.write("second");
        let (text, rows) = doc.finish().expect("finished");
        assert_eq!(rows.len(), 1, "the empty paragraph is gone");
        let structure =
            Structure::build(&rows, text.len() as u64, &mut budget()).expect("a valid table");
        assert_eq!(structure.len(), 1);
    }

    /// The node cap counts rows, whatever syntax produced them.
    #[test]
    fn opening_a_row_is_charged_against_the_node_cap() {
        let mut caps = Caps::DEFAULT;
        caps.nodes = 2;
        let mut b = Budget::new(caps, 1 << 10).expect("budget");
        let mut doc = Doc::new("test");
        doc.open("verse", loc("Gen.1.1"), 0, &mut b).expect("first");
        doc.open("verse", loc("Gen.1.2"), 0, &mut b)
            .expect("second");
        let err = doc
            .open("verse", loc("Gen.1.3"), 0, &mut b)
            .expect_err("the cap");
        match err {
            LibError::Limit { cap, limit, .. } => {
                assert_eq!(cap, "nodes");
                assert_eq!(limit, 2);
            }
            other => panic!("{other:?}"),
        }
    }
}
