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
//! unconditionally in `convert`, so `SMY-E033` records what a model tried rather than work
//! outstanding. See `needs_repair`.

use std::collections::BTreeMap;

use smysl_check::{check, CheckOptions, Pass};
use smysl_core::{
    canonical_uid, Code, Diagnostic, GranularityProfile, KernelType, Label, Record, Relation,
    Report, Rung, Severity, Status, Subject, Uid, UnitCore, UnitCoreBuilder,
};
use smysl_graph::Store;

use crate::ceiling;
use crate::json_ast;
use crate::IngestPath;

/// The payload marker an unrepairable span carries (§22.3).
pub const UNREPAIRED_KEY: &str = "ingest:unrepaired";

/// Convert one model answer into units, checking what can be checked locally.
///
/// Returns the units and any diagnostics. The caller decides whether to spend a repair
/// attempt; this function has no opinion about retries, which keeps it testable without a
/// provider.
pub fn convert(
    answer: &str,
    path: IngestPath,
    rung: Rung,
) -> (Vec<UnitCore>, Vec<Relation>, Vec<Diagnostic>) {
    convert_with(answer, path, rung, None)
}

/// [`convert`], with a caller-supplied source applied on either path before units are built.
pub fn convert_with(
    answer: &str,
    path: IngestPath,
    rung: Rung,
    source: Option<&(smysl_core::SourceRef, smysl_core::SourcePolicy)>,
) -> (Vec<UnitCore>, Vec<Relation>, Vec<Diagnostic>) {
    let (units, relations, diagnostics, _) = convert_labelled(answer, path, rung, source);
    (units, relations, diagnostics)
}

/// [`convert_with`], and the labels the answer gave its units, following rule T's cap.
///
/// The labels used to stop here: nothing downstream received them, so every staged unit was
/// unnamed. They are remapped through the cap because capping a status moves a uid, and a label
/// left on the old one would name a unit that is not in the batch.
pub fn convert_labelled(
    answer: &str,
    path: IngestPath,
    rung: Rung,
    source: Option<&(smysl_core::SourceRef, smysl_core::SourcePolicy)>,
) -> (
    Vec<UnitCore>,
    Vec<Relation>,
    Vec<Diagnostic>,
    BTreeMap<smysl_core::Label, smysl_core::Uid>,
) {
    let answer = crate::prompt::strip_echo(answer);
    let mut labels = BTreeMap::new();
    let mut relations = Vec::new();
    let (mut units, mut diagnostics) = match path {
        IngestPath::JsonAst => {
            let out = json_ast::convert_with(answer, source);
            relations = out.relations;
            labels = out.labels;
            (out.units, out.diagnostics)
        }
        IngestPath::Surface => match smysl_core::surface::parse_surface_with(answer, &{
            let mut o = smysl_core::surface::ParseOptions::default();
            if let Some((s, p)) = source {
                o = o.with_source(s.clone(), *p);
            }
            o
        }) {
            Ok(out) => {
                relations = out
                    .records
                    .iter()
                    .filter_map(|r| match r {
                        Record::Relation(rel) => Some(rel.clone()),
                        _ => None,
                    })
                    .collect();
                labels = out.labels.clone();
                (
                    out.units().cloned().collect::<Vec<_>>(),
                    out.diagnostics.clone(),
                )
            }
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
    let before: Vec<_> = units.iter().map(smysl_core::canonical_uid).collect();
    let mut capped = Vec::with_capacity(units.len());
    for u in units.drain(..) {
        let applied = ceiling::apply(&u, rung, None);
        diagnostics.extend(applied.diagnostics);
        capped.push(applied.core);
    }

    // The cap moves identities, so everything pointing at a capped unit has to follow it.
    // Latent until relations existed: a `grounds` entry naming a unit rule T then lowered
    // was already dangling, and only showed up as an `SMY-E060` at staging.
    let (capped, relations, remap) = crate::monotone::resettle(&before, capped, relations);
    let labels = labels
        .into_iter()
        .map(|(l, u)| (l, remap.get(&u).copied().unwrap_or(u)))
        .collect();

    (capped, relations, diagnostics, labels)
}

/// Whether a set of diagnostics is worth spending a repair attempt on.
///
/// Only errors are. A warning means the answer is usable and something is worth saying;
/// re-asking the model would spend a call to remove a remark.
///
/// **`SMY-E033` is the exception**, because rule T already fixed it. The ceiling is applied
/// unconditionally in `convert` and the diagnostic records that a model tried, not that
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

/// Errors that are a defect of one unit and say nothing about the others in its answer.
///
/// Only `SMY-E022`, a gist over `l0_max`, and that is not an oversight — it is the only error a
/// *constructed* unit can carry. A model cannot count tokens the way the estimator does, so an
/// over-long gist can survive every repair turn while its siblings were fine from the first, and
/// rule I degrading the whole span cost a live run all 16 of its valid units for one gist a token
/// over.
///
/// 1.7 tried to widen this to the other defects that are one unit's shape — `detail` without a
/// `body` (`SMY-E023`), `derived` with no grounds (`SMY-E031`), `cited` with no source
/// (`SMY-E032`), an authored `unfounded` (`SMY-E034`) — and found the widening inert.
/// `UnitCoreBuilder` refuses all four at construction, and the CBOR decoder runs the same
/// constructor, so a unit reaching `salvage` cannot be carrying one; `passes::shape` keeps them
/// only as defence in depth against a future constructor bypass. `SMY-E022` is genuinely
/// different, as that pass says: the gist bound is relative to a granularity profile, which a
/// constructor has no access to.
///
/// Two codes are excluded on judgement rather than reachability, and both are worth keeping:
///
/// - A fabricated quote (`SMY-E307`) is a unit's defect too, but it is evidence about the answer
///   it came in: a model that invented one attribution may have invented others.
/// - A multi-assertion body (`SMY-E040`) is the same kind of evidence — a model running two
///   claims into one unit is describing how it is reading, not slipping on one unit.
pub const UNIT_LOCAL: &[Code] = &[Code::E022];

/// What [`salvage`] kept of an answer whose repair budget ran out.
#[derive(Debug)]
pub struct Salvaged {
    pub units: Vec<UnitCore>,
    pub relations: Vec<Relation>,
    pub labels: BTreeMap<Label, Uid>,
    /// One `SMY-W304` per degraded unit, then the answer's remaining diagnostics.
    pub diagnostics: Vec<Diagnostic>,
    /// How many units were degraded.
    pub degraded: usize,
}

/// Rule I applied to the unit rather than the span, when that is where the defect is.
///
/// `None` unless every error that would need repair is in [`UNIT_LOCAL`] and names a unit in
/// the answer. Otherwise: each such unit degrades to opaque prose holding its own gist and
/// body, and so does every unit in the answer that rests on one through `grounds` or `deps`,
/// transitively — a unit is not kept resting on something that was not. Relations touching a
/// degraded unit are dropped and so are labels naming one; everything else is kept as the
/// model wrote it.
pub fn salvage(
    units: &[UnitCore],
    relations: &[Relation],
    labels: &BTreeMap<Label, Uid>,
    diagnostics: &[Diagnostic],
    rung: Rung,
    why: &str,
) -> Option<Salvaged> {
    let present: std::collections::BTreeSet<Uid> = units.iter().map(canonical_uid).collect();
    let errors: Vec<&Diagnostic> = diagnostics
        .iter()
        .filter(|d| needs_repair(std::slice::from_ref(*d)))
        .collect();
    if units.is_empty() || errors.is_empty() {
        return None;
    }
    let mut bad = std::collections::BTreeSet::new();
    for d in &errors {
        match d.subject {
            Subject::Unit(u) if UNIT_LOCAL.contains(&d.code) && present.contains(&u) => {
                bad.insert(u);
            }
            _ => return None,
        }
    }
    let defective = bad.clone();
    loop {
        let before = bad.len();
        for u in units {
            if u.grounds.iter().chain(&u.deps).any(|g| bad.contains(g)) {
                bad.insert(canonical_uid(u));
            }
        }
        if bad.len() == before {
            break;
        }
    }
    if bad.len() == units.len() {
        return None;
    }

    let mut out = Salvaged {
        units: Vec::new(),
        relations: Vec::new(),
        labels: BTreeMap::new(),
        diagnostics: Vec::new(),
        degraded: 0,
    };
    for u in units {
        let uid = canonical_uid(u);
        if !bad.contains(&uid) {
            out.units.push(u.clone());
            continue;
        }
        let text = match &u.body {
            Some(b) => format!("{}\n\n{b}", u.gist),
            None => u.gist.clone(),
        };
        let reason = if defective.contains(&uid) {
            format!("its own error after {why}")
        } else {
            "it rests on a unit that degraded".to_string()
        };
        let (core, mut d) = degrade(&text, rung, &reason);
        d.message = format!("unit degraded to opaque prose: {reason}");
        out.units.push(core);
        out.diagnostics.push(d);
        out.degraded += 1;
    }
    out.relations = relations
        .iter()
        .filter(|r| !bad.contains(&r.from) && !bad.contains(&r.to))
        .cloned()
        .collect();
    out.labels = labels
        .iter()
        .filter(|(_, u)| !bad.contains(u))
        .map(|(l, u)| (l.clone(), *u))
        .collect();
    // The errors are already in the caller's attempt history; what is left concerns the kept
    // units (an elided quote, a capped status) and is reported as on a clean answer.
    out.diagnostics.extend(
        diagnostics
            .iter()
            .filter(|d| !needs_repair(std::slice::from_ref(*d)))
            .filter(|d| !matches!(d.subject, Subject::Unit(u) if bad.contains(&u)))
            .cloned(),
    );
    Some(out)
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

    // The bound is `l0_max` as the estimator counts it, four bytes a token. It was
    // `GIST_MAX_CHARS`, 240 characters — twice what `SMY-E022` allows — so a span whose first
    // sentence was long degraded to a prose unit that failed the gist check at staging.
    let budget = GranularityProfile::default().l0_max as usize * 4;
    if first.len() <= budget {
        return first.to_string();
    }
    let ellipsis = '\u{2026}'.len_utf8();
    let mut out = String::new();
    for c in first.chars() {
        if out.len() + c.len_utf8() + ellipsis > budget {
            break;
        }
        out.push(c);
    }
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

    /// Why `UNIT_LOCAL` is one code long, asserted rather than argued (1.7).
    ///
    /// The other shape defects cannot reach `salvage`: `UnitCoreBuilder` refuses them, so there
    /// is no unit to degrade. A future constructor that stopped refusing one would fail here
    /// first, which is the point of writing it down.
    #[test]
    fn the_other_shape_defects_cannot_reach_salvage() {
        use smysl_core::{KernelType, Status, UnitCoreBuilder};

        let refused = [
            UnitCoreBuilder::new(KernelType::Claim, "the canary stayed clean", Status::Cited)
                .build()
                .err(),
            UnitCoreBuilder::new(KernelType::Claim, "the pool saturated", Status::Derived)
                .build()
                .err(),
            UnitCoreBuilder::new(KernelType::Claim, "the pool saturated", Status::Unfounded)
                .build()
                .err(),
            UnitCoreBuilder::new(KernelType::Claim, "the pool saturated", Status::Speculative)
                .detail("a detail with no body above it")
                .build()
                .err(),
        ];
        for (i, e) in refused.iter().enumerate() {
            let code = e
                .as_ref()
                .unwrap_or_else(|| panic!("case {i} was built, so its code could reach salvage"))
                .code();
            assert!(
                !UNIT_LOCAL.contains(&code),
                "{code} is unreachable at salvage, so listing it would be inert"
            );
        }
        assert_eq!(UNIT_LOCAL, &[Code::E022]);
    }

    /// And a defect that is evidence about the whole answer still degrades the whole answer.
    #[test]
    fn a_fabricated_quote_is_not_a_unit_local_defect() {
        use smysl_core::{KernelType, Status, UnitCoreBuilder};

        let u = UnitCoreBuilder::new(KernelType::Claim, "the pool saturated", Status::Speculative)
            .build()
            .unwrap();
        let uid = canonical_uid(&u);
        assert!(
            salvage(
                std::slice::from_ref(&u),
                &[],
                &BTreeMap::new(),
                &[Diagnostic::on(Code::E307, uid)],
                Rung::Model,
                "the budget ran out",
            )
            .is_none(),
            "a model that invented one attribution may have invented others"
        );
    }

    #[test]
    fn a_clean_surface_answer_converts_without_diagnostics() {
        let (units, _, d) = convert(GOOD_SURFACE, IngestPath::Surface, Rung::Model);
        assert_eq!(units.len(), 1);
        assert!(!needs_repair(&d), "{d:?}");
    }

    #[test]
    fn a_clean_json_answer_converts_without_diagnostics() {
        let (units, _, d) = convert(
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
        let (units, _, d) = convert(json, IngestPath::JsonAst, Rung::Model);
        assert_eq!(units.len(), 1);
        assert!(units[0].status < Status::Measured, "not capped");
        assert!(d.iter().any(|x| x.code == Code::E033), "not reported");

        let surface =
            "@evidence e/x { status: measured, source: { kind: metric, ref: m } }\n~ p95 rose\n";
        let (units, _, d) = convert(surface, IngestPath::Surface, Rung::Model);
        assert_eq!(units.len(), 1);
        assert!(units[0].status < Status::Measured);
        assert!(d.iter().any(|x| x.code == Code::E033));
    }

    #[test]
    fn every_rung_caps_on_the_way_through() {
        let json = r#"{"units":[{"type":"evidence","gist":"g","status":"measured",
                       "source":{"kind":"metric","ref":"m"},"grounds":[]}]}"#;
        for &r in Rung::ALL {
            let (units, _, _) = convert(json, IngestPath::JsonAst, r);
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
        let (units, _, d) = convert(json, IngestPath::JsonAst, Rung::Document);

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
        assert!(core.gist.ends_with('\u{2026}'));
        assert!(core.gist.ends_with("word\u{2026}"), "{}", core.gist);
    }

    /// The synthesised gist passes the gist check it will meet at staging, in bytes as the
    /// estimator counts them — multi-byte text included. It was bounded at 240 characters.
    #[test]
    fn a_degraded_gist_is_within_l0_max() {
        let l0 = GranularityProfile::default().l0_max;
        for span in ["word ".repeat(200), "сервер ".repeat(100), "x".repeat(1000)] {
            let (core, _) = degrade(&span, Rung::Model, "x");
            let n = smysl_core::tokens(&core.gist);
            assert!(n <= l0, "{n} > {l0}: {}", core.gist);
            let report = check_local(std::slice::from_ref(&core), Rung::Model);
            assert!(!report.iter().any(|d| d.code == Code::E022), "{report:?}");
        }
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
                let (_, _, d) = convert(answer, path, Rung::Model);
                // Whatever happened, the caller can act: either units or diagnostics.
                let _ = needs_repair(&d);
            }
            // And whatever it was, it can always degrade rather than fail.
            let (core, _) = degrade(answer, Rung::Model, "exhausted");
            assert!(is_unrepaired(&core));
        }
    }
}
