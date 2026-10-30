//! `telegram/1` — a Telegram Desktop JSON export (`result.json`).
//!
//! One object with a header and a `messages` array; one node per message, one part per UTC day
//! (see `readers::chat`).
//!
//! # Why this reader parses to a `Value` and `json/1` does not
//!
//! `json/1` walks the deserialiser by hand, because for a document with no schema the *order*
//! of an object's keys is the order its text is assembled in, and `serde_json::Value` holds an
//! object in a `BTreeMap`. Here the schema is known: this reader reads named fields and never
//! iterates an object's keys, and the two places order matters — `messages` and a split
//! `text` — are JSON arrays, which are `Vec` either way. So the simple parser is the correct
//! one, and the comment is here because the next reader of these two files will otherwise
//! conclude that one of them is wrong.
//!
//! # The timestamp, which is two fields and one of them is a trap
//!
//! An export carries `date` (`"2024-01-15T10:23:45"`) and `date_unixtime` (`"1705314225"`).
//! **`date` has no offset and is not UTC** — it is the local wall clock of the machine the
//! export was taken on. A reader that took it for UTC would put messages on the wrong day for
//! most of the world, and a part boundary is a tid, so the error would be baked into every
//! identity in the corpus.
//!
//! So `date_unixtime` is the timestamp, and `date` is used only when it is absent — which
//! happens in exports from before 2021 — and then only if the caller says what offset to
//! assume, with `--param tz=±HHMM`. Without it, the refusal says so. Guessing UTC would be
//! choosing the one thing the field is not.
//!
//! `tz` is therefore a **parameter that changes the output**, which is why it is recorded in
//! the manifest beside the reader id (`telegram/1 tz=+0300`): the same file read under two
//! offsets is two different partitions into days.
//!
//! # What is dropped, and why dropping it is the privacy-preserving choice
//!
//! `forwarded_from` is a display name, and it is the name of a **second** person — not the
//! sender, with no user id beside it to key a pseudonym from. Carrying it would put a name in
//! a field that `--pseudonymise` does not reach ([`crate::speaker`] says why it reaches only
//! the speaker), so it is dropped and the read is marked lossy. The same goes for a message
//! whose `from` is present without a `from_id`.
//!
//! Attachments, stickers, polls, locations, contacts and edit times are dropped too, each
//! marking the read lossy. A reader of a *text* library is not the place to decide how a poll
//! is prose.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::limits::Budget;
use crate::readers::build::Doc;
use crate::readers::chat::{self, Message};
use crate::readers::{Input, ParamSpec, Params, ReadOutput, Reader};
use crate::reading::Level;
use crate::LibError;

const ID: &str = "telegram/1";

/// The locator head. One export is one conversation, so the conversation needs no name of its
/// own — `chat.20240115.7` reads as "the seventh message of that day in this chat", and the
/// chat's own numeric id is in the manifest's `identifiers` where a tool can find it.
const HEAD: &str = "chat";

/// The parameter this reader accepts.
const PARAMS: &[ParamSpec] = &[ParamSpec {
    key: "tz",
    required: false,
    // The one parameter in this build that is *conditionally* required: needed only for an
    // export with no `date_unixtime`. `required: true` would demand it of every modern export,
    // where it is not merely unnecessary but ignored — and a recorded setting that changed
    // nothing would make two identical corpora two different manifests.
    what: "the UTC offset `date` was written in, `±HHMM` or `Z`, for an export with no \
           `date_unixtime`",
}];

/// Keys whose presence means the source carried something this reader does not.
const DROPPED: &[&str] = &[
    "photo",
    "file",
    "media_type",
    "sticker_emoji",
    "poll",
    "location_information",
    "contact_information",
    "game_title",
    "edited",
    "edited_unixtime",
    "forwarded_from",
    "via_bot",
];

pub struct Telegram;

impl Reader for Telegram {
    fn id(&self) -> &'static str {
        ID
    }

    fn params(&self) -> &'static [ParamSpec] {
        PARAMS
    }

    fn read(
        &self,
        input: &Input<'_>,
        params: &Params,
        limits: &mut Budget,
    ) -> Result<ReadOutput, LibError> {
        limits.scan(input.len() as u64)?;
        let offset = match params.get("tz") {
            Some(text) => Some(chat::offset_of(text).ok_or_else(|| LibError::BadParam {
                reader: ID.to_string(),
                key: "tz".to_string(),
                reason: format!("`{text}` is not `±HHMM` or `Z`"),
            })?),
            None => None,
        };

        let root: Value = serde_json::from_slice(input.bytes()).map_err(|e| unreadable(0, e))?;
        let object = root.as_object().ok_or_else(|| {
            unreadable(
                0,
                "a Telegram export is one JSON object with a `messages` array",
            )
        })?;
        let messages = object
            .get("messages")
            .and_then(Value::as_array)
            .ok_or_else(|| unreadable(0, "key `messages`, holding an array"))?;

        let mut lossy = false;
        let mut parsed: Vec<Message> = Vec::new();
        for (index, message) in messages.iter().enumerate() {
            // Charged per message, before the message is turned into anything: the cost of
            // reading an export is its messages, and a refusal belongs before the allocation.
            limits.fuel(1)?;
            match read_message(message, offset, index, &mut lossy)? {
                Some(m) => parsed.push(m),
                None => lossy = true,
            }
        }

        let mut doc = Doc::new(ID);
        chat::conversation(&mut doc, ID, HEAD, &parsed, limits)?;
        let (text, rows) = doc.finish()?;

        let mut out = ReadOutput::new(
            text,
            rows,
            Level::new(chat::DAY).expect("`day` is a valid level"),
        );
        if let Some(name) = object.get("name").and_then(Value::as_str) {
            if !name.trim().is_empty() {
                out.title = Some(name.to_string());
            }
        }
        if let Some(id) = object.get("id").and_then(scalar) {
            out.identifiers.insert("telegram:chat".to_string(), id);
        }
        // The header's own words for what kind of conversation this is, kept because nothing
        // in a manifest has a field for it and because it is the difference between a corpus
        // of two people and a corpus of a public channel.
        let mut raw = BTreeMap::new();
        for key in ["type", "name", "id"] {
            if let Some(v) = object.get(key).and_then(scalar) {
                raw.insert(key.to_string(), v);
            }
        }
        if !raw.is_empty() {
            let bytes = chat::raw_map(&raw);
            limits.raw(bytes.len() as u64)?;
            out.raw = Some(bytes);
        }
        out.lossy = lossy;
        Ok(out)
    }
}

fn unreadable(at: usize, what: impl std::fmt::Display) -> LibError {
    LibError::Unreadable {
        reader: ID.to_string(),
        at,
        what: what.to_string(),
    }
}

/// A JSON scalar as the text a manifest field holds. `None` for a container.
///
/// Numbers keep the spelling `serde_json` gives them, which for a chat id is the digits it had.
fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// One message, or `None` for one this reader does not carry.
fn read_message(
    value: &Value,
    offset: Option<i32>,
    index: usize,
    lossy: &mut bool,
) -> Result<Option<Message>, LibError> {
    let object = match value.as_object() {
        Some(o) => o,
        // Not a refusal: an array holding something that is not a message is a file this
        // reader did not write, and the messages around it are still readable. Lossy says so.
        None => return Ok(None),
    };
    // A service message is an *event* — somebody joined, a message was pinned — and it carries
    // an `action` rather than prose. Skipped rather than rendered: "Alice pinned a message" is
    // a sentence this reader would have written, and a corpus should not hold sentences its
    // reader invented.
    if object.get("type").and_then(Value::as_str) != Some("message") {
        return Ok(None);
    }

    let observed = match object.get("date_unixtime").and_then(scalar) {
        Some(seconds) => seconds
            .parse::<i64>()
            .ok()
            .filter(|s| *s >= 0)
            .and_then(|s| s.checked_mul(1000))
            .and_then(|ms| u64::try_from(ms).ok())
            .ok_or_else(|| {
                unreadable(
                    index,
                    format!("`date_unixtime` as a second count at or after 1970, not `{seconds}`"),
                )
            })?,
        None => {
            let naive = object
                .get("date")
                .and_then(Value::as_str)
                .ok_or_else(|| unreadable(index, "`date_unixtime` or `date`"))?;
            let offset = offset.ok_or_else(|| LibError::BadParam {
                reader: ID.to_string(),
                key: "tz".to_string(),
                reason: "this export has no `date_unixtime`, and `date` is a local wall \
                             clock with no offset in it — say which offset it was written in, \
                             because assuming UTC would put messages on the wrong day"
                    .to_string(),
            })?;
            local(naive, offset)
                .ok_or_else(|| unreadable(index, format!("`date` as a timestamp, not `{naive}`")))?
        }
    };

    let mut text = String::new();
    if let Some(value) = object.get("text") {
        prose(value, &mut text, lossy);
    }

    let mut ids = BTreeMap::new();
    if let Some(id) = object.get("id").and_then(scalar) {
        ids.insert("msg".to_string(), id);
    }
    // A reply is an edge, and the time engine (TX-P3) is what turns it into a constraint —
    // `SMY-W448` is what it reports when the message a reply names is not there. Kept as the
    // source's own
    // number: this reader does not know which row that number names, and resolving it here
    // would mean a reader with an opinion about the whole file rather than about a message.
    if let Some(reply) = object
        .get("reply_to_message_id")
        .and_then(scalar)
        .filter(|r| r != "0")
    {
        ids.insert("reply".to_string(), reply);
    }

    let speaker = object.get("from_id").and_then(Value::as_str);
    if speaker.is_none() && object.get("from").and_then(Value::as_str).is_some() {
        // A name without an id: see the module header. Dropped, not carried.
        *lossy = true;
    }
    if DROPPED.iter().any(|key| object.contains_key(*key)) {
        *lossy = true;
    }

    Ok(Some(Message {
        text,
        speaker: speaker.map(str::to_string),
        observed,
        // Telegram's own offset, for the row. `date_unixtime` fixes the instant; `date` beside
        // it is the same instant in the exporter's local time, and the difference between them
        // is that offset — which is a fact about the conversation worth keeping and not worth
        // reconstructing by subtraction here. Recorded only when the caller named it.
        tz_offset: offset,
        ids,
    }))
}

/// Telegram's `text`: a string, or an array of strings and `{type, text}` entities.
fn prose(value: &Value, out: &mut String, lossy: &mut bool) {
    match value {
        Value::String(s) => out.push_str(s),
        Value::Array(items) => {
            for item in items {
                match item {
                    Value::String(s) => out.push_str(s),
                    // An entity carries the same characters the plain text would have, plus a
                    // name for what they are: a link, a mention, a code span. The characters
                    // are the message; the markup is not, and `md/1` already settled that a
                    // dropped link target is lossy rather than silent.
                    Value::Object(o) => match o.get("text").and_then(Value::as_str) {
                        Some(s) => {
                            out.push_str(s);
                            if o.get("type").and_then(Value::as_str) != Some("plain") {
                                *lossy = true;
                            }
                        }
                        None => *lossy = true,
                    },
                    _ => *lossy = true,
                }
            }
        }
        Value::Null => {}
        _ => *lossy = true,
    }
}

/// A naive `YYYY-MM-DDTHH:MM:SS` read as a local time at `offset`, in UTC milliseconds.
fn local(naive: &str, offset: i32) -> Option<u64> {
    let (date, time) = naive.split_once('T')?;
    let mut d = date.split('-');
    let year: i64 = d.next()?.parse().ok()?;
    let month: u32 = d.next()?.parse().ok()?;
    let day: u32 = d.next()?.parse().ok()?;
    if d.next().is_some() {
        return None;
    }
    let mut t = time.split(':');
    let hour: u32 = t.next()?.parse().ok()?;
    let minute: u32 = t.next()?.parse().ok()?;
    let second: u32 = t.next().unwrap_or("0").parse().ok()?;
    if t.next().is_some() {
        return None;
    }
    let wall = chat::instant(year, month, day, hour, minute, second)?;
    // The offset is *east* of UTC, so a local wall clock is that many minutes ahead of the
    // instant it names. Subtracted, and refused rather than clamped when the result is before
    // the epoch: a chat message from 1969 is a file that is not a Telegram export.
    let shift = i64::from(offset) * 60_000;
    u64::try_from(wall as i64 - shift).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::Caps;
    use crate::readers::read_with;
    use crate::structure::Structure;

    fn read_with_params(bytes: &[u8], params: &Params) -> Result<ReadOutput, LibError> {
        let mut budget = Budget::new(Caps::DEFAULT, bytes.len() as u64).expect("budget");
        read_with(&Telegram, &Input::new(bytes), params, &mut budget)
    }

    fn read(bytes: &[u8]) -> ReadOutput {
        read_with_params(bytes, &Params::new()).expect("read")
    }

    const TWO_DAYS: &str = r#"{
      "name": "Книжный клуб",
      "type": "private_group",
      "id": 1234567890,
      "messages": [
        {"id": 1, "type": "message", "date": "2024-01-15T10:23:45",
         "date_unixtime": "1705314225", "from": "Alice", "from_id": "user111",
         "text": "Прочитал первую главу."},
        {"id": 2, "type": "service", "date": "2024-01-15T10:30:00",
         "date_unixtime": "1705314600", "actor": "Alice", "actor_id": "user111",
         "action": "pin_message"},
        {"id": 3, "type": "message", "date": "2024-01-15T11:00:00",
         "date_unixtime": "1705316400", "from": "Bob", "from_id": "user222",
         "text": "И что скажешь?", "reply_to_message_id": 1},
        {"id": 4, "type": "message", "date": "2024-01-16T09:00:00",
         "date_unixtime": "1705395600", "from": "Alice", "from_id": "user111",
         "text": ["Вот ссылка: ", {"type": "link", "text": "https://example.org"}]}
      ]
    }"#;

    /// The structure SMYSL-2.4 §3.2 asks for: one node per message, grouped by UTC day.
    #[test]
    fn an_export_is_days_of_messages_and_the_days_are_the_parts() {
        let out = read(TWO_DAYS.as_bytes());
        let addresses: Vec<String> = out.rows.iter().map(|r| r.locator.to_string()).collect();
        assert_eq!(
            addresses,
            vec![
                "chat.20240115",
                "chat.20240115.1",
                "chat.20240115.2",
                "chat.20240116",
                "chat.20240116.1",
            ]
        );
        assert_eq!(out.top_level.as_str(), chat::DAY);
        let text = out.text.as_str();
        let slice =
            |i: usize| text[out.rows[i].start as usize..out.rows[i].end as usize].to_string();
        assert_eq!(slice(1), "Прочитал первую главу.");
        assert_eq!(slice(2), "И что скажешь?");
        assert_eq!(slice(4), "Вот ссылка: https://example.org");
    }

    /// The sender is the platform's id, and the text holds no name at all.
    ///
    /// This is the property `--pseudonymise` stands on: the bytes a tid is taken over do not
    /// mention who said anything, so pseudonymising changes no tid.
    #[test]
    fn the_speaker_is_an_id_and_the_text_is_only_prose() {
        let out = read(TWO_DAYS.as_bytes());
        assert_eq!(out.rows[1].speaker.as_deref(), Some("user111"));
        assert_eq!(out.rows[2].speaker.as_deref(), Some("user222"));
        let text = out.text.as_str();
        for name in ["Alice", "Bob", "user111", "user222"] {
            assert!(!text.contains(name), "`{name}` reached the text: {text}");
        }
    }

    /// A service message is an event, not prose, and leaves no node.
    #[test]
    fn a_service_message_is_not_a_sentence_this_reader_writes() {
        let out = read(TWO_DAYS.as_bytes());
        assert!(!out.text.as_str().contains("pin"), "{}", out.text.as_str());
        assert!(out.lossy, "dropping it is a loss and is reported as one");
    }

    /// The platform's own numbers are kept where a tool can read them.
    #[test]
    fn a_message_keeps_its_id_and_its_reply() {
        let out = read(TWO_DAYS.as_bytes());
        assert_eq!(out.rows[1].ids.get("msg").map(String::as_str), Some("1"));
        assert_eq!(out.rows[2].ids.get("msg").map(String::as_str), Some("3"));
        assert_eq!(out.rows[2].ids.get("reply").map(String::as_str), Some("1"));
        assert!(
            !out.rows[1].ids.contains_key("reply"),
            "a message that replies to nothing has no reply id"
        );
    }

    /// The timestamp is the instant, and the day is that instant's UTC day.
    #[test]
    fn the_day_comes_from_the_unix_timestamp_and_not_from_the_wall_clock() {
        let out = read(TWO_DAYS.as_bytes());
        assert_eq!(out.rows[1].observed, Some(1_705_314_225_000));
        // 1705395600 is 2024-01-16T09:00:00Z, and the row says the 16th.
        assert_eq!(out.rows[4].observed, Some(1_705_395_600_000));
    }

    /// The header reaches the manifest: a title, an identifier, and `raw` for the rest.
    #[test]
    fn the_header_becomes_a_title_an_identifier_and_raw_metadata() {
        let out = read(TWO_DAYS.as_bytes());
        assert_eq!(out.title.as_deref(), Some("Книжный клуб"));
        assert_eq!(
            out.identifiers.get("telegram:chat").map(String::as_str),
            Some("1234567890")
        );
        let raw = out.raw.expect("raw metadata");
        // Canonical CBOR: a three-entry text-keyed map, in encoded-key order.
        let mut d = smysl_core::cbor::Dec::new(&raw);
        assert_eq!(d.map_head().expect("a map"), 3);
        assert_eq!(d.text().expect("a key"), "id");
        assert_eq!(d.text().expect("a value"), "1234567890");
        assert_eq!(d.text().expect("a key"), "name");
    }

    /// An export with no `date_unixtime` is refused until the caller says what `date` meant.
    #[test]
    fn a_naive_date_without_an_offset_is_refused_rather_than_assumed_to_be_utc() {
        let old = br#"{"name":"x","messages":[
            {"id":1,"type":"message","date":"2024-01-15T23:30:00","from_id":"user1","text":"late"}]}"#;
        let err = read_with_params(old, &Params::new()).expect_err("refused");
        match err {
            LibError::BadParam { key, reason, .. } => {
                assert_eq!(key, "tz");
                assert!(reason.contains("wrong day"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    /// With the offset, the local wall clock becomes an instant — and a different day.
    ///
    /// `23:30` at `+0300` is `20:30` UTC on the same date; at `-0500` it is `04:30` UTC on the
    /// *next* one. Both are asserted, because the whole point of the parameter is that the
    /// partition into parts depends on it.
    #[test]
    fn an_offset_decides_which_day_a_late_message_belongs_to() {
        let old = br#"{"name":"x","messages":[
            {"id":1,"type":"message","date":"2024-01-15T23:30:00","from_id":"user1","text":"late"}]}"#;
        let mut east = Params::new();
        east.set(ID, "tz", "+0300").expect("set");
        let out = read_with_params(old, &east).expect("read");
        assert_eq!(out.rows[0].locator.to_string(), "chat.20240115");
        // The instant is on the message row; a day is not an instant and carries none.
        assert_eq!(out.rows[1].observed, Some(1_705_350_600_000));
        assert_eq!(out.rows[0].observed, None);
        assert_eq!(out.rows[1].tz_offset, Some(180));

        let mut west = Params::new();
        west.set(ID, "tz", "-0500").expect("set");
        let out = read_with_params(old, &west).expect("read");
        assert_eq!(out.rows[0].locator.to_string(), "chat.20240116");
        assert_eq!(out.rows[1].tz_offset, Some(-300));
    }

    /// A bad `tz` is refused when the reader runs, by name.
    #[test]
    fn an_unreadable_offset_is_refused_naming_the_parameter() {
        let mut params = Params::new();
        params.set(ID, "tz", "noon").expect("set");
        let err = read_with_params(TWO_DAYS.as_bytes(), &params).expect_err("refused");
        match err {
            LibError::BadParam { key, reason, .. } => {
                assert_eq!(key, "tz");
                assert!(reason.contains("±HHMM"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    /// A forwarded name is a second person with no id, and it does not reach the corpus.
    #[test]
    fn a_forwarded_display_name_is_dropped_and_the_read_says_so() {
        let fwd = br#"{"name":"x","messages":[
            {"id":1,"type":"message","date_unixtime":"1705314225","from_id":"user1",
             "forwarded_from":"Carol Danvers","text":"look at this"}]}"#;
        let out = read(fwd);
        assert!(
            !out.text.as_str().contains("Carol"),
            "{}",
            out.text.as_str()
        );
        for row in &out.rows {
            assert!(
                !row.ids.values().any(|v| v.contains("Carol")),
                "a name reached the ids: {:?}",
                row.ids
            );
        }
        assert!(out.lossy);
    }

    /// Prose with nothing dropped is not lossy.
    #[test]
    fn a_plain_conversation_is_not_lossy() {
        let plain = br#"{"name":"x","messages":[
            {"id":1,"type":"message","date_unixtime":"1705314225","from_id":"user1",
             "text":"one"},
            {"id":2,"type":"message","date_unixtime":"1705314226","from_id":"user2",
             "text":[{"type":"plain","text":"two"}]}]}"#;
        let out = read(plain);
        assert_eq!(out.text.as_str(), "one two");
        assert!(!out.lossy, "nothing was dropped");
    }

    /// A message that is only an attachment has no prose, so it has no node.
    #[test]
    fn a_message_with_no_text_is_not_a_node() {
        let media = br#"{"name":"x","messages":[
            {"id":1,"type":"message","date_unixtime":"1705314225","from_id":"user1",
             "photo":"photos/x.jpg","text":""},
            {"id":2,"type":"message","date_unixtime":"1705314226","from_id":"user1",
             "text":"after"}]}"#;
        let out = read(media);
        let addresses: Vec<String> = out.rows.iter().map(|r| r.locator.to_string()).collect();
        assert_eq!(addresses, vec!["chat.20240115", "chat.20240115.1"]);
        assert_eq!(out.text.as_str(), "after");
        assert!(out.lossy);
    }

    /// The rows are a structure, and every locator this reader emits resolves in it.
    #[test]
    fn the_rows_build_a_structure_that_resolves_every_locator() {
        let out = read(TWO_DAYS.as_bytes());
        let mut budget = Budget::new(Caps::DEFAULT, 1 << 16).expect("budget");
        let structure = Structure::build(&out.rows, out.text.len() as u64, &mut budget)
            .expect("the rows are a valid table");
        for row in &out.rows {
            assert_eq!(
                structure.resolve(&row.locator),
                Some(row.range()),
                "{:?}",
                row.locator
            );
        }
        assert_eq!(structure.roots().len(), 2, "two days, two roots");
    }

    /// Bytes that are not a Telegram export are refused, naming what was expected.
    #[test]
    fn a_file_that_is_not_an_export_is_refused_by_what_is_missing() {
        for (bytes, expected) in [
            (&b"[1,2,3]"[..], "one JSON object"),
            (&b"{\"name\":\"x\"}"[..], "messages"),
        ] {
            let err = read_with_params(bytes, &Params::new()).expect_err("refused");
            match err {
                LibError::Unreadable { what, .. } => {
                    assert!(what.contains(expected), "{what}")
                }
                other => panic!("{other:?}"),
            }
        }
    }

    /// Not JSON at all is a refusal, not a panic.
    #[test]
    fn bytes_that_are_not_json_are_refused() {
        let err = read_with_params(b"{oh no", &Params::new()).expect_err("refused");
        assert!(matches!(err, LibError::Unreadable { .. }), "{err:?}");
    }

    /// The node cap is charged per message.
    #[test]
    fn too_many_messages_is_a_cap_refusal() {
        let mut caps = Caps::DEFAULT;
        caps.nodes = 3;
        let mut budget = Budget::new(caps, TWO_DAYS.len() as u64).expect("budget");
        let err = read_with(
            &Telegram,
            &Input::new(TWO_DAYS.as_bytes()),
            &Params::new(),
            &mut budget,
        )
        .expect_err("refused");
        match err {
            LibError::Limit { cap, limit, .. } => {
                assert_eq!(cap, "nodes");
                assert_eq!(limit, 3);
            }
            other => panic!("{other:?}"),
        }
    }
}
