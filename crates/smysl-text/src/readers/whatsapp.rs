//! `whatsapp/1` — the `_chat.txt` a WhatsApp export contains.
//!
//! The only one of the three chat formats that is not a data format. It is a transcript
//! intended for a person to read, written by a phone, in that phone's locale, and almost every
//! decision in this file is about what can and cannot be recovered from that.
//!
//! ```text
//! [15/01/2024, 10:23:45] Alice: Прочитал первую главу.
//! 15/01/2024, 10:23 - Alice: the Android shape of the same line
//! ```
//!
//! # Why the date order is a required parameter
//!
//! `03/04/2024` is the third of April or the fourth of March, and the file does not say which.
//! The order comes from the locale of the phone that wrote it, which is nowhere in the bytes.
//! Both readings are complete, plausible conversations; they differ in which **day** a message
//! falls on, and a day is a part, and a part is a tid. So `--param date-format=dmy|mdy|ymd` is
//! required: the one thing worse than asking is guessing, because a guess is not detectable
//! afterwards.
//!
//! This is the first reader in the format that takes a parameter, and that is why manifest key
//! 3 now records a reader id **with its settings** — `whatsapp/1 date-format=dmy tz=+0300` —
//! rather than an id alone. `readers::reader_field` writes it, the format spec describes it,
//! and the three ports read it back.
//!
//! # Why the timezone is not
//!
//! The clock in the file is also local, and also unlabelled. Unlike the date order it has a
//! defensible default: with no `tz`, the times are read as UTC, which is what SMYSL-2.4 §3.2's
//! "one part per UTC day" already says the partition is in. The cost is stated rather than
//! hidden — the day boundaries are then the writer's local midnight only if the writer lived
//! at UTC — and `--param tz=±HHMM` is how somebody who knows says so. The date order has no
//! such default: there is no sense in which `dmy` is the neutral reading of `03/04`.
//!
//! # What this reader does *not* try to recognise
//!
//! `<Media omitted>`, `image omitted`, `<attached: …>` — the placeholders WhatsApp leaves where
//! a photo was. They are **kept, as text**, and the read is not marked lossy. Two reasons, and
//! the second is the one that decides it:
//!
//! - It was the *exporter* that dropped the photo, not this reader. `lossy` means "the reader
//!   dropped something the source carried", and the source carries a placeholder.
//! - The placeholders are **localised**. A Russian export says `<Без медиафайлов>`, a German one
//!   `<Medien ausgeschlossen>`. A list of them would be a list that is wrong for the next
//!   locale, and a reader that silently dropped a line matching an English string would read
//!   two exports of one conversation as two different texts.
//!
//! A line with a timestamp and **no sender** is a different matter: WhatsApp's own notices
//! ("Messages and calls are end-to-end encrypted") have no speaker, and they are skipped with
//! the read marked lossy, for the reason `telegram/1` skips a service message — a corpus
//! should not hold sentences its reader attributed to nobody.
//!
//! The known limit of that rule, recorded rather than papered over: a sender is whatever
//! precedes the first `": "`, so a localised notice that happens to contain one is read as a
//! message from a speaker whose name is that notice's first half. Nothing in the file
//! distinguishes the two cases, and the alternative — a list of notices per locale — is the
//! thing the paragraph above rejects.

use std::collections::BTreeMap;

use crate::limits::Budget;
use crate::norm::Normalised;
use crate::readers::build::Doc;
use crate::readers::chat::{self, Message};
use crate::readers::{Input, ParamSpec, Params, ReadOutput, Reader};
use crate::reading::Level;
use crate::LibError;

const ID: &str = "whatsapp/1";

/// The locator head, as in every chat reader: one export is one conversation.
const HEAD: &str = "chat";

const PARAMS: &[ParamSpec] = &[
    ParamSpec {
        key: "date-format",
        required: true,
        what: "the order of day, month and year in a timestamp: `dmy`, `mdy` or `ymd`",
    },
    ParamSpec {
        key: "tz",
        required: false,
        what: "the UTC offset the transcript's clock was written in, `±HHMM` or `Z` \
               (default: the times are read as UTC)",
    },
];

/// Which of the three numbers in a date is which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Order {
    Dmy,
    Mdy,
    Ymd,
}

impl Order {
    fn parse(s: &str) -> Option<Order> {
        match s {
            "dmy" => Some(Order::Dmy),
            "mdy" => Some(Order::Mdy),
            "ymd" => Some(Order::Ymd),
            _ => None,
        }
    }

    /// `(year, month, day)` out of the three numbers in the order they were written.
    fn apply(self, a: i64, b: i64, c: i64) -> (i64, i64, i64) {
        match self {
            Order::Dmy => (c, b, a),
            Order::Mdy => (c, a, b),
            Order::Ymd => (a, b, c),
        }
    }
}

/// The bidi marks WhatsApp's exporter sprinkles around its own punctuation.
///
/// U+200E and U+200F are invisible, carry no prose, and appear in positions chosen by the
/// exporter's layout rather than by anybody writing. They are removed from the **start** of a
/// line only, which is where they interfere with recognising a timestamp; one inside a message
/// stays, because this reader is not in a position to decide that a character in the middle of
/// what somebody wrote is an artefact.
const BIDI: &[char] = &['\u{200e}', '\u{200f}'];

pub struct Whatsapp;

impl Reader for Whatsapp {
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
        // `check` has already refused a missing `date-format` by the time a reader runs
        // (`read_with` validates first), so this is the value being unreadable rather than
        // absent.
        let spelled = params
            .get("date-format")
            .ok_or_else(|| LibError::BadParam {
                reader: ID.to_string(),
                key: "date-format".to_string(),
                reason: PARAMS[0].what.to_string(),
            })?;
        let order = Order::parse(spelled).ok_or_else(|| LibError::BadParam {
            reader: ID.to_string(),
            key: "date-format".to_string(),
            reason: format!("`{spelled}` is not `dmy`, `mdy` or `ymd`"),
        })?;
        let offset = match params.get("tz") {
            Some(text) => Some(chat::offset_of(text).ok_or_else(|| LibError::BadParam {
                reader: ID.to_string(),
                key: "tz".to_string(),
                reason: format!("`{text}` is not `±HHMM` or `Z`"),
            })?),
            None => None,
        };

        let source = Normalised::new(input.bytes())?;
        let mut lossy = false;
        let mut messages: Vec<Message> = Vec::new();
        for (index, line) in source.as_str().split('\n').enumerate() {
            limits.fuel(1)?;
            let line = line.trim_start_matches(BIDI);
            match header(line, order) {
                Some((stamp, rest)) => {
                    let observed = stamp.instant(offset).ok_or_else(|| LibError::Unreadable {
                        reader: ID.to_string(),
                        at: index,
                        what: format!(
                            "a date the calendar has, read as `{spelled}`, not `{}`",
                            stamp.written
                        ),
                    })?;
                    match sender_and_body(rest) {
                        Some((who, body)) => messages.push(Message {
                            text: body.to_string(),
                            speaker: Some(who.to_string()),
                            observed,
                            tz_offset: offset,
                            ids: BTreeMap::new(),
                        }),
                        // A notice with a time and no speaker. See the module header.
                        None => lossy = true,
                    }
                }
                None => {
                    // A continuation of the message above: a second line of what somebody
                    // wrote. A line before the first timestamp is not that — it belongs to no
                    // message, so it is dropped and said to be dropped.
                    if line.trim().is_empty() {
                        continue;
                    }
                    match messages.last_mut() {
                        Some(previous) => {
                            previous.text.push('\n');
                            previous.text.push_str(line);
                        }
                        None => lossy = true,
                    }
                }
            }
        }

        let mut doc = Doc::new(ID);
        chat::conversation(&mut doc, ID, HEAD, &messages, limits)?;
        let (text, rows) = doc.finish()?;
        let mut out = ReadOutput::new(
            text,
            rows,
            Level::new(chat::DAY).expect("`day` is a valid level"),
        );
        out.lossy = lossy;
        Ok(out)
    }
}

/// A timestamp as the transcript wrote it, with the numbers already in calendar order.
struct Stamp<'a> {
    year: i64,
    month: i64,
    day: i64,
    hour: u32,
    minute: u32,
    second: u32,
    /// The text the numbers came from, for a refusal to quote.
    written: &'a str,
}

impl Stamp<'_> {
    fn instant(&self, offset: Option<i32>) -> Option<u64> {
        let year = u32::try_from(self.year).ok()?;
        let month = u32::try_from(self.month).ok()?;
        let day = u32::try_from(self.day).ok()?;
        let wall = chat::instant(
            i64::from(year),
            month,
            day,
            self.hour,
            self.minute,
            self.second,
        )?;
        let shift = i64::from(offset.unwrap_or(0)) * 60_000;
        u64::try_from(wall as i64 - shift).ok()
    }
}

/// Split a line into its timestamp and the rest, in either of the two shapes WhatsApp writes.
fn header(line: &str, order: Order) -> Option<(Stamp<'_>, &str)> {
    let (stamp, rest) = if let Some(after) = line.strip_prefix('[') {
        let close = after.find(']')?;
        (&after[..close], after[close + 1..].trim_start_matches(BIDI))
    } else {
        // `" - "` and not `"-"`: a date written `15-01-2024` holds the same character, and
        // splitting on the first one would cut the date in half.
        let at = line.find(" - ")?;
        (&line[..at], &line[at + 3..])
    };
    Some((timestamp(stamp.trim(), order)?, rest.trim_start()))
}

/// `15/01/2024, 10:23:45` or `1/15/24, 10:23 PM`, in the order the caller declared.
fn timestamp(text: &str, order: Order) -> Option<Stamp<'_>> {
    let (date, time) = text.split_once(',')?;
    let mut parts = date.trim().split(['/', '.', '-']).map(str::trim);
    let a: i64 = parts.next()?.parse().ok()?;
    let b: i64 = parts.next()?.parse().ok()?;
    let c: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let (year, month, day) = order.apply(a, b, c);
    // A two-digit year is this century. WhatsApp did not exist in 1924, and the alternative —
    // refusing the shape — would refuse a real export: the `mdy` locale writes `1/15/24`.
    let year = if year < 100 { 2000 + year } else { year };

    let time = time.trim();
    // `AM`/`PM`, where the locale uses a twelve-hour clock. Case-insensitive, and the
    // separator may be a space or a non-breaking space.
    let (clock, half) = match time.rsplit_once(|c: char| c.is_whitespace()) {
        Some((head, tail))
            if tail.eq_ignore_ascii_case("am") || tail.eq_ignore_ascii_case("pm") =>
        {
            (head.trim(), Some(tail.to_ascii_lowercase()))
        }
        _ => (time, None),
    };
    let mut hms = clock.split(':').map(str::trim);
    let mut hour: u32 = hms.next()?.parse().ok()?;
    let minute: u32 = hms.next()?.parse().ok()?;
    let second: u32 = match hms.next() {
        Some(s) => s.parse().ok()?,
        None => 0,
    };
    if hms.next().is_some() {
        return None;
    }
    match half.as_deref() {
        // 12 AM is midnight and 12 PM is noon, which is the one case a `+ 12` gets wrong in
        // both directions.
        Some("am") if hour == 12 => hour = 0,
        Some("pm") if hour < 12 => hour += 12,
        Some("pm") | Some("am") => {}
        Some(_) => return None,
        None => {}
    }
    Some(Stamp {
        year,
        month,
        day,
        hour,
        minute,
        second,
        written: text,
    })
}

/// `Alice: hello` → `("Alice", "hello")`. `None` for a line with no sender at all.
fn sender_and_body(rest: &str) -> Option<(&str, &str)> {
    let (who, body) = rest.split_once(": ")?;
    let who = who.trim().trim_start_matches(BIDI).trim();
    if who.is_empty() {
        return None;
    }
    Some((who, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::Caps;
    use crate::readers::read_with;
    use crate::structure::Structure;

    fn params(pairs: &[(&str, &str)]) -> Params {
        let mut p = Params::new();
        for (k, v) in pairs {
            p.set(ID, *k, *v).expect("set");
        }
        p
    }

    fn read_as(bytes: &[u8], pairs: &[(&str, &str)]) -> Result<ReadOutput, LibError> {
        let mut budget = Budget::new(Caps::DEFAULT, bytes.len() as u64).expect("budget");
        read_with(&Whatsapp, &Input::new(bytes), &params(pairs), &mut budget)
    }

    fn read(bytes: &[u8]) -> ReadOutput {
        read_as(bytes, &[("date-format", "dmy")]).expect("read")
    }

    const IOS: &str = "\u{200e}[15/01/2024, 10:23:45] Alice: Прочитал первую главу.\n\
                       [15/01/2024, 11:00:02] Bob: И что скажешь?\n\
                       Вторая строка того же сообщения.\n\
                       [16/01/2024, 09:00:00] Alice: Следующий день.\n";

    const ANDROID: &str = "15/01/2024, 10:23 - Alice: first\n\
                           15/01/2024, 22:05 - Bob: second\n";

    /// Both shapes read, and both give the structure §3.2 asks for.
    #[test]
    fn the_two_shapes_whatsapp_writes_are_both_read() {
        for text in [IOS, ANDROID] {
            let out = read(text.as_bytes());
            assert_eq!(out.top_level.as_str(), chat::DAY);
            assert!(
                out.rows.len() >= 3,
                "a day and its messages: {:?}",
                out.rows.len()
            );
            assert_eq!(out.rows[0].locator.to_string(), "chat.20240115");
        }
    }

    /// The sender leaves the text and lands in the row, as in every chat reader.
    #[test]
    fn the_sender_is_the_speaker_and_not_part_of_the_prose() {
        let out = read(IOS.as_bytes());
        let text = out.text.as_str();
        assert!(!text.contains("Alice"), "{text}");
        assert!(!text.contains("10:23"), "{text}");
        assert_eq!(out.rows[1].speaker.as_deref(), Some("Alice"));
        assert_eq!(out.rows[2].speaker.as_deref(), Some("Bob"));
        let slice =
            |i: usize| text[out.rows[i].start as usize..out.rows[i].end as usize].to_string();
        assert_eq!(slice(1), "Прочитал первую главу.");
    }

    /// A line with no timestamp is the rest of the message above it.
    #[test]
    fn a_line_without_a_timestamp_continues_the_message_above() {
        let out = read(IOS.as_bytes());
        let text = out.text.as_str();
        let second = &text[out.rows[2].start as usize..out.rows[2].end as usize];
        assert_eq!(second, "И что скажешь? Вторая строка того же сообщения.");
        assert_eq!(out.rows.len(), 5, "two days, three messages");
    }

    /// The date order is applied, and it decides which day a message is on.
    ///
    /// `03/04/2024` is two different days, and the parameter is the only thing that says which.
    #[test]
    fn the_declared_order_decides_what_the_numbers_mean() {
        let line = b"03/04/2024, 12:00:00 - Alice: ambiguous\n";
        let dmy = read_as(line, &[("date-format", "dmy")]).expect("read");
        assert_eq!(dmy.rows[0].locator.to_string(), "chat.20240403");
        let mdy = read_as(line, &[("date-format", "mdy")]).expect("read");
        assert_eq!(mdy.rows[0].locator.to_string(), "chat.20240304");
        let ymd = read_as(
            b"2024-04-03, 12:00:00 - Alice: iso\n",
            &[("date-format", "ymd")],
        )
        .expect("read");
        assert_eq!(ymd.rows[0].locator.to_string(), "chat.20240403");
    }

    /// No `date-format` is a refusal before a byte is read, and the message says what it means.
    #[test]
    fn the_date_order_is_required_and_refused_by_name() {
        let err = read_as(IOS.as_bytes(), &[]).expect_err("refused");
        match err {
            LibError::BadParam { key, reason, .. } => {
                assert_eq!(key, "date-format");
                assert!(reason.contains("day, month and year"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    /// A date order this reader does not have is refused, not approximated.
    #[test]
    fn an_unknown_date_order_is_refused() {
        let err = read_as(IOS.as_bytes(), &[("date-format", "ydm")]).expect_err("refused");
        match err {
            LibError::BadParam { key, reason, .. } => {
                assert_eq!(key, "date-format");
                assert!(reason.contains("dmy"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    /// A twelve-hour clock, including the two hours that are not `+ 12`.
    #[test]
    fn a_twelve_hour_clock_is_read_including_noon_and_midnight() {
        let lines = b"15/01/2024, 12:00 AM - A: midnight\n\
                      15/01/2024, 12:30 PM - A: noon\n\
                      15/01/2024, 11:45 PM - A: late\n";
        let out = read(lines);
        let at = |i: usize| out.rows[i].observed.expect("an instant");
        assert_eq!(at(1), chat::instant(2024, 1, 15, 0, 0, 0).expect("ms"));
        assert_eq!(at(2), chat::instant(2024, 1, 15, 12, 30, 0).expect("ms"));
        assert_eq!(at(3), chat::instant(2024, 1, 15, 23, 45, 0).expect("ms"));
    }

    /// A two-digit year is this century, because the alternative refuses a real export.
    #[test]
    fn a_two_digit_year_is_this_century() {
        let out = read(b"15/01/24, 10:00 - A: short year\n");
        assert_eq!(out.rows[0].locator.to_string(), "chat.20240115");
    }

    /// Without `tz` the clock is UTC; with it, the instant and possibly the day move.
    #[test]
    fn the_offset_moves_the_instant_and_can_move_the_day() {
        let line = b"15/01/2024, 23:30:00 - Alice: late\n";
        let utc = read(line);
        assert_eq!(utc.rows[0].locator.to_string(), "chat.20240115");
        assert_eq!(utc.rows[1].tz_offset, None, "nothing was claimed");

        let west = read_as(line, &[("date-format", "dmy"), ("tz", "-0500")]).expect("read");
        assert_eq!(west.rows[0].locator.to_string(), "chat.20240116");
        assert_eq!(west.rows[1].tz_offset, Some(-300));
    }

    /// A notice with a time and no sender is skipped, and the read says it dropped something.
    #[test]
    fn a_notice_with_no_sender_is_not_attributed_to_anybody() {
        let lines = "[15/01/2024, 10:00:00] Messages and calls are end-to-end encrypted.\n\
                     [15/01/2024, 10:23:45] Alice: hello\n";
        let out = read(lines.as_bytes());
        assert_eq!(out.text.as_str(), "hello");
        assert!(out.lossy);
        assert_eq!(out.rows.len(), 2);
    }

    /// A media placeholder is text, and keeping it is not a loss.
    ///
    /// The exporter dropped the photo; this reader dropped nothing. The placeholder is
    /// localised, so recognising it would be a list that is wrong for the next locale.
    #[test]
    fn a_media_placeholder_is_kept_as_the_text_it_is() {
        let lines = "[15/01/2024, 10:00:00] Alice: <Media omitted>\n\
                     [15/01/2024, 10:01:00] Bob: \u{200e}<Без медиафайлов>\n";
        let out = read(lines.as_bytes());
        assert!(out.text.as_str().contains("<Media omitted>"));
        assert!(out.text.as_str().contains("<Без медиафайлов>"));
        assert!(!out.lossy, "the reader dropped nothing");
    }

    /// A date the calendar does not have is a refusal naming the line.
    #[test]
    fn a_date_that_does_not_exist_is_refused_with_the_text_it_came_from() {
        let err = read_as(b"31/02/2024, 10:00 - A: nope\n", &[("date-format", "dmy")])
            .expect_err("refused");
        match err {
            LibError::Unreadable { what, at, .. } => {
                assert!(what.contains("31/02/2024"), "{what}");
                assert_eq!(at, 0, "the line it was on");
            }
            other => panic!("{other:?}"),
        }
    }

    /// A text that is not a transcript at all reads as a conversation with nothing in it.
    ///
    /// Not a refusal: a file with no timestamps has no messages, and `SMY-E4xx`-free emptiness
    /// is what `Library::add` turns into `NoParts` — which names the level and the reader, and
    /// is a better diagnostic than this reader could give.
    #[test]
    fn a_file_with_no_timestamps_has_no_messages() {
        let out = read(b"just some prose\nwith no timestamps at all\n");
        assert!(out.rows.is_empty());
        assert!(out.text.is_empty());
        assert!(out.lossy, "every line was dropped, and that is reported");
    }

    /// The rows are a structure, and every locator resolves in it.
    #[test]
    fn the_rows_build_a_structure_that_resolves_every_locator() {
        let out = read(IOS.as_bytes());
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
    }

    /// Invalid UTF-8 is refused before anything is parsed.
    #[test]
    fn bytes_that_are_not_text_are_refused() {
        let err = read_as(
            b"[15/01/2024, 10:00:00] A: \xff\n",
            &[("date-format", "dmy")],
        )
        .expect_err("refused");
        assert!(matches!(err, LibError::NotText { .. }), "{err:?}");
    }

    /// The parameters round-trip through the field a manifest records.
    ///
    /// This reader is why that field has a grammar at all: it is the first one whose output
    /// depends on a setting, so `whatsapp/1 date-format=dmy` is what a corpus has to record to
    /// mean the same thing on re-read.
    #[test]
    fn the_settings_are_what_a_manifest_records_beside_the_id() {
        let p = params(&[("date-format", "dmy"), ("tz", "+0300")]);
        let field = crate::readers::reader_field(ID, &p);
        assert_eq!(field, "whatsapp/1 date-format=dmy tz=+0300");
        let (id, back) = crate::readers::parse_reader_field(&field).expect("parsed");
        assert_eq!(id, ID);
        assert_eq!(back, p);
    }
}
