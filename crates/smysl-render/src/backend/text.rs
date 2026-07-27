//! The plain-text backend. Always available (§20).
//!
//! No markup at all, which makes it the one artifact a terminal, a commit message, or a
//! prompt can take verbatim. Wrapping is deliberately absent: the consumer knows its own
//! width and this one does not.

use smysl_core::error::RenderError;

use super::{suppression_note, Artifact, Backend};
use crate::ir::Ir;
use crate::profile::Profile;
use crate::Target;

pub struct Text;

impl Backend for Text {
    fn emit(&self, ir: &Ir, _p: &Profile) -> Result<Artifact, RenderError> {
        let mut out = String::new();

        let title = if ir.gist.is_empty() {
            ir.meta.thread.as_str()
        } else {
            ir.gist.as_str()
        };
        out.push_str(title);
        out.push('\n');
        out.push_str(&"=".repeat(title.chars().count()));
        out.push_str("\n\n");

        for block in &ir.blocks {
            out.push_str(&format!("{} [{}]\n", block.role, block.uid.short()));
            let line = block.joined();
            for l in line.lines() {
                // A marker on a blank separator line is trailing whitespace pretending to
                // be a qualification.
                if l.trim().is_empty() {
                    out.push('\n');
                } else {
                    out.push_str(&format!("  {} {l}\n", block.marker));
                }
            }
            for note in &block.notes {
                out.push_str(&format!("      - {}\n", note.text));
            }
            out.push('\n');
        }

        if let Some(note) = suppression_note(ir) {
            out.push_str(&note);
            out.push('\n');
        } else if !ir.meta.open_contentions.is_empty() {
            out.push_str("open contentions: ");
            out.push_str(
                &ir.meta
                    .open_contentions
                    .iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            out.push('\n');
        }

        Ok(Artifact::new(Target::Text, out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fixture;

    fn render() -> String {
        let p = Profile::builtin("plain").unwrap();
        Text.emit(&fixture::ir(&p), &p).unwrap().text
    }

    #[test]
    fn there_is_no_markup() {
        let t = render();
        for marker in ["# ", "## ", "[^", "<!--", "```"] {
            assert!(!t.contains(marker), "found `{marker}` in plain text:\n{t}");
        }
    }

    #[test]
    fn the_title_is_underlined_to_its_own_width() {
        let t = render();
        let mut lines = t.lines();
        let title = lines.next().unwrap();
        let rule = lines.next().unwrap();
        assert_eq!(title.chars().count(), rule.chars().count());
        assert!(rule.chars().all(|c| c == '='));
    }

    #[test]
    fn each_block_names_its_role_and_short_uid() {
        let t = render();
        assert!(t.contains("bottom-line ["), "{t}");
    }

    /// A multi-line body must not lose its marker on the continuation lines, or the second
    /// paragraph would read as unqualified.
    #[test]
    fn every_line_of_a_block_carries_the_marker() {
        let p = Profile::builtin("plain").unwrap();
        let ir = fixture::ir(&p);
        let t = Text.emit(&ir, &p).unwrap().text;
        let block = ir.blocks.iter().find(|b| b.text.contains('\n')).unwrap();
        for line in block.text.lines().filter(|l| !l.trim().is_empty()) {
            // Block lines are indented; searching the whole document would match the
            // title, which contains the bottom line as a prefix.
            let rendered = t
                .lines()
                .filter(|r| r.starts_with("  "))
                .find(|r| r.contains(line))
                .unwrap_or_else(|| panic!("line missing: {line}"));
            assert!(rendered.contains(&block.marker), "unmarked: {rendered}");
        }
    }

    #[test]
    fn a_blank_separator_line_carries_no_marker() {
        let t = render();
        for line in t.lines() {
            assert_eq!(line, line.trim_end(), "trailing whitespace: {line:?}");
        }
    }

    #[test]
    fn open_contentions_are_listed() {
        assert!(render().contains("open contentions: k/pool-vs-canary"));
    }
}
