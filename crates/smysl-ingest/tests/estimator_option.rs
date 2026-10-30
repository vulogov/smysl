//! `ingest --estimator` (F-2): the bound the prompt states and the bound the units are checked
//! against must be the same bound.
//!
//! F-2 shipped the estimator as a capability nothing could reach: ingest chose among three
//! presets and all three counted bytes, so the 12.67% of Russian units S0 measured as destroyed
//! kept being destroyed. These are the two halves of making it reachable, and the trap between
//! them.

use smysl_core::{ProfileEstimator, TokenEstimator};
use smysl_ingest::{prompt, IngestOptions};

#[test]
fn the_default_is_unchanged_in_both_halves() {
    let o = IngestOptions::default();
    assert_eq!(o.estimator, TokenEstimator::Utf8Div4);
    assert!(
        !o.estimator_named,
        "an ingest that does not ask has not asked"
    );
    // Absent, not named: the default is what an absent field has always meant, so a default
    // ingest's profile is the preset it would have been before F-2 existed.
    assert_eq!(
        o.granularity_profile().unwrap().estimator,
        ProfileEstimator::Unset
    );
    assert_eq!(
        o.granularity_profile().unwrap(),
        smysl_core::GranularityProfile::preset("default").unwrap()
    );

    // The shipped sentence, byte for byte: changing it would change every recipe.
    let t = prompt::content_ingest_surface();
    assert_eq!(
        t.system,
        prompt::content_ingest_surface_with(TokenEstimator::Utf8Div4).system
    );
    assert!(
        t.system
            .contains("120 characters in Latin script, 60 in Cyrillic or Greek, 40 in "),
        "the default bound sentence moved"
    );
}

#[test]
fn naming_an_estimator_moves_the_profile_and_the_prompt_together() {
    let o = IngestOptions::default().with_estimator(TokenEstimator::Content1);
    assert!(o.estimator_named, "naming it has to be recorded");
    assert_eq!(
        o.granularity_profile().unwrap().estimator,
        ProfileEstimator::Known(TokenEstimator::Content1),
        "the units must be checked under the estimator that was asked for"
    );

    let t = prompt::content_ingest_surface_with(TokenEstimator::Content1);
    assert!(
        !t.system.contains("60 in Cyrillic or Greek"),
        "the prompt still states the byte bound: {}",
        t.system
    );
    // The version does not move with the estimator: which one produced the text is recorded in
    // the recipe, not in a version that would then mean two things.
    assert_eq!(t.version, prompt::content_ingest_surface().version);
    assert_eq!(t.id, prompt::content_ingest_surface().id);
}

/// Every number the prompt states must be one the check will actually accept.
///
/// A sentence that over-states the budget is worse than the vague one it replaced: the model
/// writes to the stated limit and the check then refuses it. So each stated figure is tested as
/// prose of that script, not as a pure run of one character class.
#[test]
fn every_stated_bound_is_one_the_check_accepts() {
    let est = TokenEstimator::Content1;
    let l0 = smysl_core::GranularityProfile::default().l0_max;
    let text = prompt::gist_bound_for(est).to_string();

    // Pull the four numbers back out of the sentence the model is given.
    let numbers: Vec<u32> = text
        .split_whitespace()
        .filter_map(|w| w.trim_end_matches(',').parse::<u32>().ok())
        .collect();
    assert_eq!(numbers.len(), 4, "four scripts are stated: {text}");

    // Prose of each script at exactly the stated length, four characters to a word.
    for (n, (word, label)) in numbers.iter().zip([
        ("abcd ", "Latin"),
        ("абвг ", "Cyrillic"),
        ("αβγδ ", "Greek"),
        ("漢字", "CJK"),
    ]) {
        let mut s = String::new();
        while s.chars().count() < *n as usize {
            s.push_str(word);
        }
        let s: String = s.chars().take(*n as usize).collect();
        assert_eq!(s.chars().count(), *n as usize);
        assert!(
            est.count(&s) <= l0,
            "{label}: the prompt states {n} characters, which counts {} against l0_max {l0}",
            est.count(&s)
        );
    }
}

/// The recipe has to distinguish two runs that were asked for different gist lengths.
#[test]
fn the_estimator_is_part_of_the_recipe() {
    use smysl_ingest::recipe::Conditions;
    let base = Conditions::new("ingest.content.surface", 6).with_provider("p", "m");
    let content = base.clone().with_estimator(TokenEstimator::Content1);

    assert_ne!(
        base.recipe(),
        content.recipe(),
        "two runs told different gist bounds are not one recipe"
    );
    assert_ne!(base.family(), content.family());

    // The default is recorded as absent, so every recipe computed before the field existed is
    // unchanged - the `normaliser` precedent.
    let default = base.clone().with_estimator(TokenEstimator::Utf8Div4);
    assert_eq!(default.estimator, None);
    assert_eq!(
        base.recipe(),
        default.recipe(),
        "naming the default must not move a recipe"
    );
}
