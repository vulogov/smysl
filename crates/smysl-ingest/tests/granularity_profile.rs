//! `--granularity` chooses the profile a staged batch is checked under (1.7).
//!
//! The preset was validated and then discarded for three releases: the name was refused if it was
//! not a preset, hashed into the recipe, and then thrown away, so a batch ingested at `coarse` was
//! checked against whatever the store's view declared. The flag did not do the thing its name
//! says. `l1_min`/`l1_max` is where that shows: `coarse` admits a body of 120-400 tokens and
//! `default` admits 40-120, so the same unit is clean under one and `SMY-W041` under the other.

use std::collections::BTreeMap;

use smysl_core::diag::Code;
use smysl_core::{AgentId, GranularityProfile, Hlc, KernelType, Rung, Status, UnitCoreBuilder};
use smysl_graph::Store;
use smysl_ingest::stage::{self, Attest};

/// A body of roughly 200 tokens: inside `coarse`'s range, above `default`'s.
fn long_body() -> String {
    let sentence = "The connection pool saturated under sustained load from the retry storm. ";
    sentence.repeat(11)
}

fn batch() -> Vec<smysl_core::UnitCore> {
    vec![UnitCoreBuilder::new(
        KernelType::Claim,
        "the eu-west connection pool saturated",
        Status::Speculative,
    )
    .body(long_body())
    .build()
    .unwrap()]
}

fn attest() -> Attest {
    let agent = AgentId::new("tool:rust-smysl").unwrap();
    Attest::new(agent.clone(), Rung::Document, Hlc::zero(agent))
}

fn staged_under(g: Option<GranularityProfile>) -> smysl_ingest::Staged {
    stage::prepare_under(
        &Store::new(),
        batch(),
        Vec::new(),
        BTreeMap::new(),
        Vec::new(),
        &attest(),
        g,
    )
}

fn has_w041(s: &smysl_ingest::Staged) -> bool {
    s.report.iter().any(|d| d.code == Code::W041)
}

/// The profile the caller names is the one the batch is checked under.
#[test]
fn the_named_profile_is_the_one_applied() {
    assert!(
        has_w041(&staged_under(Some(GranularityProfile::standard()))),
        "a 200-token body is outside `default`'s 40-120 range"
    );
    assert!(
        !has_w041(&staged_under(Some(GranularityProfile::coarse()))),
        "and inside `coarse`'s 120-400, so naming coarse must silence it"
    );
    assert!(
        has_w041(&staged_under(Some(GranularityProfile::fine()))),
        "`fine` is narrower still"
    );
}

/// Naming nothing keeps the behaviour every release before 1.7 had: the profile comes from the
/// store, not from a default this function invented.
#[test]
fn naming_nothing_leaves_the_store_to_decide() {
    let unnamed = staged_under(None);
    let store_default = staged_under(Some(GranularityProfile::standard()));
    assert_eq!(
        has_w041(&unnamed),
        has_w041(&store_default),
        "an empty store's profile is the standard one, so these must agree"
    );
}

/// `with_granularity` is what says the caller named one, and the string keeps its own meaning:
/// it is hashed into the recipe as written, so `standard` and `default` stay distinct there.
#[test]
fn naming_a_granularity_is_recorded_on_the_options() {
    let plain = smysl_ingest::IngestOptions::at_rung(Rung::Document);
    assert!(!plain.granularity_named);
    assert_eq!(plain.granularity, "standard");

    let named = plain.clone().with_granularity("coarse");
    assert!(named.granularity_named);
    assert_eq!(
        named.granularity_profile().unwrap(),
        GranularityProfile::coarse()
    );

    // Naming the default explicitly still counts as naming it: the caller said so.
    let explicit = plain.with_granularity("default");
    assert!(explicit.granularity_named);
}
