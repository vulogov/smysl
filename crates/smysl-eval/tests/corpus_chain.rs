//! The SM-P15 gate's first clause: **the smysl arm runs over the corpus and its structural
//! metrics hold at every hop.**
//!
//! E3 and E4 are the ones that must hold rather than merely be reported. They are
//! structural, so a violation is a bug in the guarantee, not a bad result - and §28 asks
//! that they be measured anyway, because a guarantee that never binds is a statement about
//! the corpus rather than about the format.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use smysl_eval::{measure, run_smysl_arm, Arm, ChainOptions, Metric, Outcome};
use smysl_graph::Store;

fn corpus() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/corpus");
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("corpus directory")
        .map(|e| e.expect("corpus entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "smy"))
        .collect();
    out.sort();
    out
}

/// F6 is the adversarial fixture and is *supposed* to violate rule M, so it is excluded
/// from the clean-arm assertions and asserted separately below.
fn clean_corpus() -> Vec<PathBuf> {
    corpus()
        .into_iter()
        .filter(|p| !p.to_string_lossy().contains("F6-"))
        .collect()
}

fn store_of(path: &Path) -> Store {
    let src = std::fs::read_to_string(path).unwrap();
    let out = smysl_core::surface::parse_surface(&src).unwrap();
    Store::from_records(out.records)
}

fn value(ms: &[smysl_eval::Measurement], m: Metric) -> f64 {
    ms.iter()
        .find(|x| x.metric == m)
        .unwrap_or_else(|| panic!("{m:?} was not measured"))
        .outcome
        .value()
        .unwrap_or_else(|| panic!("{m:?} reported no value"))
}

/// The chain runs on every clean fixture, and rule R holds at every hop of every one.
#[test]
fn rule_r_holds_at_every_hop_of_every_fixture() {
    for f in clean_corpus() {
        let store = store_of(&f);
        let run = run_smysl_arm(&store, &ChainOptions::default());
        assert_eq!(run.arm, Arm::Smysl);

        for hop in &run.hops {
            assert_eq!(
                hop.rebuttals_honoured,
                hop.rebuttals_possible,
                "{}: hop {} shipped {} rebutted unit(s) without their rebuttal",
                f.display(),
                hop.index,
                hop.rebuttals_possible - hop.rebuttals_honoured
            );
        }
    }
}

/// E3 over the clean corpus: no rule M or T violation reaches a survivor.
#[test]
fn no_clean_fixture_corrupts_its_epistemics_across_five_hops() {
    for f in clean_corpus() {
        let store = store_of(&f);
        let run = run_smysl_arm(&store, &ChainOptions::default());
        let ms = measure(&store, &run);
        assert_eq!(
            value(&ms, Metric::EpistemicCorruption),
            0.0,
            "{}: rule M/T violation among survivors",
            f.display()
        );
    }
}

/// The guarantee has to *bind* somewhere or the corpus is not exercising it. At least one
/// fixture must carry a rebuttal through a hop, otherwise E4 is 1.0 by vacuity.
#[test]
fn rule_r_is_not_vacuous_across_the_corpus() {
    let bound: usize = clean_corpus()
        .iter()
        .map(|f| {
            let store = store_of(f);
            let run = run_smysl_arm(&store, &ChainOptions::default());
            run.hops.iter().map(|h| h.rebuttals_possible).sum::<usize>()
        })
        .sum();
    assert!(
        bound > 0,
        "no fixture carried a rebutted unit through any hop; E4 would be vacuously 1.0"
    );
}

/// **The guard that matters.** A budget that does not bind produces E1 = 1.0 and E2 = 1.0
/// on every input. Those are exactly the numbers a harness measuring nothing produces, and
/// from the outside they are indistinguishable from a spectacular result. The first version
/// of this crate defaulted to an absolute 400 tokens, which was larger than every fixture,
/// so the whole chain was a no-op and every metric read perfect.
#[test]
fn the_default_budget_actually_binds() {
    let mut bound = 0usize;
    for f in clean_corpus() {
        let store = store_of(&f);
        let run = run_smysl_arm(&store, &ChainOptions::default());
        let ms = measure(&store, &run);
        let e1 = value(&ms, Metric::TokenCost);
        assert!(
            e1 < 1.0,
            "{}: E1 = {e1}, so the budget never bound and the metrics are vacuous",
            f.display()
        );
        bound += 1;
    }
    assert!(bound > 0, "no fixture was measured at all");
}

/// The regime the format is built for: shed detail, keep the claim. A chain that hit the
/// same E1 by throwing units away would be prose summarisation with extra steps.
///
/// The condition is arithmetic rather than a tuned threshold. Every unit can be carried at
/// `L0` for the cost of its gist, so when the gists all fit inside the budget, nothing
/// *has* to be dropped and a drop means something other than arithmetic decided it. When
/// they do not fit, units must go and no format can prevent it - F5 is that case, being
/// twelve mostly gist-only `data` and `artifact-ref` units with little detail to trade.
#[test]
fn units_are_kept_whenever_their_gists_fit_the_budget() {
    use smysl_eval::chain::{floor_tokens, full_tokens, Budget};
    use smysl_pack::Estimator;

    let est = Estimator::default();
    let mut exercised = 0usize;
    for f in clean_corpus() {
        let store = store_of(&f);
        let run = run_smysl_arm(&store, &ChainOptions::default());
        let budget = Budget::Fraction(0.6).tokens(full_tokens(&store, &run.initial, &est));
        let floor = floor_tokens(&store, &run.initial, &est);
        if floor > budget {
            continue; // gists alone overrun the budget; dropping units is forced
        }
        exercised += 1;

        let ms = measure(&store, &run);
        assert_eq!(
            value(&ms, Metric::ClaimSurvival),
            1.0,
            "{}: gists fit in {budget} tokens (floor {floor}) yet units were dropped",
            f.display()
        );
    }
    assert!(
        exercised >= 3,
        "only {exercised} fixture(s) had room to shed detail; the test is near-vacuous"
    );
}

/// A chain is only a chain if it converges: once packing reaches a fixed point, later hops
/// must not keep shedding units. Compounding loss is the prose baseline's failure, and
/// reproducing it here would mean the pack is not idempotent on its own output.
#[test]
fn the_chain_reaches_a_fixed_point_rather_than_eroding() {
    for f in clean_corpus() {
        let store = store_of(&f);
        let run = run_smysl_arm(&store, &ChainOptions::default());
        let sizes: Vec<usize> = run.hops.iter().map(|h| h.surviving.len()).collect();
        for w in sizes.windows(2) {
            assert!(
                w[1] >= w[0],
                "{}: hop sizes {sizes:?} shrink after the first pack",
                f.display()
            );
        }
    }
}

/// Determinism, which is what makes E1 a cost rather than a price: same store, same
/// budget, same survivors. Rule D covers `pack`; this asserts it survives five of them.
#[test]
fn the_whole_chain_is_reproducible() {
    for f in clean_corpus() {
        let store = store_of(&f);
        let a = run_smysl_arm(&store, &ChainOptions::default());
        let b = run_smysl_arm(&store, &ChainOptions::default());
        assert_eq!(a, b, "{}: two identical runs differed", f.display());
    }
}

/// E8 and E9 need a model. They must be *present and marked*, not absent - a metric that
/// silently vanishes from a report is the failure this crate exists to detect elsewhere.
#[test]
fn model_dependent_metrics_are_reported_as_not_run() {
    let store = store_of(&corpus()[0]);
    let run = run_smysl_arm(&store, &ChainOptions::default());
    let ms = measure(&store, &run);

    let reported: BTreeSet<Metric> = ms.iter().map(|m| m.metric).collect();
    for m in Metric::ALL {
        assert!(reported.contains(m), "{m:?} is missing from the report");
    }
    for m in [Metric::IngestOverhead, Metric::ProviderConformance] {
        let entry = ms.iter().find(|x| x.metric == m).unwrap();
        assert!(
            matches!(entry.outcome, Outcome::NotRun(_)),
            "{m:?} claims a value without a provider"
        );
    }
}

/// The prose baseline is declared but not landed. This pins the honest reading: it needs a
/// model, so a run without one is not a comparison.
#[test]
fn the_prose_arm_is_declared_as_needing_a_model() {
    assert!(Arm::Prose.needs_model());
    assert!(!Arm::Smysl.needs_model());
}
