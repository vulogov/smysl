//! EDTF — the Extended Date/Time Format of ISO 8601-2, levels 0 and 1 (SMYSL-2.3 A-2.3).
//!
//! Two fields in the format carry one of these strings: `source.published` (key 4) and the
//! absolute value of a dating (record 17, key 2 entry 0). Both say *when the source says it
//! happened*, as the source says it — a title page, a byline, a dateline, an export header —
//! and a source that says "about 1920" is not improved by a producer that writes
//! `1920-01-01T00:00:00Z`. That is the whole reason the format stores EDTF text rather than an
//! instant: the uncertainty is in the evidence, and a format that cannot write it down forces
//! every producer to invent precision it does not have.
//!
//! **Why this module is in `smysl-core`.** The surface parser and the CBOR producers must
//! refuse a malformed value (`SMY-E410`) and neither may depend on `smysl-text`, where the
//! time engine lives (SMYSL-2.4 §3.3). So the syntax is here, at the bottom of the workspace,
//! and `smysl-text::time::edtf` re-exports it and adds what needs a calendar.
//!
//! **Syntax, not meaning.** Nothing here computes an interval, compares two values or decides
//! which of two dates is earlier. `1920?` parses to a year and a qualifier; *how far* a `?`
//! widens an interval is SMYSL-2.3 rule E, applied in one place in the engine, and the AST
//! keeps the qualifier so that place is the only one that needs to know. For the same reason a
//! reversed interval (`2005/1984`) parses: it is well-formed text whose *meaning* is an empty
//! interval, which rule E reports as a temporal inconsistency (`SMY-W413`) rather than a parse
//! error. A parser that rejected it would be answering a question it cannot see the store to
//! answer — the same value is fine as a manifest's `published` and wrong as a dating.
//!
//! **What is covered** (SMYSL-2.4 §3.3): level 0 dates and date-times with `Z` or a numeric
//! offset; level 1 qualifiers `?` `~` `%`, unspecified rightmost digits `X`, intervals with
//! open (`..`) and unknown (empty) ends, negative years, `Y`-prefixed long years, and the
//! seasons 21–24. Level 2 is not covered: a set (`[1667,1668]`), an interior `X` (`1XX3`), a
//! per-component qualifier (`2004-06~-11`) and a repeat rule are all rejected, because
//! accepting a spelling this build cannot interpret would put it inside a uid.
//!
//! **Acceptance is canonical.** `parse(s).to_string() == s` for every accepted `s`: there is
//! exactly one spelling of each value, so a producer cannot write two byte strings that mean
//! one date and give one unit two uids. `+2010`, `2010-6-2`, lowercase `t` and `z`, and a
//! `Y`-year with leading zeros are all refused for that reason and no other.

use core::fmt;
use core::fmt::Write as _;

/// A parsed EDTF value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Edtf {
    /// A date, to the precision the source gave: a year, a year and month, or a full date.
    Date(Date),
    /// A date and time of day, with or without a zone (level 0).
    DateTime(DateTime),
    /// Two ends. Either may be a date, open (`..`) or unknown (empty).
    Interval { lo: End, hi: End },
}

/// One end of an interval.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum End {
    Date(Date),
    /// `..` — the interval extends past this end. "Published from 1984 onwards."
    Open,
    /// Empty — this end exists and is not known. "Published some time up to 1984."
    ///
    /// Distinct from [`End::Open`] on purpose, and the distinction is not decoration: an open
    /// end says there is no bound, an unknown end says there is one and nobody recorded it.
    /// Rule E treats the first as *no constraint* and the second as *a constraint whose value
    /// is missing*, which is what makes `1984/` and `1984/..` different claims.
    Unknown,
}

/// A date at year, month or day precision, possibly masked and possibly qualified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Date {
    pub year: Year,
    /// A month 1–12, a season 21–24, or `XX`. Absent at year precision.
    pub month: Option<Part>,
    /// A day 1–31, or `XX`. Absent at year or month precision, and never present with a
    /// season: a season has no days, and `2001-21-05` is refused rather than guessed at.
    pub day: Option<Part>,
    pub qualifier: Option<Qualifier>,
}

/// A two-digit component: a number, or `XX` for "the source does not say".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Part {
    Num(u8),
    Unspecified,
}

impl Part {
    /// The number, or `None` when the component is `XX`.
    pub const fn num(self) -> Option<u8> {
        match self {
            Part::Num(n) => Some(n),
            Part::Unspecified => None,
        }
    }
}

/// How sure the source is (level 1).
///
/// Qualifies the whole date, which is level 1's rule. Level 2's per-component form
/// (`2004-06~-11`, "June is approximate but the 11th is not") is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Qualifier {
    /// `?` — uncertain: the source is not sure this is the date.
    Uncertain,
    /// `~` — approximate: the date is near this one.
    Approximate,
    /// `%` — both uncertain and approximate.
    Both,
}

impl Qualifier {
    pub const fn as_char(self) -> char {
        match self {
            Qualifier::Uncertain => '?',
            Qualifier::Approximate => '~',
            Qualifier::Both => '%',
        }
    }

    pub const fn from_char(c: char) -> Option<Qualifier> {
        match c {
            '?' => Some(Qualifier::Uncertain),
            '~' => Some(Qualifier::Approximate),
            '%' => Some(Qualifier::Both),
            _ => None,
        }
    }
}

/// A year, with the two spellings level 1 adds to level 0's four digits.
///
/// Fields are private because two of the three combinations they can hold are not a year:
/// a masked long year has no EDTF spelling, and a mask wider than the digits does not either.
/// The constructors are the only way in, and `Display` is therefore exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Year {
    /// The value with masked digits read as zero: `201X` is 2010, `19XX` is 1900.
    value: i32,
    /// How many of the rightmost digits are `X`, 0–4.
    unspecified: u8,
    /// `Y`-prefixed: a year that needs more than four digits (`Y170000002`).
    long: bool,
}

impl Year {
    /// A plain four-digit year, negative for BCE. `-9999..=9999`.
    #[allow(
        clippy::manual_range_contains,
        reason = "RangeInclusive::contains is not const"
    )]
    pub const fn new(value: i32) -> Option<Year> {
        if value < -9999 || value > 9999 {
            return None;
        }
        Some(Year {
            value,
            unspecified: 0,
            long: false,
        })
    }

    /// A year whose `unspecified` rightmost digits the source does not give.
    ///
    /// `value` must have zeros in the masked positions, so that the spelling is unambiguous:
    /// `201X` is `masked(2010, 1)` and nothing else can be.
    pub fn masked(value: i32, unspecified: u8) -> Option<Year> {
        if !(1..=4).contains(&unspecified) || !(-9999..=9999).contains(&value) {
            return None;
        }
        let step = 10i32.pow(unspecified as u32);
        if value % step != 0 {
            return None;
        }
        Some(Year {
            value,
            unspecified,
            long: false,
        })
    }

    /// A `Y`-prefixed year, for the ones four digits cannot hold.
    ///
    /// Rejects anything four digits *can* hold: `Y1984` and `1984` would otherwise be two
    /// spellings of one year, and both would be accepted, and a producer choosing between them
    /// would be choosing between two uids.
    #[allow(
        clippy::manual_range_contains,
        reason = "RangeInclusive::contains is not const"
    )]
    pub const fn long(value: i32) -> Option<Year> {
        if value >= -9999 && value <= 9999 {
            return None;
        }
        Some(Year {
            value,
            unspecified: 0,
            long: true,
        })
    }

    /// The value, with any masked digit read as zero.
    pub const fn value(self) -> i32 {
        self.value
    }

    /// How many rightmost digits are `X`.
    pub const fn unspecified(self) -> u8 {
        self.unspecified
    }

    pub const fn is_long(self) -> bool {
        self.long
    }
}

impl fmt::Display for Year {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.long {
            return write!(f, "Y{}", self.value);
        }
        let sign = if self.value < 0 { "-" } else { "" };
        let digits = format!("{:04}", self.value.unsigned_abs());
        let keep = digits.len() - self.unspecified as usize;
        write!(
            f,
            "{sign}{}{}",
            &digits[..keep],
            "X".repeat(self.unspecified as usize)
        )
    }
}

/// A date and a time of day (level 0).
///
/// Level 0 only, which is why the date half has no mask and no qualifier: a time of day on an
/// approximate date is a contradiction the standard does not spell, and nothing in this format
/// needs it — an export header gives an instant or it gives a date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct DateTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// `Z`, a numeric offset, or none — and *none* is the interesting case: a local time with
    /// no zone names no instant, so rule E seeds it as an interval a day wide rather than
    /// pretending it is UTC.
    pub zone: Option<Zone>,
}

/// The zone a date-time names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Zone {
    /// `Z`.
    Utc,
    /// `+HH:MM` or `-HH:MM`, in minutes east of UTC. `-00:00` is refused: it is a spelling of
    /// `Z` and a second spelling of one value is a second uid.
    Offset(i16),
}

/// Why a string is not EDTF this build accepts.
///
/// Carries the text as well as the reason, because the value reaches here from three places —
/// a surface document, a CBOR record and an argument — and the diagnostic has to be able to
/// quote what it refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdtfError {
    pub found: String,
    pub why: &'static str,
}

impl fmt::Display for EdtfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}` is not EDTF: {}", self.found, self.why)
    }
}

impl std::error::Error for EdtfError {}

type Res<T> = Result<T, EdtfError>;

fn err<T>(found: &str, why: &'static str) -> Res<T> {
    Err(EdtfError {
        found: found.to_string(),
        why,
    })
}

/// Parse an EDTF level-0 or level-1 value.
pub fn parse(s: &str) -> Res<Edtf> {
    if s.is_empty() {
        return err(s, "the value is empty");
    }
    // An interval is the only form with a `/`, and exactly one: `1984/1985/1986` is three
    // ends, which means nothing.
    let mut halves = s.split('/');
    let lo_txt = halves.next().unwrap_or("");
    if let Some(hi_txt) = halves.next() {
        if halves.next().is_some() {
            return err(s, "an interval has two ends, so one `/`");
        }
        let lo = end(s, lo_txt)?;
        let hi = end(s, hi_txt)?;
        if matches!(lo, End::Date(_)) || matches!(hi, End::Date(_)) {
            return Ok(Edtf::Interval { lo, hi });
        }
        // `/`, `../..`, `../` and so on: an interval with no date at either end is not a
        // statement about time at all.
        return err(s, "an interval needs a date at one end");
    }
    if s.contains('T') {
        return Ok(Edtf::DateTime(date_time(s)?));
    }
    Ok(Edtf::Date(date(s)?))
}

/// Whether `s` is a value [`parse`] accepts.
pub fn is_valid(s: &str) -> bool {
    parse(s).is_ok()
}

fn end(whole: &str, txt: &str) -> Res<End> {
    match txt {
        "" => Ok(End::Unknown),
        ".." => Ok(End::Open),
        _ if txt.contains('T') => err(
            whole,
            "an interval end is a date, never a date-time (level 0)",
        ),
        _ => Ok(End::Date(date(txt)?)),
    }
}

fn date(txt: &str) -> Res<Date> {
    let (body, qualifier) = match txt.chars().next_back().and_then(Qualifier::from_char) {
        Some(q) => (&txt[..txt.len() - q.as_char().len_utf8()], Some(q)),
        None => (txt, None),
    };
    // A long year stands alone. It has to: `Y-170000002` carries its sign *after* the `Y`, so
    // splitting on `-` first would read the year as `Y` and the month as `170000002`.
    if body.starts_with('Y') {
        return Ok(Date {
            year: long_year(txt, body)?,
            month: None,
            day: None,
            qualifier,
        });
    }
    // An ordinary year may begin with `-` as well, so the field split happens after the sign.
    let (neg, rest) = match body.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, body),
    };
    let mut fields = rest.split('-');
    let year = plain_year(txt, neg, fields.next().unwrap_or(""))?;
    let month = match fields.next() {
        None => None,
        Some(m) => Some(two(txt, m, Component::Month)?),
    };
    let day = match fields.next() {
        None => None,
        Some(d) => Some(two(txt, d, Component::Day)?),
    };
    if fields.next().is_some() {
        return err(txt, "a date has at most a year, a month and a day");
    }
    if day.is_some() && month.is_none() {
        return err(txt, "a day needs a month");
    }
    if let (Some(Part::Num(m)), Some(_)) = (month, day) {
        if m >= 21 {
            return err(txt, "a season has no days");
        }
    }
    // A day that does not exist in its month is not a date. Only checkable when both are
    // given: `2019-XX-31` stays accepted, because some month has a 31st.
    if let (Some(Part::Num(m)), Some(Part::Num(d))) = (month, day) {
        if year.unspecified() == 0 && d > days_in_month(year.value(), m) {
            return err(txt, "that day does not exist in that month");
        }
    }
    Ok(Date {
        year,
        month,
        day,
        qualifier,
    })
}

enum Component {
    Month,
    Day,
}

fn two(whole: &str, txt: &str, which: Component) -> Res<Part> {
    if txt == "XX" {
        return Ok(Part::Unspecified);
    }
    if txt.len() != 2 || !txt.bytes().all(|b| b.is_ascii_digit()) {
        return err(
            whole,
            "a month and a day are two digits or `XX`, zero-padded",
        );
    }
    let n: u8 = txt.parse().map_err(|_| EdtfError {
        found: whole.to_string(),
        why: "a month and a day are two digits",
    })?;
    match which {
        // 21–24 are the seasons spring, summer, autumn, winter (level 1). 13–20 are reserved
        // by the standard for level 2's other sub-year divisions, so they are refused here
        // rather than passed through as a month nobody can name.
        Component::Month if (1..=12).contains(&n) || (21..=24).contains(&n) => Ok(Part::Num(n)),
        Component::Month => err(whole, "a month is 01 to 12, or a season 21 to 24"),
        Component::Day if (1..=31).contains(&n) => Ok(Part::Num(n)),
        Component::Day => err(whole, "a day is 01 to 31"),
    }
}

/// `Y170000002` or `Y-170000002`: the years four digits cannot hold.
fn long_year(whole: &str, txt: &str) -> Res<Year> {
    let rest = txt.strip_prefix('Y').unwrap_or(txt);
    let (neg, digits) = match rest.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, rest),
    };
    if digits.len() < 5 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return err(
            whole,
            "a `Y` year is five digits or more, and four-digit years are written without it",
        );
    }
    if digits.starts_with('0') {
        return err(whole, "a `Y` year has no leading zero");
    }
    let too_big = || EdtfError {
        found: whole.to_string(),
        why: "that year does not fit in a 32-bit integer",
    };
    let n: i64 = digits.parse().map_err(|_| too_big())?;
    let n = i32::try_from(if neg { -n } else { n }).map_err(|_| too_big())?;
    Year::long(n).ok_or_else(|| EdtfError {
        found: whole.to_string(),
        why: "a `Y` year is five digits or more",
    })
}

/// Four digits, optionally signed, with the rightmost digits optionally masked.
fn plain_year(whole: &str, neg: bool, digits: &str) -> Res<Year> {
    let shape = || EdtfError {
        found: whole.to_string(),
        why: "a year is four digits, zero-padded",
    };
    if digits.len() != 4 {
        return Err(shape());
    }
    // Level 1 masks the *rightmost* digits only. An interior `X` (`1X34`) is level 2 and is
    // refused: this build could not say what interval it means.
    let masked = digits.bytes().filter(|b| *b == b'X').count() as u8;
    let (known, tail) = digits.split_at(digits.len() - masked as usize);
    if !tail.bytes().all(|b| b == b'X') {
        return err(whole, "only the rightmost digits of a year may be `X`");
    }
    if !known.bytes().all(|b| b.is_ascii_digit()) {
        return Err(shape());
    }
    let mut value: i32 = if known.is_empty() {
        0
    } else {
        known.parse().map_err(|_| shape())?
    };
    value *= 10i32.pow(masked as u32);
    if neg {
        value = -value;
    }
    if masked == 0 {
        Year::new(value)
    } else {
        Year::masked(value, masked)
    }
    .ok_or_else(shape)
}

fn date_time(txt: &str) -> Res<DateTime> {
    let (d_txt, t_txt) = txt.split_once('T').ok_or_else(|| EdtfError {
        found: txt.to_string(),
        why: "a date-time separates its halves with `T`",
    })?;
    let d = date(d_txt)?;
    if d.qualifier.is_some() {
        return err(txt, "a date-time is level 0, and takes no qualifier");
    }
    let (Some(Part::Num(month)), Some(Part::Num(day))) = (d.month, d.day) else {
        return err(txt, "a date-time needs a full date");
    };
    if d.year.unspecified() != 0 || d.year.is_long() {
        return err(txt, "a date-time needs a four-digit year with no `X`");
    }
    let (clock, zone) = split_zone(txt, t_txt)?;
    let f: Vec<&str> = clock.split(':').collect();
    if f.len() != 3
        || f.iter()
            .any(|p| p.len() != 2 || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return err(txt, "a time of day is `HH:MM:SS`, zero-padded");
    }
    let num = |s: &str| -> u8 { s.parse().unwrap_or(99) };
    let (hour, minute, second) = (num(f[0]), num(f[1]), num(f[2]));
    if hour > 23 || minute > 59 || second > 59 {
        // 23:59:60 exists in ISO 8601 and is refused here: a leap second has no instant in
        // the millisecond scale rule E orders, and no export this format reads emits one.
        return err(txt, "a time of day runs to 23:59:59");
    }
    Ok(DateTime {
        year: d.year.value(),
        month,
        day,
        hour,
        minute,
        second,
        zone,
    })
}

fn split_zone<'a>(whole: &str, t: &'a str) -> Res<(&'a str, Option<Zone>)> {
    if let Some(clock) = t.strip_suffix('Z') {
        return Ok((clock, Some(Zone::Utc)));
    }
    // The offset's own sign is the last `+` or `-` in the time half, and a clock holds
    // neither, so finding it needs no backtracking.
    if let Some(p) = t.rfind(['+', '-']) {
        let (clock, off) = t.split_at(p);
        let east = off.starts_with('+');
        let hhmm = &off[1..];
        let (h, m) = hhmm.split_once(':').ok_or_else(|| EdtfError {
            found: whole.to_string(),
            why: "an offset is `+HH:MM` or `-HH:MM`",
        })?;
        if h.len() != 2 || m.len() != 2 || !(h.bytes().chain(m.bytes())).all(|b| b.is_ascii_digit())
        {
            return err(whole, "an offset is `+HH:MM` or `-HH:MM`, zero-padded");
        }
        let (h, m): (i16, i16) = (h.parse().unwrap_or(99), m.parse().unwrap_or(99));
        if h > 23 || m > 59 {
            return err(whole, "an offset runs to 23:59");
        }
        let minutes = h * 60 + m;
        if minutes == 0 && !east {
            return err(whole, "`-00:00` is `Z`, and one value has one spelling");
        }
        return Ok((
            clock,
            Some(Zone::Offset(if east { minutes } else { -minutes })),
        ));
    }
    Ok((t, None))
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Proleptic Gregorian, as A-2.3 requires: year 0 exists and is a leap year, and the rule is
/// applied before 1582 as well as after. A Julian date is converted when intervals are derived
/// (manifest key 18), never on the way in.
fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

// ---------------------------------------------------------------------------
// Display — the inverse of `parse`
// ---------------------------------------------------------------------------

impl fmt::Display for Part {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Part::Num(n) => write!(f, "{n:02}"),
            Part::Unspecified => f.write_str("XX"),
        }
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.year)?;
        if let Some(m) = self.month {
            write!(f, "-{m}")?;
        }
        if let Some(d) = self.day {
            write!(f, "-{d}")?;
        }
        if let Some(q) = self.qualifier {
            f.write_char(q.as_char())?;
        }
        Ok(())
    }
}

impl fmt::Display for Zone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Zone::Utc => f.write_str("Z"),
            Zone::Offset(m) => {
                let (sign, m) = if *m < 0 { ('-', -*m) } else { ('+', *m) };
                write!(f, "{sign}{:02}:{:02}", m / 60, m % 60)
            }
        }
    }
}

impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.year < 0 { "-" } else { "" };
        write!(
            f,
            "{sign}{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year.unsigned_abs(),
            self.month,
            self.day,
            self.hour,
            self.minute,
            self.second
        )?;
        if let Some(z) = self.zone {
            write!(f, "{z}")?;
        }
        Ok(())
    }
}

impl fmt::Display for End {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            End::Date(d) => write!(f, "{d}"),
            End::Open => f.write_str(".."),
            End::Unknown => Ok(()),
        }
    }
}

impl fmt::Display for Edtf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Edtf::Date(d) => write!(f, "{d}"),
            Edtf::DateTime(dt) => write!(f, "{dt}"),
            Edtf::Interval { lo, hi } => write!(f, "{lo}/{hi}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The acceptance tables are not here.** They are the fixture
    /// `fixtures/library/edtf/cases.json`, generated and checked by
    /// `tests/edtf_fixture.rs`, which holds every accepted form with the level that defines it
    /// and every refused one with the reason. One table, in the place another implementation
    /// can read it — a second copy here would be a second thing to keep in step, and the
    /// refusals are the half that matters: EDTF has more than one spelling of some values and
    /// this format accepts exactly one, because both fields that carry it are inside an
    /// identity.
    ///
    /// What is left in this module is the structure behind those strings: that a mask is a
    /// range, that an open end is not an unknown end, that a zone keeps its sign, and that the
    /// calendar is proleptic Gregorian. None of those is visible in a round trip.
    #[test]
    fn a_mask_is_a_range_and_the_value_is_its_floor() {
        let Edtf::Date(d) = parse("201X").unwrap() else {
            panic!("a date")
        };
        assert_eq!(d.year.value(), 2010);
        assert_eq!(d.year.unspecified(), 1);
        let Edtf::Date(d) = parse("2XXX").unwrap() else {
            panic!("a date")
        };
        assert_eq!(d.year.value(), 2000);
        assert_eq!(d.year.unspecified(), 3);
        let Edtf::Date(d) = parse("XXXX").unwrap() else {
            panic!("a date")
        };
        assert_eq!(d.year.value(), 0);
        assert_eq!(d.year.unspecified(), 4);
    }

    /// A masked year cannot be built with a value in a masked position, because `201X` would
    /// then have two in-memory forms and `Display` would have to choose.
    #[test]
    fn a_masked_year_rejects_digits_it_claims_not_to_have() {
        assert!(Year::masked(2010, 1).is_some());
        assert_eq!(Year::masked(2011, 1), None);
        assert_eq!(Year::masked(2010, 0), None, "that is `Year::new`");
        assert_eq!(Year::masked(2010, 5), None);
        assert_eq!(Year::new(10000), None);
        assert_eq!(Year::long(1984), None, "four digits need no `Y`");
        assert!(Year::long(170000002).is_some());
    }

    /// An open end and an unknown end are different claims, so they are different values.
    #[test]
    fn an_open_end_is_not_an_unknown_end() {
        let open = parse("1984/..").unwrap();
        let unknown = parse("1984/").unwrap();
        assert_ne!(open, unknown);
        assert_eq!(
            open,
            Edtf::Interval {
                lo: End::Date(Date {
                    year: Year::new(1984).unwrap(),
                    month: None,
                    day: None,
                    qualifier: None,
                }),
                hi: End::Open,
            }
        );
        assert!(matches!(
            unknown,
            Edtf::Interval {
                hi: End::Unknown,
                ..
            }
        ));
    }

    /// A reversed interval parses. Its *meaning* is an empty interval, which is rule E's
    /// `SMY-W413` and needs a store to see; the parser cannot tell a manifest's publication
    /// range from a dating's claim, and only one of those is wrong.
    #[test]
    fn a_reversed_interval_is_a_question_for_the_engine_not_the_parser() {
        assert!(parse("2005/1984").is_ok());
    }

    #[test]
    fn a_zone_keeps_its_sign_and_its_minutes() {
        let Edtf::DateTime(dt) = parse("1984-06-02T09:30:01+05:30").unwrap() else {
            panic!("a date-time")
        };
        assert_eq!(dt.zone, Some(Zone::Offset(330)));
        let Edtf::DateTime(dt) = parse("1984-06-02T09:30:01-04:00").unwrap() else {
            panic!("a date-time")
        };
        assert_eq!(dt.zone, Some(Zone::Offset(-240)));
        let Edtf::DateTime(dt) = parse("1984-06-02T09:30:01").unwrap() else {
            panic!("a date-time")
        };
        assert_eq!(dt.zone, None, "a local time names no instant");
    }

    /// Year zero exists in the proleptic Gregorian calendar and is a leap year, which is the
    /// one place the calendar A-2.3 names differs visibly from the one most libraries use.
    #[test]
    fn the_calendar_is_proleptic_gregorian() {
        assert!(parse("0000-02-29").is_ok());
        assert!(parse("1900-02-29").is_err(), "1900 is not a leap year");
        assert!(parse("2000-02-29").is_ok());
        assert!(parse("-0004-02-29").is_ok(), "-4 % 4 == 0");
    }

    #[test]
    fn the_error_quotes_what_it_refused() {
        let e = parse("1984-13").unwrap_err();
        assert_eq!(e.found, "1984-13");
        assert!(e.to_string().contains("1984-13"));
        assert!(e.to_string().contains("season"));
    }

    /// The seasons are 21–24 and the codes between 13 and 20 are reserved by the standard for
    /// level 2's other divisions, so a reader that accepted them would be accepting a
    /// precision it cannot name.
    #[test]
    fn the_seasons_are_the_four_the_standard_numbers() {
        for m in 21..=24 {
            assert!(parse(&format!("2001-{m}")).is_ok());
        }
        for m in 13..=20 {
            assert!(parse(&format!("2001-{m}")).is_err());
        }
        assert!(parse("2001-25").is_err());
    }
}
