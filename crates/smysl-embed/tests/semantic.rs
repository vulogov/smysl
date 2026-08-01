//! Semantic retrieval, against a real model.
//!
//! ```text
//! SMYSL_EMBED_MODEL=/path/to/potion-base-8M cargo test -p smysl-embed
//! ```
//!
//! A model is three files and tens of megabytes, so it is not in this repository and these
//! tests skip without one — the same convention `SMYSL_OLLAMA` and `SMYSL_EVAL_LIVE` already
//! use. Set `SMYSL_EMBED_MODEL=required` alongside the path to turn a skip into a failure,
//! which is what a release check should do: a live test that quietly skips is a live test
//! nobody runs, and that is how a mapper stays broken without anyone noticing.

use smysl_core::{canonical_uid, KernelType, Record, Status, Uid, UnitCoreBuilder};
use smysl_embed::{Model, Semantic};
use smysl_graph::Store;
use smysl_retrieve::{Query, Retriever};

fn model() -> Option<Model> {
    let path = std::env::var("SMYSL_EMBED_MODEL").ok()?;
    if path == "required" {
        panic!("SMYSL_EMBED_MODEL=required needs a path as well");
    }
    match Model::from_dir(&path) {
        Ok(m) => Some(m),
        Err(e) => panic!("SMYSL_EMBED_MODEL is set to {path} and the model would not load: {e}"),
    }
}

/// Skip unless a model is configured, and say so rather than passing silently.
macro_rules! model_or_skip {
    () => {
        match model() {
            Some(m) => m,
            None => {
                eprintln!("skipped: set SMYSL_EMBED_MODEL to a model directory");
                return;
            }
        }
    };
}

fn fixture() -> (Store, Vec<(&'static str, Uid)>) {
    let mut records = Vec::new();
    let mut named = Vec::new();
    let mut add = |name: &'static str, kind: KernelType, gist: &str| {
        let core = UnitCoreBuilder::new(kind, gist, Status::Speculative)
            .build()
            .expect("fixture unit builds");
        named.push((name, canonical_uid(&core)));
        records.push(Record::Unit(core));
    };
    add(
        "pool",
        KernelType::Claim,
        "the eu-west connection pool is saturated",
    );
    add(
        "serialiser",
        KernelType::Claim,
        "the cart serialiser began issuing one query per line item",
    );
    add(
        "seasonal",
        KernelType::Prose,
        "a note about seasonal traffic patterns in the reporting pipeline",
    );
    (Store::from_records(records), named)
}

fn uid_of(named: &[(&str, Uid)], name: &str) -> Uid {
    named.iter().find(|(n, _)| *n == name).expect(name).1
}

#[test]
fn every_unit_is_embedded() {
    let m = model_or_skip!();
    let (store, named) = fixture();
    let idx = Semantic::index(&store, m);
    assert_eq!(
        idx.len(),
        named.len(),
        "an index holding nothing looks exactly like a query matching nothing"
    );
}

/// The case this crate exists for: a query that shares no words with the gist.
///
/// "database queried too often" and "issuing one query per line item" overlap on `query`
/// alone, and `pool` does not contain it at all. BM25 ranks by that overlap; the point of an
/// embedding is to rank by what the sentence means.
#[test]
fn a_paraphrase_finds_the_claim_it_paraphrases() {
    let m = model_or_skip!();
    let (store, named) = fixture();
    let idx = Semantic::index(&store, m);

    let hits = idx.search(&Query::new("the code asked the database too many times", 3));
    assert!(!hits.is_empty(), "no hits at all");
    assert_eq!(
        hits[0].uid,
        uid_of(&named, "serialiser"),
        "the paraphrase should rank the claim it paraphrases first"
    );
}

/// Determinism, within a build. See the crate docs for what is *not* claimed across builds.
#[test]
fn retrieval_is_reproducible() {
    let m = model_or_skip!();
    let (store, _) = fixture();
    let idx = Semantic::index(&store, m);
    let q = Query::new("connection backlog on the european shard", 3);
    assert_eq!(idx.search(&q), idx.search(&q));
}

#[test]
fn a_kind_filter_still_applies() {
    let m = model_or_skip!();
    let (store, named) = fixture();
    let idx = Semantic::index(&store, m);
    let hits = idx.search(&Query::new("traffic", 5).kinds([KernelType::Prose]));
    assert!(
        hits.iter().all(|h| h.uid == uid_of(&named, "seasonal")),
        "a kind filter returned something of another kind"
    );
}
