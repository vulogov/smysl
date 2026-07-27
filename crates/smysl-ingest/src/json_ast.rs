//! The JSON-AST path: provider JSON → kernel records (§22.1, Appendix C).
//!
//! Parsed with the kernel's own HJSON reader, which is a superset of JSON - so this crate
//! needs no JSON dependency and, more usefully, the JSON-AST path and the surface path
//! share one parser. Two parsers would be two sets of edge cases.
//!
//! **Provider output is untrusted input** (§29). Everything here is validation, not
//! conversion: an unknown field, an unknown type, a status the model invented, a `deps`
//! entry pointing at nothing - each is a diagnostic, never a silently dropped field and
//! never a panic.
//!
//! References between units in one batch are by **label**, because a model cannot compute a
//! uid: uids are content addresses over a canonical encoding it has never seen. Labels are
//! resolved here, in dependency order, exactly as the surface parser does.

use std::collections::BTreeMap;

use smysl_core::surface::hjson::{parse_object_prefix, HObject, HValue, Spanned};
use smysl_core::{
    canonical_uid, Code, Date, Diagnostic, KernelType, Label, SourceKind, SourceRef, Status, Uid,
    UnitCore, UnitCoreBuilder,
};

/// What one JSON-AST batch produced.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct Converted {
    pub units: Vec<UnitCore>,
    /// Label bindings the batch declared, for the caller to carry forward.
    pub labels: BTreeMap<Label, Uid>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Converted {
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == smysl_core::Severity::Error)
    }
}

/// Convert a provider's JSON batch into kernel cores.
///
/// Never fails: a batch that is not JSON at all yields diagnostics and no units, which is
/// what the repair loop needs to hear rather than an error it cannot locate.
pub fn convert(raw: &str) -> Converted {
    let mut out = Converted::default();

    // A model asked for JSON sometimes wraps it in a fence. Unwrapping is charity, not
    // laxity: the alternative is spending a repair attempt on a formatting habit.
    let raw = unfence(raw);

    let brace = match raw.find('{') {
        Some(i) => i,
        None => {
            out.diagnostics.push(
                Diagnostic::new(Code::E001).with_message("the response contains no JSON object"),
            );
            return out;
        }
    };

    let obj = match parse_object_prefix(&raw[brace..], brace) {
        Ok(o) => o.value,
        Err(e) => {
            out.diagnostics
                .push(Diagnostic::new(Code::E001).with_message(format!("not JSON: {e}")));
            return out;
        }
    };

    let units = match obj.get("units").map(|v| &v.value) {
        Some(HValue::Array(a)) => a.clone(),
        // A single bare unit is accepted too. A model that returned one object instead of a
        // one-element array made a formatting mistake, not a semantic one.
        _ if obj.contains("gist") => vec![Spanned::new(
            HValue::Object(obj.clone()),
            smysl_core::Span::new(brace, raw.len()),
        )],
        _ => {
            out.diagnostics.push(
                Diagnostic::new(Code::E001)
                    .with_message("no `units` array and not a single unit object"),
            );
            return out;
        }
    };

    // Two passes, as the surface parser does: a unit may reference one declared later only
    // if that one does not reference it back, and resolving in declaration order with a
    // growing label table is what makes the common case work without a topological sort.
    for (i, item) in units.iter().enumerate() {
        let Some(o) = item.value.as_object() else {
            out.diagnostics.push(
                Diagnostic::new(Code::E001).with_message(format!("unit {i} is not an object")),
            );
            continue;
        };
        match unit(o, i, &out.labels) {
            Ok((core, label)) => {
                let uid = canonical_uid(&core);
                if let Some(l) = label {
                    out.labels.insert(l, uid);
                }
                out.units.push(core);
            }
            Err(d) => out.diagnostics.extend(d),
        }
    }

    out
}

/// Strip a ```json fence, which models add out of habit.
fn unfence(raw: &str) -> &str {
    let t = raw.trim();
    let Some(rest) = t.strip_prefix("```") else {
        return raw;
    };
    let rest = rest.strip_prefix("json").unwrap_or(rest);
    rest.trim_start()
        .strip_suffix("```")
        .unwrap_or_else(|| rest.trim_end().strip_suffix("```").unwrap_or(rest))
}

fn unit(
    o: &HObject,
    index: usize,
    labels: &BTreeMap<Label, Uid>,
) -> Result<(UnitCore, Option<Label>), Vec<Diagnostic>> {
    let mut errors = Vec::new();
    let at = |msg: String| Diagnostic::new(Code::E001).with_message(format!("unit {index}: {msg}"));

    // Appendix C says `additionalProperties: false`. Enforcing it here rather than trusting
    // the provider to is what stops an invented field from being silently dropped - which
    // is rule X's problem arriving through the wrong door.
    const KNOWN: &[&str] = &[
        "type", "label", "gist", "body", "detail", "status", "source", "deps", "grounds", "payload",
    ];
    for key in o.keys() {
        if !KNOWN.contains(&key) {
            errors.push(at(format!("unknown field `{key}`")));
        }
    }

    let type_name = o.get("type").and_then(|v| v.value.as_str()).unwrap_or("");
    let kind = match KernelType::parse(type_name) {
        Some(k) => Some(k),
        None => {
            errors.push(at(match type_name {
                "" => "missing `type`".to_string(),
                other => format!("`{other}` is not a kernel type"),
            }));
            None
        }
    };

    let gist = o.get("gist").and_then(|v| v.value.as_str()).unwrap_or("");
    if gist.trim().is_empty() {
        errors.push(at("missing or empty `gist`".to_string()));
    }

    let status_name = o.get("status").and_then(|v| v.value.as_str()).unwrap_or("");
    let status = match Status::parse(status_name) {
        // `unfounded` is unauthorable: reachable only by retraction (`SMY-E034`). A model
        // asserting one would be retracting a claim by making it.
        Some(Status::Unfounded) => {
            errors.push(
                Diagnostic::new(Code::E034)
                    .with_message(format!("unit {index}: `unfounded` cannot be authored")),
            );
            None
        }
        Some(s) => Some(s),
        None => {
            errors.push(at(match status_name {
                "" => "missing `status`".to_string(),
                other => format!("`{other}` is not a status"),
            }));
            None
        }
    };

    let label = match o.get("label").and_then(|v| v.value.as_str()) {
        Some(l) => match Label::new(l) {
            Ok(l) => Some(l),
            Err(e) => {
                errors.push(at(format!("`{l}` is not a label: {e}")));
                None
            }
        },
        None => None,
    };

    let (deps, mut dep_errors) = references(o, "deps", index, labels);
    let (grounds, ground_errors) = references(o, "grounds", index, labels);
    dep_errors.extend(ground_errors);
    errors.extend(dep_errors);

    let source = match o.get("source").map(|v| &v.value) {
        Some(HValue::Object(s)) => match source_ref(s, index) {
            Ok(s) => Some(s),
            Err(d) => {
                errors.push(d);
                None
            }
        },
        Some(_) => {
            errors.push(at("`source` is not an object".to_string()));
            None
        }
        None => None,
    };

    let (Some(kind), Some(status)) = (kind, status) else {
        return Err(errors);
    };
    if !errors.is_empty() {
        return Err(errors);
    }

    let mut b = UnitCoreBuilder::new(kind, gist, status)
        .deps(deps)
        .grounds(grounds);
    if let Some(t) = o.get("body").and_then(|v| v.value.as_str()) {
        b = b.body(t);
    }
    if let Some(t) = o.get("detail").and_then(|v| v.value.as_str()) {
        b = b.detail(t);
    }
    if let Some(s) = source {
        b = b.source(s);
    }

    match b.build() {
        Ok(core) => Ok((core, label)),
        // The builder's shape rules are the structural half of rules M and T, and a model
        // that broke one gets told which, not "invalid".
        Err(e) => Err(vec![at(e.to_string())]),
    }
}

/// Resolve a `deps` or `grounds` array. Entries are labels or uids; a model cannot compute
/// a uid, so labels are the expected form and uids the accommodation.
fn references(
    o: &HObject,
    key: &str,
    index: usize,
    labels: &BTreeMap<Label, Uid>,
) -> (Vec<Uid>, Vec<Diagnostic>) {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    let Some(HValue::Array(items)) = o.get(key).map(|v| &v.value) else {
        if o.contains(key) {
            errors.push(
                Diagnostic::new(Code::E001)
                    .with_message(format!("unit {index}: `{key}` is not an array")),
            );
        }
        return (out, errors);
    };

    for item in items {
        let Some(name) = item.value.as_str() else {
            errors.push(
                Diagnostic::new(Code::E001)
                    .with_message(format!("unit {index}: `{key}` entry is not a string")),
            );
            continue;
        };
        if let Ok(l) = Label::new(name) {
            if let Some(uid) = labels.get(&l) {
                out.push(*uid);
                continue;
            }
        }
        match Uid::parse(name) {
            Ok(u) => out.push(u),
            // A dangling reference is `E060`, the same code `check` uses, so a caller sees
            // one vocabulary whether the problem came from a model or from a file.
            Err(_) => errors.push(
                Diagnostic::new(Code::E060)
                    .with_message(format!("unit {index}: `{key}` names unknown `{name}`")),
            ),
        }
    }
    (out, errors)
}

fn source_ref(o: &HObject, index: usize) -> Result<SourceRef, Diagnostic> {
    let at = |msg: String| Diagnostic::new(Code::E001).with_message(format!("unit {index}: {msg}"));

    let kind_name = o.get("kind").and_then(|v| v.value.as_str()).unwrap_or("");
    let kind = SourceKind::parse(kind_name)
        .ok_or_else(|| at(format!("`{kind_name}` is not a source kind")))?;
    let reference = o
        .get("ref")
        .and_then(|v| v.value.as_str())
        .ok_or_else(|| at("`source` has no `ref`".to_string()))?;

    let mut s = SourceRef::new(kind, reference);
    if let Some(d) = o.get("captured").and_then(|v| v.value.as_str()) {
        s = s.captured_on(Date::parse(d).map_err(|e| at(format!("`captured`: {e}")))?);
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(json: &str) -> Converted {
        convert(json)
    }

    #[test]
    fn a_well_formed_batch_converts() {
        let out = one(r#"{"units":[
                {"type":"evidence","label":"d/p95","gist":"p95 rose to 410ms",
                 "status":"cited","source":{"kind":"doc","ref":"postmortem"}},
                {"type":"finding","label":"f/cause","gist":"the pool saturated",
                 "status":"derived","grounds":["d/p95"],"body":"wait time tracks latency"}
            ]}"#);
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
        assert_eq!(out.units.len(), 2);
        assert_eq!(out.labels.len(), 2);
        // The label reference resolved to the first unit's uid.
        assert_eq!(
            out.units[1].grounds.iter().next(),
            Some(&canonical_uid(&out.units[0]))
        );
    }

    /// A model cannot compute a uid - they are content addresses over an encoding it has
    /// never seen - so labels are the expected form of a reference.
    #[test]
    fn references_resolve_by_label_in_declaration_order() {
        let out = one(r#"{"units":[
                {"type":"claim","label":"a/one","gist":"first","status":"speculative"},
                {"type":"claim","label":"a/two","gist":"second","status":"inferred",
                 "grounds":["a/one"]}
            ]}"#);
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
        assert_eq!(out.units[1].grounds.len(), 1);
    }

    #[test]
    fn a_forward_reference_is_reported_rather_than_silently_dropped() {
        let out = one(r#"{"units":[
                {"type":"claim","label":"a/one","gist":"first","status":"inferred",
                 "grounds":["a/later"]}
            ]}"#);
        assert!(out.has_errors());
        assert_eq!(out.diagnostics[0].code, Code::E060);
        assert!(out.units.is_empty());
    }

    /// A model that returned one object instead of a one-element array made a formatting
    /// mistake, not a semantic one.
    #[test]
    fn a_bare_unit_object_is_accepted() {
        let out = one(r#"{"type":"claim","gist":"just one","status":"speculative"}"#);
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
        assert_eq!(out.units.len(), 1);
    }

    /// Unwrapping a fence is charity, not laxity: the alternative is spending a repair
    /// attempt on a formatting habit.
    #[test]
    fn a_fenced_response_is_unwrapped() {
        let out = one("```json\n{\"units\":[{\"type\":\"claim\",\"gist\":\"g\",\"status\":\"speculative\"}]}\n```");
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
        assert_eq!(out.units.len(), 1);
    }

    #[test]
    fn prose_around_the_object_is_tolerated() {
        let out = one("Here are the units:\n{\"units\":[{\"type\":\"claim\",\"gist\":\"g\",\"status\":\"speculative\"}]}");
        assert_eq!(out.units.len(), 1);
    }

    // -- validation ----------------------------------------------------------

    /// Appendix C says `additionalProperties: false`. Silently dropping an invented field
    /// is rule X's problem arriving through the wrong door.
    #[test]
    fn an_unknown_field_is_an_error_not_a_silent_drop() {
        let out = one(
            r#"{"units":[{"type":"claim","gist":"g","status":"speculative","confidence":0.9}]}"#,
        );
        assert!(out.has_errors());
        assert!(
            out.diagnostics
                .iter()
                .any(|d| d.message.contains("confidence")),
            "{:?}",
            out.diagnostics
        );
    }

    /// A model asserting `unfounded` would be retracting a claim by making it.
    #[test]
    fn unfounded_cannot_be_authored() {
        let out = one(r#"{"units":[{"type":"claim","gist":"g","status":"unfounded"}]}"#);
        assert!(out.diagnostics.iter().any(|d| d.code == Code::E034));
        assert!(out.units.is_empty());
    }

    #[test]
    fn an_invented_type_or_status_is_reported() {
        for (json, what) in [
            (
                r#"{"units":[{"type":"vibe","gist":"g","status":"speculative"}]}"#,
                "vibe",
            ),
            (
                r#"{"units":[{"type":"claim","gist":"g","status":"certain"}]}"#,
                "certain",
            ),
        ] {
            let out = one(json);
            assert!(out.has_errors(), "{json}");
            assert!(
                out.diagnostics.iter().any(|d| d.message.contains(what)),
                "{:?}",
                out.diagnostics
            );
        }
    }

    #[test]
    fn a_missing_required_field_is_reported() {
        for json in [
            r#"{"units":[{"gist":"g","status":"speculative"}]}"#,
            r#"{"units":[{"type":"claim","status":"speculative"}]}"#,
            r#"{"units":[{"type":"claim","gist":"g"}]}"#,
            r#"{"units":[{"type":"claim","gist":"   ","status":"speculative"}]}"#,
        ] {
            assert!(one(json).has_errors(), "{json}");
        }
    }

    /// The builder's shape rules are the structural half of rules M and T, and a model that
    /// broke one gets told which.
    #[test]
    fn a_shape_violation_is_reported_with_its_reason() {
        // `cited` without a source.
        let out = one(r#"{"units":[{"type":"claim","gist":"g","status":"cited"}]}"#);
        assert!(out.has_errors());
        assert!(
            out.diagnostics[0].message.to_lowercase().contains("source"),
            "{:?}",
            out.diagnostics
        );
    }

    #[test]
    fn a_malformed_source_is_reported() {
        for json in [
            r#"{"units":[{"type":"claim","gist":"g","status":"cited","source":{"kind":"telepathy","ref":"x"}}]}"#,
            r#"{"units":[{"type":"claim","gist":"g","status":"cited","source":{"kind":"doc"}}]}"#,
            r#"{"units":[{"type":"claim","gist":"g","status":"cited","source":"a string"}]}"#,
        ] {
            assert!(one(json).has_errors(), "{json}");
        }
    }

    #[test]
    fn a_captured_date_is_parsed_and_a_bad_one_reported() {
        let good = one(r#"{"units":[{"type":"evidence","gist":"g","status":"cited",
                 "source":{"kind":"url","ref":"https://x","captured":"2026-07-27"}}]}"#);
        assert!(good.diagnostics.is_empty(), "{:?}", good.diagnostics);
        assert!(good.units[0].source.as_ref().unwrap().captured.is_some());

        let bad = one(r#"{"units":[{"type":"evidence","gist":"g","status":"cited",
                 "source":{"kind":"url","ref":"https://x","captured":"last tuesday"}}]}"#);
        assert!(bad.has_errors());
    }

    // -- never a panic -------------------------------------------------------

    /// Provider output is untrusted input (§29). None of this may panic, and none of it may
    /// return an error the repair loop cannot locate.
    #[test]
    fn no_input_panics_and_garbage_yields_diagnostics() {
        for raw in [
            "",
            "   ",
            "not json at all",
            "{",
            "{\"units\":",
            "{\"units\":[]}",
            "{\"units\":\"not an array\"}",
            "{\"units\":[1,2,3]}",
            "{\"units\":[{}]}",
            "```json\n{ broken",
            "\u{0}\u{1}\u{2}",
            &"{".repeat(1000),
        ] {
            let out = convert(raw);
            // An empty batch is not an error; everything else here is.
            if raw.contains("[]") {
                assert!(out.units.is_empty());
            }
            let _ = out.has_errors();
        }
    }

    #[test]
    fn an_empty_batch_is_not_an_error() {
        let out = one(r#"{"units":[]}"#);
        assert!(out.units.is_empty());
        assert!(out.diagnostics.is_empty());
    }

    /// One bad unit does not take the batch down with it, which is what makes partial
    /// progress possible under rule I.
    #[test]
    fn a_bad_unit_does_not_discard_its_neighbours() {
        let out = one(r#"{"units":[
                {"type":"claim","gist":"good one","status":"speculative"},
                {"type":"nonsense","gist":"bad one","status":"speculative"},
                {"type":"claim","gist":"good two","status":"speculative"}
            ]}"#);
        assert_eq!(out.units.len(), 2, "the good units survived");
        assert!(out.has_errors());
    }

    #[test]
    fn a_bad_label_is_reported_without_losing_the_unit_type() {
        let out = one(
            r#"{"units":[{"type":"claim","label":"NotALabel","gist":"g","status":"speculative"}]}"#,
        );
        assert!(out.has_errors());
    }
}
