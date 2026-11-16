//! JSON, read exactly (RFC 8259), with every number kept as the token it was written as.
//!
//! **Why not `serde_json`.** Its `Number` is an `f64`, an `i64` or a `u64`, so `0.1`, `41.0`
//! and `18446744073709551616` are already something other than what the snapshot said by the
//! time a caller sees them. Its `arbitrary_precision` feature keeps tokens, but a Cargo feature
//! is unified across the workspace, so turning it on here would change how every other crate's
//! JSON parses. A reader that never converts is a hundred and fifty lines, so it is this.
//!
//! **Strict.** A duplicated key in one object is refused: RFC 8259 leaves it undefined, and
//! two readers picking different members would give one snapshot two diffs. So is anything
//! after the value, a trailing comma, a lone surrogate, and nesting deeper than
//! [`MAX_DEPTH`].
//!
//! **Strings are NFC**, keys included, because both end up as text a uid is computed over: a
//! value as a change's `tel:from`/`tel:to`, a key as a segment of its pointer. Two snapshots of
//! one object written in different normalisation forms are one object.

use core::fmt;

use unicode_normalization::UnicodeNormalization;

/// The deepest nesting accepted, as the kernel's CBOR reader's (§3 constraint 9).
pub const MAX_DEPTH: usize = 128;

/// A JSON value. Objects keep their members in document order; nothing here depends on that
/// order, and the canonical forms ([`crate::jcs`], [`crate::leaf`]) sort.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Json {
    Null,
    Bool(bool),
    /// The number exactly as written: `41.0` stays `41.0`.
    Number(String),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    /// The member named `key`, if this is an object that has one.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }
}

/// Why text is not JSON, and the byte offset where that became clear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonError {
    pub at: usize,
    pub why: &'static str,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not JSON at byte {}: {}", self.at, self.why)
    }
}

impl std::error::Error for JsonError {}

/// Parse one JSON text: a value, with whitespace around it and nothing else.
pub fn parse(text: &str) -> Result<Json, JsonError> {
    let mut p = Parser {
        b: text.as_bytes(),
        src: text,
        i: 0,
    };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.b.len() {
        return Err(p.err("text after the value"));
    }
    Ok(v)
}

/// Parse a sequence of JSON texts, one per non-empty line: a watch stream (`kubectl get -w -o
/// json` writes one event per line).
pub fn parse_lines(text: &str) -> Result<Vec<Json>, JsonError> {
    let mut out = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if !line.trim().is_empty() {
            out.push(parse(line).map_err(|e| JsonError {
                at: offset + e.at,
                why: e.why,
            })?);
        }
        offset += line.len();
    }
    Ok(out)
}

struct Parser<'a> {
    b: &'a [u8],
    src: &'a str,
    i: usize,
}

impl Parser<'_> {
    fn err(&self, why: &'static str) -> JsonError {
        JsonError { at: self.i, why }
    }

    fn ws(&mut self) {
        while self
            .b
            .get(self.i)
            .is_some_and(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r'))
        {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &str) -> Result<(), JsonError> {
        if self.b[self.i..].starts_with(lit.as_bytes()) {
            self.i += lit.len();
            Ok(())
        } else {
            Err(self.err("not a JSON value"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, JsonError> {
        if depth > MAX_DEPTH {
            return Err(self.err("nested too deeply"));
        }
        match self.b.get(self.i) {
            None => Err(self.err("the text ended where a value was expected")),
            Some(b'n') => self.eat("null").map(|_| Json::Null),
            Some(b't') => self.eat("true").map(|_| Json::Bool(true)),
            Some(b'f') => self.eat("false").map(|_| Json::Bool(false)),
            Some(b'"') => self.string().map(Json::String),
            Some(b'[') => self.array(depth),
            Some(b'{') => self.object(depth),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("not a JSON value")),
        }
    }

    /// RFC 8259 §6's grammar, checked and kept verbatim.
    fn number(&mut self) -> Result<Json, JsonError> {
        let start = self.i;
        if self.b[self.i] == b'-' {
            self.i += 1;
        }
        match self.b.get(self.i) {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => self.digits(),
            _ => return Err(self.err("a number needs a digit")),
        }
        if self.b.get(self.i) == Some(&b'.') {
            self.i += 1;
            if !self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                return Err(self.err("a fraction needs a digit"));
            }
            self.digits();
        }
        if matches!(self.b.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.b.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if !self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                return Err(self.err("an exponent needs a digit"));
            }
            self.digits();
        }
        Ok(Json::Number(self.src[start..self.i].to_string()))
    }

    fn digits(&mut self) {
        while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
            self.i += 1;
        }
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let h = self
            .src
            .get(self.i..self.i + 4)
            .ok_or_else(|| self.err("a \\u escape needs four hex digits"))?;
        let v = u32::from_str_radix(h, 16).map_err(|_| self.err("not a hex digit"))?;
        if !h.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(self.err("not a hex digit"));
        }
        self.i += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.i += 1; // the opening quote
        let mut out = String::new();
        loop {
            let run = self.i;
            while self
                .b
                .get(self.i)
                .is_some_and(|&c| c != b'"' && c != b'\\' && c >= 0x20)
            {
                self.i += 1;
            }
            out.push_str(&self.src[run..self.i]);
            match self.b.get(self.i) {
                None => return Err(self.err("an unterminated string")),
                Some(b'"') => {
                    self.i += 1;
                    break;
                }
                Some(b'\\') => {
                    self.i += 1;
                    let c = *self
                        .b
                        .get(self.i)
                        .ok_or_else(|| self.err("an unterminated escape"))?;
                    self.i += 1;
                    match c {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let cp = if (0xD800..0xDC00).contains(&hi) {
                                if !self.b[self.i..].starts_with(b"\\u") {
                                    return Err(self.err("a lone high surrogate"));
                                }
                                self.i += 2;
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return Err(self.err("a high surrogate without its low half"));
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else if (0xDC00..0xE000).contains(&hi) {
                                return Err(self.err("a lone low surrogate"));
                            } else {
                                hi
                            };
                            out.push(
                                char::from_u32(cp).ok_or_else(|| self.err("not a scalar value"))?,
                            );
                        }
                        _ => return Err(self.err("an unknown escape")),
                    }
                }
                Some(_) => return Err(self.err("a control character inside a string")),
            }
        }
        Ok(nfc(out))
    }

    fn array(&mut self, depth: usize) -> Result<Json, JsonError> {
        self.i += 1;
        let mut items = Vec::new();
        self.ws();
        if self.b.get(self.i) == Some(&b']') {
            self.i += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.ws();
            items.push(self.value(depth + 1)?);
            self.ws();
            match self.b.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.err("expected `,` or `]`")),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, JsonError> {
        self.i += 1;
        let mut members: Vec<(String, Json)> = Vec::new();
        self.ws();
        if self.b.get(self.i) == Some(&b'}') {
            self.i += 1;
            return Ok(Json::Object(members));
        }
        loop {
            self.ws();
            if self.b.get(self.i) != Some(&b'"') {
                return Err(self.err("an object key must be a string"));
            }
            let at = self.i;
            let key = self.string()?;
            if members.iter().any(|(k, _)| *k == key) {
                return Err(JsonError {
                    at,
                    why: "a key repeated in one object",
                });
            }
            self.ws();
            if self.b.get(self.i) != Some(&b':') {
                return Err(self.err("expected `:`"));
            }
            self.i += 1;
            self.ws();
            let v = self.value(depth + 1)?;
            members.push((key, v));
            self.ws();
            match self.b.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Json::Object(members));
                }
                _ => return Err(self.err("expected `,` or `}`")),
            }
        }
    }
}

fn nfc(s: String) -> String {
    if unicode_normalization::is_nfc(&s) {
        s
    } else {
        s.nfc().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_kept_as_written() {
        let v = parse(r#"[41, 41.0, 4.10e1, -0, 18446744073709551616, 1E+400]"#).unwrap();
        let Json::Array(items) = v else { panic!() };
        let tokens: Vec<&str> = items
            .iter()
            .map(|i| match i {
                Json::Number(t) => t.as_str(),
                _ => panic!(),
            })
            .collect();
        assert_eq!(
            tokens,
            [
                "41",
                "41.0",
                "4.10e1",
                "-0",
                "18446744073709551616",
                "1E+400"
            ]
        );
    }

    #[test]
    fn rfc_8259_is_the_grammar() {
        for bad in [
            "01",
            "1.",
            ".5",
            "+1",
            "1e",
            "-",
            "[1,]",
            "{\"a\":1,}",
            "{a:1}",
            "'x'",
            "[1 2]",
            "\"\\x\"",
            "\"a\tb\"",
            "nul",
            "1 2",
            "",
            "\"\\ud800\"",
            "\"\\udc00\"",
            "NaN",
            "Infinity",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            parse(r#" {"a": [true, false, null, "x\u00e9\n"]} "#).unwrap(),
            Json::Object(vec![(
                "a".into(),
                Json::Array(vec![
                    Json::Bool(true),
                    Json::Bool(false),
                    Json::Null,
                    Json::String("xé\n".into()),
                ])
            )])
        );
        assert_eq!(
            parse(r#""\ud83d\ude00""#).unwrap(),
            Json::String("😀".into())
        );
    }

    #[test]
    fn a_repeated_key_is_refused() {
        let e = parse(r#"{"a": 1, "a": 2}"#).unwrap_err();
        assert_eq!(e.why, "a key repeated in one object");
        // After NFC, which is the case a byte comparison would miss.
        assert!(parse("{\"e\u{301}\": 1, \"\u{e9}\": 2}").is_err());
    }

    #[test]
    fn strings_and_keys_are_nfc() {
        let v = parse("{\"cafe\u{301}\": \"cafe\u{301}\"}").unwrap();
        assert_eq!(
            v,
            Json::Object(vec![("café".into(), Json::String("café".into()))])
        );
    }

    #[test]
    fn nesting_is_bounded() {
        let deep = format!("{}{}", "[".repeat(MAX_DEPTH + 2), "]".repeat(MAX_DEPTH + 2));
        assert_eq!(parse(&deep).unwrap_err().why, "nested too deeply");
        let ok = format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
        assert!(parse(&ok).is_ok());
    }

    #[test]
    fn a_watch_stream_is_one_value_per_line() {
        let vs = parse_lines("{\"type\":\"ADDED\"}\n\n{\"type\":\"MODIFIED\"}\n").unwrap();
        assert_eq!(vs.len(), 2);
        let e = parse_lines("{}\n{bad}\n").unwrap_err();
        assert_eq!(e.at, 4, "the offset is into the whole stream");
    }
}
