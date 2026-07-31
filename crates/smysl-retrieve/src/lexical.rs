//! BM25 over gists, bodies and details.
//!
//! The default implementation, and deliberately the boring one. It has a single transitive
//! dependency, needs no model, downloads nothing, runs offline, and returns the same ranking
//! on every machine. Whether it is *good enough* is an empirical question about the content
//! a given pipeline carries — which is why the seam exists and why this ships first: the
//! measurement is worth more than the guess it would replace.

use std::collections::BTreeMap;

use bm25::{Embedder, EmbedderBuilder, Scorer, Tokenizer};

use smysl_core::{KernelType, Status, Uid};
use smysl_graph::Store;

use crate::{candidates, indexable, Hit, Query, Retriever};

/// Our tokeniser, wrapping [`crate::tokenize`] for `bm25`.
#[derive(Clone, Default)]
struct SmyslTokenizer;

impl Tokenizer for SmyslTokenizer {
    fn tokenize(&self, input: &str) -> Vec<String> {
        crate::tokenize::tokenize(input)
    }
}

/// A lexical index over a store.
pub struct Bm25 {
    embedder: Embedder<u32, SmyslTokenizer>,
    scorer: Scorer<Uid>,
    /// Kernel type and status per indexed uid, for the query filters.
    facts: BTreeMap<Uid, (KernelType, Status)>,
}

/// How many times each part is repeated in the indexed text.
///
/// BM25 has no per-field weighting, so weight is expressed as repetition — a term in the
/// gist counts four times, in the body twice, in the detail once. Repetition rather than a
/// post-hoc score multiplier because it also feeds the length normalisation, which is the
/// part of BM25 that decides a short precise gist beats a long rambling body.
fn repeats(weight: f32) -> usize {
    match weight {
        w if w >= 1.0 => 4,
        w if w >= 0.5 => 2,
        _ => 1,
    }
}

fn document_text(store: &Store, uid: &Uid) -> Option<String> {
    let parts = indexable(store, uid)?;
    let mut out = String::new();
    for (text, weight) in parts {
        for _ in 0..repeats(weight) {
            out.push_str(&text);
            out.push(' ');
        }
    }
    Some(out)
}

impl Bm25 {
    /// Build an index over every unit in `store`.
    ///
    /// Fitting is over the same corpus that is then scored, which is what makes the inverse
    /// document frequencies meaningful: a term common in *this* store is uninformative in
    /// this store, whatever it is worth elsewhere.
    pub fn index(store: &Store) -> Bm25 {
        let facts = candidates(store);

        // Deterministic order: `facts` is a BTreeMap, so the corpus is built in uid order on
        // every machine. BM25 statistics do not depend on order, but reproducibility should
        // not rest on that being true of an implementation we do not own.
        let docs: Vec<(Uid, String)> = facts
            .keys()
            .filter_map(|u| document_text(store, u).map(|t| (*u, t)))
            .collect();

        let corpus: Vec<&str> = docs.iter().map(|(_, t)| t.as_str()).collect();
        let embedder: Embedder<u32, SmyslTokenizer> =
            EmbedderBuilder::with_tokenizer_and_fit_to_corpus(SmyslTokenizer, &corpus).build();

        let mut scorer = Scorer::<Uid>::new();
        for (uid, text) in &docs {
            scorer.upsert(uid, embedder.embed(text));
        }

        Bm25 {
            embedder,
            scorer,
            facts,
        }
    }
}

impl Retriever for Bm25 {
    fn search(&self, query: &Query) -> Vec<Hit> {
        if query.limit == 0 {
            return Vec::new();
        }
        let q = self.embedder.embed(&query.text);

        let mut hits: Vec<Hit> = self
            .facts
            .iter()
            .filter(|(_, (kind, status))| query.admits(*kind, *status))
            .filter_map(|(uid, _)| {
                // `None` means the uid was never indexed; a zero score means no query term
                // occurs in it. Both are excluded, because returning them pads the result to
                // `limit` with units that do not match, which reads as a ranking and is not
                // one.
                let score = self.scorer.score(uid, &q)?;
                (score > 0.0).then_some(Hit { uid: *uid, score })
            })
            .collect();

        // Descending by score, then by uid. The tiebreak is what makes this a function
        // rather than a preference: equal scores are common in a small store, and a sort
        // that leaves their order to the input would make retrieval depend on record order.
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.uid.cmp(&b.uid))
        });
        hits.truncate(query.limit);
        hits
    }

    fn len(&self) -> usize {
        self.facts.len()
    }
}
