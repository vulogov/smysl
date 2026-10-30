//! Transformer embeddings for a smysl store: S1 of RFC SMYSL-2.5 §3.14.
//!
//! S0 (`smysl-embed`) is a static table — a token maps to a vector. S1 runs an encoder: the
//! XLM-R family (`multilingual-e5-small`, `-base`) and `bge-m3`'s dense head, on candle, on the
//! CPU, loaded from a directory the operator supplies (`config.json`, `tokenizer.json`,
//! `model.safetensors`). No network, as for S0.
//!
//! - [`Encoder`] — the model, its tokenizer and a [`Pooling`]; [`Encoder::model_hash`] is
//!   BLAKE3 over the three files, the pooling id and the prefixes.
//! - [`VectorCache`] — vectors by `(model hash, uid)`, in a sidecar `<store>.vec-<hash12>`;
//!   one written under another model hash is ignored and re-embedded (`SMY-W476`).
//! - [`CandleSemantic`] — a store's gists embedded, behind the `Retriever` seam, as S0's
//!   `Semantic` is.
//!
//! # Determinism, stated exactly
//!
//! **Within a build**, a text's vector is a function of the text and the model hash: each text
//! is encoded alone (no batch padding to change a reduction) and pooled by plain loops in a
//! fixed order. Two runs agree; that is tested. Across machines nothing is claimed — a matmul's
//! reduction order is candle's and the CPU's — so this crate is `model-dependent`, as S0 is,
//! and its results MUST NOT feed a pure operation.
//!
//! # No C
//!
//! TX-P10 step 1 verified the dependencies: candle 0.9, `tokenizers` 0.21 with `fancy-regex`,
//! no `-sys` crate and no `cc` — which `xtask check-native` keeps true.
#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

mod cache;
mod encoder;
mod pool;

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::{SchemaId, Status, Uid};
use smysl_graph::StoreRead;
use smysl_retrieve::{Hit, Query, Retriever};

pub use cache::VectorCache;
pub use encoder::{Encoder, TextRole};
pub use pool::Pooling;

/// Why a model, a text or a cache could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The path given is not a directory; a model is three files on disk.
    NotADirectory(String),
    /// One of the three files is missing or unreadable: its name, and why.
    Missing(String, String),
    /// `config.json` did not parse, or not as its architecture's configuration.
    Config(String),
    /// `config.json` names an architecture this crate does not run.
    Architecture(String),
    Tokenizer(String),
    /// The weights did not load, or a forward pass failed.
    Model(String),
    /// A sidecar could not be read, decoded or written.
    Cache(String),
    /// A cache handed to [`CandleSemantic::index_cached`] is another encoder's.
    WrongCache,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::NotADirectory(p) => write!(
                f,
                "{p} is not a directory; a model is three files on disk, and this build \
                 cannot download one"
            ),
            Error::Missing(name, why) => write!(f, "the model directory's {name}: {why}"),
            Error::Config(e) => write!(f, "config.json: {e}"),
            Error::Architecture(t) => write!(
                f,
                "config.json's model_type is `{t}`; this build runs `bert` and `xlm-roberta`"
            ),
            Error::Tokenizer(e) => write!(f, "tokenizer: {e}"),
            Error::Model(e) => write!(f, "the model: {e}"),
            Error::Cache(e) => write!(f, "vector cache: {e}"),
            Error::WrongCache => write!(f, "the vector cache was opened for another encoder"),
        }
    }
}

impl std::error::Error for Error {}

/// A store's gists, embedded by an [`Encoder`].
///
/// The gist and only the gist, as S0: an index of what each unit is *about*. Passages are
/// embedded as [`TextRole::Passage`] and a query as [`TextRole::Query`].
pub struct CandleSemantic {
    vectors: BTreeMap<Uid, Vec<f32>>,
    facts: BTreeMap<Uid, (SchemaId, Status)>,
    payloads: BTreeMap<Uid, BTreeMap<String, BTreeSet<String>>>,
    encoder: Encoder,
}

impl CandleSemantic {
    /// Embed every unit in `store`, with no cache.
    pub fn index(
        store: &(impl StoreRead + ?Sized),
        encoder: Encoder,
    ) -> Result<CandleSemantic, Error> {
        let mut cache = VectorCache::new(&encoder);
        CandleSemantic::index_cached(store, encoder, &mut cache)
    }

    /// Embed every unit in `store` that `cache` does not already hold, and leave `cache`
    /// holding exactly the store's units — a vector for each new one, none for a unit gone.
    /// The caller saves it.
    pub fn index_cached(
        store: &(impl StoreRead + ?Sized),
        encoder: Encoder,
        cache: &mut VectorCache,
    ) -> Result<CandleSemantic, Error> {
        if cache.model_hash() != encoder.model_hash() {
            return Err(Error::WrongCache);
        }
        let mut facts = BTreeMap::new();
        let mut payloads = BTreeMap::new();
        let mut vectors = BTreeMap::new();
        for (uid, unit) in store.units() {
            facts.insert(*uid, (unit.core.schema.clone(), unit.core.status));
            let strings = unit
                .core
                .payload
                .as_deref()
                .map(smysl_core::surface::payload::payload_strings)
                .unwrap_or_default();
            payloads.insert(*uid, strings);
            let v = match cache.get(uid) {
                Some(v) => v.to_vec(),
                None => {
                    let v = encoder.embed_one(&unit.core.gist, TextRole::Passage)?;
                    cache.insert(*uid, v.clone());
                    v
                }
            };
            vectors.insert(*uid, v);
        }
        cache.retain(|u| facts.contains_key(u));
        Ok(CandleSemantic {
            vectors,
            facts,
            payloads,
            encoder,
        })
    }

    /// The vector a unit was indexed under.
    pub fn vector(&self, uid: &Uid) -> Option<&[f32]> {
        self.vectors.get(uid).map(Vec::as_slice)
    }

    pub fn encoder(&self) -> &Encoder {
        &self.encoder
    }
}

/// The dot product, which is cosine for the unit vectors pooling returns; zero for vectors
/// of different lengths.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

impl Retriever for CandleSemantic {
    /// Ranked by cosine, ties broken by uid. A query the encoder cannot embed — a tokenizer
    /// failure — finds nothing rather than panicking: the seam has no error, and an empty
    /// result is what "no unit is like this" already means.
    fn search(&self, query: &Query) -> Vec<Hit> {
        if query.limit == 0 {
            return Vec::new();
        }
        let Ok(q) = self.encoder.embed_one(&query.text, TextRole::Query) else {
            return Vec::new();
        };
        let mut hits: Vec<Hit> = self
            .facts
            .iter()
            .filter(|(uid, (schema, status))| query.admits_schema(uid, schema, *status))
            .filter(|(uid, _)| {
                query.payload.is_none()
                    || self
                        .payloads
                        .get(*uid)
                        .is_some_and(|s| query.admits_payload(s))
            })
            .filter_map(|(uid, _)| {
                let score = dot(self.vectors.get(uid)?, &q);
                // As S0: a score at or below zero is not a hit, and padding a result to
                // `limit` with one would read as a ranking.
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
