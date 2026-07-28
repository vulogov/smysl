//! The kernel JSON Schema for the JSON-AST path (Appendix C).
//!
//! Emitted as a string rather than assembled from a JSON library: this crate has no JSON
//! dependency, the schema is a constant, and a constant that a test compares against the
//! kernel's own type tables cannot drift from them silently.
//!
//! **One schema, translated per provider.** This is draft 2020-12, written conservatively;
//! a mapper whose endpoint speaks something narrower translates it on the way out (§21.2,
//! responsibility 2). Five schemas would be five things to keep in step, and the one that
//! drifted would be the one nobody ran.
//!
//! It was originally written as the *intersection* of what every provider accepts. That was
//! wrong, and a live Gemini call proved it: Gemini's `responseSchema` is an OpenAPI 3.0
//! `Schema` object, not a subset of draft 2020-12, and it has no `additionalProperties`
//! field at all - while OpenAI strict mode *requires* `additionalProperties: false`. The
//! intersection of those two is not a schema. See `smysl_provider::map::gemini::dialect`.
//!
//! The schema encodes the **structural** half of rules M and T. The ordering half of M and
//! the ceiling half of T are not expressible in JSON Schema and are enforced by `check`
//! after conversion. `unfounded` is deliberately absent from the status enum - it is
//! unauthorable, reachable only by retraction.

use smysl_core::{KernelType, RelKind, SourceKind, Status};

/// The unit schema (Appendix C).
pub fn unit_schema() -> String {
    format!(
        r#"{{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "smysl.kernel/0.1 unit",
  "type": "object",
  "required": ["type", "gist", "status"],
  "additionalProperties": false,
  "properties": {{
    "type": {{ "enum": [{types}] }},
    "label": {{ "type": "string", "pattern": "^[a-z][a-z0-9_-]*/[a-z0-9_-]+$" }},
    "gist": {{ "type": "string", "minLength": 1, "maxLength": {gist_max} }},
    "body": {{ "type": "string" }},
    "detail": {{ "type": "string" }},
    "status": {{ "enum": [{statuses}] }},
    "source": {{
      "type": "object",
      "required": ["kind", "ref"],
      "properties": {{
        "kind": {{ "enum": [{source_kinds}] }},
        "ref": {{ "type": "string" }},
        "captured": {{ "type": "string" }}
      }}
    }},
    "deps": {{ "type": "array", "items": {{ "type": "string" }} }},
    "grounds": {{ "type": "array", "items": {{ "type": "string" }} }},
    "payload": {{ "type": "object" }}
  }},
  "allOf": [
    {{ "if": {{ "properties": {{ "status": {{ "enum": ["measured", "cited"] }} }} }},
      "then": {{ "required": ["source"] }} }},
    {{ "if": {{ "properties": {{ "status": {{ "enum": ["derived", "inferred"] }} }} }},
      "then": {{ "required": ["grounds"] }} }},
    {{ "if": {{ "required": ["detail"] }}, "then": {{ "required": ["body"] }} }}
  ]
}}"#,
        types = quoted(authorable_types()),
        statuses = quoted(authorable_statuses()),
        source_kinds = quoted(SourceKind::ALL.iter().map(|k| k.as_str().to_string())),
        gist_max = GIST_MAX_CHARS,
    )
}

/// The relation schema.
///
/// **Relations are what most of the format's machinery operates on**, and for a long time
/// ingest could not produce any: rule R had no rebuttals to keep with a claim, merge had no
/// live rebuttals to detect, threads could not fill a `caveat` role, and rendering had no
/// kind to choose a connective by. A graph ingested without edges is a list with provenance.
///
/// `weight` is optional and bounded, because a rebuttal's strength is the one quantity a
/// reader most wants and a model most readily invents.
pub fn relation_schema() -> String {
    format!(
        r#"{{
  "type": "object",
  "required": ["kind", "from", "to"],
  "properties": {{
    "kind": {{ "enum": [{kinds}] }},
    "from": {{ "type": "string" }},
    "to": {{ "type": "string" }},
    "weight": {{ "type": "number", "minimum": 0, "maximum": 1 }}
  }}
}}"#,
        kinds = quoted(authorable_relations()),
    )
}

/// A batch of units and the edges between them, which is what an ingest call asks for.
pub fn batch_schema() -> String {
    format!(
        r#"{{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "smysl.kernel/0.1 unit batch",
  "type": "object",
  "required": ["units"],
  "additionalProperties": false,
  "properties": {{
    "units": {{ "type": "array", "items": {unit} }},
    "relations": {{ "type": "array", "items": {relation} }}
  }}
}}"#,
        unit = indent(&unit_schema()),
        relation = indent(&relation_schema()),
    )
}

/// Relation kinds a model may author.
///
/// `supersedes` and `retracts` are excluded: both are *lifecycle* edges that assert
/// something about the history of a graph the model is only reading. A model that could
/// retract a unit by mentioning it would be a model that could delete evidence.
pub fn authorable_relations() -> Vec<String> {
    RelKind::KERNEL
        .iter()
        .filter(|k| !matches!(k, RelKind::Supersedes | RelKind::Retracts))
        .map(|k| k.as_str().to_string())
        .collect()
}

/// Appendix C's gist bound. Characters, not tokens: a JSON Schema cannot count tokens, and
/// a bound the model can actually respect is worth more than an exact one it cannot.
pub const GIST_MAX_CHARS: usize = 240;

/// Kernel types a model may author.
///
/// `contention` and `packinfo` are excluded: both are *derived* records that merge and pack
/// produce, and a model asserting one would be fabricating a machine's conclusion.
pub fn authorable_types() -> Vec<String> {
    KernelType::ALL
        .iter()
        .filter(|t| !matches!(t, KernelType::Contention | KernelType::PackInfo))
        .map(|t| t.as_str().to_string())
        .collect()
}

/// Statuses a model may author. `unfounded` is unauthorable (§1.4, `SMY-E034`).
pub fn authorable_statuses() -> Vec<String> {
    Status::ALL
        .iter()
        .filter(|s| **s != Status::Unfounded)
        .map(|s| s.as_str().to_string())
        .collect()
}

fn quoted(items: impl IntoIterator<Item = String>) -> String {
    items
        .into_iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

fn indent(s: &str) -> String {
    s.lines().collect::<Vec<_>>().join("\n    ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A crude structural check, since this crate has no JSON parser: balanced braces and
    /// brackets, no trailing comma before a close.
    fn well_formed(s: &str) -> bool {
        let mut depth = 0i32;
        let mut in_str = false;
        let mut escaped = false;
        let mut prev = ' ';
        for c in s.chars() {
            if in_str {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_str = false;
                    prev = '"';
                }
                continue;
            }
            match c {
                '"' => in_str = true,
                '{' | '[' => depth += 1,
                '}' | ']' => {
                    depth -= 1;
                    if prev == ',' {
                        return false;
                    }
                }
                _ => {}
            }
            if !c.is_whitespace() {
                prev = c;
            }
            if depth < 0 {
                return false;
            }
        }
        depth == 0 && !in_str
    }

    #[test]
    fn both_schemas_are_well_formed() {
        assert!(well_formed(&unit_schema()), "{}", unit_schema());
        assert!(well_formed(&batch_schema()));
    }

    /// The whole point of generating rather than hand-writing: the enum cannot drift from
    /// the kernel's own type table.
    #[test]
    fn the_type_enum_tracks_the_kernel() {
        let s = unit_schema();
        for t in KernelType::ALL {
            let quoted = format!("\"{}\"", t.as_str());
            let authorable = !matches!(t, KernelType::Contention | KernelType::PackInfo);
            assert_eq!(
                s.contains(&quoted),
                authorable,
                "{t} is {} in the schema",
                if authorable { "missing" } else { "present" }
            );
        }
    }

    /// `unfounded` is reachable only by retraction. A schema that let a model author one
    /// would be a schema that let a model retract a claim by asserting it.
    #[test]
    fn unfounded_is_absent_from_the_status_enum() {
        let s = unit_schema();
        assert!(!s.contains("\"unfounded\""));
        for st in Status::ALL.iter().filter(|s| **s != Status::Unfounded) {
            assert!(s.contains(&format!("\"{}\"", st.as_str())), "{st}");
        }
    }

    /// A model asserting a contention would be fabricating a machine's conclusion: merge
    /// produces those, and only merge.
    #[test]
    fn derived_record_types_are_unauthorable() {
        let t = authorable_types();
        assert!(!t.contains(&"contention".to_string()));
        assert!(!t.contains(&"packinfo".to_string()));
        assert!(t.contains(&"claim".to_string()));
        assert!(t.contains(&"prose".to_string()), "rule I needs prose");
    }

    /// The structural half of rules M and T. The ordering half of M and the ceiling half of
    /// T are enforced by `check` after conversion - JSON Schema cannot express either.
    #[test]
    fn the_conditional_requirements_are_present() {
        let s = unit_schema();
        assert!(s.contains(r#""required": ["source"]"#), "measured/cited");
        assert!(s.contains(r#""required": ["grounds"]"#), "derived/inferred");
        assert!(s.contains(r#""required": ["body"]"#), "detail implies body");
    }

    #[test]
    fn the_source_kind_enum_tracks_the_kernel() {
        let s = unit_schema();
        for k in SourceKind::ALL {
            assert!(s.contains(&format!("\"{}\"", k.as_str())), "{k}");
        }
    }

    #[test]
    fn the_gist_bound_is_appendix_cs() {
        assert_eq!(GIST_MAX_CHARS, 240);
        assert!(unit_schema().contains("\"maxLength\": 240"));
    }

    /// A conservative core, not an intersection - no provider dialect reached here takes
    /// any of these, so a keyword from this list would be one every mapper had to translate
    /// away. What a *particular* endpoint refuses is that mapper's problem to translate.
    #[test]
    fn the_schema_stays_inside_the_conservative_core() {
        let s = unit_schema();
        for outside in [
            "$ref",
            "$defs",
            "patternProperties",
            "propertyNames",
            "oneOf",
            "not",
            "dependentSchemas",
            "unevaluatedProperties",
            "contains",
            "format",
        ] {
            assert!(
                !s.contains(outside),
                "`{outside}` is outside the conservative core"
            );
        }
    }

    #[test]
    fn the_batch_schema_wraps_the_unit_schema() {
        let b = batch_schema();
        assert!(b.contains("\"units\""));
        assert!(
            b.contains("\"smysl.kernel/0.1 unit\""),
            "the unit is inlined"
        );
        assert!(b.contains("\"type\": \"array\""));
    }

    #[test]
    fn additional_properties_are_refused() {
        // Without this, a model can invent a field and the converter silently drops it -
        // which is rule X's problem arriving through the wrong door.
        assert!(unit_schema().contains(r#""additionalProperties": false"#));
    }
}
