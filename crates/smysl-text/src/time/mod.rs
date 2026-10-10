//! Effective time: rule E (SMYSL-2.3 A-12.2), and the instants it is computed over.
//!
//! Four axes, three of which rule E derives: *said* (when it was said or published), *composed*
//! (when the work was written) and *about* (the time the claim refers to). The fourth, *known*,
//! is the earliest attestation clock of a unit and is never corrected — there is nothing for a
//! dating to say about it.
//!
//! **Effective time is never stored** (A-12.2). An implementation MAY index it and MUST
//! recompute it when the record set changes, which is what makes a dating a record about units
//! rather than an edit to them: correcting a unit's `observed` in place would change its uid,
//! and a store that re-identified its contents whenever a clock turned out to be wrong could
//! not be cited.

pub mod calendar;
pub mod constraints;
pub mod edtf;
pub mod engine;
pub mod stn;

pub use smysl_core::types::Axis;

/// A thing rule E assigns a time to.
///
/// Units, and **manifests**. A manifest is in here because two of the five free constraints are
/// about manifests and nothing else: a manifest is no earlier than the one it derives from (key
/// 11) and no earlier than the one it supersedes (key 13). Leaving manifests out would have left
/// those two constraints with nothing to constrain.
///
/// Parts and windows are *scopes* rather than subjects: a dating names them and they resolve to
/// the units drawn from them. The distinction matters because a part has no time of its own —
/// the bytes were not said at a moment, the messages in them were.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Subject {
    Unit(smysl_core::Uid),
    Manifest(smysl_core::Mid),
}

impl std::fmt::Display for Subject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Subject::Unit(u) => write!(f, "{u}"),
            Subject::Manifest(m) => write!(f, "{m}"),
        }
    }
}

/// A millisecond instant, proleptic Gregorian, counted from the Unix epoch.
///
/// `i64` milliseconds reaches about ±2.9 × 10⁸ years, which comfortably contains every date a
/// corpus of texts can name and every `Y`-prefixed EDTF year this format accepts. A value
/// outside it is kept as text and indexed as an open bound (`SMY-W449`) rather than clamped: a
/// clamped bound is a claim nobody made, and it would propagate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant(pub i64);

impl Instant {
    pub const EPOCH: Instant = Instant(0);

    pub const fn ms(self) -> i64 {
        self.0
    }

    pub fn civil(self) -> calendar::Civil {
        calendar::civil_from_ms(self.0)
    }
}

impl std::fmt::Display for Instant {
    /// ISO 8601 to the second, with `Z`, and the millisecond only when it is not zero.
    ///
    /// For a why-chain and a `date show`, both of which a person reads. The millisecond is
    /// suppressed when it is zero because every date-level value has one and printing `.000`
    /// four times a line hides the digits that differ.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let c = self.civil();
        let rem = self.0.rem_euclid(calendar::MS_PER_DAY);
        let (h, m, s, ms) = (
            rem / 3_600_000,
            rem % 3_600_000 / 60_000,
            rem % 60_000 / 1000,
            rem % 1000,
        );
        write!(
            f,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            c.year, c.month, c.day, h, m, s
        )?;
        if ms != 0 {
            write!(f, ".{ms:03}")?;
        }
        f.write_str("Z")
    }
}

/// One end of an interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Bound {
    /// No bound on this side. "Published from 1984 onwards", or undated.
    Open,
    At(Instant),
}

impl Bound {
    pub const fn instant(self) -> Option<Instant> {
        match self {
            Bound::At(i) => Some(i),
            Bound::Open => None,
        }
    }
}

/// An interval of instants, **half-open**: `[lo, hi)`.
///
/// Half-open at millisecond precision is what makes A-2.4 rule 3's interval equality exact
/// integer comparison rather than a tolerance. It is also what makes the rule bite so rarely,
/// which is the point: `2026-09-30T14:05:00Z` is a one-*second* interval, and it is not equal
/// to the one-millisecond interval any `observed` instant denotes, so a producer must keep both
/// values rather than deciding they say the same thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Interval {
    pub lo: Bound,
    pub hi: Bound,
}

impl Interval {
    /// The interval with no bounds at all: *undated*. Not early, not late (A-2.4 rule 5).
    pub const UNDATED: Interval = Interval {
        lo: Bound::Open,
        hi: Bound::Open,
    };

    /// `[at, at + 1)`: the one millisecond an instant denotes.
    ///
    /// The degenerate interval of A-2.4 rule 3, and one millisecond wide rather than zero wide
    /// because a zero-wide half-open interval is empty, and an `observed` instant is a claim
    /// about a millisecond rather than about nothing.
    pub fn at(at: Instant) -> Interval {
        Interval {
            lo: Bound::At(at),
            hi: Bound::At(Instant(at.0.saturating_add(1))),
        }
    }

    /// `[lo, hi)` from two millisecond counts.
    pub fn of(lo: i64, hi: i64) -> Interval {
        Interval {
            lo: Bound::At(Instant(lo)),
            hi: Bound::At(Instant(hi)),
        }
    }

    pub fn is_undated(&self) -> bool {
        self.lo == Bound::Open && self.hi == Bound::Open
    }

    /// Whether the interval contains no instant at all, which rule E step 5 calls out.
    pub fn is_empty(&self) -> bool {
        match (self.lo, self.hi) {
            (Bound::At(a), Bound::At(b)) => a.0 >= b.0,
            _ => false,
        }
    }

    pub fn contains(&self, at: Instant) -> bool {
        self.lo.instant().is_none_or(|l| at.0 >= l.0)
            && self.hi.instant().is_none_or(|h| at.0 < h.0)
    }

    /// The tightest interval containing both: what a coarse claim widens to.
    pub fn union(self, other: Interval) -> Interval {
        let lo = match (self.lo, other.lo) {
            (Bound::At(a), Bound::At(b)) => Bound::At(Instant(a.0.min(b.0))),
            _ => Bound::Open,
        };
        let hi = match (self.hi, other.hi) {
            (Bound::At(a), Bound::At(b)) => Bound::At(Instant(a.0.max(b.0))),
            _ => Bound::Open,
        };
        Interval { lo, hi }
    }

    /// The overlap, which may be empty.
    pub fn intersect(self, other: Interval) -> Interval {
        let lo = match (self.lo, other.lo) {
            (Bound::At(a), Bound::At(b)) => Bound::At(Instant(a.0.max(b.0))),
            (Bound::At(a), Bound::Open) | (Bound::Open, Bound::At(a)) => Bound::At(a),
            _ => Bound::Open,
        };
        let hi = match (self.hi, other.hi) {
            (Bound::At(a), Bound::At(b)) => Bound::At(Instant(a.0.min(b.0))),
            (Bound::At(a), Bound::Open) | (Bound::Open, Bound::At(a)) => Bound::At(a),
            _ => Bound::Open,
        };
        Interval { lo, hi }
    }

    /// Shift by a signed offset in milliseconds. Saturating, because an offset that pushed a
    /// bound past the range of the type would otherwise wrap and put a date in another era.
    pub fn shift(self, ms: i64) -> Interval {
        let m = |b: Bound| match b {
            Bound::At(i) => Bound::At(Instant(i.0.saturating_add(ms))),
            Bound::Open => Bound::Open,
        };
        Interval {
            lo: m(self.lo),
            hi: m(self.hi),
        }
    }
}

impl std::fmt::Display for Interval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_undated() {
            return f.write_str("undated");
        }
        match self.lo {
            Bound::At(i) => write!(f, "[{i}")?,
            Bound::Open => f.write_str("(..")?,
        }
        f.write_str(", ")?;
        match self.hi {
            Bound::At(i) => write!(f, "{i})"),
            Bound::Open => f.write_str("..)"),
        }
    }
}

/// How well a time value is evidenced (A-12.2), and the order rule E stratifies by.
///
/// The same four names the epistemic statuses use and **not** the same enumeration: a unit's
/// `Status` is about the claim, and this is about the clock behind one of its dates. A unit may
/// be `measured` and carry a `speculative` date somebody typed in, which is exactly the pair
/// the two enumerations have to be able to express at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TimeStatus {
    /// Somebody's guess, with nothing behind it. The weakest, and the default.
    Speculative = 0,
    /// Computed by this implementation from records it holds.
    Derived = 1,
    /// A source said so: a reader's `observed`, a manifest's `published`.
    Cited = 2,
    /// An instrument recorded it: `observed` on a unit attested `Imported` at rung `computed`.
    Measured = 3,
}

impl TimeStatus {
    /// Highest first, which is the order rule E's strata run in.
    pub const STRATA: &'static [TimeStatus] = &[
        TimeStatus::Measured,
        TimeStatus::Cited,
        TimeStatus::Derived,
        TimeStatus::Speculative,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            TimeStatus::Speculative => "speculative",
            TimeStatus::Derived => "derived",
            TimeStatus::Cited => "cited",
            TimeStatus::Measured => "measured",
        }
    }
}

impl std::fmt::Display for TimeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_instant_prints_as_a_person_reads_it() {
        assert_eq!(Instant(0).to_string(), "1970-01-01T00:00:00Z");
        assert_eq!(Instant(1726500000000).to_string(), "2024-09-16T15:20:00Z");
        assert_eq!(
            Instant(1726500000123).to_string(),
            "2024-09-16T15:20:00.123Z"
        );
        // Before the epoch, where `div_euclid` is the whole reason this is right.
        assert_eq!(Instant(-1).to_string(), "1969-12-31T23:59:59.999Z");
    }

    /// Rule 3's comparison, which is the reason the interval is half-open at millisecond
    /// precision. A one-second EDTF value is not an instant, and the rule says so.
    #[test]
    fn a_second_wide_interval_is_not_an_instant() {
        let t = 1726500000000;
        assert_ne!(Interval::of(t, t + 1000), Interval::at(Instant(t)));
        assert_eq!(Interval::of(t, t + 1), Interval::at(Instant(t)));
    }

    #[test]
    fn an_empty_interval_is_one_that_contains_nothing() {
        assert!(Interval::of(5, 5).is_empty());
        assert!(Interval::of(6, 5).is_empty());
        assert!(!Interval::of(5, 6).is_empty());
        assert!(!Interval::UNDATED.is_empty());
        assert!(Interval::UNDATED.is_undated());
    }

    #[test]
    fn intersecting_with_an_open_end_keeps_the_bound_there_is() {
        let open_hi = Interval {
            lo: Bound::At(Instant(10)),
            hi: Bound::Open,
        };
        assert_eq!(open_hi.intersect(Interval::of(0, 20)), Interval::of(10, 20));
        assert_eq!(
            Interval::UNDATED.intersect(Interval::of(0, 20)),
            Interval::of(0, 20)
        );
    }

    #[test]
    fn a_union_widens_to_whichever_side_is_unbounded() {
        assert_eq!(
            Interval::of(0, 5).union(Interval::of(3, 9)),
            Interval::of(0, 9)
        );
        assert_eq!(
            Interval::of(0, 5).union(Interval::UNDATED),
            Interval::UNDATED
        );
    }

    #[test]
    fn containment_is_half_open() {
        let i = Interval::of(10, 20);
        assert!(i.contains(Instant(10)));
        assert!(i.contains(Instant(19)));
        assert!(!i.contains(Instant(20)));
        assert!(!i.contains(Instant(9)));
        assert!(Interval::UNDATED.contains(Instant(i64::MIN)));
    }

    #[test]
    fn the_strata_run_from_measured_down() {
        assert_eq!(TimeStatus::STRATA[0], TimeStatus::Measured);
        assert_eq!(TimeStatus::STRATA[3], TimeStatus::Speculative);
        assert!(TimeStatus::Measured > TimeStatus::Cited);
        assert!(TimeStatus::Cited > TimeStatus::Derived);
        assert!(TimeStatus::Derived > TimeStatus::Speculative);
    }
}
