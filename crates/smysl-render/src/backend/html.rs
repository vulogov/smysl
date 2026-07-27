//! The HTML backend, behind the `html` feature (§20).
//!
//! Semantic elements and a `data-status` attribute on every block, so rule V1's
//! distinction survives into the DOM and not merely into the visible glyph. A stylesheet
//! that hid the marker would still leave the status machine-readable.
//!
//! No external CSS and no scripts: the artifact is one file that says what it says without
//! fetching anything.

use smysl_core::error::RenderError;

use super::{suppression_note, Artifact, Backend};
use crate::ir::{Ir, NoteKind};
use crate::profile::Profile;
use crate::Target;

pub struct Html;

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

impl Backend for Html {
    fn emit(&self, ir: &Ir, p: &Profile) -> Result<Artifact, RenderError> {
        let mut out = String::new();
        out.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n");
        out.push_str("<meta charset=\"utf-8\">\n");
        out.push_str(&format!("<title>{}</title>\n", escape(&ir.gist)));
        out.push_str(&format!(
            "<meta name=\"smysl-profile\" content=\"{}\">\n",
            escape(&ir.meta.profile)
        ));
        out.push_str(&format!(
            "<meta name=\"smysl-thread\" content=\"{}\">\n",
            escape(&ir.meta.thread)
        ));
        // Rule V2 lives in the head as well as the body, so a suppressed contention is
        // recoverable from the document even if nothing in the visible text mentions it.
        out.push_str(&format!(
            "<meta name=\"smysl-contentions-suppressed\" content=\"{}\">\n",
            ir.meta.contentions_suppressed
        ));
        if !ir.meta.open_contentions.is_empty() {
            out.push_str(&format!(
                "<meta name=\"smysl-open-contentions\" content=\"{}\">\n",
                escape(
                    &ir.meta
                        .open_contentions
                        .iter()
                        .map(|c| c.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                )
            ));
        }
        out.push_str("</head>\n<body>\n");

        out.push_str(&format!("<h1>{}</h1>\n", escape(&ir.gist)));
        out.push_str(&format!(
            "<p class=\"meta\">{} · profile {}</p>\n",
            escape(&ir.meta.schema),
            escape(&ir.meta.profile)
        ));

        for b in &ir.blocks {
            out.push_str(&format!(
                "<section class=\"block\" data-role=\"{}\" data-status=\"{}\" data-uid=\"{}\" data-lod=\"{}\">\n",
                escape(&b.role.to_string()),
                escape(&b.status.to_string()),
                escape(&b.uid.canonical()),
                escape(&b.level.to_string()),
            ));
            out.push_str(&format!("<h2>{}</h2>\n", escape(&b.role.to_string())));
            let line = b.joined();
            out.push_str(&format!(
                "<p><span class=\"status-marker\">{}</span> {}</p>\n",
                escape(&b.marker),
                escape(&line)
            ));
            for n in &b.notes {
                let class = match n.kind {
                    NoteKind::Contention => "contention",
                    NoteKind::Provenance => "provenance",
                    _ => "note",
                };
                out.push_str(&format!(
                    "<aside class=\"{class}\">{}</aside>\n",
                    escape(&n.text)
                ));
            }
            out.push_str("</section>\n");
        }

        if let Some(note) = suppression_note(ir) {
            out.push_str(&format!("<!-- {} -->\n", escape(&note)));
        } else if !ir.meta.open_contentions.is_empty() {
            out.push_str(&format!(
                "<footer><strong>Open contentions:</strong> {}</footer>\n",
                escape(
                    &ir.meta
                        .open_contentions
                        .iter()
                        .map(|c| c.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            ));
        }

        out.push_str("</body>\n</html>\n");
        let _ = p;
        Ok(Artifact::new(Target::Html, out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fixture;

    fn render() -> (crate::ir::Ir, String) {
        let p = Profile::builtin("plain").unwrap();
        let ir = fixture::ir(&p);
        let text = Html.emit(&ir, &p).unwrap().text;
        (ir, text)
    }

    /// Status must survive into the DOM, not only into the visible glyph: a stylesheet
    /// that hides the marker must not be able to flatten the document.
    #[test]
    fn every_block_carries_its_status_as_an_attribute() {
        let (ir, h) = render();
        for b in &ir.blocks {
            assert!(
                h.contains(&format!("data-status=\"{}\"", b.status)),
                "no data-status for {}",
                b.uid
            );
        }
    }

    #[test]
    fn markup_characters_in_content_are_escaped() {
        assert_eq!(escape("<a & 'b'>"), "&lt;a &amp; &#39;b&#39;&gt;");
    }

    #[test]
    fn the_document_is_self_contained() {
        let (_, h) = render();
        for forbidden in ["<script", "<link", "http://", "https://"] {
            assert!(!h.contains(forbidden), "found `{forbidden}`");
        }
    }

    #[test]
    fn suppression_is_in_the_head_as_well_as_a_comment() {
        let p = Profile::load("profile q { show: { contentions: suppress } }").unwrap();
        let (store, thread) = fixture::corpus();
        let ir = crate::ir::build(&store, &thread, &p, &crate::ir::BuildOptions::default());
        let h = Html.emit(&ir, &p).unwrap().text;
        assert!(h.contains("name=\"smysl-contentions-suppressed\" content=\"true\""));
        assert!(h.contains("SMY-W211"));
    }

    #[test]
    fn tags_are_balanced() {
        let (_, h) = render();
        for tag in ["html", "head", "body", "section"] {
            let open = h.matches(&format!("<{tag}")).count();
            let close = h.matches(&format!("</{tag}>")).count();
            assert_eq!(open, close, "unbalanced <{tag}>");
        }
    }
}
