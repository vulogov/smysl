//! RFC 8785 (JCS) canonical JSON, with one exception: numbers are `dec` spellings (§5.5).
//!
//! A leaf that is an unkeyed array or an opaque object is diffed as one value of `vtype json`,
//! so its text has to be one string per value. JCS gives that for everything but numbers:
//! members sorted by the UTF-16 code units of their keys, no whitespace, and strings escaped
//! the way ECMAScript's `JSON.stringify` escapes them. JCS writes a number as the binary64 it
//! parses to, which turns `18446744073709551615` into `18446744073709552000` [RFC SMYSL-3 A §5];
//! here a number is its `dec` spelling instead, so `41` and `41.0` are one value inside an
//! array as they are outside one.
//!
//! A number outside `dec`'s range has no `dec` spelling. It is written as its token, verbatim,
//! and the caller is told, because that leaf is no longer canonical across spellings and a
//! change unit says so (`SMY-W613`).

use smysl_core::Decimal;

use crate::json::Json;

/// The canonical text, and whether any number in it was out of `dec`'s range and written as
/// its token.
pub fn canonical(v: &Json) -> (String, bool) {
    let mut out = String::new();
    let mut verbatim = false;
    write(&mut out, v, &mut verbatim);
    (out, verbatim)
}

fn write(out: &mut String, v: &Json, verbatim: &mut bool) {
    match v {
        Json::Null => out.push_str("null"),
        Json::Bool(true) => out.push_str("true"),
        Json::Bool(false) => out.push_str("false"),
        Json::Number(t) => match Decimal::parse_token(t) {
            Ok(d) => out.push_str(&d.to_string()),
            Err(_) => {
                *verbatim = true;
                out.push_str(t);
            }
        },
        Json::String(s) => string(out, s),
        Json::Array(items) => {
            out.push('[');
            for (n, i) in items.iter().enumerate() {
                if n > 0 {
                    out.push(',');
                }
                write(out, i, verbatim);
            }
            out.push(']');
        }
        Json::Object(members) => {
            let mut sorted: Vec<&(String, Json)> = members.iter().collect();
            // RFC 8785 §3.2.3: by the keys' UTF-16 code units, not their UTF-8 bytes. The two
            // orders differ for a key with a character above U+FFFF beside one in
            // U+E000–U+FFFF.
            sorted.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
            out.push('{');
            for (n, (k, v)) in sorted.into_iter().enumerate() {
                if n > 0 {
                    out.push(',');
                }
                string(out, k);
                out.push(':');
                write(out, v, verbatim);
            }
            out.push('}');
        }
    }
}

/// A string as ECMAScript's `JSON.stringify` writes it (RFC 8785 §3.2.2.2).
pub fn string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::parse;

    fn c(text: &str) -> String {
        canonical(&parse(text).unwrap()).0
    }

    #[test]
    fn members_sort_and_whitespace_goes() {
        assert_eq!(
            c(r#"{ "b": 1, "a": [ true , null ] }"#),
            r#"{"a":[true,null],"b":1}"#
        );
    }

    #[test]
    fn numbers_are_dec_spellings() {
        assert_eq!(
            c("[41, 41.0, 4.10e1, -0, 1e3, 0.50]"),
            "[41,41,41,0,1000,0.5]"
        );
        // Where JCS would write the binary64, 18446744073709552000.
        assert_eq!(c("[18446744073709551615]"), "[18446744073709551615]");
    }

    #[test]
    fn a_number_out_of_range_is_its_token_and_says_so() {
        let (text, verbatim) = canonical(&parse("[1e999]").unwrap());
        assert_eq!(text, "[1e999]");
        assert!(verbatim);
        assert!(!canonical(&parse("[1]").unwrap()).1);
    }

    #[test]
    fn strings_escape_as_json_stringify_does() {
        assert_eq!(
            c(r#""a\"b\\c\/d\u0001\u001f\n\t""#),
            r#""a\"b\\c/d\u0001\u001f\n\t""#
        );
        assert_eq!(c(r#"" é😀""#), "\"\u{2028}é😀\"");
    }

    /// RFC 8785 §3.2.3's point: UTF-16 order, in which U+1F600 (a surrogate pair, 0xD83D…) sorts
    /// before U+FFFD, while in UTF-8 or code-point order it sorts after.
    #[test]
    fn keys_sort_by_utf_16_code_units() {
        assert_eq!(
            c("{\"\u{fffd}\": 1, \"\u{1f600}\": 2}"),
            "{\"\u{1f600}\":2,\"\u{fffd}\":1}"
        );
    }
}
