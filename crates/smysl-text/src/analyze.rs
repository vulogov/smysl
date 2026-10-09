//! Analyzer chains: `smysl/an-<lang>/1`.
//!
//! A term is what `find` matches, what BM25 weighs and what a lexical proposition class is
//! computed over. Which terms a text has is a decision about a *language*: `книгами` and
//! `книга` are the same word in Russian and two strings everywhere else, and `Häuser` and
//! `Haus` are the same word in German only if something folds the umlaut and stems the plural.
//! This module is that decision, one chain per tier-1 language (SMYSL-2.4 §3.7).
//!
//! # The chain
//!
//! | step | what | by |
//! |---|---|---|
//! | 1 | UAX #29 word boundaries | `unicode-segmentation`'s `unicode_words`, which also drops the punctuation — so Spanish `¿` and `¡` are gone before any rule has to name them |
//! | 2 | full case folding | `caseless`, which is **not** `to_lowercase`: it maps `ß` to `ss`, so German `Straße` and `Strasse` are one term. That is §3.7's "de ß→ss", and it costs no rule of ours |
//! | 3 | `ё` → `е` | ours. Case folding leaves `ё` alone, and Russian typography uses it in perhaps one word in a thousand — a corpus that writes `ёлка` and a query that writes `елка` are asking about the same tree |
//! | 4 | Snowball stem, **emitted beside the surface term** | `rust-stemmers`, behind feature `stem` |
//!
//! Step 4 is the one worth stating twice: the stem is an *additional* term, never a
//! replacement. A query for `книгами` and a query for `книга` both hit, and a query for the
//! exact surface form still ranks it above its relatives, because the surface term is in the
//! index too. Replacing would have been smaller and would have made `find "Häuser"` and
//! `find "Haus"` indistinguishable, which is a loss of information no query can undo.
//!
//! Step 4 is behind a feature because `rust-stemmers` 1.2.0 hard-depends on `serde` and the
//! pure core carries no serde stack (OQ-37) — this crate's `Cargo.toml` has the argument. A
//! build without it has one chain, `smysl/an/1`, for every language: steps 1 to 3 are the
//! same in all five, so what is lost is the stemming and not the analysis, and the id says
//! which was done. `cli` turns it on, where `reader-json` has already paid for serde.
//!
//! A stem equal to its folded surface is **not** emitted twice. Most English words stem to
//! themselves, and emitting both would double their term frequency — which is not a tie-break,
//! it is a different ranking for half the vocabulary.
//!
//! # The id
//!
//! `Chain::id` is `smysl/an-ru/1` and the like, and it goes wherever terms are used: a
//! recipe's conditions, an index's header, a saved query. Two indexes built under different
//! ids are not comparable and the id is how that is noticed rather than inferred from a bad
//! answer. The dependency pins in `Cargo.toml` are what make the id mean one algorithm.
//!
//! # Not here yet
//!
//! `impl smysl_retrieve::Analyze for Chain`. The seam is TX-P4 step 2, and the chain is
//! TX-P2 step 1 — it is wanted by the segmenter's callers before retrieval has the trait to
//! hang it on. [`Chain::terms`] is the method that trait will call, with that signature.

#[cfg(feature = "stem")]
use rust_stemmers::{Algorithm, Stemmer};
use unicode_segmentation::UnicodeSegmentation;

use smysl_core::ids::LangTag;

/// The chains, as (primary subtag, id, Snowball algorithm).
///
/// The five tier-1 languages of §3.7, which are also the five `lingua` is built for and the
/// five with an abbreviation list. One table rather than three parallel ones: a language that
/// gains a chain and not a list is a language whose sentences and terms disagree about what
/// language it is.
#[cfg(feature = "stem")]
const CHAINS: &[(&str, &str, Algorithm)] = &[
    ("de", "smysl/an-de/1", Algorithm::German),
    ("en", "smysl/an-en/1", Algorithm::English),
    ("es", "smysl/an-es/1", Algorithm::Spanish),
    ("fr", "smysl/an-fr/1", Algorithm::French),
    ("ru", "smysl/an-ru/1", Algorithm::Russian),
];

/// The id of the chain with no stemmer: folding and word segmentation only.
///
/// What a language outside tier 1 gets. It is a real analyzer — folding and UAX #29 are the
/// two steps that are the same in every language — and it is recorded under its own id so
/// that an index built for Portuguese does not claim to have been stemmed.
pub const ID_PLAIN: &str = "smysl/an/1";

/// Every id this module can record — one without `stem`, six with it.
///
/// A build's own list, not the format's: an id this build cannot produce has no business in
/// an index header, and a caller comparing a recorded id against this iterator is asking
/// exactly the right question ("could this build have written that?").
pub fn ids() -> impl Iterator<Item = &'static str> {
    #[cfg(feature = "stem")]
    let rest = CHAINS.iter().map(|(_, id, _)| *id);
    #[cfg(not(feature = "stem"))]
    let rest = std::iter::empty();
    std::iter::once(ID_PLAIN).chain(rest)
}

/// One language's analyzer chain.
///
/// Holds a `Stemmer`, which is a table lookup over a compiled-in automaton — cheap to build,
/// not free, so a caller indexing a store should build one chain per language and keep it
/// rather than one per unit.
/// `Debug` by hand: `rust_stemmers::Stemmer` does not implement it, and a chain is identified
/// by its id rather than by the automaton behind it — so the id is what a debug print should
/// show anyway.
pub struct Chain {
    lang: Option<&'static str>,
    id: &'static str,
    #[cfg(feature = "stem")]
    stemmer: Option<Stemmer>,
}

impl core::fmt::Debug for Chain {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Chain")
            .field("id", &self.id)
            .field("lang", &self.lang)
            .finish()
    }
}

impl Chain {
    /// The chain for a language tag, by its primary subtag; [`Chain::plain`] for the rest.
    ///
    /// `ru-RU` and `ru` get the same chain and the same id, because a stemmer is about a
    /// language and not about a region. This is the same negotiation
    /// [`crate::segment::Segmenter::new`] does, deliberately — a unit's `lang` chooses both,
    /// and two different answers from one tag would be a bug nobody could see.
    pub fn new(lang: &LangTag) -> Chain {
        #[cfg(not(feature = "stem"))]
        {
            let _ = lang;
            return Chain::plain();
        }
        #[cfg(feature = "stem")]
        {
            let primary = lang
                .as_str()
                .split('-')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            match CHAINS.iter().find(|(l, _, _)| *l == primary) {
                Some((l, id, alg)) => Chain {
                    lang: Some(l),
                    id,
                    stemmer: Some(Stemmer::create(*alg)),
                },
                None => Chain::plain(),
            }
        }
    }

    /// Folding and word segmentation, with no stemmer — and the only chain a build without
    /// feature `stem` has.
    pub fn plain() -> Chain {
        Chain {
            lang: None,
            id: ID_PLAIN,
            #[cfg(feature = "stem")]
            stemmer: None,
        }
    }

    /// The id recorded wherever these terms are used.
    pub fn id(&self) -> &'static str {
        self.id
    }

    /// The language whose stemmer is loaded, or `None` for [`Chain::plain`].
    pub fn lang(&self) -> Option<&'static str> {
        self.lang
    }

    /// Steps 2 and 3 for one word: full case folding, then `ё` → `е`.
    ///
    /// Exposed because a caller matching a term against an index must fold the same way, and
    /// a second implementation of "the same way" is how a query comes to miss what was
    /// indexed.
    pub fn fold(&self, word: &str) -> String {
        let folded = caseless::default_case_fold_str(word);
        if folded.contains('ё') {
            folded.replace('ё', "е")
        } else {
            folded
        }
    }

    /// The terms of a text, in order, with stems beside their surface forms.
    ///
    /// Duplicates are kept: term frequency is what BM25 weighs, so a text that says a word
    /// twice must yield it twice.
    pub fn terms(&self, text: &str) -> Vec<String> {
        let mut out = Vec::new();
        for word in text.unicode_words() {
            let folded = self.fold(word);
            #[cfg(feature = "stem")]
            if let Some(st) = &self.stemmer {
                let stem = st.stem(&folded).into_owned();
                if stem != folded {
                    out.push(folded);
                    out.push(stem);
                    continue;
                }
            }
            out.push(folded);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(lang: &str) -> Chain {
        Chain::new(&LangTag::new(lang).unwrap())
    }

    /// The three cases SMYSL-2.4 §5.1 names, measured rather than quoted.
    #[test]
    #[cfg(feature = "stem")]
    fn the_stems_the_rfc_names_are_the_stems_this_build_produces() {
        assert_eq!(chain("ru").terms("книгами"), ["книгами", "книг"]);
        assert_eq!(
            chain("fr").terms("continuellement"),
            ["continuellement", "continuel"]
        );
        assert_eq!(chain("de").terms("Häuser"), ["häuser", "haus"]);
    }

    /// A stem is emitted *beside* the surface term, never instead of it.
    ///
    /// The property that makes `find` able to tell `Häuser` from `Haus` while still matching
    /// one against the other.
    #[test]
    #[cfg(feature = "stem")]
    fn a_stem_never_replaces_the_surface_term() {
        let t = chain("ru").terms("книгами и книга");
        assert_eq!(t, ["книгами", "книг", "и", "книга", "книг"]);
        // Every surface form is present, in order.
        assert!(t.contains(&"книгами".to_string()));
        assert!(t.contains(&"книга".to_string()));
    }

    /// A word that stems to itself yields one term, not the same term twice.
    #[test]
    fn a_word_that_stems_to_itself_is_not_counted_twice() {
        assert_eq!(chain("en").terms("book"), ["book"]);
        assert_eq!(chain("en").terms("the book book"), ["the", "book", "book"]);
    }

    /// Full case folding, which is not lowercasing: `ß` is two letters when folded.
    #[test]
    fn folding_makes_strasse_and_strae_one_term() {
        assert_eq!(chain("de").fold("Straße"), "strasse");
        assert_eq!(chain("de").terms("Straße"), chain("de").terms("Strasse"));
        // And `to_lowercase` would not have: this is the line that says why `caseless` is a
        // dependency rather than a method call.
        assert_eq!("Straße".to_lowercase(), "straße");
    }

    /// `ё` → `е`, which case folding does not do.
    #[test]
    fn a_russian_text_that_writes_yo_matches_a_query_that_does_not() {
        assert_eq!(chain("ru").fold("Ёлка"), "елка");
        assert_eq!(chain("ru").terms("Ёлка"), chain("ru").terms("елка"));
        // It is applied in every chain, not only `ru`: a `mul` manifest's English sentence
        // may still quote a Russian word, and the fold has to agree with the index.
        assert_eq!(chain("en").fold("ёж"), "еж");
    }

    /// Punctuation is gone at step 1, so Spanish needs no rule for `¿` and `¡`.
    ///
    /// `qué` brings its stem `que` with it, which is step 4 doing its job and not an artefact
    /// of the punctuation: the accent is part of the word, and the Spanish stemmer removes it.
    #[test]
    #[cfg(feature = "stem")]
    fn spanish_opening_marks_are_dropped_by_word_segmentation() {
        assert_eq!(
            chain("es").terms("¿Qué es esto?"),
            ["qué", "que", "es", "esto"]
        );
        assert_eq!(chain("es").terms("¡Vamos!"), ["vamos", "vam"]);
    }

    /// French elision is kept: `aujourd’hui` is one word, under either apostrophe.
    #[test]
    fn french_elision_stays_one_term() {
        assert_eq!(chain("fr").terms("aujourd’hui"), ["aujourd’hui"]);
        assert_eq!(chain("fr").terms("aujourd'hui"), ["aujourd'hui"]);
        // One *word*, and therefore one surface term — with its stem beside it, as every
        // other word has. The elision is kept inside the term rather than cut at the
        // apostrophe, which is what §3.7 means by "fr elision kept".
        assert_eq!(chain("fr").terms("l’homme")[0], "l’homme");
    }

    /// Without the stemmer there is one chain, and `Chain::id` is how a caller knows.
    ///
    /// The thing this test exists to prevent: a build that folded but did not stem while
    /// reporting `smysl/an-ru/1`, which would have made two incomparable indexes claim the
    /// same analyzer.
    #[test]
    #[cfg(not(feature = "stem"))]
    fn a_build_without_the_stemmer_has_one_chain_and_names_it() {
        for lang in crate::segment::LANGUAGES {
            let c = chain(lang);
            assert_eq!(c.id(), ID_PLAIN, "{lang}");
            assert_eq!(c.lang(), None, "{lang}");
        }
        assert_eq!(chain("ru").terms("книгами"), ["книгами"]);
    }

    /// The stemmed chains line up with the segmenter's languages.
    #[test]
    #[cfg(feature = "stem")]
    fn a_region_subtag_costs_neither_a_chain_nor_a_different_id() {
        assert_eq!(chain("RU").lang(), Some("ru"));
        assert_eq!(chain("en-GB").id(), "smysl/an-en/1");
    }

    /// A language outside tier 1 is analyzed, not refused — and says so in its id.
    #[test]
    fn a_language_outside_tier_1_folds_without_stemming() {
        let c = chain("pt-BR");
        assert_eq!(c.id(), ID_PLAIN);
        assert_eq!(c.lang(), None);
        assert_eq!(c.terms("Os Livros"), ["os", "livros"]);
        assert_eq!(Chain::plain().id(), ID_PLAIN);
    }

    /// The ids are the shape everything that records one assumes, and each appears once.
    #[test]
    fn every_chain_id_is_named_once_and_shaped_like_an_id() {
        let all: Vec<&str> = ids().collect();
        let mut sorted = all.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), all.len());
        #[cfg(feature = "stem")]
        assert_eq!(all.len(), CHAINS.len() + 1);
        #[cfg(not(feature = "stem"))]
        assert_eq!(all, [ID_PLAIN]);
        for id in all {
            let (prefix, version) = id.rsplit_once('/').unwrap();
            assert!(prefix.starts_with("smysl/an"), "{id}");
            assert_eq!(version, "1", "{id}");
        }
        // A region subtag costs neither a chain nor a different id.
        assert_eq!(chain("ru-RU").id(), chain("ru").id());
    }

    /// The chain and the segmenter answer the same tag with the same language.
    ///
    /// They are chosen by one field — a unit's `lang` — and a text segmented as Russian and
    /// analyzed as something else would put terms in the index that no query could reach.
    #[test]
    #[cfg(feature = "stem")]
    fn the_chain_and_the_segmenter_agree_about_every_tier_1_tag() {
        for lang in crate::segment::LANGUAGES {
            let tag = LangTag::new(*lang).unwrap();
            assert_eq!(
                Chain::new(&tag).lang(),
                crate::segment::Segmenter::new(&tag).lang(),
                "{lang}"
            );
        }
    }
}
