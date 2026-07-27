//! The markdown backend. Always available (§20).
//!
//! Notes become footnotes when the profile asks for footnotes and parentheticals when it
//! asks for inline, because a markdown reader that does not resolve footnote syntax should
//! still be able to see where a claim came from.

use smysl_core::error::RenderError;

use super::{suppression_note, Artifact, Backend};
use crate::ir::{Ir, NoteKind};
use crate::profile::{Profile, Provenance};
use crate::Target;

pub struct Markdown;

impl Backend for Markdown {
    fn emit(&self, ir: &Ir, p: &Profile) -> Result<Artifact, RenderError> {
        let mut out = String::new();

        out.push_str("# ");
        out.push_str(if ir.gist.is_empty() {
            &ir.meta.thread
        } else {
            &ir.gist
        });
        out.push_str("\n\n");

        out.push_str(&format!(
            "*{} · profile {}*\n",
            ir.meta.schema, ir.meta.profile
        ));
        if let Some(a) = &ir.meta.audience {
            out.push_str(&format!("*for {a}*\n"));
        }
        out.push('\n');

        let mut footnotes: Vec<(usize, String)> = Vec::new();

        for block in &ir.blocks {
            out.push_str(&format!("## {}\n\n", block.role));

            let line = block.joined();
            out.push_str(&format!("{} {}\n", block.marker, line.trim_start()));

            for note in &block.notes {
                match (note.kind, p.show.provenance) {
                    (NoteKind::Provenance, Provenance::Footnote) => {
                        let n = footnotes.len() + 1;
                        out.push_str(&format!("[^{n}]\n"));
                        footnotes.push((n, note.text.clone()));
                    }
                    (NoteKind::Contention, _) => {
                        out.push_str(&format!("\n> **contested** — {}\n", note.text));
                    }
                    _ => out.push_str(&format!("\n*{}*\n", note.text)),
                }
            }
            out.push('\n');
        }

        if !footnotes.is_empty() {
            out.push_str("---\n\n");
            for (n, text) in &footnotes {
                out.push_str(&format!("[^{n}]: {text}\n"));
            }
            out.push('\n');
        }

        if let Some(note) = suppression_note(ir) {
            out.push_str(&format!("---\n\n<!-- {note} -->\n"));
        } else if !ir.meta.open_contentions.is_empty() {
            out.push_str("---\n\n**Open contentions:** ");
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

        Ok(Artifact::new(Target::Markdown, out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fixture;

    fn render(profile: &str) -> String {
        let p = Profile::builtin(profile).unwrap();
        Markdown.emit(&fixture::ir(&p), &p).unwrap().text
    }

    #[test]
    fn the_gist_becomes_the_title() {
        let md = render("plain");
        assert!(
            md.starts_with("# Pool saturation is the leading cause"),
            "{md}"
        );
    }

    #[test]
    fn each_role_becomes_a_heading() {
        let md = render("plain");
        assert!(md.contains("## bottom-line"));
        assert!(md.contains("## support"));
        assert!(md.contains("## risk"));
    }

    #[test]
    fn footnotes_are_numbered_and_defined() {
        let md = render("plain");
        assert!(md.contains("[^1]\n"), "no footnote reference: {md}");
        assert!(md.contains("[^1]: "), "no footnote definition: {md}");
    }

    /// A contention is a blockquote rather than a footnote: rule V2 says it is surfaced,
    /// and a footnote at the bottom of the page is not surfacing.
    #[test]
    fn a_contention_is_surfaced_in_the_body() {
        let md = render("plain");
        assert!(md.contains("> **contested**"), "{md}");
        assert!(md.contains("k/pool-vs-canary"));
    }

    #[test]
    fn suppression_is_recorded_as_a_comment() {
        let p = Profile::load("profile q { show: { contentions: suppress } }").unwrap();
        let (store, thread) = fixture::corpus();
        let ir = crate::ir::build(&store, &thread, &p, &crate::ir::BuildOptions::default());
        let md = Markdown.emit(&ir, &p).unwrap().text;
        assert!(md.contains("<!-- SMY-W211"), "{md}");
        assert!(!md.contains("> **contested**"));
    }

    #[test]
    fn the_analyst_profile_spells_statuses_out() {
        let md = render("analyst");
        assert!(md.contains("[derived]"), "{md}");
        assert!(md.contains("[speculative]"));
    }

    #[test]
    fn the_audience_appears_when_the_profile_names_one() {
        assert!(render("exec").contains("*for engineering leadership*"));
        assert!(!render("plain").contains("*for "));
    }
}
