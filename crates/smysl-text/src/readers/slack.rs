//! `slack/1` — a Slack workspace export, which is a zip.
//!
//! ```text
//! channels.json              the channel list: name, id, topic, purpose
//! users.json                 the user table
//! general/2024-01-15.json    an array of messages, one file per channel per local day
//! random/2024-01-16.json
//! ```
//!
//! Three levels: `channel` → `day` → `message`, and a part is cut on the day
//! (`readers::chat`). A locator is `general.20240115.7`.
//!
//! # The archive, which SMYSL-2.4 §3.2 called an unverified choice
//!
//! It is verified now, and the verification is a version. `zip` 9.0.0 — the current release —
//! declares rustc **1.88**, and `smysl-text` is in the pure tier at **1.85**: adopting it
//! would have raised the floor of the eleven crates the MSRV job compiles, for one reader.
//! `zip` 4.3.0 declares 1.82, and the highest floor anywhere in its tree is `hashbrown` 0.17's
//! 1.85 — exactly the base. Eleven crates, no `cc`, and nothing on the purity gate's `NEVER`
//! list. Both pins are exact in the workspace manifest, which also records the measurement
//! that `zip`'s own `deflate-flate2` feature does not name a backend and does not compile
//! alone.
//!
//! An archive reader is also what [`crate::limits`] was built for and nothing had used:
//! `Budget::entry` asks four questions of every entry — how many, how large, how large in
//! total, and at what expansion ratio — which is what keeps a 40-byte zip that decompresses to
//! a terabyte from being read instead of refused.
//!
//! # Why the output does not depend on the order of the archive
//!
//! Zip entries are in whatever order the exporter wrote them, and a reading has to be a
//! function of the bytes. So the entries are **collected and sorted** — by channel, then by
//! day — before a single message is turned into a row. Reading them in archive order would
//! have produced a different text and a different tid for two archives holding the same
//! conversation, which is the defect this crate's whole reader contract exists to prevent.
//!
//! # Why the days are regrouped
//!
//! Slack's file names are the workspace's local days; the format's parts are UTC days. A
//! `2024-01-15.json` therefore holds messages that belong to two UTC days, and a reader that
//! trusted the file name would cut parts on a boundary that is nowhere in the data. So a
//! channel's messages are concatenated in file order — which is chronological, because the
//! timestamps within each file are — and the UTC day is taken from each message's own `ts`.
//!
//! # The user table is read and not used
//!
//! `users.json` maps `U024BE7LH` to a display name. The speaker stays the **id**: a display
//! name is the person, changes over time, and is the thing `--pseudonymise` exists to keep out
//! of a corpus. So the table is skipped, which is one of several reasons a Slack read is
//! almost always marked lossy — an archive carries a user table, file attachments, reactions
//! and emoji, and a text library holds none of them. The flag says so rather than implying
//! that a chat export round-trips.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use serde_json::Value;

use crate::limits::Budget;
use crate::norm::Normalised;
use crate::readers::build::Doc;
use crate::readers::chat::{self, Message};
use crate::readers::{Input, Params, ReadOutput, Reader};
use crate::reading::Level;
use crate::LibError;

const ID: &str = "slack/1";

/// The channel list, which is the only archive member besides the message files this reader
/// reads at all.
const CHANNELS: &str = "channels.json";

/// Slack's own subtypes for things that happened to a channel rather than things somebody
/// said.
///
/// Skipped, with the read marked lossy — the same decision `telegram/1` makes about a service
/// message, for the same reason: `<@U1> has joined the channel` is a sentence Slack wrote, and
/// a corpus of what people said should not hold it.
///
/// These are machine names and therefore not localised, which is what makes a list of them
/// defensible where the list of WhatsApp's media placeholders was not. A subtype **not** on this
/// list is carried: `bot_message` and `thread_broadcast` are prose, whoever or whatever wrote
/// them.
const EVENTS: &[&str] = &[
    "channel_join",
    "channel_leave",
    "channel_topic",
    "channel_purpose",
    "channel_name",
    "channel_archive",
    "channel_unarchive",
    "group_join",
    "group_leave",
    "pinned_item",
    "unpinned_item",
    "bot_add",
    "bot_remove",
    "reminder_add",
    "tombstone",
];

/// Keys whose presence means a message carried something this reader does not.
const DROPPED: &[&str] = &["files", "attachments", "reactions", "edited", "blocks"];

pub struct Slack;

impl Reader for Slack {
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
        let mut archive = zip::ZipArchive::new(Cursor::new(input.bytes()))
            .map_err(|e| unreadable(0, format_args!("a zip archive ({e})")))?;

        let mut lossy = false;
        // What to read, decided before anything is decompressed: an entry's name and its sizes
        // are in the central directory, so a 2 GB `users.json` costs nothing to decline.
        let mut days: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
        let mut channels_at: Option<usize> = None;
        for index in 0..archive.len() {
            let entry = archive
                .by_index_raw(index)
                .map_err(|e| unreadable(index, format_args!("a readable archive entry ({e})")))?;
            if !entry.is_file() {
                continue;
            }
            let name = entry.name().to_string();
            // Charged for every entry, read or not: the count is a cap of its own, and an
            // archive with a million members is refused before it is walked rather than after.
            limits.entry(entry.size(), entry.compressed_size())?;
            if name == CHANNELS {
                channels_at = Some(index);
                continue;
            }
            match day_file(&name) {
                // One index per channel-day. There is no refusal here for a *second* entry of
                // the same name, and that is a measurement rather than an omission: `zip`
                // keeps its central directory in a map keyed by name, so an archive holding
                // `general/2024-01-15.json` twice has `len() == 1` and this loop is never
                // offered the second. A check for it would have been a refusal nothing can
                // trigger, which is worse than no check — a reader of this file would believe
                // the case was handled here when it was handled in the zip crate.
                Some((channel, day)) => {
                    days.entry(channel.to_string())
                        .or_default()
                        .insert(day.to_string(), index);
                }
                // `users.json`, `integration_logs.json`, a directory of canvases, an
                // attachment: everything a text library does not hold.
                None => lossy = true,
            }
        }

        let mut raw = BTreeMap::new();
        if let Some(index) = channels_at {
            let bytes = read_entry(&mut archive, index, limits)?;
            let parsed: Value = serde_json::from_slice(&bytes)
                .map_err(|e| unreadable(index, format_args!("`{CHANNELS}` as JSON ({e})")))?;
            for channel in parsed.as_array().into_iter().flatten() {
                let object = match channel.as_object() {
                    Some(o) => o,
                    None => continue,
                };
                if let (Some(name), Some(id)) = (
                    object.get("name").and_then(Value::as_str),
                    object.get("id").and_then(Value::as_str),
                ) {
                    // The export's own mapping from the directory name a locator is built from
                    // to the id the Slack API knows. Nothing here decodes `raw`; it is kept so
                    // that whoever has to go back to the source can.
                    raw.insert(name.to_string(), id.to_string());
                }
            }
        }

        let mut doc = Doc::new(ID);
        for (channel, files) in &days {
            let head = chat::head_of(channel).ok_or_else(|| {
                unreadable(
                    0,
                    format_args!(
                        "a channel name with an ASCII letter in it, to build a locator from, \
                         not `{channel}`"
                    ),
                )
            })?;
            let mut messages: Vec<Message> = Vec::new();
            for index in files.values() {
                let bytes = read_entry(&mut archive, *index, limits)?;
                let text = Normalised::new(&bytes)?;
                read_day(text.as_str(), *index, &mut messages, &mut lossy)?;
            }
            if messages.is_empty() {
                continue;
            }
            doc.open(chat::CHANNEL, chat::canonical(ID, &head, &[])?, 0, limits)?;
            chat::conversation(&mut doc, ID, &head, &messages, limits)?;
            doc.close_level(chat::CHANNEL);
        }
        let (text, rows) = doc.finish()?;

        let mut out = ReadOutput::new(
            text,
            rows,
            Level::new(chat::DAY).expect("`day` is a valid level"),
        );
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

/// `general/2024-01-15.json` → `("general", "2024-01-15")`.
///
/// The date is kept as text and only checked for shape: it is used to **order** a channel's
/// files and for nothing else, because the day a message belongs to comes from its own
/// timestamp. A file whose name is not a date is not a message file.
fn day_file(name: &str) -> Option<(&str, &str)> {
    let (channel, file) = name.rsplit_once('/')?;
    if channel.is_empty() || channel.contains('/') {
        return None;
    }
    let day = file.strip_suffix(".json")?;
    let bytes = day.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    if !bytes
        .iter()
        .enumerate()
        .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
    {
        return None;
    }
    Some((channel, day))
}

/// Decompress one entry, under the budget that already counted it.
fn read_entry(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    index: usize,
    limits: &mut Budget,
) -> Result<Vec<u8>, LibError> {
    let mut entry = archive
        .by_index(index)
        .map_err(|e| unreadable(index, format_args!("a readable archive entry ({e})")))?;
    let declared = entry.size();
    // `with_capacity` on a declared size, and `take` so that a lying header cannot make this
    // allocate past it: the declared size was what `Budget::entry` approved, so it is also the
    // most this is allowed to read.
    let mut bytes = Vec::with_capacity(declared.min(1 << 20) as usize);
    let read = entry
        .by_ref()
        .take(declared)
        .read_to_end(&mut bytes)
        .map_err(|e| unreadable(index, format_args!("the entry this header describes ({e})")))?;
    limits.fuel(read as u64)?;
    Ok(bytes)
}

/// One `YYYY-MM-DD.json`: an array of messages, appended in file order.
fn read_day(
    text: &str,
    index: usize,
    out: &mut Vec<Message>,
    lossy: &mut bool,
) -> Result<(), LibError> {
    let parsed: Value = serde_json::from_str(text)
        .map_err(|e| unreadable(index, format_args!("an array of messages ({e})")))?;
    let array = parsed
        .as_array()
        .ok_or_else(|| unreadable(index, "an array of messages"))?;
    for value in array {
        match read_message(value, lossy) {
            Some(message) => out.push(message),
            None => *lossy = true,
        }
    }
    Ok(())
}

fn read_message(value: &Value, lossy: &mut bool) -> Option<Message> {
    let object = value.as_object()?;
    if object.get("type").and_then(Value::as_str) != Some("message") {
        return None;
    }
    if let Some(subtype) = object.get("subtype").and_then(Value::as_str) {
        if EVENTS.contains(&subtype) {
            return None;
        }
    }
    let ts = object.get("ts").and_then(Value::as_str)?;
    let observed = instant_of(ts)?;

    let mut text = String::new();
    if let Some(raw) = object.get("text").and_then(Value::as_str) {
        prose(raw, &mut text, lossy);
    }

    let mut ids = BTreeMap::new();
    // `ts` *is* Slack's identifier for a message: a reply names its parent by it, and a
    // permalink is built from it. Kept under `msg` so that the three chat readers agree about
    // the key, and the fact that it looks like a timestamp is Slack's doing.
    ids.insert("msg".to_string(), ts.to_string());
    if let Some(thread) = object
        .get("thread_ts")
        .and_then(Value::as_str)
        .filter(|t| *t != ts)
    {
        ids.insert("thread".to_string(), thread.to_string());
    }

    let speaker = object
        .get("user")
        .and_then(Value::as_str)
        .or_else(|| object.get("bot_id").and_then(Value::as_str));
    if speaker.is_none() {
        *lossy = true;
    }
    if DROPPED.iter().any(|key| object.contains_key(*key)) {
        *lossy = true;
    }

    Some(Message {
        text,
        speaker: speaker.map(str::to_string),
        observed,
        // Slack records an instant and no offset: `ts` is epoch seconds, and the workspace's
        // timezone is a property of the workspace rather than of the message. Nothing to claim.
        tz_offset: None,
        ids,
    })
}

/// `"1705314225.000200"` → milliseconds. Slack's `ts` is epoch seconds with six decimals.
fn instant_of(ts: &str) -> Option<u64> {
    let (seconds, fraction) = match ts.split_once('.') {
        Some((s, f)) => (s, f),
        None => (ts, "0"),
    };
    let seconds: u64 = seconds.parse().ok()?;
    // Six digits of microseconds, of which three are milliseconds. A shorter fraction is
    // padded; a longer one is truncated, which loses nothing a segment row can hold.
    let mut millis = 0u64;
    for (i, c) in fraction.chars().take(3).enumerate() {
        let digit = c.to_digit(10)? as u64;
        millis += digit * 10u64.pow(2 - i as u32);
    }
    seconds.checked_mul(1000)?.checked_add(millis)
}

/// Slack's message text: angle-bracket spans and three HTML entities.
///
/// A link becomes its label, or its target where it has no label — the same decision `md/1`
/// makes, and the dropped target is why it is lossy. A mention (`<@U024BE7LH>`), a channel
/// reference (`<#C024BE91L|general>`) and a broadcast (`<!here>`) are kept **as written**,
/// because they are what the message says and because rewriting them would be this reader
/// inventing prose. That a user id in the text is not reached by `--pseudonymise` is stated in
/// [`crate::speaker`]: the text is `text redact`'s business, not a reader's.
///
/// The entities are unescaped **after** the spans are found, not before. Slack escapes a
/// literal `<` as `&lt;`, so unescaping first would turn a quoted `&lt;http://x|y&gt;` into a
/// link that nobody wrote.
fn prose(raw: &str, out: &mut String, lossy: &mut bool) {
    let mut rest = raw;
    while let Some(start) = rest.find('<') {
        unescape(&rest[..start], out);
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else {
            // An unbalanced `<` is a literal one. Slack would have escaped it; something else
            // wrote this file.
            unescape(&rest[start..], out);
            return;
        };
        let span = &after[..end];
        match span.chars().next() {
            Some('@') | Some('#') | Some('!') => {
                out.push('<');
                unescape(span, out);
                out.push('>');
            }
            _ => match span.split_once('|') {
                Some((_target, label)) => {
                    unescape(label, out);
                    *lossy = true;
                }
                None => unescape(span, out),
            },
        }
        rest = &after[end + 1..];
    }
    unescape(rest, out);
}

/// The three entities Slack escapes, and no others. It escapes exactly these, so a table is
/// the whole rule rather than a subset of HTML.
fn unescape(text: &str, out: &mut String) {
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let replaced = [("&amp;", '&'), ("&lt;", '<'), ("&gt;", '>')]
            .into_iter()
            .find(|(entity, _)| tail.starts_with(entity));
        match replaced {
            Some((entity, c)) => {
                out.push(c);
                rest = &tail[entity.len()..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::Caps;
    use crate::readers::read_with;
    use crate::structure::Structure;

    /// A minimal zip writer: stored entries, no compression.
    ///
    /// Hand-built rather than written with `zip`'s own writer, for the reason the conformance
    /// fixtures are hand-built: a test that produces its input with the same library it reads
    /// it with checks that the library agrees with itself.
    fn zip_of(entries: &[(&str, &str)]) -> Vec<u8> {
        fn crc32(bytes: &[u8]) -> u32 {
            let mut table = [0u32; 256];
            for (i, slot) in table.iter_mut().enumerate() {
                let mut c = i as u32;
                for _ in 0..8 {
                    c = if c & 1 == 1 {
                        0xEDB8_8320 ^ (c >> 1)
                    } else {
                        c >> 1
                    };
                }
                *slot = c;
            }
            let mut c = 0xFFFF_FFFFu32;
            for b in bytes {
                c = table[((c ^ u32::from(*b)) & 0xFF) as usize] ^ (c >> 8);
            }
            c ^ 0xFFFF_FFFF
        }

        let mut out: Vec<u8> = Vec::new();
        let mut directory: Vec<u8> = Vec::new();
        let mut count = 0u16;
        for (name, body) in entries {
            let offset = out.len() as u32;
            let crc = crc32(body.as_bytes());
            let size = body.len() as u32;
            let mut local = Vec::new();
            local.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            // Version 2.0, general-purpose flags with **bit 11 set** (the name is UTF-8),
            // stored, and a zero time and date. Bit 11 is not decoration: without it `zip`
            // decodes the name as CP437, which is correct of it and turns `проект` into
            // mojibake — found by this file's own refusal quoting the mangled name back.
            local.extend_from_slice(&[20, 0, 0, 8, 0, 0, 0, 0, 0, 0]);
            local.extend_from_slice(&crc.to_le_bytes());
            local.extend_from_slice(&size.to_le_bytes());
            local.extend_from_slice(&size.to_le_bytes());
            local.extend_from_slice(&(name.len() as u16).to_le_bytes());
            local.extend_from_slice(&0u16.to_le_bytes());
            local.extend_from_slice(name.as_bytes());
            local.extend_from_slice(body.as_bytes());
            out.extend_from_slice(&local);

            directory.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            directory.extend_from_slice(&[20, 0, 20, 0, 0, 8, 0, 0, 0, 0, 0, 0]);
            directory.extend_from_slice(&crc.to_le_bytes());
            directory.extend_from_slice(&size.to_le_bytes());
            directory.extend_from_slice(&size.to_le_bytes());
            directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
            directory.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            directory.extend_from_slice(&offset.to_le_bytes());
            directory.extend_from_slice(name.as_bytes());
            count += 1;
        }
        let dir_offset = out.len() as u32;
        let dir_size = directory.len() as u32;
        out.extend_from_slice(&directory);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&dir_size.to_le_bytes());
        out.extend_from_slice(&dir_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    const CHANNELS_JSON: &str =
        r#"[{"id":"C024BE91L","name":"general"},{"id":"C024BE92M","name":"random"}]"#;

    // 1705314225 is 2024-01-15T10:23:45Z; 1705395600 is 2024-01-16T09:00:00Z.
    const GENERAL_15: &str = r#"[
        {"type":"message","user":"U111","ts":"1705314225.000200","text":"First message."},
        {"type":"message","subtype":"channel_join","user":"U222","ts":"1705314300.000100",
         "text":"<@U222> has joined the channel"},
        {"type":"message","user":"U222","ts":"1705316400.000300","text":"Reply to it.",
         "thread_ts":"1705314225.000200"}
    ]"#;
    const GENERAL_16: &str = r#"[
        {"type":"message","user":"U111","ts":"1705395600.000000","text":"Next day."}
    ]"#;
    const RANDOM_15: &str = r#"[
        {"type":"message","user":"U333","ts":"1705314999.000000","text":"Elsewhere."}
    ]"#;

    fn export() -> Vec<u8> {
        zip_of(&[
            ("channels.json", CHANNELS_JSON),
            ("general/2024-01-15.json", GENERAL_15),
            ("general/2024-01-16.json", GENERAL_16),
            ("random/2024-01-15.json", RANDOM_15),
            ("users.json", r#"[{"id":"U111","name":"alice"}]"#),
        ])
    }

    fn read(bytes: &[u8]) -> ReadOutput {
        let mut budget = Budget::new(Caps::DEFAULT, bytes.len() as u64).expect("budget");
        read_with(&Slack, &Input::new(bytes), &Params::new(), &mut budget).expect("read")
    }

    /// Three levels, and the channels in name order whatever order the archive had them in.
    #[test]
    fn an_export_is_channels_of_days_of_messages() {
        let out = read(&export());
        let addresses: Vec<String> = out.rows.iter().map(|r| r.locator.to_string()).collect();
        assert_eq!(
            addresses,
            vec![
                "general",
                "general.20240115",
                "general.20240115.1",
                "general.20240115.2",
                "general.20240116",
                "general.20240116.1",
                "random",
                "random.20240115",
                "random.20240115.1",
            ]
        );
        assert_eq!(out.top_level.as_str(), chat::DAY);
    }

    /// The reading does not depend on the order the entries happen to be in.
    ///
    /// The property the reader contract requires, and the one an archive is most likely to
    /// break: two exports of one workspace, zipped in two orders, are one corpus.
    #[test]
    fn the_same_entries_in_another_order_are_the_same_reading() {
        let forward = read(&export());
        let backward = read(&zip_of(&[
            ("users.json", r#"[{"id":"U111","name":"alice"}]"#),
            ("random/2024-01-15.json", RANDOM_15),
            ("general/2024-01-16.json", GENERAL_16),
            ("general/2024-01-15.json", GENERAL_15),
            ("channels.json", CHANNELS_JSON),
        ]));
        assert_eq!(forward.text, backward.text);
        assert_eq!(forward.rows, backward.rows);
        assert_eq!(forward.raw, backward.raw);
        assert_eq!(forward.text.tid(), backward.text.tid(), "one text, one tid");
    }

    /// A day is the UTC day of the message's own timestamp, not the file it was in.
    ///
    /// `2024-01-15.json` here holds a message at `2024-01-16T00:30:00Z`, and it belongs to the
    /// 16th. A reader that trusted the file name would cut a part on a boundary that is in no
    /// field of the data.
    #[test]
    fn the_day_comes_from_the_timestamp_and_not_from_the_file_name() {
        let bytes = zip_of(&[(
            "general/2024-01-15.json",
            r#"[{"type":"message","user":"U1","ts":"1705314225.000000","text":"daytime"},
                {"type":"message","user":"U1","ts":"1705365000.000000","text":"after midnight"}]"#,
        )]);
        let out = read(&bytes);
        let addresses: Vec<String> = out.rows.iter().map(|r| r.locator.to_string()).collect();
        assert_eq!(
            addresses,
            vec![
                "general",
                "general.20240115",
                "general.20240115.1",
                "general.20240116",
                "general.20240116.1",
            ]
        );
    }

    /// The speaker is the user id, and the text holds no display name.
    #[test]
    fn the_speaker_is_the_user_id() {
        let out = read(&export());
        assert_eq!(out.rows[2].speaker.as_deref(), Some("U111"));
        assert_eq!(out.rows[3].speaker.as_deref(), Some("U222"));
        assert!(
            !out.text.as_str().contains("alice"),
            "{}",
            out.text.as_str()
        );
    }

    /// The message id is `ts`, and a thread parent is kept beside it.
    #[test]
    fn a_message_keeps_its_timestamp_id_and_its_thread() {
        let out = read(&export());
        assert_eq!(
            out.rows[3].ids.get("msg").map(String::as_str),
            Some("1705316400.000300")
        );
        assert_eq!(
            out.rows[3].ids.get("thread").map(String::as_str),
            Some("1705314225.000200")
        );
        assert_eq!(out.rows[3].observed, Some(1_705_316_400_000));
    }

    /// A channel event is not a sentence this reader attributes to anybody.
    #[test]
    fn a_join_is_an_event_and_not_prose() {
        let out = read(&export());
        assert!(
            !out.text.as_str().contains("joined"),
            "{}",
            out.text.as_str()
        );
        assert!(out.lossy);
    }

    /// The channel list is kept as `raw`, canonically encoded.
    #[test]
    fn the_channel_list_becomes_raw_metadata() {
        let out = read(&export());
        let raw = out.raw.expect("raw metadata");
        let mut d = smysl_core::cbor::Dec::new(&raw);
        assert_eq!(d.map_head().expect("a map"), 2);
        // Encoded-key order: `random` (6 bytes) before `general` (7).
        assert_eq!(d.text().expect("a key"), "random");
        assert_eq!(d.text().expect("a value"), "C024BE92M");
        assert_eq!(d.text().expect("a key"), "general");
    }

    /// Slack's markup: a link becomes its label, a mention stays as written, entities unescape.
    #[test]
    fn the_markup_becomes_the_characters_it_stood_for() {
        let bytes = zip_of(&[(
            "general/2024-01-15.json",
            r#"[{"type":"message","user":"U1","ts":"1705314225.000000",
                 "text":"see <https://example.org|the paper> and ask <@U222> about 3 &lt; 5 &amp; more"},
                {"type":"message","user":"U1","ts":"1705314226.000000",
                 "text":"bare <https://example.org> and &lt;not a link&gt;"}]"#,
        )]);
        let out = read(&bytes);
        let text = out.text.as_str();
        assert!(
            text.contains("see the paper and ask <@U222> about 3 < 5 & more"),
            "{text}"
        );
        assert!(
            text.contains("bare https://example.org and <not a link>"),
            "{text}"
        );
        assert!(out.lossy, "the link target was dropped");
    }

    /// An entry that is not a message file is skipped, and skipping it is reported.
    #[test]
    fn what_a_text_library_does_not_hold_is_reported_as_dropped() {
        let out = read(&export());
        assert!(out.lossy, "users.json was not read");
        let plain = read(&zip_of(&[(
            "general/2024-01-15.json",
            r#"[{"type":"message","user":"U1","ts":"1705314225.000000","text":"only this"}]"#,
        )]));
        assert!(
            !plain.lossy,
            "an archive of nothing but messages drops nothing"
        );
    }

    /// An archive naming one channel-day twice is one entry by the time this reader sees it.
    ///
    /// Written as a test of the **library**, because that is where the behaviour is: `zip`
    /// keys the central directory by name, so the duplicate is resolved before any of this
    /// crate's code runs. The reader's own refusal for the case was deleted once this test
    /// showed it could not be reached, and the test stays to say why there is none.
    #[test]
    fn a_duplicated_entry_name_never_reaches_this_reader() {
        let bytes = zip_of(&[
            ("general/2024-01-15.json", RANDOM_15),
            ("general/2024-01-15.json", RANDOM_15),
        ]);
        let archive = zip::ZipArchive::new(Cursor::new(&bytes[..])).expect("an archive");
        assert_eq!(archive.len(), 1, "two entries, one name, one member");

        let out = read(&bytes);
        let addresses: Vec<String> = out.rows.iter().map(|r| r.locator.to_string()).collect();
        assert_eq!(
            addresses,
            vec!["general", "general.20240115", "general.20240115.1"]
        );
    }

    /// A channel name with no ASCII letter cannot be a locator head, and says so.
    #[test]
    fn a_channel_name_that_cannot_be_a_locator_is_refused_by_name() {
        let bytes = zip_of(&[("проект/2024-01-15.json", RANDOM_15)]);
        let mut budget = Budget::new(Caps::DEFAULT, bytes.len() as u64).expect("budget");
        let err = read_with(&Slack, &Input::new(&bytes), &Params::new(), &mut budget)
            .expect_err("refused");
        match err {
            LibError::Unreadable { what, .. } => assert!(what.contains("проект"), "{what}"),
            other => panic!("{other:?}"),
        }
    }

    /// Bytes that are not an archive are refused rather than guessed at.
    #[test]
    fn bytes_that_are_not_a_zip_are_refused() {
        let mut budget = Budget::new(Caps::DEFAULT, 16).expect("budget");
        let err = read_with(
            &Slack,
            &Input::new(b"PK not really"),
            &Params::new(),
            &mut budget,
        )
        .expect_err("refused");
        match err {
            LibError::Unreadable { what, .. } => assert!(what.contains("zip"), "{what}"),
            other => panic!("{other:?}"),
        }
    }

    /// The entry cap is charged per archive member, read or not.
    #[test]
    fn too_many_entries_is_a_cap_refusal() {
        let mut caps = Caps::DEFAULT;
        caps.entries = 2;
        let bytes = export();
        let mut budget = Budget::new(caps, bytes.len() as u64).expect("budget");
        let err = read_with(&Slack, &Input::new(&bytes), &Params::new(), &mut budget)
            .expect_err("refused");
        match err {
            LibError::Limit { cap, limit, .. } => {
                assert_eq!(cap, "entries");
                assert_eq!(limit, 2);
            }
            other => panic!("{other:?}"),
        }
    }

    /// An entry larger than the cap is refused before it is decompressed.
    #[test]
    fn an_oversized_entry_is_refused_before_it_is_read() {
        let mut caps = Caps::DEFAULT;
        caps.entry_bytes = 16;
        let bytes = export();
        let mut budget = Budget::new(caps, bytes.len() as u64).expect("budget");
        let err = read_with(&Slack, &Input::new(&bytes), &Params::new(), &mut budget)
            .expect_err("refused");
        match err {
            LibError::Limit { cap, .. } => assert_eq!(cap, "entry_bytes"),
            other => panic!("{other:?}"),
        }
    }

    /// The rows are a structure, and every locator resolves in it.
    #[test]
    fn the_rows_build_a_structure_that_resolves_every_locator() {
        let out = read(&export());
        let mut budget = Budget::new(Caps::DEFAULT, 1 << 16).expect("budget");
        let structure =
            Structure::build(&out.rows, out.text.len() as u64, &mut budget).expect("a valid table");
        for row in &out.rows {
            assert_eq!(
                structure.resolve(&row.locator),
                Some(row.range()),
                "{:?}",
                row.locator
            );
        }
        assert_eq!(structure.roots().len(), 2, "two channels, two roots");
    }

    /// Slack's `ts` is seconds with six decimals, and the milliseconds survive.
    #[test]
    fn a_slack_timestamp_is_seconds_and_microseconds() {
        assert_eq!(instant_of("1705314225.000200"), Some(1_705_314_225_000));
        assert_eq!(instant_of("1705314225.123456"), Some(1_705_314_225_123));
        assert_eq!(instant_of("1705314225.5"), Some(1_705_314_225_500));
        assert_eq!(instant_of("1705314225"), Some(1_705_314_225_000));
        assert_eq!(instant_of("not a time"), None);
    }
}
