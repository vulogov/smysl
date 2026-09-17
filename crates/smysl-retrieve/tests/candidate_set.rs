//! R16 and R19 (1.5): retrieval restricted to a caller's candidates, and optional suffix folding.

use smysl_core::{canonical_uid, KernelType, Record, Status, Uid, UnitCoreBuilder};
use smysl_graph::Store;
use smysl_retrieve::{Bm25, Query, Retriever, Tokenizer};

fn claim(gist: &str) -> smysl_core::UnitCore {
    UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
        .build()
        .unwrap()
}

/// Several claims: `kinds` cannot tell them apart, which is why `within` exists.
fn fixture() -> (Store, Vec<Uid>) {
    let gists = [
        "the connection pool saturated under load",
        "the retry storm held every connection open",
        "the canary shard stayed clean throughout",
        "a rejected alternative: raise the pool size instead",
    ];
    let units: Vec<smysl_core::UnitCore> = gists.iter().map(|g| claim(g)).collect();
    let uids: Vec<Uid> = units.iter().map(canonical_uid).collect();
    (
        Store::from_records(units.into_iter().map(Record::Unit).collect()),
        uids,
    )
}

/// The restriction is applied before the limit, and it changes nothing else: the same query
/// filtered afterwards gives the same units in the same order.
#[test]
fn within_restricts_candidates_without_changing_the_ranking() {
    let (store, uids) = fixture();
    let index = Bm25::index(&store);
    let eligible: Vec<Uid> = vec![uids[1], uids[3]];

    let restricted = index.search(&Query::new("connection pool", 10).within(eligible.clone()));
    assert!(restricted.iter().all(|h| eligible.contains(&h.uid)));

    let filtered: Vec<Uid> = index
        .search(&Query::new("connection pool", 10))
        .into_iter()
        .filter(|h| eligible.contains(&h.uid))
        .map(|h| h.uid)
        .collect();
    assert_eq!(
        restricted.iter().map(|h| h.uid).collect::<Vec<_>>(),
        filtered,
        "restricting changed the order or the set"
    );

    // The scores are the index's, not a re-weighting: IDF still comes from the whole store.
    for hit in &restricted {
        let whole = index.search(&Query::new("connection pool", 10));
        let same = whole
            .iter()
            .find(|h| h.uid == hit.uid)
            .expect("still a hit");
        assert_eq!(hit.score, same.score);
    }
}

/// Restricting keeps eligible units that a plain `limit` would never reach.
#[test]
fn within_surfaces_what_a_limit_would_bury() {
    let (store, uids) = fixture();
    let index = Bm25::index(&store);
    let wanted = uids[3];

    let top_one = index.search(&Query::new("pool", 1));
    assert_ne!(
        top_one[0].uid, wanted,
        "the fixture does not show the problem"
    );

    let restricted = index.search(&Query::new("pool", 1).within([wanted]));
    assert_eq!(restricted.len(), 1);
    assert_eq!(restricted[0].uid, wanted);
}

#[test]
fn an_empty_within_is_no_restriction() {
    let (store, _) = fixture();
    let index = Bm25::index(&store);
    let q = Query::new("connection pool", 10);
    assert_eq!(index.search(&q), index.search(&q.clone().within([])));
}

/// R19: with folding, a query in one inflection retrieves a unit written in another; without it,
/// the index scores exactly as it did.
#[test]
fn folding_matches_inflections_and_is_opt_in() {
    let unit = claim("Seven commands require an argument, so clap rejects bare names");
    let uid = canonical_uid(&unit);
    let store = Store::from_records(vec![Record::Unit(unit)]);

    let plain = Bm25::index_with(&store, Tokenizer::plain());
    assert!(
        plain.search(&Query::new("required argument", 5)).is_empty()
            || plain.search(&Query::new("required argument", 5))[0].uid != uid
            || plain.search(&Query::new("required", 5)).is_empty(),
        "the fixture already matched without folding"
    );
    assert_eq!(
        plain.search(&Query::new("commands require", 5)),
        Bm25::index(&store).search(&Query::new("commands require", 5)),
        "plain is what index() does"
    );

    let folding = Bm25::index_with(&store, Tokenizer::folding());
    let hits = folding.search(&Query::new("required argument", 5));
    assert_eq!(hits.first().map(|h| h.uid), Some(uid), "{hits:?}");
}
