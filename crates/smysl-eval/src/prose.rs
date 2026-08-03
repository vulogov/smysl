//! The prose baseline arm (§28): each hop reads prose and writes prose.
//!
//! This is the arm the whole document is an argument against, so it has to be run honestly
//! rather than imagined. Two things make that awkward, and both are solved here by naming
//! them rather than by working around them.
//!
//! **It needs a model at every hop.** A hop is a summarisation, and there is no
//! deterministic stand-in for one: a simulated baseline would measure the simulation. So
//! this module defines [`Summariser`] as a *port* and never links a provider. The
//! deterministic tests drive it with a fake; the live test wires a real endpoint.
//!
//! **Prose has no uids.** The smysl arm measures survival by tracking identities through
//! the chain, which prose destroys by construction - that is the point of the experiment.
//! Survival is therefore decided by a [`Judge`], which is also a model, and the judging is
//! kept apart from the summarising so that a bad judge cannot be mistaken for a bad
//! baseline: [`Judged::abstained`] counts what the judge would not rule on.

use std::collections::BTreeSet;

use smysl_core::{tokens, SourceRef, Status, Uid};
use smysl_graph::Store;

/// Anything that can go wrong in an arm that talks to a model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalError(pub String);

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(&self.0)
    }
}

impl std::error::Error for EvalError {}

/// One hop: read prose, write shorter prose.
///
/// A port, so this crate links no HTTP client and the arm stays testable without a key.
pub trait Summariser {
    /// Summarise `text` for the next system in the chain, within `max_tokens`.
    fn summarise(&self, text: &str, max_tokens: u64) -> Result<String, EvalError>;
}

/// What the original graph asserted, in the form a judge can be asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub uid: Uid,
    pub gist: String,
    /// What the original said about its own confidence.
    pub status: Status,
    /// What the original said it rested on, as the prose names it. `None` when the unit
    /// carried no source, in which case there is no attribution to lose.
    pub source: Option<String>,
}

/// A judge's reading of one claim against one piece of prose.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Verdict {
    /// Whether the claim is stated in the text at all.
    pub present: bool,
    /// The strongest confidence the *text* supports, which is the number E3 is about.
    /// `None` when the judge would not rule.
    pub as_stated: Option<Status>,
    /// What the passage says the claim rests on, if it still says anything. `None` means
    /// the text states the claim with nothing behind it.
    pub attributed_to: Option<String>,
}

impl Verdict {
    pub fn absent() -> Verdict {
        Verdict {
            present: false,
            as_stated: None,
            attributed_to: None,
        }
    }
}

/// Decides whether a claim survived a chain of summarisation, and how it now reads.
pub trait Judge {
    fn verdict(&self, claim: &Claim, text: &str) -> Result<Verdict, EvalError>;
}

/// The prose arm's trace.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProseRun {
    /// The text handed to hop 1.
    pub initial: String,
    /// What each hop produced, in order.
    pub hops: Vec<String>,
}

impl ProseRun {
    /// The text the last system in the chain received.
    pub fn final_text(&self) -> &str {
        self.hops
            .last()
            .map(String::as_str)
            .unwrap_or(&self.initial)
    }

    pub fn initial_tokens(&self) -> u64 {
        u64::from(tokens(&self.initial))
    }

    pub fn final_tokens(&self) -> u64 {
        u64::from(tokens(self.final_text()))
    }
}

/// How a careful writer would hedge a claim of this status in ordinary prose.
///
/// **This is the load-bearing part of the baseline, and getting it wrong makes the whole
/// experiment vacuous.** A store keeps confidence in a *field*; prose has no fields, so a
/// renderer that simply dropped the status would hand the baseline a passage with no hedges
/// in it at all. Every hedge would then be "lost" before the first hop, and the measurement
/// would be of this function deleting a column rather than of summarisation destroying
/// meaning. The baseline has to start with the epistemics stated in words, exactly as a
/// competent human writing prose would state them, or it is not a baseline.
fn hedge(status: Status) -> &'static str {
    match status {
        Status::Unfounded => "This has been retracted, but it was claimed that",
        Status::Speculative => "It is possible, though unconfirmed, that",
        Status::Inferred => "We infer, from reasoning rather than direct evidence, that",
        Status::Derived => "It follows from the evidence recorded here that",
        Status::Cited => "According to the source cited alongside it,",
        Status::Measured => "Measurement shows that",
        _ => "It is reported that",
    }
}

/// How a careful writer would state a source in ordinary prose.
///
/// **The second load-bearing renderer, and wrong in the same way the first was.** A store
/// keeps provenance in a field; prose has none, so a renderer that dropped the source would
/// hand the baseline a passage attributing nothing — every attribution "lost" before the
/// first hop, and the measurement would be of this function rather than of summarisation.
fn attribution(source: &SourceRef) -> String {
    format!(
        "according to {} {}",
        match source.kind {
            smysl_core::SourceKind::Metric => "the metric",
            smysl_core::SourceKind::Doc => "the document",
            smysl_core::SourceKind::File => "the file",
            smysl_core::SourceKind::Url => "the page at",
            smysl_core::SourceKind::Tool => "the tool",
            _ => "the source",
        },
        source.reference
    )
}

/// Lower an ordinary leading capital so the hedge reads as a sentence. `IEEE`, `SLO` and
/// `p99` are left exactly as written - the same rule the renderer's connectives use.
fn joined(hedge: &str, text: &str) -> String {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return hedge.to_string();
    };
    let rest: String = chars.collect();
    let lowered = if rest.chars().next().is_some_and(|c| c.is_lowercase()) {
        first.to_lowercase().to_string()
    } else {
        first.to_string()
    };
    format!("{hedge} {lowered}{rest}")
}

/// Render a store as the prose a baseline pipeline would have carried.
///
/// One unit per paragraph, in the store's own order, with its confidence written out in
/// words by `hedge`. Deliberately plain otherwise: dressing it up would be writing the
/// baseline's summary for it, and the comparison is about what summarisation costs rather
/// than about prose style.
pub fn to_prose(store: &Store) -> String {
    let mut out = String::new();
    for (_, unit) in store.units() {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&joined(hedge(unit.core.status), &unit.core.gist));
        if let Some(src) = &unit.core.source {
            out.push_str(&format!(" — {}.", attribution(src)));
        }
        if let Some(body) = &unit.core.body {
            out.push(' ');
            out.push_str(body);
        }
    }
    out
}

/// Every claim the original graph made, for the judge to look for afterwards.
pub fn claims_of(store: &Store) -> Vec<Claim> {
    store
        .units()
        .map(|(uid, unit)| Claim {
            uid: *uid,
            gist: unit.core.gist.clone(),
            status: unit.core.status,
            source: unit.core.source.as_ref().map(|s| s.reference.clone()),
        })
        .collect()
}

/// Run the prose arm: `hops` successive summarisations, each of the last one's output.
///
/// The budget is the same one the smysl arm was given, so E1 compares like with like.
pub fn run_prose_arm(
    store: &Store,
    hops: usize,
    budget: u64,
    model: &dyn Summariser,
) -> Result<ProseRun, EvalError> {
    let initial = to_prose(store);
    let mut run = ProseRun {
        hops: Vec::with_capacity(hops),
        initial,
    };

    let mut text = run.initial.clone();
    for _ in 0..hops {
        text = model.summarise(&text, budget)?;
        run.hops.push(text.clone());
    }
    Ok(run)
}

/// What the judge found in the final text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Judged {
    /// Claims the judge found stated.
    pub surviving: BTreeSet<Uid>,
    /// Claims the judge read as *more* certain than the original asserted them.
    ///
    /// This is E3 on the prose side, and the number the whole format exists to keep at
    /// zero: a hedge that vanishes across a handoff is a guess promoted to a finding.
    pub inflated: Vec<(Uid, Status, Status)>,
    /// Claims the judge declined to rule on. Reported rather than counted as either
    /// outcome: a judge that abstains often is a judge whose verdicts mean little, and
    /// folding abstentions into "absent" would flatter the format.
    pub abstained: usize,
    pub total: usize,

    /// Surviving claims whose *source* the passage still names.
    ///
    /// The second thing prose loses, after hedges. "By the third hop nobody can say where
    /// the number came from" is the README's own claim, and this is what tests it.
    pub attributed: BTreeSet<Uid>,
    /// Claims that had a source to lose in the first place - the denominator. A claim the
    /// original never sourced cannot have its attribution dropped, and counting it would
    /// dilute the measure in the format's favour.
    pub attributable: usize,
}

impl Judged {
    pub fn survival(&self) -> f64 {
        if self.total == 0 {
            return 1.0;
        }
        self.surviving.len() as f64 / self.total as f64
    }

    /// Whether the judge ruled often enough for the verdicts to mean anything.
    pub fn is_usable(&self) -> bool {
        self.total > 0 && self.abstained * 2 < self.total
    }

    /// The fraction of sourced claims whose attribution survived.
    pub fn attribution(&self) -> f64 {
        if self.attributable == 0 {
            return 1.0;
        }
        self.attributed.len() as f64 / self.attributable as f64
    }
}

/// Ask a judge which claims survived, and which got stronger on the way.
pub fn judge(claims: &[Claim], text: &str, judge: &dyn Judge) -> Result<Judged, EvalError> {
    let mut out = Judged {
        total: claims.len(),
        ..Judged::default()
    };
    for claim in claims {
        let v = judge.verdict(claim, text)?;
        if claim.source.is_some() {
            out.attributable += 1;
        }
        if !v.present {
            continue;
        }
        out.surviving.insert(claim.uid);

        // Attribution counts only when the passage names *this* claim's source, not merely
        // that some source exists somewhere in the text.
        if let (Some(was), Some(now)) = (&claim.source, &v.attributed_to) {
            if names_the_same_source(was, now) {
                out.attributed.insert(claim.uid);
            }
        }
        match v.as_stated {
            None => out.abstained += 1,
            Some(read) if read > claim.status => out.inflated.push((claim.uid, claim.status, read)),
            Some(_) => {}
        }
    }
    Ok(out)
}

/// Whether what the passage attributes a claim to is the source the original recorded.
///
/// Lenient about how a summariser words it and strict about the identifier: a reference is
/// a name, and a passage that says "a metric" without saying which one has not preserved
/// the attribution — it has preserved the *fact that there was one*, which is what a reader
/// cannot act on.
fn names_the_same_source(original: &str, stated: &str) -> bool {
    let norm = |s: &str| {
        s.to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '_' || *c == '/')
            .collect::<String>()
    };
    let (o, s) = (norm(original), norm(stated));
    if o.is_empty() {
        return false;
    }
    s.contains(&o) || o.contains(&s) && s.len() >= o.len() / 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{KernelType, Record, UnitCoreBuilder};

    /// A summariser that drops the last paragraph each hop and never rewrites anything.
    /// Deterministic, so the arm's own logic can be tested without a model - it is not a
    /// stand-in for one, and no metric is ever reported from it.
    struct Truncating;

    impl Summariser for Truncating {
        fn summarise(&self, text: &str, _max: u64) -> Result<String, EvalError> {
            let mut paras: Vec<&str> = text.split("\n\n").collect();
            paras.pop();
            Ok(paras.join("\n\n"))
        }
    }

    struct Failing;

    impl Summariser for Failing {
        fn summarise(&self, _t: &str, _m: u64) -> Result<String, EvalError> {
            Err(EvalError("no model".into()))
        }
    }

    /// A judge that says a claim is present when its gist appears verbatim, and reads
    /// everything as `Measured` - the worst case, so inflation is easy to assert.
    struct Literal;

    impl Judge for Literal {
        fn verdict(&self, claim: &Claim, text: &str) -> Result<Verdict, EvalError> {
            Ok(Verdict {
                present: text.contains(&claim.gist),
                as_stated: Some(Status::Measured),
                attributed_to: claim.source.clone(),
            })
        }
    }

    /// The baseline prose has to state the source in words, or attribution is "lost"
    /// before the first hop and the measurement is of `to_prose` rather than of the chain -
    /// the same mistake the hedges made.
    #[test]
    fn the_baseline_prose_states_the_source_in_words() {
        let core = UnitCoreBuilder::new(KernelType::Evidence, "p95 rose", Status::Measured)
            .source(smysl_core::SourceRef::new(
                smysl_core::SourceKind::Metric,
                "checkout.p95",
            ))
            .build()
            .unwrap();
        let text = to_prose(&Store::from_records(vec![Record::Unit(core)]));
        assert!(text.contains("checkout.p95"), "{text}");
        assert!(text.contains("according to"), "{text}");
    }

    /// A passage naming "a metric" without saying which one has kept the *fact* of a source
    /// and lost the attribution, which is the half a reader can act on.
    #[test]
    fn a_vague_attribution_does_not_count_as_survival() {
        assert!(names_the_same_source(
            "checkout.p95",
            "the metric checkout.p95"
        ));
        assert!(!names_the_same_source("checkout.p95", "a metric"));
        assert!(!names_the_same_source("checkout.p95", "pool.wait_ms"));
    }

    fn store() -> Store {
        let records: Vec<Record> = ["alpha claim", "beta claim", "gamma claim"]
            .iter()
            .map(|g| {
                Record::Unit(
                    UnitCoreBuilder::new(KernelType::Claim, *g, Status::Speculative)
                        .build()
                        .unwrap(),
                )
            })
            .collect();
        Store::from_records(records)
    }

    #[test]
    fn prose_carries_every_gist() {
        let text = to_prose(&store());
        for g in ["alpha claim", "beta claim", "gamma claim"] {
            assert!(text.contains(g), "{g} missing from {text:?}");
        }
    }

    #[test]
    fn each_hop_reads_the_previous_hop_not_the_original() {
        let run = run_prose_arm(&store(), 2, 100, &Truncating).unwrap();
        assert_eq!(run.hops.len(), 2);
        assert!(
            run.hops[1].len() < run.hops[0].len(),
            "the second hop did not consume the first"
        );
        assert!(run.final_tokens() < run.initial_tokens());
    }

    /// A provider failure must surface, not silently shorten the chain into a better
    /// looking result.
    #[test]
    fn a_failed_hop_is_an_error_rather_than_a_short_chain() {
        assert!(run_prose_arm(&store(), 5, 100, &Failing).is_err());
    }

    #[test]
    fn a_dropped_claim_does_not_survive() {
        let s = store();
        let run = run_prose_arm(&s, 1, 100, &Truncating).unwrap();
        let j = judge(&claims_of(&s), run.final_text(), &Literal).unwrap();
        assert_eq!(j.total, 3);
        assert_eq!(j.surviving.len(), 2, "one paragraph was dropped");
        assert!((j.survival() - 2.0 / 3.0).abs() < 1e-9);
    }

    /// The measurement the format exists for: a `speculative` claim that the text now
    /// states as `measured` is a hedge that vanished.
    #[test]
    fn a_claim_read_more_strongly_than_it_was_written_counts_as_inflated() {
        let s = store();
        let run = run_prose_arm(&s, 0, 100, &Truncating).unwrap();
        let j = judge(&claims_of(&s), run.final_text(), &Literal).unwrap();
        assert_eq!(j.inflated.len(), 3, "all three were read as measured");
        for (_, was, now) in &j.inflated {
            assert_eq!(*was, Status::Speculative);
            assert_eq!(*now, Status::Measured);
        }
    }

    /// Abstentions are reported, not folded into either answer.
    #[test]
    fn an_abstaining_judge_is_reported_as_unusable() {
        struct Abstains;
        impl Judge for Abstains {
            fn verdict(&self, _c: &Claim, _t: &str) -> Result<Verdict, EvalError> {
                Ok(Verdict {
                    present: true,
                    as_stated: None,
                    attributed_to: None,
                })
            }
        }
        let s = store();
        let j = judge(&claims_of(&s), "anything", &Abstains).unwrap();
        assert_eq!(j.abstained, 3);
        assert!(!j.is_usable(), "a judge that never rules is not a judge");
        assert!(j.inflated.is_empty(), "an abstention is not an inflation");
    }

    #[test]
    fn zero_hops_leaves_the_text_untouched() {
        let s = store();
        let run = run_prose_arm(&s, 0, 100, &Truncating).unwrap();
        assert_eq!(run.final_text(), run.initial);
    }
}
