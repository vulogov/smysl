//! Civil dates and instants, and the one calendar conversion the format needs.
//!
//! Proleptic Gregorian throughout, as SMYSL-2.3 A-2.3 requires: year zero exists and is a leap
//! year, and the rule is applied before 1582 as well as after. Howard Hinnant's era arithmetic,
//! which is exact over the whole range of `i64` and has no table in it.
//!
//! **The chat readers had these three functions first**, behind their own feature gates, because
//! a WhatsApp line is a date written by hand and a reader has to turn it into an instant. The
//! time engine needs the same arithmetic unconditionally, and two copies of a calendar is two
//! calendars: a difference between them would move a message to a different day in one path and
//! not the other, and the identity of a part depends on which day a reader puts a message in.
//! So they live here, and `readers::chat` uses them.

/// Milliseconds in a day. No leap seconds: the format's instants are a count of milliseconds,
/// which is what makes rule E's interval arithmetic exact integer comparison.
pub const MS_PER_DAY: i64 = 86_400_000;

/// A date in the proleptic Gregorian calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Civil {
    pub year: i64,
    pub month: u32,
    pub day: u32,
}

/// The UTC date of a Unix millisecond timestamp.
///
/// Hinnant's `civil_from_days`. The comments name the steps rather than re-deriving them.
pub fn civil_from_ms(ms: i64) -> Civil {
    let days = ms.div_euclid(MS_PER_DAY);
    civil_from_days(days)
}

/// The UTC date `days` after the Unix epoch.
pub fn civil_from_days(days: i64) -> Civil {
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
    Civil {
        year: y + i64::from(m <= 2),
        month: m as u32,
        day: d as u32,
    }
}

/// Days since the Unix epoch of a UTC calendar date, the inverse of [`civil_from_days`].
///
/// `None` for a date outside the calendar's own rules — month 13, day 32, 29 February in a
/// common year. The readers that parse a date by hand need the refusal rather than a
/// wrap-around: `31/02/2024` in a WhatsApp export is a line that is not a timestamp, and
/// reading it as 2 March would put a message on a day it was not sent.
pub fn days_from_civil(year: i64, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return None;
    }
    Some(days_from_civil_unchecked(year, month, day))
}

/// The same arithmetic with no validity check, for a caller that has one already.
///
/// Separate rather than a flag, because the two callers want different things: a reader wants
/// the refusal, and an EDTF value whose month and day the parser has already validated wants
/// the number. A flag would have made the check look optional at the call site.
fn days_from_civil_unchecked(year: i64, month: u32, day: u32) -> i64 {
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * i64::from(mp) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

pub fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// The millisecond timestamp of a UTC date and time of day.
///
/// `None` for a date the calendar does not have, a time of day outside `00:00:00..24:00:00`,
/// or an instant the arithmetic cannot hold.
pub fn ms_from_civil(
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> Option<i64> {
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    let secs = days
        .checked_mul(86_400)?
        .checked_add(i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second))?;
    secs.checked_mul(1000)
}

/// The start of a year, as a millisecond instant. `None` outside the range of the type.
pub fn year_start_ms(year: i64) -> Option<i64> {
    days_from_civil(year, 1, 1)?.checked_mul(MS_PER_DAY)
}

/// The start of a month, as a millisecond instant.
pub fn month_start_ms(year: i64, month: u32) -> Option<i64> {
    days_from_civil(year, month, 1)?.checked_mul(MS_PER_DAY)
}

/// The first instant of the month after `(year, month)`.
pub fn next_month_start_ms(year: i64, month: u32) -> Option<i64> {
    if month == 12 {
        year_start_ms(year.checked_add(1)?)
    } else {
        month_start_ms(year, month + 1)
    }
}

// ---------------------------------------------------------------------------
// Julian ↔ Gregorian
// ---------------------------------------------------------------------------

/// The Gregorian date of a Julian-calendar date, through Julian Day Numbers.
///
/// Applied **only in the index** and only to a value whose manifest says it is Julian (manifest
/// key 18), and never written back (A-2.3). A manifest that records 1611 in the Julian calendar
/// records the characters `1611`; converting on the way in would make the stored value disagree
/// with the title page it was copied from, and the mid is over the stored value.
///
/// The two calendars differ by 10 days in 1582 and by 13 in the twentieth century, which is why
/// this cannot be a constant: the drift is three days every four centuries.
pub fn julian_to_gregorian(year: i64, month: u32, day: u32) -> Option<Civil> {
    // The Julian calendar's own leap rule: every fourth year, with no century exception.
    if !(1..=12).contains(&month) || day == 0 || day > julian_days_in_month(year, month) {
        return None;
    }
    let jdn = julian_day_number(year, month, day)?;
    // JDN 2_440_588 is 1970-01-01 in the Gregorian calendar.
    Some(civil_from_days(jdn - 2_440_588))
}

/// The Julian Day Number of a Julian-calendar date, at noon-to-noon day granularity.
///
/// The standard expression, with the era shift done in integers so that it holds for negative
/// years as well. A fixture in `tests/time_algebra.rs` pins the three dates every library
/// disagrees about: 1582-10-04/15, 1700-02-29 (which exists in the Julian calendar and not in
/// the Gregorian) and 0001-01-01.
fn julian_day_number(year: i64, month: u32, day: u32) -> Option<i64> {
    let (y, m) = if month > 2 {
        (year, i64::from(month))
    } else {
        (year - 1, i64::from(month) + 12)
    };
    let jdn = (365.25_f64 * (y + 4716) as f64).floor() as i64
        + (30.6001_f64 * (m + 1) as f64).floor() as i64
        + i64::from(day)
        - 1524;
    Some(jdn)
}

fn julian_days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.rem_euclid(4) == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_epoch_round_trips() {
        assert_eq!(
            civil_from_days(0),
            Civil {
                year: 1970,
                month: 1,
                day: 1
            }
        );
        assert_eq!(days_from_civil(1970, 1, 1), Some(0));
        assert_eq!(ms_from_civil(1970, 1, 1, 0, 0, 0), Some(0));
    }

    /// Every day over four centuries, in both directions. The era arithmetic is exact or it is
    /// not, and a spot check would not say which.
    #[test]
    fn civil_and_days_are_inverses_over_four_centuries() {
        for days in -73_048..73_048 {
            let c = civil_from_days(days);
            assert_eq!(days_from_civil(c.year, c.month, c.day), Some(days), "{c:?}");
        }
    }

    #[test]
    fn the_calendar_is_proleptic_and_year_zero_is_a_leap_year() {
        assert!(is_leap(0));
        assert_eq!(
            days_from_civil(0, 2, 29).map(civil_from_days).unwrap().day,
            29
        );
        assert_eq!(days_from_civil(1900, 2, 29), None);
        assert!(days_from_civil(2000, 2, 29).is_some());
        assert_eq!(days_from_civil(2024, 13, 1), None);
        assert_eq!(days_from_civil(2024, 0, 1), None);
        assert_eq!(days_from_civil(2024, 1, 32), None);
    }

    /// The three dates every calendar library disagrees about.
    #[test]
    fn julian_to_gregorian_is_right_where_it_is_hard() {
        // The Gregorian reform: 1582-10-04 Julian is 1582-10-14 Gregorian, and the next
        // Julian day, the 5th, is the 15th — the ten days nobody lived through.
        assert_eq!(
            julian_to_gregorian(1582, 10, 4),
            Some(Civil {
                year: 1582,
                month: 10,
                day: 14
            })
        );
        assert_eq!(
            julian_to_gregorian(1582, 10, 5),
            Some(Civil {
                year: 1582,
                month: 10,
                day: 15
            })
        );
        // 1700 is a leap year in the Julian calendar and not in the Gregorian, so this date
        // exists in one and not the other. It must convert rather than be refused.
        assert_eq!(
            julian_to_gregorian(1700, 2, 29),
            Some(Civil {
                year: 1700,
                month: 3,
                day: 11
            })
        );
        // And 1700-02-30 does not exist in either.
        assert_eq!(julian_to_gregorian(1700, 2, 30), None);
        // The start of the era: 0001-01-01 Julian is 0000-12-30 Gregorian.
        assert_eq!(
            julian_to_gregorian(1, 1, 1),
            Some(Civil {
                year: 0,
                month: 12,
                day: 30
            })
        );
    }

    /// The drift is three days every four centuries, which is why no constant would do.
    #[test]
    fn the_two_calendars_drift_by_three_days_every_four_centuries() {
        let drift = |y: i64| -> i64 {
            let g = julian_to_gregorian(y, 3, 1).expect("1 March exists in both");
            days_from_civil(g.year, g.month, g.day).expect("a real date")
                - days_from_civil(y, 3, 1).expect("a real date")
        };
        assert_eq!(drift(1500), 10);
        assert_eq!(drift(1900), 13);
        assert_eq!(drift(2100), 14);
    }

    #[test]
    fn month_boundaries_are_where_the_months_end() {
        assert_eq!(
            next_month_start_ms(2024, 12),
            year_start_ms(2025),
            "December rolls over the year"
        );
        let feb = month_start_ms(2024, 2).unwrap();
        let mar = next_month_start_ms(2024, 2).unwrap();
        assert_eq!((mar - feb) / MS_PER_DAY, 29, "2024 is a leap year");
    }
}
