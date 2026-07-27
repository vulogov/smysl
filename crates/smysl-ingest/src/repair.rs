//! The repair loop (§22.3) - rule I.
//!
//! ```text
//! for attempt in 0..=repair_attempts:            # default 2
//!     out   = provider.complete(req)
//!     parse = parse_surface(&out)                # never hard-fails (§15.3)
//!     if parse.diagnostics.is_empty():
//!         chk = check_local(&parse.records)      # shape, rule L, rule T; not rule M
//!         if chk.is_clean() { return Ok(records) }
//!         diags = chk
//!     else { diags = parse.diagnostics }
//!     req = req.with_repair(span_of(diags), render_diagnostics(diags))
//! # exhausted:
//! emit prose unit { … , payload: { "ingest:unrepaired": true } }    # W304
//! ```
//!
//! **`ingest` MUST always make progress.** An unrepairable span degrades to an opaque
//! `prose` unit rather than failing the run: a corpus with some opaque units is usable, and
//! a failed ingest is not. That is the whole of rule I, and it is why every path out of this
//! module produces units.
//!
//! Rule M is *not* checked here. It is checked at staging against the store, because
//! grounds may reference units the chunk did not contain - a claim resting on something
//! ingested an hour ago is not a rule M violation, it is the normal case.
//!
//! Rule T *is* checked here, but its diagnostic does not buy a turn: the ceiling is applied
//! unconditionally in [`convert`], so `SMY-E033` records what a model tried rather than work
//! outstanding. See [`needs_repair`].

use smysl_check::{check, CheckOptions, Pass};
use smysl_core::surface::parse_surface;
use smysl_core::{
    Code, Diagnostic, KernelType, Record, Report, Rung, Severity, Status, UnitCore, UnitCoreBuilder,
};
use smysl_graph::Store;

use crate::ceiling;
use crate::json_ast;
use crate::IngestPath;

/// The payload marker an unrepairable span carries (§22.3).
pub const UNREPAIRED_KEY: &str = "ingest:unrepaired";

/// What one span's conversion produced.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Attempted {
    pub units: Vec<UnitCore>,
    pub diagnostics: Vec<Diagnostic>,
    /// How many attempts were spent. Zero means the first answer was already clean.
    pub attempts: u32,
    /// True when the span degraded to opaque prose (`SMY-W304`).
    pub degraded: bool,
}

impl Attempted {
    pub fn is_clean(&self) -> bool {
        !self.degraded
            && !self
                .diagnostics
                .iter()
                .any(|d| d.severity == Severity::Error)
    }
}

/// Convert one model answer into units, checking what can be checked locally.
///
/// Returns the units and any diagnostics. The caller decides whether to spend a repair
/// attempt; this function has no opinion about retries, which keeps it testable without a
/// provider.
pub fn convert(answer: &str, path: IngestPath, rung: Rung) -> (Vec<UnitCore>, Vec<Diagnostic>) {
    let (mut units, mut diagnostics) = match path {
        IngestPath::JsonAst => {
            let out = json_ast::convert(answer);
            (out.units, out.diagnostics)
        }
        IngestPath::Surface => match parse_surface(answer) {
            Ok(out) => (
                out.units().cloned().collect::<Vec<_>>(),
                out.diagnostics.clone(),
            ),
            // §15.3: parsing never hard-fails, but a caller of `parse_surface` can still
            // get an error for a malformed document header. That is a diagnostic here, not
            // a panic and not a lost span.
            Err(e) => (
                Vec::new(),
                vec![Diagnostic::new(Code::E001).with_message(e.to_string())],
            ),
        },
    };

    // Rule T, applied unconditionally after parse (§22.4). A model claiming `measured` is
    // downgraded and told, whatever else was wrong with the answer.
    let mut capped = Vec::with_capacity(units.len());
    for u in units.drain(..) {
        let applied = ceiling::apply(&u, rung, None);
        diagnostics.extend(applied.diagnostics);
        capped.push(applied.core);
    }

    (capped, diagnostics)
}

/// Whether a set of diagnostics is worth spending a repair attempt on.
///
/// Only errors are. A warning means the answer is usable and something is worth saying;
/// re-asking the model would spend a call to remove a remark.
///
/// **`SMY-E033` is the exception**, because rule T already fixed it. The ceiling is applied
/// unconditionally in [`convert`] and the diagnostic records that a model tried, not that
/// something is outstanding - so retrying asks the model to solve a problem that no longer
/// exists, and a model confident enough to claim `measured` once claims it again. Observed
/// live: three Gemini attempts, three identical `measured` claims, and a chunk of good
/// capped units discarded to opaque prose by rule I on the way out.
///
/// If the cap were ever to fail, the units would reach `stage::prepare` still over the
/// ceiling and its check would refuse them there. This shortens the loop; it does not widen
/// the gate.
pub fn needs_repair(diagnostics: &[Diagnostic]) -> bool {
    diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error && d.code != Code::E033)
}

/// Render diagnostics for the repair turn.
///
/// Verbatim: they already name the code, the span, and the rule, and a paraphrase would be
/// a second wording to keep in step with the first.
pub fn render_diagnostics(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| format!("- {d}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The unrepairable span, as an opaque `prose` unit (rule I, `SMY-W304`).
///
/// The raw span becomes the body verbatim, so nothing is lost: a later pass, a human, or a
/// better model can come back to it. The gist is synthesised from the first sentence,
/// because a unit needs one and inventing a summary would be asserting something the model
/// failed to establish.
pub fn degrade(span: &str, rung: Rung, why: &str) -> (UnitCore, Diagnostic) {
    let status = ceiling::ceiling(rung).min(Status::Speculative);
    let gist = synth_gist(span);

    let mut b = UnitCoreBuilder::new(KernelType::Prose, &gist, status)
        .body(span)
        .payload(payload_marker());
    // A prose unit with no body is possible when the span was whitespace; the builder
    // rejects an empty body, so it is simply omitted.
    if span.trim().is_empty() {
        b = UnitCoreBuilder::new(KernelType::Prose, &gist, status).payload(payload_marker());
    }

    let core = b
        .build()
        .expect("an opaque prose unit has no shape requirement to violate");

    let d = Diagnostic::on(Code::W304, smysl_core::canonical_uid(&core))
        .with_message(format!("span degraded to opaque prose after {why}"));
    (core, d)
}

/// `{ "ingest:unrepaired": true }` as deterministic CBOR.
///
/// Hand-encoded: a one-key map of a text key to `true` is four bytes, and reaching for an
/// encoder to produce them would be the tail wagging the dog.
fn payload_marker() -> Vec<u8> {
    let mut out = vec![0xa1];
    let key = UNREPAIRED_KEY.as_bytes();
    // A text string of length < 24 encodes in one head byte.
    out.push(0x60 | key.len() as u8);
    out.extend_from_slice(key);
    out.push(0xf5); // true
    out
}

/// Whether a unit is one of rule I's opaque survivors.
pub fn is_unrepaired(core: &UnitCore) -> bool {
    core.payload.as_deref() == Some(payload_marker().as_slice())
}

/// A gist for an opaque span: its first sentence, truncated.
///
/// Not a summary. Inventing one would be asserting something the model failed to establish,
/// which is precisely the failure that got the span here.
fn synth_gist(span: &str) -> String {
    let text = span.trim();
    if text.is_empty() {
        return "an unrepairable span with no content".to_string();
    }
    let first = text
        .split_terminator(['.', '!', '?', '\n'])
        .next()
        .unwrap_or(text)
        .trim();
    let first = if first.is_empty() { text } else { first };

    let limit = crate::schema::GIST_MAX_CHARS;
    if first.chars().count() <= limit {
        return first.to_string();
    }
    let mut out: String = first.chars().take(limit - 1).collect();
    // Trim back to a word boundary so the gist reads as a shortened sentence rather than a
    // severed one.
    if let Some(i) = out.rfind(char::is_whitespace) {
        out.truncate(i);
    }
    out.push('\u{2026}');
    out
}

/// Local checks only: shape, rule L, granularity, and rule T - never rule M (§22.3).
///
/// Rule M is the one exclusion, and §22.3 gives the reason: grounds may reference units the
/// chunk did not contain, so a claim resting on something ingested an hour ago would read as
/// a violation here and is not one. It is checked at staging instead.
///
/// Granularity *is* local - a body with two paragraphs under single-assertion admission is
/// wrong whatever else is in the store - so it belongs here, where the model can still be
/// asked to fix it. Leaving it to staging would mean discovering it after the calls were
/// paid for and with no turn left to spend.
pub fn check_local(units: &[UnitCore], rung: Rung) -> Report {
    let store = Store::from_records(units.iter().cloned().map(Record::Unit).collect());
    let mut report = check(
        &store,
        CheckOptions::default().only([Pass::Shape, Pass::Closure, Pass::Granularity, Pass::Trust]),
    );

    for u in units {
        if u.status > ceiling::ceiling(rung) {
            report.push(
                Diagnostic::on(Code::E033, smysl_core::canonical_uid(u))
                    .with_message(format!("status {} exceeds the {rung} ceiling", u.status)),
            );
        }
    }
    report.sort();
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD_SURFACE: &str = "@claim c/one { status: speculative }\n~ the pool saturated\n";

    #[test]
    fn a_clean_surface_answer_converts_without_diagnostics() {
        let (units, d) = convert(GOOD_SURFACE, IngestPath::Surface, Rung::Model);
        assert_eq!(units.len(), 1);
        assert!(!needs_repair(&d), "{d:?}");
    }

    #[test]
    fn a_clean_json_answer_converts_without_diagnostics() {
        let (units, d) = convert(
            r#"{"units":[{"type":"claim","gist":"the pool saturated","status":"speculative"}]}"#,
            IngestPath::JsonAst,
            Rung::Model,
        );
        assert_eq!(units.len(), 1);
        assert!(!needs_repair(&d), "{d:?}");
    }

    /// Rule T is applied after parse on both paths, whatever else was wrong.
    #[test]
    fn the_ceiling_applies_on_both_paths() {
        let json = r#"{"units":[{"type":"evidence","gist":"p95 rose","status":"measured",
                       "source":{"kind":"metric","ref":"m"}}]}"#;
        let (units, d) = convert(json, IngestPath::JsonAst, Rung::Model);
        assert_eq!(units.len(), 1);
        assert!(units[0].status < Status::Measured, "not capped");
        assert!(d.iter().any(|x| x.code == Code::E033), "not reported");

        let surface =
            "@evidence e/x { status: measured, source: { kind: metric, ref: m } }\n~ p95 rose\n";
        let (units, d) = convert(surface, IngestPath::Surface, Rung::Model);
        assert_eq!(units.len(), 1);
        assert!(units[0].status < Status::Measured);
        assert!(d.iter().any(|x| x.code == Code::E033));
    }

    #[test]
    fn every_rung_caps_on_the_way_through() {
        let json = r#"{"units":[{"type":"evidence","gist":"g","status":"measured",
                       "source":{"kind":"metric","ref":"m"},"grounds":[]}]}"#;
        for &r in Rung::ALL {
            let (units, _) = convert(json, IngestPath::JsonAst, r);
            assert!(units[0].status <= ceiling::ceiling(r), "{r}");
        }
    }

    /// Only errors are worth a call. A warning means the answer is usable and something is
    /// worth saying; re-asking would spend a call to remove a remark.
    #[test]
    fn only_errors_warrant_a_repair_attempt() {
        assert!(needs_repair(&[Diagnostic::new(Code::E001)]));
        assert!(!needs_repair(&[Diagnostic::new(Code::W304)]));
        assert!(!needs_repair(&[]));
    }

    /// Rule T's cap is the fix, so the diagnostic recording it is not work outstanding.
    /// Retrying it asks the model to solve a problem that no longer exists - and a model
    /// confident enough to claim `measured` once claims it again, so the budget is spent
    /// and the chunk degrades with good capped units inside it.
    #[test]
    fn a_capped_ceiling_does_not_spend_the_repair_budget() {
        let json = r#"{"units":[{"type":"evidence","gist":"p95 rose","status":"measured",
                       "source":{"kind":"metric","ref":"m"}}]}"#;
        let (units, d) = convert(json, IngestPath::JsonAst, Rung::Document);

        assert!(d.iter().any(|x| x.code == Code::E033), "still reported");
        assert!(!needs_repair(&d), "but not retried: {d:?}");
        assert!(
            units[0].status <= ceiling::ceiling(Rung::Document),
            "capped"
        );
    }

    /// The exemption is `E033` alone - a real error alongside it still buys a turn.
    #[test]
    fn a_capped_ceiling_does_not_mask_a_genuine_error() {
        let d = [Diagnostic::new(Code::E033), Diagnostic::new(Code::E001)];
        assert!(needs_repair(&d));
    }

    #[test]
    fn rendered_diagnostics_carry_only_the_errors() {
        let d = vec![
            Diagnostic::new(Code::E001).with_message("broken here"),
            Diagnostic::new(Code::W035).with_message("just a remark"),
        ];
        let text = render_diagnostics(&d);
        assert!(text.contains("SMY-E001"));
        assert!(text.contains("broken here"));
        assert!(!text.contains("SMY-W035"), "{text}");
    }

    // -- rule I ---------------------------------------------------------------

    /// **The gate.** An unrepairable span becomes an opaque `prose` unit rather than
    /// failing the run.
    #[test]
    fn an_unrepairable_span_becomes_opaque_prose() {
        let span = "This paragraph never parsed. It says something about the incident.";
        let (core, d) = degrade(span, Rung::Model, "2 attempts");

        assert_eq!(core.schema.kernel(), Some(KernelType::Prose));
        assert_eq!(d.code, Code::W304);
        assert_eq!(d.severity, Severity::Warn, "rule I: never fatal");
        assert!(is_unrepaired(&core));
    }

    /// Nothing is lost: a later pass, a human, or a better model can come back to it.
    #[test]
    fn the_raw_span_survives_verbatim_in_the_body() {
        let span = "Some text\nwith lines\n\nand paragraphs, unparseable as it is.";
        let (core, _) = degrade(span, Rung::Document, "2 attempts");
        assert_eq!(core.body.as_deref(), Some(span));
    }

    /// Inventing a summary would be asserting something the model failed to establish -
    /// which is precisely the failure that got the span here.
    #[test]
    fn the_gist_is_the_first_sentence_not_an_invention() {
        let (core, _) = degrade(
            "The pool saturated at noon. Then other things happened at length.",
            Rung::Model,
            "x",
        );
        assert_eq!(core.gist, "The pool saturated at noon");
    }

    #[test]
    fn a_long_first_sentence_is_truncated_on_a_word_boundary() {
        let span = format!("{} and it goes on", "word ".repeat(200));
        let (core, _) = degrade(&span, Rung::Model, "x");
        assert!(core.gist.chars().count() <= crate::schema::GIST_MAX_CHARS);
        assert!(core.gist.ends_with('\u{2026}'));
    }

    /// A degraded unit must never claim more than its rung allows either.
    #[test]
    fn a_degraded_unit_respects_the_ceiling() {
        for &r in Rung::ALL {
            let (core, _) = degrade("some text", r, "x");
            assert!(core.status <= ceiling::ceiling(r), "{r}");
            assert!(
                core.status <= Status::Speculative,
                "{r}: an opaque span is a guess"
            );
        }
    }

    #[test]
    fn an_empty_span_still_produces_a_unit() {
        let (core, _) = degrade("   \n  ", Rung::Model, "x");
        assert!(!core.gist.is_empty());
        assert!(is_unrepaired(&core));
    }

    #[test]
    fn the_marker_is_recognisable_and_specific() {
        let (degraded, _) = degrade("text", Rung::Model, "x");
        assert!(is_unrepaired(&degraded));

        let ordinary = UnitCoreBuilder::new(KernelType::Claim, "g", Status::Speculative)
            .build()
            .unwrap();
        assert!(!is_unrepaired(&ordinary));
    }

    /// The payload is deterministic CBOR, because it is part of the unit's identity.
    #[test]
    fn the_marker_payload_is_the_canonical_encoding() {
        let p = payload_marker();
        assert_eq!(p[0], 0xa1, "a one-key map");
        assert_eq!(p[1], 0x60 | UNREPAIRED_KEY.len() as u8, "a short text key");
        assert_eq!(*p.last().unwrap(), 0xf5, "true");
        assert_eq!(p, payload_marker());
    }

    /// Two identical unrepairable spans produce the same uid and merge to one unit with two
    /// attestations - the same self-healing that makes chunk overlap free.
    #[test]
    fn identical_degraded_spans_share_a_uid() {
        let (a, _) = degrade("the same text", Rung::Model, "one reason");
        let (b, _) = degrade("the same text", Rung::Model, "another reason");
        assert_eq!(smysl_core::canonical_uid(&a), smysl_core::canonical_uid(&b));
    }

    // -- local checks ---------------------------------------------------------

    /// A body with two paragraphs under single-assertion admission is wrong whatever else
    /// is in the store, so the model can still be asked to fix it. This is the case a live
    /// DeepSeek run produced, and the reason `check_local` is wired into the loop at all.
    #[test]
    fn local_checks_catch_a_granularity_violation() {
        let sprawling = UnitCoreBuilder::new(KernelType::Claim, "one gist", Status::Speculative)
            .body("First paragraph of the body.\n\nSecond paragraph of the body.")
            .build()
            .unwrap();
        let report = check_local(&[sprawling], Rung::Document);
        assert!(!report.is_clean());
        assert!(
            report.iter().any(|d| d.code == Code::E040),
            "{:?}",
            report.iter().collect::<Vec<_>>()
        );
    }

    /// Rule M is not checked here: a claim resting on something ingested an hour ago is not
    /// a violation, it is the normal case.
    #[test]
    fn local_checks_cover_the_ceiling_and_not_rule_m() {
        // A `derived` unit whose ground is a `speculative` one in the same batch: rule M
        // would object, and `check_local` deliberately does not.
        let weak = UnitCoreBuilder::new(KernelType::Claim, "a guess", Status::Speculative)
            .build()
            .unwrap();
        let strong = UnitCoreBuilder::new(KernelType::Finding, "a strong claim", Status::Derived)
            .grounds([smysl_core::canonical_uid(&weak)])
            .build()
            .unwrap();
        let report = check_local(&[weak, strong], Rung::Computed);
        assert!(
            !report.iter().any(|d| d.code == Code::E030),
            "rule M must wait for staging"
        );

        // A unit above its ceiling is caught; the converter would normally have capped it,
        // so this is the belt to that braces.
        let over = UnitCoreBuilder::new(KernelType::Evidence, "g", Status::Measured)
            .source(smysl_core::SourceRef::new(
                smysl_core::SourceKind::Metric,
                "m",
            ))
            .build()
            .unwrap();
        assert!(!check_local(&[over], Rung::Model).is_clean());
    }

    // -- never a failure ------------------------------------------------------

    /// Rule I: every path out of this module produces something, or the run could fail on a
    /// bad answer - which is exactly what rule I forbids.
    #[test]
    fn no_answer_makes_conversion_panic() {
        for answer in [
            "",
            "   ",
            "not anything at all",
            "@claim",
            "@claim c/x {",
            "{\"units\":",
            "\u{0}\u{1}",
            &"@".repeat(500),
        ] {
            for path in [IngestPath::Surface, IngestPath::JsonAst] {
                let (_, d) = convert(answer, path, Rung::Model);
                // Whatever happened, the caller can act: either units or diagnostics.
                let _ = needs_repair(&d);
            }
            // And whatever it was, it can always degrade rather than fail.
            let (core, _) = degrade(answer, Rung::Model, "exhausted");
            assert!(is_unrepaired(&core));
        }
    }
}
