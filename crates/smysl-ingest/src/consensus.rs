//! Consensus extraction — two passes per window, and what their agreement means (§3.4).
//!
//! SMYSL-2.4 §1.1 item 2 makes extraction consensus by default: a window is extracted
//! **twice**, both results are staged, and nothing is deduplicated by this module. The
//! agreement is *read afterwards*, over staged units, because the only relation that can
//! establish it without a judge is `identical-span` — same tid, same span, same gist — and
//! S0 measured the judges at α 0.600, which is why the broader form is not claimed.
//!
//! **The second pass is worthless above temperature 0,** and that is a measurement rather
//! than a preference. S0's MS-8 found arm L bit-identical across all 145 runs at T=0, and
//! sharing *no uid at all* at T=0.7 on five of six inputs: two runs at T>0 cannot agree by
//! `identical-span` except by accident. So [`Passes::admits`] refuses the combination, and
//! `temperature` is a correctness parameter here rather than a quality knob.
//!
//! **What this module does not do.** It does not merge, dedup, vote or score. Two passes that
//! produce the same unit produce the same uid, so the staged batch holds one record for it —
//! that is content addressing, not deduplication, and it is why the agreement is *recorded*
//! by [`crate::IngestReport`] rather than inferred later from a unit count that cannot
//! distinguish "both passes said this" from "one did".

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::UnitCore;

/// How many times a window is extracted.
///
/// `Single` is the default, and deliberately: the doubling §3.4 states is `ingest --text`'s,
/// and document ingest — every `ingest` this release has — would otherwise double its cost
/// and its call count for every caller that never asked. [`Passes::TEXT`] is what the text
/// path takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[non_exhaustive]
pub enum Passes {
    /// One pass. `--single-pass`, and what every release up to 1.10 did.
    #[default]
    Single,
    /// Two passes, both staged. Two models where two are configured, otherwise two runs of
    /// one — the registry decides which, since both passes ask the same task.
    Consensus,
}

impl Passes {
    /// What `ingest --text` extracts under (§3.4).
    pub const TEXT: Passes = Passes::Consensus;

    /// How many extractions one window gets.
    pub const fn count(self) -> usize {
        match self {
            Passes::Single => 1,
            Passes::Consensus => 2,
        }
    }

    /// The identifier a recipe records. A corpus built one way is not comparable with one
    /// built the other, which is why it is in the recipe at all.
    pub const fn id(self) -> &'static str {
        match self {
            Passes::Single => "single",
            Passes::Consensus => "consensus",
        }
    }

    /// Whether this policy admits a sampling temperature, and why it does not (MS-8).
    ///
    /// The message names the flag, because a caller told only that the combination is refused
    /// has two ways to satisfy it and no way to know which one the tool meant.
    pub fn admits(self, temperature: f32) -> Result<(), String> {
        match self {
            Passes::Consensus if temperature > 0.0 => Err(format!(
                "a temperature of {temperature} with two passes: two runs above temperature 0 \
                 share no identity, so the second pass buys no `identical-span` agreement \
                 (S0 MS-8). Pass `--single-pass` to sample deliberately, or leave the \
                 temperature at 0"
            )),
            _ => Ok(()),
        }
    }
}

impl std::fmt::Display for Passes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.id())
    }
}

/// What two passes over one input agreed about, by `identical-span`.
///
/// `comparable` is distinct keys, not units: the question is how many extracted *places* both
/// passes found, and a key is a place. `spanless` counts units that carry no span, per pass,
/// since a unit with no span cannot establish `identical-span` in either direction — before
/// span attachment lands (§6 TX-P5 step 3) that is most of them, and reporting them as
/// disagreement would read as instability that was never measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Agreement {
    /// How many passes were compared. One pass agrees with nothing, and reports nothing.
    pub passes: usize,
    /// Distinct `identical-span` keys any pass produced.
    pub comparable: usize,
    /// Keys **every** pass produced.
    pub agreed: usize,
    /// Units no pass could compare, because they carry no span.
    pub spanless: usize,
    /// Distinct *places* — tid and span, with the gist left out — any pass produced.
    pub comparable_places: usize,
    /// Places **every** pass produced, whatever words each pass described them in.
    pub agreed_places: usize,
}

impl Agreement {
    /// `agreed / comparable`, or `None` when there was nothing to compare.
    ///
    /// `None` rather than 1.0, in both of the ways a comparison can be empty: no unit carried a
    /// span, or there was **one pass**. One pass agrees with itself by construction — every key
    /// it produced was produced by every pass — so a rate computed over it is 1.0 and means
    /// nothing. GE-T2 reports this figure per language, and a stability claim about nothing is
    /// worse than an absent one.
    pub fn rate(&self) -> Option<f64> {
        (self.passes > 1 && self.comparable > 0)
            .then(|| self.agreed as f64 / self.comparable as f64)
    }

    /// Add another window's share of a run, field by field, in **one** place.
    ///
    /// Per-window agreements are summed and not averaged: a window with two comparable places
    /// weighs twice a window with one, and a mean over windows would let a window that extracted
    /// a single claim outvote a chapter. `passes` is assigned rather than summed, because every
    /// window of a run takes the same number of passes.
    ///
    /// It is a method because the alternative is a caller that adds each field itself, and that
    /// caller silently drops any field added later — which is exactly what happened to
    /// `comparable_places` and `agreed_places` the first time they were measured over a corpus.
    pub fn merge(&mut self, other: &Agreement) {
        self.passes = other.passes;
        self.comparable += other.comparable;
        self.agreed += other.agreed;
        self.spanless += other.spanless;
        self.comparable_places += other.comparable_places;
        self.agreed_places += other.agreed_places;
    }

    /// The same question with the **gist left out**: `agreed_places / comparable_places`.
    ///
    /// `identical-span` includes the gist by §3.5, so [`Agreement::rate`] answers *did both
    /// passes find this place and describe it in the same words*, and the second half of that
    /// is prose. Two passes of a real model re-word a gist freely, so a low `rate` does not
    /// say whether the instability is in the finding or only in the wording — and the two have
    /// different remedies, one of them TX-P10's and the other nobody's. This figure separates
    /// them, and it is a diagnostic rather than a redefinition: GE-T2's reported figure stays
    /// [`Agreement::rate`].
    ///
    /// A scripted provider cannot tell the two apart, because it returns one string by
    /// construction and its two passes agree on the wording for free.
    pub fn place_rate(&self) -> Option<f64> {
        (self.passes > 1 && self.comparable_places > 0)
            .then(|| self.agreed_places as f64 / self.comparable_places as f64)
    }
}

/// The `identical-span` key of a unit: tid, span, gist (§3.5).
///
/// The **tid** and not the whole reference, because `reference` may carry `#<locator>` and a
/// locator is a reader's name for a place: the span is authoritative and the locator
/// informative (A-2.1 key 5), so two passes that named one range `#3.16` and `#3.16a` found
/// the same bytes. The **mid** is deliberately absent — a tid identifies normalised part
/// bytes, so two manifestations of the same part text are the same text.
fn key(u: &UnitCore) -> Option<(String, (u64, u64), String)> {
    let (tid, span) = place(u)?;
    Some((tid, span, u.gist.clone()))
}

/// The same key with the gist left out: the *place* alone. See [`Agreement::place_rate`].
fn place(u: &UnitCore) -> Option<(String, (u64, u64))> {
    let s = u.source.as_ref()?;
    let span = s.span()?;
    let tid = match s.reference.split_once('#') {
        Some((tid, _)) => tid,
        None => s.reference.as_str(),
    };
    Some((tid.to_string(), span))
}

/// What the passes agreed about. Pure: it reads the units and writes nothing.
///
/// Fewer than two passes is not an error — a single-pass run has an agreement of one pass rather
/// than no answer — and [`Agreement::rate`] is `None` for it, because a single pass agrees with
/// itself by construction.
pub fn agreement(passes: &[Vec<UnitCore>]) -> Agreement {
    let mut seen: BTreeMap<(String, (u64, u64), String), BTreeSet<usize>> = BTreeMap::new();
    let mut places: BTreeMap<(String, (u64, u64)), BTreeSet<usize>> = BTreeMap::new();
    let mut spanless = 0usize;
    for (i, units) in passes.iter().enumerate() {
        for u in units {
            match key(u) {
                Some(k) => {
                    seen.entry(k).or_default().insert(i);
                    // `place` cannot be `None` where `key` was `Some`: it is the same read of
                    // the same source, one field shorter.
                    if let Some(p) = place(u) {
                        places.entry(p).or_default().insert(i);
                    }
                }
                None => spanless += 1,
            }
        }
    }
    let n = passes.len();
    Agreement {
        passes: n,
        comparable: seen.len(),
        agreed: seen.values().filter(|p| p.len() == n).count(),
        spanless,
        comparable_places: places.len(),
        agreed_places: places.values().filter(|p| p.len() == n).count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{KernelType, Mid, SourceKind, SourceRef, Status, UnitCoreBuilder};

    fn mid() -> Mid {
        Mid::of(b"a manifestation")
    }

    fn unit(gist: &str, span: Option<(u64, u64)>, locator: &str) -> UnitCore {
        let reference = format!("t3:{}{locator}", "a".repeat(64));
        let mut source = SourceRef::new(SourceKind::Doc, reference);
        if let Some((start, end)) = span {
            source = source.in_part(start, end, mid()).expect("a forward range");
        }
        UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
            .source(source)
            .build()
            .expect("a gist-only speculative unit")
    }

    #[test]
    fn two_passes_that_found_the_same_places_agree() {
        let a = vec![
            unit("one", Some((0, 10)), ""),
            unit("two", Some((10, 20)), ""),
        ];
        let b = a.clone();
        let g = agreement(&[a, b]);
        assert_eq!((g.passes, g.comparable, g.agreed, g.spanless), (2, 2, 2, 0));
        assert_eq!(g.rate(), Some(1.0));
    }

    /// The same bytes under two reader's names are the same place: the span decides.
    #[test]
    fn a_locator_does_not_decide_the_key() {
        let a = vec![unit("one", Some((0, 10)), "#3.16")];
        let b = vec![unit("one", Some((0, 10)), "#iii.16")];
        assert_eq!(agreement(&[a, b]).rate(), Some(1.0));
    }

    #[test]
    fn a_pass_that_found_one_more_place_lowers_the_rate() {
        let a = vec![unit("one", Some((0, 10)), "")];
        let mut b = a.clone();
        b.push(unit("two", Some((10, 20)), ""));
        let g = agreement(&[a, b]);
        assert_eq!((g.comparable, g.agreed), (2, 1));
        assert_eq!(g.rate(), Some(0.5));
    }

    /// Same place, different wording: `identical-span` includes the gist, so this is a
    /// disagreement and not a match. The broader reading needs a judge (§3.4).
    #[test]
    fn one_span_with_two_wordings_is_two_keys() {
        let a = vec![unit("the pool waited", Some((0, 10)), "")];
        let b = vec![unit("the pool was waiting", Some((0, 10)), "")];
        let g = agreement(&[a, b]);
        assert_eq!((g.comparable, g.agreed), (2, 0));
        assert_eq!(g.rate(), Some(0.0));
        // And the decomposition: it is *one place*, found by both passes. This pair is the
        // whole reason `place_rate` exists — `rate` reports 0.0 for two passes that disagreed
        // about nothing but a verb tense, and over a live corpus that is most of the figure.
        assert_eq!((g.comparable_places, g.agreed_places), (1, 1));
        assert_eq!(g.place_rate(), Some(1.0));
    }

    /// Two passes that found different bytes disagree on **both** figures.
    ///
    /// The control the pair above needs: `place_rate` must not be 1.0 by construction, or it
    /// would report perfect agreement about the finding no matter what was found.
    #[test]
    fn two_passes_that_found_different_places_disagree_on_the_place_too() {
        let a = vec![unit("the pool waited", Some((0, 10)), "")];
        let b = vec![unit("the pool waited", Some((20, 30)), "")];
        let g = agreement(&[a, b]);
        assert_eq!((g.comparable, g.agreed), (2, 0));
        assert_eq!((g.comparable_places, g.agreed_places), (2, 0));
        assert_eq!(g.place_rate(), Some(0.0));
    }

    /// Summing two windows carries **every** field, including the ones added last.
    #[test]
    fn merging_two_windows_carries_every_field() {
        let a = agreement(&[
            vec![unit("the pool waited", Some((0, 10)), "")],
            vec![unit("the pool waited", Some((0, 10)), "")],
        ]);
        let b = agreement(&[
            vec![unit("one", Some((20, 30)), ""), unit("x", None, "")],
            vec![unit("two", Some((20, 30)), "")],
        ]);
        let mut sum = Agreement::default();
        sum.merge(&a);
        sum.merge(&b);
        assert_eq!(sum.passes, 2);
        // The first window contributed one key agreed by both passes; the second contributed
        // two keys agreed by neither, because its passes worded one place differently.
        assert_eq!((sum.comparable, sum.agreed), (3, 1));
        // One place each, both agreed — which is the second window's whole point.
        assert_eq!((sum.comparable_places, sum.agreed_places), (2, 2));
        assert_eq!(sum.spanless, 1);
        // The pair the decomposition exists for, summed: both windows found their place twice,
        // and the second window disagreed about nothing but the wording.
        assert_eq!(sum.place_rate(), Some(1.0));
    }

    /// A spanless unit is outside both figures, not just the gist one.
    #[test]
    fn the_place_figure_is_none_when_nothing_carried_a_span() {
        let g = agreement(&[vec![unit("one", None, "")], vec![unit("one", None, "")]]);
        assert_eq!((g.comparable_places, g.agreed_places), (0, 0));
        assert_eq!(g.place_rate(), None);
    }

    /// And one pass reports no place figure either, for `rate`'s reason.
    #[test]
    fn a_single_pass_reports_no_place_figure_either() {
        let g = agreement(&[vec![unit("one", Some((0, 10)), "")]]);
        assert_eq!(g.place_rate(), None);
    }

    /// An empty comparison is not perfect stability.
    #[test]
    fn units_with_no_span_are_counted_and_not_scored() {
        let a = vec![unit("one", None, "")];
        let b = vec![unit("one", None, "")];
        let g = agreement(&[a, b]);
        assert_eq!((g.comparable, g.agreed, g.spanless), (0, 0, 2));
        assert_eq!(g.rate(), None);
    }

    #[test]
    fn a_single_pass_run_scores_nothing() {
        let g = agreement(&[vec![unit("one", Some((0, 10)), "")]]);
        assert_eq!(g.passes, 1);
        // One pass "agrees" with itself by construction — the count says so — and that is
        // exactly why `rate` refuses to divide: the figure would be 1.0 for a run that measured
        // no stability at all. Found by TX-P5 step 5, where the text path reports the agreement
        // as a field rather than as an `Option`.
        assert_eq!(g.agreed, 1);
        assert_eq!(g.rate(), None);
    }

    #[test]
    fn consensus_refuses_a_temperature_and_single_pass_does_not() {
        assert!(Passes::Consensus.admits(0.0).is_ok());
        assert!(Passes::Single.admits(0.7).is_ok());
        let e = Passes::Consensus.admits(0.7).expect_err("refused");
        assert!(e.contains("--single-pass"), "{e}");
        assert_eq!(Passes::TEXT, Passes::Consensus);
        assert_eq!((Passes::Single.count(), Passes::TEXT.count()), (1, 2));
    }
}
