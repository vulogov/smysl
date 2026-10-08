//! `json/1` — any JSON document, addressed by JSON Pointer.
//!
//! The generic reader, and the one with no vocabulary of its own: every string **value** in the
//! document is text, every container is structure, and the address of a node is the pointer
//! that reaches it — `/messages/3/text`, which is the locator form SMYSL-2.4 §3.2 gives this
//! reader. Keys are not text: a key is how a value is addressed, and a corpus whose prose
//! included the word `text` once per message would be measuring its own schema.
//!
//! Two levels. A direct child of the root is an `entry`, which is what a part boundary falls
//! on, and a string deeper inside one is a `field`. For the shape this reader exists to read —
//! an array of records — that makes one entry per record and one field per string in it.
//!
//! # Source order, without a feature flag
//!
//! `serde_json::Value` holds objects in a `BTreeMap` unless the `preserve_order` feature is
//! on, so parsing to a `Value` would assemble the text in **alphabetical key order** rather
//! than the document's. For a chat export that reorders every message's fields, and the tid
//! would be over a text no reader of the source would recognise.
//!
//! Turning that feature on is not available either: features unify across a build, and
//! `serde_json` is also what `smysl-provider` serialises prompts with — so a flag set here to
//! fix a reader would quietly change what a provider sends. This reader therefore walks the
//! deserialiser directly, where entries arrive in the order the bytes have them.

use std::fmt;

use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};

use crate::limits::Budget;
use crate::locator::Locator;
use crate::norm::Normalised;
use crate::readers::build::Doc;
use crate::readers::{Input, Params, ReadOutput};
use crate::reading::Level;
use crate::LibError;

const ID: &str = "json/1";

/// The level this reader cuts parts on.
pub const LEVEL: &str = "entry";

pub struct Json;

impl crate::readers::Reader for Json {
    fn id(&self) -> &'static str {
        ID
    }

    fn read(
        &self,
        input: &Input<'_>,
        _params: &Params,
        limits: &mut Budget,
    ) -> Result<ReadOutput, LibError> {
        limits.scan(input.len() as u64)?;
        let source = Normalised::new(input.bytes())?;
        read_json(source.as_str(), limits)
    }
}

struct State<'a> {
    doc: Doc,
    /// The pointer tokens of the value being read, unescaped.
    path: Vec<String>,
    limits: &'a mut Budget,
    /// The first refusal, kept here because a `serde` visitor can only fail with a `serde`
    /// error and a cap refusal has to arrive at the caller as the refusal it is.
    refused: Option<LibError>,
}

impl State<'_> {
    /// Record a refusal and produce the serde error that stops the walk.
    fn refuse<E: serde::de::Error>(&mut self, err: LibError) -> E {
        let message = err.to_string();
        if self.refused.is_none() {
            self.refused = Some(err);
        }
        E::custom(message)
    }

    fn locator<E: serde::de::Error>(&mut self) -> Result<Locator, E> {
        match Locator::pointer(&self.path) {
            Ok(loc) => Ok(loc),
            Err(e) => {
                let err = LibError::Unreadable {
                    reader: ID.to_string(),
                    at: 0,
                    what: format!("a pointer this crate can write ({e})"),
                };
                Err(self.refuse(err))
            }
        }
    }

    /// The level a node at this depth sits at.
    fn level(&self) -> &'static str {
        if self.path.len() <= 1 {
            LEVEL
        } else {
            "field"
        }
    }

    fn open<E: serde::de::Error>(&mut self, level: &str) -> Result<(), E> {
        let locator = self.locator()?;
        // No source offset: `serde_json`'s position is not reachable from inside a
        // visitor, and a pointer is already a precise address.
        if let Err(err) = self.doc.open(level, locator, 0, self.limits) {
            return Err(self.refuse(err));
        }
        Ok(())
    }
}

struct Walk<'a, 'b> {
    state: &'a mut State<'b>,
}

impl<'de> DeserializeSeed<'de> for Walk<'_, '_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Walk<'_, '_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<(), E> {
        // A string is the only thing in a JSON document that is prose. Numbers, booleans and
        // nulls are data about it; putting them in the text would make a timestamp a sentence.
        if value.trim().is_empty() {
            return Ok(());
        }
        let level = self.state.level();
        self.state.open(level)?;
        self.state.doc.write(value);
        Ok(())
    }

    fn visit_string<E: serde::de::Error>(self, value: String) -> Result<(), E> {
        self.visit_str(&value)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        if let Err(err) = self.state.limits.enter() {
            return Err(self.state.refuse(err));
        }
        // A container that is a direct child of the root is the entry a part is cut on. Opened
        // before its fields so that the row order is document order, which is what the segment
        // table requires.
        if self.state.path.len() == 1 {
            self.state.open(LEVEL)?;
        }
        while let Some(key) = map.next_key::<String>()? {
            self.state.path.push(key);
            let result = map.next_value_seed(Walk {
                state: &mut *self.state,
            });
            self.state.path.pop();
            result?;
        }
        self.state.limits.leave();
        Ok(())
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        if let Err(err) = self.state.limits.enter() {
            return Err(self.state.refuse(err));
        }
        if self.state.path.len() == 1 {
            self.state.open(LEVEL)?;
        }
        let mut index = 0u64;
        while {
            self.state.path.push(index.to_string());
            let more = seq.next_element_seed(Walk {
                state: &mut *self.state,
            })?;
            self.state.path.pop();
            more.is_some()
        } {
            index += 1;
        }
        self.state.limits.leave();
        Ok(())
    }

    fn visit_bool<E: serde::de::Error>(self, _v: bool) -> Result<(), E> {
        Ok(())
    }

    fn visit_i64<E: serde::de::Error>(self, _v: i64) -> Result<(), E> {
        Ok(())
    }

    fn visit_u64<E: serde::de::Error>(self, _v: u64) -> Result<(), E> {
        Ok(())
    }

    fn visit_f64<E: serde::de::Error>(self, _v: f64) -> Result<(), E> {
        Ok(())
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }

    fn visit_some<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

fn read_json(source: &str, limits: &mut Budget) -> Result<ReadOutput, LibError> {
    let mut state = State {
        doc: Doc::new(ID),
        path: Vec::new(),
        limits,
        refused: None,
    };
    let mut deserializer = serde_json::Deserializer::from_str(source);
    let walk = Walk { state: &mut state };
    let outcome = walk.deserialize(&mut deserializer);
    // A cap refusal travels as a serde error and arrives back here as the refusal it is; a
    // real parse failure keeps the parser's own position, which is the only thing that can
    // point at the byte where a document stopped making sense.
    if let Some(err) = state.refused.take() {
        return Err(err);
    }
    outcome.map_err(|e| LibError::Unreadable {
        reader: ID.to_string(),
        at: 0,
        what: format!(
            "well-formed JSON (line {}, column {}: {e})",
            e.line(),
            e.column()
        ),
    })?;
    deserializer.end().map_err(|e| LibError::Unreadable {
        reader: ID.to_string(),
        at: 0,
        what: format!(
            "one JSON document and nothing after it (line {}: {e})",
            e.line()
        ),
    })?;

    let (text, rows) = state.doc.finish()?;
    Ok(ReadOutput::new(
        text,
        rows,
        Level::new(LEVEL).expect("a level"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::Caps;
    use crate::readers::{read_with, Reader};
    use crate::structure::Structure;

    const CHAT: &str = r#"{"messages":[
        {"from":"Ada","date":"2026-01-01T10:00:00Z","text":"The engine is ready."},
        {"from":"Bea","date":"2026-01-01T10:01:00Z","text":"Then we begin."}
    ]}"#;

    fn read(source: &str) -> ReadOutput {
        let mut budget = Budget::new(Caps::DEFAULT, source.len() as u64).expect("budget");
        read_with(
            &Json,
            &Input::new(source.as_bytes()),
            &Params::new(),
            &mut budget,
        )
        .expect("read")
    }

    fn refuse(source: &str) -> LibError {
        let mut budget = Budget::new(Caps::DEFAULT, source.len() as u64).expect("budget");
        read_with(
            &Json,
            &Input::new(source.as_bytes()),
            &Params::new(),
            &mut budget,
        )
        .expect_err("refused")
    }

    fn ids(out: &ReadOutput) -> Vec<String> {
        out.rows.iter().map(|r| r.locator.to_string()).collect()
    }

    fn at<'a>(out: &'a ReadOutput, id: &str) -> &'a str {
        let row = out
            .rows
            .iter()
            .find(|r| r.locator.to_string() == id)
            .unwrap_or_else(|| panic!("{id} is not in {:?}", ids(out)));
        &out.text.as_str()[row.start as usize..row.end as usize]
    }

    /// Every string is addressed by the pointer that reaches it.
    #[test]
    fn strings_are_addressed_by_pointer() {
        let out = read(CHAT);
        assert_eq!(
            ids(&out),
            vec![
                "/messages",
                "/messages/0/from",
                "/messages/0/date",
                "/messages/0/text",
                "/messages/1/from",
                "/messages/1/date",
                "/messages/1/text",
            ]
        );
        assert_eq!(at(&out, "/messages/0/text"), "The engine is ready.");
        assert_eq!(at(&out, "/messages/1/from"), "Bea");
    }

    /// The text is in the document's order, not in the keys' alphabetical order.
    ///
    /// The reason this reader walks the deserialiser rather than a `Value`: with
    /// `serde_json`'s default map, `date` would precede `from` would precede `text`, and the
    /// text of a chat export would read in an order no export has.
    #[test]
    fn the_text_follows_the_document_order() {
        let out = read(CHAT);
        assert_eq!(
            out.text.as_str(),
            "Ada 2026-01-01T10:00:00Z The engine is ready. Bea 2026-01-01T10:01:00Z Then we begin."
        );
    }

    /// Keys are structure. They are how a value is addressed, not prose about it.
    #[test]
    fn keys_are_not_text() {
        let out = read(CHAT);
        assert!(!out.text.as_str().contains("messages"));
        assert!(!out.text.as_str().contains("from"));
    }

    /// Numbers, booleans and nulls are data about the text and not text.
    #[test]
    fn only_strings_are_text() {
        let out = read(r#"[{"id":41,"ok":true,"gone":null,"text":"Only this."}]"#);
        assert_eq!(out.text.as_str(), "Only this.");
        assert_eq!(ids(&out), vec!["/0", "/0/text"]);
    }

    /// A pointer token containing `/` or `~` is escaped, and round-trips.
    #[test]
    fn pointer_tokens_are_escaped() {
        let out = read(r#"{"a/b":{"c~d":"escaped."}}"#);
        assert_eq!(ids(&out), vec!["/a~1b", "/a~1b/c~0d"]);
        assert_eq!(at(&out, "/a~1b/c~0d"), "escaped.");
    }

    /// A string at the root's own level is an entry, because a part has to be cut on something.
    #[test]
    fn a_string_directly_in_the_root_is_an_entry() {
        let out = read(r#"["first","second"]"#);
        assert_eq!(ids(&out), vec!["/0", "/1"]);
        for row in &out.rows {
            assert_eq!(row.level.as_str(), LEVEL);
        }
    }

    #[test]
    fn the_rows_are_a_structure_that_resolves_every_pointer() {
        let out = read(CHAT);
        let mut budget = Budget::new(Caps::DEFAULT, 1 << 16).expect("budget");
        let structure =
            Structure::build(&out.rows, out.text.len() as u64, &mut budget).expect("a valid table");
        for row in &out.rows {
            assert_eq!(structure.resolve(&row.locator), Some(row.range()));
        }
    }

    #[test]
    fn malformed_json_is_refused_with_a_position() {
        let err = refuse(r#"{"messages": [ {"text": "unterminated }"#);
        match err {
            LibError::Unreadable { what, .. } => {
                assert!(what.contains("well-formed JSON"), "{what}");
                assert!(what.contains("line 1"), "{what}");
            }
            other => panic!("{other:?}"),
        }
    }

    /// Two documents in one file is a refusal: the second would be read as if it were part of
    /// the first, and a corpus would hold a text nobody sent.
    #[test]
    fn trailing_content_after_the_document_is_refused() {
        let err = refuse(r#"{"text":"one"} {"text":"two"}"#);
        match err {
            LibError::Unreadable { what, .. } => {
                assert!(what.contains("nothing after it"), "{what}")
            }
            other => panic!("{other:?}"),
        }
    }

    /// Nesting is charged against the nesting cap, and the refusal is `SMY-E440`.
    ///
    /// This is the cap that stands between this reader and a document built to exhaust the
    /// stack, and it is charged by the walk rather than by `serde_json`'s own recursion limit
    /// — which is a number this crate does not set and cannot name in a diagnostic.
    #[test]
    fn deep_nesting_is_refused_by_the_nesting_cap() {
        let mut caps = Caps::DEFAULT;
        caps.nesting = 4;
        let source = "[[[[[[[[\"deep\"]]]]]]]]";
        let mut budget = Budget::new(caps, source.len() as u64).expect("budget");
        let err = read_with(
            &Json,
            &Input::new(source.as_bytes()),
            &Params::new(),
            &mut budget,
        )
        .expect_err("refused");
        match err {
            LibError::Limit { cap, limit, .. } => {
                assert_eq!(cap, "nesting");
                assert_eq!(limit, 4);
            }
            other => panic!("{other:?}"),
        }
    }

    /// A cap refusal arrives as a cap refusal, not as a parse error that happens to mention it.
    #[test]
    fn the_node_cap_refusal_is_not_disguised_as_a_parse_error() {
        let mut caps = Caps::DEFAULT;
        caps.nodes = 2;
        let source = r#"["a","b","c","d"]"#;
        let mut budget = Budget::new(caps, source.len() as u64).expect("budget");
        let err = read_with(
            &Json,
            &Input::new(source.as_bytes()),
            &Params::new(),
            &mut budget,
        )
        .expect_err("refused");
        match err {
            LibError::Limit { cap, .. } => assert_eq!(cap, "nodes"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_empty_document_is_an_empty_text() {
        let out = read("{}");
        assert!(out.rows.is_empty());
        assert!(out.text.is_empty());
    }

    #[test]
    fn the_reader_answers_to_its_id() {
        assert_eq!(Json.id(), "json/1");
        assert!(Json.params().is_empty());
    }
}
