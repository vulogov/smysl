//! Epistemic status, level of detail, and source references (§1.3, §1.4, §9.3).

use core::fmt;

use crate::error::IdError;

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// What an agent claimed about its own certainty - never what is so (N1).
///
/// The integer order **is** the order rule M compares, which is why the discriminants are
/// pinned: changing one would silently change what rule M permits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum Status {
    /// Reachable only by retraction. MUST NOT be authored (`SMY-E034`).
    Unfounded = 0,
    Speculative = 1,
    Inferred = 2,
    Derived = 3,
    Cited = 4,
    Measured = 5,
}

impl Status {
    pub const ALL: &'static [Status] = &[
        Status::Unfounded,
        Status::Speculative,
        Status::Inferred,
        Status::Derived,
        Status::Cited,
        Status::Measured,
    ];

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn from_u8(v: u8) -> Option<Status> {
        match v {
            0 => Some(Status::Unfounded),
            1 => Some(Status::Speculative),
            2 => Some(Status::Inferred),
            3 => Some(Status::Derived),
            4 => Some(Status::Cited),
            5 => Some(Status::Measured),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Status::Unfounded => "unfounded",
            Status::Speculative => "speculative",
            Status::Inferred => "inferred",
            Status::Derived => "derived",
            Status::Cited => "cited",
            Status::Measured => "measured",
        }
    }

    pub fn parse(s: &str) -> Option<Status> {
        Status::ALL.iter().copied().find(|st| st.as_str() == s)
    }

    /// `measured` and `cited` ground out externally and MUST carry a source (§1.4).
    pub const fn requires_source(self) -> bool {
        matches!(self, Status::Measured | Status::Cited)
    }

    /// `derived` and `inferred` are claims about other units and MUST have grounds.
    pub const fn requires_grounds(self) -> bool {
        matches!(self, Status::Derived | Status::Inferred)
    }

    /// Whether rule M's ceiling applies. `measured` and `cited` are exempt because their
    /// support is external to the graph.
    pub const fn is_rule_m_constrained(self) -> bool {
        self.requires_grounds()
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Level of detail
// ---------------------------------------------------------------------------

/// Authored level of detail (rule L). Making summarisation a lookup is what gives
/// `summarize(summarize(x)) = summarize(x)` a fixed point (F5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum Lod {
    L0 = 0,
    L1 = 1,
    L2 = 2,
}

impl Lod {
    pub const ALL: &'static [Lod] = &[Lod::L0, Lod::L1, Lod::L2];

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn from_u8(v: u8) -> Option<Lod> {
        match v {
            0 => Some(Lod::L0),
            1 => Some(Lod::L1),
            2 => Some(Lod::L2),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Lod::L0 => "L0",
            Lod::L1 => "L1",
            Lod::L2 => "L2",
        }
    }

    pub fn parse(s: &str) -> Option<Lod> {
        Lod::ALL.iter().copied().find(|l| l.as_str() == s)
    }

    /// Packing value gain (§8). Diminishing, so breadth beats depth at equal salience.
    pub const fn gain(self) -> f32 {
        match self {
            Lod::L0 => 1.0,
            Lod::L1 => 1.6,
            Lod::L2 => 1.85,
        }
    }
}

impl fmt::Display for Lod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum SourceKind {
    Url = 0,
    File = 1,
    Metric = 2,
    Tool = 3,
    Doc = 4,
}

impl SourceKind {
    pub const ALL: &'static [SourceKind] = &[
        SourceKind::Url,
        SourceKind::File,
        SourceKind::Metric,
        SourceKind::Tool,
        SourceKind::Doc,
    ];

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn from_u8(v: u8) -> Option<SourceKind> {
        match v {
            0 => Some(SourceKind::Url),
            1 => Some(SourceKind::File),
            2 => Some(SourceKind::Metric),
            3 => Some(SourceKind::Tool),
            4 => Some(SourceKind::Doc),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            SourceKind::Url => "url",
            SourceKind::File => "file",
            SourceKind::Metric => "metric",
            SourceKind::Tool => "tool",
            SourceKind::Doc => "doc",
        }
    }

    pub fn parse(s: &str) -> Option<SourceKind> {
        SourceKind::ALL.iter().copied().find(|k| k.as_str() == s)
    }

    /// Whether a `web`-rung ingest of this kind must also carry a capture date (§9.3).
    pub const fn requires_capture_date(self) -> bool {
        matches!(self, SourceKind::Url)
    }
}

impl fmt::Display for SourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A calendar date, stored and encoded as `YYYY-MM-DD`.
///
/// Deliberately not a timestamp: `captured` records when a source was observed, and
/// sub-day precision would invite a wall-clock read into an otherwise pure path (rule D).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

impl Date {
    pub fn new(year: u16, month: u8, day: u8) -> Result<Date, IdError> {
        let d = Date { year, month, day };
        if d.is_valid() {
            Ok(d)
        } else {
            Err(IdError {
                kind: "date",
                found: format!("{year:04}-{month:02}-{day:02}"),
            })
        }
    }

    fn is_valid(&self) -> bool {
        if self.year == 0 || !(1..=12).contains(&self.month) || self.day == 0 {
            return false;
        }
        self.day <= days_in_month(self.year, self.month)
    }

    pub fn parse(s: &str) -> Result<Date, IdError> {
        let err = || IdError {
            kind: "date",
            found: s.to_string(),
        };
        let b = s.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
            return Err(err());
        }
        let num = |r: std::ops::Range<usize>| -> Result<u16, IdError> {
            s[r].parse::<u16>().map_err(|_| err())
        };
        let year = num(0..4)?;
        let month = num(5..7)? as u8;
        let day = num(8..10)? as u8;
        Date::new(year, month, day)
    }
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap(y: u16) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// Where a `measured` or `cited` unit grounds out (§1.1).
///
/// Only an instrument or tool adapter recording `op: Imported` with a machine-checkable
/// reference may assign `measured` (rule T); `ingest` never may, however confidently a
/// model phrases the claim.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceRef {
    pub kind: SourceKind,
    pub reference: String,
    pub captured: Option<Date>,
}

impl SourceRef {
    pub fn new(kind: SourceKind, reference: impl Into<String>) -> SourceRef {
        SourceRef {
            kind,
            reference: reference.into(),
            captured: None,
        }
    }

    pub fn captured_on(mut self, d: Date) -> SourceRef {
        self.captured = Some(d);
        self
    }
}

impl fmt::Display for SourceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.kind, self.reference)?;
        if let Some(d) = self.captured {
            write!(f, " @{d}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_discriminants_are_the_rule_m_order() {
        assert_eq!(Status::Unfounded.as_u8(), 0);
        assert_eq!(Status::Speculative.as_u8(), 1);
        assert_eq!(Status::Inferred.as_u8(), 2);
        assert_eq!(Status::Derived.as_u8(), 3);
        assert_eq!(Status::Cited.as_u8(), 4);
        assert_eq!(Status::Measured.as_u8(), 5);
        assert!(Status::Speculative < Status::Inferred);
        assert!(Status::Derived < Status::Measured);
    }

    #[test]
    fn status_round_trips_through_u8_and_text() {
        for &s in Status::ALL {
            assert_eq!(Status::from_u8(s.as_u8()), Some(s));
            assert_eq!(Status::parse(s.as_str()), Some(s));
        }
        assert_eq!(Status::from_u8(6), None);
        assert_eq!(Status::parse("certain"), None);
    }

    #[test]
    fn only_measured_and_cited_require_a_source() {
        let need: Vec<&str> = Status::ALL
            .iter()
            .filter(|s| s.requires_source())
            .map(|s| s.as_str())
            .collect();
        assert_eq!(need, ["cited", "measured"]);
    }

    #[test]
    fn only_derived_and_inferred_require_grounds_and_bind_rule_m() {
        let need: Vec<&str> = Status::ALL
            .iter()
            .filter(|s| s.requires_grounds())
            .map(|s| s.as_str())
            .collect();
        assert_eq!(need, ["inferred", "derived"]);
        for &s in Status::ALL {
            assert_eq!(s.is_rule_m_constrained(), s.requires_grounds());
        }
    }

    /// Rule M is exempt for the two statuses that ground out externally; if that ever
    /// stopped being true, a `measured` unit would need a `measured` ground, which does
    /// not exist.
    #[test]
    fn externally_grounded_statuses_are_exempt_from_rule_m() {
        assert!(!Status::Measured.is_rule_m_constrained());
        assert!(!Status::Cited.is_rule_m_constrained());
    }

    #[test]
    fn lod_gains_diminish() {
        assert_eq!(Lod::L0.gain(), 1.0);
        assert!(Lod::L1.gain() > Lod::L0.gain());
        assert!(Lod::L2.gain() > Lod::L1.gain());
        assert!(
            Lod::L2.gain() - Lod::L1.gain() < Lod::L1.gain() - Lod::L0.gain(),
            "breadth must beat depth at equal salience"
        );
    }

    #[test]
    fn lod_round_trips() {
        for &l in Lod::ALL {
            assert_eq!(Lod::from_u8(l.as_u8()), Some(l));
            assert_eq!(Lod::parse(l.as_str()), Some(l));
        }
        assert_eq!(Lod::from_u8(3), None);
    }

    #[test]
    fn source_kinds_round_trip() {
        assert_eq!(SourceKind::ALL.len(), 5);
        for &k in SourceKind::ALL {
            assert_eq!(SourceKind::from_u8(k.as_u8()), Some(k));
            assert_eq!(SourceKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(SourceKind::from_u8(5), None);
    }

    #[test]
    fn dates_validate_the_calendar() {
        assert!(Date::new(2026, 7, 9).is_ok());
        assert!(Date::new(2024, 2, 29).is_ok(), "2024 is a leap year");
        assert!(Date::new(2026, 2, 29).is_err());
        assert!(Date::new(1900, 2, 29).is_err(), "1900 is not a leap year");
        assert!(Date::new(2000, 2, 29).is_ok(), "2000 is a leap year");
        assert!(Date::new(2026, 13, 1).is_err());
        assert!(Date::new(2026, 0, 1).is_err());
        assert!(Date::new(2026, 4, 31).is_err());
        assert!(Date::new(0, 1, 1).is_err());
    }

    #[test]
    fn dates_round_trip_through_iso_text() {
        let d = Date::new(2026, 7, 9).unwrap();
        assert_eq!(d.to_string(), "2026-07-09");
        assert_eq!(Date::parse("2026-07-09").unwrap(), d);
    }

    #[test]
    fn malformed_dates_are_rejected() {
        for s in [
            "",
            "2026-7-9",
            "2026/07/09",
            "20260709",
            "2026-07-09T00:00:00",
            "xxxx-xx-xx",
        ] {
            assert!(Date::parse(s).is_err(), "`{s}` must be rejected");
        }
    }

    #[test]
    fn dates_order_chronologically() {
        let a = Date::new(2026, 7, 9).unwrap();
        let b = Date::new(2026, 7, 10).unwrap();
        let c = Date::new(2026, 8, 1).unwrap();
        assert!(a < b && b < c);
    }

    #[test]
    fn source_refs_carry_an_optional_capture_date() {
        let s = SourceRef::new(SourceKind::Metric, "grafana://board/12/panel/4");
        assert!(s.captured.is_none());
        let s = s.captured_on(Date::new(2026, 7, 9).unwrap());
        assert_eq!(s.captured.unwrap().to_string(), "2026-07-09");
        assert!(s.to_string().contains("metric:"));
    }

    /// A fetched URL without a capture date is unverifiable later - the page changed.
    #[test]
    fn only_urls_demand_a_capture_date() {
        assert!(SourceKind::Url.requires_capture_date());
        for k in [
            SourceKind::File,
            SourceKind::Metric,
            SourceKind::Tool,
            SourceKind::Doc,
        ] {
            assert!(!k.requires_capture_date());
        }
    }
}
