//! Retrieval over a store: a seam, and a lexical default.
//!
//! # Why a seam rather than an engine
//!
//! smysl carries whatever an AI pipeline puts in it — a gist, a stack trace, a metric
//! series, a diff, a user's question, a research abstract. No single retrieval strategy
//! covers that. Lexical search is what actually works for identifiers and code; semantic
//! search is what closes the vocabulary gap between a question and its answer; neither
//! touches a raw numeric series usefully.
//!
//! So this crate defines [`Retriever`] and ships one implementation. Choosing an embedding
//! model, distributing its weights, and accepting that inference is only reproducible on
//! comparable hardware are all real decisions, and the seam is what keeps them reversible
//! and out of the pure crates.
//!
//! # What gets indexed
//!
//! **The gist, principally.** Every unit has one — it is required, `SMY-E021` — and it is by
//! construction a natural-language summary of whatever the payload holds. That is the
//! property that makes payload heterogeneity tractable: you are not indexing the telemetry,
//! you are indexing *"p95 auth latency tripled after the 4.3 serialiser change"*.
//!
//! Body and detail are indexed too, at lower weight, because they are the same content at
//! greater length. Raw payload bytes are not indexed: a metric series has no terms worth
//! matching, and indexing it would dilute every score it touched.
//!
//! This does mean retrieval quality is bounded by gist quality, which is an ingest concern
//! rather than a retrieval one. `SMY-W041` and the granularity report already point at it.
//!
//! # Determinism
//!
//! This crate is **pure** (rule D): retrieval is a bit-reproducible function of the store and
//! the query. That is why the `bm25` dependency is taken with `default-features = false` —
//! its language detection would make tokenisation depend on the corpus, and its `parallelism`
//! feature would put a rayon reduction inside a result that must not vary.
//!
//! An implementation that cannot promise this — anything doing neural inference — belongs
//! behind the same seam but outside this crate, in the impure, feature-gated tier where the
//! providers live.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

use std::collections::BTreeMap;

use smysl_core::{KernelType, SchemaId, Status, Uid};
use smysl_graph::Store;

pub mod tokenize;

mod lexical;
pub use lexical::Bm25;

/// One retrieved unit and why it scored.
///
/// `#[non_exhaustive]` with a constructor, decided in the §1.2 S3 shape review. `Query`, three
/// declarations below, already carried the attribute and `Hit` did not — an asymmetry with no
/// reason behind it, in the pair of types that face each other across the same call.
///
/// It matters here more than for most output types because `Retriever` is a public trait that
/// anyone may implement, so `Hit` is a type third parties have to *construct*: `smysl-embed`
/// builds it at two sites today. A retrieval result plausibly grows — which field matched, a
/// snippet, an explanation of the score — and at 1.0 an exhaustive struct could not gain one
/// without a 2.0.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Hit {
    pub uid: Uid,
    /// Higher is more relevant. Scales differ between implementations, so compare within one
    /// result set and never across two.
    pub score: f32,
}

impl Hit {
    /// The two things every hit has. Anything added later gets a setter or a builder, so this
    /// signature stays the one an implementation of `Retriever` writes.
    pub fn new(uid: Uid, score: f32) -> Hit {
        Hit { uid, score }
    }
}

/// What to retrieve, and from where.
///
/// Filters are part of the query rather than applied afterwards, because a retriever that
/// knows a filter can often satisfy it more cheaply than a caller trimming a ranked list —
/// and because trimming afterwards silently returns fewer than `limit` results.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Query {
    pub text: String,
    /// Maximum hits to return. A retriever MAY return fewer; it MUST NOT return more.
    pub limit: usize,
    /// Restrict to these kernel types. Empty means no restriction.
    ///
    /// The reason this is first-class: a store mixing `Question`, `Data` and `Prose` is a
    /// store where one aggregate relevance number is close to meaningless, and callers
    /// almost always want one kind at a time.
    pub kinds: Vec<KernelType>,
    /// Restrict to units at or above this status. `None` means no restriction.
    pub min_status: Option<Status>,
}

impl Query {
    pub fn new(text: impl Into<String>, limit: usize) -> Query {
        Query {
            text: text.into(),
            limit,
            kinds: Vec::new(),
            min_status: None,
        }
    }

    pub fn kinds(mut self, kinds: impl IntoIterator<Item = KernelType>) -> Query {
        self.kinds = kinds.into_iter().collect();
        self
    }

    pub fn min_status(mut self, s: Status) -> Query {
        self.min_status = Some(s);
        self
    }

    /// Whether a unit passes the filters. Provided so every implementation applies them
    /// identically rather than each inventing its own reading of `min_status`.
    pub fn admits(&self, kind: KernelType, status: Status) -> bool {
        if !self.kinds.is_empty() && !self.kinds.contains(&kind) {
            return false;
        }
        match self.min_status {
            Some(m) => status >= m,
            None => true,
        }
    }
}

/// Something that can rank a store's units against a query.
///
/// Implementations are built from a store and then queried, rather than queried against a
/// store, because every useful implementation has an index to amortise and rebuilding it per
/// query would make the cheap ones as slow as the expensive ones.
///
/// The index is **derived state**, like the one `reindex` rebuilds from the log. It is never
/// part of a uid, and discarding it loses nothing but time.
pub trait Retriever {
    /// Rank units against `query`, best first.
    ///
    /// MUST be deterministic for a given index and query: equal scores are broken by uid, so
    /// there is one right answer rather than an arbitrary one.
    fn search(&self, query: &Query) -> Vec<Hit>;

    /// How many units the index holds. Useful for asserting an index was actually built,
    /// which is the failure that otherwise looks like "no results".
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The text of a unit that is worth indexing, and how much each part counts.
///
/// Returned as parts rather than one joined string so an implementation can weight them.
/// Gist first and heaviest: it is the summary the format guarantees exists.
pub(crate) fn indexable(store: &Store, uid: &Uid) -> Option<Vec<(String, f32)>> {
    let unit = store.get(uid)?;
    let mut parts = vec![(unit.core.gist.clone(), 1.0)];
    if let Some(b) = &unit.core.body {
        parts.push((b.clone(), 0.5));
    }
    if let Some(d) = &unit.core.detail {
        parts.push((d.clone(), 0.25));
    }
    Some(parts)
}

/// Every unit in the store, with the facts the filters need.
pub(crate) fn candidates(store: &Store) -> BTreeMap<Uid, (KernelType, Status)> {
    store
        .units()
        .filter_map(|(u, unit)| match &unit.core.schema {
            // Extension and unknown-kernel units are indexed by nothing here: a filter on
            // `KernelType` cannot express them, and silently treating them as some kernel
            // type would be worse than leaving them out. Retrieval over extension types is
            // a real gap, recorded rather than papered over.
            SchemaId::Kernel(k) => Some((*u, (*k, unit.core.status))),
            _ => None,
        })
        .collect()
}
