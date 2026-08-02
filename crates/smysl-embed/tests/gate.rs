//! The semantic path, exercised in CI without a downloaded model.
//!
//! `tests/semantic.rs` and `make eval-semantic` need a real model and a key to the internet
//! that produced it, so neither runs here. That left the whole `Semantic` code path — loading
//! a model, embedding a store, ranking a query — checked only by hand, which is the same
//! shape as a fuzz target nobody runs. This project has been bitten by that twice: two stack
//! overflows survived to 0.3 in targets that existed and were never run, and `pack_exact` sat
//! unwired for a cycle.
//!
//! So `fixtures/embed-tiny` is a **synthetic** Model2Vec model: a fifty-word vocabulary drawn
//! from the corpus's own language and sixteen-dimensional vectors derived from a fixed hash
//! of each word. 4 KB, generated rather than trained, and committed.
//!
//! It is emphatically **not** a quality benchmark — the vectors mean nothing, so two words
//! are near each other because their letters are, not because they are related. What it
//! proves is that the machinery runs end to end and stays deterministic. Quality is
//! `make eval-semantic`'s job, against a real model, and the numbers live in the changelog.

use smysl_core::{canonical_uid, KernelType, Record, Status, UnitCoreBuilder};
use smysl_embed::{Hybrid, Model, Semantic};
use smysl_graph::Store;
use smysl_retrieve::{Bm25, Query, Retriever};

const TINY: &str = "../../fixtures/embed-tiny";

fn store() -> Store {
    let mut records = Vec::new();
    for gist in [
        "the connection pool is saturated in eu west",
        "checkout latency tripled after the release",
        "a claim about seasonal traffic in the reporting pipeline",
    ] {
        let core = UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
            .build()
            .expect("builds");
        let _ = canonical_uid(&core);
        records.push(Record::Unit(core));
    }
    Store::from_records(records)
}

#[test]
fn the_model_loads_from_a_directory() {
    Model::from_dir(TINY).expect("the committed fixture model loads");
}

#[test]
fn a_missing_directory_is_an_error_rather_than_a_download() {
    // The failure mode worth pinning: `hf-hub` is compiled out, so a path that is not a
    // directory must fail here rather than quietly fetching something.
    let Err(e) = Model::from_dir("../../fixtures/there-is-no-such-model") else {
        panic!("a path that is not a directory must be an error, not a download");
    };
    assert!(
        e.to_string().contains("not a directory"),
        "unexpected error: {e}"
    );
}

#[test]
fn every_unit_is_embedded_and_searchable() {
    let s = store();
    let idx = Semantic::index(&s, Model::from_dir(TINY).unwrap());
    assert_eq!(
        idx.len(),
        3,
        "an empty index looks like a query that matched nothing"
    );
    let hits = idx.search(&Query::new("connection pool saturated", 3));
    assert!(
        !hits.is_empty(),
        "the semantic path returned nothing at all"
    );
    assert!(hits.iter().all(|h| h.score > 0.0));
    assert!(hits.len() <= 3, "limit exceeded");
}

/// Rule D reaches as far as this crate can promise it: within a build, retrieval is a
/// function of the store, the query and the model.
#[test]
fn retrieval_is_reproducible() {
    let s = store();
    let a = Semantic::index(&s, Model::from_dir(TINY).unwrap());
    let b = Semantic::index(&s, Model::from_dir(TINY).unwrap());
    let q = Query::new("latency after the release", 3);
    assert_eq!(a.search(&q), b.search(&q), "two indexes disagreed");
    assert_eq!(
        a.search(&q),
        a.search(&q),
        "one index disagreed with itself"
    );
}

/// The routing is unit-tested with stubs; this checks it against the real pair, so a change
/// to either engine's construction cannot silently break the composition.
#[test]
fn the_hybrid_runs_against_both_real_engines() {
    let s = store();
    let h = Hybrid::new(
        Bm25::index(&s),
        Semantic::index(&s, Model::from_dir(TINY).unwrap()),
    );
    // Identifier-shaped: lexical answers, and finds nothing here, which is a legitimate
    // answer rather than an error.
    let _ = h.search(&Query::new("pool.wait_ms", 3));
    // Prose: the embedder answers, and must return something.
    let hits = h.search(&Query::new("the pool is saturated", 3));
    assert!(!hits.is_empty(), "the hybrid returned nothing for prose");
}
