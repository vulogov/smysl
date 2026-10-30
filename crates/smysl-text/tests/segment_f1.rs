//! Sentence-boundary F1 against a marked-up gold set — TX-P2 step 1's exit test.
//!
//! The plan's bar is **F1 ≥ 0.97** for en, es, fr, de and **≥ 0.95** for ru, over *500
//! sentences per tier-1 language* (SMYSL-2.4 §6, TX-P2 step 1). This harness is that test; the
//! corpus it is run against here is not. `fixtures/segment/gold/` holds ~30 sentences per
//! language, written for the purpose, and it is a **development set**: it was authored
//! alongside the segmenter, so a score on it measures the lists and the three suppressions
//! against the cases their author thought of. A 500-sentence gold set from text nobody here
//! wrote is an outstanding corpus item, recorded beside GE-T1's Bibles in SMYSL-2.4 §6.
//!
//! Point the harness at one with `SMYSL_SEG_GOLD=<dir>`: a directory of `<lang>.txt` in the
//! format `fixtures/segment/README.md` describes. That is the whole difference between this
//! being a development check and being the exit test — no code changes.
//!
//! # What is measured
//!
//! A **boundary** is the offset just past a sentence's last text byte. Gold boundaries are
//! where the `‖` markers were; predicted ones are the ends of [`Segmenter::sentences`]. F1 is
//! over those two sets, which is the measure the plan names — not sentence accuracy, which
//! punishes one wrong boundary twice.

use std::collections::BTreeSet;
use std::path::PathBuf;

use smysl_text::segment::{Segmenter, ID, LANGUAGES};
use smysl_text::LangTag;

/// The floors this build *measures*, as distinct from the bars the phase exit asks for.
///
/// They are pinned so that a change to a list or a suppression has to be looked at: a drop is
/// a regression and a rise is worth propagating to the other four languages. They are **not**
/// the plan's 0.97/0.95 — those are claims about a 500-sentence corpus, and asserting them
/// against 30 sentences the author wrote would be the kind of green gate this repository keeps
/// finding and deleting.
/// Four of the five are exact on this set. Russian is 0.963 and the two boundaries it misses
/// are the `г.` and `мин.` trade-off `segment.rs` documents — a row that holds an address
/// together loses a sentence that ends on a year. That is the one number here worth looking
/// at again when a real corpus arrives.
const MEASURED: &[(&str, f64)] = &[
    ("de", 1.00),
    ("en", 1.00),
    ("es", 1.00),
    ("fr", 1.00),
    ("ru", 0.96),
];

struct Score {
    gold: usize,
    found: usize,
    hit: usize,
}

impl Score {
    fn precision(&self) -> f64 {
        if self.found == 0 {
            0.0
        } else {
            self.hit as f64 / self.found as f64
        }
    }
    fn recall(&self) -> f64 {
        if self.gold == 0 {
            0.0
        } else {
            self.hit as f64 / self.gold as f64
        }
    }
    fn f1(&self) -> f64 {
        let (p, r) = (self.precision(), self.recall());
        if p + r == 0.0 {
            0.0
        } else {
            2.0 * p * r / (p + r)
        }
    }
}

fn gold_dir() -> PathBuf {
    match std::env::var_os("SMYSL_SEG_GOLD") {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/segment/gold")
            .canonicalize()
            .expect("fixtures/segment/gold"),
    }
}

/// Strip the markers, and keep where they were.
fn read_gold(marked: &str) -> (String, BTreeSet<usize>) {
    let mut text = String::with_capacity(marked.len());
    let mut at = BTreeSet::new();
    for ch in marked.chars() {
        if ch == '‖' {
            at.insert(text.len());
        } else {
            text.push(ch);
        }
    }
    (text, at)
}

fn score(lang: &str, marked: &str) -> Score {
    let (text, gold) = read_gold(marked);
    let s = Segmenter::new(&LangTag::new(lang).unwrap());
    let found: BTreeSet<usize> = s.sentences(&text).into_iter().map(|r| r.end).collect();
    Score {
        gold: gold.len(),
        found: found.len(),
        hit: gold.intersection(&found).count(),
    }
}

#[test]
fn the_segmenter_scores_what_this_build_measured_on_the_development_set() {
    let dir = gold_dir();
    println!("segmenter {ID}, gold set {}", dir.display());
    println!("  lang  gold  found   hit      P      R     F1  floor");
    let mut below = Vec::new();
    for (lang, floor) in MEASURED {
        let marked = std::fs::read_to_string(dir.join(format!("{lang}.txt")))
            .unwrap_or_else(|e| panic!("{lang}.txt: {e}"));
        let s = score(lang, &marked);
        println!(
            "  {lang:>4}  {:>4}  {:>5}  {:>4}  {:.3}  {:.3}  {:.3}  {floor:.2}",
            s.gold,
            s.found,
            s.hit,
            s.precision(),
            s.recall(),
            s.f1()
        );
        // Rounded to three places before comparing: a floor is a claim about the measurement,
        // and a measurement that differs from it in the fifteenth decimal is the same number.
        if (s.f1() * 1000.0).round() / 1000.0 < *floor {
            below.push(format!("{lang}: F1 {:.3} below {floor:.2}", s.f1()));
        }
    }
    assert!(below.is_empty(), "{below:#?}");
}

/// Every tier-1 language has a gold file, so a language cannot be added to the segmenter
/// without being measured.
#[test]
fn every_language_with_a_list_is_measured() {
    let dir = gold_dir();
    for lang in LANGUAGES {
        assert!(
            dir.join(format!("{lang}.txt")).exists(),
            "{lang} has an abbreviation list and no gold file"
        );
        assert!(
            MEASURED.iter().any(|(l, _)| l == lang),
            "{lang} has a gold file and no floor"
        );
    }
    assert_eq!(MEASURED.len(), LANGUAGES.len());
}

/// The marker never appears in the text the harness measures, so a gold file cannot smuggle
/// one into the input and make a boundary out of nothing.
#[test]
fn the_markers_are_the_only_thing_the_harness_removes() {
    let dir = gold_dir();
    for lang in LANGUAGES {
        let marked = std::fs::read_to_string(dir.join(format!("{lang}.txt"))).unwrap();
        let (text, at) = read_gold(&marked);
        assert!(!text.contains('‖'), "{lang}");
        assert_eq!(at.len(), marked.matches('‖').count(), "{lang}");
        assert_eq!(
            text.len() + at.len() * '‖'.len_utf8(),
            marked.len(),
            "{lang}"
        );
        // A marker sits immediately after a sentence's last byte, never in whitespace: that is
        // the format's one rule, and a gold file that broke it would score the segmenter
        // against a boundary no segmenter could produce.
        for off in &at {
            let before = text[..*off].chars().next_back();
            assert!(
                before.is_some_and(|c| !c.is_whitespace()),
                "{lang}: a marker at {off} follows whitespace"
            );
        }
    }
}
