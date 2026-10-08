//! `md/1` — CommonMark, read for its prose rather than its typography.
//!
//! Markdown's structure is headings and leaf blocks, and that is what this reader emits: a
//! `section` per heading and a `paragraph` per block of prose inside it. Addressed by line,
//! `L12`, which is the locator form [`super::txt`] uses — for the same reason, that a line
//! number is the one address a person can find in their own editor.
//!
//! **The line is the line of the source document, not of the text.** Markdown's text is what
//! is left once the markup is gone, so the two do not have the same lines at all: a paragraph
//! written across four lines is one run of prose. A-5 settles which of the two is
//! authoritative — the byte range is, and the locator beside it is informative — so the range
//! addresses the text and the locator addresses the file somebody edits.
//!
//! # Sections are flat
//!
//! A heading opens a section and the next heading of any depth closes it, so an `##` does not
//! nest inside its `#`. Deliberate, and the reason is the locator grammar: one node has one
//! address, nesting sections by depth needs a level name per depth (`section1`…`section6`), and
//! then the level a part boundary falls on would depend on which heading depth a document
//! happens to start at. A flat reading has one boundary level for every document. Hierarchy,
//! if it is ever wanted, is `md/2` — the reader id is versioned for exactly this.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::limits::Budget;
use crate::locator::Locator;
use crate::norm::Normalised;
use crate::readers::build::Doc;
use crate::readers::{Input, Params, ReadOutput};
use crate::reading::Level;
use crate::LibError;

const ID: &str = "md/1";

/// The level this reader cuts parts on.
pub const LEVEL: &str = "section";

pub struct Md;

impl crate::readers::Reader for Md {
    fn id(&self) -> &'static str {
        ID
    }

    fn read(
        &self,
        input: &Input<'_>,
        _params: &Params,
        limits: &mut Budget,
    ) -> Result<ReadOutput, LibError> {
        limits.scan(input.len() as u64)?;
        let source = Normalised::new(input.bytes())?;
        read_markdown(source.as_str(), limits)
    }
}

/// The 1-based line of a byte offset into the source.
///
/// Counted rather than cached: the alternative is a table of line starts, and a binary search
/// over it, for a figure needed once per block.
fn line_of(source: &str, offset: usize) -> u64 {
    let upto = offset.min(source.len());
    source[..upto].bytes().filter(|b| *b == b'\n').count() as u64 + 1
}

fn read_markdown(source: &str, limits: &mut Budget) -> Result<ReadOutput, LibError> {
    // Tables and footnotes are recognised so that their content is read as prose rather than
    // appearing as literal pipes and markers in the text. Strikethrough likewise.
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;

    let mut doc = Doc::new(ID);
    let mut title: Option<String> = None;
    let mut lossy = false;
    // Depth of the heading currently open, so its text can become the title and so a nested
    // paragraph is not opened inside it.
    let mut in_heading = false;
    let mut heading_text = String::new();
    let mut open_leaf = false;

    // Collected through `catch_unwind` because the iterator that carries source offsets
    // panics on some inputs. `pulldown-cmark` 0.13.4's `OffsetIter` reaches
    // `tree.cur().unwrap()` on a `None` while ending a tight paragraph (`parse.rs:2199`); the
    // smallest input found is `"- [:]:`\n \t\t"`, and the *plain* iterator reads the same
    // input without complaint, so this is the offset API alone.
    //
    // A library in this crate may not panic on untrusted bytes — that is the whole claim of
    // `forbid(unsafe_code)` and of functions that are total over their arguments. Since the
    // defect is a dependency's and the offsets are what the locators are made of, the panic
    // is caught and turned into the refusal it should have been. Deterministic: the same
    // bytes panic in the same place and refuse the same way on every machine.
    //
    // The alternative was hand-rolling CommonMark's block structure, which is new code in the
    // one place untrusted bytes arrive — the argument OQ-37 settled the other way for JSON.
    // When the upstream fix lands, this wrapper and the `=0.13.4` pin come off together.
    let events = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Parser::new_ext(source, options)
            .into_offset_iter()
            .collect::<Vec<_>>()
    }))
    .map_err(|_| LibError::Unreadable {
        reader: ID.to_string(),
        at: 0,
        what: "markdown that pulldown-cmark 0.13.4 can locate (its offset iterator panicked \
               on this input; the text itself may be fine)"
            .to_string(),
    })?;

    for (event, range) in events {
        let line = line_of(source, range.start);
        match event {
            Event::Start(Tag::Heading { .. }) => {
                doc.open("section", locator(line, range.start)?, range.start, limits)?;
                in_heading = true;
                heading_text.clear();
                open_leaf = false;
            }
            Event::End(TagEnd::Heading(_)) => {
                in_heading = false;
                if title.is_none() {
                    let text = heading_text.trim();
                    if !text.is_empty() {
                        title = Some(text.to_string());
                    }
                }
            }
            // Leaf blocks: a block of prose with an address of its own. A heading is a leaf
            // block too, and deliberately not one of these — its text belongs to the section
            // it opened, and a node starting on the same line as its section would be a
            // second node with the same locator.
            Event::Start(Tag::Paragraph | Tag::CodeBlock(_) | Tag::Item | Tag::TableCell) => {
                if !in_heading {
                    doc.open(
                        "paragraph",
                        locator(line, range.start)?,
                        range.start,
                        limits,
                    )?;
                    open_leaf = true;
                }
            }
            Event::End(
                TagEnd::Paragraph | TagEnd::CodeBlock | TagEnd::Item | TagEnd::TableCell,
            ) => {
                if open_leaf {
                    doc.close_level("paragraph");
                    open_leaf = false;
                }
            }
            Event::Text(text) | Event::Code(text) => {
                if in_heading {
                    heading_text.push_str(&text);
                }
                doc.write(&text);
            }
            // A line break inside a block separates words and is not itself text.
            Event::SoftBreak | Event::HardBreak | Event::Rule => doc.gap(),
            // A link's target, an image, and raw HTML are content this reader does not carry.
            Event::Start(Tag::Link { .. } | Tag::Image { .. }) => lossy = true,
            Event::Html(_) | Event::InlineHtml(_) => {
                lossy = true;
                doc.gap();
            }
            Event::FootnoteReference(_) => lossy = true,
            _ => {}
        }
    }

    let (text, rows) = doc.finish()?;
    let mut out = ReadOutput::new(text, rows, Level::new(LEVEL).expect("a level"));
    out.title = title;
    out.lossy = lossy;
    Ok(out)
}

fn locator(line: u64, at: usize) -> Result<Locator, LibError> {
    Locator::line(line).map_err(|e| LibError::Unreadable {
        reader: ID.to_string(),
        at,
        what: format!("a line number ({e})"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::Caps;
    use crate::readers::{read_with, Reader};
    use crate::structure::Structure;

    const DOC: &str = "# The Title\n\nFirst paragraph, written\nacross two lines.\n\n## A Section\n\nSecond paragraph.\n\n- one item\n- another item\n";

    fn read(source: &str) -> ReadOutput {
        let mut budget = Budget::new(Caps::DEFAULT, source.len() as u64).expect("budget");
        read_with(
            &Md,
            &Input::new(source.as_bytes()),
            &Params::new(),
            &mut budget,
        )
        .expect("read")
    }

    fn at<'a>(out: &'a ReadOutput, id: &str) -> &'a str {
        let row = out
            .rows
            .iter()
            .find(|r| r.locator.to_string() == id)
            .unwrap_or_else(|| panic!("{id} is not in the table: {:?}", ids(out)));
        &out.text.as_str()[row.start as usize..row.end as usize]
    }

    fn ids(out: &ReadOutput) -> Vec<String> {
        out.rows.iter().map(|r| r.locator.to_string()).collect()
    }

    /// Every node is addressed by the source line it starts on, and no two share an address.
    #[test]
    fn nodes_are_addressed_by_their_source_line() {
        let out = read(DOC);
        assert_eq!(ids(&out), vec!["L1", "L3", "L6", "L8", "L10", "L11"]);
        let mut sorted = ids(&out);
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), out.rows.len(), "one address per node");
    }

    /// A paragraph written across two source lines is one run of prose.
    #[test]
    fn a_paragraph_across_lines_is_one_paragraph() {
        let out = read(DOC);
        assert_eq!(at(&out, "L3"), "First paragraph, written across two lines.");
    }

    /// A heading's text belongs to the section it opened, not to a node of its own.
    #[test]
    fn a_heading_opens_a_section_and_is_its_first_text() {
        let out = read(DOC);
        let section = at(&out, "L1");
        assert!(section.starts_with("The Title"), "{section}");
        assert_eq!(out.title.as_deref(), Some("The Title"));
    }

    /// The next heading closes the section before it, at any depth.
    #[test]
    fn sections_are_flat_and_end_at_the_next_heading() {
        let out = read(DOC);
        let mut budget = Budget::new(Caps::DEFAULT, 1 << 16).expect("budget");
        let structure =
            Structure::build(&out.rows, out.text.len() as u64, &mut budget).expect("a valid table");
        assert_eq!(
            structure.roots().len(),
            2,
            "two sections, neither inside the other"
        );
        let first = at(&out, "L1");
        assert!(
            !first.contains("A Section"),
            "the first section stops: {first}"
        );
        let second = at(&out, "L6");
        assert!(second.starts_with("A Section"), "{second}");
        assert!(second.contains("another item"), "{second}");
    }

    /// List items are leaf blocks, each with its own address.
    #[test]
    fn list_items_are_separate_nodes() {
        let out = read(DOC);
        assert_eq!(at(&out, "L10"), "one item");
        assert_eq!(at(&out, "L11"), "another item");
    }

    /// A link's text is prose; its target is not carried, and that is a loss.
    #[test]
    fn a_link_keeps_its_words_and_loses_its_target() {
        let out = read("Read [the manual](https://example.invalid/manual) first.\n");
        assert_eq!(at(&out, "L1"), "Read the manual first.");
        assert!(out.lossy, "the target was dropped");
        assert!(!out.text.as_str().contains("example.invalid"));
    }

    /// Plain prose loses nothing and says so.
    #[test]
    fn plain_prose_is_not_lossy() {
        let out = read("Just a sentence.\n\nAnd another.\n");
        assert!(!out.lossy);
        assert_eq!(ids(&out), vec!["L1", "L3"]);
    }

    /// Emphasis is typography: the words stay and nothing is reported lost.
    #[test]
    fn emphasis_is_not_a_loss() {
        let out = read("A *stressed* word and a **strong** one.\n");
        assert_eq!(at(&out, "L1"), "A stressed word and a strong one.");
        assert!(!out.lossy);
    }

    /// A code block is text, kept as written.
    #[test]
    fn a_code_block_is_kept_as_text() {
        let out = read("Intro.\n\n```\nlet x = 1;\n```\n");
        assert_eq!(at(&out, "L3"), "let x = 1;");
    }

    /// A document that begins with prose rather than a heading still reads.
    #[test]
    fn a_document_with_no_heading_has_paragraphs_at_the_root() {
        let out = read("No heading here.\n\nJust prose.\n");
        let mut budget = Budget::new(Caps::DEFAULT, 1 << 16).expect("budget");
        let structure =
            Structure::build(&out.rows, out.text.len() as u64, &mut budget).expect("a valid table");
        assert_eq!(structure.roots().len(), 2);
        assert_eq!(out.title, None, "no heading, no title");
    }

    /// An empty heading and an empty list item produce no rows, and the table still reads.
    ///
    /// The input the fuzzer minimised this to, in full: `f`, an `#` with no title, a `*` with
    /// no item, and one more character. Both empty blocks used to become zero-length rows, and
    /// one of them landed *before* the start of a row opened earlier — a table
    /// `Structure::build` refuses outright.
    #[test]
    fn empty_blocks_produce_no_rows_and_do_not_disorder_the_table() {
        let out = read("f\n#\n*\nx\n");
        let mut budget = Budget::new(Caps::DEFAULT, 1 << 16).expect("budget");
        let structure = Structure::build(&out.rows, out.text.len() as u64, &mut budget)
            .expect("the rows are a valid table");
        for row in &out.rows {
            assert!(!row.is_empty(), "{} is an empty row", row.locator);
            assert_eq!(structure.resolve(&row.locator), Some(row.range()));
        }
    }

    #[test]
    fn an_empty_document_is_an_empty_text() {
        let out = read("");
        assert!(out.rows.is_empty());
        assert!(out.text.is_empty());
    }

    #[test]
    fn the_node_cap_is_charged() {
        let mut caps = Caps::DEFAULT;
        caps.nodes = 2;
        let source = "a\n\nb\n\nc\n\nd\n";
        let mut budget = Budget::new(caps, source.len() as u64).expect("budget");
        let err = read_with(
            &Md,
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

    /// An input that panics the dependency becomes a refusal, not a crash.
    ///
    /// `pulldown-cmark` 0.13.4 panics inside `OffsetIter` on this input — found by fuzzing at
    /// 670,000 executions, and reproducible in three lines against the crate directly. What
    /// this test pins is that `smysl-text` turns it into a refusal naming the cause: a panic
    /// escaping a reader would take down whatever called it, and a reader is specified to
    /// refuse things it cannot read.
    #[test]
    fn an_input_that_panics_the_parser_is_a_refusal() {
        let source = "- [:]:`\n \t\t";
        let mut budget = Budget::new(Caps::DEFAULT, source.len() as u64).expect("budget");
        let err = read_with(
            &Md,
            &Input::new(source.as_bytes()),
            &Params::new(),
            &mut budget,
        )
        .expect_err("the dependency panics on this input, so the reader must refuse it");
        match err {
            LibError::Unreadable { what, .. } => {
                assert!(what.contains("offset iterator panicked"), "{what}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_reader_answers_to_its_id() {
        assert_eq!(Md.id(), "md/1");
        assert!(Md.params().is_empty());
    }
}
