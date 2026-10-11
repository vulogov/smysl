//! The analyzer seam: what a term is, and who decides (SMYSL-2.4 §4.3.5, TX-P4 step 2).
//!
//! # Why a seam over a function
//!
//! Which terms a text has is a decision about a *language*. `tokenize` splits on
//! non-alphanumerics and lowercases, which is right for identifiers and for English prose and
//! wrong for Russian morphology; `smysl_text::analyze::Chain` folds case the Unicode way, maps
//! `ё` to `е` and stems, which is right for the five tier-1 languages and brings
//! `unicode-segmentation`, `caseless` and optionally `rust-stemmers` with it. Both are correct
//! answers to different questions, and neither belongs inside a scorer.
//!
//! So an index records *which* analyzer produced its terms and a query is split by the same
//! one. [`Analyze::id`] is what makes that checkable rather than assumed: two indexes built
//! under different ids are not comparable, and the id is how that is noticed instead of
//! inferred from a bad answer.
//!
//! # Why the trait lives here and the chains live in `smysl-text`
//!
//! This crate is the one that *consumes* terms — BM25 weighs them, a postings table stores
//! them. The chains are a fact about language and belong with normalisation, segmentation and
//! the readers. A trait in the consumer and the implementations in the producer is the shape
//! that keeps `smysl-retrieve` free of `unicode-segmentation` and a stemmer, and it is why the
//! dependency runs `smysl-text` → `smysl-retrieve` and not the other way.
//!
//! # Determinism
//!
//! An implementation MUST be a pure function of its text: `terms` is called at index time and
//! again at query time, and a term that is not reproduced is a term that cannot be matched.
//! That is the same rule `Retriever` is under (rule D), stated where an implementor will read
//! it.

use crate::tokenize::Tokenizer;

/// `Tokenizer::plain`'s id (1.10).
pub const ID_PLAIN: &str = "smysl/plain/1";

/// `Tokenizer::folding`'s id (1.10).
///
/// The `-en` is in the name because the fold is English and always was: `fold_suffix` strips
/// English suffixes from whatever it is handed. An index built under this id over a Spanish
/// corpus is a mistake that the id records.
pub const ID_FOLD_EN: &str = "smysl/fold-en/1";

/// Something that turns a text into terms.
///
/// The `id` is part of the contract, not a label: a postings table, a saved query and a recipe
/// all record it, and a reader of any of the three needs to know whether the terms in front of
/// it are the terms its query will produce.
pub trait Analyze {
    /// A stable identifier for this analysis, `vendor/name/version`.
    ///
    /// Stable means: the same id implies the same terms for the same text, in this release and
    /// in every later one. An implementation that changes what it emits takes a new id —
    /// `smysl/an/2`, not `smysl/an/1` quietly improved — because the id is what an index
    /// already on disk is compared against.
    fn id(&self) -> &str;

    /// The terms of `text`, in order, with repetition kept.
    ///
    /// Order and repetition both matter: BM25's term frequency is a count of occurrences, and a
    /// decomposition of a score sums over the query's tokens as they were written. An
    /// implementation that deduplicated here would make a two-word query and a one-word query
    /// score alike.
    fn terms(&self, text: &str) -> Vec<String>;
}

impl Analyze for Tokenizer {
    fn id(&self) -> &str {
        if self.folds() {
            ID_FOLD_EN
        } else {
            ID_PLAIN
        }
    }

    fn terms(&self, text: &str) -> Vec<String> {
        Tokenizer::terms(self, text)
    }
}

/// Every analyzer id this crate implements, in a stable order (1.10).
///
/// For a command that has to say what `--analyzer` takes. `smysl-text` has its own list —
/// `smysl_text::analyze::ids` — and a caller offering both offers both lists, because neither
/// crate can name the other's chains.
pub fn ids() -> impl Iterator<Item = &'static str> {
    [ID_PLAIN, ID_FOLD_EN].into_iter()
}

/// The analyzer a caller named, when it is one of this crate's two (1.10).
///
/// `None` for anything else, including an id this crate will never implement: a caller that
/// supports more chains tries them in turn, and one that does not reports the id it was given
/// rather than falling back to a default. A silent fallback would build an index under an
/// analyzer nobody asked for and record a different id than the one on the command line.
pub fn by_id(id: &str) -> Option<Tokenizer> {
    match id {
        ID_PLAIN => Some(Tokenizer::plain()),
        ID_FOLD_EN => Some(Tokenizer::folding()),
        _ => None,
    }
}
