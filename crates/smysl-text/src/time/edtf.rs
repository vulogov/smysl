//! EDTF as an interval of instants: the one place the syntax becomes arithmetic.
//!
//! The parser and the AST are `smysl_core::edtf`, re-exported here, because the surface parser
//! and the CBOR producers have to refuse a malformed value and neither may depend on this crate
//! (SMYSL-2.4 §3.3). What this module adds is the extent: what instants a value admits.
//!
//! **Soundness before precision.** Every interval here is a *superset* of the instants the value
//! admits, never a guess at a narrower one, because rule E propagates these bounds and a bound
//! that excluded the truth would exclude it everywhere downstream. Two cases make that choice
//! visible and both are recorded as **OQ-71**:
//!
//! - **A qualifier is not a width.** `1920~` has the extent of `1920`. "About 1920" does not say
//!   "within five years" — any number picked here would be invented, and an invented width would
//!   propagate into effective intervals and then into reports. What the qualifier does affect is
//!   how much the value should be *trusted*, which is a time status rather than a bound, and the
//!   AST keeps the qualifier so that decision lands in one place when it is made.
//! - **A season is its year.** EDTF numbers the seasons 21–24 and fixes no hemisphere, so
//!   `2001-21` is spring in Oslo and autumn in Wellington. Narrowing it to three months would
//!   assume one, and `2001-24` under a northern reading reaches into 2002 — so the only reading
//!   that is sound without a hemisphere is the enclosing year.

pub use smysl_core::edtf::{
    parse, Date, DateTime, Edtf, EdtfError, End, Part, Qualifier, Year, Zone,
};

use smysl_core::types::Calendar;

use super::calendar;
use super::{Bound, Instant, Interval};

/// How far a local time with no zone can be from the same reading in UTC.
///
/// UTC−12 to UTC+14 are the extremes in use, so an unzoned `09:30:01` is somewhere in a 26-hour
/// window around the UTC instant with those digits. That is wide, and it is the honest width: a
/// date-time with no zone names no instant, and reading it as UTC would be inventing one. Chat
/// exports are the reason this matters — half of them write local time and say so in a header.
const MAX_WEST_MS: i64 = 14 * 3_600_000;
const MAX_EAST_MS: i64 = 12 * 3_600_000;

/// The instants an EDTF value admits, in the proleptic Gregorian calendar.
pub fn to_interval(v: &Edtf) -> Interval {
    to_interval_in(v, None)
}

/// The same, for a value a manifest says is in the Julian calendar (manifest key 18).
///
/// The conversion happens here and the value is never written back (A-2.3): a manifest that
/// records 1611 from a title page records the characters `1611`, and its mid is over those
/// characters. Only the date half of a date-time converts, and a `Y`-year or a masked year does
/// not convert at all — the Julian rule is about days, and neither of those names one.
pub fn to_interval_in(v: &Edtf, cal: Option<Calendar>) -> Interval {
    match v {
        Edtf::Date(d) => date_extent(d, cal),
        Edtf::DateTime(dt) => datetime_extent(dt, cal),
        Edtf::Interval { lo, hi } => Interval {
            lo: match lo {
                End::Date(d) => date_extent(d, cal).lo,
                // An **open** end says there is no bound; an **unknown** end says there is one
                // and nobody recorded it. As an extent the two are the same — neither gives a
                // number — and the distinction survives in the AST, where a why-chain can say
                // which of the two it met.
                // `End` is `#[non_exhaustive]`, so an end a later level adds falls here with
                // the two that have no number: unbounded, which excludes nothing.
                _ => Bound::Open,
            },
            hi: match hi {
                End::Date(d) => date_extent(d, cal).hi,
                _ => Bound::Open,
            },
        },
        // `Edtf` is `#[non_exhaustive]`: a form a later level adds has no extent this build can
        // compute, and *undated* is the only sound answer — it excludes nothing.
        _ => Interval::UNDATED,
    }
}

/// Whether every bound of the value fits in the millisecond instant range.
///
/// `false` is not an error. The value is kept as text and indexed as an open bound, which is
/// what `SMY-W449` reports: a `Y`-year of 200 million years is a legitimate EDTF value and a
/// clamped bound would be a claim nobody made.
pub fn fits(v: &Edtf) -> bool {
    let i = to_interval(v);
    match v {
        Edtf::Interval { lo, hi } => {
            (!matches!(lo, End::Date(_)) || i.lo != Bound::Open)
                && (!matches!(hi, End::Date(_)) || i.hi != Bound::Open)
        }
        _ => i.lo != Bound::Open && i.hi != Bound::Open,
    }
}

/// The half-open extent of a date, widest-first: the earliest instant it admits and the first
/// instant after the last one it admits.
fn date_extent(d: &Date, cal: Option<Calendar>) -> Interval {
    // A masked year is a range of years and a long year is one the calendar arithmetic can
    // still hold; both are handled by taking the first and last year the value admits.
    let span = 10i64.pow(u32::from(d.year.unspecified()));
    let first_year = i64::from(d.year.value());
    let last_year = first_year + span - 1;

    let (lo_m, lo_d, hi_m, hi_d) = match (d.month, d.day) {
        // A season, or a month the source does not give: the whole year.
        (None, _) | (Some(Part::Unspecified), _) => (1u32, 1u32, 12u32, 31u32),
        (Some(Part::Num(m)), _) if m >= 21 => (1u32, 1u32, 12u32, 31u32),
        // A month, with no day or an unspecified one: the whole month.
        (Some(Part::Num(m)), None) | (Some(Part::Num(m)), Some(Part::Unspecified)) => {
            (u32::from(m), 1, u32::from(m), 31)
        }
        (Some(Part::Num(m)), Some(Part::Num(day))) => {
            (u32::from(m), u32::from(day), u32::from(m), u32::from(day))
        }
        // `Part` is `#[non_exhaustive]`: a component a later level adds is read as "the source
        // does not say", which is the widest reading and therefore the sound one.
        _ => (1u32, 1u32, 12u32, 31u32),
    };

    let lo = start_of(first_year, lo_m, lo_d, cal);
    let hi = end_of(last_year, hi_m, hi_d, cal);
    Interval {
        lo: lo.map_or(Bound::Open, |ms| Bound::At(Instant(ms))),
        hi: hi.map_or(Bound::Open, |ms| Bound::At(Instant(ms))),
    }
}

/// The first instant of a (year, month, day), clamping the day into the month.
///
/// Clamping rather than refusing, because the caller has already been through the parser: a day
/// that does not exist in its month is `SMY-E410` and never reaches here. What does reach here
/// is day 31 standing for "the end of the month", from a value that gave no day at all.
fn start_of(year: i64, month: u32, day: u32, cal: Option<Calendar>) -> Option<i64> {
    let (year, month, day) = shift_calendar(year, month, day, cal)?;
    let day = day.min(calendar::days_in_month(year, month));
    calendar::days_from_civil(year, month, day)?.checked_mul(calendar::MS_PER_DAY)
}

/// The first instant *after* a (year, month, day).
fn end_of(year: i64, month: u32, day: u32, cal: Option<Calendar>) -> Option<i64> {
    let (year, month, day) = shift_calendar(year, month, day, cal)?;
    let day = day.min(calendar::days_in_month(year, month));
    calendar::days_from_civil(year, month, day)?
        .checked_add(1)?
        .checked_mul(calendar::MS_PER_DAY)
}

/// Convert a Julian date to its Gregorian equivalent, or pass a Gregorian one through.
fn shift_calendar(
    year: i64,
    month: u32,
    day: u32,
    cal: Option<Calendar>,
) -> Option<(i64, u32, u32)> {
    match cal {
        Some(Calendar::Julian) => {
            let day = day.min(calendar::days_in_month(year, month).max(28));
            let c = calendar::julian_to_gregorian(year, month, day)?;
            Some((c.year, c.month, c.day))
        }
        _ => Some((year, month, day)),
    }
}

fn datetime_extent(dt: &DateTime, cal: Option<Calendar>) -> Interval {
    let (year, month, day) = match shift_calendar(
        i64::from(dt.year),
        u32::from(dt.month),
        u32::from(dt.day),
        cal,
    ) {
        Some(t) => t,
        None => return Interval::UNDATED,
    };
    let Some(ms) = calendar::ms_from_civil(
        year,
        month,
        day,
        u32::from(dt.hour),
        u32::from(dt.minute),
        u32::from(dt.second),
    ) else {
        return Interval::UNDATED;
    };
    match dt.zone {
        // A second of precision: EDTF level 0 writes no fraction, so `09:30:01Z` is the second
        // beginning at that instant and not the millisecond.
        Some(Zone::Utc) => Interval::of(ms, ms + 1000),
        Some(Zone::Offset(minutes)) => {
            let shift = i64::from(minutes) * 60_000;
            Interval::of(ms - shift, ms - shift + 1000)
        }
        // No zone, no instant. The 26-hour window is the honest width, and a zone a later
        // version of `Zone` adds lands here for the same reason: a window that contains every
        // offset contains whatever that zone turns out to mean.
        _ => Interval::of(ms - MAX_WEST_MS, ms + MAX_EAST_MS + 1000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extent(s: &str) -> Interval {
        to_interval(&parse(s).expect("the fixture's own values parse"))
    }

    fn ms(y: i64, m: u32, d: u32) -> i64 {
        calendar::days_from_civil(y, m, d).expect("a real date") * calendar::MS_PER_DAY
    }

    #[test]
    fn a_year_is_the_year() {
        assert_eq!(extent("1984"), Interval::of(ms(1984, 1, 1), ms(1985, 1, 1)));
    }

    #[test]
    fn a_month_is_the_month_and_a_day_is_the_day() {
        assert_eq!(
            extent("1984-06"),
            Interval::of(ms(1984, 6, 1), ms(1984, 7, 1))
        );
        assert_eq!(
            extent("1984-06-02"),
            Interval::of(ms(1984, 6, 2), ms(1984, 6, 3))
        );
        assert_eq!(
            extent("2024-02-29"),
            Interval::of(ms(2024, 2, 29), ms(2024, 3, 1)),
            "a leap day is a day"
        );
    }

    /// A mask is a range the value states, so this one *is* narrowed — unlike a qualifier.
    #[test]
    fn a_mask_is_the_range_it_states() {
        assert_eq!(extent("201X"), Interval::of(ms(2010, 1, 1), ms(2020, 1, 1)));
        assert_eq!(extent("20XX"), Interval::of(ms(2000, 1, 1), ms(2100, 1, 1)));
        assert_eq!(
            extent("1984-XX"),
            Interval::of(ms(1984, 1, 1), ms(1985, 1, 1))
        );
        assert_eq!(
            extent("1984-06-XX"),
            Interval::of(ms(1984, 6, 1), ms(1984, 7, 1))
        );
        // A masked year with a month is coarse on purpose: the extent of "June of some year in
        // the 2010s" has to contain every June in the decade, and the simplest superset that
        // does is the decade with its ends trimmed to June.
        assert_eq!(
            extent("201X-06"),
            Interval::of(ms(2010, 6, 1), ms(2019, 7, 1))
        );
    }

    /// OQ-71, as implemented: the qualifier changes nothing about the extent.
    #[test]
    fn a_qualifier_is_not_a_width() {
        assert_eq!(extent("1984?"), extent("1984"));
        assert_eq!(extent("1984-06~"), extent("1984-06"));
        assert_eq!(extent("1984-06-02%"), extent("1984-06-02"));
    }

    /// OQ-71, as implemented: a season is its year, because EDTF fixes no hemisphere.
    #[test]
    fn a_season_is_its_year() {
        for s in 21..=24 {
            assert_eq!(extent(&format!("2001-{s}")), extent("2001"), "season {s}");
        }
    }

    #[test]
    fn a_zoned_date_time_is_a_second_and_an_unzoned_one_is_a_day_and_a_bit() {
        let t = ms(1984, 6, 2) + 9 * 3_600_000 + 30 * 60_000 + 1000;
        assert_eq!(
            extent("1984-06-02T09:30:01Z"),
            Interval::of(t, t + 1000),
            "a second of precision, because level 0 writes no fraction"
        );
        assert_eq!(
            extent("1984-06-02T09:30:01+05:30"),
            Interval::of(t - 19_800_000, t - 19_800_000 + 1000),
            "+05:30 means the UTC instant is earlier"
        );
        let local = extent("1984-06-02T09:30:01");
        assert_eq!(local, Interval::of(t - MAX_WEST_MS, t + MAX_EAST_MS + 1000));
        assert!(
            local.contains(Instant(t)),
            "whatever the zone, the UTC reading of the digits is inside the window"
        );
    }

    #[test]
    fn an_open_end_and_an_unknown_end_are_both_unbounded_as_an_extent() {
        assert_eq!(
            extent("1984/.."),
            Interval {
                lo: Bound::At(Instant(ms(1984, 1, 1))),
                hi: Bound::Open
            }
        );
        assert_eq!(extent("1984/"), extent("1984/.."));
        assert_eq!(
            extent("../1984"),
            Interval {
                lo: Bound::Open,
                hi: Bound::At(Instant(ms(1985, 1, 1)))
            }
        );
    }

    #[test]
    fn an_interval_runs_from_one_end_to_the_other() {
        assert_eq!(
            extent("1984/1985"),
            Interval::of(ms(1984, 1, 1), ms(1986, 1, 1)),
            "to the end of the closing year, not to its start"
        );
    }

    /// A reversed interval is empty, which is what rule E reports rather than what the parser
    /// refuses.
    #[test]
    fn a_reversed_interval_is_an_empty_extent() {
        assert!(extent("2005/1984").is_empty());
    }

    #[test]
    fn a_negative_year_is_before_the_epoch() {
        let i = extent("-0999");
        assert!(i.lo.instant().expect("bounded").0 < 0);
        assert_eq!(i, Interval::of(ms(-999, 1, 1), ms(-998, 1, 1)));
    }

    /// A long year is a legitimate value with no instant, and the bound is open rather than
    /// clamped: `SMY-W449` says so, and a clamped bound would be a claim nobody made.
    #[test]
    fn a_year_outside_the_instant_range_is_open_rather_than_clamped() {
        // `Y170000002` — EDTF's own example of a long year — **fits**. A millisecond `i64`
        // reaches about ±2.9 × 10⁸ years, so 170 million is inside it, and a reader who
        // assumed otherwise would be widening a bound that did not need widening.
        let inside = parse("Y170000002").expect("a long year");
        assert!(fits(&inside));
        assert!(to_interval(&inside).lo.instant().expect("bounded").0 > 0);
        // Three hundred million years is not inside it, and the bound is open rather than
        // clamped: `SMY-W449` says so, and a clamped bound would be a claim nobody made.
        let outside = parse("Y300000000").expect("a long year");
        assert!(!fits(&outside));
        let i = to_interval(&outside);
        assert_eq!(i.lo, Bound::Open);
        assert_eq!(i.hi, Bound::Open);
        assert!(fits(&parse("1984").expect("a year")));
        assert!(fits(&parse("1984/..").expect("an open interval")));
    }

    /// A Julian value converts on the way out and is not rewritten (A-2.3).
    #[test]
    fn a_julian_value_converts_to_gregorian_instants() {
        let v = parse("1611-05-02").expect("a date");
        let g = to_interval_in(&v, Some(Calendar::Julian));
        assert_eq!(g, Interval::of(ms(1611, 5, 12), ms(1611, 5, 13)));
        assert_ne!(g, to_interval(&v), "the two calendars differ by ten days");
        assert_eq!(v.to_string(), "1611-05-02", "the text is untouched");
    }

    #[test]
    fn a_julian_year_is_the_julian_year() {
        let v = parse("1611").expect("a year");
        let g = to_interval_in(&v, Some(Calendar::Julian));
        // 1611-01-01 Julian is 1611-01-11 Gregorian, and the year ends ten days later too.
        assert_eq!(g, Interval::of(ms(1611, 1, 11), ms(1612, 1, 11)));
    }
}
