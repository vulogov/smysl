//! The canonical surface emitter (§6, `fmt`).
//!
//! `fmt` MUST canonicalise, and `surface → CBOR → surface` MUST be lossless modulo
//! formatting. Emission is therefore fully determined by the records and their labels:
//! field order is fixed, quoting is decided by content rather than by how the author
//! wrote it, and the writer always emits `→` even where `->` was read.
//!
//! Hashes are computed over CBOR only, so reformatting never changes identity.

use std::collections::BTreeMap;

use crate::ids::{Label, Uid};
use crate::surface::hjson::{HObject, HValue};
use crate::surface::payload::payload_to_object;
use crate::types::relation::Relation;
use crate::types::thread::Thread;
use crate::types::unit::UnitCore;
use crate::types::view::View;
use crate::types::Record;

/// Everything the writer needs that is not in the records themselves.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct WriteContext {
    /// uid -> label, so references come back out as the names they were authored with.
    pub labels: BTreeMap<Uid, Label>,
    pub salience: BTreeMap<Uid, f32>,
    /// What the `@doc` header will declare.
    ///
    /// Defaults to `FORMAT_VERSION_DEFAULT`, which is what a document with no history — one
    /// built from CBOR, where the wire carries no version — should say. Set it from
    /// `ParseOutcome::format_version` to re-emit a document as the version it arrived as.
    pub format_version: String,
}

impl WriteContext {
    /// Build from the label map a parse produced.
    ///
    /// Two labels can name one uid — identity is content, so `@claim a/x` and `@claim a/y`
    /// with the same gist and status are the same unit under two names. Surface syntax
    /// writes a label as part of a unit declaration and has nowhere to put a second, so one
    /// of them cannot be emitted. This keeps the **first in canonical order** and the parser
    /// drops the same one, so `parse -> write -> parse` agrees with itself rather than
    /// silently swapping which name survives.
    pub fn from_labels(labels: &BTreeMap<Label, Uid>) -> WriteContext {
        let mut out: BTreeMap<Uid, Label> = BTreeMap::new();
        for (l, u) in labels {
            out.entry(*u).or_insert_with(|| l.clone());
        }
        WriteContext {
            labels: out,
            salience: BTreeMap::new(),
            format_version: crate::FORMAT_VERSION_DEFAULT.to_string(),
        }
    }

    /// Emit the version a document declared, rather than the one this build prefers.
    ///
    /// A round trip that relabelled a document would be the defect §8.5 describes: content
    /// unaffected, because uids are over CBOR and CBOR carries no version, but the header
    /// lying about which version it is — and the next reader trusts the header.
    pub fn with_format_version(mut self, version: impl Into<String>) -> WriteContext {
        self.format_version = version.into();
        self
    }

    pub fn with_salience(mut self, salience: BTreeMap<Uid, f32>) -> WriteContext {
        self.salience = salience;
        self
    }

    /// A reference as it should appear: the label if one is bound, else the canonical uid.
    fn reference(&self, u: &Uid) -> String {
        match self.labels.get(u) {
            Some(l) => l.as_str().to_string(),
            None => u.canonical(),
        }
    }
}

/// Emit a document in canonical surface form.
pub fn write_surface(view: Option<&View>, records: &[Record], ctx: &WriteContext) -> String {
    let mut out = String::new();
    if let Some(v) = view {
        write_doc(&mut out, v, ctx);
    }
    for r in records {
        match r {
            Record::Unit(u) => write_unit(&mut out, u, ctx),
            Record::Relation(rel) => write_relation(&mut out, rel, ctx),
            Record::Thread(t) => write_thread(&mut out, t, ctx),
            // Records with no surface form travel as CBOR only.
            _ => {}
        }
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

fn write_doc(out: &mut String, v: &View, ctx: &WriteContext) {
    out.push_str("@doc ");
    out.push_str(&ctx.format_version);
    out.push_str(" {\n");
    out.push_str(&format!("  id: {}\n", quoteless_or_quoted(v.id.as_str())));
    if !v.intent.is_empty() {
        out.push_str(&format!("  intent: {}\n", quoteless_or_quoted(&v.intent)));
    }
    out.push_str(&format!(
        "  lang: {}\n",
        quoteless_or_quoted(v.lang.as_str())
    ));
    if !v.requires.is_empty() {
        let items: Vec<String> = v
            .requires
            .iter()
            .map(|s| quoteless_or_quoted(s.as_str()))
            .collect();
        out.push_str(&format!("  requires: [{}]\n", items.join(", ")));
    }
    out.push_str(&format!(
        "  granularity: {{ profile: {}, l0_max: {}, l1_range: [{}, {}], admission: {} }}\n",
        quoteless_or_quoted(&v.granularity.profile),
        v.granularity.l0_max,
        v.granularity.l1_min,
        v.granularity.l1_max,
        v.granularity.admission
    ));
    if !v.threads.is_empty() {
        let items: Vec<String> = v
            .threads
            .iter()
            .map(|t| quoteless_or_quoted(t.as_str()))
            .collect();
        out.push_str(&format!("  threads: [{}]\n", items.join(", ")));
    }
    if !v.roots.is_empty() {
        let items: Vec<String> = v.roots.iter().map(|u| ctx.reference(u)).collect();
        out.push_str(&format!("  roots: [{}]\n", items.join(", ")));
    }
    out.push_str("}\n\n");
}

fn write_unit(out: &mut String, u: &UnitCore, ctx: &WriteContext) {
    let uid = crate::hash::canonical_uid(u);
    out.push('@');
    out.push_str(u.schema.as_str());
    if let Some(l) = ctx.labels.get(&uid) {
        out.push(' ');
        out.push_str(l.as_str());
    }

    let mut fields: Vec<String> = Vec::new();
    fields.push(format!("status: {}", u.status));
    if !u.deps.is_empty() {
        fields.push(format!("deps: [{}]", refs(&u.deps, ctx)));
    }
    if !u.grounds.is_empty() {
        fields.push(format!("grounds: [{}]", refs(&u.grounds, ctx)));
    }
    if let Some(s) = &u.source {
        let mut src = format!(
            "source: {{ kind: {}, ref: {}",
            s.kind,
            quoteless_or_quoted(&s.reference)
        );
        if let Some(d) = s.captured {
            src.push_str(&format!(", captured: {d}"));
        }
        src.push_str(" }");
        fields.push(src);
    }
    if let Some(s) = ctx.salience.get(&uid) {
        fields.push(format!("salience: {}", trim_float(*s)));
    }
    if let Some(p) = &u.payload {
        // Unknown header keys come back out as they went in (rule X).
        if let Ok(o) = payload_to_object(p) {
            for (k, v) in o.iter() {
                fields.push(format!("{}: {}", quoted_key(&k.value), value(&v.value)));
            }
        }
    }
    out.push_str(&format!(" {{ {} }}\n", fields.join(", ")));

    out.push_str("~ ");
    out.push_str(&u.gist);
    out.push('\n');

    if let Some(b) = &u.body {
        out.push('\n');
        out.push_str(&escape_block(b));
        out.push('\n');
    }
    if let Some(d) = &u.detail {
        out.push_str("\n--\n");
        out.push_str(&escape_block(d));
        out.push('\n');
    }
    out.push('\n');
}

/// Escape any line of a body or detail that would otherwise read back as a comment.
///
/// A line beginning `#` or `//` at column 0 is a comment wherever it sits, so a body holding
/// a Markdown heading or a line of C++ could not be written back and read again unchanged —
/// the line was simply dropped. `\\` is escaped too, because a line already starting with a
/// backslash would otherwise be *un*escaped on the way in and lose it.
///
/// Nothing else is touched: a backslash anywhere but at the start of a line, and a `#` in
/// the middle of one, are ordinary text and stay ordinary.
fn escape_block(text: &str) -> String {
    if !text
        .lines()
        .any(|l| l.starts_with('#') || l.starts_with("//") || l.starts_with('\\'))
    {
        return text.to_string();
    }
    text.lines()
        .map(|l| {
            if l.starts_with('#') || l.starts_with("//") || l.starts_with('\\') {
                format!("\\{l}")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn write_relation(out: &mut String, r: &Relation, ctx: &WriteContext) {
    out.push_str(&format!(
        "@rel {} --{}--> {}",
        ctx.reference(&r.from),
        r.kind,
        ctx.reference(&r.to)
    ));
    let mut fields: Vec<String> = Vec::new();
    if let Some(w) = r.weight {
        fields.push(format!("weight: {}", trim_float(w)));
    }
    if let Some(n) = &r.note {
        fields.push(format!("note: {}", ctx.reference(n)));
    }
    if !fields.is_empty() {
        out.push_str(&format!(" {{ {} }}", fields.join(", ")));
    }
    out.push_str("\n\n");
}

fn write_thread(out: &mut String, t: &Thread, ctx: &WriteContext) {
    out.push_str(&format!(
        "@thread {} {{ schema: {}, owner: {}, ts: [{}, {}] }}\n",
        t.id,
        t.schema,
        quoteless_or_quoted(t.owner.as_str()),
        t.ts.wall_ms,
        t.ts.counter
    ));
    out.push_str("~ ");
    out.push_str(&t.gist);
    out.push('\n');
    for s in &t.steps {
        out.push_str(&format!("  {} \u{2192} {}", s.role, ctx.reference(&s.unit)));
        if let Some(n) = &s.note {
            out.push_str(&format!(": {n}"));
        }
        out.push('\n');
    }
    out.push('\n');
}

fn refs(set: &std::collections::BTreeSet<Uid>, ctx: &WriteContext) -> String {
    set.iter()
        .map(|u| ctx.reference(u))
        .collect::<Vec<_>>()
        .join(", ")
}

fn value(v: &HValue) -> String {
    match v {
        HValue::Null => "null".into(),
        HValue::Bool(b) => b.to_string(),
        HValue::Int(i) => i.to_string(),
        HValue::Float(f) => trim_float(*f as f32),
        HValue::Str(s) => quoteless_or_quoted(s),
        HValue::Array(a) => format!(
            "[{}]",
            a.iter()
                .map(|i| value(&i.value))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        HValue::Object(o) => format!("{{ {} }}", object(o)),
    }
}

fn object(o: &HObject) -> String {
    o.iter()
        .map(|(k, v)| format!("{}: {}", quoted_key(&k.value), value(&v.value)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Quote only when the content would not survive quoteless parsing.
///
/// Deciding by content rather than by how the author wrote it is what makes `fmt`
/// idempotent: reformatting a formatted file changes nothing.
fn quoteless_or_quoted(s: &str) -> String {
    let needs_quotes = s.is_empty()
        || s.trim() != s
        || s.chars().any(|c| {
            matches!(
                c,
                ',' | '}' | ']' | '{' | '[' | '"' | '\\' | '\n' | '\r' | '\t'
            )
        })
        || matches!(s, "true" | "false" | "null")
        // A header accepts `#` and `//` comments inside a record. The comment skip runs
        // *before* a value begins, while a quoteless value itself runs to `,`, `}`, `]` or
        // end of line without stopping at either marker — so the hazard is a value that
        // **starts** with one, which gets eaten along with the rest of the line and the
        // closing brace, losing the whole unit. A marker in the middle is ordinary text,
        // which is why `grafana://board/12` needs no quotes and does not get any. Comments
        // arrived in 0.2.0 and this quoter was not revisited; found by fuzzing on `["#x"]`.
        || s.starts_with('#')
        || s.starts_with("//")
        || s.parse::<f64>().is_ok();
    if !needs_quotes {
        return s.to_string();
    }
    quoted(s)
}

/// Quote a *key* when a bare one would not read back as the same key.
///
/// Keys were emitted raw while values went through `quoteless_or_quoted`. An unknown header
/// key survives verbatim under rule X, and nothing constrains what a peer puts there — a key
/// holding a newline, a `:` or a `}` tore the header apart, and the whole unit vanished on
/// re-parse. Found by fuzzing: the writer emitting what its own parser rejects.
///
/// The bare-key terminators are the parser's own (`:`, whitespace, `,`, `{`, `}`), plus the
/// two characters that would break the quoted form itself. `char::is_whitespace` is wider
/// than the parser's `is_ascii_whitespace` on purpose: quoting one key too many costs a pair
/// of quotes, and quoting one too few loses a record.
fn quoted_key(s: &str) -> String {
    let needs_quotes = s.is_empty()
        || s.starts_with('#')
        || s.starts_with("//")
        || s.chars()
            .any(|c| c.is_whitespace() || matches!(c, ':' | ',' | '{' | '}' | '"' | '\\'));
    if !needs_quotes {
        return s.to_string();
    }
    quoted(s)
}

/// The quoted form, with the escapes the parser understands.
fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Render a quantised float without trailing noise. Values are already multiples of
/// 1/1024, so at most four decimal places are ever needed to round-trip one.
fn trim_float(f: f32) -> String {
    let mut s = format!("{f:.6}");
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.push('0');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::KernelType;
    use crate::types::epistemics::{SourceKind, SourceRef, Status};
    use crate::types::unit::UnitCoreBuilder;

    fn ctx() -> WriteContext {
        WriteContext::default()
    }

    #[test]
    fn quoteless_where_possible() {
        assert_eq!(quoteless_or_quoted("measured"), "measured");
        assert_eq!(quoteless_or_quoted("c/auth-p95"), "c/auth-p95");
        assert_eq!(quoteless_or_quoted("2026-07-09"), "2026-07-09");
        // A colon is safe in a value: only keys terminate at one.
        assert_eq!(quoteless_or_quoted("model:x/y"), "model:x/y");
        assert_eq!(
            quoteless_or_quoted("grafana://board/12"),
            "grafana://board/12"
        );
    }

    #[test]
    fn quotes_what_would_not_survive_quoteless_parsing() {
        for s in [
            "", " x", "x ", "a, b", "a}b", "a]b", "a\nb", "true", "42", "0.5",
        ] {
            let q = quoteless_or_quoted(s);
            assert!(q.starts_with('"'), "{s:?} should be quoted, got {q}");
        }
    }

    #[test]
    fn quoting_escapes_the_json_set() {
        assert_eq!(quoteless_or_quoted("a\"b"), r#""a\"b""#);
        assert_eq!(quoteless_or_quoted("a\\b"), r#""a\\b""#);
        assert_eq!(quoteless_or_quoted("a\nb"), r#""a\nb""#);
        assert_eq!(quoteless_or_quoted("a\tb"), r#""a\tb""#);
    }

    #[test]
    fn floats_render_without_trailing_noise() {
        assert_eq!(trim_float(0.5), "0.5");
        assert_eq!(trim_float(1.0), "1.0");
        assert_eq!(trim_float(0.0), "0.0");
        assert_eq!(trim_float(614.0 / 1024.0), "0.599609");
    }

    #[test]
    fn a_gist_only_unit_emits_a_header_a_gist_and_nothing_else() {
        let u = UnitCoreBuilder::new(KernelType::Claim, "p95 tripled", Status::Speculative)
            .build()
            .unwrap();
        let s = write_surface(None, &[Record::Unit(u)], &ctx());
        assert_eq!(s, "@claim { status: speculative }\n~ p95 tripled\n");
    }

    #[test]
    fn a_label_follows_the_type() {
        let u = UnitCoreBuilder::new(KernelType::Claim, "g", Status::Speculative)
            .build()
            .unwrap();
        let uid = crate::hash::canonical_uid(&u);
        let mut c = ctx();
        c.labels.insert(uid, Label::new("c/x").unwrap());
        let s = write_surface(None, &[Record::Unit(u)], &c);
        assert!(s.starts_with("@claim c/x {"), "{s}");
    }

    #[test]
    fn a_source_is_emitted_inline() {
        let u = UnitCoreBuilder::new(KernelType::Evidence, "traces", Status::Measured)
            .source(
                SourceRef::new(SourceKind::Metric, "grafana://board/12")
                    .captured_on(crate::types::Date::new(2026, 7, 9).unwrap()),
            )
            .build()
            .unwrap();
        let s = write_surface(None, &[Record::Unit(u)], &ctx());
        assert!(
            s.contains("source: { kind: metric, ref: grafana://board/12, captured: 2026-07-09 }"),
            "{s}"
        );
    }

    #[test]
    fn body_and_detail_are_separated_as_the_grammar_requires() {
        let u = UnitCoreBuilder::new(KernelType::Claim, "g", Status::Speculative)
            .body("the body")
            .detail("the detail")
            .build()
            .unwrap();
        let s = write_surface(None, &[Record::Unit(u)], &ctx());
        assert!(s.contains("~ g\n\nthe body\n\n--\nthe detail"), "{s}");
    }

    #[test]
    fn the_writer_always_emits_the_unicode_arrow() {
        use crate::ids::{AgentId, ThreadId};
        use crate::types::provenance::Hlc;
        use crate::types::thread::{Role, Step, Thread, ThreadSchema};
        let ag = AgentId::new("model:openai/gpt").unwrap();
        let t = Thread::new(
            ThreadId::new("t/brief").unwrap(),
            ThreadSchema::Brief,
            ag.clone(),
            "a brief",
            Hlc::zero(ag),
        )
        .with_steps([Step::new(Role::BottomLine, Uid::from_bytes([1; 32]))]);
        let s = write_surface(None, &[Record::Thread(t)], &ctx());
        assert!(s.contains(" \u{2192} "), "{s}");
        assert!(!s.contains("->"));
    }

    #[test]
    fn records_without_a_surface_form_are_skipped() {
        let r = Record::Unknown {
            code: 99,
            payload: vec![0xA0],
        };
        assert_eq!(write_surface(None, &[r], &ctx()), "");
    }
}
