//! The postings seam, and BM25 over anything that implements it (SMYSL-2.4 §4.3.5, TX-P4 step 2).
//!
//! # What this is for
//!
//! `find` spends its time *building* an index, not searching one: 7.06 s of 7.59 s at 155k units
//! (SMYSL-2.8 M-5, F-10). Nothing about that work is query-dependent — the terms of a unit are a
//! fact about the unit — so it is work that should be done once and kept. Keeping it needs two
//! things: a shape for the kept data, and a scorer that can read that shape instead of its own
//! private one. [`Postings`] is the shape and [`Bm25View`] is the scorer.
//!
//! The persistent implementation is the `postings` table of SMYSL-2.4 §3.8, in
//! `smysl_text::index`. It lives there because partitioning is a fact about a corpus of texts —
//! a part, a language, an analyzer — and this crate knows about none of the three.
//!
//! # Why the scores are bit-identical and not merely close
//!
//! [`Bm25`](crate::Bm25) scores through the `bm25` crate. A second scorer that was *about* as
//! good would make every result depend on which path produced it, and the exit clause for this
//! step is that the two agree. So [`Bm25View`] reproduces that crate's arithmetic exactly,
//! operation for operation and in the same order:
//!
//! | quantity | formula |
//! |---|---|
//! | `avgdl` | `(total_len as f64 / docs as f64) as f32`, or 256.0 for an empty corpus |
//! | `idf(t)` | `(1 + (N - df + 0.5) / (df + 0.5)).ln()` in `f32`, `N` = indexed docs |
//! | `value(t, d)` | `tf·(k1+1) / (tf + k1·((1 - b) + b·(len_d / avgdl)))`, `k1` = 1.2, `b` = 0.75 |
//! | `score(d, q)` | `idf(t)·value(t, d)` summed over the query's tokens **in order, with repetition** |
//!
//! The summation order is the part that is easy to lose. Floating-point addition is not
//! associative, so a scorer that iterated the document's terms, or deduplicated the query's,
//! would differ in the last bits from one that walks the query as written — and `--why` prints
//! four decimal places of it. A term the document does not hold contributes exactly `0.0`,
//! which is why this walks every query token rather than only the matching ones: adding zero is
//! free and keeps the order the same.
//!
//! # The one place the two paths can disagree, and why this one is right
//!
//! The `bm25` crate identifies a term by `hash32` of it, and its document frequencies are
//! counted per *hash*. Two distinct terms that collide therefore share a `df` and a `tf` there.
//! A postings table is keyed by the term itself and cannot collide. Over a vocabulary of 50k
//! terms the birthday bound puts one colliding pair at a few parts in a thousand, so this is
//! not theoretical at corpus scale — and where it happens, the path that keyed by the term is
//! the one that is right. The divergence is recorded rather than smoothed over: a seam that
//! reproduced a 32-bit hash to stay compatible with it would be preserving a defect.
//!
//! # Determinism
//!
//! Every collection here is a `BTreeMap`, every list is sorted, and the search is the same
//! function of the postings and the query on every machine — the rule the whole crate is
//! under. A postings table is derived state, like the sidecar `reindex` rebuilds: discarding it
//! loses time and nothing else.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::Uid;
use smysl_graph::Store;

use crate::{indexable, Analyze, Hit, Query, Retriever};

/// BM25's term-frequency saturation, as the `bm25` crate defaults it.
pub const K1: f32 = 1.2;

/// BM25's length normalisation, as the `bm25` crate defaults it.
pub const B: f32 = 0.75;

/// The average document length an empty corpus is scored against, as the `bm25` crate defaults
/// it. It cannot matter — an empty corpus has no documents to score — but it has to agree, since
/// a one-document corpus reaches it through `avgdl <= 0.0`.
pub const FALLBACK_AVGDL: f32 = 256.0;

/// An inverted index over analysed documents.
///
/// Implementations are free about storage and bound about arithmetic: the counts here are what
/// [`Bm25View`] weighs, so a `df` that counted something else — terms rather than documents, say,
/// or including documents that were filtered out — would not be a slower index, it would be a
/// different ranking.
///
/// Every method is a lookup and none is a scan. That is the contract: a seam whose `df` walked
/// the corpus would reintroduce exactly the cost this exists to remove.
pub trait Postings {
    /// How many documents hold `term`. Zero for a term the corpus does not have.
    fn df(&self, term: &str) -> u32;

    /// The documents holding `term` and how many times each holds it, in uid order.
    ///
    /// Uid order because it is the order every other derived structure here is in, and because a
    /// caller merging two lists can then do it in one pass.
    fn postings(&self, term: &str) -> Vec<(Uid, u32)>;

    /// The length of a document in terms, or `None` if it is not indexed.
    ///
    /// Length *after* analysis and after whatever weighting the builder applied, because that is
    /// the number BM25 normalises by. [`MemPostings::index`] repeats the gist four times and the
    /// body twice, so a unit's length is not the length of its gist.
    fn doc_len(&self, uid: &Uid) -> Option<u32>;

    /// How many documents are indexed. BM25's `N`.
    fn docs(&self) -> u32;

    /// The total length of every indexed document.
    ///
    /// Not in SMYSL-2.4 §4.3.5's four methods, and needed: `avgdl` is a corpus-wide quantity, so
    /// a scorer without this would have to walk every document's length on every query — which
    /// is the scan this seam exists to avoid, and it would make a four-method `Postings` a
    /// slower index than the one it replaced. A persistent table keeps the sum beside the count
    /// and answers both in constant time.
    fn total_len(&self) -> u64;

    /// How many times one document holds one term, without materialising the postings list.
    ///
    /// Provided, because [`Postings::postings`] answers it and an implementation need not do
    /// better. It should: the default clones a list that can be as long as the corpus — a term
    /// in every document has a posting in every document — and the question is asked once per
    /// surviving hit when a score is decomposed. An implementation with its list in hand can
    /// binary-search it.
    fn tf(&self, term: &str, uid: &Uid) -> u32 {
        self.postings(term)
            .binary_search_by(|(u, _)| u.cmp(uid))
            .ok()
            .and_then(|at| self.postings(term).get(at).map(|(_, tf)| *tf))
            .unwrap_or(0)
    }

    /// The analyzer id the terms were produced under, if the implementation records one.
    ///
    /// `None` is allowed because an implementation may not know; it is not an invitation to not
    /// care. A query analysed by one chain against terms produced by another retrieves close to
    /// nothing and says nothing about why, which is the failure this id exists to catch.
    fn analyzer(&self) -> Option<&str> {
        None
    }

    /// BM25's `avgdl`, reproducing the `bm25` crate's arithmetic exactly.
    ///
    /// Provided rather than required: it is a function of [`Postings::total_len`] and
    /// [`Postings::docs`], and an implementation that computed it differently would score
    /// differently for no reason anybody chose.
    fn avgdl(&self) -> f32 {
        if self.docs() == 0 {
            return FALLBACK_AVGDL;
        }
        let avgdl = (self.total_len() as f64 / self.docs() as f64) as f32;
        if avgdl <= 0.0 {
            FALLBACK_AVGDL
        } else {
            avgdl
        }
    }
}

/// An in-memory inverted index, built the way [`Bm25::index_with`](crate::Bm25::index_with)
/// builds its own.
///
/// This is the reference implementation of the seam and the thing a persistent table is compared
/// against: it is built by today's path, so "results identical to the in-memory path" is a test
/// that can actually be written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemPostings {
    terms: BTreeMap<String, Vec<(Uid, u32)>>,
    lens: BTreeMap<Uid, u32>,
    total: u64,
    analyzer: String,
}

impl MemPostings {
    /// An empty index that will record terms produced by `analyzer`.
    pub fn new(analyzer: &dyn Analyze) -> MemPostings {
        MemPostings {
            terms: BTreeMap::new(),
            lens: BTreeMap::new(),
            total: 0,
            analyzer: analyzer.id().to_string(),
        }
    }

    /// Index every unit of `store` under `analyzer`.
    ///
    /// The same corpus [`Bm25::index_with`](crate::Bm25::index_with) builds: every unit the store
    /// holds, in uid order, with the gist repeated four times, the body twice and the detail
    /// once. The repetition is BM25's only way to express field weight, and it has to be here
    /// too or the lengths and the term frequencies would both be different numbers.
    pub fn index(store: &Store, analyzer: &dyn Analyze) -> MemPostings {
        let mut out = MemPostings::new(analyzer);
        for (uid, _) in store.units() {
            if let Some(text) = document_text(store, uid) {
                out.add(*uid, &analyzer.terms(&text));
            }
        }
        out
    }

    /// Record one document's terms, replacing it if it was already indexed.
    ///
    /// Replacing rather than adding, so that an incremental builder handed the same unit twice
    /// does not double its term frequencies. A unit's text is immutable — its gist is part of its
    /// identity — so this is a safety property rather than a feature.
    pub fn add(&mut self, uid: Uid, terms: &[String]) {
        self.remove(&uid);
        let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
        for t in terms {
            *counts.entry(t.as_str()).or_insert(0) += 1;
        }
        for (term, tf) in counts {
            let list = self.terms.entry(term.to_string()).or_default();
            match list.binary_search_by(|(u, _)| u.cmp(&uid)) {
                Ok(at) => list[at] = (uid, tf),
                Err(at) => list.insert(at, (uid, tf)),
            }
        }
        let len = terms.len() as u32;
        self.lens.insert(uid, len);
        self.total += u64::from(len);
    }

    /// Forget a document, if it was indexed.
    ///
    /// Needed by the incremental path for a withdrawn or redacted unit, and by [`MemPostings::add`]
    /// so that a replacement is a replacement.
    pub fn remove(&mut self, uid: &Uid) {
        let Some(len) = self.lens.remove(uid) else {
            return;
        };
        self.total -= u64::from(len);
        self.terms.retain(|_, list| {
            if let Ok(at) = list.binary_search_by(|(u, _)| u.cmp(uid)) {
                list.remove(at);
            }
            !list.is_empty()
        });
    }

    /// Every term, in order, with its postings list. For a builder persisting this.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &[(Uid, u32)])> {
        self.terms.iter().map(|(t, l)| (t.as_str(), l.as_slice()))
    }

    /// Every indexed document and its length, in uid order.
    pub fn lengths(&self) -> impl Iterator<Item = (&Uid, u32)> {
        self.lens.iter().map(|(u, l)| (u, *l))
    }

    /// Rebuild a table from what a persistent one stored, without the documents.
    ///
    /// The decoder's constructor. A table on disk holds counts rather than texts — that is the
    /// point of it — so there is no term list to replay through [`MemPostings::add`], and a
    /// reader must be able to say "these are the postings" directly.
    ///
    /// `total_len` is recomputed from `lens` rather than read, because it is a sum of them and a
    /// stored sum that disagreed with its terms would be a corrupt table that still answered
    /// questions. Nothing else here is checked: a postings table is derived state, so the
    /// repair for a wrong one is to rebuild it, and a decoder that refused would turn a cache
    /// into a failure.
    pub fn from_parts(
        analyzer: &str,
        terms: BTreeMap<String, Vec<(Uid, u32)>>,
        lens: BTreeMap<Uid, u32>,
    ) -> MemPostings {
        let total = lens.values().map(|l| u64::from(*l)).sum();
        MemPostings {
            terms,
            lens,
            total,
            analyzer: analyzer.to_string(),
        }
    }
}

impl Postings for MemPostings {
    fn df(&self, term: &str) -> u32 {
        self.terms.get(term).map_or(0, |l| l.len() as u32)
    }

    fn postings(&self, term: &str) -> Vec<(Uid, u32)> {
        self.terms.get(term).cloned().unwrap_or_default()
    }

    fn doc_len(&self, uid: &Uid) -> Option<u32> {
        self.lens.get(uid).copied()
    }

    fn docs(&self) -> u32 {
        self.lens.len() as u32
    }

    fn total_len(&self) -> u64 {
        self.total
    }

    fn tf(&self, term: &str, uid: &Uid) -> u32 {
        self.terms
            .get(term)
            .and_then(|list| {
                list.binary_search_by(|(u, _)| u.cmp(uid))
                    .ok()
                    .map(|at| list[at].1)
            })
            .unwrap_or(0)
    }

    fn analyzer(&self) -> Option<&str> {
        Some(&self.analyzer)
    }
}

/// A [`Postings`] restricted to some of its documents (TX-P6 step 3).
///
/// # What it is for
///
/// SMYSL-2.5 §3.2 phase 4 splits a candidate set into partitions and §3.4 asks for IDF that is
/// the partition's rather than the corpus's. The RFC asked for that as
/// `Bm25::index_subset(store, uids, tokenizer)` — a second *index* per partition — and TX-P4
/// step 2 made it unnecessary before anybody built it: every quantity BM25 needs comes through
/// this trait, so restricting the answers restricts the statistics, and no term is read twice.
/// `df`, `N`, `avgdl` and the candidate lists are all the subset's.
///
/// So this is the pure-tier counterpart of `smysl_text::index::postings::View`, which sums
/// *disjoint partition tables*. Here there is one table and a set to restrict it to, which is
/// the shape a plain `Store` gives the planner: one corpus, partitioned after the fact.
///
/// # The claim
///
/// Scoring through a `Subset` is bit-identical to scoring a table built from those units alone.
/// That is what makes it a replacement for the second index rather than an approximation of
/// one, and it is asserted rather than asserted-to-be-obvious: see `tests/postings.rs`.
///
/// `uids` naming documents the inner table does not hold is not an error — a partition is a set
/// of units and a postings table indexes the ones that had text — so [`Postings::docs`] counts
/// the overlap and not the request.
pub struct Subset<'p> {
    inner: &'p dyn Postings,
    uids: &'p BTreeSet<Uid>,
    /// Counted once at construction rather than per query: `docs` is `N`, which every `idf`
    /// call reads, and walking the set per term would make the restriction cost more than the
    /// index it replaces.
    docs: u32,
    total_len: u64,
}

impl<'p> Subset<'p> {
    /// Restrict `inner` to `uids`. O(|uids|), and it builds nothing.
    pub fn of(inner: &'p dyn Postings, uids: &'p BTreeSet<Uid>) -> Subset<'p> {
        let mut docs = 0;
        let mut total_len = 0u64;
        for uid in uids {
            if let Some(len) = inner.doc_len(uid) {
                docs += 1;
                total_len += u64::from(len);
            }
        }
        Subset {
            inner,
            uids,
            docs,
            total_len,
        }
    }
}

impl Postings for Subset<'_> {
    fn df(&self, term: &str) -> u32 {
        self.inner
            .postings(term)
            .iter()
            .filter(|(uid, _)| self.uids.contains(uid))
            .count() as u32
    }

    fn postings(&self, term: &str) -> Vec<(Uid, u32)> {
        // Uid order is preserved because a filter preserves order, which is what the trait
        // promises and what a caller merging two lists in one pass relies on.
        let mut list = self.inner.postings(term);
        list.retain(|(uid, _)| self.uids.contains(uid));
        list
    }

    fn doc_len(&self, uid: &Uid) -> Option<u32> {
        if self.uids.contains(uid) {
            self.inner.doc_len(uid)
        } else {
            None
        }
    }

    fn docs(&self) -> u32 {
        self.docs
    }

    fn total_len(&self) -> u64 {
        self.total_len
    }

    fn tf(&self, term: &str, uid: &Uid) -> u32 {
        if self.uids.contains(uid) {
            self.inner.tf(term, uid)
        } else {
            0
        }
    }

    fn analyzer(&self) -> Option<&str> {
        self.inner.analyzer()
    }
}

/// The weighted text of a unit, as [`crate::Bm25`] indexes it.
///
/// Shared with `lexical.rs` through this crate's own `indexable` rather than reimplemented: the
/// weights are the one thing the two paths must not disagree about, because a difference there is
/// a different ranking that still looks like a ranking. (Named rather than linked: `indexable` is
/// `pub(crate)`, and a public item's documentation may not link to a private one.)
///
/// Public because a *persistent* implementation of the seam has to index the same text — the
/// `postings` table in `smysl_text::index` does — and a second implementation of "the gist four
/// times, the body twice, the detail once" is how two tables come to score the same corpus
/// differently. `None` for a uid the store does not hold.
pub fn document_text(store: &Store, uid: &Uid) -> Option<String> {
    let parts = indexable(store, uid)?;
    let mut out = String::new();
    for (text, weight) in parts {
        for _ in 0..crate::lexical::repeats(weight) {
            out.push_str(&text);
            out.push(' ');
        }
    }
    Some(out)
}

/// BM25 over a [`Postings`] and an [`Analyze`], scoring as [`crate::Bm25`] scores.
///
/// A view rather than an index: it owns nothing and builds nothing, which is the whole point —
/// constructing one is free, and the work it would have done is already on disk.
///
/// # Why it holds a store
///
/// SMYSL-2.4 §4.3.5 wrote `Bm25::over(postings, a) -> Bm25View<'_>`, and a view of that shape
/// cannot answer a query. [`Query`] filters on kernel type, status, schema and payload; none of
/// the four is a fact about a term, so none can come from a postings table. A view without them
/// would have to ignore `--kind`, `--min-status`, `--schema` and `--payload` — which is not a
/// faster answer to the question asked, it is a confident answer to a different one. So the
/// store comes too, and the filters are read from the units that scored rather than from every
/// unit in the corpus: the candidates of a query are the documents holding one of its terms, and
/// that is a far smaller set than the store.
pub struct Bm25View<'p> {
    postings: &'p dyn Postings,
    analyzer: &'p dyn Analyze,
    store: &'p Store,
}

impl<'p> Bm25View<'p> {
    /// Score `postings` for queries analysed by `analyzer`, over the units of `store`.
    ///
    /// Nothing checks that `analyzer` is the one the postings were built under, because nothing
    /// can: [`Postings::analyzer`] may be `None`, and an id this crate does not implement is
    /// still a valid id. [`Bm25View::agrees`] is the check, and a caller that can refuse should
    /// call it.
    pub fn new(
        postings: &'p dyn Postings,
        analyzer: &'p dyn Analyze,
        store: &'p Store,
    ) -> Bm25View<'p> {
        Bm25View {
            postings,
            analyzer,
            store,
        }
    }

    /// Whether the postings were built under the analyzer this view will split queries with.
    ///
    /// `true` when the table records no analyzer at all: that is an implementation that does not
    /// say, and refusing to search it would be inventing a disagreement rather than finding one.
    pub fn agrees(&self) -> bool {
        match self.postings.analyzer() {
            Some(id) => id == self.analyzer.id(),
            None => true,
        }
    }

    /// `ln(1 + (N - df + 0.5) / (df + 0.5))`, as the `bm25` crate's `Scorer::idf` computes it.
    fn idf(&self, df: u32) -> f32 {
        let df = df as f32;
        let numerator = self.postings.docs() as f32 - df + 0.5;
        let denominator = df + 0.5;
        (1f32 + (numerator / denominator)).ln()
    }

    /// `tf·(k1+1) / (tf + k1·((1-b) + b·(len/avgdl)))`, as the `bm25` crate's `Embedder::embed`
    /// computes it. Zero for a document that does not hold the term, which is the value its
    /// embedding would have lacked.
    fn value(&self, tf: u32, len: u32, avgdl: f32) -> f32 {
        if tf == 0 {
            return 0.0;
        }
        let tf = tf as f32;
        let numerator = tf * (K1 + 1.0);
        let denominator = tf + K1 * (1.0 - B + B * (len as f32 / avgdl));
        numerator / denominator
    }

    /// Every candidate document, with its length and its score, in uid order.
    ///
    /// One merge per distinct query term over a list that is already in uid order, rather than a
    /// map keyed by uid: the first shape this was written in built a `BTreeMap` per candidate to
    /// hold its term frequencies, and at 155k units with a term in every document that is 155k
    /// map allocations per query — 270 ms of p95 against a clause asking for 200.
    ///
    /// # Why the arithmetic is still the other path's arithmetic
    ///
    /// Scoring term-by-term across documents, rather than document-by-document across terms,
    /// looks like a reassociation of a float sum and is not: each document's accumulator
    /// receives its contributions in **query-term order**, which is the order the per-document
    /// loop added them in. The two differences are both exact. A term the document does not hold
    /// contributed `idf · 0.0`, and adding `0.0` to a non-negative float is the identity; a term
    /// the *corpus* does not hold has an empty postings list and contributed the same zero to
    /// every document, so it is skipped entirely.
    fn candidates(&self, terms: &[String], avgdl: f32) -> Vec<(Uid, u32, f32)> {
        let mut acc: Vec<(Uid, u32, f32)> = Vec::new();
        for term in terms {
            let df = self.postings.df(term);
            if df == 0 {
                continue;
            }
            let idf = self.idf(df);
            let list = self.postings.postings(term);
            let mut merged: Vec<(Uid, u32, f32)> = Vec::with_capacity(acc.len() + list.len());
            let (mut i, mut j) = (0usize, 0usize);
            while i < acc.len() || j < list.len() {
                let take_acc = match (acc.get(i), list.get(j)) {
                    (Some(a), Some(b)) => a.0 <= b.0,
                    (Some(_), None) => true,
                    _ => false,
                };
                if take_acc {
                    let (uid, len, score) = acc[i];
                    i += 1;
                    if list.get(j).is_some_and(|(u, _)| *u == uid) {
                        let tf = list[j].1;
                        j += 1;
                        merged.push((uid, len, score + idf * self.value(tf, len, avgdl)));
                    } else {
                        merged.push((uid, len, score));
                    }
                } else {
                    let (uid, tf) = list[j];
                    j += 1;
                    // The one lookup per document that is not avoidable: BM25 normalises by
                    // length, and a document is first seen here. Later terms reuse it.
                    let Some(len) = self.postings.doc_len(&uid) else {
                        continue;
                    };
                    merged.push((uid, len, idf * self.value(tf, len, avgdl)));
                }
            }
            acc = merged;
        }
        acc
    }

    /// What each query term contributed to this document's score.
    ///
    /// [`crate::Bm25`]'s own decomposition, reproduced: per distinct term, `idf·value` times the
    /// number of times the query wrote it, zero contributions dropped, ordered by contribution
    /// descending and then by term. The multiplication order is `(idf · value) · n` because that
    /// is the order the other path multiplies in, and three f32 products are not associative
    /// either.
    fn contributions(
        &self,
        uid: &Uid,
        len: u32,
        avgdl: f32,
        terms: &[String],
    ) -> Vec<(String, f32)> {
        let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
        for t in terms {
            *counts.entry(t.as_str()).or_insert(0) += 1;
        }
        let mut out: Vec<(String, f32)> = counts
            .into_iter()
            .filter_map(|(term, n)| {
                let have = self.postings.tf(term, uid);
                let one = self.idf(self.postings.df(term)) * self.value(have, len, avgdl);
                let c = one * n as f32;
                (c > 0.0).then(|| (term.to_string(), c))
            })
            .collect();
        out.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        out
    }

    /// Whether a unit passes the query's filters, read from the unit itself.
    ///
    /// The payload is decoded here rather than at index time, which is the opposite of what
    /// [`crate::Bm25`] does and right for the same reason: that path had already walked every
    /// unit, and this one has walked none. It decodes a payload per *candidate*.
    fn admits(&self, query: &Query, uid: &Uid) -> bool {
        let Some(unit) = self.store.get(uid) else {
            return false;
        };
        if !query.admits_schema(uid, &unit.core.schema, unit.core.status) {
            return false;
        }
        if query.payload.is_none() {
            return true;
        }
        let strings = unit
            .core
            .payload
            .as_deref()
            .map(smysl_core::surface::payload::payload_strings)
            .unwrap_or_default();
        query.admits_payload(&strings)
    }
}

impl Retriever for Bm25View<'_> {
    fn search(&self, query: &Query) -> Vec<Hit> {
        if query.limit == 0 {
            return Vec::new();
        }
        let terms = self.analyzer.terms(&query.text);
        let avgdl = self.postings.avgdl();
        let candidates = self.candidates(&terms, avgdl);

        // The filter is skipped when there is nothing to filter on, which is the ordinary query.
        // It costs a `Store::get` per candidate, and a candidate set is the whole corpus for a
        // term the whole corpus holds — so for `find "the"` this is the difference between
        // reading 155k units and reading none of them. `Query::filters_anything` is a method and
        // not a field test here so that a filter added to `Query` later cannot be forgotten at
        // this site.
        let filtered = query.filters_anything();
        let mut hits: Vec<Hit> = candidates
            .iter()
            .filter(|(uid, _, score)| {
                // A score of zero is not a hit: every candidate holds one of the query's terms,
                // so a zero here means the arithmetic cancelled, and padding a result to
                // `limit` with units that do not match reads as a ranking and is not one.
                *score > 0.0 && (!filtered || self.admits(query, uid))
            })
            .map(|(uid, _, score)| Hit::new(*uid, *score))
            .collect();

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.uid.cmp(&b.uid))
        });
        hits.truncate(query.limit);

        // After the truncation, as the other path does it: the decomposition is exact but not
        // free, and the candidates that did not survive are not going to be explained to
        // anybody.
        for hit in &mut hits {
            let Some(len) = self.postings.doc_len(&hit.uid) else {
                continue;
            };
            hit.terms = self.contributions(&hit.uid, len, avgdl, &terms);
        }
        hits
    }

    fn len(&self) -> usize {
        self.postings.docs() as usize
    }
}
