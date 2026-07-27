//! The JSON backend. Always available (§20).
//!
//! This one is the IR itself, serialised - the target for a consuming agent rather than a
//! reader. Keys are emitted in a fixed order and floats never appear, so two renders of the
//! same graph are byte-identical and a diff over artifacts is a diff over content.
//!
//! Hand-written rather than derived: `smysl-render` is a pure crate, and a serialisation
//! framework is a dependency the format does not need in order to emit six field names.

use smysl_core::error::RenderError;
use smysl_core::json_escape as q;

use super::{Artifact, Backend};
use crate::ir::Ir;
use crate::profile::Profile;
use crate::Target;

pub struct Json;

impl Backend for Json {
    fn emit(&self, ir: &Ir, p: &Profile) -> Result<Artifact, RenderError> {
        let mut out = String::new();
        out.push_str("{\n");

        out.push_str(&format!("  \"gist\": {},\n", q(&ir.gist)));

        out.push_str("  \"meta\": {\n");
        out.push_str(&format!("    \"profile\": {},\n", q(&ir.meta.profile)));
        out.push_str(&format!("    \"thread\": {},\n", q(&ir.meta.thread)));
        out.push_str(&format!("    \"schema\": {},\n", q(&ir.meta.schema)));
        out.push_str(&format!(
            "    \"audience\": {},\n",
            match &ir.meta.audience {
                Some(a) => q(a),
                None => "null".into(),
            }
        ));
        out.push_str(&format!(
            "    \"contentions_suppressed\": {},\n",
            ir.meta.contentions_suppressed
        ));
        // Rule V2: which contentions, not merely how many. A count would tell a consumer
        // that something was hidden without telling it what to go and look up.
        out.push_str("    \"open_contentions\": [");
        out.push_str(
            &ir.meta
                .open_contentions
                .iter()
                .map(|c| q(&c.to_string()))
                .collect::<Vec<_>>()
                .join(", "),
        );
        out.push(']');
        if ir.meta.contentions_suppressed {
            out.push_str(",\n    \"warning\": ");
            out.push_str(&q(&super::suppression_note(ir).unwrap_or_default()));
        }
        out.push_str("\n  },\n");

        out.push_str("  \"blocks\": [\n");
        for (i, b) in ir.blocks.iter().enumerate() {
            out.push_str("    {\n");
            out.push_str(&format!("      \"role\": {},\n", q(&b.role.to_string())));
            out.push_str(&format!("      \"uid\": {},\n", q(&b.uid.canonical())));
            out.push_str(&format!("      \"level\": {},\n", q(&b.level.to_string())));
            out.push_str(&format!(
                "      \"status\": {},\n",
                q(&b.status.to_string())
            ));
            out.push_str(&format!("      \"marker\": {},\n", q(&b.marker)));
            out.push_str(&format!(
                "      \"connective\": {},\n",
                match b.connective {
                    Some(c) => q(c),
                    None => "null".into(),
                }
            ));
            out.push_str(&format!("      \"text\": {},\n", q(&b.text)));
            out.push_str("      \"notes\": [");
            out.push_str(
                &b.notes
                    .iter()
                    .map(|n| {
                        format!(
                            "{{\"kind\": {}, \"text\": {}}}",
                            q(&format!("{:?}", n.kind).to_lowercase()),
                            q(&n.text)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            out.push_str("]\n");
            out.push_str(if i + 1 == ir.blocks.len() {
                "    }\n"
            } else {
                "    },\n"
            });
        }
        out.push_str("  ]\n}\n");

        let _ = p;
        Ok(Artifact::new(Target::Json, out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fixture;
    use crate::ir::{build, BuildOptions};

    fn render(profile: &str) -> String {
        let p = Profile::builtin(profile).unwrap();
        Json.emit(&fixture::ir(&p), &p).unwrap().text
    }

    /// No parser here, so the check is structural: balanced braces and brackets, and no
    /// trailing comma before a close. It catches the mistakes a hand-written emitter
    /// actually makes.
    fn well_formed(s: &str) -> bool {
        let mut depth = 0i32;
        let mut in_str = false;
        let mut escaped = false;
        let mut prev_significant = ' ';
        for c in s.chars() {
            if in_str {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_str = false;
                    prev_significant = '"';
                }
                continue;
            }
            match c {
                '"' => in_str = true,
                '{' | '[' => depth += 1,
                '}' | ']' => {
                    depth -= 1;
                    if prev_significant == ',' {
                        return false;
                    }
                }
                _ => {}
            }
            if !c.is_whitespace() {
                prev_significant = c;
            }
            if depth < 0 {
                return false;
            }
        }
        depth == 0 && !in_str
    }

    #[test]
    fn the_output_is_well_formed() {
        for p in ["plain", "exec", "analyst"] {
            let j = render(p);
            assert!(well_formed(&j), "{p}:\n{j}");
        }
    }

    #[test]
    fn every_block_carries_its_uid_and_status() {
        let p = Profile::builtin("plain").unwrap();
        let ir = fixture::ir(&p);
        let j = Json.emit(&ir, &p).unwrap().text;
        for b in &ir.blocks {
            assert!(j.contains(&b.uid.canonical()), "missing uid");
            assert!(j.contains(&format!("\"status\": \"{}\"", b.status)));
        }
    }

    #[test]
    fn suppression_appears_as_a_flag_a_warning_and_the_ids() {
        let p = Profile::load("profile q { show: { contentions: suppress } }").unwrap();
        let (store, thread) = fixture::corpus();
        let ir = build(&store, &thread, &p, &BuildOptions::default());
        let j = Json.emit(&ir, &p).unwrap().text;
        assert!(j.contains("\"contentions_suppressed\": true"));
        assert!(j.contains("SMY-W211"));
        assert!(j.contains("k/pool-vs-canary"));
        assert!(well_formed(&j), "{j}");
    }

    #[test]
    fn an_unsuppressed_render_says_so() {
        let j = render("plain");
        assert!(j.contains("\"contentions_suppressed\": false"));
        assert!(!j.contains("warning"));
    }

    #[test]
    fn a_null_audience_is_a_json_null_not_an_empty_string() {
        assert!(render("plain").contains("\"audience\": null"));
        assert!(render("exec").contains("\"audience\": \"engineering leadership\""));
    }

    #[test]
    fn an_absent_connective_is_null() {
        assert!(render("plain").contains("\"connective\": null"));
    }

    #[test]
    fn an_empty_document_is_still_well_formed() {
        let p = Profile::builtin("plain").unwrap();
        let ir = build(
            &smysl_graph::Store::new(),
            &smysl_core::Thread::new(
                smysl_core::ThreadId::new("t/e").unwrap(),
                smysl_core::ThreadSchema::Brief,
                smysl_core::AgentId::new("tool:t").unwrap(),
                "",
                smysl_core::Hlc::zero(smysl_core::AgentId::new("tool:t").unwrap()),
            ),
            &p,
            &BuildOptions::default(),
        );
        let j = Json.emit(&ir, &p).unwrap().text;
        assert!(well_formed(&j), "{j}");
    }

    /// The escaping is the reason `json_escape` exists rather than `{:?}`.
    #[test]
    fn text_with_quotes_and_newlines_stays_parseable() {
        let p = Profile::builtin("analyst").unwrap();
        let ir = fixture::ir(&p);
        let j = Json.emit(&ir, &p).unwrap().text;
        assert!(!j.contains("\\u{"), "Rust-style escapes are not JSON");
        assert!(well_formed(&j));
    }
}
