//! What the three chat readers share: a day, a message, and the locator that names one.
//!
//! Telegram, Slack and WhatsApp export three unrelated things — a JSON object, a zip of
//! per-day JSON arrays, and a text file — and agree on almost nothing except what a
//! conversation *is*: an ordered run of messages, each with a time, a sender and some prose.
//! So what lives here is the part that is not about a file format, the way
//! `readers::build` holds the part of the scripture readers that is not about markup.
//!
//! # One part per UTC day
//!
//! SMYSL-2.4 §3.2: "one segment per message, one part per UTC day by default". Both halves are
//! here. The structure is `day` → `message`, [`ReadOutput::top_level`](crate::readers::ReadOutput)
//! is `day`, and a part policy that says nothing cuts there.
//!
//! **UTC, and not the writer's day.** A chat export is written by someone who lives in a
//! timezone, and their idea of Tuesday is not the format's. Choosing their day would mean the
//! same messages fall into different parts depending on a field that two of the three formats
//! do not carry — and a part boundary is a tid. So the day is UTC, the offset the source gave
//! is kept per row (segment key 8) for anyone who wants to ask the other question, and the
//! readers that know an offset at all say so.
//!
//! # The locator
//!
//! `head.yyyymmdd.n`: the conversation, the UTC day, and the message's ordinal **within that
//! day**, counting from 1. `chat.20240115.7` is the seventh message of 15 January 2024.
//!
//! The ordinal rather than the platform's own message id, although Telegram has one and Slack
//! has a timestamp that works like one. Three reasons, in order of weight: WhatsApp has
//! nothing of the kind, so a uniform locator has to be computable from position alone; the
//! locator is the one thing in a reading a person types by hand (`t3:…#chat.20240115.7`), and
//! `1705314225.000200` is not that; and the platform's id is kept anyway, in the row's `ids`,
//! where a tool can read it without anybody having to type it.
//!
//! The ordinal is stable under appending, which is what `text append` needs: a later message is
//! a later ordinal, and nothing already written moves.
//!
//! **Out of order is a refusal.** A file whose messages are not in time order would open one
//! day twice, and `Doc::open` refuses a repeated address —
//! so the reading is refused rather than silently holding two nodes that each claim to be a
//! day. A chat export has no reason to be out of order, and one that is has been edited.
//!
//! # The calendar here is temporary
//!
//! TX-P3 builds the time engine — `Instant`, EDTF, intervals — and this conversion belongs in
//! it. Until it exists, a reader that cannot turn a Unix timestamp into a date cannot group
//! messages by day at all, so the two directions are written here, in about forty lines, with
//! no dependency. They are proleptic Gregorian, UTC, and have no notion of a leap second,
//! which is what Unix time has too.

use std::collections::BTreeMap;

use crate::limits::Budget;
use crate::locator::Locator;
use crate::readers::build::Doc;
use crate::reading::Segment;
use crate::LibError;

/// The level a part boundary falls on.
pub(crate) const DAY: &str = "day";

/// The level one message sits at.
pub(crate) const MESSAGE: &str = "message";

/// The level a Slack export's channels sit at, above the days.
#[cfg(feature = "reader-slack")]
pub(crate) const CHANNEL: &str = "channel";

/// Milliseconds in a day. Unix time has no leap seconds, so this is exact.
const MS_PER_DAY: i64 = 86_400_000;

/// One message, as a reader found it.
///
/// `text` is the prose and nothing else: not the sender, not the timestamp, not a rendering of
/// either. That is what makes a pseudonymised library and a plain one hold the same bytes —
/// see [`crate::speaker`] — and it is why a chat corpus can be searched for what people said
/// rather than for how an exporter laid it out.
#[derive(Debug, Clone, Default)]
pub(crate) struct Message {
    pub text: String,
    /// The platform's identifier for the sender, as the source spelled it. `None` for a
    /// message whose sender the export does not name.
    pub speaker: Option<String>,
    /// Milliseconds since the Unix epoch, UTC.
    pub observed: u64,
    /// Minutes east of UTC, where the source said. `None` where it did not.
    pub tz_offset: Option<i32>,
    /// The platform's own identifiers: a message id, a reply id, a thread id.
    pub ids: BTreeMap<String, String>,
}

/// A calendar date, UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Date {
    pub year: i64,
    pub month: u32,
    pub day: u32,
}

impl Date {
    /// `yyyymmdd`, which is the locator's middle step.
    ///
    /// A single number rather than three steps, because `chat.2024.1.15.7` would put the day
    /// and the message at the same syntactic level and a reader of the locator could not tell
    /// which trailing number was which.
    pub fn stamp(&self) -> u64 {
        self.year as u64 * 10_000 + u64::from(self.month) * 100 + u64::from(self.day)
    }
}

/// The UTC date of a Unix millisecond timestamp.
///
/// Howard Hinnant's `civil_from_days`, which is exact for the whole range of the type and has
/// no table in it. The era arithmetic is what makes that true; the comments name the steps
/// rather than re-deriving them.
pub(crate) fn date_of(ms: u64) -> Date {
    let days = (ms as i64).div_euclid(MS_PER_DAY);
    // Shift the epoch to 0000-03-01 so that a leap day is the last day of a year.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // day of era, 0..=146_096
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // 0..=399
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // 0..=365, March-based
    let mp = (5 * doy + 2) / 153; // 0..=11, March-based month
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    Date {
        year: y + i64::from(m <= 2),
        month: m as u32,
        day: d as u32,
    }
}

#[cfg(any(feature = "reader-telegram", feature = "reader-whatsapp"))]
/// Days since the Unix epoch of a UTC calendar date, the inverse of [`date_of`].
///
/// `None` for a date outside the proleptic Gregorian calendar's own rules — month 13, day 32,
/// 29 February in a common year. The readers that parse a date by hand need the refusal, not a
/// wrap-around: `31/02/2024` in a WhatsApp export is a line that is not a timestamp, and
/// reading it as 2 March would put a message on a day it was not sent.
pub(crate) fn days_from_civil(year: i64, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return None;
    }
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * i64::from(mp) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

#[cfg(any(feature = "reader-telegram", feature = "reader-whatsapp"))]
fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

#[cfg(any(feature = "reader-telegram", feature = "reader-whatsapp"))]
fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

#[cfg(any(feature = "reader-telegram", feature = "reader-whatsapp"))]
/// The millisecond timestamp of a UTC date and time of day.
///
/// `None` for a date the calendar does not have, a time of day outside `00:00:00..24:00:00`,
/// or an instant before the Unix epoch. The last one is a limit of the format rather than of
/// the arithmetic: a segment's `observed` is an unsigned millisecond count (record 18 key 6),
/// so a reader cannot record 1969 and must say so instead of recording something else.
pub(crate) fn instant(
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> Option<u64> {
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    let secs = days
        .checked_mul(86_400)?
        .checked_add(i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second))?;
    u64::try_from(secs.checked_mul(1000)?).ok()
}

#[cfg(any(feature = "reader-telegram", feature = "reader-whatsapp"))]
/// `±HHMM` or `Z` as minutes east of UTC.
///
/// Minutes and not hours: India is `+0530` and Nepal is `+0545`, and an offset in whole hours
/// would be wrong for a fifth of the world.
pub(crate) fn offset_of(text: &str) -> Option<i32> {
    if text == "Z" || text == "z" {
        return Some(0);
    }
    let bytes = text.as_bytes();
    if bytes.len() != 5 || !bytes[1..].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let sign = match bytes[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let hours: i32 = text[1..3].parse().ok()?;
    let minutes: i32 = text[3..5].parse().ok()?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some(sign * (hours * 60 + minutes))
}

/// Write one conversation's messages into `doc` as `day` → `message` rows.
///
/// `head` is the locator's identifier part: the conversation. The caller opens any level above
/// the days — a Slack export opens a `channel` — and this function leaves the day and message
/// levels closed behind it, so a second call for a second conversation starts clean.
///
/// Returns the number of messages that produced a row. A message with no prose produces none:
/// its row would be a zero-length range, which
/// `Doc`'s own rule already removes, and an empty row is a node
/// no span can attach to and no locator can usefully resolve.
pub(crate) fn conversation(
    doc: &mut Doc,
    reader: &'static str,
    head: &str,
    messages: &[Message],
    limits: &mut Budget,
) -> Result<usize, LibError> {
    let mut written = 0usize;
    let mut current: Option<u64> = None;
    let mut ordinal: u64 = 0;
    for message in messages {
        if message.text.trim().is_empty() {
            continue;
        }
        let stamp = date_of(message.observed).stamp();
        if current != Some(stamp) {
            let day = canonical(reader, head, &[stamp])?;
            doc.open(DAY, day, 0, limits)?;
            current = Some(stamp);
            ordinal = 0;
        }
        ordinal += 1;
        let locator = canonical(reader, head, &[stamp, ordinal])?;
        doc.open(MESSAGE, locator, 0, limits)?;
        doc.write(&message.text);
        doc.annotate(|row| decorate(row, message));
        written += 1;
    }
    doc.close_level(DAY);
    Ok(written)
}

/// Put a message's metadata on the row the text just landed in.
fn decorate(row: &mut Segment, message: &Message) {
    row.speaker = message.speaker.clone();
    row.observed = Some(message.observed);
    row.tz_offset = message.tz_offset;
    row.ids = message.ids.clone();
}

/// A canonical locator, or the refusal that says which identifier could not be one.
pub(crate) fn canonical(
    reader: &'static str,
    head: &str,
    steps: &[u64],
) -> Result<Locator, LibError> {
    Locator::canonical(head, steps).map_err(|e| LibError::Unreadable {
        reader: reader.to_string(),
        at: 0,
        what: format!("a locator head this crate can write, not `{head}` ({e})"),
    })
}

#[cfg(any(feature = "reader-telegram", feature = "reader-slack"))]
/// A text-keyed CBOR map, in **encoded**-key order: what a reader puts in `raw`.
///
/// Manifest key 16 is "the reader's metadata it could not place in a named field", and it is
/// carried as opaque canonical CBOR. Opaque means nothing downstream decodes it, which is
/// exactly why it has to be canonical anyway: it is inside the manifest, so it is inside the
/// mid, and a map whose key order depended on how a reader happened to insert would give one
/// export two manifest identities.
///
/// Encoded-key order and not string order, for the reason [`crate::reading`] states for the
/// same shape: a text key's encoding begins with its length, so `"zz"` sorts before `"aaa"`.
///
/// Flat, and only scalars. A reader that copied a nested object out of its source would be
/// putting input-controlled depth inside a manifest, and the manifest's own encoder refuses
/// past [`smysl_core::cbor::MAX_NESTING`] — so the depth of what a reader keeps is the one
/// thing about `raw` that must not come from the file.
pub(crate) fn raw_map(entries: &BTreeMap<String, String>) -> Vec<u8> {
    use smysl_core::cbor::major;
    use smysl_core::cbor::writer::enc;

    let mut rows: Vec<(Vec<u8>, Vec<u8>)> = entries
        .iter()
        .map(|(k, v)| (enc(|e| e.text(k)), enc(|e| e.text(v))))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    enc(|e| {
        e.head(major::MAP, rows.len() as u64);
        for (k, v) in &rows {
            e.raw(k);
            e.raw(v);
        }
    })
}

#[cfg(feature = "reader-slack")]
/// A locator head made out of a name the source chose.
///
/// A head is ASCII alphanumeric with at least one letter ([`crate::locator`]), and a channel is
/// called `#general-2024` or `проект`. So the name is **reduced**, not escaped: ASCII letters
/// and digits are kept, everything else is dropped, and the result is checked. Reduction can
/// collide — `general-2024` and `general2024` reduce alike — and the collision is not papered
/// over: two conversations reducing to one head produce one repeated address, which
/// `Doc::open` refuses by name. A silent `-2` suffix would have been a locator that names a
/// channel nobody can find.
pub(crate) fn head_of(name: &str) -> Option<String> {
    let reduced: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if reduced.is_empty() || !reduced.chars().any(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some(reduced)
}

// The shared module's own tests, which reach every part of it — so they compile where every
// part of it exists. A single-reader build runs that reader's tests instead; `--all-features`
// and `cli` run these.
#[cfg(all(
    test,
    feature = "reader-telegram",
    feature = "reader-slack",
    feature = "reader-whatsapp"
))]
mod tests {
    use super::*;
    use crate::limits::Caps;

    fn budget() -> Budget {
        Budget::new(Caps::DEFAULT, 1 << 16).expect("budget")
    }

    fn message(text: &str, observed: u64, who: &str) -> Message {
        Message {
            text: text.to_string(),
            speaker: Some(who.to_string()),
            observed,
            ..Message::default()
        }
    }

    /// The two directions agree, over every day of a leap year and two century boundaries.
    #[test]
    fn the_calendar_round_trips_every_day_it_can_name() {
        for days in -800_000..800_000i64 {
            let ms = days.saturating_mul(MS_PER_DAY);
            if ms < 0 {
                continue;
            }
            let d = date_of(ms as u64);
            assert_eq!(
                days_from_civil(d.year, d.month, d.day),
                Some(days),
                "{d:?} at {days}"
            );
        }
    }

    /// The dates a reader of a chat export actually meets.
    #[test]
    fn known_dates_are_the_dates_they_are_known_to_be() {
        // 1970-01-01T00:00:00Z, 2000-02-29 (a leap year that a century rule nearly removed),
        // 2024-01-15T10:23:45Z, and 1900-03-01 is not reachable (before the epoch).
        assert_eq!(
            date_of(0),
            Date {
                year: 1970,
                month: 1,
                day: 1
            }
        );
        assert_eq!(
            date_of(951_782_400_000),
            Date {
                year: 2000,
                month: 2,
                day: 29
            }
        );
        let ms = instant(2024, 1, 15, 10, 23, 45).expect("a valid instant");
        assert_eq!(ms, 1_705_314_225_000);
        assert_eq!(date_of(ms).stamp(), 20_240_115);
    }

    /// A date the calendar does not have is refused rather than wrapped.
    #[test]
    fn a_date_that_does_not_exist_is_not_the_day_after_the_last_one() {
        assert!(days_from_civil(2024, 2, 30).is_none());
        assert!(
            days_from_civil(2023, 2, 29).is_none(),
            "2023 is not a leap year"
        );
        assert!(days_from_civil(2024, 13, 1).is_none());
        assert!(days_from_civil(2024, 0, 1).is_none());
        assert!(days_from_civil(2024, 1, 0).is_none());
        assert!(instant(2024, 1, 15, 24, 0, 0).is_none());
        assert!(instant(2024, 1, 15, 0, 60, 0).is_none());
        assert!(
            instant(1969, 12, 31, 23, 59, 59).is_none(),
            "before the epoch"
        );
    }

    /// Two days, three messages: the rows are the structure §3.2 describes.
    #[test]
    fn a_conversation_is_days_of_messages_numbered_within_each_day() {
        let day1 = instant(2024, 1, 15, 10, 0, 0).expect("instant");
        let day2 = instant(2024, 1, 16, 9, 0, 0).expect("instant");
        let messages = vec![
            message("First.", day1, "alice"),
            message("Second.", day1 + 60_000, "bob"),
            message("Next day.", day2, "alice"),
        ];
        let mut doc = Doc::new("test/1");
        let written = conversation(&mut doc, "test/1", "chat", &messages, &mut budget())
            .expect("a conversation");
        assert_eq!(written, 3);
        let (text, rows) = doc.finish().expect("finished");
        assert_eq!(text.as_str(), "First. Second. Next day.");

        let addresses: Vec<String> = rows.iter().map(|r| r.locator.to_string()).collect();
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
        let slice = |r: &Segment| text.as_str()[r.start as usize..r.end as usize].to_string();
        assert_eq!(slice(&rows[0]), "First. Second.", "the day is its messages");
        assert_eq!(slice(&rows[2]), "Second.");
        assert_eq!(rows[2].speaker.as_deref(), Some("bob"));
        assert_eq!(rows[2].observed, Some(day1 + 60_000));
        assert!(rows[0].speaker.is_none(), "a day has no speaker");
    }

    /// A message with no prose is no row, and does not take an ordinal with it.
    #[test]
    fn an_empty_message_is_not_a_node_and_does_not_consume_a_number() {
        let at = instant(2024, 3, 1, 12, 0, 0).expect("instant");
        let messages = vec![
            message("Here.", at, "alice"),
            message("   ", at + 1000, "bob"),
            message("There.", at + 2000, "alice"),
        ];
        let mut doc = Doc::new("test/1");
        let written = conversation(&mut doc, "test/1", "chat", &messages, &mut budget())
            .expect("a conversation");
        assert_eq!(written, 2);
        let (_, rows) = doc.finish().expect("finished");
        let addresses: Vec<String> = rows.iter().map(|r| r.locator.to_string()).collect();
        assert_eq!(
            addresses,
            vec!["chat.20240301", "chat.20240301.1", "chat.20240301.2"]
        );
    }

    /// Messages out of time order would name one day twice, and that is refused.
    #[test]
    fn a_conversation_that_goes_backwards_is_refused_by_address() {
        let day1 = instant(2024, 1, 15, 10, 0, 0).expect("instant");
        let day2 = instant(2024, 1, 16, 9, 0, 0).expect("instant");
        let messages = vec![
            message("One.", day1, "alice"),
            message("Two.", day2, "bob"),
            message("Back again.", day1 + 1000, "alice"),
        ];
        let mut doc = Doc::new("test/1");
        let err = conversation(&mut doc, "test/1", "chat", &messages, &mut budget())
            .expect_err("refused");
        match err {
            LibError::Unreadable { what, .. } => {
                assert!(what.contains("chat.20240115"), "{what}");
                assert!(what.contains("twice"), "{what}");
            }
            other => panic!("{other:?}"),
        }
    }

    /// Offsets are read in minutes, so `+0545` is a real offset and `+2500` is not.
    ///
    /// Minutes matter: India is `+0530`, Nepal `+0545`, and the Marquesas `-0930`. A reader
    /// that took whole hours would put a message on the wrong day for a fifth of the world
    /// twice a day.
    #[test]
    fn an_offset_is_minutes_and_is_checked() {
        assert_eq!(offset_of("+0530"), Some(330));
        assert_eq!(offset_of("+0545"), Some(345));
        assert_eq!(offset_of("-0330"), Some(-210));
        assert_eq!(offset_of("-0930"), Some(-570));
        assert_eq!(offset_of("Z"), Some(0));
        for bad in ["+2500", "+0060", "0300", "+03:00", "+03", "", "x", "+03000"] {
            assert!(offset_of(bad).is_none(), "{bad}");
        }
    }

    /// A channel name becomes a head by reduction, and a name with nothing to keep is refused.
    #[test]
    fn a_head_keeps_the_letters_and_digits_of_a_name() {
        assert_eq!(head_of("general").as_deref(), Some("general"));
        assert_eq!(head_of("general-2024").as_deref(), Some("general2024"));
        assert_eq!(head_of("#random").as_deref(), Some("random"));
        assert_eq!(head_of("проект"), None, "no ASCII letter to keep");
        assert_eq!(head_of("2024"), None, "a head needs a letter");
        assert_eq!(head_of(""), None);
    }
}
