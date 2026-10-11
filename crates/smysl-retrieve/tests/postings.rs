//! The postings seam against the path it replaces (TX-P4 step 2).
//!
//! # What is being asserted, and why in this form
//!
//! The step's exit clause is two claims: that `find` gets faster, and that it returns the same
//! thing. The first is a measurement and lives in `tests/postings_timing.rs`, which is
//! `#[ignore]`d because a timing assertion in CI is a flake generator. The second is this file,
//! and it is asserted as **equality of the whole result** — uid, score and per-term
//! decomposition — rather than as agreement about the top hit or the ordering.
//!
//! That is a deliberately brittle test and the brittleness is the point. A scorer that agreed
//! about the ranking and disagreed in the fourth decimal place would pass a looser test and
//! would still have changed every `--why` line the tool has ever printed, and `doc-output`
//! compares those. Scores are compared with `==` on `f32`: the arithmetic is reproduced
//! operation for operation, so anything other than exact equality is a divergence to be found
//! now rather than a tolerance to be tuned later.
//!
//! The fixture is a surface document rather than a builder loop because the filters are half of
//! what is being compared: an extension schema with a payload field, four statuses, bodies and
//! details at their three weights. A corpus of five paraphrases would agree under any scorer.

use smysl_core::surface::parse_surface;
use smysl_core::{Label, Uid};
use smysl_graph::Store;
use smysl_retrieve::{analyze, Analyze, Bm25, MemPostings, Postings, Query, Retriever, Tokenizer};

const SRC: &str = "\
@schema x.code/v1 { version: 1 }

@observation o/latency { status: measured, source: { kind: metric, ref: \"m/1\" } }
~ p95 checkout latency tripled after the 4.3 serialiser release

Measured over a one-minute window across eu-west, where the connection pool saturated.
The window is the one the alert fired on.

--
Required reading before the retrospective, which the window also covers.

@claim c/pool { status: speculative }
~ the eu-west connection_pool_size is saturated and requires raising

Sustained queue depth suggests the pool is the binding constraint.

@question q/why { status: speculative }
~ why did checkout latency regress after 4.3?

@artifact-ref a/code { status: cited, source: { kind: file, ref: \"PoolManager.rs\" } }
~ serialiser change in PoolManager.acquire introduced an n-plus-one

See PoolManager::acquire and its call in CheckoutService.

@prose p/season { status: speculative }
~ a note about unrelated seasonal traffic patterns in the reporting pipeline

@decision d/pin { status: speculative, \"code:kind\": \"decision\" }
~ Pin the connection pool size rather than tracking the range.

@claim r/raise { status: speculative, \"code:kind\": \"rejected-alternative\" }
~ Raise the connection pool ceiling instead, requiring no drain.
";

fn corpus() -> (Store, Vec<(&'static str, Uid)>) {
    let out = parse_surface(SRC).expect("the fixture parses");
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let uid = |l: &str| out.labels[&Label::new(l).expect("a label")];
    let named = vec![
        ("o/latency", uid("o/latency")),
        ("c/pool", uid("c/pool")),
        ("q/why", uid("q/why")),
        ("a/code", uid("a/code")),
        ("p/season", uid("p/season")),
        ("d/pin", uid("d/pin")),
        ("r/raise", uid("r/raise")),
    ];
    (Store::from_records(out.records), named)
}

fn by(named: &[(&str, Uid)], n: &str) -> Uid {
    named.iter().find(|(l, _)| *l == n).expect(n).1
}

/// Every query shape the two paths have to agree about.
///
/// Filters included, and not as an afterthought: the postings path reads a unit's schema, status
/// and payload from the unit rather than from a table built beside the index, which is a
/// different code path for the same decision and exactly where a divergence would hide.
fn queries(named: &[(&str, Uid)]) -> Vec<Query> {
    vec![
        Query::new("checkout latency", 10),
        // A repeated term: the score sums over the query's tokens with repetition, so this is
        // the query that catches a scorer which deduplicated them.
        Query::new("pool pool pool", 10),
        Query::new("connection_pool_size", 10),
        Query::new("PoolManager.acquire", 5),
        // Nothing in the corpus holds either term.
        Query::new("kangaroo photosynthesis", 5),
        // A limit smaller than the number of matches, so the truncation is exercised.
        Query::new("the pool latency serialiser", 2),
        Query::new("latency", 10).min_status(smysl_core::Status::Cited),
        Query::new("checkout latency", 10).kinds([smysl_core::KernelType::Question]),
        Query::new("connection pool", 10).with_payload("code:kind", ["decision"]),
        Query::new("connection pool", 10).within([by(named, "c/pool"), by(named, "r/raise")]),
        Query::new("connection pool", 10)
            .schemas([smysl_core::SchemaId::parse("x.code/v1").expect("a schema id")]),
        // Zero means nothing, not everything.
        Query::new("latency", 0),
    ]
}

fn both(store: &Store, tok: Tokenizer) -> (Bm25, MemPostings) {
    let postings = MemPostings::index(store, &tok);
    (Bm25::index_with(store, tok), postings)
}

/// The exit clause's second half, at its strongest reading.
#[test]
fn the_two_paths_return_the_same_hits_bit_for_bit() {
    let (store, named) = corpus();
    for tok in [Tokenizer::plain(), Tokenizer::folding()] {
        let (index, postings) = both(&store, tok);
        let view = Bm25::over(&postings, &tok, &store);
        let mut matched = 0;
        for q in queries(&named) {
            let a = index.search(&q);
            let b = view.search(&q);
            assert_eq!(
                a,
                b,
                "the two paths disagreed about `{}` under {}",
                q.text,
                tok.id()
            );
            matched += a.len();
        }
        // Non-vacuity: a pair of scorers that both returned nothing would agree perfectly.
        assert!(
            matched > 10,
            "only {matched} hits across every query under {} — the fixture stopped matching",
            tok.id()
        );
    }
}

/// `len` is the corpus size on both paths, which is the failure that otherwise reads as
/// "no results".
#[test]
fn both_paths_report_the_same_corpus_size() {
    let (store, _) = corpus();
    let tok = Tokenizer::plain();
    let (index, postings) = both(&store, tok);
    assert_eq!(index.len(), 7);
    assert_eq!(Bm25::over(&postings, &tok, &store).len(), index.len());
}

/// The three table facts, asserted directly rather than through a score.
#[test]
fn the_table_counts_documents_and_occurrences_separately() {
    let (store, named) = corpus();
    let tok = Tokenizer::plain();
    let postings = MemPostings::index(&store, &tok);

    // `latency` is in two gists. `df` is a document count: a table that counted occurrences
    // would say eight, since each of the two repeats its gist four times.
    assert_eq!(postings.df("latency"), 2, "two units mention latency");
    let list = postings.postings("latency");
    assert_eq!(list.len(), 2);
    assert!(list.windows(2).all(|w| w[0].0 < w[1].0), "uid order");

    let latency = by(&named, "o/latency");
    let tf = list
        .iter()
        .find(|(u, _)| *u == latency)
        .expect("the observation is in the list")
        .1;
    assert_eq!(
        tf, 4,
        "a gist counts four times, which is how BM25 is told about weight"
    );

    // All three weights in one number: `window` occurs twice in the body and once in the
    // detail, so 2·2 + 1·1 = 5. A table built without the repetition would say three, and
    // every score over it would be a different number that still looked like a score.
    assert_eq!(
        postings
            .postings("window")
            .iter()
            .find(|(u, _)| *u == latency)
            .expect("the body and detail mention a window")
            .1,
        5,
        "body twice and detail once, over the two body occurrences and the one in the detail"
    );

    assert_eq!(postings.docs(), 7);
    assert!(postings.doc_len(&latency).expect("an indexed unit") > 0);
    assert_eq!(postings.df("kangaroo"), 0);
    assert!(postings.postings("kangaroo").is_empty());
}

/// `avgdl` is a corpus quantity and has to come from the table, not from a walk.
#[test]
fn the_average_length_is_the_total_over_the_count() {
    let (store, _) = corpus();
    let tok = Tokenizer::plain();
    let postings = MemPostings::index(&store, &tok);
    let sum: u64 = postings.lengths().map(|(_, l)| u64::from(l)).sum();
    assert_eq!(postings.total_len(), sum);
    assert_eq!(
        postings.avgdl(),
        (sum as f64 / postings.docs() as f64) as f32
    );
}

/// An empty corpus reaches the fallback rather than dividing by zero.
#[test]
fn an_empty_table_scores_nothing_and_divides_by_nothing() {
    let store = Store::new();
    let tok = Tokenizer::plain();
    let postings = MemPostings::index(&store, &tok);
    assert_eq!(postings.docs(), 0);
    assert_eq!(postings.avgdl(), smysl_retrieve::postings::FALLBACK_AVGDL);
    let view = Bm25::over(&postings, &tok, &store);
    assert!(view.search(&Query::new("latency", 5)).is_empty());
    assert!(view.is_empty());
}

/// Removing a document leaves the table the rebuild over the smaller corpus would have built.
///
/// The incremental half of the same gate step 1 set for the index: a table maintained by
/// removals and a table built fresh must be the same table, or the two paths disagree about the
/// corpus in a way that only shows up as a slightly wrong score.
#[test]
fn a_removal_leaves_what_a_rebuild_without_that_unit_builds() {
    let (store, named) = corpus();
    let tok = Tokenizer::plain();
    let dropped = by(&named, "p/season");

    let mut maintained = MemPostings::index(&store, &tok);
    maintained.remove(&dropped);

    let records: Vec<_> = store
        .iter()
        .filter(|r| match r {
            smysl_core::Record::Unit(c) => smysl_core::canonical_uid(c) != dropped,
            _ => true,
        })
        .cloned()
        .collect();
    let smaller = Store::from_records(records);
    let rebuilt = MemPostings::index(&smaller, &tok);

    assert_eq!(
        maintained, rebuilt,
        "a maintained table and a rebuilt one disagree about the same corpus"
    );
    assert_eq!(maintained.docs(), 6);
}

/// Indexing the same unit twice must not double its term frequencies.
#[test]
fn adding_a_unit_twice_indexes_it_once() {
    let (store, named) = corpus();
    let tok = Tokenizer::plain();
    let once = MemPostings::index(&store, &tok);

    let mut twice = MemPostings::index(&store, &tok);
    let uid = by(&named, "c/pool");
    let text = {
        let unit = store.get(&uid).expect("the unit");
        format!(
            "{} {} {} {} {} {}",
            unit.core.gist,
            unit.core.gist,
            unit.core.gist,
            unit.core.gist,
            unit.core.body.as_deref().unwrap_or_default(),
            unit.core.body.as_deref().unwrap_or_default()
        )
    };
    twice.add(uid, &tok.terms(&text));
    assert_eq!(
        twice.df("saturated"),
        once.df("saturated"),
        "a re-indexed unit was counted twice"
    );
    assert_eq!(twice.docs(), once.docs());
}

/// The analyzer id travels with the table, because a query split by another chain finds nothing
/// and says nothing about why.
#[test]
fn the_table_records_which_analyzer_produced_its_terms() {
    let (store, _) = corpus();
    let plain = Tokenizer::plain();
    let folding = Tokenizer::folding();
    let postings = MemPostings::index(&store, &plain);

    assert_eq!(postings.analyzer(), Some(analyze::ID_PLAIN));
    assert!(Bm25::over(&postings, &plain, &store).agrees());
    assert!(
        !Bm25::over(&postings, &folding, &store).agrees(),
        "a folding query over a plain table is a disagreement the id is there to catch"
    );
    assert_eq!(
        analyze::by_id(analyze::ID_FOLD_EN).map(|t| t.folds()),
        Some(true)
    );
    assert_eq!(analyze::by_id("smysl/an-ru/1"), None);
    assert_eq!(analyze::ids().count(), 2);
}

/// Rule D, for the new path: the same table and query give the same answer twice.
#[test]
fn the_view_is_a_function_of_the_table_and_the_query() {
    let (store, _) = corpus();
    let tok = Tokenizer::plain();
    let postings = MemPostings::index(&store, &tok);
    let q = Query::new("latency pool serialiser", 5);
    let a = Bm25::over(&postings, &tok, &store);
    let second = MemPostings::index(&store, &tok);
    let b = Bm25::over(&second, &tok, &store);
    assert_eq!(a.search(&q), b.search(&q), "two tables disagreed");
    assert_eq!(
        a.search(&q),
        a.search(&q),
        "one table disagreed with itself"
    );
}

/// Record order must not change a ranking on this path either.
#[test]
fn record_order_does_not_change_the_table() {
    let (store, _) = corpus();
    let tok = Tokenizer::plain();
    let mut reversed: Vec<smysl_core::Record> = store.iter().cloned().collect();
    reversed.reverse();
    let other = Store::from_records(reversed);
    assert_eq!(
        MemPostings::index(&store, &tok),
        MemPostings::index(&other, &tok),
        "reversing the records built a different table"
    );
}
