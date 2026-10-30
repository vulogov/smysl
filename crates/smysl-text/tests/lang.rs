//! Language identification, and the two things measuring it settled — TX-P2 step 1.
//!
//! `#![cfg(feature = "detect")]`, so a build without `lingua` skips the file rather than
//! failing to compile it. The segmenter is needed to cut the gold passages into sentences, so
//! `reader-txt` is not: these tests read the fixtures directly.

#![cfg(feature = "detect")]

use std::path::PathBuf;

use smysl_text::lang::{Detector, ID, MIN_CHARS};
use smysl_text::segment::Segmenter;
use smysl_text::LangTag;

fn gold(lang: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/segment/gold")
        .join(format!("{lang}.txt"));
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{lang}: {e}"))
        .chars()
        .filter(|c| *c != '‖')
        .collect()
}

const TIER_1: &[&str] = &["de", "en", "es", "fr", "ru"];

/// A passage is identified, which is the case §3.7 asks the detector for.
#[test]
fn every_tier_1_passage_is_identified() {
    let d = Detector::new();
    for lang in TIER_1 {
        let text = gold(lang);
        assert_eq!(
            d.of(&text).as_ref().map(LangTag::as_str),
            Some(*lang),
            "{lang}"
        );
    }
}

/// Every sentence of every passage is identified too — all 146 of them, at every length,
/// and 32 of those are shorter than the threshold this module will not detect below.
///
/// Which is why [`MIN_CHARS`] is **not** a measurement on this corpus: the development set
/// cannot locate the threshold, because it never fails. The passages are strongly marked
/// prose — Cyrillic, diacritics, function words — and a chat export is not. The number that
/// matters for the threshold comes from GE-T1's chat corpora, and this test is here to say
/// so out loud rather than to leave `MIN_CHARS = 30` looking derived.
#[test]
fn the_development_set_cannot_locate_the_short_text_threshold() {
    let d = Detector::new();
    let mut total = 0;
    let mut short = 0;
    for lang in TIER_1 {
        let text = gold(lang);
        let s = Segmenter::new(&LangTag::new(*lang).unwrap());
        for r in s.sentences(&text) {
            let slice = &text[r];
            total += 1;
            if slice.chars().count() < MIN_CHARS {
                short += 1;
            }
            assert_eq!(
                d.of(slice).as_ref().map(LangTag::as_str),
                Some(*lang),
                "{lang}: {slice:?}"
            );
        }
    }
    println!("{total} sentences identified, {short} of them under MIN_CHARS = {MIN_CHARS}");
    assert_eq!(total, 146);
    assert_eq!(short, 32, "the set must keep containing short sentences");
}

/// One-word utterances are where the detector stops being reliable, and this is the
/// measurement that justifies inheriting instead of guessing.
///
/// Each word below is a word of the language it is labelled with; several are also words of
/// another, which is the point — a chat message of one word carries almost no evidence, and
/// `lingua` answers anyway. Eight of these twenty disagree with the label. The count is
/// pinned because `lingua` is pinned exactly: a change here is a change of detector, and it
/// should be looked at rather than absorbed.
#[test]
fn a_one_word_utterance_is_not_evidence_of_a_language() {
    let d = Detector::new();
    let cases: &[(&str, &str)] = &[
        ("Hotel.", "de"),
        ("No.", "es"),
        ("Vamos.", "es"),
        ("Information.", "en"),
        ("Si.", "es"),
        ("Il est là.", "fr"),
        ("Das ist gut.", "de"),
        ("Это так.", "ru"),
        ("Merci.", "fr"),
        ("Danke.", "de"),
        ("Gracias.", "es"),
        ("Thanks.", "en"),
        ("Oui.", "fr"),
        ("Ja.", "de"),
        ("Da.", "ru"),
        ("Ok.", "en"),
        ("Natural.", "es"),
        ("Total.", "fr"),
        ("Normal.", "de"),
        ("Final.", "en"),
    ];
    let agreed = cases
        .iter()
        .filter(|(text, want)| d.of(text).as_ref().map(LangTag::as_str) == Some(*want))
        .count();
    println!(
        "short utterances: {agreed}/{} agreed with the label",
        cases.len()
    );
    assert_eq!(agreed, 12, "the detector's behaviour on short text moved");
    // And every one of them is below the threshold, so `rows` never asks.
    for (text, _) in cases {
        assert!(text.chars().count() < MIN_CHARS, "{text:?}");
    }
}

/// A short row inherits from the row before it; a long row is detected.
#[test]
fn a_short_row_inherits_the_language_of_the_row_before_it() {
    let d = Detector::new();
    let text = "Это предложение достаточно длинное, чтобы его язык был определён уверенно. \
                Да. \
                This sentence is also quite long and is plainly written in English. \
                Ok.";
    let rows = Segmenter::new(&LangTag::new("ru").unwrap()).sentences(text);
    assert_eq!(rows.len(), 4, "{rows:?}");
    let got: Vec<Option<String>> = d
        .rows(text, &rows)
        .into_iter()
        .map(|t| t.map(|t| t.as_str().to_string()))
        .collect();
    assert_eq!(
        got,
        [
            Some("ru".to_string()),
            Some("ru".to_string()),
            Some("en".to_string()),
            Some("en".to_string())
        ]
    );
}

/// A short row at the head of a text inherits forwards, because there is nothing behind it.
#[test]
fn a_short_first_row_inherits_from_the_first_row_that_was_identified() {
    let d = Detector::new();
    let text = "Oui. Cette phrase est assez longue pour que sa langue soit identifiée sans \
                hésitation.";
    let rows = Segmenter::new(&LangTag::new("fr").unwrap()).sentences(text);
    assert_eq!(rows.len(), 2);
    let got = d.rows(text, &rows);
    assert_eq!(got[0].as_ref().map(LangTag::as_str), Some("fr"));
    assert_eq!(got[1].as_ref().map(LangTag::as_str), Some("fr"));
}

/// A text that is nothing but short rows gets `None`, not a guess.
///
/// The honest answer: the caller has a manifest language and should use it. Returning the
/// detector's opinion of "Ja." would have put a language into a reading on no evidence.
#[test]
fn a_text_of_only_short_rows_is_not_guessed_at() {
    let d = Detector::new();
    let text = "Ja. Nein. Ok. Da.";
    let rows = Segmenter::new(&LangTag::new("de").unwrap()).sentences(text);
    assert_eq!(rows.len(), 4);
    assert!(d.rows(text, &rows).iter().all(Option::is_none));
}

/// The confidence list is ordered, and its head is what `of` returns.
#[test]
fn the_confidence_list_leads_with_the_language_that_was_chosen() {
    let d = Detector::new();
    let text = gold("es");
    let c = d.confidence(&text);
    assert_eq!(c.len(), 5, "one row per language this build carries");
    assert_eq!(c[0].0.as_str(), "es");
    assert_eq!(d.of(&text).as_ref().map(LangTag::as_str), Some("es"));
    for w in c.windows(2) {
        assert!(w[0].1 >= w[1].1, "{c:?}");
    }
}

/// The id carries the `lingua` version, because §7's risk is that the version decides an rdid.
#[test]
fn the_id_names_the_pinned_detector_version() {
    assert_eq!(ID, "smysl/lingua-1.8.0/1");
    assert_eq!(Detector::new().id(), ID);
    let pinned =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml"))
            .unwrap();
    assert!(
        pinned.contains("lingua = { version = \"=1.8.0\""),
        "the id says 1.8.0; the manifest must pin it"
    );
}
