//! Language identification: `smysl/lingua-1.8.0/1`, behind feature `detect`.
//!
//! A manifest records a language, and almost always it is told one: a reader's own metadata
//! (`<language>` in Zefania, `\id` in USFM, a JSON field), or the operator's `--lang`. This
//! module is the third answer, for a file that names none — and it is deliberately the last
//! one asked (SMYSL-2.4 §3.7).
//!
//! # Why this is never automatic
//!
//! A manifest's `lang` is part of its mid. If `Library::add` fell back to detection whenever
//! a file named no language, then the same file added by a build with `detect` and a build
//! without it would get **two different mids** — a feature flag deciding an identity. So
//! detection is something a caller *does*, and what it produces is an ordinary
//! `AddSpec::with_lang`: the guess becomes data before it becomes an identity, and the
//! manifest cannot tell whether a human or this module made it. What records that it was this
//! module is the recipe ([`ID`]), which is also where the `lingua` version lives, because
//! §7's risk is that a detector changing its mind changes a reading's rdid.
//!
//! # What it costs
//!
//! Five language features of `lingua`: 57 crates and 14.4 MB of compiled-in models, including
//! `rayon`, `getrandom` and `wasm-bindgen`. Measured in TX-P2 step 1, where §4.5 had left the
//! figure unverified. That is the whole argument for the feature gate — see this crate's
//! `Cargo.toml`.
//!
//! # Short text
//!
//! A sentence of ordinary prose is identified reliably — all 146 in
//! `fixtures/segment/gold/` are — but a one-word utterance is not: on twenty of them the
//! detector disagrees with the language the word was taken from eight times, and several of
//! those words belong to more than one of the five. A chat export is mostly those.
//! [`Detector::rows`] is therefore not "detect each row": a row shorter than [`MIN_CHARS`]
//! inherits its neighbours' language rather than guessing, which is §3.7's "minimum length
//! below which a segment inherits its neighbours' language". Row detection is for `mul`
//! manifests; a manifest with one language has nothing to ask.

use std::ops::Range;

use lingua::{Language, LanguageDetector, LanguageDetectorBuilder};

use smysl_core::ids::LangTag;

/// The detector id a recipe records. The `lingua` version is in it on purpose (§7).
pub const ID: &str = "smysl/lingua-1.8.0/1";

/// Below this many characters, a row inherits rather than guesses.
///
/// **Not derived from the development set**, and `tests/lang.rs` says so in a test rather
/// than leaving the number looking measured: on `fixtures/segment/gold/` every one of 146
/// sentences is identified correctly, 32 of them under this threshold, so that corpus cannot
/// locate it at all. What *is* measured is the failure mode — on twenty one-word utterances
/// the detector disagrees with the language the word was taken from eight times, and several
/// of those words belong to more than one of the five.
///
/// So 30 characters is a precaution with a measured reason and an unmeasured value: it is the
/// length below which a row's own evidence is worth less than its neighbour's. GE-T1's chat
/// corpora are what can price it, because a chat message is the short row this exists for.
pub const MIN_CHARS: usize = 30;

/// The languages this build can name, as (`lingua` language, tag).
///
/// The five of §3.7. `lingua`'s `Language` enum is feature-gated per language, so this table
/// cannot name one the build does not carry — which is why it is a table and not a match on a
/// string.
fn languages() -> [(Language, &'static str); 5] {
    [
        (Language::German, "de"),
        (Language::English, "en"),
        (Language::Spanish, "es"),
        (Language::French, "fr"),
        (Language::Russian, "ru"),
    ]
}

fn tag_of(l: Language) -> LangTag {
    let tag = languages()
        .iter()
        .find(|(cand, _)| *cand == l)
        .map(|(_, t)| *t)
        // `languages()` is the set the detector was built from, so this is unreachable; the
        // fallback is `und` rather than a panic because a guess is not worth a crash.
        .unwrap_or("und");
    LangTag::new(tag).unwrap_or_default()
}

/// A built detector.
///
/// Building one is cheap; the models load lazily on first use and are cached inside `lingua`,
/// so a caller identifying many texts should keep one detector rather than make one per text.
pub struct Detector {
    inner: LanguageDetector,
}

impl core::fmt::Debug for Detector {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Detector").field("id", &ID).finish()
    }
}

impl Default for Detector {
    fn default() -> Detector {
        Detector::new()
    }
}

impl Detector {
    pub fn new() -> Detector {
        let langs: Vec<Language> = languages().iter().map(|(l, _)| *l).collect();
        Detector {
            inner: LanguageDetectorBuilder::from_languages(&langs).build(),
        }
    }

    /// The id, for the recipe.
    pub fn id(&self) -> &'static str {
        ID
    }

    /// The language of a whole text, or `None` when the detector will not choose.
    ///
    /// `None` is not an error and must not become `und` here: a caller that cannot proceed
    /// without a language should say so in its own words, and `Library::add` already does
    /// (`lang`: "neither the source nor the caller named a language").
    pub fn of(&self, text: &str) -> Option<LangTag> {
        self.inner.detect_language_of(text).map(tag_of)
    }

    /// Every candidate with its confidence, highest first.
    ///
    /// For a caller that wants to show its work — `text add --detect-lang` printing what it
    /// decided and how narrowly is the difference between a guess a human can check and one
    /// they cannot.
    pub fn confidence(&self, text: &str) -> Vec<(LangTag, f64)> {
        self.inner
            .compute_language_confidence_values(text)
            .into_iter()
            .map(|(l, c)| (tag_of(l), c))
            .collect()
    }

    /// One language per row, with short rows inheriting from their neighbours.
    ///
    /// Rows are byte ranges into `text` — [`crate::segment::Segmenter::sentences`]'s output,
    /// or a reading's segment table. A row of [`MIN_CHARS`] or more is detected; a shorter one
    /// takes the nearest detected language, preferring the row before it, because a
    /// conversation changes language between turns far less often than within one.
    ///
    /// `None` survives only when *no* row in the text was long enough to detect, which is the
    /// honest answer for a text that is entirely short rows: the caller then has a manifest
    /// language and should use it.
    pub fn rows(&self, text: &str, rows: &[Range<usize>]) -> Vec<Option<LangTag>> {
        let mut out: Vec<Option<LangTag>> = rows
            .iter()
            .map(|r| {
                let slice = text.get(r.clone())?;
                if slice.chars().count() < MIN_CHARS {
                    None
                } else {
                    self.of(slice)
                }
            })
            .collect();

        // Backwards first, so that a short row at the head of the text can inherit from the
        // first identified row after it; then forwards, which is the preferred direction and
        // therefore the one that wins.
        let mut carry: Option<LangTag> = None;
        for slot in out.iter_mut().rev() {
            match slot {
                Some(l) => carry = Some(l.clone()),
                None => *slot = carry.clone(),
            }
        }
        let mut carry: Option<LangTag> = None;
        for (slot, row) in out.iter_mut().zip(rows) {
            let short = text
                .get(row.clone())
                .is_some_and(|s| s.chars().count() < MIN_CHARS);
            match (&slot, short) {
                (Some(_), false) => carry = slot.clone(),
                (_, true) if carry.is_some() => *slot = carry.clone(),
                _ => {}
            }
        }
        out
    }
}
