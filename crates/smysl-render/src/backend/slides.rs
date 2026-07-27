//! The slides backend, Typst-backed, behind the `typst` feature (§20).
//!
//! One block per slide; the role becomes the slide kind. That is the whole design, and it
//! is why slides need a backend rather than a profile: the *unit of the document* changes
//! from the paragraph to the block, which no amount of styling expresses.
//!
//! Notes do not become footnotes here - a slide has no page bottom. They become speaker
//! notes, which is where a presenter wants provenance anyway.

use smysl_core::error::RenderError;

use super::typst::escape;
use super::{suppression_note, Artifact, Backend};
use crate::ir::{Ir, NoteKind};
use crate::profile::Profile;
use crate::Target;

pub struct Slides;

impl Backend for Slides {
    fn emit(&self, ir: &Ir, p: &Profile) -> Result<Artifact, RenderError> {
        let mut out = String::new();
        out.push_str("#set page(width: 25cm, height: 14cm, margin: 1.5cm)\n");
        out.push_str("#set text(size: 20pt)\n");
        out.push_str(&format!(
            "#set document(title: \"{}\")\n\n",
            escape(&ir.gist)
        ));

        // Title slide.
        out.push_str(&format!("#align(center + horizon)[\n  #text(size: 28pt)[{}]\n\n  #text(size: 14pt)[{} · {}]\n]\n#pagebreak()\n\n",
            escape(&ir.gist),
            escape(&ir.meta.schema),
            escape(&ir.meta.profile),
        ));

        for (i, b) in ir.blocks.iter().enumerate() {
            if i > 0 {
                out.push_str("#pagebreak()\n\n");
            }
            out.push_str(&format!(
                "// slide {}: {}\n#text(size: 14pt)[{}]\n\n",
                i + 1,
                escape(&b.uid.short()),
                escape(&b.role.to_string())
            ));
            let line = b.joined();
            out.push_str(&format!(
                "#strong[{}] {}\n\n",
                escape(&b.marker),
                escape(&line)
            ));

            let contested: Vec<&crate::ir::Note> = b
                .notes
                .iter()
                .filter(|n| n.kind == NoteKind::Contention)
                .collect();
            for n in &contested {
                // Rule V2 on a slide deck means on the slide, not in the speaker notes:
                // the audience sees the slide and never the notes.
                out.push_str(&format!(
                    "#block(stroke: 1pt, inset: 6pt)[contested — {}]\n\n",
                    escape(&n.text)
                ));
            }
            for n in b.notes.iter().filter(|n| n.kind != NoteKind::Contention) {
                out.push_str(&format!("// speaker note: {}\n", escape(&n.text)));
            }
        }

        if let Some(note) = suppression_note(ir) {
            out.push_str(&format!("\n// {}\n", escape(&note)));
        } else if !ir.meta.open_contentions.is_empty() {
            out.push_str(&format!(
                "\n#pagebreak()\n#strong[Open contentions:] {}\n",
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

        let _ = p;
        Ok(Artifact::new(Target::Slides, out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fixture;

    fn render() -> (crate::ir::Ir, String) {
        let p = Profile::builtin("plain").unwrap();
        let ir = fixture::ir(&p);
        let text = Slides.emit(&ir, &p).unwrap().text;
        (ir, text)
    }

    /// One block per slide, plus the title slide and a closing contentions slide - that
    /// change in the unit of the document is the whole reason this is a backend rather
    /// than a profile.
    #[test]
    fn there_is_one_slide_per_block_plus_the_title_and_the_contentions() {
        let (ir, s) = render();
        let slides = s.matches("#pagebreak()").count() + 1;
        assert!(!ir.meta.open_contentions.is_empty(), "the fixture contests");
        assert_eq!(slides, ir.blocks.len() + 2);
    }

    #[test]
    fn an_uncontested_deck_has_exactly_one_slide_per_block_plus_the_title() {
        let p = Profile::load("profile q { show: { contentions: suppress } }").unwrap();
        let (store, thread) = fixture::corpus();
        let ir = crate::ir::build(&store, &thread, &p, &crate::ir::BuildOptions::default());
        let s = Slides.emit(&ir, &p).unwrap().text;
        assert_eq!(s.matches("#pagebreak()").count() + 1, ir.blocks.len() + 1);
    }

    #[test]
    fn the_role_becomes_the_slide_kind() {
        let (ir, s) = render();
        for b in &ir.blocks {
            assert!(
                s.contains(&escape(&b.role.to_string())),
                "no slide kind for {}",
                b.role
            );
        }
    }

    /// A slide has no page bottom, so provenance becomes a speaker note rather than a
    /// footnote that would have nowhere to sit.
    #[test]
    fn provenance_becomes_a_speaker_note() {
        let (_, s) = render();
        assert!(s.contains("// speaker note:"), "{s}");
        assert!(!s.contains("#footnote["));
    }

    /// ...but a contention does not. The audience sees the slide and never the notes, so
    /// rule V2 would be defeated by hiding a contention in the commentary.
    #[test]
    fn a_contention_stays_on_the_slide_itself() {
        let (_, s) = render();
        assert!(s.contains("#block(stroke:"), "{s}");
        assert!(!s.contains("// speaker note: k/pool-vs-canary"));
    }

    #[test]
    fn the_deck_opens_with_a_title_slide() {
        let (_, s) = render();
        assert!(s.contains("#align(center + horizon)["), "{s}");
    }

    #[test]
    fn suppression_is_recorded() {
        let p = Profile::load("profile q { show: { contentions: suppress } }").unwrap();
        let (store, thread) = fixture::corpus();
        let ir = crate::ir::build(&store, &thread, &p, &crate::ir::BuildOptions::default());
        let s = Slides.emit(&ir, &p).unwrap().text;
        assert!(s.contains("// SMY-W211"), "{s}");
    }
}
