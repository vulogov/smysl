//! The granularity bounds are counted with the profile's own estimator (SMYSL-2.1 F-2).
//!
//! Two obligations. The default count is what it always was, so no existing store starts
//! reporting differently; and a profile naming an estimator this build does not have leaves
//! `l0_max` **unevaluable** — `SMY-W025` — rather than evaluable under some other count, which
//! would report a breach of a bound nobody set.

use std::collections::BTreeMap;

use smysl_check::{check, CheckOptions};
use smysl_core::{
    Code, GranularityProfile, KernelType, ProfileEstimator, Record, Severity, Status,
    TokenEstimator, UnitCoreBuilder, View, ViewId,
};
use smysl_graph::Store;

fn store_with(profile: GranularityProfile, gists: &[&str]) -> Store {
    let mut records = vec![Record::View(
        View::new(ViewId::new("v/e").unwrap(), "i").with_granularity(profile),
    )];
    for (i, g) in gists.iter().enumerate() {
        let core = UnitCoreBuilder::new(KernelType::Claim, (*g).to_string(), Status::Speculative)
            .build()
            .unwrap();
        let _ = i;
        records.push(Record::Unit(core));
    }
    Store::from_records(records)
}

/// A Russian gist that `utf8-div4` refuses and `content/1` accepts.
///
/// 108 characters: 198 bytes, so 50 tokens under `utf8-div4` against an `l0_max` of 30, and it
/// degrades. Under `content/1` it is inside the bound, which is the whole point — the
/// proposition was never too long, the count was unfair to the script stating it.
const RU: &str = "Синод постановил, что переписка между двумя приходами сохраняется в архиве \
                  до конца следующего года.";

#[test]
fn the_default_count_is_unchanged() {
    let p = GranularityProfile::standard();
    assert_eq!(
        p.estimator.estimator(),
        Some(TokenEstimator::Utf8Div4),
        "the default must stay the pre-F-2 count"
    );
    assert_eq!(p.tokens(RU), Some(smysl_core::tokens(RU)));

    let report = check(&store_with(p, &[RU]), CheckOptions::default());
    assert_eq!(
        report.count(Code::E022),
        1,
        "under the default count this gist is still over the bound"
    );
    assert_eq!(report.count(Code::W025), 0);
}

#[test]
fn content1_admits_a_gist_the_byte_count_refused() {
    let mut p = GranularityProfile::standard();
    p.estimator = ProfileEstimator::Known(TokenEstimator::Content1);
    let report = check(&store_with(p, &[RU]), CheckOptions::default());
    assert_eq!(
        report.count(Code::E022),
        0,
        "content/1 counts this proposition inside l0_max"
    );
    assert_eq!(report.count(Code::W025), 0);
}

/// `check --estimator` overrides whatever the store's view says.
#[test]
fn the_override_replaces_the_profiles_own_estimator() {
    let store = store_with(GranularityProfile::standard(), &[RU]);
    let mut opts = CheckOptions::default();
    opts.estimator = Some(ProfileEstimator::Known(TokenEstimator::Content1));
    let report = check(&store, opts);
    assert_eq!(report.count(Code::E022), 0, "the override was not applied");
}

#[test]
fn an_unknown_estimator_makes_the_bound_unevaluable() {
    let mut p = GranularityProfile::standard();
    p.estimator = ProfileEstimator::Unknown("smysl/from-the-future/9".into());
    let report = check(&store_with(p, &[RU]), CheckOptions::default());

    assert_eq!(
        report.count(Code::E022),
        0,
        "a bound that cannot be evaluated cannot be breached"
    );
    assert_eq!(report.count(Code::W025), 1, "and it has to be reported");
    let d = report
        .diagnostics
        .iter()
        .find(|d| d.code == Code::W025)
        .unwrap();
    assert!(
        d.message.contains("smysl/from-the-future/9"),
        "the message must name the id: {}",
        d.message
    );
    assert_eq!(Code::W025.severity(), Severity::Warn);
    // A warning, so a store carrying one is still usable.
    assert!(report.fail_on(Severity::Error).is_ok());
}

/// Every corpus fixture reports the same diagnostics under the default as it did before F-2.
///
/// F-2 asks for this directly: the field is additive and the default is the old count, so a
/// store written before any of this existed must check byte for byte identically. Comparing the
/// default against an explicit `Utf8Div4` is the same statement from the other side — if the
/// dispatch were wrong in either, these would diverge.
#[test]
fn the_corpus_checks_identically_under_the_default() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/corpus");
    let mut seen = 0;
    for entry in std::fs::read_dir(dir).expect("fixtures/corpus") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("smy") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let out = smysl_core::surface::parse_surface(&text).expect("fixture parses");
        let store = Store::from_records(out.records.clone());

        let tally = |opts: CheckOptions| -> BTreeMap<String, usize> {
            let mut m = BTreeMap::new();
            for d in check(&store, opts).diagnostics {
                *m.entry(d.code.as_str().to_string()).or_insert(0) += 1;
            }
            m
        };

        let implicit = tally(CheckOptions::default());
        let mut explicit = CheckOptions::default();
        explicit.estimator = Some(ProfileEstimator::Known(TokenEstimator::Utf8Div4));
        assert_eq!(
            implicit,
            tally(explicit),
            "{} checks differently once the default count is named",
            path.display()
        );
        seen += 1;
    }
    assert!(seen >= 13, "expected the corpus, found {seen} documents");
}
