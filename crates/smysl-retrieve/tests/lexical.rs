//! What retrieval must do, over a store that looks like a real pipeline's.
//!
//! The fixture deliberately mixes payload kinds — a telemetry observation, a claim, a
//! question, a code artifact reference, prose — because that mixture is the thing the design
//! is answering. A test over five paraphrases of one sentence would pass whatever the
//! tokeniser did.

use smysl_core::{
    canonical_uid, KernelType, Record, SourceKind, SourceRef, Status, Uid, UnitCoreBuilder,
};
use smysl_graph::Store;
use smysl_retrieve::{Bm25, Query, Retriever};

/// A store shaped like something an AI pipeline would actually hand over.
fn fixture() -> (Store, Vec<(&'static str, Uid)>) {
    let mut records = Vec::new();
    let mut named = Vec::new();

    let mut add = |name: &'static str,
                   kind: KernelType,
                   status: Status,
                   gist: &str,
                   body: Option<&str>,
                   src: Option<SourceKind>| {
        let mut b = UnitCoreBuilder::new(kind, gist, status);
        if let Some(t) = body {
            b = b.body(t.to_string());
        }
        // `measured` and `cited` both require a source (`SMY-E032`), which is the format
        // insisting that a claim to have observed or quoted something say where.
        if let Some(kind) = src {
            b = b.source(SourceRef::new(kind, "m"));
        }
        let core = b.build().expect("fixture unit builds");
        let uid = canonical_uid(&core);
        records.push(Record::Unit(core));
        named.push((name, uid));
    };

    add(
        "latency",
        KernelType::Observation,
        Status::Measured,
        "p95 checkout latency tripled after the 4.3 release",
        Some("Measured over a one-minute window across eu-west."),
        Some(SourceKind::Metric),
    );
    add(
        "pool",
        KernelType::Claim,
        Status::Speculative,
        "the eu-west connection_pool_size is saturated",
        Some("Sustained queue depth suggests the pool is the binding constraint."),
        None,
    );
    add(
        "question",
        KernelType::Question,
        Status::Speculative,
        "why did checkout latency regress after 4.3?",
        None,
        None,
    );
    add(
        "code",
        KernelType::ArtifactRef,
        Status::Cited,
        "serialiser change in PoolManager.acquire introduced an n-plus-one",
        Some("See PoolManager::acquire and its call in CheckoutService."),
        Some(SourceKind::File),
    );
    add(
        "prose",
        KernelType::Prose,
        Status::Speculative,
        "a note about unrelated seasonal traffic patterns in the reporting pipeline",
        None,
        None,
    );

    (Store::from_records(records), named)
}

fn uid_of(named: &[(&str, Uid)], name: &str) -> Uid {
    named.iter().find(|(n, _)| *n == name).expect(name).1
}

#[test]
fn the_index_holds_every_unit() {
    let (store, named) = fixture();
    let idx = Bm25::index(&store);
    assert_eq!(
        idx.len(),
        named.len(),
        "an index that silently held nothing would look exactly like a query that matched \
         nothing"
    );
    assert!(!idx.is_empty());
}

#[test]
fn a_query_finds_the_unit_it_describes() {
    let (store, named) = fixture();
    let idx = Bm25::index(&store);
    let hits = idx.search(&Query::new("checkout latency regression", 3));
    assert!(!hits.is_empty(), "no hits at all");
    let top = hits[0].uid;
    assert!(
        top == uid_of(&named, "latency") || top == uid_of(&named, "question"),
        "expected the latency observation or the question about it, got {top}"
    );
}

/// The identifier case, which is why the tokeniser does not stem.
#[test]
fn an_identifier_is_findable_by_its_parts_and_whole() {
    let (store, named) = fixture();
    let idx = Bm25::index(&store);
    let pool = uid_of(&named, "pool");

    for q in ["connection_pool_size", "pool", "PoolManager"] {
        let hits = idx.search(&Query::new(q, 5));
        assert!(
            !hits.is_empty(),
            "`{q}` matched nothing; a stemming tokeniser is the usual cause"
        );
    }
    let whole = idx.search(&Query::new("connection_pool_size", 5));
    assert_eq!(
        whole[0].uid, pool,
        "the exact identifier should rank its own unit first"
    );
}

/// Filtering by kernel type is the feature the heterogeneity argument rests on.
#[test]
fn a_kind_filter_restricts_the_result_to_that_kind() {
    let (store, named) = fixture();
    let idx = Bm25::index(&store);

    let hits = idx.search(&Query::new("checkout latency", 5).kinds([KernelType::Question]));
    assert!(!hits.is_empty(), "the question mentions checkout latency");
    assert!(
        hits.iter().all(|h| h.uid == uid_of(&named, "question")),
        "a kind filter returned something of another kind"
    );
}

#[test]
fn a_status_floor_excludes_everything_below_it() {
    let (store, named) = fixture();
    let idx = Bm25::index(&store);

    let hits = idx.search(&Query::new("latency", 5).min_status(Status::Cited));
    assert!(
        hits.iter().all(|h| h.uid != uid_of(&named, "question")),
        "a speculative unit survived a `cited` floor"
    );
    assert!(
        hits.iter().any(|h| h.uid == uid_of(&named, "latency")),
        "the measured observation should still be there"
    );
}

/// Nothing that does not match is returned, even when `limit` leaves room.
#[test]
fn a_limit_is_not_padded_with_non_matches() {
    let (store, _) = fixture();
    let idx = Bm25::index(&store);
    let hits = idx.search(&Query::new("connection_pool_size", 50));
    assert!(
        hits.len() < idx.len(),
        "every unit scored for a term that occurs in one of them"
    );
    assert!(hits.iter().all(|h| h.score > 0.0));
}

#[test]
fn a_query_matching_nothing_returns_nothing() {
    let (store, _) = fixture();
    let idx = Bm25::index(&store);
    assert!(idx
        .search(&Query::new("kangaroo photosynthesis", 5))
        .is_empty());
}

/// Rule D reaches this crate: retrieval is a function of the store and the query.
#[test]
fn retrieval_is_reproducible() {
    let (store, _) = fixture();
    let a = Bm25::index(&store);
    let b = Bm25::index(&store);
    let q = Query::new("latency pool serialiser", 5);
    assert_eq!(a.search(&q), b.search(&q), "two indexes disagreed");
    assert_eq!(
        a.search(&q),
        a.search(&q),
        "one index disagreed with itself"
    );
}

/// Record order must not change a ranking, for the same reason it must not change a pack.
#[test]
fn record_order_does_not_change_the_ranking() {
    let (store, _) = fixture();
    let mut reversed: Vec<Record> = store.iter().cloned().collect();
    reversed.reverse();
    let other = Store::from_records(reversed);

    let q = Query::new("checkout latency pool", 5);
    assert_eq!(
        Bm25::index(&store).search(&q),
        Bm25::index(&other).search(&q),
        "reversing the records changed the ranking"
    );
}

#[test]
fn a_zero_limit_returns_nothing_rather_than_everything() {
    let (store, _) = fixture();
    assert!(Bm25::index(&store)
        .search(&Query::new("latency", 0))
        .is_empty());
}
