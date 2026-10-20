//! The diagnostic registry, and the report machinery built on it.
//!
//! Appendix D of the manual transcribes this table; the two are cross-checked at release.
//! It used to cite RFC SMYSL-1, retired in 0.5.0 — the codes are defined here now, not
//! ratified elsewhere.
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
///
/// **Closed by design** (§1.1 audit, 0.13). Two, and the whole diagnostic model rests on the
/// split: `Error` blocks, `Warn` does not. A third level would not be a new variant so much as
/// a different design, and every `match` on this in the workspace is exhaustive on purpose.
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
///
/// **Closed by design** (§1.1 audit, 0.13). A byte range is two offsets. Anything a
/// diagnostic wants to say *about* the range belongs on `Diagnostic`, which is
/// `#[non_exhaustive]` for exactly that reason.
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

// Groups are declared once and blocks name them, so a block may name a group again. `Code` is
// numbered in declaration order and `cargo-semver-checks` treats a renumbered variant as a major
// break, so a code added after a release goes at the end — in a trailing block for its own group
// when that is not the last one. 1.4's `W056` is the first to need it.
macro_rules! registry {
    (
        groups { $( $g:ident ),+ $(,)? }
        $(
            $group:ident => {
                $( $var:ident = $wire:literal, $sev:ident, $msg:literal ; )+
            }
        )+
    ) => {
        /// Appendix D section a code belongs to. Used for grouped reporting only; it is
        /// not part of the wire form.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[non_exhaustive]
        pub enum Group { $( $g, )+ }

        impl Group {
            pub const ALL: &'static [Group] = &[ $( Group::$g, )+ ];

            pub const fn as_str(self) -> &'static str {
                match self { $( Group::$g => stringify!($g), )+ }
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
    groups { Parse, Identity, Lod, Epistemics, Merge, PackRender, Extension, Provider, Library, Query, Measure }

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
        W054 = "SMY-W054", Warn,  "Label and uid do not correspond one to one - two labels for a unit, or one label for two";
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
        E307 = "SMY-E307", Error, "Attributed quote does not occur in the source text";
        W308 = "SMY-W308", Warn,  "Attributed quote occurs only loosely - elided or reworded";
        W309 = "SMY-W309", Warn,  "A unit's own source was replaced by the caller's (source policy override)";
    }

    // --- Added after 1.3, each in its own group, at the end so no code is renumbered ----------
    Merge => {
        W056 = "SMY-W056", Warn,  "Withdrawal of a retracts or supersedes edge has no effect";
    }
    PackRender => {
        E203 = "SMY-E203", Error, "Reservation leaves no budget to pack into";
    }
    Identity => {
        W111 = "SMY-W111", Warn,  "The log holds records more than once; compact removes them";
    }
    Epistemics => {
        W057 = "SMY-W057", Warn,  "A unit is more committed than the weakest thing it rests on";
    }
    Merge => {
        W058 = "SMY-W058", Warn,  "Two agents' latest commitments to one unit disagree";
    }
    Extension => {
        W409 = "SMY-W409", Warn,  "Unknown enumeration code - preserved verbatim, treated as unknown";
        W434 = "SMY-W434", Warn,  "Bundle carries records this build cannot interpret";
        W432 = "SMY-W432", Warn,  "A payload key named `lang` is the unit core key spelled unquoted";
    }
    Provider => {
        W435 = "SMY-W435", Warn,  "A chunk succeeded on a later attempt; the earlier error is kept";
        W433 = "SMY-W433", Warn,  "Ingest input contains a smysl prompt marker; sent inside a derived fence";
    }
    Lod => {
        W025 = "SMY-W025", Warn,  "Granularity bounds unevaluable - the profile names an estimator this build does not have";
    }
    Provider => {
        W436 = "SMY-W436", Warn,  "A unit is not in the script of the passage it came from (language policy)";
    }
    // --- The library (RFC SMYSL-2.4), in the order `smysl-text` raises them -------------------
    //
    // Registered with the code that raises them, not with the RFC that allocated them. A code
    // nothing can trigger is worse than a missing one, because a reader who greps for it finds
    // a promise with nothing behind it. The rest arrive with their readers, their ingest path
    // and their check pass.
    Library => {
        E401 = "SMY-E401", Error, "Structure hash or rdid does not match the part entry on re-read";
        E402 = "SMY-E402", Error, "Carrying text is refused by the licence recorded in the manifest";
        E403 = "SMY-E403", Error, "A malformed library identity, or provenance the unit's type forbids";
        E410 = "SMY-E410", Error, "Malformed EDTF value in published or a dating";
        W411 = "SMY-W411", Warn,  "The observed instant lies outside the published interval";
        W412 = "SMY-W412", Warn,  "A live dating rule E could not apply: held, outranked, or applying to nothing";
        W413 = "SMY-W413", Warn,  "Temporal inconsistency: a negative cycle or an empty interval under rule E";
        W415 = "SMY-W415", Warn,  "Units excluded from a time-scoped result for lack of a time on the axis asked; counted";
        W418 = "SMY-W418", Warn,  "Expression fork: one alias has two manifest heads";
        E440 = "SMY-E440", Error, "Reader resource cap exceeded; nothing was written";
        E445 = "SMY-E445", Error, "The library or a shard is locked by another writer";
        E446 = "SMY-E446", Error, "An object does not hash to the identity it is stored under";
        W449 = "SMY-W449", Warn,  "An EDTF value outside the instant range; indexed as an open bound";
        E450 = "SMY-E450", Error, "Appending to a pseudonymised expression without its pseudonym key";
        E452 = "SMY-E452", Error, "A record 15 or 18 was offered to a log; text lives in the object store";
        W451 = "SMY-W451", Warn,  "An alignment scheme does not cover a locator; the pair is left unaligned and counted";
        E404 = "SMY-E404", Error, "A unit's span reaches past the length of the part it names";
        W405 = "SMY-W405", Warn,  "A unit's locator and its span name different places; the span wins";
        W419 = "SMY-W419", Warn,  "Ambiguous attribution: the quote occurs more than once in the node named";
        W441 = "SMY-W441", Warn,  "An attributed quote was absent after repair; staged with no span and a capped status";
        W447 = "SMY-W447", Warn,  "A structure node larger than the window was split at sentence boundaries";
        E442 = "SMY-E442", Error, "A model answer references a unit outside its batch and ledger digest";
        W443 = "SMY-W443", Warn,  "Untrusted text contained a prompt fence marker; neutralised in the prompt only";
        W444 = "SMY-W444", Warn,  "A tool-owned field in a model answer was dropped";
        W453 = "SMY-W453", Warn,  "The pivot pass returned no translation for a unit; it is staged in the source language only";
    }

    // --- The query language (RFC SMYSL-2.9, planner SMYSL-2.5 §3.2) ---------------------------
    //
    // Three of these are the lexer's, the parser's and the printer's, raised in the order a
    // parse meets them. The other three arrived with the planner in TX-P6 step 2, on the rule
    // the library block above states — a code nothing can trigger is worse than a missing one,
    // and until a planner existed `E461` and `E462` could not be raised by anything.
    //
    // `E511` is allocated here rather than in SMYSL-2.5's range because that range is full
    // (E460-W479, every code used) and because the refusal is about a *name the grammar
    // accepted*: `from store:ledger` parses, and the span the diagnostic carries is a grammar
    // span. SMYSL-2.0 §6.1 records the allocation against 2.9.
    Query => {
        E427 = "SMY-E427", Error, "Unsupported sq version in a `using` clause";
        E460 = "SMY-E460", Error, "sq syntax error, or a parameter left unbound";
        E461 = "SMY-E461", Error, "A stage is not available in this build or phase";
        E462 = "SMY-E462", Error, "A traversal names a relation kind nothing in scope declares";
        W463 = "SMY-W463", Warn,  "A traversal stopped at its depth with an unexplored frontier";
        W464 = "SMY-W464", Warn,  "A supersession fork in scope: no single head, every successor kept";
        E466 = "SMY-E466", Error, "A query cannot be pinned: the store has no such declaration";
        W417 = "SMY-W417", Warn,  "A dossier subject names more than one entity; the candidates are listed and nothing ran";
        E468 = "SMY-E468", Error, "A question matches no intent rule; name the intent";
        E477 = "SMY-E477", Error, "Unknown dossier template set";
        W478 = "SMY-W478", Warn,  "An answer unit grounds outside its dossier and was dropped";
        W510 = "SMY-W510", Warn,  "A string literal is not in NFC; it is matched byte-exactly as written";
        E511 = "SMY-E511", Error, "A scope names something this environment does not hold";
        E512 = "SMY-E512", Error, "An operator does not apply to the type of the field it names";
    }

    // --- Measures (RFC SMYSL-2.6 §8), in an eleventh group --------------------------------------
    //
    // Two of the ten SMYSL-2.6 allocates, on the library block's rule: registered with the code
    // that raises them. TX-P8 M1 builds the recipe, the payload and the unit writer, and those
    // can refuse a measure nobody registered (`E489`) and decline to persist a value whose
    // scope has nothing to ground it on (`W483`). The refusals that need a measure — lossy
    // texts, a missing class policy, an undated scope, a compressor window, a small sample —
    // arrive with the measures, in M2 to M6. `E486` is withdrawn in the RFC and stays unused.
    // M2 brings three with T1 and T4, the first measures that read a text: `E480` and `W408`
    // for a lossy one, refused or admitted, and `W485` for a sample below the recipe's `min:n`.
    // `W408` is SMYSL-2.0's, filed in the library range and raised only by a measure, so it
    // sits here with the refusal it is the other half of. M3 brings `E484` with T6, the pair of
    // texts too long for the compressor to see both. TX-P9 N1 brings two with the unit
    // measures: `E481` for a class measure (A2, A3) whose recipe names no class policy, and
    // `E482` for a time-ordered one (A3) over a scope whose undated share is above the
    // recipe's `undated:max`. N2 brings `E407` with A8: draft 3's, in the library range like
    // `W408`, and raised only by a measure that budgets a scope in two languages by bytes.
    // N6 brings the two the assessment raises: `W487` for a reader's unit that answers no item
    // of the key, and `W488` for an item whose question or answer no longer stands.
    Measure => {
        E407 = "SMY-E407", Error, "A byte-counting estimator was refused over a scope in more than one language";
        W408 = "SMY-W408", Warn,  "A measure ran over a lossy text because its recipe admits one";
        E480 = "SMY-E480", Error, "A measure refused a lossy text its recipe does not admit";
        E481 = "SMY-E481", Error, "A class-level measure or assessment has no class policy in its recipe";
        E482 = "SMY-E482", Error, "A time-ordered measure was refused: the scope's undated share exceeds the recipe's undated:max";
        W483 = "SMY-W483", Warn,  "A measure was computed but not persisted: its scope has no basis to ground a derived unit";
        E484 = "SMY-E484", Error, "A compression distance was refused: the two texts together exceed the compressor's window";
        W485 = "SMY-W485", Warn,  "A measure was not run: its sample is below the recipe's min:n";
        W487 = "SMY-W487", Warn,  "A response unit was not scored: it answers no item of the key";
        W488 = "SMY-W488", Warn,  "A key item drifted: its question or an answer was withdrawn, retracted, superseded or became unfounded";
        E489 = "SMY-E489", Error, "Recipe incompatible: an unknown measure id or version, or inputs that cannot be compared";
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
#[non_exhaustive]
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
#[non_exhaustive]
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
#[non_exhaustive]
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

    /// The registry is the authority; the manual's Appendix D transcribes it, and the two
    /// are cross-checked at every release cut.
    ///
    /// This used to be phrased as a count of RFC SMYSL-1's Appendix D, "one more than it
    /// lists", with three codes recorded as divergences to reconcile — `SMY-W036` for rule M
    /// lowering a unit at the ingest boundary rather than rejecting it, and `SMY-E307` and
    /// `SMY-W308` for a unit attributing itself to a quote. They were never divergences.
    /// They were the implementation learning something the sketch had not, and the RFC is
    /// retired.
    ///
    /// 51 as of 0.6.0: `SMY-W306` was deleted rather than emitted, having described a usage
    /// threshold that does not exist. 52 as of 1.3.0: `SMY-W309`, a caller's source replacing
    /// the one a unit gave — numbered past `W306`, which stays retired rather than reused. 53 as
    /// of 1.4.0: `SMY-W056`, a withdrawal naming an edge 1.4 does not let anyone withdraw.
    /// 61 as of 1.9.0, the last of them `SMY-W435`: a repair that worked used to erase what it
    /// repaired. 62 with `SMY-W433`: an input carrying a prompt marker is told about rather than
    /// trusted to be innocent. 63 with `SMY-W025`: a granularity bound nobody in this build can
    /// evaluate. 64 as of 1.10.0: `SMY-W436`, a unit written in a different script from its
    /// passage — numbered at the end of the ingest run rather than in the `W30x` block it
    /// belongs to semantically, because the free number there is `W306` and that one is
    /// retired. 69, also 1.10.0, with the first five of RFC SMYSL-2.4's library codes —
    /// `E401`, `E402`, `E440`, `E445`, `E446` — in a ninth group. Five and not the twelve that
    /// RFC allocates: these are the ones `smysl-text` can raise as it stands, and the rest
    /// arrive with the code that raises them. 70, still 1.10.0, with `SMY-E452` — the sixth,
    /// registered in TX-P1 step 4 because that is the step where `Store::append` can raise it:
    /// a log is offered a record 15 or 18 and refuses it (OQ-39). 72 with `SMY-E403` and
    /// `SMY-W418`, which TX-P1 step 5's `Library` check pass raises. 73 with `SMY-E450` in
    /// TX-P2 step 3, where `text append` exists to refuse: an append to a pseudonymised
    /// expression with no pseudonym key. Step 2 built `--pseudonymise` and left the code
    /// unregistered, which is this rule applied rather than an omission. 74 with `SMY-E410` in
    /// TX-P3 step 1, where `@date` and `source { published: … }` give the surface parser two
    /// places to meet a malformed EDTF value and refuse it.
    ///
    /// 76 with `SMY-W412` and `SMY-W413` in TX-P3 step 2, where the time engine computes an
    /// interval and can therefore say that a live dating did not move a bound, or that a set of
    /// records cannot all be true. They are also the two `DetectionKind` values A-8.2 adds, and
    /// `DetectionKind::code()` is total — every kind names the diagnostic it is reported as — so
    /// the kinds and the codes had to arrive together, which is why step 1 shipped neither.
    ///
    /// 78 with `SMY-W411` and `SMY-W449` in TX-P3 step 3, where the `Time` check pass exists
    /// to raise them: an `observed` outside the `published` interval it stands beside, and an
    /// EDTF value outside the millisecond instant range, which is indexed as an open bound
    /// rather than clamped. Both are findings about a *store*, which is why they waited for a
    /// pass — the engine computes intervals and pushes no diagnostics at all.
    ///
    /// 79 with `SMY-W451` in TX-P4 step 4, where `text align` exists: an alignment scheme that
    /// does not cover a locator leaves the pair unaligned, and the count of those is what says
    /// whether a map covers a corpus. Registered now rather than in step 1 for the usual reason
    /// — until there was a command that aligned two expressions, nothing in this build could
    /// find an uncovered locator.
    ///
    /// 81 with `SMY-E404` and `SMY-W405` in TX-P5 step 1, where FC-3 gives a unit a `span` and a
    /// `manifest` and the library pass can therefore compare them against the part: a span past
    /// the part's length, and a locator naming a different place than the span beside it. They
    /// were allocated in TX-P1 step 5 and deliberately left out of the registry, because the
    /// field they read did not exist — this is that promise being kept rather than a late
    /// addition.
    ///
    /// 84 with `SMY-W419`, `SMY-W441` and `SMY-W447` in TX-P5 step 3, where windowing a reading
    /// and attaching a span to a quote exist: a quote that occurs more than once in the node the
    /// model named, a quote the window does not support after repair, and a structure node too
    /// large for one window. All three are about a *place* in a text, which is why none of them
    /// could be raised before there was code that chose one — `W419` has been allocated since
    /// draft 3 and had nothing behind it until now.
    ///
    /// 87 with `SMY-E442`, `SMY-W443` and `SMY-W444` in TX-P5 step 4, where a text ingest has a
    /// ledger digest to scope a reference against, a second fence to neutralise a marker inside,
    /// and an answer whose tool-owned fields are dropped rather than trusted. All three need a
    /// prompt that sends a digest and a converter that reads one, which is what step 4 builds.
    ///
    /// 88 with `SMY-W453` in TX-P5's pivot policy, the first of the range's seven free numbers
    /// to be spent: `pivot:<lang>` asks a model for one line per unit, and a line that does not
    /// come back is a unit with no translation. Reported rather than inferred from a count,
    /// because the alternative — a staged unit that silently stayed in the source language — is
    /// the policy failure F-6 exists to report. A translation that comes back in the *wrong*
    /// language reuses `SMY-W436` instead of taking a number of its own: it is the same defect
    /// that code already names, measured against the language asked for rather than the
    /// passage's.
    ///
    /// 91 with `SMY-E427`, `SMY-E460` and `SMY-W510` in TX-P6 step 1, where a lexer, a parser
    /// and a canonical printer for `sq/1` exist (RFC SMYSL-2.9). They are the three a parse can
    /// produce: a `using` clause naming a version this build does not support, a syntax error or
    /// an unbound `$parameter`, and a string literal that is not in NFC. The last is a *warning*
    /// and not an error on purpose — SMYSL-2.9's QG-5 keeps a literal's bytes exactly as written,
    /// because a quote must match a corpus byte for byte, and the hazard that creates is a
    /// non-NFC quote matching nothing at all while looking correct on screen. Silence there is
    /// the worst available outcome, so the parse succeeds and says so.
    ///
    /// 95 with `SMY-E461`, `SMY-E462`, `SMY-E466` and `SMY-E511` in TX-P6 step 2, where the
    /// planner exists. The first three were allocated by RFC SMYSL-2.5 and withheld until
    /// something could raise them: a stage this build does not lower, a traversal naming a
    /// relation kind nothing in scope declares, and a `using` pin for a declaration the store
    /// does not hold. `E511` is new, and it is in SMYSL-2.9's range rather than SMYSL-2.5's
    /// because that range is full and because the refusal is about a name the *grammar*
    /// accepted — `from store:ledger` parses, and the span is a grammar span.
    ///
    /// 98 with `SMY-W464` in TX-P6 step 5, where the supersession pass exists. Allocated by
    /// SMYSL-2.5 and withheld on the same rule as the rest: a fork is two units superseding
    /// one target with nothing ordering them, and until a pass chose heads there was nothing
    /// to decline to choose.
    ///
    /// 99 with `SMY-W415` in TX-P6 step 6, where time stages exist. It is the oldest withheld
    /// code in the registry — draft 3 allocated it in the `E401`–`W431` block and SMYSL-2.6
    /// §4's A3 has cited it since — and it was withheld for the usual reason and one more. The
    /// usual one: until a query could say `said during 1769`, nothing excluded a unit for
    /// having no time. The extra one is that the exclusion is **not** a filter failing. A unit
    /// with no date on the axis asked did not fail the test; it could not take it, and A-2.4
    /// rule 5 says an undated interval is *not early and not late* rather than outside every
    /// range. So the count is reported beside the answer, like `W463`, and the two are the only
    /// warnings this crate's query path raises about a run that succeeded.
    ///
    /// 97 with `SMY-W463` and `SMY-E512` in TX-P6 step 4, where traversals and filters exist.
    /// `W463` was allocated by SMYSL-2.5 and withheld on the same rule as `E461`: a traversal
    /// that stopped at its depth with the graph still going is worth saying, and nothing could
    /// say it until something walked a graph with a depth.
    ///
    /// `SMY-E512` is new: an operator that does not apply
    /// to the type of the field it names, such as `>` on `kind`. Also SMYSL-2.9's range, for
    /// SMYSL-2.5's reason and one of its own — the grammar admits `<` after any field, because
    /// a grammar cannot know a field's type, so this refusal belongs to the document that
    /// defines what the grammar accepted. It is **not** `E421`/`E422`: those are about a field
    /// nothing declares and a key retired without a mapping, and this field is declared and
    /// readable; what is wrong is the question asked of it. That leaves `W513`–`W519`
    /// unallocated.
    ///
    /// `SMY-E421` and `SMY-E422` stay out for the same reason the four above came in. Field
    /// resolution needs declared field types, and a declaration's `payload_shape` is opaque
    /// CBOR until FC-9 decoding lands in TX-P12 step 1; the planner records that the check did
    /// not run rather than pretending it passed.
    ///
    /// That leaves `454`–`459` of RFC SMYSL-2.4's `E401`–`W459` range unallocated.
    ///
    /// Step 5's plan named six codes and two of them are **not** here. `SMY-E404` is a span
    /// past a part's length and `SMY-W405` is a locator disagreeing with a span; both read
    /// `SourceRef.span`, which SMYSL-2.4 §4.3.1 gives to TX-P5. Registering them with the pass
    /// that will one day raise them would put two codes in the registry that nothing in this
    /// build can produce, which is the thing this comment exists to prevent.
    /// 103 with `SMY-W417`, `SMY-E468`, `SMY-E477` and `SMY-W478` in TX-P8 step 3, where `ask`
    /// exists. Three are SMYSL-2.5's and one is draft 3's, and each was withheld until something
    /// could raise it: a subject that names two entities, a question no intent rule matches, a
    /// template set this build does not carry, and a model answer grounding outside the
    /// dossier it was given. `W417` sits in the query block although its number is in the
    /// library range, because the ambiguity is the dossier's: the library resolves a name, and
    /// only a question needs that name to be one thing.
    ///
    /// 105 with `SMY-W483` and `SMY-E489` in TX-P8 M1 (SMYSL-2.6), in an eleventh group,
    /// `Measure`: the two of the RFC's ten that a recipe, a payload writer and a unit writer can
    /// raise before any measure exists. The other seven are refusals a measure makes, and wait
    /// for the measures.
    ///
    /// 108 with `SMY-W408`, `SMY-E480` and `SMY-W485` in TX-P8 M2, where T1 and T4 read a text
    /// and can refuse a lossy one, admit it under a recipe that says so, or find it too short.
    ///
    /// 109 with `SMY-E484` in TX-P8 M3, where T6 refuses a pair of texts longer than the
    /// compressor's window.
    ///
    /// 111 with `SMY-E481` and `SMY-E482` in TX-P9 N1, where A2 and A3 count proposition
    /// classes and A3 orders a scope by effective time.
    ///
    /// 112 with `SMY-E407` in TX-P9 N2, where A8 budgets a scope with `smysl/utf8-div4` and
    /// refuses one whose units are drawn from text in more than one language.
    ///
    /// 114 with `SMY-W487` and `SMY-W488` in TX-P9 N6, where a score skips a reader's unit
    /// that answers no item and leaves out an item that drifted after its key was made.
    #[test]
    fn the_registry_is_the_size_it_is_meant_to_be() {
        assert_eq!(Code::ALL.len(), 114);
    }

    #[test]
    fn group_membership_is_complete() {
        for &g in Group::ALL {
            assert!(
                Code::ALL.iter().any(|c| c.group() == g),
                "group {g} has no codes"
            );
        }
        assert_eq!(Group::ALL.len(), 11);
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
