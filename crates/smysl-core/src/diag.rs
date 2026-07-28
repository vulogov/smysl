//! Diagnostic registry (RFC SMYSL-1 Appendix D) and the report machinery built on it.
//!
//! Every diagnostic the implementation can emit has a stable [`Code`] here. Codes are
//! stable across minor versions; retiring one requires a major bump (§25). Adding one is
//! a minor-version change, which is why [`Code`] is `#[non_exhaustive]`.
//!
//! The registry is single-sourced: the wire string, severity, Appendix D group, and the
//! one-line meaning all come from the `registry!` invocation below. Tests in this module
//! assert that the declared severity agrees with the `E`/`W` letter of the wire string,
//! so the two encodings of severity cannot drift apart.

use core::fmt;
use std::collections::BTreeMap;

use crate::error::Error;
use crate::ids::Uid;

// ---------------------------------------------------------------------------
// Severity
// ---------------------------------------------------------------------------

/// Diagnostic severity. Ordered: `Warn < Error`, so `fail_on(Warn)` is stricter than
/// `fail_on(Error)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Warn,
    Error,
}

impl Severity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Severity::Warn => "warning",
            Severity::Error => "error",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Byte spans
// ---------------------------------------------------------------------------

/// A half-open byte range into a surface-syntax source.
///
/// Spans are what let the ingest repair loop (§22.3) resend only the offending region
/// instead of retrying a whole chunk, so every parse-time diagnostic carries one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub const fn new(start: usize, end: usize) -> Span {
        Span { start, end }
    }

    pub const fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub const fn is_empty(self) -> bool {
        self.start >= self.end
    }

    /// The smallest span covering both.
    pub fn join(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    pub fn slice<'a>(&self, src: &'a str) -> Option<&'a str> {
        src.get(self.start..self.end)
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

// ---------------------------------------------------------------------------
// The registry
// ---------------------------------------------------------------------------

macro_rules! registry {
    ( $(
        $group:ident => {
            $( $var:ident = $wire:literal, $sev:ident, $msg:literal ; )+
        }
    )+ ) => {
        /// Appendix D section a code belongs to. Used for grouped reporting only; it is
        /// not part of the wire form.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[non_exhaustive]
        pub enum Group { $( $group, )+ }

        impl Group {
            pub const ALL: &'static [Group] = &[ $( Group::$group, )+ ];

            pub const fn as_str(self) -> &'static str {
                match self { $( Group::$group => stringify!($group), )+ }
            }
        }

        impl fmt::Display for Group {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.pad(self.as_str())
            }
        }

        /// A stable diagnostic code (Appendix D).
        ///
        /// Declaration order is Appendix D order, and `Ord` follows it, so grouped output
        /// is deterministic without a secondary sort key.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[non_exhaustive]
        pub enum Code { $( $( $var, )+ )+ }

        impl Code {
            /// Every code in the registry, in Appendix D order.
            pub const ALL: &'static [Code] = &[ $( $( Code::$var, )+ )+ ];

            /// The wire form, e.g. `"SMY-E030"`.
            pub const fn as_str(self) -> &'static str {
                match self { $( $( Code::$var => $wire, )+ )+ }
            }

            /// The severity this code is always emitted at.
            pub const fn severity(self) -> Severity {
                match self { $( $( Code::$var => Severity::$sev, )+ )+ }
            }

            /// The Appendix D section this code is registered under.
            pub const fn group(self) -> Group {
                match self { $( $( Code::$var => Group::$group, )+ )+ }
            }

            /// The registry's one-line meaning. Emitters may supply a more specific
            /// message on the [`Diagnostic`]; this is the fallback and the documentation.
            pub const fn message(self) -> &'static str {
                match self { $( $( Code::$var => $msg, )+ )+ }
            }

            /// Parse a wire form back into a code. Accepts only the canonical spelling.
            pub fn parse(s: &str) -> Option<Code> {
                match s {
                    $( $( $wire => Some(Code::$var), )+ )+
                    _ => None,
                }
            }
        }
    };
}

registry! {
    // --- Parse and encoding ------------------------------------------------
    Parse => {
        E001 = "SMY-E001", Error, "Surface parse error";
        E002 = "SMY-E002", Error, "Unsupported kernel major version";
        E003 = "SMY-E003", Error, "Unsupported format version";
        E004 = "SMY-E004", Error, "Malformed CBOR envelope";
        W014 = "SMY-W014", Warn,  "Unknown envelope type code - preserved verbatim, skipped semantically";
        E080 = "SMY-E080", Error, "Non-deterministic encoding (key order, indefinite length, non-shortest int, null optional, non-NFC text)";
        E081 = "SMY-E081", Error, "Float not binary32 or not quantised to 1/1024";
    }

    // --- Identity and integrity --------------------------------------------
    Identity => {
        E060 = "SMY-E060", Error, "Dangling reference";
        E061 = "SMY-E061", Error, "Cycle in deps";
        W062 = "SMY-W062", Warn,  "Cycle in causes or sequences";
        E070 = "SMY-E070", Error, "Hash mismatch - recomputed uid differs from stored uid";
        E071 = "SMY-E071", Error, "Truncated uid in a canonical record";
        E072 = "SMY-E072", Error, "Ambiguous uid prefix";
        W110 = "SMY-W110", Warn,  "Stale or corrupt index - rebuilding";
    }

    // --- LOD and granularity ------------------------------------------------
    Lod => {
        E020 = "SMY-E020", Error, "L1 closure violation - body references a uid absent from deps or grounds";
        E021 = "SMY-E021", Error, "Missing gist";
        E022 = "SMY-E022", Error, "Gist exceeds l0_max";
        E023 = "SMY-E023", Error, "detail without body";
        W024 = "SMY-W024", Warn,  "Gist appears to depend on body (heuristic; confirm via attest)";
        E040 = "SMY-E040", Error, "Multi-assertion body under single-assertion admission";
        W041 = "SMY-W041", Warn,  "Body outside l1_range";
    }

    // --- Epistemics ---------------------------------------------------------
    Epistemics => {
        E030 = "SMY-E030", Error, "Rule M violation - status exceeds weakest ground";
        E031 = "SMY-E031", Error, "derived/inferred with empty grounds";
        E032 = "SMY-E032", Error, "measured/cited without source";
        E033 = "SMY-E033", Error, "Rule T violation - status exceeds the ceiling for the attestation's rung";
        E034 = "SMY-E034", Error, "unfounded authored";
        W035 = "SMY-W035", Warn,  "measured with op: Authored rather than Imported";
        W036 = "SMY-W036", Warn,  "Rule M applied at ingest - status lowered to what its grounds support";
    }

    // --- Retraction, supersession, merge ------------------------------------
    Merge => {
        E050 = "SMY-E050", Error, "Orphaned grounds - all grounds retracted under strict";
        E051 = "SMY-E051", Error, "Retraction authority not satisfied";
        W052 = "SMY-W052", Warn,  "Retracted unit retained under advisory";
        W053 = "SMY-W053", Warn,  "Concurrent supersession materialised as a contention";
        W054 = "SMY-W054", Warn,  "Label bound to differing uids across views in scope";
        W055 = "SMY-W055", Warn,  "Agent contention rate exceeds --max-contentions-per-agent";
    }

    // --- Packing and rendering ----------------------------------------------
    PackRender => {
        E200 = "SMY-E200", Error, "Pack infeasible - C3/C4/C5 unsatisfiable; reports minimum feasible budget";
        E201 = "SMY-E201", Error, "Focus unit absent from store";
        W202 = "SMY-W202", Warn,  "Greedy mode above exact_threshold; optimality gap reported";
        E210 = "SMY-E210", Error, "Rule V1 - profile lacks a rendering for some status";
        W211 = "SMY-W211", Warn,  "Rule V2 - contentions suppressed; recorded in output metadata";
    }

    // --- Extension and conformance -------------------------------------------
    Extension => {
        W010 = "SMY-W010", Warn,  "Unknown schema - degraded fidelity (rule X)";
        E011 = "SMY-E011", Error, "Rule X violation - unrecognised payload dropped on re-emission";
        E012 = "SMY-E012", Error, "Extension schema attempts to weaken a kernel rule";
        W013 = "SMY-W013", Warn,  "Unknown relation kind treated as elaborates for closure";
    }

    // --- Provider and ingest --------------------------------------------------
    Provider => {
        E300 = "SMY-E300", Error, "Provider unreachable, no fallback configured";
        E301 = "SMY-E301", Error, "Offline violation";
        E302 = "SMY-E302", Error, "Context window exceeded after chunking";
        W303 = "SMY-W303", Warn,  "Structured output unsupported; fell back to the surface path";
        W304 = "SMY-W304", Warn,  "Span unrepairable; degraded to opaque prose (rule I)";
        W305 = "SMY-W305", Warn,  "Token count estimated rather than provider-reported";
        W306 = "SMY-W306", Warn,  "Usage threshold exceeded - informational only, never blocks";
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Diagnostic
// ---------------------------------------------------------------------------

/// What a diagnostic is about: a unit in a store, or a byte range in a source.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Subject {
    /// A byte range in surface text.
    Span(Span),
    /// A unit, identified by uid.
    Unit(Uid),
    /// Neither - a whole-store or whole-run diagnostic.
    Store,
}

/// One diagnostic: a stable code, a severity, what it is about, and an optional fix.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Diagnostic {
    pub code: Code,
    pub severity: Severity,
    pub subject: Subject,
    /// Human-readable detail. Defaults to `code.message()`.
    pub message: String,
    /// Suggested fix, when one can be named mechanically.
    pub suggestion: Option<String>,
}

impl Diagnostic {
    /// A diagnostic at the code's registered severity, with the registry message.
    pub fn new(code: Code) -> Diagnostic {
        Diagnostic {
            code,
            severity: code.severity(),
            subject: Subject::Store,
            message: code.message().to_string(),
            suggestion: None,
        }
    }

    pub fn at(code: Code, span: Span) -> Diagnostic {
        Diagnostic::new(code).with_subject(Subject::Span(span))
    }

    pub fn on(code: Code, uid: Uid) -> Diagnostic {
        Diagnostic::new(code).with_subject(Subject::Unit(uid))
    }

    pub fn with_subject(mut self, subject: Subject) -> Diagnostic {
        self.subject = subject;
        self
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Diagnostic {
        self.message = message.into();
        self
    }

    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Diagnostic {
        self.suggestion = Some(suggestion.into());
        self
    }

    pub fn span(&self) -> Option<Span> {
        match self.subject {
            Subject::Span(s) => Some(s),
            _ => None,
        }
    }

    pub fn uid(&self) -> Option<&Uid> {
        match &self.subject {
            Subject::Unit(u) => Some(u),
            _ => None,
        }
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}: {}", self.severity, self.code, self.message)?;
        match &self.subject {
            Subject::Span(s) => write!(f, " (at {s})")?,
            Subject::Unit(u) => write!(f, " (at {u})")?,
            Subject::Store => {}
        }
        if let Some(s) = &self.suggestion {
            write!(f, " [try: {s}]")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

/// A collected diagnostic set plus per-code counts (§17).
///
/// Check passes never short-circuit, so a report is expected to carry the *full*
/// diagnostic set: the ingest repair loop (§22.3) needs all of them at once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub diagnostics: Vec<Diagnostic>,
    pub counts: BTreeMap<Code, usize>,
}

impl Report {
    pub fn new() -> Report {
        Report::default()
    }

    pub fn push(&mut self, d: Diagnostic) {
        *self.counts.entry(d.code).or_insert(0) += 1;
        self.diagnostics.push(d);
    }

    pub fn extend(&mut self, other: impl IntoIterator<Item = Diagnostic>) {
        for d in other {
            self.push(d);
        }
    }

    /// Absorb another report. Union of diagnostics, sum of counts.
    pub fn absorb(&mut self, other: Report) {
        self.extend(other.diagnostics);
    }

    pub fn count(&self, code: Code) -> usize {
        self.counts.get(&code).copied().unwrap_or(0)
    }

    pub fn len(&self) -> usize {
        self.diagnostics.len()
    }

    pub fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }

    pub fn max_severity(&self) -> Option<Severity> {
        self.diagnostics.iter().map(|d| d.severity).max()
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }

    /// No error-severity diagnostics. Warnings are compatible with a clean result -
    /// rule I depends on that, since a degraded span (`W304`) must not fail a run.
    pub fn is_clean(&self) -> bool {
        !self.has_errors()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Diagnostic> {
        self.diagnostics.iter()
    }

    /// Diagnostics of exactly this severity.
    pub fn of_severity(&self, s: Severity) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(move |d| d.severity == s)
    }

    /// Sort into a canonical order so report output is bit-reproducible (rule D).
    pub fn sort(&mut self) {
        self.diagnostics.sort();
    }

    /// `Err` if any diagnostic is at or above `s`.
    pub fn fail_on(&self, s: Severity) -> Result<(), Error> {
        if self.diagnostics.iter().any(|d| d.severity >= s) {
            Err(Error::Check(self.clone()))
        } else {
            Ok(())
        }
    }
}

impl FromIterator<Diagnostic> for Report {
    fn from_iter<I: IntoIterator<Item = Diagnostic>>(iter: I) -> Report {
        let mut r = Report::new();
        r.extend(iter);
        r
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for d in &self.diagnostics {
            writeln!(f, "{d}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry declares severity twice - as the `E`/`W` letter of the wire string
    /// and as an explicit `Severity`. They must not drift.
    #[test]
    fn severity_agrees_with_wire_letter() {
        for &c in Code::ALL {
            let letter = c.as_str().as_bytes()[4];
            let expected = match letter {
                b'E' => Severity::Error,
                b'W' => Severity::Warn,
                other => panic!("{c}: unexpected severity letter {}", other as char),
            };
            assert_eq!(
                c.severity(),
                expected,
                "{c} severity disagrees with wire form"
            );
        }
    }

    #[test]
    fn every_code_has_the_smy_prefix_and_four_digits() {
        for &c in Code::ALL {
            let s = c.as_str();
            assert_eq!(s.len(), 8, "{s} is not 8 characters");
            assert!(s.starts_with("SMY-"), "{s} lacks the SMY- prefix");
            assert!(
                s[5..].bytes().all(|b| b.is_ascii_digit()),
                "{s} does not end in three digits"
            );
        }
    }

    #[test]
    fn wire_forms_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for &c in Code::ALL {
            assert!(seen.insert(c.as_str()), "duplicate wire form {c}");
        }
        assert_eq!(seen.len(), Code::ALL.len());
    }

    #[test]
    fn every_code_has_a_nonempty_message() {
        for &c in Code::ALL {
            assert!(!c.message().is_empty(), "{c} has no message");
        }
    }

    #[test]
    fn parse_round_trips_every_code() {
        for &c in Code::ALL {
            assert_eq!(Code::parse(c.as_str()), Some(c));
        }
        assert_eq!(Code::parse("SMY-E999"), None);
        assert_eq!(Code::parse("E030"), None);
        assert_eq!(Code::parse(""), None);
    }

    /// Appendix D, counted section by section: 7 + 7 + 7 + 7 + 6 + 5 + 4 + 7.
    ///
    /// **One more than Appendix D lists.** `SMY-W036` is an addition: rule M at the ingest
    /// boundary lowers an over-claiming unit rather than rejecting it, and the lowering has
    /// to be reportable. Appendix D has no code for it because §9.1 assumed rejection, so
    /// this is a divergence to reconcile rather than a miscount.
    #[test]
    fn registry_matches_appendix_d_size() {
        assert_eq!(Code::ALL.len(), 50);
    }

    #[test]
    fn group_membership_is_complete() {
        for &g in Group::ALL {
            assert!(
                Code::ALL.iter().any(|c| c.group() == g),
                "group {g} has no codes"
            );
        }
        assert_eq!(Group::ALL.len(), 8);
    }

    #[test]
    fn code_ordering_is_declaration_order() {
        let mut sorted = Code::ALL.to_vec();
        sorted.sort();
        assert_eq!(sorted, Code::ALL.to_vec());
    }

    #[test]
    fn display_is_the_wire_form() {
        assert_eq!(Code::E030.to_string(), "SMY-E030");
        assert_eq!(Code::W304.to_string(), "SMY-W304");
    }

    #[test]
    fn severity_orders_warn_below_error() {
        assert!(Severity::Warn < Severity::Error);
    }

    #[test]
    fn diagnostic_defaults_to_registry_severity_and_message() {
        let d = Diagnostic::new(Code::E030);
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.message, Code::E030.message());
        assert_eq!(d.subject, Subject::Store);
        assert!(d.is_error());
    }

    #[test]
    fn diagnostic_builders_set_subject() {
        let d = Diagnostic::at(Code::E001, Span::new(4, 9));
        assert_eq!(d.span(), Some(Span::new(4, 9)));
        assert!(d.uid().is_none());

        let u = Uid::from_bytes([7u8; 32]);
        let d = Diagnostic::on(Code::E030, u).with_suggestion("weaken to speculative");
        assert_eq!(d.uid(), Some(&u));
        assert!(d.span().is_none());
        assert_eq!(d.suggestion.as_deref(), Some("weaken to speculative"));
    }

    #[test]
    fn report_counts_by_code() {
        let mut r = Report::new();
        r.push(Diagnostic::new(Code::E030));
        r.push(Diagnostic::new(Code::E030));
        r.push(Diagnostic::new(Code::W304));
        assert_eq!(r.count(Code::E030), 2);
        assert_eq!(r.count(Code::W304), 1);
        assert_eq!(r.count(Code::E001), 0);
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn warnings_alone_are_clean_but_not_empty() {
        let mut r = Report::new();
        r.push(Diagnostic::new(Code::W304));
        assert!(r.is_clean(), "rule I depends on warnings not failing a run");
        assert!(!r.is_empty());
        assert!(!r.has_errors());
        assert_eq!(r.max_severity(), Some(Severity::Warn));
    }

    #[test]
    fn fail_on_respects_severity_threshold() {
        let mut r = Report::new();
        r.push(Diagnostic::new(Code::W304));
        assert!(r.fail_on(Severity::Error).is_ok());
        assert!(r.fail_on(Severity::Warn).is_err());

        r.push(Diagnostic::new(Code::E030));
        assert!(r.fail_on(Severity::Error).is_err());
    }

    #[test]
    fn empty_report_never_fails() {
        let r = Report::new();
        assert!(r.fail_on(Severity::Warn).is_ok());
        assert!(r.fail_on(Severity::Error).is_ok());
        assert_eq!(r.max_severity(), None);
        assert!(r.is_clean());
    }

    #[test]
    fn absorb_unions_diagnostics_and_sums_counts() {
        let mut a = Report::new();
        a.push(Diagnostic::new(Code::E030));
        let mut b = Report::new();
        b.push(Diagnostic::new(Code::E030));
        b.push(Diagnostic::new(Code::E021));
        a.absorb(b);
        assert_eq!(a.count(Code::E030), 2);
        assert_eq!(a.count(Code::E021), 1);
        assert_eq!(a.len(), 3);
    }

    #[test]
    fn sort_is_canonical_and_idempotent() {
        let mut r = Report::new();
        r.push(Diagnostic::at(Code::E030, Span::new(9, 10)));
        r.push(Diagnostic::at(Code::E001, Span::new(0, 1)));
        r.push(Diagnostic::new(Code::W304));
        r.sort();
        let once = r.clone();
        r.sort();
        assert_eq!(r, once);
        assert_eq!(r.diagnostics[0].code, Code::E001);
    }

    #[test]
    fn spans_join_and_slice() {
        let s = Span::new(2, 5).join(Span::new(7, 9));
        assert_eq!(s, Span::new(2, 9));
        assert_eq!(s.len(), 7);
        assert!(!s.is_empty());
        assert!(Span::new(3, 3).is_empty());
        assert_eq!(Span::new(0, 5).slice("@claim c/x"), Some("@clai"));
        assert_eq!(Span::new(0, 99).slice("short"), None);
    }
}
