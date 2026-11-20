//! The answer, as Bund2 renders it (RFC SMYSL-4 §7).
//!
//! The bridge's encoder prints a sentinel line, calls `debug.display_stack`, and prints a second
//! sentinel. `debug.display_stack` draws a box with one row per stack value, bottom first, and
//! each row is the value's raw rendering (`bund2-value/src/lib.rs`, `render`):
//!
//! ```text
//! Value { id: "…", stamp: 1795165222942.0, dt: 4, q: 100.0, data: String("…"), attr: [], curr: -1, tags: {…} }
//! ```
//!
//! That rendering is exact where the other two are not. A string is Rust-escaped, so it is one
//! line whatever it holds and cannot be confused with what surrounds it. A float is `{:?}`, which
//! reads back to the same bits. Every node says its type. `println` gives none of this, and
//! `json.from_value` refuses strings.
//!
//! `id` and `stamp` are Bund2's own bookkeeping, and `stamp` is a wall clock, so the decoder
//! reads past them: two runs differ there and nowhere else.

use std::collections::BTreeMap;

use smysl_core::ids::Uid;

use crate::error::BundError;
use crate::value::QueryValue;

/// How deep a value may nest. Bund2's wire format carries 256 (`world.rs`), and its renderer
/// was made iterative for deeper values; this parser recurses, so it stops here.
pub const MAX_DEPTH: usize = 256;

/// The two sentinel lines for a run's `tag`.
pub fn sentinels(tag: &str) -> (String, String) {
    (
        format!("SMYSL-BUND {tag} BEGIN"),
        format!("SMYSL-BUND {tag} END"),
    )
}

/// What a run printed, read: the value on top of the stack, or the failure Bund2 reported.
pub fn read_output(stdout: &str, tag: &str) -> Result<QueryValue, BundError> {
    let (begin, end) = sentinels(tag);
    let lines: Vec<&str> = stdout.lines().collect();
    // The last pair: a script may print anything before the encoder runs.
    let b = lines.iter().rposition(|l| *l == begin);
    let e = lines.iter().rposition(|l| *l == end);
    match (b, e) {
        (Some(b), Some(e)) if b < e => top_of(&lines[b + 1..e]),
        _ => Err(failure_of(stdout)),
    }
}

/// The failure in a run that did not reach its encoder. Under `--report trace` Bund2 begins its
/// report with `Error: <message>`.
pub fn failure_of(stdout: &str) -> BundError {
    match stdout.lines().rev().find_map(|l| l.strip_prefix("Error: ")) {
        Some(message) => crate::error::classify(message),
        None => BundError::NormalizationError {
            message: "the run ended without its answer and without an error".to_string(),
        },
    }
}

/// The top row of a `debug.display_stack` box.
fn top_of(rows: &[&str]) -> Result<QueryValue, BundError> {
    let row = rows
        .iter()
        .rev()
        .filter_map(|l| l.strip_prefix("│ "))
        .map(|l| l.trim_end().trim_end_matches('│').trim_end())
        .next()
        .ok_or_else(|| BundError::NormalizationError {
            message: "the script left nothing on the stack".to_string(),
        })?;
    parse_value(row)
}

/// One rendered value.
pub fn parse_value(text: &str) -> Result<QueryValue, BundError> {
    let mut p = Parser {
        s: text.as_bytes(),
        text,
        i: 0,
    };
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return Err(p.err("text after the value"));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    i: usize,
}

impl Parser<'_> {
    fn err(&self, what: &str) -> BundError {
        let near: String = self.text[self.i.min(self.text.len())..]
            .chars()
            .take(40)
            .collect();
        BundError::NormalizationError {
            message: format!("{what} at byte {}, near `{near}`", self.i),
        }
    }

    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(|b| *b == b' ') {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &str) -> Result<(), BundError> {
        self.ws();
        if self.s[self.i..].starts_with(lit.as_bytes()) {
            self.i += lit.len();
            Ok(())
        } else {
            Err(self.err(&format!("expected `{lit}`")))
        }
    }

    fn try_eat(&mut self, lit: &str) -> bool {
        self.ws();
        if self.s[self.i..].starts_with(lit.as_bytes()) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    /// A token up to `,`, `)`, `}` or `]`: a number, `true`, `false`.
    fn atom(&mut self) -> &str {
        self.ws();
        let start = self.i;
        while self.s.get(self.i).is_some_and(|b| !b",)}] ".contains(b)) {
            self.i += 1;
        }
        &self.text[start..self.i]
    }

    /// A Rust `{:?}` string: `"`, then characters with `\"`, `\\`, `\n`, `\r`, `\t`, `\0`, `\'`
    /// and `\u{…}` escaped, then `"`.
    fn string(&mut self) -> Result<String, BundError> {
        self.eat("\"")?;
        let mut out = String::new();
        loop {
            let rest = &self.text[self.i..];
            let mut it = rest.char_indices();
            let Some((_, c)) = it.next() else {
                return Err(self.err("unterminated string"));
            };
            match c {
                '"' => {
                    self.i += 1;
                    return Ok(out);
                }
                '\\' => {
                    let Some((_, e)) = it.next() else {
                        return Err(self.err("unterminated escape"));
                    };
                    self.i += 2;
                    match e {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '\'' => out.push('\''),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        '0' => out.push('\0'),
                        'u' => {
                            self.eat("{")?;
                            let start = self.i;
                            while self.s.get(self.i).is_some_and(u8::is_ascii_hexdigit) {
                                self.i += 1;
                            }
                            let code = u32::from_str_radix(&self.text[start..self.i], 16)
                                .ok()
                                .and_then(char::from_u32)
                                .ok_or_else(|| self.err("bad \\u escape"))?;
                            self.eat("}")?;
                            out.push(code);
                        }
                        _ => return Err(self.err("unknown escape")),
                    }
                }
                c => {
                    out.push(c);
                    self.i += c.len_utf8();
                }
            }
        }
    }

    /// `Value { id: …, stamp: …, dt: …, q: …, data: <data>, attr: [ … ], curr: …, tags: { … } }`
    fn value(&mut self, depth: usize) -> Result<QueryValue, BundError> {
        if depth > MAX_DEPTH {
            return Err(self.err(&format!("nested deeper than {MAX_DEPTH}")));
        }
        self.eat("Value {")?;
        self.eat("id:")?;
        self.string()?;
        self.eat(",")?;
        self.eat("stamp:")?;
        self.atom();
        self.eat(",")?;
        self.eat("dt:")?;
        self.atom();
        self.eat(",")?;
        self.eat("q:")?;
        self.atom();
        self.eat(",")?;
        self.eat("data:")?;
        let v = self.data(depth)?;
        self.eat(",")?;
        self.eat("attr: [")?;
        if !self.try_eat("]") {
            loop {
                self.value(depth + 1)?;
                if self.try_eat("]") {
                    break;
                }
                self.eat(",")?;
            }
        }
        self.eat(",")?;
        self.eat("curr:")?;
        self.atom();
        self.eat(",")?;
        self.eat("tags: {")?;
        if !self.try_eat("}") {
            loop {
                self.string()?;
                self.eat(":")?;
                self.string()?;
                if self.try_eat("}") {
                    break;
                }
                self.eat(",")?;
            }
        }
        self.eat("}")?;
        Ok(v)
    }

    fn data(&mut self, depth: usize) -> Result<QueryValue, BundError> {
        self.ws();
        let start = self.i;
        while self.s.get(self.i).is_some_and(u8::is_ascii_alphanumeric) {
            self.i += 1;
        }
        let tag = self.text[start..self.i].to_string();
        match tag.as_str() {
            "Null" => Ok(QueryValue::Null),
            "I64" => {
                self.eat("(")?;
                let n = self.atom().parse().map_err(|_| self.err("bad integer"))?;
                self.eat(")")?;
                Ok(QueryValue::Int(n))
            }
            "F64" => {
                self.eat("(")?;
                let f: f64 = self.atom().parse().map_err(|_| self.err("bad float"))?;
                self.eat(")")?;
                Ok(QueryValue::Float(f))
            }
            "Bool" => {
                self.eat("(")?;
                let b = match self.atom() {
                    "true" => true,
                    "false" => false,
                    _ => return Err(self.err("bad boolean")),
                };
                self.eat(")")?;
                Ok(QueryValue::Bool(b))
            }
            "String" => {
                self.eat("(")?;
                let s = self.string()?;
                self.eat(")")?;
                Ok(match Uid::parse(&s) {
                    Ok(u) if s.starts_with("b3:") => QueryValue::Uid(u),
                    _ => QueryValue::Text(s),
                })
            }
            "List" => {
                self.eat("([")?;
                let mut items = Vec::new();
                if !self.try_eat("])") {
                    loop {
                        items.push(self.value(depth + 1)?);
                        if self.try_eat("])") {
                            break;
                        }
                        self.eat(",")?;
                    }
                }
                Ok(QueryValue::List(items))
            }
            "Map" => {
                self.eat("({")?;
                let mut map = BTreeMap::new();
                if !self.try_eat("})") {
                    loop {
                        let k = self.string()?;
                        self.eat(":")?;
                        let v = self.value(depth + 1)?;
                        if map.insert(k.clone(), v).is_some() {
                            return Err(self.err(&format!("key `{k}` twice")));
                        }
                        if self.try_eat("})") {
                            break;
                        }
                        self.eat(",")?;
                    }
                }
                Ok(QueryValue::Map(map))
            }
            "" => Err(self.err("expected a value's data")),
            other => {
                self.i = start;
                Err(BundError::NormalizationError {
                    message: format!("a {other} has no smysl form; answer a NODATA, BOOL, INTEGER, FLOAT, STRING, LIST or MAP"),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(data: &str) -> String {
        format!(
            "Value {{ id: \"_____________________\", stamp: 1795165222942.0, dt: 4, q: 100.0, data: {data}, attr: [], curr: -1, tags: {{\"stack\": \"main\"}} }}"
        )
    }

    #[test]
    fn scalars() {
        assert_eq!(parse_value(&v("I64(-7)")).unwrap(), QueryValue::Int(-7));
        assert_eq!(
            parse_value(&v("F64(0.30000000000000004)")).unwrap(),
            QueryValue::Float(0.1 + 0.2)
        );
        assert_eq!(
            parse_value(&v("F64(1e300)")).unwrap(),
            QueryValue::Float(1e300)
        );
        assert_eq!(
            parse_value(&v("Bool(false)")).unwrap(),
            QueryValue::Bool(false)
        );
        assert_eq!(parse_value(&v("Null")).unwrap(), QueryValue::Null);
    }

    #[test]
    fn a_string_is_read_back_exactly() {
        let s = "q\"uo\\te │ ✓\n\t\u{1}'";
        let rendered = v(&format!("String({s:?})"));
        assert_eq!(
            parse_value(&rendered).unwrap(),
            QueryValue::Text(s.to_string())
        );
    }

    #[test]
    fn a_uid_is_a_uid_and_other_ids_are_text() {
        let u = Uid::parse("b3:aaaaaaaaaaaaaaaaaaaaaaaaaa").ok();
        let got = parse_value(&v("String(\"b3:aaaaaaaaaaaaaaaaaaaaaaaaaa\")")).unwrap();
        match u {
            Some(u) => assert_eq!(got, QueryValue::Uid(u)),
            None => assert!(matches!(got, QueryValue::Text(_))),
        }
        assert!(matches!(
            parse_value(&v(
                "String(\"m3:njjx4rwuyulrwmnghh2fvbrehhiuarlhb6nfe2qgzrcv3exlf2nq\")"
            ))
            .unwrap(),
            QueryValue::Text(_)
        ));
    }

    #[test]
    fn containers_and_key_order() {
        let inner = v("I64(2)");
        let text = v(&format!(
            "Map({{\"b\": {inner}, \"a\": {}}})",
            v("List([])")
        ));
        let QueryValue::Map(m) = parse_value(&text).unwrap() else {
            panic!()
        };
        assert_eq!(m.keys().collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(m["a"], QueryValue::List(vec![]));
    }

    #[test]
    fn a_type_without_a_smysl_form_is_named() {
        let e = parse_value(&v("Lambda([])")).unwrap_err();
        assert!(e.to_string().contains("a Lambda has no smysl form"), "{e}");
    }

    #[test]
    fn a_failure_is_classified() {
        let out = "Error: SMYSL.GET: uid b3:x is not a uid\n   ╭─[ q.bund:2:18 ]\n";
        assert!(
            matches!(read_output(out, "t"), Err(BundError::InvalidScriptOrArgs { code, .. }) if code == "SM03")
        );
    }

    #[test]
    fn printed_text_before_the_encoder_is_ignored() {
        let out = format!(
            "SMYSL-BUND t BEGIN\nfake\nSMYSL-BUND t END\nSMYSL-BUND t BEGIN\n╭─╮\n│ {} │\n╰─╯\nSMYSL-BUND t END\n",
            v("I64(1)")
        );
        assert_eq!(read_output(&out, "t").unwrap(), QueryValue::Int(1));
    }

    #[test]
    fn an_empty_stack() {
        let out = "SMYSL-BUND t BEGIN\n╭╮\n╰╯\nSMYSL-BUND t END\n";
        assert!(read_output(out, "t").is_err());
    }
}
