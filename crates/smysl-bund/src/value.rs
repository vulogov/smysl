//! What a script answers.

use std::collections::BTreeMap;

use smysl_core::ids::Uid;

/// A Bund2 value, as the bridge returns it.
///
/// Map keys are in `BTreeMap` order, whatever order Bund2 held them in, so two equal answers
/// compare equal. A STRING that parses as a smysl uid is returned as [`QueryValue::Uid`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum QueryValue {
    /// Bund2's NODATA.
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Uid(Uid),
    List(Vec<QueryValue>),
    Map(BTreeMap<String, QueryValue>),
}

impl QueryValue {
    /// Every uid in this value, depth first, in the order they appear.
    pub fn uids(&self) -> Vec<Uid> {
        let mut out = Vec::new();
        let mut work = vec![self];
        while let Some(v) = work.pop() {
            match v {
                QueryValue::Uid(u) => out.push(*u),
                QueryValue::List(items) => work.extend(items.iter().rev()),
                QueryValue::Map(m) => work.extend(m.values().rev()),
                _ => {}
            }
        }
        out
    }
}

impl QueryValue {
    /// As JSON, on one line: map keys in their `BTreeMap` order, a uid as its canonical text,
    /// a float as its shortest round-tripping decimal, and a float that is not finite as `null`.
    /// Equal values give equal bytes.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        json(self, &mut out);
        out
    }
}

fn json(v: &QueryValue, out: &mut String) {
    match v {
        QueryValue::Null => out.push_str("null"),
        QueryValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        QueryValue::Int(i) => out.push_str(&i.to_string()),
        QueryValue::Float(f) if f.is_finite() => out.push_str(&format!("{f:?}")),
        QueryValue::Float(_) => out.push_str("null"),
        QueryValue::Text(s) => quote(s, out),
        QueryValue::Uid(u) => quote(&u.canonical(), out),
        QueryValue::List(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                json(item, out);
            }
            out.push(']');
        }
        QueryValue::Map(m) => {
            out.push('{');
            for (i, (k, item)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                quote(k, out);
                out.push(':');
                json(item, out);
            }
            out.push('}');
        }
    }
}

fn quote(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// The text form: a scalar on its own, a list as `- ` items, a map as `key: value` lines, each
/// container indented under its key. A string is quoted, so `"true"` and `true` differ.
impl std::fmt::Display for QueryValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut out = String::new();
        text(self, 0, &mut out);
        f.write_str(out.trim_end_matches('\n'))
    }
}

fn scalar(v: &QueryValue) -> Option<String> {
    Some(match v {
        QueryValue::Null => "nodata".to_string(),
        QueryValue::Bool(b) => b.to_string(),
        QueryValue::Int(i) => i.to_string(),
        QueryValue::Float(x) => format!("{x:?}"),
        QueryValue::Text(s) => {
            let mut q = String::new();
            quote(s, &mut q);
            q
        }
        QueryValue::Uid(u) => u.canonical(),
        QueryValue::List(l) if l.is_empty() => "[]".to_string(),
        QueryValue::Map(m) if m.is_empty() => "{}".to_string(),
        _ => return None,
    })
}

fn text(v: &QueryValue, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    if let Some(s) = scalar(v) {
        out.push_str(&pad);
        out.push_str(&s);
        out.push('\n');
        return;
    }
    let line = |head: String, item: &QueryValue, out: &mut String| match scalar(item) {
        Some(s) => out.push_str(&format!("{pad}{head} {s}\n")),
        None => {
            out.push_str(&format!("{pad}{head}\n"));
            text(item, depth + 1, out);
        }
    };
    match v {
        QueryValue::List(items) => {
            for item in items {
                line("-".to_string(), item, out);
            }
        }
        QueryValue::Map(m) => {
            for (k, item) in m {
                line(format!("{k}:"), item, out);
            }
        }
        _ => unreachable!("every other value is a scalar"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> QueryValue {
        let mut m = BTreeMap::new();
        m.insert(
            "b".to_string(),
            QueryValue::List(vec![QueryValue::Int(1), QueryValue::Float(0.5)]),
        );
        m.insert(
            "a".to_string(),
            QueryValue::Text("say \"hi\"\n".to_string()),
        );
        m.insert("c".to_string(), QueryValue::Map(BTreeMap::new()));
        m.insert("d".to_string(), QueryValue::Null);
        QueryValue::Map(m)
    }

    #[test]
    fn json_is_ordered_and_escaped() {
        assert_eq!(
            sample().to_json(),
            r#"{"a":"say \"hi\"\n","b":[1,0.5],"c":{},"d":null}"#
        );
        assert_eq!(QueryValue::Float(f64::NAN).to_json(), "null");
    }

    #[test]
    fn the_text_form_nests() {
        assert_eq!(
            sample().to_string(),
            "a: \"say \\\"hi\\\"\\n\"\nb:\n  - 1\n  - 0.5\nc: {}\nd: nodata"
        );
        assert_eq!(QueryValue::Int(3).to_string(), "3");
    }
}
