//! A span-tracking HJSON-subset parser for record headers and configuration.
//!
//! §13 nominates the `deser-hjson` crate. It is a serde deserializer, and header handling
//! needs two things serde cannot give here: an **untyped** value model, because unknown
//! keys must be captured verbatim into `payload` (rule X at the surface level), and a
//! **byte span for every key and value**, because the ingest repair loop resends only the
//! offending region rather than the whole chunk (§22.3). So the subset is parsed here,
//! and `smysl-core` keeps no serde dependency.
//!
//! The subset:
//!
//! - objects `{ k: v, k2: v2 }`, nestable, commas optional at line ends;
//! - arrays `[a, b]`;
//! - quoted strings with the JSON escape set;
//! - quoteless strings, terminated by `,` `}` `]` or end of line - a pragmatic narrowing
//!   of HJSON, which allows them only as the last value on a line, adopted because the
//!   RFC's own examples put them in `[…]` and after commas;
//! - integers, floats, `true`, `false`, `null`;
//! - `#`, `//`, and `/* */` comments **between** entries; a quoteless value runs to
//!   end of line, so a comment cannot follow one on the same line.
//!
//! Object entries keep their authored order, which is what lets an unknown key round-trip
//! into `payload` and back out unchanged.

use core::fmt;

use crate::diag::Span;

/// A value with the byte range it was parsed from.
#[derive(Debug, Clone, PartialEq)]
pub struct Spanned<T> {
    pub value: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub fn new(value: T, span: Span) -> Spanned<T> {
        Spanned { value, span }
    }
}

/// An HJSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum HValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<Spanned<HValue>>),
    Object(HObject),
}

impl HValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            HValue::Null => "null",
            HValue::Bool(_) => "boolean",
            HValue::Int(_) => "integer",
            HValue::Float(_) => "float",
            HValue::Str(_) => "string",
            HValue::Array(_) => "array",
            HValue::Object(_) => "object",
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            HValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            HValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            HValue::Int(i) => Some(*i as f64),
            HValue::Float(f) => Some(*f),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            HValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Spanned<HValue>]> {
        match self {
            HValue::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&HObject> {
        match self {
            HValue::Object(o) => Some(o),
            _ => None,
        }
    }
}

/// An object, preserving authored key order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HObject {
    entries: Vec<(Spanned<String>, Spanned<HValue>)>,
}

impl HObject {
    pub fn new() -> HObject {
        HObject::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&Spanned<HValue>> {
        self.entries
            .iter()
            .find(|(k, _)| k.value == key)
            .map(|(_, v)| v)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Remove and return a key. Header parsing takes the keys it knows this way, so
    /// whatever remains is by definition unrecognised and belongs in `payload`.
    /// Remove **every** entry under `key`, returning the first.
    ///
    /// Removing only the first left any duplicate behind, and whatever a caller leaves
    /// behind becomes the unknown-key payload under rule X. So a header with two `deps`
    /// parsed as one real `deps` plus a second smuggled into the payload — which the writer
    /// then emitted as a plain `deps:` line, because that is what the key is called. On the
    /// next parse there was only one, it was taken as the field, and the payload lost a key:
    /// `parse -> write -> parse` was not a fixed point. Found by fuzzing.
    ///
    /// First wins, which is what `object_to_payload` already does when it sorts and dedups
    /// unknown keys by encoded bytes. One rule for duplicates, everywhere.
    pub fn take(&mut self, key: &str) -> Option<Spanned<HValue>> {
        let i = self.entries.iter().position(|(k, _)| k.value == key)?;
        let first = self.entries.remove(i).1;
        self.entries.retain(|(k, _)| k.value != key);
        Some(first)
    }

    pub fn insert(&mut self, key: Spanned<String>, value: Spanned<HValue>) {
        self.entries.push((key, value));
    }

    pub fn iter(&self) -> impl Iterator<Item = &(Spanned<String>, Spanned<HValue>)> {
        self.entries.iter()
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(k, _)| k.value.as_str())
    }
}

/// A syntax error, with the byte offset it was found at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HError {
    pub at: usize,
    pub message: String,
}

impl HError {
    fn new(at: usize, message: impl Into<String>) -> HError {
        HError {
            at,
            message: message.into(),
        }
    }
}

impl fmt::Display for HError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.message, self.at)
    }
}

type Res<T> = Result<T, HError>;

/// Parse one object starting at the first `{` in `src`, whose byte offsets are reported
/// relative to `base`.
pub fn parse_object(src: &str, base: usize) -> Res<Spanned<HObject>> {
    let mut p = Parser {
        s: src.as_bytes(),
        src,
        i: 0,
        base,
    };
    p.skip_trivia();
    let o = p.object(0)?;
    p.skip_trivia();
    if p.i < p.s.len() {
        return Err(HError::new(p.base + p.i, "trailing content after object"));
    }
    Ok(o)
}

/// Parse one object at the start of `src`, leaving whatever follows it alone.
///
/// Record headers are followed by a gist line, so the object cannot be required to be the
/// whole input. The returned span says where it ended.
pub fn parse_object_prefix(src: &str, base: usize) -> Res<Spanned<HObject>> {
    let mut p = Parser {
        s: src.as_bytes(),
        src,
        i: 0,
        base,
    };
    p.skip_trivia();
    p.object(0)
}

/// Parse a bare value, for configuration files and tests.
pub fn parse_value(src: &str, base: usize) -> Res<Spanned<HValue>> {
    let mut p = Parser {
        s: src.as_bytes(),
        src,
        i: 0,
        base,
    };
    p.skip_trivia();
    let v = p.value(0)?;
    p.skip_trivia();
    if p.i < p.s.len() {
        return Err(HError::new(p.base + p.i, "trailing content after value"));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    src: &'a str,
    i: usize,
    base: usize,
}

impl<'a> Parser<'a> {
    fn span(&self, start: usize) -> Span {
        Span::new(self.base + start, self.base + self.i)
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn eof(&self, what: &str) -> HError {
        HError::new(
            self.base + self.i,
            format!("unexpected end of input in {what}"),
        )
    }

    /// Skip whitespace and comments.
    fn skip_trivia(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_ascii_whitespace() => self.i += 1,
                Some(b'#') => self.skip_line(),
                Some(b'/') if self.s.get(self.i + 1) == Some(&b'/') => self.skip_line(),
                Some(b'/') if self.s.get(self.i + 1) == Some(&b'*') => {
                    self.i += 2;
                    while self.i < self.s.len()
                        && !(self.s[self.i] == b'*' && self.s.get(self.i + 1) == Some(&b'/'))
                    {
                        self.i += 1;
                    }
                    self.i = (self.i + 2).min(self.s.len());
                }
                _ => return,
            }
        }
    }

    /// Refuse rather than descend.
    ///
    /// `object`/`array`/`value` are mutually recursive, so an unbounded depth meant a deeply
    /// nested header overflowed the stack and **aborted the process** — measured at roughly
    /// 5 000 levels. An abort cannot be caught, so an embedder could not contain it, and
    /// rule A1 promises no panics on untrusted input. A `.smy` file is untrusted by
    /// definition: it is the thing another agent hands you.
    ///
    /// The limit is shared with the CBOR reader, since both walk the same shapes and a
    /// document that survives one should survive the other.
    fn check_depth(&self, depth: usize) -> Res<()> {
        if depth > crate::cbor::MAX_NESTING {
            return Err(HError::new(
                self.base + self.i,
                format!("nesting deeper than {}", crate::cbor::MAX_NESTING),
            ));
        }
        Ok(())
    }

    fn skip_line(&mut self) {
        while self.i < self.s.len() && self.s[self.i] != b'\n' {
            self.i += 1;
        }
    }

    /// Skip spaces and comments but stop at a newline, which is a separator.
    fn skip_inline_trivia(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c == b' ' || c == b'\t' || c == b'\r' => self.i += 1,
                Some(b'#') => self.skip_line(),
                Some(b'/') if self.s.get(self.i + 1) == Some(&b'/') => self.skip_line(),
                _ => return,
            }
        }
    }

    /// `depth` is a parameter rather than a field on the parser, so it unwinds with the
    /// recursion: there is no counter to decrement on each of the `?` paths, and therefore
    /// no way to leak one.
    fn object(&mut self, depth: usize) -> Res<Spanned<HObject>> {
        self.check_depth(depth)?;
        let start = self.i;
        if self.peek() != Some(b'{') {
            return Err(HError::new(self.base + self.i, "expected `{`"));
        }
        self.i += 1;
        let mut obj = HObject::new();
        loop {
            self.skip_trivia();
            match self.peek() {
                None => return Err(self.eof("object")),
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Spanned::new(obj, self.span(start)));
                }
                Some(b',') => {
                    self.i += 1;
                    continue;
                }
                _ => {}
            }
            let key = self.key()?;
            self.skip_trivia();
            if self.peek() != Some(b':') {
                return Err(HError::new(
                    self.base + self.i,
                    format!("expected `:` after key `{}`", key.value),
                ));
            }
            self.i += 1;
            self.skip_inline_trivia();
            let value = self.value(depth + 1)?;
            obj.insert(key, value);
        }
    }

    fn array(&mut self, depth: usize) -> Res<Spanned<HValue>> {
        self.check_depth(depth)?;
        let start = self.i;
        self.i += 1; // `[`
        let mut items = Vec::new();
        loop {
            self.skip_trivia();
            match self.peek() {
                None => return Err(self.eof("array")),
                Some(b']') => {
                    self.i += 1;
                    return Ok(Spanned::new(HValue::Array(items), self.span(start)));
                }
                Some(b',') => {
                    self.i += 1;
                    continue;
                }
                _ => items.push(self.value(depth + 1)?),
            }
        }
    }

    fn key(&mut self) -> Res<Spanned<String>> {
        let start = self.i;
        if self.peek() == Some(b'"') {
            let s = self.quoted()?;
            return Ok(Spanned::new(s, self.span(start)));
        }
        while let Some(c) = self.peek() {
            if c == b':' || c.is_ascii_whitespace() || c == b',' || c == b'{' || c == b'}' {
                break;
            }
            self.i += 1;
        }
        if self.i == start {
            return Err(HError::new(self.base + self.i, "expected a key"));
        }
        Ok(Spanned::new(
            self.src[start..self.i].to_string(),
            self.span(start),
        ))
    }

    fn value(&mut self, depth: usize) -> Res<Spanned<HValue>> {
        self.skip_inline_trivia();
        let start = self.i;
        match self.peek() {
            None => Err(self.eof("value")),
            Some(b'{') => {
                let o = self.object(depth + 1)?;
                Ok(Spanned::new(HValue::Object(o.value), o.span))
            }
            Some(b'[') => self.array(depth + 1),
            Some(b'"') => {
                let s = self.quoted()?;
                Ok(Spanned::new(HValue::Str(s), self.span(start)))
            }
            _ => self.quoteless(),
        }
    }

    fn quoted(&mut self) -> Res<String> {
        self.i += 1; // opening quote
        let mut out = String::new();
        loop {
            let c = self.peek().ok_or_else(|| self.eof("string"))?;
            match c {
                b'"' => {
                    self.i += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.i += 1;
                    let e = self.peek().ok_or_else(|| self.eof("escape"))?;
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'u' => {
                            let hex = self
                                .src
                                .get(self.i..self.i + 4)
                                .ok_or_else(|| self.eof("unicode escape"))?;
                            let n = u32::from_str_radix(hex, 16).map_err(|_| {
                                HError::new(self.base + self.i, "invalid unicode escape")
                            })?;
                            out.push(char::from_u32(n).ok_or_else(|| {
                                HError::new(self.base + self.i, "invalid code point")
                            })?);
                            self.i += 4;
                        }
                        other => {
                            return Err(HError::new(
                                self.base + self.i - 1,
                                format!("unknown escape `\\{}`", other as char),
                            ))
                        }
                    }
                }
                _ => {
                    // Advance by one whole character so multi-byte text survives.
                    let rest = &self.src[self.i..];
                    let ch = rest.chars().next().ok_or_else(|| self.eof("string"))?;
                    out.push(ch);
                    self.i += ch.len_utf8();
                }
            }
        }
    }

    /// A quoteless value, terminated by `,` `}` `]` or end of line.
    fn quoteless(&mut self) -> Res<Spanned<HValue>> {
        let start = self.i;
        while let Some(c) = self.peek() {
            if matches!(c, b',' | b'}' | b']' | b'\n' | b'\r') {
                break;
            }
            self.i += 1;
        }
        let raw = self.src[start..self.i].trim_end();
        self.i = start + raw.len();
        if raw.is_empty() {
            return Err(HError::new(self.base + start, "expected a value"));
        }
        let v = classify(raw);
        Ok(Spanned::new(v, self.span(start)))
    }
}

/// Classify a quoteless token. Anything that is not a literal is a string, which is what
/// makes `default`, `en`, `c/auth-p95`, and `2026-07-09` all work without quoting.
fn classify(raw: &str) -> HValue {
    match raw {
        "true" => return HValue::Bool(true),
        "false" => return HValue::Bool(false),
        "null" => return HValue::Null,
        _ => {}
    }
    if let Ok(i) = raw.parse::<i64>() {
        return HValue::Int(i);
    }
    // A float must look like a number, not like a version or a date.
    if raw.parse::<f64>().is_ok()
        && raw
            .bytes()
            .all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'-' | b'+' | b'e' | b'E'))
    {
        if let Ok(f) = raw.parse::<f64>() {
            return HValue::Float(f);
        }
    }
    HValue::Str(raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(src: &str) -> HObject {
        parse_object(src, 0).unwrap().value
    }

    #[test]
    fn parses_the_rfc_doc_header() {
        let o = obj(r#"{
  intent: incident-brief, lang: en
  requires: ["smysl.kernel/0.1", "x.sre/1"]
  granularity: { profile: default }
  roots: [c/auth-p95-regression]
}"#);
        assert_eq!(
            o.get("intent").unwrap().value.as_str(),
            Some("incident-brief")
        );
        assert_eq!(o.get("lang").unwrap().value.as_str(), Some("en"));
        assert_eq!(
            o.get("requires").unwrap().value.as_array().unwrap().len(),
            2
        );
        assert_eq!(
            o.get("granularity")
                .unwrap()
                .value
                .as_object()
                .unwrap()
                .get("profile")
                .unwrap()
                .value
                .as_str(),
            Some("default")
        );
        assert_eq!(
            o.get("roots").unwrap().value.as_array().unwrap()[0]
                .value
                .as_str(),
            Some("c/auth-p95-regression")
        );
    }

    #[test]
    fn parses_the_rfc_unit_header() {
        let o = obj(r#"{ status: measured, grounds: [e/trace-jul], deps: [d/p95] }"#);
        assert_eq!(o.get("status").unwrap().value.as_str(), Some("measured"));
        assert_eq!(o.get("grounds").unwrap().value.as_array().unwrap().len(), 1);
        assert_eq!(o.len(), 3);
    }

    #[test]
    fn parses_a_nested_source_header() {
        let o = obj(r#"{ status: measured
                 source: { kind: metric, ref: "grafana://board/12/panel/4", captured: 2026-07-09 } }"#);
        let s = o.get("source").unwrap().value.as_object().unwrap();
        assert_eq!(s.get("kind").unwrap().value.as_str(), Some("metric"));
        assert_eq!(
            s.get("ref").unwrap().value.as_str(),
            Some("grafana://board/12/panel/4")
        );
        assert_eq!(
            s.get("captured").unwrap().value.as_str(),
            Some("2026-07-09")
        );
    }

    #[test]
    fn an_empty_object_is_valid() {
        assert!(obj("{}").is_empty());
        assert!(obj("{ }").is_empty());
        assert!(obj("{\n}").is_empty());
    }

    #[test]
    fn newlines_separate_entries_without_commas() {
        let o = obj("{\n a: 1\n b: 2\n}");
        assert_eq!(o.len(), 2);
        assert_eq!(o.get("b").unwrap().value.as_int(), Some(2));
    }

    #[test]
    fn trailing_commas_are_tolerated() {
        assert_eq!(obj("{ a: 1, b: 2, }").len(), 2);
        assert_eq!(
            obj("{ a: [1, 2,] }")
                .get("a")
                .unwrap()
                .value
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn numbers_and_literals_are_typed() {
        let o = obj("{ i: 30, f: 0.6, neg: -2, t: true, f2: false, n: null }");
        assert_eq!(o.get("i").unwrap().value, HValue::Int(30));
        assert_eq!(o.get("f").unwrap().value, HValue::Float(0.6));
        assert_eq!(o.get("neg").unwrap().value, HValue::Int(-2));
        assert_eq!(o.get("t").unwrap().value, HValue::Bool(true));
        assert_eq!(o.get("f2").unwrap().value, HValue::Bool(false));
        assert_eq!(o.get("n").unwrap().value, HValue::Null);
    }

    /// A date and a version look numeric but are not numbers. Classifying them as floats
    /// would silently rewrite `2026-07-09` and `0.1`.
    #[test]
    fn dates_and_versions_stay_strings() {
        let o = obj("{ d: 2026-07-09, v: smysl/0.1, k: x.sre/1 }");
        assert_eq!(o.get("d").unwrap().value.as_str(), Some("2026-07-09"));
        assert_eq!(o.get("v").unwrap().value.as_str(), Some("smysl/0.1"));
        assert_eq!(o.get("k").unwrap().value.as_str(), Some("x.sre/1"));
    }

    #[test]
    fn quoted_strings_handle_escapes() {
        let o = obj(r#"{ s: "a \"b\" c\nd\te\\fé" }"#);
        assert_eq!(
            o.get("s").unwrap().value.as_str(),
            Some("a \"b\" c\nd\te\\f\u{e9}")
        );
    }

    #[test]
    fn quoted_strings_carry_multibyte_text() {
        let o = obj("{ s: \"caf\u{e9} \u{4f60}\u{597d}\" }");
        assert_eq!(
            o.get("s").unwrap().value.as_str(),
            Some("caf\u{e9} \u{4f60}\u{597d}")
        );
    }

    #[test]
    fn comments_are_skipped_between_entries() {
        let o = obj("{\n # a comment\n a: 1\n /* block\n comment */ b: 2\n}");
        assert_eq!(o.len(), 2);
        assert_eq!(o.get("a").unwrap().value.as_int(), Some(1));
        assert_eq!(o.get("b").unwrap().value.as_int(), Some(2));
    }

    /// A quoteless value runs to end of line, comment markers included. That is HJSON's
    /// rule and it is the right one here: without it, `ref: grafana://board/12` would
    /// silently truncate at the `//`.
    #[test]
    fn a_quoteless_value_swallows_comment_markers() {
        let o = obj("{\n url: grafana://board/12/panel/4\n n: 1 // not a comment\n}");
        assert_eq!(
            o.get("url").unwrap().value.as_str(),
            Some("grafana://board/12/panel/4")
        );
        assert_eq!(
            o.get("n").unwrap().value.as_str(),
            Some("1 // not a comment"),
            "quote the value or put the comment on its own line"
        );
    }

    #[test]
    fn entry_order_is_preserved() {
        let o = obj("{ z: 1, a: 2, m: 3 }");
        assert_eq!(o.keys().collect::<Vec<_>>(), ["z", "a", "m"]);
    }

    /// `take` is how header parsing separates known keys from unknown ones: whatever is
    /// left after the known keys are taken belongs in `payload` (rule X).
    #[test]
    fn take_removes_and_leaves_the_rest() {
        let mut o = obj("{ status: measured, sre_severity: 2, custom: yes }");
        assert_eq!(o.take("status").unwrap().value.as_str(), Some("measured"));
        assert!(o.take("status").is_none());
        assert_eq!(o.keys().collect::<Vec<_>>(), ["sre_severity", "custom"]);
    }

    #[test]
    fn spans_point_at_the_source() {
        let src = "{ status: measured }";
        let o = parse_object(src, 0).unwrap();
        assert_eq!(o.span, Span::new(0, src.len()));
        let v = o.value.get("status").unwrap();
        assert_eq!(v.span.slice(src), Some("measured"));
    }

    #[test]
    fn spans_are_offset_by_the_base() {
        let src = "{ a: 1 }";
        let o = parse_object(src, 100).unwrap();
        assert_eq!(o.span, Span::new(100, 108));
        assert_eq!(o.value.get("a").unwrap().span, Span::new(105, 106));
    }

    #[test]
    fn nested_spans_cover_their_own_extent() {
        let src = "{ g: { profile: fine } }";
        let o = parse_object(src, 0).unwrap();
        let g = o.value.get("g").unwrap();
        assert_eq!(g.span.slice(src), Some("{ profile: fine }"));
    }

    #[test]
    fn a_prefix_parse_leaves_the_rest_alone() {
        let src = "{ a: 1 }\n~ a gist\n";
        let o = parse_object_prefix(src, 0).unwrap();
        assert_eq!(o.span, Span::new(0, 8));
        assert_eq!(o.value.len(), 1);
        assert!(
            parse_object(src, 0).is_err(),
            "the strict form rejects the tail"
        );
    }

    #[test]
    fn errors_report_a_byte_offset() {
        let e = parse_object("{ a 1 }", 0).unwrap_err();
        assert!(e.message.contains("expected `:`"));
        assert!(e.at > 0);

        let e = parse_object("{ a: 1", 0).unwrap_err();
        assert!(e.message.contains("end of input"));

        assert!(parse_object("not an object", 0).is_err());
        assert!(parse_object("{ } trailing", 0).is_err());
    }

    #[test]
    fn error_offsets_are_absolute() {
        let e = parse_object("{ a 1 }", 500).unwrap_err();
        assert!(e.at >= 500, "offset {} is not absolute", e.at);
    }

    #[test]
    fn unterminated_strings_and_bad_escapes_are_errors() {
        assert!(parse_object(r#"{ s: "abc }"#, 0).is_err());
        assert!(parse_object(r#"{ s: "a\qb" }"#, 0).is_err());
        assert!(parse_object(r#"{ s: "a\u00" }"#, 0).is_err());
    }

    #[test]
    fn arrays_nest() {
        let v = parse_value("[[1, 2], [3]]", 0).unwrap().value;
        let outer = v.as_array().unwrap();
        assert_eq!(outer.len(), 2);
        assert_eq!(outer[0].value.as_array().unwrap().len(), 2);
    }

    #[test]
    fn value_accessors_are_type_checked() {
        let v = HValue::Int(3);
        assert_eq!(v.as_int(), Some(3));
        assert_eq!(v.as_f64(), Some(3.0));
        assert_eq!(v.as_str(), None);
        assert_eq!(v.as_bool(), None);
        assert_eq!(v.type_name(), "integer");
        assert_eq!(HValue::Null.type_name(), "null");
    }
}
