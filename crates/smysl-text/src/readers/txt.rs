//! `txt/1` — plain text, one node per line.
//!
//! The simplest reader there is, and the one that fixes the vocabulary the others are measured
//! against: a node per line at level `line`, addressed `L412`, which is the locator form
//! SMYSL-2.4 §3.2's table gives this reader.
//!
//! **Blank lines get no node.** They are still in the text — the bytes a tid is taken over are
//! the whole input — but a node whose range holds nothing but whitespace is a node no span can
//! be attached to and no window needs. Line *numbers* still count them, because `L412` has to
//! mean the 412th line of the file as an editor would show it; a reader that numbered only the
//! lines it kept would answer a different question from the one anybody asks.

use crate::limits::Budget;
use crate::locator::Locator;
use crate::norm::Normalised;
use crate::readers::{Input, Params, ReadOutput, Reader};
use crate::reading::{Level, Segment};
use crate::LibError;

/// The level this reader cuts parts on.
pub const LEVEL: &str = "line";

pub struct Txt;

impl Reader for Txt {
    fn id(&self) -> &'static str {
        "txt/1"
    }

    fn read(
        &self,
        input: &Input<'_>,
        _params: &Params,
        limits: &mut Budget,
    ) -> Result<ReadOutput, LibError> {
        // Charged before anything is allocated: the cost of reading an input is the input,
        // and a refusal should happen before the memory is spent rather than after.
        limits.scan(input.len() as u64)?;
        let text = Normalised::new(input.bytes())?;
        let level = Level::new(LEVEL).expect("`line` is a valid level");

        let mut rows: Vec<Segment> = Vec::new();
        let mut offset: u64 = 0;
        for (index, line) in text.as_str().split('\n').enumerate() {
            let len = line.len() as u64;
            // `enumerate` is 0-based and a line number is not. The locator is the only thing
            // in a reading a person types by hand, so it counts the way an editor does.
            let number = index as u64 + 1;
            if !line.trim().is_empty() {
                limits.node()?;
                rows.push(Segment::new(
                    offset,
                    offset + len,
                    level.clone(),
                    // `number` came from `enumerate() + 1`, so it is never the zero
                    // `Locator::line` refuses.
                    Locator::line(number).expect("a line number is 1-based"),
                ));
            }
            // `+ 1` for the newline `split` consumed. The last line has none, and the sum is
            // then one past the end — which is why this is the loop's last statement and
            // nothing reads `offset` afterwards.
            offset += len + 1;
        }

        Ok(ReadOutput::new(text, rows, level))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::Caps;
    use crate::readers::read_with;
    use crate::structure::Structure;

    fn read(bytes: &[u8]) -> ReadOutput {
        let mut budget = Budget::new(Caps::DEFAULT, bytes.len() as u64).expect("budget");
        read_with(&Txt, &Input::new(bytes), &Params::new(), &mut budget).expect("read")
    }

    /// Every node's range is exactly the line it names, taken out of the text itself.
    ///
    /// Checked by slicing rather than by comparing offsets to a table of expected numbers: an
    /// expectation written by hand can agree with an off-by-one, and the text cannot.
    #[test]
    fn each_node_is_the_line_its_locator_names() {
        let out = read(b"alpha\nbeta\ngamma\n");
        let text = out.text.as_str();
        assert_eq!(out.rows.len(), 3);
        for (row, expected) in out.rows.iter().zip(["alpha", "beta", "gamma"]) {
            let slice = &text[row.start as usize..row.end as usize];
            assert_eq!(slice, expected, "{:?}", row.locator);
        }
        assert_eq!(out.rows[2].locator, Locator::line(3).expect("L3"));
    }

    /// A blank line is numbered and not given a node.
    #[test]
    fn blank_lines_are_counted_but_not_nodes() {
        let out = read(b"one\n\n\nfour\n   \nsix\n");
        let numbers: Vec<_> = out
            .rows
            .iter()
            .map(|r| r.locator.to_string())
            .collect::<Vec<_>>();
        assert_eq!(numbers, vec!["L1", "L4", "L6"]);
        let text = out.text.as_str();
        assert_eq!(
            &text[out.rows[1].start as usize..out.rows[1].end as usize],
            "four"
        );
    }

    /// A file with no trailing newline keeps its last line.
    #[test]
    fn a_last_line_without_a_newline_is_still_a_line() {
        let out = read(b"alpha\nomega");
        assert_eq!(out.rows.len(), 2);
        let text = out.text.as_str();
        let last = out.rows.last().expect("a row");
        assert_eq!(&text[last.start as usize..last.end as usize], "omega");
        assert_eq!(last.end as usize, text.len());
    }

    /// The offsets are into the *normalised* text, not into the input.
    ///
    /// CRLF input is a byte longer per line than what a tid is taken over, so a reader that
    /// counted input bytes would hand every node a range one further along than the text it
    /// describes — and the error grows with the line number, which is the kind that looks
    /// right in a two-line test.
    #[test]
    fn offsets_follow_the_normalised_text_not_the_input() {
        let out = read(b"\xef\xbb\xbfalpha\r\nbeta\r\n");
        let text = out.text.as_str();
        assert_eq!(text, "alpha\nbeta\n");
        for (row, expected) in out.rows.iter().zip(["alpha", "beta"]) {
            assert_eq!(&text[row.start as usize..row.end as usize], expected);
        }
    }

    /// The rows are a structure: ordered, disjoint, and resolvable by the locators emitted.
    #[test]
    fn the_rows_build_a_structure_that_resolves_every_locator() {
        let out = read(b"alpha\nbeta\ngamma\n");
        let mut budget = Budget::new(Caps::DEFAULT, 1 << 16).expect("budget");
        let structure = Structure::build(&out.rows, out.text.len() as u64, &mut budget)
            .expect("the rows are a valid table");
        for row in &out.rows {
            assert_eq!(
                structure.resolve(&row.locator),
                Some(row.range()),
                "{:?}",
                row.locator
            );
        }
        assert_eq!(structure.roots().len(), 3, "lines are siblings, not nested");
    }

    /// Invalid UTF-8 is refused before anything is read.
    #[test]
    fn bytes_that_are_not_text_are_refused() {
        let mut budget = Budget::new(Caps::DEFAULT, 8).expect("budget");
        let err = read_with(
            &Txt,
            &Input::new(b"ok\n\xff\n"),
            &Params::new(),
            &mut budget,
        )
        .expect_err("refused");
        assert!(matches!(err, LibError::NotText { .. }), "{err:?}");
    }

    /// An empty input is a text with no nodes, not an error.
    #[test]
    fn an_empty_input_reads_as_an_empty_text() {
        let out = read(b"");
        assert!(out.rows.is_empty());
        assert!(out.text.is_empty());
        assert_eq!(out.top_level.as_str(), LEVEL);
    }

    /// The node cap is charged per node, so a file of many lines refuses with `SMY-E440`.
    #[test]
    fn too_many_lines_is_a_cap_refusal_naming_the_cap() {
        let mut caps = Caps::DEFAULT;
        caps.nodes = 4;
        let input = b"a\nb\nc\nd\ne\nf\n";
        let mut budget = Budget::new(caps, input.len() as u64).expect("budget");
        let err =
            read_with(&Txt, &Input::new(input), &Params::new(), &mut budget).expect_err("refused");
        match err {
            LibError::Limit { cap, limit, .. } => {
                assert_eq!(cap, "nodes");
                assert_eq!(limit, 4);
            }
            other => panic!("{other:?}"),
        }
    }
}
