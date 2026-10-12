//! TX-P5 step 3b's exit clause: the asserted-as-reported rate over a probe set (§3.4, §6).
//!
//! # What the clause asks, and why it cannot pass here
//!
//! S0 measured reported speech extracted *as* asserted in 11.1–24.5% of attribution-bearing
//! units (T8), every cell above the 10% threshold, and the arm that added a holder paragraph to
//! the prompt did no better (T9 does not hold). So §1.1 item 4 moved `text:mode` out of the
//! prompt and into the converter, and TX-P5's exit gained a gate: **below 10%** over a probe set
//! of attribution-bearing passages with known reported speech.
//!
//! That probe set is **material this repository does not hold**. Authoring one here would make it
//! a development set — the passages would be chosen by the person writing the detector, which is
//! the same objection GE-T1's Bibles, TX-P2's boundary gold and GE-T13's chronology record. So
//! this harness measures the clause when a probe set is supplied and says the clause is *owed*
//! when it is not, rather than passing quietly:
//!
//! ```text
//! SMYSL_PROBE_SET=/path/to/probes cargo test -p smysl-ingest --features text --test probe_set
//! ```
//!
//! The directory holds `probes.tsv` — `file<TAB>quote<TAB>expected mode`, one probe per line,
//! `#` comments ignored — and the passage files it names. A passage may bring its own marks in
//! `<file>.marks`, one `start-end` byte range per line, which is what a reader that read USFM's
//! `\wj` or OSIS's `<q who>` would supply; without one, the marks are
//! [`smysl_ingest::text::quoted_spans`] over the passage's own punctuation.
//!
//! # What the measurement will say, and it is worth knowing before it runs
//!
//! For a modern edition that punctuates its quotations, the scan finds them and the rate should
//! be near zero. For a 1769 Bible it finds nothing at all — Psalm 14:1 marks its reported clause
//! with a comma and a capital letter — and **no reader in this build marks one either**. On such
//! a probe set this gate fails, and that is the finding rather than a bug in the gate: the fix is
//! reader support for the two markers real sources carry, and that is a reader change with
//! identity consequences (a reading's `rdid`, a part's structure hash), not an ingest change.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use smysl_core::ids::{LangTag, Tid};
use smysl_core::quote::{Normaliser, QUOTE_KEY};
use smysl_core::surface::hjson::{HObject, HValue, Spanned};
use smysl_core::surface::payload::{object_to_payload, payload_to_object};
use smysl_core::types::{Manifest, Status, UnitCore, UnitCoreBuilder};
use smysl_core::{ids::KernelType, Span};
use smysl_ingest::text::{attach, quoted_spans, windows, Opts, Pane};
use smysl_text::limits::{Budget, Caps};
use smysl_text::reading::{Level, Segment};
use smysl_text::segment::Segmenter;
use smysl_text::structure::Structure;
use smysl_text::{Locator, ManifestBuilder, Policy};

/// The threshold §6 TX-P5 step 3b names, as a percentage.
const THRESHOLD: f64 = 10.0;

/// One probe: a passage, a quote inside it, and the mode a human says that quote is in.
#[derive(Debug, Clone)]
struct Probe {
    file: String,
    quote: String,
    expected: String,
}

fn manifest(alias: &str) -> Manifest {
    ManifestBuilder::new(
        alias,
        LangTag::new("en").expect("a tag"),
        "txt/1",
        "public-domain",
        &Policy::default(),
    )
    .expect("a builder")
    .build()
    .expect("a manifest")
}

/// A passage as one part whose nodes are its lines.
fn rows_of(text: &str) -> Vec<Segment> {
    let level = Level::new("line").expect("a level");
    let mut rows = Vec::new();
    let mut at = 0u64;
    for (i, line) in text.lines().enumerate() {
        let len = line.len() as u64;
        rows.push(Segment::new(
            at,
            at + len,
            level.clone(),
            Locator::line(i as u64 + 1).expect("a locator"),
        ));
        at += len + 1;
    }
    rows
}

fn unit_with(quote: &str) -> UnitCore {
    let span = Span::new(0, 0);
    let mut obj = HObject::default();
    obj.insert(
        Spanned::new(QUOTE_KEY.to_string(), span),
        Spanned::new(HValue::Str(quote.to_string()), span),
    );
    let mut b = UnitCoreBuilder::new(KernelType::Claim, quote, Status::Speculative);
    b.payload = object_to_payload(&obj);
    b.build().expect("a unit")
}

/// The `text:mode` a unit carrying `quote` comes out of attachment with.
///
/// `None` for a quote the passage does not support, which is counted separately: a probe whose
/// quote cannot be found measures the probe set or the quote comparison, not the mode.
fn mode_of(text: &str, marks: &[Range<u64>], quote: &str, alias: &str) -> Option<Option<String>> {
    let rows = rows_of(text);
    let mut budget = Budget::new(Caps::default(), text.len() as u64).expect("a budget");
    let structure = Structure::build(&rows, text.len() as u64, &mut budget).expect("a tree");
    let tid = Tid::from_normalised_bytes(smysl_text::norm::Normalised::of(text).as_bytes());
    let panes = [Pane::new(tid, text, &structure, &rows).with_reported(marks)];
    let (ws, _) = windows(
        &panes,
        &Level::new("line").expect("a level"),
        u32::MAX / 2,
        smysl_core::TokenEstimator::Utf8Div4,
        &Segmenter::neutral(),
    );
    let m = manifest(alias);
    let o = Opts {
        manifest: &m,
        normaliser: Normaliser::V1,
        lang: None,
    };
    for w in &ws {
        let out = attach(vec![unit_with(quote)], w, &o);
        if out.spanned == 0 {
            continue;
        }
        let mode = out.units[0]
            .payload
            .as_deref()
            .and_then(|p| payload_to_object(p).ok())
            .and_then(|o| {
                o.get("text:mode")
                    .and_then(|v| v.value.as_str())
                    .map(str::to_string)
            });
        return Some(mode);
    }
    None
}

fn read_probes(dir: &Path) -> Vec<Probe> {
    let raw = std::fs::read_to_string(dir.join("probes.tsv")).expect("probes.tsv in the probe set");
    raw.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let mut f = l.split('\t');
            Probe {
                file: f.next().expect("a file").to_string(),
                quote: f.next().expect("a quote").to_string(),
                expected: f.next().expect("an expected mode").trim().to_string(),
            }
        })
        .collect()
}

/// A passage's marks: its own sidecar where it has one, the punctuation scan otherwise.
fn marks_for(dir: &Path, file: &str, text: &str) -> Vec<Range<u64>> {
    let sidecar = dir.join(format!("{file}.marks"));
    match std::fs::read_to_string(&sidecar) {
        Ok(raw) => raw
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let (a, b) = l.split_once('-').expect("a start-end range");
                a.trim().parse::<u64>().expect("a start")..b.trim().parse::<u64>().expect("an end")
            })
            .collect(),
        Err(_) => quoted_spans(text),
    }
}

/// The rate the clause names: of the probes a human marked `reported`, the share that did not
/// come out `reported`.
///
/// Computed over the probes whose quote was **found**, and the unfound ones are reported beside
/// it rather than folded in: a quote the passage does not support says nothing about mode, and
/// counting it as a miss would let a bad probe file fail a good detector.
fn rate(outcomes: &[(String, Option<String>)]) -> Option<f64> {
    let reported: Vec<&(String, Option<String>)> =
        outcomes.iter().filter(|(e, _)| e == "reported").collect();
    if reported.is_empty() {
        return None;
    }
    let missed = reported
        .iter()
        .filter(|(_, got)| got.as_deref() != Some("reported"))
        .count();
    Some(100.0 * missed as f64 / reported.len() as f64)
}

#[test]
fn the_asserted_as_reported_rate_is_below_ten_percent() {
    let Some(dir) = std::env::var_os("SMYSL_PROBE_SET") else {
        println!(
            "TX-P5 step 3b's probe-set clause is OWED: no SMYSL_PROBE_SET. The set is \
             attribution-bearing passages with known reported speech, and authoring one in this \
             repository would make it a development set. See this file's header for the format."
        );
        return;
    };
    let dir = PathBuf::from(dir);
    let probes = read_probes(&dir);
    assert!(!probes.is_empty(), "the probe set names no probes");

    let mut texts: BTreeMap<String, String> = BTreeMap::new();
    let mut outcomes: Vec<(String, Option<String>)> = Vec::new();
    let mut unfound = 0usize;
    for p in &probes {
        let text = texts.entry(p.file.clone()).or_insert_with(|| {
            std::fs::read_to_string(dir.join(&p.file)).expect("a passage the probe set names")
        });
        let marks = marks_for(&dir, &p.file, text);
        match mode_of(text, &marks, &p.quote, "probes") {
            Some(mode) => outcomes.push((p.expected.clone(), mode)),
            None => {
                unfound += 1;
                println!("probe not found in {}: {:?}", p.file, p.quote);
            }
        }
    }

    let measured = rate(&outcomes);
    println!(
        "probe set: {} probe(s), {} not found, {} attribution-bearing",
        probes.len(),
        unfound,
        outcomes.iter().filter(|(e, _)| e == "reported").count(),
    );
    match measured {
        Some(r) => {
            println!("asserted-as-reported rate: {r:.1}% (threshold {THRESHOLD:.0}%)");
            assert!(
                r < THRESHOLD,
                "the asserted-as-reported rate is {r:.1}%, at or above the {THRESHOLD:.0}% \
                 threshold TX-P5 step 3b exits on. Where the passages are unpunctuated — a 1769 \
                 Bible, say — this is the reader gap rather than the converter's: nothing in this \
                 build marks reported speech, so nothing can read it off the structure.",
            );
        }
        None => panic!("the probe set holds no probe marked `reported`; it cannot measure T8"),
    }
}

/// The arithmetic, over outcomes this test states rather than reads.
///
/// The harness above cannot run here, so the thing that *can* be checked is the measure it
/// computes: a rate over the probes a human called `reported`, blind to everything else.
#[test]
fn the_rate_counts_missed_reported_speech_and_nothing_else() {
    let outcomes = |v: &[(&str, Option<&str>)]| -> Vec<(String, Option<String>)> {
        v.iter()
            .map(|(e, g)| (e.to_string(), g.map(str::to_string)))
            .collect()
    };

    // Ten reported probes, one missed: 10%, which is *at* the threshold and therefore a failure.
    let mut v: Vec<(&str, Option<&str>)> = vec![("reported", Some("reported")); 9];
    v.push(("reported", Some("asserted")));
    assert_eq!(rate(&outcomes(&v)), Some(10.0));
    // The clause is "below 10%", not "at most": a rate equal to the threshold fails, which is
    // what `the_asserted_as_reported_rate_is_below_ten_percent` asserts with `r < THRESHOLD`.
    assert_eq!(rate(&outcomes(&v)), Some(THRESHOLD));

    // A mode nobody wrote is a miss too: §3.4 does not let `asserted` be a default, and a unit
    // with no mode at all is still a unit whose reported speech was not recorded.
    assert_eq!(
        rate(&outcomes(&[("reported", None)])),
        Some(100.0),
        "an absent mode is a missed one",
    );

    // Probes a human called asserted are not in the denominator: this measures T8's defect,
    // which is reported speech read as assertion and not the reverse.
    assert_eq!(
        rate(&outcomes(&[
            ("asserted", Some("reported")),
            ("reported", Some("reported")),
        ])),
        Some(0.0),
    );
    assert_eq!(rate(&outcomes(&[("asserted", Some("asserted"))])), None);
}

/// What a punctuated passage gives, as a worked example of the measurement with no probe set.
///
/// Not the gate: three sentences this file wrote are a development set by construction. It
/// exists so the harness above is known to work — a gate nobody has ever seen produce a number
/// is a gate that will produce the wrong one when it finally runs.
#[test]
fn the_harness_measures_a_punctuated_passage_at_zero_and_an_unpunctuated_one_at_a_hundred() {
    let punctuated = "He said, \u{201c}there is no God\u{201d}, and left.\n";
    let scan = quoted_spans(punctuated);
    let got = mode_of(punctuated, &scan, "there is no God", "probes").expect("the quote is there");
    assert_eq!(got.as_deref(), Some("reported"));

    // The 1769 spelling of the same claim, which no punctuation scan can see.
    let unpunctuated = "The fool hath said in his heart, There is no God.\n";
    let scan = quoted_spans(unpunctuated);
    assert!(scan.is_empty());
    let got =
        mode_of(unpunctuated, &scan, "There is no God", "probes").expect("the quote is there");
    assert_eq!(got, None, "no mark, no speaker, and therefore no mode");

    // And with the marks a reader *could* supply, the same verse comes out reported — which is
    // what names the gap as the reader's rather than the converter's.
    let at = unpunctuated.find("There is no God").expect("the clause");
    let marks: Vec<Range<u64>> =
        std::iter::once(at as u64..(at + "There is no God".len()) as u64).collect();
    let got = mode_of(unpunctuated, &marks, "There is no God", "probes").expect("the quote");
    assert_eq!(got.as_deref(), Some("reported"));
}
