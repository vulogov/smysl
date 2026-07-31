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
                (score > 0.0).then_some(Hit { uid: *uid, score })
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

/// Lexical and semantic together, chosen per kernel type rather than blended.
///
/// The 0.5.0 measurement is unusually clear about which engine to use where. `evidence` and
/// `data` scored 1.00 on BM25 and identifiers scored 1.00 with perfect precision, because
/// those units name concrete things and concrete things are findable by name. `claim` scored
/// 0.67, and on a paraphrased query the right claim ranked first once in eight, because a
/// claim is an interpretation and interpretations get worded differently by different people.
///
/// So this dispatches rather than blends. Blending would need a weight, the weight would need
/// tuning, tuning would need a larger query set than exists, and the result would be worse on
/// the half that already works — BM25 at 1.00 has nothing to gain from a second opinion and
/// something to lose. Routing by what a unit *is* uses the structure the format already
/// carries, which is the whole argument for having kernel types at all.
///
/// A query with an explicit `kinds` filter is routed by that filter. A query without one runs
/// both and merges, because the caller has not said what they are looking for and either
/// engine may be the right one.
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

// Generic over *both* sides, with `Semantic` only the default. The routing is the part of
// this crate with a judgement in it, and tying it to a concrete embedder would have made it
// testable only where a model file exists — which is nowhere in CI. Two stubs test it in
// milliseconds instead.
impl<L: Retriever, S: Retriever> Hybrid<L, S> {
    pub fn new(lexical: L, semantic: S) -> Hybrid<L, S> {
        Hybrid { lexical, semantic }
    }

    /// Which engine a kind should be answered by.
    ///
    /// `Question` joins the interpretive set even though 0.5.0 had only one of them, so the
    /// evidence for it is thin. The reasoning is that a question is phrased by whoever asked
    /// it and searched for by whoever answers — the vocabulary-mismatch case by construction.
    /// Recorded as reasoning rather than measurement, so it is clear which it is.
    pub fn is_interpretive(kind: KernelType) -> bool {
        INTERPRETIVE.contains(&kind)
    }
}

impl<L: Retriever, S: Retriever> Retriever for Hybrid<L, S> {
    fn search(&self, query: &Query) -> Vec<Hit> {
        if query.limit == 0 {
            return Vec::new();
        }

        // An explicit filter says what the caller wants, so route on it. All-interpretive
        // goes to the embedder, all-concrete to BM25, and a mixture runs both.
        if !query.kinds.is_empty() {
            let interpretive = query.kinds.iter().all(|k| Self::is_interpretive(*k));
            let concrete = query.kinds.iter().all(|k| !Self::is_interpretive(*k));
            if interpretive {
                return self.semantic.search(query);
            }
            if concrete {
                return self.lexical.search(query);
            }
        }

        // No filter, or a mixed one: run both and merge. Scores are not comparable across
        // engines — BM25 is unbounded and cosine is in [-1, 1] — so merging on score would
        // let one engine's scale decide everything. Merge on *rank* instead, keeping each
        // engine's best where they disagree.
        let mut merged: BTreeMap<Uid, (usize, f32)> = BTreeMap::new();
        for (rank, h) in self.lexical.search(query).into_iter().enumerate() {
            merged.insert(h.uid, (rank, h.score));
        }
        for (rank, h) in self.semantic.search(query).into_iter().enumerate() {
            merged
                .entry(h.uid)
                .and_modify(|e| {
                    if rank < e.0 {
                        *e = (rank, h.score);
                    }
                })
                .or_insert((rank, h.score));
        }

        let mut hits: Vec<(usize, Hit)> = merged
            .into_iter()
            .map(|(uid, (rank, score))| (rank, Hit { uid, score }))
            .collect();
        // Best rank first, ties broken by uid so the result is a function rather than a
        // preference — the same rule both engines use.
        hits.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.uid.cmp(&b.1.uid)));
        hits.into_iter().map(|(_, h)| h).take(query.limit).collect()
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
        name: &'static str,
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

    fn stub(name: &'static str, ids: &[u8]) -> Stub {
        Stub {
            name,
            hits: ids
                .iter()
                .enumerate()
                .map(|(i, n)| Hit {
                    uid: uid(*n),
                    score: 10.0 - i as f32,
                })
                .collect(),
        }
    }

    #[test]
    fn an_interpretive_filter_routes_to_the_embedder() {
        let h = Hybrid::new(stub("lex", &[1, 2]), stub("sem", &[9]));
        let got = h.search(&Query::new("x", 5).kinds([KernelType::Claim]));
        assert_eq!(got.len(), 1, "{:?}", h.lexical.name);
        assert_eq!(
            got[0].uid,
            uid(9),
            "a claim query should go to the embedder"
        );
    }

    #[test]
    fn a_concrete_filter_routes_to_lexical() {
        let h = Hybrid::new(stub("lex", &[1, 2]), stub("sem", &[9]));
        let got = h.search(&Query::new("x", 5).kinds([KernelType::Evidence]));
        assert_eq!(
            got.iter().map(|x| x.uid).collect::<Vec<_>>(),
            vec![uid(1), uid(2)],
            "evidence scored 1.00 on BM25; sending it to an embedder can only lose"
        );
    }

    /// A mixed filter cannot be answered by one engine, so both run.
    #[test]
    fn a_mixed_filter_runs_both() {
        let h = Hybrid::new(stub("lex", &[1]), stub("sem", &[9]));
        let got = h.search(&Query::new("x", 5).kinds([KernelType::Claim, KernelType::Evidence]));
        let ids: Vec<_> = got.iter().map(|x| x.uid).collect();
        assert!(ids.contains(&uid(1)) && ids.contains(&uid(9)), "{ids:?}");
    }

    /// With no filter the caller has not said what they want, so neither engine is dismissed.
    #[test]
    fn no_filter_merges_on_rank_not_score() {
        // Lexical scores are unbounded and cosine is in [-1, 1]. If this merged on score the
        // lexical side would win everything regardless of rank, which is the bug the merge is
        // written to avoid.
        let lex = Stub {
            name: "lex",
            hits: vec![Hit {
                uid: uid(1),
                score: 900.0,
            }],
        };
        let sem = Stub {
            name: "sem",
            hits: vec![Hit {
                uid: uid(9),
                score: 0.8,
            }],
        };
        let got = Hybrid::new(lex, sem).search(&Query::new("x", 5));
        assert_eq!(
            got.len(),
            2,
            "both engines' top hit should survive a rank merge"
        );
        assert_eq!(
            got.iter().map(|h| h.uid).collect::<Vec<_>>(),
            vec![uid(1), uid(9)],
            "rank 0 from each, tie broken by uid — not by the larger score"
        );
    }

    #[test]
    fn a_zero_limit_returns_nothing() {
        let h = Hybrid::new(stub("lex", &[1]), stub("sem", &[9]));
        assert!(h.search(&Query::new("x", 0)).is_empty());
    }
}
