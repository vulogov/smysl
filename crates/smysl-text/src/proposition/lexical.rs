//! `lexical`: BM25 neighbours within one language, filtered by adjacency (§3.6).
//!
//! # What it proposes, and what it is for
//!
//! Two units are proposed when each is among the other's — or at least among one of the
//! other's — top `k` BM25 neighbours over the store, both are written in the same language,
//! and they are located in the same or a neighbouring place. The first condition is the
//! proposal; the second and third are what keep it from being noise.
//!
//! This is the engine for two translations in one language, two editions, two extraction runs
//! whose gists are worded differently — everything [`super::IdenticalSpan`] cannot see because
//! the bytes or the wording differ by one character.
//!
//! # Adjacency is load-bearing, which S0 measured and did not assume
//!
//! S0's cosine proposer reached precision 0.789 same-language and 0.427 cross-lingual at
//! recall 0.7; **with adjacency, 0.835 and 0.712** (SMYSL-2.2 §3.7, T6). §3.6 draws the
//! conclusion: adjacency is load-bearing rather than a filter, because it does not rank
//! better — it removes the pairs a ranker is worst at, and it keeps doing that once TX-P10's
//! embeddings land.
//!
//! So [`Lexical::adjacent_only`] defaults to **on**. It can be turned off, and the reason it
//! is a field rather than a constant is that the 0.789 figure has to be reproducible: an
//! engine that could only be run in its good configuration could never be shown to be better
//! than its bad one.
//!
//! # Same language, and why that is a hard condition here
//!
//! BM25 is a term overlap, and two languages share almost no terms — except the ones that
//! matter least, proper nouns and numbers, which is how a cross-lingual lexical match happens
//! at all and why it is wrong when it does. Cross-lingual classes wait for TX-P10's S1
//! embeddings (§3.6); until then the engine for two languages is [`super::Anchored`], where a
//! versification alignment is exact and a term overlap is a coincidence.
//!
//! The language is the unit's own (core key 9, FC-1), not its manifest's. A gist written in
//! English about a Russian passage is an English gist — that is what `SMY-W436` reports — and
//! indexing it as Russian would make the engine's one hard condition read the wrong field.
//! A unit with no `lang` is in no language group and is not proposed: absent means "the
//! language of the view that presents it", which is not a fact about the unit.
//!
//! # Not symmetric, and taken as symmetric anyway
//!
//! "`b` is in `a`'s top `k`" does not imply the reverse, since BM25 scores are not symmetric
//! and a long gist retrieves differently from a short one. A same-as claim is symmetric, so
//! one direction is enough to propose it and the proposal says which direction found it. The
//! alternative — requiring both — makes `k` mean something different for every pair of
//! lengths, and S0's candidate sets were one-directional (`find -n 5`).

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::ids::{LangTag, Uid};
use smysl_graph::Store;
use smysl_retrieve::{Bm25, Query, Retriever};

use super::{adjacent_locators, candidates, place, Engine, Place, Proposal};

/// The `lexical` engine. Pure: it builds its own index over the store it is given.
#[derive(Debug, Clone, Copy)]
pub struct Lexical {
    /// How many neighbours per unit. S0 used 5 (`find -n 5`), which is the default.
    pub k: usize,
    /// Whether to keep only pairs in the same or a neighbouring place.
    ///
    /// On by default; see the module docs for the measurement that decided it.
    pub adjacent_only: bool,
}

impl Default for Lexical {
    fn default() -> Lexical {
        Lexical {
            k: 5,
            adjacent_only: true,
        }
    }
}

impl Engine for Lexical {
    fn id(&self) -> &'static str {
        "lexical"
    }

    fn propose(&self, store: &Store, scope: Option<&BTreeSet<Uid>>) -> Vec<Proposal> {
        if self.k == 0 {
            return Vec::new();
        }
        let mut by_lang: BTreeMap<LangTag, BTreeSet<Uid>> = BTreeMap::new();
        let mut gists: BTreeMap<Uid, String> = BTreeMap::new();
        let mut places: BTreeMap<Uid, Place> = BTreeMap::new();
        for (uid, unit) in candidates(store, scope) {
            let Some(lang) = unit.core.lang.clone() else {
                continue;
            };
            by_lang.entry(lang).or_default().insert(uid);
            gists.insert(uid, unit.core.gist.clone());
            if let Some(at) = place(unit) {
                places.insert(uid, at);
            }
        }

        // One index over the whole store, scored per language group through `Query::within`.
        // Fitting over the whole corpus rather than per group is deliberate: an inverse
        // document frequency is a statement about the corpus, and refitting per language would
        // make a term's weight depend on how many languages happened to be ingested.
        let index = Bm25::index(store);
        let mut out: BTreeSet<Proposal> = BTreeSet::new();
        for (lang, group) in &by_lang {
            if group.len() < 2 {
                continue;
            }
            for uid in group {
                let Some(text) = gists.get(uid) else {
                    continue;
                };
                // `k + 1`: a unit is its own best neighbour, and a budget of `k` that spent one
                // on itself would be a budget of `k - 1` silently.
                let query = Query::new(text.clone(), self.k + 1).within(group.iter().copied());
                for hit in index.search(&query) {
                    if hit.uid == *uid {
                        continue;
                    }
                    if self.adjacent_only && !adjacent(places.get(uid), places.get(&hit.uid)) {
                        continue;
                    }
                    let why = format!(
                        "a top-{} BM25 neighbour in {lang} at {:.3}{}",
                        self.k,
                        hit.score,
                        match (places.get(uid), places.get(&hit.uid)) {
                            (Some(x), Some(y)) => match (&x.locator, &y.locator) {
                                (Some(a), Some(b)) if a == b => format!(", both at {a}"),
                                (Some(a), Some(b)) => format!(", at {a} and {b}"),
                                _ => String::new(),
                            },
                            _ => String::new(),
                        }
                    );
                    if let Some(p) = Proposal::new(*uid, hit.uid, why) {
                        out.insert(p);
                    }
                }
            }
        }
        // Deduplicated to one proposal per pair: both directions can find the same pair with
        // different scores, and two rows for one claim would make `attested:n` count one
        // engine twice. The surviving one is the smaller `why` in sort order, which is stable
        // and therefore reproducible — not "the better score", which would be a judgement this
        // engine is not entitled to make.
        let mut once: BTreeMap<(Uid, Uid), Proposal> = BTreeMap::new();
        for p in out {
            once.entry((p.a, p.b)).or_insert(p);
        }
        once.into_values().collect()
    }
}

/// Whether two places are the same or neighbouring ones.
///
/// By locator where both have one, and by byte range otherwise: two spans of one part that
/// touch or overlap are adjacent in the only sense available without a structure tree. Two
/// units with no place at all are **not** adjacent — an engine whose filter passed everything
/// it could not evaluate would report the unfiltered precision under the filtered name.
fn adjacent(a: Option<&Place>, b: Option<&Place>) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return false;
    };
    match (&a.locator, &b.locator) {
        (Some(x), Some(y)) => adjacent_locators(x, y),
        _ => match (a.tid == b.tid, a.span, b.span) {
            (true, Some((s1, e1)), Some((s2, e2))) => s1 <= e2 && s2 <= e1,
            _ => false,
        },
    }
}
