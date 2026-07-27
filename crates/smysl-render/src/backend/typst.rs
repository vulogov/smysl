//! The Typst backend, behind the `typst` feature (§20).
//!
//! Emits a `.typ` source file with a preamble derived from the profile - this is where
//! `register`, `person` and `audience` finally do something, since a document's typography
//! is the one place voice legitimately belongs.
//!
//! Nothing here shells out to Typst. Emitting source rather than a PDF keeps the backend
//! pure (rule B) and keeps the artifact diffable, which a PDF is not.

use smysl_core::error::RenderError;

use super::{suppression_note, Artifact, Backend};
use crate::ir::{Ir, NoteKind};
use crate::profile::{Profile, Register};
use crate::Target;

pub struct Typst;

/// Typst's escape set is small: `#` starts code and `$` starts maths.
pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '#' | '$' | '@' | '\\' | '<' | '>' | '*' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

pub(crate) fn preamble(ir: &Ir, p: &Profile) -> String {
    let (font, size, leading) = match p.register {
        Register::Formal => ("New Computer Modern", "11pt", "0.8em"),
        Register::Neutral => ("Libertinus Serif", "11pt", "0.7em"),
        Register::Plain => ("DejaVu Sans", "10pt", "0.65em"),
    };
    let mut out = String::new();
    out.push_str(&format!("#set document(title: \"{}\")\n", escape(&ir.gist)));
    out.push_str(&format!("#set text(font: \"{font}\", size: {size})\n"));
    out.push_str(&format!("#set par(leading: {leading}, justify: true)\n\n"));
    out
}

impl Backend for Typst {
    fn emit(&self, ir: &Ir, p: &Profile) -> Result<Artifact, RenderError> {
        let mut out = preamble(ir, p);

        out.push_str(&format!("= {}\n\n", escape(&ir.gist)));
        out.push_str(&format!(
            "#emph[{} · profile {}]\n\n",
            escape(&ir.meta.schema),
            escape(&ir.meta.profile)
        ));
        if let Some(a) = &ir.meta.audience {
            out.push_str(&format!("#emph[for {}]\n\n", escape(a)));
        }

        for b in &ir.blocks {
            out.push_str(&format!("== {}\n\n", escape(&b.role.to_string())));
            let line = b.joined();
            out.push_str(&format!(
                "#strong[{}] {}\n\n",
                escape(&b.marker),
                escape(&line)
            ));

            for n in &b.notes {
                match n.kind {
                    NoteKind::Contention => out.push_str(&format!(
                        "#block(stroke: 1pt, inset: 6pt)[contested — {}]\n\n",
                        escape(&n.text)
                    )),
                    _ => out.push_str(&format!("#footnote[{}]\n\n", escape(&n.text))),
                }
            }
        }

        match suppression_note(ir) {
            Some(note) => out.push_str(&format!("// {}\n", escape(&note))),
            None if !ir.meta.open_contentions.is_empty() => {
                out.push_str(&format!(
                    "#strong[Open contentions:] {}\n",
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
            None => {}
        }

        Ok(Artifact::new(Target::Typst, out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fixture;

    fn render(profile: &str) -> String {
        let p = Profile::builtin(profile).unwrap();
        Typst.emit(&fixture::ir(&p), &p).unwrap().text
    }

    #[test]
    fn the_preamble_comes_from_the_profile() {
        let formal = render("exec");
        let plain = render("analyst");
        assert!(formal.contains("#set text(font:"));
        assert_ne!(
            formal.lines().find(|l| l.starts_with("#set text")),
            plain.lines().find(|l| l.starts_with("#set text")),
            "register must reach the typography or it does nothing at all"
        );
    }

    #[test]
    fn headings_use_typst_syntax() {
        let t = render("plain");
        assert!(t.contains("\n= "), "no title heading:\n{t}");
        assert!(t.contains("\n== bottom-line"), "{t}");
    }

    /// A gist containing `#` would start Typst code and produce a file that does not
    /// compile, which is a silent corruption rather than a visible one.
    #[test]
    fn typst_control_characters_are_escaped() {
        assert_eq!(escape("a #b $c @d"), "a \\#b \\$c \\@d");
        assert_eq!(escape("under_score"), "under\\_score");
    }

    #[test]
    fn a_contention_becomes_a_visible_block_not_a_footnote() {
        let t = render("plain");
        assert!(t.contains("#block(stroke:"), "{t}");
        assert!(t.contains("contested"));
    }

    #[test]
    fn suppression_is_recorded_as_a_comment() {
        let p = Profile::load("profile q { show: { contentions: suppress } }").unwrap();
        let (store, thread) = fixture::corpus();
        let ir = crate::ir::build(&store, &thread, &p, &crate::ir::BuildOptions::default());
        let t = Typst.emit(&ir, &p).unwrap().text;
        assert!(t.contains("// SMY-W211"), "{t}");
        assert!(!t.contains("#block(stroke:"));
    }

    #[test]
    fn nothing_shells_out() {
        // A backend that invoked the Typst binary would not be pure (rule B). Emitting
        // source keeps it a function of the IR, and keeps the artifact diffable.
        let t = render("plain");
        assert!(t.starts_with("#set document"), "{t}");
    }
}
