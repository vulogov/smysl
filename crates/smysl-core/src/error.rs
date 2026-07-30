//! The error vocabulary (§25) and the exit-code mapping (Appendix E).
//!
//! Every error type in the workspace lives here, including the payload types that other
//! crates re-export under their own names (`smysl_pack::PackError`,
//! `smysl_provider::ProviderError`, and so on). They are pure data with no dependency on
//! HTTP, graph, or render internals, so hosting them in the kernel keeps the unified
//! [`Error`] free of dependency cycles while leaving the public paths of §12.2 intact.
//!
//! Rule A4: errors are typed and `#[non_exhaustive]`. There are no stringly-typed
//! failures - every variant that corresponds to a diagnostic reports its [`Code`].

use core::fmt;
use std::time::Duration;

use crate::diag::{Code, Report, Span};
use crate::ids::Uid;

// ---------------------------------------------------------------------------
// Exit codes (Appendix E)
// ---------------------------------------------------------------------------

/// Process exit codes. Part of the contract: they MUST remain stable across minor
/// versions so pipelines can branch on them (§23).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ExitCode {
    Success = 0,
    Failure = 1,
    Usage = 2,
    CheckErrors = 3,
    PackInfeasible = 4,
    Contentions = 5,
    Provider = 6,
    Offline = 7,
    UnsupportedVersion = 8,
    HashVerification = 9,
    Staged = 10,
    /// Staged or accepted, **and rule M lowered at least one unit** (`SMY-W036`).
    ///
    /// A refinement of `Staged`, not a failure: the batch is intact and every corrected
    /// unit is in it, at the status its grounds actually support. What it adds is that the
    /// model claimed more than it could support and was corrected, which a pipeline may
    /// reasonably want to route differently - to a reviewer, to a re-prompt, to a log -
    /// without parsing `--json` to find out.
    ///
    /// It supersedes both `Staged` and `Success`: `ingest --yes` used to return `0` on a
    /// corrected batch, so the one outcome most worth knowing about was the one indistinguish-
    /// able from nothing having happened.
    ///
    /// A script testing `= 10` for "staged" should test `>= 10`.
    StagedWithCorrections = 11,
}

impl ExitCode {
    pub const ALL: &'static [ExitCode] = &[
        ExitCode::Success,
        ExitCode::Failure,
        ExitCode::Usage,
        ExitCode::CheckErrors,
        ExitCode::PackInfeasible,
        ExitCode::Contentions,
        ExitCode::Provider,
        ExitCode::Offline,
        ExitCode::UnsupportedVersion,
        ExitCode::HashVerification,
        ExitCode::Staged,
        ExitCode::StagedWithCorrections,
    ];

    pub const fn as_i32(self) -> i32 {
        self as i32
    }

    pub const fn describe(self) -> &'static str {
        match self {
            ExitCode::Success => "success",
            ExitCode::Failure => "generic failure",
            ExitCode::Usage => "usage error",
            ExitCode::CheckErrors => "check reported errors",
            ExitCode::PackInfeasible => "pack infeasible",
            ExitCode::Contentions => "contentions with --fail-on-contention",
            ExitCode::Provider => "provider error",
            ExitCode::Offline => "offline violation",
            ExitCode::UnsupportedVersion => "unsupported format or kernel major",
            ExitCode::HashVerification => "hash verification failure",
            ExitCode::Staged => "output staged; confirmation required",
            ExitCode::StagedWithCorrections => "output staged; rule M lowered a unit",
        }
    }
}

impl fmt::Display for ExitCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.as_i32(), self.describe())
    }
}

// ---------------------------------------------------------------------------
// Identifier errors
// ---------------------------------------------------------------------------

/// Malformed identifier text. Every id newtype validates on construction and is
/// infallible thereafter, which is what lets the encoder be infallible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdError {
    pub kind: &'static str,
    pub found: String,
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "malformed {}: `{}`", self.kind, self.found)
    }
}

impl std::error::Error for IdError {}

// ---------------------------------------------------------------------------
// Shape errors - the validating constructor of §15.1
// ---------------------------------------------------------------------------

/// Local, store-free validation performed by `UnitCore::new`. Rule M is deliberately not
/// here: shape is local, monotonicity is global.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShapeError {
    /// `SMY-E021`
    MissingGist,
    /// `SMY-E022`
    GistTooLong { max: usize, actual: usize },
    /// `SMY-E023`
    DetailWithoutBody,
    /// `SMY-E031` - `derived`/`inferred` with empty grounds.
    GroundsRequired,
    /// `SMY-E032` - `measured`/`cited` without source.
    SourceRequired,
    /// `SMY-E034` - `unfounded` is reachable only by retraction.
    UnfoundedAuthored,
}

impl ShapeError {
    pub const fn code(&self) -> Code {
        match self {
            ShapeError::MissingGist => Code::E021,
            ShapeError::GistTooLong { .. } => Code::E022,
            ShapeError::DetailWithoutBody => Code::E023,
            ShapeError::GroundsRequired => Code::E031,
            ShapeError::SourceRequired => Code::E032,
            ShapeError::UnfoundedAuthored => Code::E034,
        }
    }
}

impl fmt::Display for ShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShapeError::GistTooLong { max, actual } => {
                write!(
                    f,
                    "{}: gist is {actual} tokens, limit is {max}",
                    self.code()
                )
            }
            other => write!(f, "{}: {}", other.code(), other.code().message()),
        }
    }
}

impl std::error::Error for ShapeError {}

// ---------------------------------------------------------------------------
// Parse
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// `SMY-E001` - only ever returned for an unparseable `@doc` header; malformed
    /// records become diagnostics so the repair loop can target a span (§15.3).
    Syntax { span: Span, message: String },
    /// `SMY-E002`
    UnsupportedKernelMajor { found: String },
    /// `SMY-E003`
    UnsupportedFormatVersion { found: String },
}

impl ParseError {
    pub const fn code(&self) -> Code {
        match self {
            ParseError::Syntax { .. } => Code::E001,
            ParseError::UnsupportedKernelMajor { .. } => Code::E002,
            ParseError::UnsupportedFormatVersion { .. } => Code::E003,
        }
    }
}

impl ParseError {
    /// The exit code this error reports (Appendix E). A version mismatch is 8; anything
    /// else a parse can hard-fail on is a check error.
    pub const fn into_exit_code(&self) -> ExitCode {
        match self {
            ParseError::UnsupportedKernelMajor { .. }
            | ParseError::UnsupportedFormatVersion { .. } => ExitCode::UnsupportedVersion,
            ParseError::Syntax { .. } => ExitCode::CheckErrors,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Syntax { span, message } => {
                write!(f, "{}: {message} (at {span})", self.code())
            }
            ParseError::UnsupportedKernelMajor { found } => {
                write!(f, "{}: kernel schema {found}", self.code())
            }
            ParseError::UnsupportedFormatVersion { found } => {
                write!(f, "{}: format version {found}", self.code())
            }
        }
    }
}

impl std::error::Error for ParseError {}

// ---------------------------------------------------------------------------
// Codec
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CodecError {
    /// `SMY-E004`
    MalformedEnvelope { at: usize },
    /// `SMY-E080` - the reader rejects rather than normalises, so one uid always
    /// corresponds to exactly one byte sequence (§7.1).
    NonDeterministic { at: usize, reason: NonDetReason },
    /// `SMY-E081`
    Float { at: usize },
    /// `SMY-E071` - a byte string where a uid belongs is not 32 bytes wide. A display
    /// abbreviation in a canonical record is not a shorter identity.
    TruncatedUid { at: usize, len: usize },
    /// Input ended mid-item.
    Truncated { at: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NonDetReason {
    NonShortestInt,
    IndefiniteLength,
    UnsortedMapKeys,
    DuplicateMapKey,
    NullOptional,
    NonNfcText,
    UnsortedSet,
}

impl NonDetReason {
    pub const fn describe(self) -> &'static str {
        match self {
            NonDetReason::NonShortestInt => "integer not in shortest form",
            NonDetReason::IndefiniteLength => "indefinite-length item",
            NonDetReason::UnsortedMapKeys => "map keys not strictly ascending",
            NonDetReason::DuplicateMapKey => "duplicate map key",
            NonDetReason::NullOptional => "absent optional encoded as null",
            NonDetReason::NonNfcText => "text is not NFC-normalised",
            NonDetReason::UnsortedSet => "set elements not sorted by encoded bytes",
        }
    }
}

impl CodecError {
    pub const fn code(&self) -> Code {
        match self {
            CodecError::MalformedEnvelope { .. } | CodecError::Truncated { .. } => Code::E004,
            CodecError::NonDeterministic { .. } => Code::E080,
            CodecError::Float { .. } => Code::E081,
            CodecError::TruncatedUid { .. } => Code::E071,
        }
    }
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodecError::MalformedEnvelope { at } => {
                write!(f, "{}: malformed envelope at byte {at}", self.code())
            }
            CodecError::NonDeterministic { at, reason } => {
                write!(f, "{}: {} at byte {at}", self.code(), reason.describe())
            }
            CodecError::Float { at } => write!(
                f,
                "{}: float is not binary32 quantised to 1/1024, at byte {at}",
                self.code()
            ),
            CodecError::Truncated { at } => {
                write!(f, "{}: input ends mid-item at byte {at}", self.code())
            }
            CodecError::TruncatedUid { at, len } => write!(
                f,
                "{}: {len}-byte uid at byte {at}, expected 32",
                self.code()
            ),
        }
    }
}

impl std::error::Error for CodecError {}

// ---------------------------------------------------------------------------
// Integrity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IntegrityError {
    /// `SMY-E060`
    Dangling { from: Uid, to: Uid },
    /// `SMY-E061`
    DepCycle { members: Vec<Uid> },
    /// `SMY-E070`
    HashMismatch { stored: Uid, recomputed: Uid },
    /// `SMY-E071`
    TruncatedUid { found: String },
    /// `SMY-E072`
    AmbiguousPrefix {
        prefix: String,
        candidates: Vec<Uid>,
    },
}

impl IntegrityError {
    pub const fn code(&self) -> Code {
        match self {
            IntegrityError::Dangling { .. } => Code::E060,
            IntegrityError::DepCycle { .. } => Code::E061,
            IntegrityError::HashMismatch { .. } => Code::E070,
            IntegrityError::TruncatedUid { .. } => Code::E071,
            IntegrityError::AmbiguousPrefix { .. } => Code::E072,
        }
    }
}

impl fmt::Display for IntegrityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntegrityError::Dangling { from, to } => {
                write!(f, "{}: {from} references absent {to}", self.code())
            }
            IntegrityError::DepCycle { members } => {
                write!(f, "{}: cycle over {} units", self.code(), members.len())
            }
            IntegrityError::HashMismatch { stored, recomputed } => write!(
                f,
                "{}: stored {stored}, recomputed {recomputed}",
                self.code()
            ),
            IntegrityError::TruncatedUid { found } => {
                write!(f, "{}: {found}", self.code())
            }
            IntegrityError::AmbiguousPrefix { prefix, candidates } => write!(
                f,
                "{}: {prefix} matches {} units",
                self.code(),
                candidates.len()
            ),
        }
    }
}

impl std::error::Error for IntegrityError {}

// ---------------------------------------------------------------------------
// Pack
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PackError {
    /// `SMY-E200` - the mandatory floor (C3/C4/C5) does not fit. Carries the minimum
    /// feasible budget so a caller can retry programmatically (§18.4).
    Infeasible { budget: u64, required: u64 },
    /// `SMY-E201`
    FocusAbsent { uid: Uid },
}

impl PackError {
    pub const fn code(&self) -> Code {
        match self {
            PackError::Infeasible { .. } => Code::E200,
            PackError::FocusAbsent { .. } => Code::E201,
        }
    }
}

impl PackError {
    /// The exit code this error reports (Appendix E). An infeasible floor is 4; a missing
    /// focus unit is an ordinary failure.
    pub const fn code_exit(&self) -> ExitCode {
        match self {
            PackError::Infeasible { .. } => ExitCode::PackInfeasible,
            PackError::FocusAbsent { .. } => ExitCode::Failure,
        }
    }
}

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackError::Infeasible { budget, required } => write!(
                f,
                "{}: budget {budget} but the mandatory floor needs {required}",
                self.code()
            ),
            PackError::FocusAbsent { uid } => write!(f, "{}: {uid}", self.code()),
        }
    }
}

impl std::error::Error for PackError {}

// ---------------------------------------------------------------------------
// Merge
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MergeError {
    /// `SMY-E050`
    OrphanedGrounds { uid: Uid },
    /// `SMY-E051`
    AuthorityUnsatisfied { uid: Uid, required: String },
    /// Contentions were detected and the caller asked to fail on them. Not a diagnostic
    /// code: contentions are a normal outcome (rule C), this is only the opt-in gate.
    ContentionsPresent { count: usize },
}

impl MergeError {
    pub const fn code(&self) -> Option<Code> {
        match self {
            MergeError::OrphanedGrounds { .. } => Some(Code::E050),
            MergeError::AuthorityUnsatisfied { .. } => Some(Code::E051),
            MergeError::ContentionsPresent { .. } => None,
        }
    }
}

impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MergeError::OrphanedGrounds { uid } => {
                write!(f, "{}: {uid} has no surviving grounds", Code::E050)
            }
            MergeError::AuthorityUnsatisfied { uid, required } => {
                write!(f, "{}: {uid} requires {required}", Code::E051)
            }
            MergeError::ContentionsPresent { count } => {
                write!(f, "{count} open contentions")
            }
        }
    }
}

impl std::error::Error for MergeError {}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RenderError {
    /// `SMY-E210` - raised at profile *load*, so a misconfigured profile cannot produce a
    /// flattened artifact at all (§20).
    ProfileFlattensStatus { profile: String, status: String },
    /// The requested target is not compiled in.
    UnsupportedTarget { target: String },
    /// A backend failed to emit.
    Backend { target: String, message: String },
}

impl RenderError {
    pub const fn code(&self) -> Option<Code> {
        match self {
            RenderError::ProfileFlattensStatus { .. } => Some(Code::E210),
            _ => None,
        }
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::ProfileFlattensStatus { profile, status } => write!(
                f,
                "{}: profile {profile} has no distinct rendering for {status}",
                Code::E210
            ),
            RenderError::UnsupportedTarget { target } => {
                write!(f, "render target {target} is not available in this build")
            }
            RenderError::Backend { target, message } => {
                write!(f, "{target} backend failed: {message}")
            }
        }
    }
}

impl std::error::Error for RenderError {}

// ---------------------------------------------------------------------------
// Provider (§21.4)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProviderError {
    Unreachable,
    Unauthorized,
    RateLimited { retry_after: Option<Duration> },
    ContextExceeded { limit: usize, requested: usize },
    StructuredUnsupported,
    OfflineViolation,
    Malformed(String),
    Upstream(u16, String),
}

impl ProviderError {
    pub const fn code(&self) -> Option<Code> {
        match self {
            ProviderError::Unreachable => Some(Code::E300),
            ProviderError::OfflineViolation => Some(Code::E301),
            ProviderError::ContextExceeded { .. } => Some(Code::E302),
            ProviderError::StructuredUnsupported => Some(Code::W303),
            _ => None,
        }
    }

    /// Fallback fires only on `Unreachable` - never on a configuration error, which
    /// would otherwise be hidden behind a different model (§21.3).
    pub const fn is_fallback_eligible(&self) -> bool {
        matches!(self, ProviderError::Unreachable)
    }

    /// Appendix E: an offline violation is 7 and everything else a provider can do is 6.
    ///
    /// The distinction is the one a pipeline branches on: exit 7 means "you asked me not
    /// to send this and I did not", which is a policy outcome rather than a failure.
    pub const fn exit_code(&self) -> ExitCode {
        match self {
            ProviderError::OfflineViolation => ExitCode::Offline,
            _ => ExitCode::Provider,
        }
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderError::Unreachable => f.write_str("provider unreachable"),
            ProviderError::Unauthorized => f.write_str("provider rejected the credentials"),
            ProviderError::RateLimited { retry_after } => match retry_after {
                Some(d) => write!(f, "rate limited, retry after {}s", d.as_secs()),
                None => f.write_str("rate limited"),
            },
            ProviderError::ContextExceeded { limit, requested } => {
                write!(f, "context window exceeded: {requested} > {limit}")
            }
            ProviderError::StructuredUnsupported => {
                f.write_str("provider does not support structured output")
            }
            ProviderError::OfflineViolation => {
                f.write_str("operation would leave the machine while --offline is set")
            }
            ProviderError::Malformed(m) => write!(f, "malformed provider response: {m}"),
            ProviderError::Upstream(status, m) => write!(f, "upstream {status}: {m}"),
        }
    }
}

impl std::error::Error for ProviderError {}

// ---------------------------------------------------------------------------
// The unified error
// ---------------------------------------------------------------------------

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Id(IdError),
    Parse(ParseError),
    Codec(CodecError),
    Shape(ShapeError),
    Integrity(IntegrityError),
    Check(Report),
    Pack(PackError),
    Merge(MergeError),
    Render(RenderError),
    Provider(ProviderError),
    Io(std::io::Error),
}

impl Error {
    /// The exit code the CLI reports for this error (Appendix E).
    pub fn exit_code(&self) -> ExitCode {
        match self {
            Error::Parse(ParseError::UnsupportedKernelMajor { .. })
            | Error::Parse(ParseError::UnsupportedFormatVersion { .. }) => {
                ExitCode::UnsupportedVersion
            }
            Error::Check(_) => ExitCode::CheckErrors,
            Error::Integrity(IntegrityError::HashMismatch { .. })
            | Error::Integrity(IntegrityError::TruncatedUid { .. }) => ExitCode::HashVerification,
            Error::Pack(PackError::Infeasible { .. }) => ExitCode::PackInfeasible,
            Error::Merge(MergeError::ContentionsPresent { .. }) => ExitCode::Contentions,
            Error::Provider(ProviderError::OfflineViolation) => ExitCode::Offline,
            Error::Provider(_) => ExitCode::Provider,
            _ => ExitCode::Failure,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Id(e) => write!(f, "{e}"),
            Error::Parse(e) => write!(f, "{e}"),
            Error::Codec(e) => write!(f, "{e}"),
            Error::Shape(e) => write!(f, "{e}"),
            Error::Integrity(e) => write!(f, "{e}"),
            Error::Check(r) => write!(
                f,
                "{} error(s), {} warning(s)",
                r.of_severity(crate::diag::Severity::Error).count(),
                r.of_severity(crate::diag::Severity::Warn).count()
            ),
            Error::Pack(e) => write!(f, "{e}"),
            Error::Merge(e) => write!(f, "{e}"),
            Error::Render(e) => write!(f, "{e}"),
            Error::Provider(e) => write!(f, "{e}"),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Id(e) => Some(e),
            Error::Parse(e) => Some(e),
            Error::Codec(e) => Some(e),
            Error::Shape(e) => Some(e),
            Error::Integrity(e) => Some(e),
            Error::Pack(e) => Some(e),
            Error::Merge(e) => Some(e),
            Error::Render(e) => Some(e),
            Error::Provider(e) => Some(e),
            Error::Io(e) => Some(e),
            Error::Check(_) => None,
        }
    }
}

macro_rules! from_impl {
    ($($ty:ident => $variant:ident),* $(,)?) => {
        $(impl From<$ty> for Error {
            fn from(e: $ty) -> Error { Error::$variant(e) }
        })*
    };
}

from_impl! {
    IdError => Id,
    ParseError => Parse,
    CodecError => Codec,
    ShapeError => Shape,
    IntegrityError => Integrity,
    PackError => Pack,
    MergeError => Merge,
    RenderError => Render,
    ProviderError => Provider,
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::Io(e)
    }
}

impl From<Report> for Error {
    fn from(r: Report) -> Error {
        Error::Check(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Diagnostic;

    #[test]
    fn exit_codes_match_appendix_e_plus_the_0_2_addition() {
        let pairs = [
            (ExitCode::Success, 0),
            (ExitCode::Failure, 1),
            (ExitCode::Usage, 2),
            (ExitCode::CheckErrors, 3),
            (ExitCode::PackInfeasible, 4),
            (ExitCode::Contentions, 5),
            (ExitCode::Provider, 6),
            (ExitCode::Offline, 7),
            (ExitCode::UnsupportedVersion, 8),
            (ExitCode::HashVerification, 9),
            (ExitCode::Staged, 10),
            // 11 is new in 0.2.0 and is *not* in Appendix E. An exit code is a permanent
            // contract, so a new one is a divergence worth naming rather than absorbing:
            // `ingest` knew rule M had corrected the model and had no way to say so, and
            // under `--yes` returned plain success - the one outcome most worth knowing
            // about, reported as though nothing had happened.
            (ExitCode::StagedWithCorrections, 11),
        ];
        for (code, n) in pairs {
            assert_eq!(code.as_i32(), n, "{code:?} must stay at {n}");
        }
        assert_eq!(ExitCode::ALL.len(), 12);
    }

    #[test]
    fn exit_codes_are_contiguous_and_unique() {
        let nums: Vec<i32> = ExitCode::ALL.iter().map(|c| c.as_i32()).collect();
        assert_eq!(nums, (0..=11).collect::<Vec<_>>());
    }

    #[test]
    fn check_failure_exits_three() {
        let mut r = Report::new();
        r.push(Diagnostic::new(Code::E030));
        assert_eq!(Error::from(r).exit_code(), ExitCode::CheckErrors);
    }

    #[test]
    fn infeasible_pack_exits_four_and_reports_the_minimum_budget() {
        let e = PackError::Infeasible {
            budget: 100,
            required: 260,
        };
        assert_eq!(e.code(), Code::E200);
        assert_eq!(Error::from(e.clone()).exit_code(), ExitCode::PackInfeasible);
        assert!(e.to_string().contains("260"));
    }

    #[test]
    fn offline_violation_exits_seven_not_six() {
        assert_eq!(
            Error::from(ProviderError::OfflineViolation).exit_code(),
            ExitCode::Offline
        );
        assert_eq!(
            Error::from(ProviderError::Unreachable).exit_code(),
            ExitCode::Provider
        );
    }

    #[test]
    fn version_errors_exit_eight() {
        assert_eq!(
            Error::from(ParseError::UnsupportedKernelMajor {
                found: "smysl.kernel/9".into()
            })
            .exit_code(),
            ExitCode::UnsupportedVersion
        );
        assert_eq!(
            Error::from(ParseError::UnsupportedFormatVersion {
                found: "smysl/9.0".into()
            })
            .exit_code(),
            ExitCode::UnsupportedVersion
        );
    }

    #[test]
    fn hash_failures_exit_nine() {
        let e = IntegrityError::HashMismatch {
            stored: Uid::from_bytes([1; 32]),
            recomputed: Uid::from_bytes([2; 32]),
        };
        assert_eq!(e.code(), Code::E070);
        assert_eq!(Error::from(e).exit_code(), ExitCode::HashVerification);
    }

    #[test]
    fn contentions_exit_five() {
        assert_eq!(
            Error::from(MergeError::ContentionsPresent { count: 3 }).exit_code(),
            ExitCode::Contentions
        );
    }

    #[test]
    fn fallback_fires_only_on_unreachable() {
        assert!(ProviderError::Unreachable.is_fallback_eligible());
        for e in [
            ProviderError::Unauthorized,
            ProviderError::ContextExceeded {
                limit: 1,
                requested: 2,
            },
            ProviderError::Malformed("x".into()),
            ProviderError::OfflineViolation,
            ProviderError::StructuredUnsupported,
        ] {
            assert!(
                !e.is_fallback_eligible(),
                "{e} must not trigger fallback - it would hide a configuration error"
            );
        }
    }

    #[test]
    fn shape_errors_map_to_their_appendix_d_codes() {
        assert_eq!(ShapeError::MissingGist.code(), Code::E021);
        assert_eq!(
            ShapeError::GistTooLong {
                max: 30,
                actual: 44
            }
            .code(),
            Code::E022
        );
        assert_eq!(ShapeError::DetailWithoutBody.code(), Code::E023);
        assert_eq!(ShapeError::GroundsRequired.code(), Code::E031);
        assert_eq!(ShapeError::SourceRequired.code(), Code::E032);
        assert_eq!(ShapeError::UnfoundedAuthored.code(), Code::E034);
    }

    #[test]
    fn codec_reasons_map_to_e080_and_e081() {
        let e = CodecError::NonDeterministic {
            at: 12,
            reason: NonDetReason::IndefiniteLength,
        };
        assert_eq!(e.code(), Code::E080);
        assert!(e.to_string().contains("indefinite"));
        assert_eq!(CodecError::Float { at: 3 }.code(), Code::E081);
        assert_eq!(CodecError::MalformedEnvelope { at: 0 }.code(), Code::E004);
    }

    #[test]
    fn errors_expose_a_source_chain() {
        use std::error::Error as _;
        let e = Error::from(ProviderError::Unreachable);
        assert!(e.source().is_some());
        assert!(Error::Check(Report::new()).source().is_none());
    }
}
