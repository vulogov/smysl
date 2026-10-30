//! R22 (1.6): which query terms a hit matched.
//!
//! A score says how relevant, not why. A prerequisite saying `require` was never retrieved for
//! five diffs saying `required`, and nothing in the result distinguished "retrieved weakly" from
//! "not retrieved at all" — the cause was found by printing packs and reading them. The fold
//! fixes that case; the blindness is what this removes.

use smysl_core::{canonical_uid, KernelType, Record, Status, Uid, UnitCoreBuilder};
use smysl_graph::Store;
use smysl_retrieve::{Bm25, Query, Retriever, Tokenizer};

fn claim(gist: &str) -> smysl_core::UnitCore {
    UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
        .build()
        .unwrap()
}

fn fixture() -> (Store, Vec<Uid>) {
    let gists = [
        "seven commands require an argument before the router runs",
        "the connection pool saturated under sustained load",
        "the canary shard stayed clean throughout the rollout",
    ];
    let units: Vec<smysl_core::UnitCore> = gists.iter().map(|g| claim(g)).collect();
    let uids: Vec<Uid> = units.iter().map(canonical_uid).collect();
    (
        Store::from_records(units.into_iter().map(Record::Unit).collect()),
        uids,
    )
}

/// The contributions are the score, decomposed: they sum to it, exactly.
#[test]
fn the_contributions_sum_to_the_score() {
    let (store, _) = fixture();
    for index in [
        Bm25::index(&store),
        Bm25::index_with(&store, Tokenizer::folding()),
    ] {
        for text in [
            "connection pool",
            "required argument",
            "pool pool pool",
            "the canary shard rollout",
        ] {
            for hit in index.search(&Query::new(text, 10)) {
                assert!(!hit.terms.is_empty(), "{text}: a hit matched nothing?");
                let sum: f32 = hit.terms.iter().map(|(_, c)| c).sum();
                assert!(
                    (sum - hit.score).abs() < 1e-4,
                    "{text}: terms sum to {sum}, score is {}: {:?}",
                    hit.score,
                    hit.terms
                );
            }
        }
    }
}

/// Only terms that contributed are listed, ordered by contribution then term.
#[test]
fn only_matching_terms_are_listed_in_a_total_order() {
    let (store, uids) = fixture();
    let index = Bm25::index(&store);

    let hits = index.search(&Query::new("connection pool aardvark", 10));
    let hit = hits.iter().find(|h| h.uid == uids[1]).expect("it matches");
    let terms: Vec<&str> = hit.terms.iter().map(|(t, _)| t.as_str()).collect();
    assert!(terms.contains(&"connection") && terms.contains(&"pool"));
    assert!(
        !terms.contains(&"aardvark"),
        "a term nothing matched is not an explanation: {terms:?}"
    );

    for pair in hit.terms.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        assert!(
            a.1 > b.1 || (a.1 == b.1 && a.0 <= b.0),
            "not ordered by contribution then term: {:?}",
            hit.terms
        );
    }

    // Deterministic: the same index and query give the same explanation.
    assert_eq!(
        index.search(&Query::new("connection pool aardvark", 10)),
        hits
    );
}

/// A query whose terms are all absent returns no hit, as it always did.
#[test]
fn a_query_that_matches_nothing_still_returns_nothing() {
    let (store, _) = fixture();
    let index = Bm25::index(&store);
    assert!(index
        .search(&Query::new("aardvark xylophone", 10))
        .is_empty());
}

/// The case that prompted this: with the fold on, a query saying `required` retrieves a unit
/// saying `require`, and the explanation names the folded term that did the work.
#[test]
fn the_explanation_names_the_folded_term_that_matched() {
    let (store, uids) = fixture();

    let plain = Bm25::index(&store);
    assert!(
        plain.search(&Query::new("required", 10)).is_empty(),
        "without the fold, `required` finds nothing — which is the blindness"
    );

    let folding = Bm25::index_with(&store, Tokenizer::folding());
    let hits = folding.search(&Query::new("required", 10));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].uid, uids[0]);
    // `required` and `require` meet at `requir` — the fold strips `ed` from one and the trailing
    // `e` from the other — and the explanation names the term that actually matched, not the one
    // the caller typed.
    let terms: Vec<&str> = hits[0].terms.iter().map(|(t, _)| t.as_str()).collect();
    assert!(
        terms.contains(&"requir"),
        "the fold did the work and should say so: {terms:?}"
    );
    let sum: f32 = hits[0].terms.iter().map(|(_, c)| c).sum();
    assert!((sum - hits[0].score).abs() < 1e-4);
}

/// A hit built by a retriever that does not decompose its score carries no terms, and that is
/// advisory rather than wrong.
#[test]
fn a_hit_carries_no_terms_unless_a_retriever_supplies_them() {
    use smysl_retrieve::Hit;

    let bare = Hit::new(Uid::from_bytes([1; 32]), 1.5);
    assert!(bare.terms.is_empty());
    let told = bare.clone().with_terms([("pool".to_string(), 1.5)]);
    assert_eq!(told.terms, vec![("pool".to_string(), 1.5)]);
    assert_eq!(told.score, bare.score);
}

/// F-4: the fold is English, and `folding_for` is how a caller says so.
///
/// `folding()` is documented as "common English suffixes" and gated on nothing, so a caller with
/// English prose in mind turned it on for a whole store, and a Spanish unit in that store was
/// folded by English rules with no diagnostic.
///
/// The RFC named `casas` for this test, and `casas` is the case that makes the weaker argument:
/// English `-s` strips to `casa`, which **is** the Spanish singular. The rule is right there by
/// coincidence, and a rule that is right by coincidence on the example chosen for it is the
/// thing worth distrusting. So the words that carry the claim are the ones where the
/// coincidence fails: `lunes` is Monday, not a plural, and folds to `lun`; `crisis` is singular
/// in both languages and folds to `crisi`. Neither matches anything a reader would type.
#[test]
fn folding_for_es_does_not_fold_casas() {
    use smysl_core::LangTag;

    let es = Tokenizer::folding_for(&LangTag::new("es").unwrap());
    for word in ["casas", "lunes", "crisis", "viernes"] {
        assert_eq!(
            es.terms(word),
            vec![word],
            "English suffix rules must not reach Spanish"
        );
    }

    // The same words under the ungated fold: what a caller got before F-4, and still gets if
    // they ask for `folding()` outright. Asserted so the difference is visible here rather than
    // inferred from the absence of a term above.
    assert_eq!(Tokenizer::folding().terms("casas"), vec!["casas", "casa"]);
    assert_eq!(Tokenizer::folding().terms("lunes"), vec!["lunes", "lun"]);
    assert_eq!(
        Tokenizer::folding().terms("crisis"),
        vec!["crisis", "crisi"]
    );
}

/// And English still folds, including through a region or an extension subtag.
///
/// The primary subtag decides. `en-GB` is English; so is `en-US-u-va-posix`. Case is ignored
/// because BCP 47 subtags are case-insensitive and `LangTag` does not normalise them.
#[test]
fn folding_for_en_matches_folding() {
    use smysl_core::LangTag;

    let plain = Tokenizer::folding();
    for tag in ["en", "en-GB", "EN", "en-US-u-va-posix"] {
        let gated = Tokenizer::folding_for(&LangTag::new(tag).unwrap());
        for text in ["required", "requires", "classes", "connection_pool_size"] {
            assert_eq!(
                gated.terms(text),
                plain.terms(text),
                "{tag} is English, so {text} must fold as it always did"
            );
        }
    }

    // A tag that is not English gets `plain()`, which is the other half of the same claim.
    let de = Tokenizer::folding_for(&LangTag::new("de").unwrap());
    assert_eq!(de.terms("required"), Tokenizer::plain().terms("required"));
}
