//! TX-P5 step 3b's exit clause: the asserted-as-reported rate over a probe set (§3.4, §6).
//!
//! # What the clause asks
//!
//! S0 measured reported speech extracted *as* asserted in 11.1-24.5% of attribution-bearing
//! units (T8), every cell above the 10% threshold, and the arm that added a holder paragraph to
//! the prompt did no better (T9 does not hold). So §1.1 item 4 moved `text:mode` out of the
//! prompt and into the converter, and TX-P5's exit gained a gate: **below 10%** over a probe set
//! of attribution-bearing passages with known reported speech.
//!
//! The probe set is `fixtures/attribution/probes/` — Mark 14-15 in two editions of scripture, one
//! that punctuates its quotations and one from 1769 that does not, with every speech act of Mark
//! 15 probed in both. Its `README.md` says where the files came from, what was chosen, and what
//! was deliberately not. `SMYSL_PROBE_SET=<dir>` replaces it with somebody else's material and
//! needs no code change, which is GE-T13's arrangement for a chronology.
//!
//! # The format
//!
//! `probes.tsv`, tab separated, `#` comments ignored, and the files it names beside it:
//!
//! ```text
//! file<TAB>quote<TAB>expected<TAB>marking<TAB>note
//! ```
//!
//! `expected` is `reported` or `none`; `marking` is `wj`, `quoted`, `both` or `bare`; `note`
//! carries the verse, the speaker and the frame the label was read off, and nothing reads it but
//! a person checking the set. A three-column line is accepted as well, with `marking` taken to be
//! unknown and therefore gated, which is what a probe set written against the first draft of this
//! harness means.
//!
//! A `.usfm` or `.osis` file is **read by its reader**, through a library, part by part — that is
//! the only way `\wj` and `<q who>` can reach the measurement at all, since
//! [`smysl_ingest::text::marks_of`] takes the marks off a reading's rows. Anything else is treated
//! as plain text whose nodes are its lines, and may bring its own marks in a `<file>.marks`
//! sidecar of `start-end` byte ranges, one per line.
//!
//! # What is gated, and what is counted and reported instead
//!
//! Only probes whose edition **marks or punctuates** the clause are in the gated denominator. A
//! clause its source leaves to a comma and a capital letter is in the `bare` arm, which is
//! reported at whatever rate it comes out at and gates nothing: nothing in a structural detector
//! can see it, §3.4's supplier list is the reader's marks and the punctuation scan, and folding
//! those probes in would make the figure a figure about typography. That is GE-T13's rule for a
//! date pair open on the side being compared — counted and reported rather than passed — and it
//! is the only way the two arms of this material can be read at once.
//!
//! The `none` probes measure the opposite error from T8's, and they are reported too: a gloss on a
//! name and a superscription are not sayings, one edition puts quotation marks round both, and a
//! mode written over either one is a false positive of the punctuation scan rather than of
//! anything the reader said.

#![cfg(feature = "text")]

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use smysl_core::ids::{KernelType, LangTag, Tid};
use smysl_core::quote::{Normaliser, QUOTE_KEY};
use smysl_core::surface::hjson::{HObject, HValue, Spanned};
use smysl_core::surface::payload::object_to_payload;
use smysl_core::types::{Carry, Manifest, Status, UnitCore, UnitCoreBuilder};
use smysl_core::Span;
use smysl_ingest::text::{attach, marks_of, quoted_spans, windows, Opts, Pane};
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::{Budget, Caps};
use smysl_text::readers::Input;
use smysl_text::reading::{Level, Segment};
use smysl_text::segment::Segmenter;
use smysl_text::structure::Structure;
use smysl_text::{Locator, ManifestBuilder, Policy};

/// The threshold §6 TX-P5 step 3b names, as a percentage.
const THRESHOLD: f64 = 10.0;

/// One probe: a passage, a quote inside it, the mode a human says that quote is in, and how the
/// source marks it.
#[derive(Debug, Clone)]
struct Probe {
    file: String,
    /// The verse the clause is in, and the verse the probe is measured at. `None` for a
    /// three-column probe set, which is measured wherever its quote attaches.
    locator: Option<String>,
    quote: String,
    expected: String,
    marking: String,
}

impl Probe {
    /// Whether this probe is in the gated denominator: its edition marked or punctuated the
    /// clause, so a supplier §3.4 names could have seen it.
    fn markable(&self) -> bool {
        self.marking != "bare"
    }
}

/// What one part of a passage holds, which is everything attachment needs.
struct Part {
    tid: Tid,
    text: String,
    rows: Vec<Segment>,
    structure: Structure,
    marks: Vec<Range<u64>>,
    level: Level,
}

/// A scratch directory that removes itself, for the libraries the readers need.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir = std::env::temp_dir()
            .join(format!("smysl-probe-set-{}-{tag}", std::process::id()))
            .join("lib");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
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

/// The reader a file's extension names, or `None` for a passage to treat as plain lines.
fn reader_of(file: &str) -> Option<&'static str> {
    match Path::new(file).extension().and_then(|e| e.to_str()) {
        Some("usfm") => Some("usfm/1"),
        Some("osis") => Some("osis/1"),
        _ => None,
    }
}

/// A passage read by its own reader, through a library: one [`Part`] per part, with the marks the
/// reader wrote into the reading.
///
/// This is the path that matters. A probe set drawn from scripture carries its reported speech in
/// `\wj` or `<q who>`, the readers put those on the rows as key 9, and `marks_of` is what takes
/// them off again — so a harness that read the file as lines would measure the punctuation scan
/// and call it the converter.
fn read_with_reader(dir: &Path, file: &str, reader: &str) -> (Vec<Part>, Manifest) {
    let raw = std::fs::read(dir.join(file)).expect("a passage the probe set names");
    let scratch = Scratch::new(&file.replace(['/', '.'], "-"));
    let mut lib = Library::create(&scratch.0).expect("a library");
    let added = lib
        .add(
            &Input::new(&raw),
            &AddSpec::new(reader, "probes", "public-domain")
                .with_lang(LangTag::new("en").expect("a tag"))
                .with_carry(Carry::Text),
            &Caps::DEFAULT,
        )
        .unwrap_or_else(|e| panic!("{file}: {e}"));
    let manifest = lib
        .store()
        .manifest(&added.mid)
        .expect("the manifest it wrote")
        .clone();

    let mut parts = Vec::new();
    for tid in &added.parts {
        let text = String::from_utf8(lib.part(tid).expect("the part's bytes").text).expect("UTF-8");
        let rows = lib
            .part_reading(&added.mid, tid)
            .expect("the part's reading")
            .rows;
        let structure = lib
            .structure(&added.mid, tid, &Caps::DEFAULT)
            .expect("a tree");
        let marks = marks_of(&rows, &text);
        // The level to window by is the deepest the reading has: a verse for scripture, and
        // whatever a reader of something else emits last.
        let level = rows
            .last()
            .map(|r| r.level.clone())
            .unwrap_or_else(|| Level::new("line").expect("a level"));
        parts.push(Part {
            tid: *tid,
            text,
            rows,
            structure,
            marks,
            level,
        });
    }
    (parts, manifest)
}

/// A plain passage as one part whose nodes are its lines, with its sidecar marks or the scan's.
fn read_as_lines(dir: &Path, file: &str) -> (Vec<Part>, Manifest) {
    let text = std::fs::read_to_string(dir.join(file)).expect("a passage the probe set names");
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
    let mut budget = Budget::new(Caps::default(), text.len() as u64).expect("a budget");
    let structure = Structure::build(&rows, text.len() as u64, &mut budget).expect("a tree");
    let tid = Tid::from_normalised_bytes(smysl_text::norm::Normalised::of(&text).as_bytes());
    let marks = sidecar(dir, file).unwrap_or_else(|| quoted_spans(&text));
    (
        vec![Part {
            tid,
            text,
            rows,
            structure,
            marks,
            level,
        }],
        manifest("probes"),
    )
}

/// A passage's own marks, where it brought them: one `start-end` byte range per line.
fn sidecar(dir: &Path, file: &str) -> Option<Vec<Range<u64>>> {
    let raw = std::fs::read_to_string(dir.join(format!("{file}.marks"))).ok()?;
    Some(
        raw.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let (a, b) = l.split_once('-').expect("a start-end range");
                a.trim().parse::<u64>().expect("a start")..b.trim().parse::<u64>().expect("an end")
            })
            .collect(),
    )
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

/// Where a probe landed, and what the structure said about it there.
#[derive(Debug)]
enum Outcome {
    /// Attached at the verse the probe names (or anywhere, for a probe that names none).
    At {
        mode: Option<String>,
        /// Which of the quote search's four scopes matched, as `Attachment::scope` prints it.
        /// A string because `Scope` is a public field of `Attachment` whose *type* this crate
        /// does not re-export, so a dependent cannot name it.
        scope: String,
    },
    /// Attached, but at another verse. Counted and reported: this measures the quote search.
    Elsewhere { locator: Option<String> },
    /// No part supports the quote at all.
    Nowhere,
}

/// The attribution a unit carrying `quote` comes out of attachment with, at the verse the probe
/// names.
///
/// **The locator is load-balanced against the quote, and it has to be.** The quote search has four
/// scopes and matches under a normaliser, so a short clause can attach to a longer one that
/// contains it — the first run of this probe set scored `Thou sayest it.` (15:2, marked) against
/// Peter's `I know not, neither understand I what thou sayest.` (14:68, unmarked), in the
/// previous chapter and therefore in the previous *part*, and read the mode off the wrong verse.
/// So a probe names its verse, only the part holding that verse is searched, and an attachment
/// that lands anywhere else is reported rather than scored.
fn mode_of(parts: &[Part], manifest: &Manifest, probe: &Probe) -> Outcome {
    let searched: Vec<&Part> = match &probe.locator {
        Some(want) => parts
            .iter()
            .filter(|p| p.rows.iter().any(|r| r.locator.to_string() == *want))
            .collect(),
        None => parts.iter().collect(),
    };
    for p in searched {
        let panes = [Pane::new(p.tid, &p.text, &p.structure, &p.rows).with_reported(&p.marks)];
        let (ws, _) = windows(
            &panes,
            &p.level,
            u32::MAX / 2,
            smysl_core::TokenEstimator::Utf8Div4,
            &Segmenter::neutral(),
        );
        let o = Opts {
            manifest,
            normaliser: Normaliser::V1,
            lang: None,
        };
        for w in &ws {
            let out = attach(vec![unit_with(&probe.quote)], w, &o);
            let Some(a) = out.attachments.first() else {
                continue;
            };
            if a.span.is_none() {
                continue;
            }
            if let Some(want) = &probe.locator {
                if a.locator.as_deref() != Some(want.as_str()) {
                    return Outcome::Elsewhere {
                        locator: a.locator.clone(),
                    };
                }
            }
            return Outcome::At {
                mode: a.attribution.mode.map(|m| m.to_string()),
                scope: format!("{:?}", a.scope),
            };
        }
    }
    Outcome::Nowhere
}

fn read_probes(dir: &Path) -> Vec<Probe> {
    let raw = std::fs::read_to_string(dir.join("probes.tsv")).expect("probes.tsv in the probe set");
    raw.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            // Two formats, told apart by width. The three-column form is this harness's first
            // draft: file, quote, expected, with no locator to measure at and no marking, which
            // is read as `unknown` and therefore gated — the safe reading, since an unstated
            // marking must not quietly leave a probe out of the figure.
            match f.len() {
                0..=3 => Probe {
                    file: f[0].to_string(),
                    locator: None,
                    quote: f[1].to_string(),
                    expected: f[2].trim().to_string(),
                    marking: "unknown".to_string(),
                },
                _ => Probe {
                    file: f[0].to_string(),
                    locator: Some(f[1].to_string()),
                    quote: f[2].to_string(),
                    expected: f[3].trim().to_string(),
                    marking: f[4].trim().to_string(),
                },
            }
        })
        .collect()
}

/// The rate the clause names: of the probes a human marked `reported`, the share that did not come
/// out `reported`.
///
/// Computed over the probes whose quote was **found**, and the unfound ones are reported beside it
/// rather than folded in: a quote the passage does not support says nothing about mode, and
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

/// The mode a plain passage's quote comes out with, for the worked example below.
///
/// A three-line passage has one node and no locator worth naming, so this is the probe the worked
/// example would have written out: no locator, measured wherever the quote attaches.
fn mode_at(parts: &[Part], manifest: &Manifest, file: &str, quote: &str) -> Option<&'static str> {
    let probe = Probe {
        file: file.to_string(),
        locator: None,
        quote: quote.to_string(),
        expected: "reported".to_string(),
        marking: "unknown".to_string(),
    };
    match mode_of(parts, manifest, &probe) {
        Outcome::At { mode, .. } => match mode.as_deref() {
            Some("reported") => Some(REPORTED),
            Some(other) => panic!("an unexpected mode: {other}"),
            None => None,
        },
        other => panic!("the quote did not attach: {other:?}"),
    }
}

/// The one mode this file compares against, spelled once.
const REPORTED: &str = "reported";

/// The directory the probe set is read from: the committed one, or whatever replaced it.
fn probe_dir() -> PathBuf {
    match std::env::var_os("SMYSL_PROBE_SET") {
        Some(dir) => PathBuf::from(dir),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/attribution/probes"),
    }
}

#[test]
fn the_asserted_as_reported_rate_is_below_ten_percent() {
    let dir = probe_dir();
    let probes = read_probes(&dir);
    assert!(!probes.is_empty(), "the probe set names no probes");

    // One read per file, not one per probe: reading two chapters of Mark into a library costs
    // more than every probe against it put together.
    let mut read: BTreeMap<String, (Vec<Part>, Manifest)> = BTreeMap::new();
    let mut gated: Vec<(String, Option<String>)> = Vec::new();
    let mut bare: Vec<(String, Option<String>)> = Vec::new();
    let mut false_positives: Vec<&Probe> = Vec::new();
    let mut misplaced: Vec<(&Probe, Option<String>)> = Vec::new();
    let mut unfound: Vec<&Probe> = Vec::new();

    for p in &probes {
        let parts = read
            .entry(p.file.clone())
            .or_insert_with(|| match reader_of(&p.file) {
                Some(reader) => read_with_reader(&dir, &p.file, reader),
                None => read_as_lines(&dir, &p.file),
            });
        let (mode, scope) = match mode_of(&parts.0, &parts.1, p) {
            Outcome::At { mode, scope } => (mode, scope),
            Outcome::Elsewhere { locator } => {
                misplaced.push((p, locator));
                continue;
            }
            Outcome::Nowhere => {
                unfound.push(p);
                continue;
            }
        };
        if p.expected == "none" {
            if mode.is_some() {
                false_positives.push(p);
            }
            continue;
        }
        if mode.as_deref() != Some(REPORTED) {
            println!(
                "  missed [{}] {} {} {:?} -> {:?}, matched at {scope}",
                p.marking,
                p.file,
                p.locator.as_deref().unwrap_or("-"),
                p.quote,
                mode
            );
        }
        let outcome = (p.expected.clone(), mode);
        if p.markable() {
            gated.push(outcome);
        } else {
            bare.push(outcome);
        }
    }

    println!(
        "probe set {}: {} probe(s) - {} gated, {} unmarked, {} not expected to be speech, \
         {} landed at another verse, {} not found",
        dir.display(),
        probes.len(),
        gated.len(),
        bare.len(),
        probes.iter().filter(|p| p.expected == "none").count(),
        misplaced.len(),
        unfound.len(),
    );
    for p in &unfound {
        println!("  not found in {}: {:?}", p.file, p.quote);
    }
    for (p, at) in &misplaced {
        println!(
            "  {:?} names {} and attached at {:?}",
            p.quote,
            p.locator.as_deref().unwrap_or("-"),
            at
        );
    }

    // The two arms the material exists to separate, and the error in the other direction.
    match rate(&bare) {
        Some(r) => println!(
            "unmarked, unpunctuated clauses: {r:.1}% not found as reported, over {} probe(s) - \
             counted and reported, gating nothing",
            bare.len()
        ),
        None => println!("unmarked, unpunctuated clauses: none in this probe set"),
    }
    for p in &false_positives {
        println!(
            "  a mode was written where the verse frames no saying: {} {} {:?} ({})",
            p.file,
            p.locator.as_deref().unwrap_or("-"),
            p.quote,
            p.marking
        );
    }
    println!(
        "modes written over clauses that are not sayings: {} of {}",
        false_positives.len(),
        probes.iter().filter(|p| p.expected == "none").count(),
    );

    let Some(measured) = rate(&gated) else {
        panic!(
            "the probe set holds no probe marked `reported` whose edition marks or punctuates \
             it; it cannot measure T8. {} unmarked probe(s) were counted and reported.",
            bare.len()
        )
    };
    println!("asserted-as-reported rate: {measured:.1}% (threshold {THRESHOLD:.0}%)");
    assert!(
        measured < THRESHOLD,
        "the asserted-as-reported rate is {measured:.1}%, at or above the {THRESHOLD:.0}% \
         threshold TX-P5 step 3b exits on, over {} probe(s) whose source marks or punctuates the \
         clause. This is the converter: every one of these was visible to a supplier §3.4 names.",
        gated.len(),
    );
}

/// The arithmetic, over outcomes this test states rather than reads.
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

/// A probe's arm is decided by what its source did, and an unknown marking is gated.
///
/// The second half is the one worth a test: a probe set written against the three-column format
/// has no `marking` column, and the safe reading of a missing one is to *gate* the probe. Treating
/// it as unmarked would let a set opt out of the clause by saying nothing.
#[test]
fn an_unmarked_probe_is_not_gated_and_an_unstated_marking_is() {
    let probe = |marking: &str| Probe {
        file: "x.usfm".to_string(),
        locator: Some("Mark.15.2".to_string()),
        quote: "q".to_string(),
        expected: "reported".to_string(),
        marking: marking.to_string(),
    };
    assert!(probe("wj").markable());
    assert!(probe("quoted").markable());
    assert!(probe("both").markable());
    assert!(!probe("bare").markable());
    assert!(probe("unknown").markable(), "a silent set does not opt out");
}

/// What a punctuated passage gives, as a worked example of the measurement in three lines.
///
/// Not the gate: three sentences this file wrote are a development set by construction. It exists
/// so the harness above is known to work on each of the three cases separately — a gate nobody has
/// ever seen produce a number is a gate that will produce the wrong one when it finally runs.
#[test]
fn the_harness_measures_a_punctuated_passage_at_zero_and_an_unpunctuated_one_at_a_hundred() {
    let dir = std::env::temp_dir().join(format!("smysl-probe-worked-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");

    let punctuated = "He said, \u{201c}there is no God\u{201d}, and left.\n";
    std::fs::write(dir.join("a.txt"), punctuated).expect("a passage");
    let (parts, m) = read_as_lines(&dir, "a.txt");
    assert_eq!(
        mode_at(&parts, &m, "a.txt", "there is no God"),
        Some(REPORTED)
    );

    // The 1769 spelling of the same claim, which no punctuation scan can see.
    let unpunctuated = "The fool hath said in his heart, There is no God.\n";
    std::fs::write(dir.join("b.txt"), unpunctuated).expect("a passage");
    let (parts, m) = read_as_lines(&dir, "b.txt");
    assert!(parts[0].marks.is_empty());
    assert_eq!(
        mode_at(&parts, &m, "b.txt", "There is no God"),
        None,
        "no mark, no speaker, and therefore no mode"
    );

    // And with the marks a reader *could* supply, in a sidecar, the same verse comes out reported.
    let at = unpunctuated.find("There is no God").expect("the clause");
    std::fs::write(
        dir.join("b.txt.marks"),
        format!("{at}-{}\n", at + "There is no God".len()),
    )
    .expect("a sidecar");
    let (parts, m) = read_as_lines(&dir, "b.txt");
    assert_eq!(parts[0].marks.len(), 1, "the sidecar supplied the mark");
    assert_eq!(
        mode_at(&parts, &m, "b.txt", "There is no God"),
        Some(REPORTED)
    );

    let _ = std::fs::remove_dir_all(&dir);
}
