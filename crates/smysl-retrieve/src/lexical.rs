//! BM25 over gists, bodies and details.
//!
//! The default implementation, and deliberately the boring one. It has a single transitive
//! dependency, needs no model, downloads nothing, runs offline, and returns the same ranking
//! on every machine. Whether it is *good enough* is an empirical question about the content
//! a given pipeline carries — which is why the seam exists and why this ships first: the
//! measurement is worth more than the guess it would replace.

use std::collections::{BTreeMap, BTreeSet};

use bm25::{
    Embedder, EmbedderBuilder, Embedding, Scorer, TokenEmbedder, TokenEmbedding, Tokenizer,
};

use smysl_core::{SchemaId, Status, Uid};
use smysl_graph::Store;

use crate::{candidates, indexable, Hit, Query, Retriever};

/// Our tokeniser, wrapping [`crate::tokenize`] for `bm25`.
#[derive(Clone, Default)]
struct SmyslTokenizer(crate::tokenize::Tokenizer);

impl Tokenizer for SmyslTokenizer {
    fn tokenize(&self, input: &str) -> Vec<String> {
        self.0.terms(input)
    }
}

/// A lexical index over a store.
pub struct Bm25 {
    embedder: Embedder<u32, SmyslTokenizer>,
    scorer: Scorer<Uid>,
    /// Schema and status per indexed uid, for the query filters. Schema rather than kernel type
    /// since 1.7: a unit authored under an extension schema has no kernel type, and used to be
    /// left out of the index rather than filtered out of a result.
    facts: BTreeMap<Uid, (SchemaId, Status)>,
    /// The string-valued payload entries per indexed uid, for `Query::with_payload` (1.6). Built
    /// once here rather than decoded per query: the eligible set is a property of the corpus, and
    /// the caller was rebuilding it per query before this existed. Units with no payload hold an
    /// empty map, which admits nothing when a payload filter is set.
    payloads: BTreeMap<Uid, BTreeMap<String, BTreeSet<String>>>,
    /// The tokeniser this index was built with, kept so a query can be split into the same terms
    /// the documents were (1.6).
    tokenizer: crate::tokenize::Tokenizer,
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
        Bm25::index_with(store, crate::tokenize::Tokenizer::plain())
    }

    /// [`Bm25::index`] with a chosen tokeniser (1.5).
    ///
    /// `Tokenizer::folding()` folds common English suffixes, so a query saying `required` retrieves
    /// a unit saying `require`. Off by default: the fold helps prose and hurts identifiers, and
    /// turning it on moves every score in the index, so it is the caller's decision and not a
    /// silent improvement.
    pub fn index_with(store: &Store, tokenizer: crate::tokenize::Tokenizer) -> Bm25 {
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
            EmbedderBuilder::with_tokenizer_and_fit_to_corpus(SmyslTokenizer(tokenizer), &corpus)
                .build();

        let mut scorer = Scorer::<Uid>::new();
        for (uid, text) in &docs {
            scorer.upsert(uid, embedder.embed(text));
        }

        let payloads = facts
            .keys()
            .map(|u| {
                let strings = store
                    .get(u)
                    .and_then(|unit| unit.core.payload.as_deref())
                    .map(smysl_core::surface::payload::payload_strings)
                    .unwrap_or_default();
                (*u, strings)
            })
            .collect();

        Bm25 {
            embedder,
            scorer,
            facts,
            payloads,
            tokenizer,
        }
    }
}

impl Bm25 {
    /// What each query term contributed to this unit's score (R22, 1.6).
    ///
    /// Exact rather than attributed: BM25 sums `idf(t) · value(t, d)` over the *distinct* terms of
    /// the query, and the query's own term frequencies never enter the sum — so scoring a
    /// one-term query against the same document yields that term's summand exactly, and the
    /// summands add up to the score. Terms contributing nothing are left out: a list saying a
    /// unit matched `the` with 0.0 is noise in an explanation.
    ///
    /// Ordered by contribution descending, then by term ascending, which is total.
    fn contributions(&self, uid: &Uid, text: &str) -> Vec<(String, f32)> {
        // Per *index*, not per term: with folding on, a term and its folded form are two indices
        // and two summands, and re-embedding a term would expand it again and count the fold
        // twice. Scoring a single-index embedding asks the scorer for exactly the summand it
        // would have added, because a query's own values never enter the sum.
        //
        // Multiplicity is kept rather than deduplicated: the scorer iterates the query's tokens,
        // so a query saying `pool pool pool` scores `pool` three times and a decomposition that
        // said otherwise would not add up to the score it is decomposing.
        let mut counts: BTreeMap<u32, (String, u32)> = BTreeMap::new();
        for term in self.tokenizer.terms(text) {
            let index = <u32 as TokenEmbedder>::embed(&term);
            counts.entry(index).or_insert_with(|| (term, 0)).1 += 1;
        }
        let mut out: Vec<(String, f32)> = counts
            .into_iter()
            .filter_map(|(index, (term, n))| {
                let one = Embedding(vec![TokenEmbedding { index, value: 1.0 }]);
                let c = self.scorer.score(uid, &one)? * n as f32;
                (c > 0.0).then_some((term, c))
            })
            .collect();
        out.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        out
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
            .filter(|(uid, (schema, status))| query.admits_schema(uid, schema, *status))
            // Before the limit, as `within` is: a filter applied to a truncated list returns
            // fewer than `limit` results and calls it a ranking.
            .filter(|(uid, _)| {
                query.payload.is_none()
                    || self
                        .payloads
                        .get(*uid)
                        .is_some_and(|s| query.admits_payload(s))
            })
            .filter_map(|(uid, _)| {
                // `None` means the uid was never indexed; a zero score means no query term
                // occurs in it. Both are excluded, because returning them pads the result to
                // `limit` with units that do not match, which reads as a ranking and is not
                // one.
                let score = self.scorer.score(uid, &q)?;
                (score > 0.0).then_some(Hit::new(*uid, score))
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

        // After the truncation, not before: the decomposition is exact but not free, and the
        // hits that did not survive are not going to be explained to anybody.
        for hit in &mut hits {
            hit.terms = self.contributions(&hit.uid, &query.text);
        }
        hits
    }

    fn len(&self) -> usize {
        self.facts.len()
    }
}
