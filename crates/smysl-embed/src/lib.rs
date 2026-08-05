//! Semantic retrieval, behind the `Retriever` seam.
//!
//! # Why this exists, and what it is allowed to claim
//!
//! 0.5.0 shipped lexical retrieval and *measured* it rather than asserting it. Twenty
//! queries over the corpus, three classes:
//!
//! | class | recall@5 | MRR | P@1 |
//! |---|---:|---:|---:|
//! | shared vocabulary | 1.00 | 0.94 | 0.88 |
//! | paraphrase | 0.75 | 0.41 | **0.12** |
//! | identifier | 1.00 | 1.00 | 1.00 |
//!
//! By kernel type, `evidence` and `data` scored 1.00 and `claim` scored 0.67. The reading is
//! specific: concrete things are findable by name, and an interpretation is phrased in
//! whatever words its author reached for. So this crate exists to serve `claim`, `finding`
//! and `hypothesis`, and it has no business touching identifiers or `evidence`, where BM25
//! is already perfect and no embedding will beat it.
//!
//! That is a narrower claim than "semantic search is better", and it is the one the
//! measurement supports.
//!
//! # Why Model2Vec rather than a transformer
//!
//! A Model2Vec model is a *static* table: a token maps to a vector, and a sentence is a
//! pooled lookup. There is no forward pass, no ONNX Runtime, no downloaded binary, no `ort`
//! release-candidate pin, and no matmul whose reduction order depends on how the library was
//! compiled. It is not free of native code — `tokenizers` brings one C++ file through
//! `esaxx-rs`, which its own manifest explains — but one `.cpp` every default toolchain
//! compiles is a different order of thing from a runtime fetched at build time, and it is
//! used only for tokenizer *training*, which this crate never does. The cost is real — static embeddings lose most word-order and negation
//! sensitivity — and it is the right trade for a format whose central claim is that two
//! implementations agree on what a document says.
//!
//! # Determinism, stated exactly
//!
//! Retrieval here is a pure function of the store, the query and the model file, **within a
//! build**. Two runs on one machine agree; that is tested.
//!
//! Across machines it is *expected* to agree, because the arithmetic is a lookup and a sum
//! rather than a matrix multiply — but it is not tested, this crate does not control the
//! summation order inside `model2vec-rs`, and a SIMD reduction that differs by target would
//! be invisible from here. So results from this crate MUST NOT feed a pure operation. It is
//! `model-dependent` in the sense §23 uses, exactly like `ingest` and `attest`, and for the
//! same reason: something outside the format decides the answer.
//!
//! # Offline
//!
//! `model2vec-rs` is taken with `default-features = false`, which drops `hf-hub` and `ureq`.
//! There is no code path here that can reach the network. A model is three files the
//! operator already has, or there is no model.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

use std::collections::BTreeMap;
use std::path::Path;

use model2vec_rs::model::StaticModel;
use smysl_core::{KernelType, SchemaId, Status, Uid};
use smysl_graph::Store;
use smysl_retrieve::{Hit, Query, Retriever};

/// A model, loaded.
///
/// Thin wrapper so callers do not depend on `model2vec_rs` types directly — the seam that
/// lets a different static-embedding implementation take its place without touching anything
/// that uses this crate.
pub struct Model(StaticModel);

impl Model {
    /// Load from a directory holding `tokenizer.json`, `model.safetensors` and `config.json`.
    ///
    /// No token is passed and no subfolder: with `hf-hub` compiled out there is nothing to
    /// authenticate to. A path that is not a local directory is an error rather than a
    /// download.
    pub fn from_dir(path: impl AsRef<Path>) -> Result<Model, Error> {
        let path = path.as_ref();
        if !path.is_dir() {
            return Err(Error::NotADirectory(path.display().to_string()));
        }
        // `normalize: None` keeps whatever the model's own config asks for. Overriding it
        // here would silently change what a published model means.
        StaticModel::from_pretrained(path, None, None, None)
            .map(Model)
            .map_err(|e| Error::Load(e.to_string()))
    }

    /// Load from bytes the caller already holds.
    ///
    /// The path for an embedder shipped inside another binary, or read from somewhere this
    /// crate should not know about.
    pub fn from_bytes(tokenizer: &[u8], model: &[u8], config: &[u8]) -> Result<Model, Error> {
        StaticModel::from_bytes(tokenizer, model, config, None)
            .map(Model)
            .map_err(|e| Error::Load(e.to_string()))
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        self.0.encode_single(text)
    }

    fn embed_all(&self, texts: &[String]) -> Vec<Vec<f32>> {
        self.0.encode(texts)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    NotADirectory(String),
    Load(String),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::NotADirectory(p) => write!(
                f,
                "{p} is not a directory; a model is three files on disk, and this build \
                 cannot download one"
            ),
            Error::Load(e) => write!(f, "the model would not load: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// A semantic index over a store's gists.
///
/// The gist and only the gist. Body and detail are deliberately not embedded: a static model
/// pools over tokens, so a long body pulls its vector toward the average of everything it
/// mentions and away from what it is *about*. Lexical retrieval has the opposite property —
/// more text is more chances to match — which is one more reason the two belong side by side
/// rather than one replacing the other.
pub struct Semantic {
    vectors: BTreeMap<Uid, Vec<f32>>,
    facts: BTreeMap<Uid, (KernelType, Status)>,
    model: Model,
}

impl Semantic {
    /// Embed every unit in `store`.
    pub fn index(store: &Store, model: Model) -> Semantic {
        let facts: BTreeMap<Uid, (KernelType, Status)> = store
            .units()
            .filter_map(|(u, unit)| match &unit.core.schema {
                SchemaId::Kernel(k) => Some((*u, (*k, unit.core.status))),
                _ => None,
            })
            .collect();

        // Batched in one call, in uid order, so the work is done once and in a fixed
        // sequence rather than per unit in whatever order a map iterates.
        let uids: Vec<Uid> = facts.keys().copied().collect();
        let texts: Vec<String> = uids
            .iter()
            .map(|u| {
                store
                    .get(u)
                    .map(|unit| unit.core.gist.clone())
                    .unwrap_or_default()
            })
            .collect();
        let vectors = uids.into_iter().zip(model.embed_all(&texts)).collect();

        Semantic {
            vectors,
            facts,
            model,
        }
    }
}

/// Cosine similarity, with zero for a zero vector rather than a division by zero.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

impl Retriever for Semantic {
    fn search(&self, query: &Query) -> Vec<Hit> {
        if query.limit == 0 {
            return Vec::new();
        }
        let q = self.model.embed(&query.text);

        let mut hits: Vec<Hit> = self
            .facts
            .iter()
            .filter(|(_, (kind, status))| query.admits(*kind, *status))
            .filter_map(|(uid, _)| {
                let score = cosine(self.vectors.get(uid)?, &q);
                // Cosine is bounded in [-1, 1] and a negative score means "less like this
                // than an unrelated sentence would be", which is not a hit. Zero is excluded
                // for the same reason lexical excludes it: padding a result to `limit` with
                // things that do not match reads as a ranking and is not one.
                (score > 0.0).then_some(Hit::new(*uid, score))
            })
            .collect();

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(core::cmp::Ordering::Equal)
                .then_with(|| a.uid.cmp(&b.uid))
        });
        hits.truncate(query.limit);
        hits
    }

    fn len(&self) -> usize {
        self.vectors.len()
    }
}

/// Lexical and semantic together, routed by what the *query* looks like.
///
/// # What the measurement changed
///
/// This dispatched on the query's `kinds` filter until it was scored. The reasoning was that
/// the format already records what a unit is, so routing on kind uses structure that is
/// already there. The flaw is in *when* the information is available: a caller who knew which
/// kind they wanted would usually not be searching, so almost every real query arrives
/// without a filter — and the fallback for that case merged both engines on rank, which
/// pulled semantic's good ranks down by averaging them with lexical's bad ones.
///
/// Measured over the shared query set with `potion-base-8M`, that hybrid scored 0.78 MRR
/// against pure semantic's 0.84. It beat lexical, which was all its assertion asked, and lost
/// to the engine it was built on.
///
/// So it routes on the query, which is the one thing always available when the decision has
/// to be made:
///
/// | query | engine | why |
/// |---|---|---|
/// | identifier-shaped | lexical | 1.00 precision-at-one against 0.75 |
/// | anything else | semantic | 0.84 MRR against 0.74, and 0.50 against 0.12 on paraphrase |
///
/// An explicit `kinds` filter still refines it: a query restricted entirely to concrete kinds
/// goes to lexical whatever it looks like, because that is the caller saying so outright.
///
/// There is no merge any more. Merging was measurably worse than either engine alone at the
/// job each is good at, and a second opinion is only worth having when it is sometimes right.
pub struct Hybrid<L: Retriever, S: Retriever = Semantic> {
    lexical: L,
    semantic: S,
}

/// Kernel types where an interpretation is being searched for, and wording varies.
const INTERPRETIVE: &[KernelType] = &[
    KernelType::Claim,
    KernelType::Finding,
    KernelType::Hypothesis,
    KernelType::Question,
];

/// Whether a query is a name rather than a sentence.
///
/// `pool.wait_ms`, `checkout.p95`, `db.queries_per_request`, `latency_by_region.csv` — the
/// shape is one token carrying a separator. Lexical retrieval answers those perfectly and an
/// embedder does not, because a static model has no vector for a symbol it never saw.
///
/// Deliberately narrow: whitespace anywhere means prose, and prose goes to the embedder even
/// when it mentions an identifier. A sentence *about* `pool.wait_ms` is still a sentence, and
/// widening this to "contains an identifier" would send paraphrases to the engine that is
/// worst at them — which is the mistake this whole function exists to correct.
pub fn looks_like_identifier(query: &str) -> bool {
    let q = query.trim();
    !q.is_empty()
        && !q.chars().any(char::is_whitespace)
        && q.chars().any(|c| matches!(c, '_' | '.' | '/' | ':' | '-'))
}

// Generic over *both* sides, with `Semantic` only the default. The routing is the part of
// this crate with a judgement in it, and tying it to a concrete embedder would have made it
// testable only where a model file exists — which is nowhere in CI. Stubs test it in
// milliseconds instead, and that is how the routing above could be rewritten with confidence.
impl<L: Retriever, S: Retriever> Hybrid<L, S> {
    pub fn new(lexical: L, semantic: S) -> Hybrid<L, S> {
        Hybrid { lexical, semantic }
    }

    pub fn is_interpretive(kind: KernelType) -> bool {
        INTERPRETIVE.contains(&kind)
    }

    /// Which engine answers this query. Exposed so a caller can explain a result, and so the
    /// decision is testable without running either engine.
    pub fn routes_to_lexical(query: &Query) -> bool {
        if looks_like_identifier(&query.text) {
            return true;
        }
        !query.kinds.is_empty() && query.kinds.iter().all(|k| !Self::is_interpretive(*k))
    }
}

impl<L: Retriever, S: Retriever> Retriever for Hybrid<L, S> {
    fn search(&self, query: &Query) -> Vec<Hit> {
        if query.limit == 0 {
            return Vec::new();
        }
        if Self::routes_to_lexical(query) {
            self.lexical.search(query)
        } else {
            self.semantic.search(query)
        }
    }

    fn len(&self) -> usize {
        self.semantic.len().max(self.lexical.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A retriever that returns what it is told to, so routing can be tested without a model.
    struct Stub {
        hits: Vec<Hit>,
    }

    impl Retriever for Stub {
        fn search(&self, q: &Query) -> Vec<Hit> {
            let _ = q;
            self.hits.clone()
        }
        fn len(&self) -> usize {
            self.hits.len()
        }
    }

    fn uid(n: u8) -> Uid {
        Uid::from_bytes([n; 32])
    }

    fn stub(ids: &[u8]) -> Stub {
        Stub {
            hits: ids
                .iter()
                .enumerate()
                .map(|(i, n)| Hit::new(uid(*n), 10.0 - i as f32))
                .collect(),
        }
    }

    #[test]
    fn an_identifier_query_goes_to_lexical() {
        let h = Hybrid::new(stub(&[1]), stub(&[9]));
        for q in [
            "pool.wait_ms",
            "checkout.p95",
            "db.queries_per_request",
            "latency_by_region.csv",
        ] {
            let got = h.search(&Query::new(q, 5));
            assert_eq!(
                got[0].uid,
                uid(1),
                "`{q}` is a name, and lexical answers names at 1.00 precision-at-one"
            );
        }
    }

    #[test]
    fn prose_goes_to_the_embedder_even_when_it_mentions_an_identifier() {
        let h = Hybrid::new(stub(&[1]), stub(&[9]));
        let got = h.search(&Query::new("why did pool.wait_ms rise so sharply", 5));
        assert_eq!(
            got[0].uid,
            uid(9),
            "a sentence about an identifier is still a sentence; widening the rule to \
             `contains an identifier` would send paraphrases to the engine worst at them"
        );
    }

    #[test]
    fn a_concrete_kinds_filter_still_routes_to_lexical() {
        let h = Hybrid::new(stub(&[1]), stub(&[9]));
        let got = h.search(&Query::new("some prose query", 5).kinds([KernelType::Evidence]));
        assert_eq!(
            got[0].uid,
            uid(1),
            "an explicit filter is the caller saying outright what they want"
        );
    }

    #[test]
    fn an_interpretive_filter_goes_to_the_embedder() {
        let h = Hybrid::new(stub(&[1]), stub(&[9]));
        let got = h.search(&Query::new("some prose query", 5).kinds([KernelType::Claim]));
        assert_eq!(got[0].uid, uid(9));
    }

    /// There is no merge, and that is deliberate rather than an omission.
    ///
    /// Merging on rank scored 0.78 MRR where pure semantic scored 0.84: averaging a good
    /// ranking with a bad one gives a middling one. This pins the absence, so restoring a
    /// merge is a decision someone makes against a number rather than a tidy-looking idea.
    #[test]
    fn one_engine_answers_and_the_other_is_not_consulted() {
        let h = Hybrid::new(stub(&[1, 2]), stub(&[9]));
        let got = h.search(&Query::new("a prose query with no filter", 5));
        assert_eq!(
            got.iter().map(|x| x.uid).collect::<Vec<_>>(),
            vec![uid(9)],
            "results from both engines means the merge came back"
        );
    }

    #[test]
    fn the_identifier_rule_is_narrow() {
        assert!(looks_like_identifier("pool.wait_ms"));
        assert!(looks_like_identifier("a/b"));
        assert!(!looks_like_identifier("pool wait ms"));
        assert!(
            !looks_like_identifier("saturated"),
            "one bare word is not a name"
        );
        assert!(!looks_like_identifier(""));
    }

    #[test]
    fn a_zero_limit_returns_nothing() {
        let h = Hybrid::new(stub(&[1]), stub(&[9]));
        assert!(h.search(&Query::new("x", 0)).is_empty());
    }
}
