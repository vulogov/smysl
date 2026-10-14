//! **GE-T4, attribution fairness across languages** (SMYSL-2.4 §5.5, §6 TX-P5; SMYSL-2.1 F-3).
//!
//! The row reads: *200 hand-checked units per tier-1 language under normaliser v1 and v2; v2
//! false-`Absent` above 1% in any language blocks TX-P5.* It is a phase **precondition**, which
//! is why it is owed rather than failed — the extraction steps can be built without it and
//! cannot be *passed* without it.
//!
//! # What a false `Absent` is, and why only one verdict needs a person
//!
//! The quote check answers `Present`, `Loose` or `Absent` for a quote against the text it claims
//! to come from, and `Absent` is the verdict with consequences: it is what `SMY-E307` refuses an
//! attribution on. A **false** `Absent` is a quote a reader would call verbatim that the check
//! rejects anyway — the failure mode F-3 exists for, measured per language because the whole
//! question is whether the instrument is as fair to Russian as it is to English.
//!
//! Only an `Absent` can be a false `Absent`. That is not a convenience, it is the whole shape of
//! the labour: of a 200-unit sample, a ruling can move the figure only on the units the check
//! **refused**, so those are the only ones a person has to read. The rest are the denominator.
//!
//! The first version of this harness collected everything that was not `Present` under both
//! forms, which read well and was wrong by a third: a `Loose` verdict cannot be a false
//! `Absent` either — `SMY-E307` refuses an attribution on `Absent` and not on `Loose` — so
//! sixteen of the first forty-five rows were units whose ruling could not have changed the
//! result by one unit. [`worklist`] is now the `Absent`s and nothing else, and
//! `a_loose_verdict_is_not_a_persons_problem` asserts the exclusion. A worklist that quietly
//! carried settled units is how an afternoon becomes the reason a row stays owed for a phase.
//!
//! # What a rule can decide, and what it must not
//!
//! Two of the shapes in that set are **decidable**, and [`decide`] decides them with evidence
//! rather than leaving them to a reader:
//!
//! - A quote with an **elision** in it (`...` or `…`) is stitched from two places. It is not in
//!   the source word for word by inspection of the quote alone.
//! - A quote the check calls `Present` against **a text the unit did not cite** is a real quote
//!   of the wrong place. `Absent` against the source it *did* cite is then the check working,
//!   and the error is the model's — it quoted past its window, from the edition rather than from
//!   the passage. The texts searched are every passage of the corpus plus whatever
//!   `SMYSL_GE_T4_CORPUS` points at, which is what the second variable is for: a corpus cut into
//!   chapters cannot witness a quote from the chapter *before* the one it holds, and the full
//!   edition can.
//!
//! Each carries the evidence into the worklist — the rule, and for the second the passage it was
//! found in — so any of them can be audited and **a person's ruling overrides it**
//! (`a_persons_ruling_overrides_a_rule`). What no rule here decides is whether a quote that is
//! merely *different* from the source is verbatim: paraphrase, mangling and punctuation are
//! judgements, and a rule that guessed at them would be the author of the instrument supplying
//! his own gold.
//!
//! # The instrument, offline
//!
//! Whether the arithmetic is right is a property of this code and is pinned here with no
//! material at all: that a verbatim quote scored `Absent` is counted, that an `Absent` on a
//! quote a reader rejected is **not**, that a language nobody has labelled reports no rate
//! rather than a perfect one, and that the sample is 200 per language, deduplicated and
//! reproducible. One of the fixtures is `всё` against `все`, which is `fixtures/quote/v2.tsv`
//! row 3: `Absent` under v1 and `Present` under v2. That pair is the measurement in miniature —
//! it is why the bar is on v2 and why v1 is reported beside it rather than dropped.
//!
//! # The measurement, which is owed
//!
//! What GE-T4 asks cannot be answered by units this repository authored, for the reason
//! GE-T13's chronology and step 3b's probe set are owed: material chosen by whoever wrote the
//! extractor is a development set. It needs extracted units, their source texts and a person's
//! ruling, and it runs when it is given them:
//!
//! ```text
//! SMYSL_GE_T4=<dir>          a tree of staged surface documents (`*.smy`) and the source
//!                            texts their units cite, in any layout: each unit names its own
//!                            source in `source.ref`, and the language comes from a
//!                            `manifest.tsv` (columns `file` and `lang`) where one is present
//! SMYSL_GE_T4_CORPUS=<dir>   further texts to witness a misplaced quote — the full editions
//!                            the passages were cut from. Searched, never scored against.
//! SMYSL_GE_T4_LABELS=<file>  one ruling per line: `<uid><TAB>verbatim|not-verbatim`
//! SMYSL_GE_T4_OUT=<file>     where to write the worklist (default `target/ge_t4/worklist.tsv`)
//! ```
//!
//! Given the corpus and no labels it scores every unit, writes the worklist and reports the
//! clause as owed — which is the step that makes the ruling cheap. Given both it computes the
//! rate per language and **asserts** the 1% bar on v2.
//!
//! The units need not have been extracted by this build: GE-T4's question is about the quote
//! *check*, not the extractor, so any corpus of units carrying `ingest:quote` beside the text
//! they cite measures it. That is stated here rather than glossed, because it is the difference
//! between this row and GE-T2's, which does need this build's own extraction.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

use smysl::{QuoteNormaliser, QuoteSupport};

/// §5.5's sample: 200 units per tier-1 language.
const SAMPLE: usize = 200;

/// §5.5's bar: above this, v2 blocks TX-P5.
const BAR: f64 = 0.01;

/// One unit's quote, under both comparison forms.
#[derive(Clone, Debug)]
struct Scored {
    uid: String,
    lang: String,
    /// The source file the unit cites, which is what a reader has to open to rule on it.
    source: String,
    quote: String,
    gist: String,
    v1: QuoteSupport,
    v2: QuoteSupport,
    v1_span: Option<Range<usize>>,
    v2_span: Option<Range<usize>>,
}

impl Scored {
    /// Whether a ruling on this one could move the figure: see the module's second section.
    ///
    /// `Absent` under either form, and not "anything but `Present`". The rate counts a quote a
    /// reader called verbatim that the check called `Absent`, so a `Loose` unit's ruling is
    /// labour that cannot change the result — and `Loose` is not what `SMY-E307` refuses on.
    fn disputed(&self) -> bool {
        self.v1 == QuoteSupport::Absent || self.v2 == QuoteSupport::Absent
    }
}

/// A person's ruling on whether a quote really is in the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Label {
    /// The reader found it in the source: an `Absent` here is a false one.
    Verbatim,
    /// The reader did not: an `Absent` here is the check working.
    NotVerbatim,
}

/// What one language measured.
#[derive(Clone, Debug, Default)]
struct Row {
    /// Units sampled: the denominator, labelled or not.
    n: usize,
    /// Of those, the ones a person ruled on.
    labelled: usize,
    /// Units the two forms disagree about. S0's figure over 5,560 quotes was zero.
    disagreements: usize,
    v1: Verdicts,
    v2: Verdicts,
    false_absent_v1: usize,
    false_absent_v2: usize,
}

#[derive(Clone, Debug, Default)]
struct Verdicts {
    present: usize,
    loose: usize,
    absent: usize,
    other: usize,
}

impl Verdicts {
    fn add(&mut self, v: QuoteSupport) {
        match v {
            QuoteSupport::Present => self.present += 1,
            QuoteSupport::Loose => self.loose += 1,
            QuoteSupport::Absent => self.absent += 1,
            // A-8.1 opened this enum. A verdict this harness does not know is counted and
            // reported, not panicked on and not silently folded into `absent`.
            _ => self.other += 1,
        }
    }
}

impl Row {
    /// The false-`Absent` rate, or `None` where nobody has ruled on anything that could be one.
    ///
    /// `None` rather than zero where a ruling is owed, the choice `Row::retry_rate` and
    /// `Agreement::rate` both make: 0.0 would read as the best possible result of a measurement
    /// nobody took, which is the one way this row could pass without the material it exists to
    /// require.
    ///
    /// But a language the check **never refused** has a rate of exactly zero and owes nobody a
    /// ruling: a false `Absent` needs an `Absent`, and there is none to rule on. Reporting that
    /// as owed would be the opposite error — material demanded for a figure already determined.
    /// Three of the five languages in the spike's sample are in that position.
    fn false_absent(&self, hits: usize, refusals: usize) -> Option<f64> {
        if refusals == 0 {
            return Some(0.0);
        }
        (self.labelled > 0).then(|| hits as f64 / self.n as f64)
    }

    fn v1_rate(&self) -> Option<f64> {
        self.false_absent(self.false_absent_v1, self.v1.absent)
    }

    fn v2_rate(&self) -> Option<f64> {
        self.false_absent(self.false_absent_v2, self.v2.absent)
    }
}

fn verdict(v: QuoteSupport) -> &'static str {
    match v {
        QuoteSupport::Present => "present",
        QuoteSupport::Loose => "loose",
        QuoteSupport::Absent => "absent",
        _ => "unknown",
    }
}

/// Score every quote-bearing unit of one staged document against the text it cites.
///
/// `only` keeps the units that cite *this* source file, which matters when one document's
/// units came from several passages: scoring a unit against a passage it never claimed would
/// manufacture an `Absent` out of nothing, and `Absent` is the verdict this row counts.
fn score(
    staged: &str,
    source: &str,
    lang: &str,
    only: Option<&str>,
) -> Result<Vec<Scored>, String> {
    let out = smysl::parse_surface(staged).map_err(|e| format!("{e}"))?;
    let mut scored = Vec::new();
    for r in &out.records {
        let Some(u) = r.as_unit() else { continue };
        if let Some(name) = only {
            let cites = u.source.as_ref().is_some_and(|s| {
                Path::new(&s.reference).file_name().and_then(|n| n.to_str()) == Some(name)
            });
            if !cites {
                continue;
            }
        }
        let payload = u
            .payload
            .as_deref()
            .map(smysl::payload_strings)
            .unwrap_or_default();
        // A unit with no quote is out of scope rather than an `Absent`: it attributes nothing,
        // so there is no attribution to be unfair about.
        let Some(quote) = payload
            .get(smysl::QUOTE_KEY)
            .and_then(|v| v.iter().next().cloned())
        else {
            continue;
        };
        let (v1, v1_span) = smysl::quote_support_span(&quote, source);
        let (v2, v2_span) = smysl::quote_support_span_with(QuoteNormaliser::V2, &quote, source);
        scored.push(Scored {
            uid: smysl::canonical_uid(u).canonical(),
            lang: lang.to_string(),
            source: u
                .source
                .as_ref()
                .and_then(|s| {
                    Path::new(&s.reference)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.to_string())
                })
                .unwrap_or_default(),
            quote,
            gist: u.gist.clone(),
            v1,
            v2,
            v1_span,
            v2_span,
        });
    }
    Ok(scored)
}

/// The sample: per language, deduplicated by uid, at most [`SAMPLE`], reproducible.
///
/// Sorted by canonical uid, which is a hash — so the order is arbitrary with respect to passage,
/// model and run, and identical on every machine. Taking the first 200 of a *file* order would
/// sample the beginnings of chapters, which is the one bias a quote check could plausibly have.
fn sample(all: Vec<Scored>) -> BTreeMap<String, Vec<Scored>> {
    let mut by_lang: BTreeMap<String, Vec<Scored>> = BTreeMap::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for s in all {
        // The same unit recurs across repeats of one run; it is one unit and one ruling.
        if seen.insert(s.uid.clone()) {
            by_lang.entry(s.lang.clone()).or_default().push(s);
        }
    }
    for units in by_lang.values_mut() {
        units.sort_by(|a, b| a.uid.cmp(&b.uid));
        units.truncate(SAMPLE);
    }
    by_lang
}

/// The units a person has to read: everything the check did not settle as `Present`.
fn worklist(units: &[Scored]) -> Vec<&Scored> {
    units.iter().filter(|s| s.disputed()).collect()
}

/// A ruling a rule can make, with the evidence for it.
///
/// `None` is the honest answer for everything else: see the module's third section.
fn decide(u: &Scored, corpus: &BTreeMap<String, String>) -> Option<(Label, String)> {
    if u.quote.contains("...") || u.quote.contains('\u{2026}') {
        return Some((Label::NotVerbatim, "elision".to_string()));
    }
    // The strongest form of the check, against every passage but the one it cited. V2 rather
    // than V1 so the rule cannot call a quote misplaced on a typographic difference alone.
    for (name, text) in corpus {
        if *name == u.source {
            continue;
        }
        if smysl::quote_support_with(QuoteNormaliser::V2, &u.quote, text) == QuoteSupport::Present {
            return Some((Label::NotVerbatim, format!("present in {name}")));
        }
    }
    None
}

/// Every ruling a rule can make over a sample, by uid.
fn decided(
    by_lang: &BTreeMap<String, Vec<Scored>>,
    corpus: &BTreeMap<String, String>,
) -> BTreeMap<String, (Label, String)> {
    let mut out = BTreeMap::new();
    for units in by_lang.values() {
        for u in units.iter().filter(|u| u.disputed()) {
            if let Some(d) = decide(u, corpus) {
                out.insert(u.uid.clone(), d);
            }
        }
    }
    out
}

/// The rulings in force: a person's where there is one, a rule's otherwise.
///
/// A person wins. A rule exists to spare a reader the units whose answer is already determined,
/// never to overrule one who looked.
fn rulings(
    human: &BTreeMap<String, Label>,
    rules: &BTreeMap<String, (Label, String)>,
) -> BTreeMap<String, Label> {
    let mut out: BTreeMap<String, Label> =
        rules.iter().map(|(k, (l, _))| (k.clone(), *l)).collect();
    for (uid, label) in human {
        out.insert(uid.clone(), *label);
    }
    out
}

fn tally(units: &[Scored], labels: &BTreeMap<String, Label>) -> Row {
    let mut row = Row {
        n: units.len(),
        ..Row::default()
    };
    for u in units {
        row.v1.add(u.v1);
        row.v2.add(u.v2);
        if u.v1 != u.v2 {
            row.disagreements += 1;
        }
        let Some(label) = labels.get(&u.uid) else {
            continue;
        };
        row.labelled += 1;
        if *label == Label::Verbatim {
            if u.v1 == QuoteSupport::Absent {
                row.false_absent_v1 += 1;
            }
            if u.v2 == QuoteSupport::Absent {
                row.false_absent_v2 += 1;
            }
        }
    }
    row
}

fn tsv(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "")
}

fn span(s: &Option<Range<usize>>) -> String {
    match s {
        Some(r) => format!("{}..{}", r.start, r.end),
        None => String::new(),
    }
}

/// The worklist as a file a person can rule on and hand back as `SMYSL_GE_T4_LABELS`.
///
/// Rows a rule decided arrive with the label filled in and the reason beside it, so the file is
/// a record of what was decided and how, not only a list of what is left. Editing one is how a
/// reader overrules it.
fn worklist_tsv(
    by_lang: &BTreeMap<String, Vec<Scored>>,
    rules: &BTreeMap<String, (Label, String)>,
) -> String {
    let mut out = String::from(
        "# GE-T4 adjudication worklist. Rule on each row by filling in `label`:\n\
         #   verbatim        the quote is in the source text THIS UNIT CITES, word for word\n\
         #   not-verbatim    it is not\n\
         # Only units the check called `absent` are here: the rate counts a verbatim quote\n\
         # scored `absent`, so a `present` or a `loose` ruling cannot move it. The rest of the\n\
         # sample is the denominator and needs no reading.\n\
         # A row with `evidence` was decided by rule, not by a person. Overrule it by changing\n\
         # `label`; a person's ruling always wins.\n\
         uid\tlabel\tevidence\tlang\tsource\tv1\tv2\tquote\tgist\n",
    );
    for (lang, units) in by_lang {
        for u in worklist(units) {
            let (label, evidence) = match rules.get(&u.uid) {
                Some((Label::Verbatim, e)) => ("verbatim", e.as_str()),
                Some((Label::NotVerbatim, e)) => ("not-verbatim", e.as_str()),
                None => ("", ""),
            };
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                u.uid,
                label,
                evidence,
                lang,
                u.source,
                verdict(u.v1),
                verdict(u.v2),
                tsv(&u.quote),
                tsv(&u.gist),
            ));
        }
    }
    out
}

fn read_labels(path: &Path) -> Result<BTreeMap<String, Label>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = BTreeMap::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("uid\t") {
            continue;
        }
        let mut cols = line.split('\t');
        let uid = cols.next().unwrap_or("").trim();
        let label = cols.next().unwrap_or("").trim();
        let label = match label {
            "verbatim" => Label::Verbatim,
            "not-verbatim" => Label::NotVerbatim,
            // An unruled row is the normal state of a worklist being worked through, and it is
            // not an error: it is a unit nobody has reached. Anything else is a typo, and a typo
            // read as "not verbatim" would quietly lower the rate this row is meant to raise.
            "" => continue,
            other => return Err(format!("line {}: `{other}` is not a ruling", n + 1)),
        };
        out.insert(uid.to_string(), label);
    }
    Ok(out)
}

/// Every file under `root` with one of these extensions, with `target` and `.git` skipped.
fn walk(root: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if p.is_dir() {
            if name != "target" && name != ".git" {
                walk(&p, ext, out);
            }
        } else if p.extension().is_some_and(|x| x == ext) {
            out.push(p);
        }
    }
}

/// `basename -> lang`, from every `manifest.tsv` under the root.
///
/// The S0 spike wrote one beside each corpus, with `file`, `lang` and a sha256 per passage. Read
/// rather than inferred, because a language assigned from a filename is a guess and this figure
/// is reported *per language* — assigning one row to the wrong language is the one error that
/// would not show up as noise.
fn languages(root: &Path) -> BTreeMap<String, String> {
    let mut manifests = Vec::new();
    walk(root, "tsv", &mut manifests);
    let mut out = BTreeMap::new();
    for m in manifests {
        if m.file_name().is_some_and(|n| n != "manifest.tsv") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&m) else {
            continue;
        };
        let mut lines = text.lines();
        let Some(header) = lines.next() else { continue };
        let cols: Vec<&str> = header.split('\t').collect();
        let (Some(fi), Some(li)) = (
            cols.iter().position(|c| *c == "file"),
            cols.iter().position(|c| *c == "lang"),
        ) else {
            continue;
        };
        for line in lines {
            let f: Vec<&str> = line.split('\t').collect();
            if let (Some(file), Some(lang)) = (f.get(fi), f.get(li)) {
                out.insert(file.trim().to_string(), lang.trim().to_string());
            }
        }
    }
    out
}

/// `basename -> path`, for resolving the source a unit names in `source.ref`.
fn sources(root: &Path) -> BTreeMap<String, PathBuf> {
    let mut texts = Vec::new();
    walk(root, "txt", &mut texts);
    let mut out = BTreeMap::new();
    for t in texts {
        if let Some(name) = t.file_name().and_then(|n| n.to_str()) {
            out.entry(name.to_string()).or_insert(t);
        }
    }
    out
}

/// The sources one staged document's units cite, as the `ref` is written.
fn cited(staged: &str) -> BTreeSet<String> {
    let Ok(out) = smysl::parse_surface(staged) else {
        return BTreeSet::new();
    };
    out.records
        .iter()
        .filter_map(|r| r.as_unit())
        .filter_map(|u| u.source.as_ref())
        .map(|s| s.reference.clone())
        .collect()
}

#[cfg(test)]
mod instrument {
    use super::*;

    /// A document of claims, each with a quote, in the surface form an ingest stages.
    fn doc(quotes: &[(&str, &str)]) -> String {
        let mut s = String::new();
        for (label, quote) in quotes {
            s.push_str(&format!(
                "@claim c/{label} {{ status: cited, source: {{ kind: file, ref: \"in/s.txt\" }}, \"ingest:quote\": \"{quote}\" }}\n~ {label}\n\n"
            ));
        }
        s
    }

    /// The four cases the rate has to tell apart, and the one that is not a defect.
    ///
    /// `всё` against a source reading `все` is `fixtures/quote/v2.tsv` row 3: `Absent` under v1,
    /// `Present` under v2. A reader calls it verbatim, so under v1 it is a false `Absent` and
    /// under v2 it is not — which is the measurement GE-T4 exists to take, in one row.
    #[test]
    fn a_verbatim_quote_scored_absent_is_the_only_thing_the_rate_counts() {
        let source = "все пошли домой. the pool wait rose to 1.9 seconds. nothing else.";
        let staged = doc(&[
            ("yo", "всё"),
            ("verbatim", "the pool wait rose to 1.9 seconds"),
            ("fabricated", "the shard was never replaced"),
            ("alsofab", "p99 returned to baseline"),
        ]);
        let units = score(&staged, source, "ru", None).expect("the document parses");
        assert_eq!(units.len(), 4, "{units:#?}");

        let by_uid: BTreeMap<&str, &Scored> = units.iter().map(|u| (u.gist.as_str(), u)).collect();
        let yo = by_uid["yo"];
        assert_eq!(
            (yo.v1, yo.v2),
            (QuoteSupport::Absent, QuoteSupport::Present),
            "the ё/е probe is the asymmetry this row measures"
        );

        // The person's ruling: the ё/е quote is in the text, the two inventions are not. The
        // fourth is left unruled, which is what a half-worked worklist looks like.
        let labels: BTreeMap<String, Label> = [
            (by_uid["yo"].uid.clone(), Label::Verbatim),
            (by_uid["verbatim"].uid.clone(), Label::Verbatim),
            (by_uid["fabricated"].uid.clone(), Label::NotVerbatim),
        ]
        .into_iter()
        .collect();

        let row = tally(&units, &labels);
        assert_eq!((row.n, row.labelled), (4, 3));
        // One false `Absent` under v1 — the ё/е quote — and none under v2.
        assert_eq!((row.false_absent_v1, row.false_absent_v2), (1, 0));
        assert_eq!(row.v1_rate(), Some(0.25), "one of the four sampled");
        assert_eq!(row.v2_rate(), Some(0.0));
        // And the invention the check rejected is not counted against it, which is the
        // difference between measuring the instrument and measuring the model.
        assert_eq!(
            row.v1.absent, 3,
            "three absent under v1, only one of them false"
        );
    }

    /// The denominator is the sample, not the worklist.
    ///
    /// Dividing by the disputed units instead would make the rate *rise* as the check improved —
    /// a perfect check with one mistake would report 100% — which is the arithmetic that turns a
    /// phase gate into a coin toss.
    #[test]
    fn the_rate_divides_by_the_sample_and_not_by_what_was_disputed() {
        let source = "все пошли домой.";
        let staged = doc(&[("yo", "всё"), ("a", "все"), ("b", "пошли"), ("c", "домой")]);
        let units = score(&staged, source, "ru", None).expect("the document parses");
        let yo = units.iter().find(|u| u.gist == "yo").expect("the probe");
        let labels = [(yo.uid.clone(), Label::Verbatim)].into_iter().collect();
        let row = tally(&units, &labels);
        assert_eq!(worklist(&units).len(), 1, "only the probe is disputed");
        assert_eq!(row.v1_rate(), Some(0.25), "1/4 sampled, not 1/1 disputed");
    }

    /// The labour claim, asserted: a settled unit never reaches a person.
    #[test]
    fn the_worklist_is_exactly_what_a_person_has_to_read() {
        let source = "the pool wait rose to 1.9 seconds. the rollback returned p99 to baseline.";
        let staged = doc(&[
            ("settled", "the pool wait rose to 1.9 seconds"),
            ("alsosettled", "the rollback returned p99 to baseline"),
            ("invented", "the shard was never replaced"),
        ]);
        let units = score(&staged, source, "en", None).expect("the document parses");
        let list = worklist(&units);
        assert_eq!(list.len(), 1, "{list:#?}");
        assert_eq!(list[0].gist, "invented");
        for u in &units {
            if !u.disputed() {
                assert_eq!(
                    (u.v1, u.v2),
                    (QuoteSupport::Present, QuoteSupport::Present),
                    "a unit kept off the worklist must be settled under both forms"
                );
            }
        }
        // And the file a person is handed carries the unruled column and the two spans.
        let by_lang = sample(units);
        let out = worklist_tsv(&by_lang, &BTreeMap::new());
        assert!(out.contains("not-verbatim"), "{out}");
        assert_eq!(
            out.lines().filter(|l| l.starts_with("u")).count()
                + out.lines().filter(|l| l.contains("\tinvented")).count(),
            out.lines().filter(|l| l.contains("\tinvented")).count() * 2,
            "one data row per disputed unit"
        );
    }

    /// A language nobody has ruled on has no rate — not a perfect one.
    ///
    /// And the two forms are owed independently, which the ё/е probe shows in one unit: v1
    /// refused it, so v1's rate waits for a reader; v2 did not refuse anything, so v2's rate is
    /// zero and waits for nobody. That asymmetry is the whole of what GE-T4 reports.
    #[test]
    fn an_unlabelled_language_reports_no_rate_rather_than_a_passing_one() {
        let source = "все пошли домой.";
        let staged = doc(&[("yo", "всё")]);
        let units = score(&staged, source, "ru", None).expect("the document parses");
        let row = tally(&units, &BTreeMap::new());
        assert_eq!(row.n, 1);
        assert_eq!(row.labelled, 0);
        assert_eq!(row.v1_rate(), None, "0.0 would be this gate passing itself");
        assert_eq!(row.v2.absent, 0, "v2 found it");
        assert_eq!(row.v2_rate(), Some(0.0), "nothing for v2 to be wrong about");
    }

    /// 200 per language, deduplicated, and the same 200 on every machine.
    #[test]
    fn the_sample_is_two_hundred_per_language_deduplicated_and_reproducible() {
        let source: String = (0..300)
            .map(|i| format!("line {i} of the source.\n"))
            .collect();
        let quotes: Vec<(String, String)> = (0..300)
            .map(|i| (format!("q{i}"), format!("line {i} of the source")))
            .collect();
        let pairs: Vec<(&str, &str)> = quotes
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let staged = doc(&pairs);

        let en = score(&staged, &source, "en", None).expect("parses");
        let ru = score(&staged, &source, "ru", None).expect("parses");
        assert_eq!(en.len(), 300);

        // The same units twice over, as five repeats of one run would give.
        let mut all = en.clone();
        all.extend(en.clone());
        let first = sample(all);
        assert_eq!(first["en"].len(), SAMPLE, "capped at the sample size");
        assert_eq!(
            first["en"]
                .iter()
                .map(|u| u.uid.clone())
                .collect::<BTreeSet<_>>()
                .len(),
            SAMPLE,
            "the repeats are one unit, not two"
        );

        let second = sample(en.clone());
        assert_eq!(
            first["en"].iter().map(|u| &u.uid).collect::<Vec<_>>(),
            second["en"].iter().map(|u| &u.uid).collect::<Vec<_>>(),
            "the sample is a function of the units and nothing else"
        );

        // Two languages are two samples. A unit of one is never a unit of the other, which is
        // the grouping the whole experiment is about — but the *uid* is the same, because a uid
        // is a hash of a unit and these are the same units read twice.
        let mut both = en;
        both.extend(ru);
        let by_lang = sample(both);
        assert_eq!(by_lang.len(), 1, "one language: the uids collide by design");
    }

    /// A `Loose` verdict is the check hedging, not refusing, and no ruling on it moves the rate.
    ///
    /// The first version of this harness put these on the worklist — sixteen of forty-five rows
    /// over the spike corpus — which is a third of a person's afternoon spent on units whose
    /// answer could not change the result by one. `SMY-E307` refuses on `Absent`.
    #[test]
    fn a_loose_verdict_is_not_a_persons_problem() {
        let source = "Il a dit « Liberté » hier. the pool wait rose to 1.9 seconds.";
        let staged = doc(&[
            ("hedged", "«Liberté»"),
            ("refused", "the shard was never replaced"),
            ("settled", "the pool wait rose to 1.9 seconds"),
        ]);
        let units = score(&staged, source, "fr", None).expect("the document parses");
        let hedged = units
            .iter()
            .find(|u| u.gist == "hedged")
            .expect("the probe");
        assert_eq!(
            (hedged.v1, hedged.v2),
            (QuoteSupport::Loose, QuoteSupport::Present),
            "the guillemet probe is `Loose` under v1: fixtures/quote/v2.tsv row 1"
        );
        let list = worklist(&units);
        assert_eq!(list.len(), 1, "{list:#?}");
        assert_eq!(
            list[0].gist, "refused",
            "only a refusal is a person's problem"
        );

        // And a ruling on it is not arithmetic: labelling the hedged unit verbatim leaves the
        // rate exactly where it was, which is why reading it was never worth an afternoon.
        let labels = [(hedged.uid.clone(), Label::Verbatim)]
            .into_iter()
            .collect();
        assert_eq!(tally(&units, &labels).false_absent_v1, 0);
    }

    /// An elided quote is stitched from two places, and the quote alone says so.
    #[test]
    fn an_elided_quote_is_decided_by_rule_and_not_by_a_person() {
        let source = "I ask one petition of thee, deny me not.";
        let staged = doc(&[(
            "elided",
            "I ask one petition... that he give me Abishag to wife",
        )]);
        let units = score(&staged, source, "en", None).expect("the document parses");
        assert_eq!(units[0].v2, QuoteSupport::Absent, "and so on the worklist");
        let (label, why) = decide(&units[0], &BTreeMap::new()).expect("a rule decides it");
        assert_eq!(label, Label::NotVerbatim);
        assert_eq!(why, "elision");
    }

    /// A real quote of the wrong passage is the model's error, and the corpus proves it.
    #[test]
    fn a_quote_from_a_passage_the_unit_did_not_cite_is_decided_by_rule() {
        let cited = "And Adam knew Eve his wife; and she conceived, and bare Cain.";
        let elsewhere = "But every man is tempted, when he is drawn away of his own lust.";
        let staged = doc(&[("outside", "every man is tempted, when he is drawn away")]);
        let units = score(&staged, cited, "en", None).expect("the document parses");
        assert_eq!(
            units[0].v2,
            QuoteSupport::Absent,
            "absent from what it cited"
        );

        let corpus: BTreeMap<String, String> = [
            ("s.txt".to_string(), cited.to_string()),
            ("jas.txt".to_string(), elsewhere.to_string()),
        ]
        .into_iter()
        .collect();
        let (label, why) = decide(&units[0], &corpus).expect("a rule decides it");
        assert_eq!(label, Label::NotVerbatim);
        assert_eq!(why, "present in jas.txt");

        // The control, and the reason the rule skips the cited source: a quote that *is* in the
        // passage it cited is not evidence of anything, and a rule that counted it would
        // decide every unit by finding it where it was supposed to be.
        let staged = doc(&[("inside", "she conceived, and bare Cain")]);
        let inside = score(&staged, cited, "en", None).expect("parses");
        assert_eq!(inside[0].v2, QuoteSupport::Present);
        assert!(!inside[0].disputed(), "a present verdict is not disputed");
    }

    /// A rule spares a reader; it never overrules one.
    #[test]
    fn a_persons_ruling_overrides_a_rule() {
        let source = "the pool wait rose to 1.9 seconds.";
        let staged = doc(&[("elided", "the pool wait... the shard was never replaced")]);
        let units = score(&staged, source, "en", None).expect("the document parses");
        let by_lang = sample(units.clone());
        let rules = decided(&by_lang, &BTreeMap::new());
        assert_eq!(rules.len(), 1, "the elision rule fires");

        // Nobody has looked: the rule stands, and the rate counts it as the rule said.
        let standing = rulings(&BTreeMap::new(), &rules);
        assert_eq!(standing[&units[0].uid], Label::NotVerbatim);
        assert_eq!(tally(&units, &standing).false_absent_v2, 0);

        // A reader looked and disagreed: a reader wins, and the figure moves.
        let human = [(units[0].uid.clone(), Label::Verbatim)]
            .into_iter()
            .collect();
        let ruled = rulings(&human, &rules);
        assert_eq!(ruled[&units[0].uid], Label::Verbatim);
        assert_eq!(tally(&units, &ruled).false_absent_v2, 1);

        // And the worklist says which rows a rule decided, so one can be found and audited.
        let out = worklist_tsv(&by_lang, &rules);
        assert!(out.contains("\tnot-verbatim\telision\t"), "{out}");
    }

    /// A language the check never refused is at zero, and owes nobody an afternoon.
    ///
    /// The mirror of the test above: `None` is right when a ruling is owed and wrong when the
    /// figure is already determined. A false `Absent` needs an `Absent`.
    #[test]
    fn a_language_with_no_refusals_is_at_zero_and_not_owed() {
        let source = "the pool wait rose to 1.9 seconds. the rollback returned p99 to baseline.";
        let staged = doc(&[
            ("a", "the pool wait rose to 1.9 seconds"),
            ("b", "the rollback returned p99 to baseline"),
        ]);
        let units = score(&staged, source, "en", None).expect("the document parses");
        let row = tally(&units, &BTreeMap::new());
        assert_eq!((row.v1.absent, row.v2.absent), (0, 0));
        assert_eq!(row.labelled, 0, "there was nothing to rule on");
        assert_eq!(row.v1_rate(), Some(0.0));
        assert_eq!(row.v2_rate(), Some(0.0));
        assert!(worklist(&units).is_empty(), "and no worklist either");
    }

    /// A ruling this harness cannot read is an error, not a silent `not-verbatim`.
    #[test]
    fn an_unreadable_ruling_is_refused_and_an_unruled_row_is_not() {
        let dir = std::env::temp_dir().join("smysl-ge-t4-labels");
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let good = dir.join("good.tsv");
        std::fs::write(
            &good,
            "# a worklist being worked through\nuid\tlabel\nu/a\tverbatim\nu/b\t\nu/c\tnot-verbatim\n",
        )
        .expect("written");
        let labels = read_labels(&good).expect("the file reads");
        assert_eq!(labels.len(), 2, "the unruled row is not a ruling");
        assert_eq!(labels["u/a"], Label::Verbatim);

        let bad = dir.join("bad.tsv");
        std::fs::write(&bad, "u/a\tprobably\n").expect("written");
        assert!(read_labels(&bad).is_err(), "a typo is not a verdict");
    }
}

/// The measurement, over material this repository does not hold.
#[test]
fn ge_t4_attribution_fairness_per_language() {
    let Ok(root) = std::env::var("SMYSL_GE_T4") else {
        println!(
            "GE-T4: owed. 200 hand-checked units per tier-1 language, under v1 and v2.\n\
             Set SMYSL_GE_T4=<dir> to a tree of staged `*.smy` documents and the source texts\n\
             their units cite; the harness scores them, writes the adjudication worklist and\n\
             reports what is left to rule on. SMYSL_GE_T4_LABELS=<file> then measures the rate."
        );
        return;
    };
    let root = PathBuf::from(root);
    let langs = languages(&root);
    let texts = sources(&root);
    let mut staged_files = Vec::new();
    walk(&root, "smy", &mut staged_files);
    staged_files.sort();
    assert!(
        !staged_files.is_empty(),
        "no `*.smy` under {}",
        root.display()
    );

    let mut all = Vec::new();
    let mut unresolved: BTreeSet<String> = BTreeSet::new();
    let mut unparsed = 0usize;
    for f in &staged_files {
        let Ok(staged) = std::fs::read_to_string(f) else {
            continue;
        };
        for reference in cited(&staged) {
            let Some(name) = Path::new(&reference)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.to_string())
            else {
                continue;
            };
            // Beside its own document first, and only then by name across the tree. A run that
            // kept its own copy of the passage is scored against *that* copy: resolving by
            // basename alone would silently score one run's units against another run's input,
            // and the two are the same bytes only until they are not.
            let local = f.parent().map(|d| d.join(&reference));
            let path = local
                .filter(|p| p.is_file())
                .or_else(|| texts.get(&name).cloned());
            let Some(path) = path else {
                unresolved.insert(name);
                continue;
            };
            let Ok(source) = std::fs::read_to_string(&path) else {
                unresolved.insert(name);
                continue;
            };
            // A document may cite more than one source; each unit is scored against the one it
            // names, which `score` cannot know, so the document is scored once per source and
            // the verdicts for the other sources are discarded by the span check below.
            let lang = langs.get(&name).cloned().unwrap_or_else(|| {
                name.split('-')
                    .next()
                    .filter(|t| t.len() <= 3 && t.chars().all(|c| c.is_ascii_lowercase()))
                    .unwrap_or("und")
                    .to_string()
            });
            match score(&staged, &source, &lang, Some(&name)) {
                Ok(units) => all.extend(units),
                Err(_) => unparsed += 1,
            }
        }
    }

    let by_lang = sample(all);
    assert!(
        !by_lang.is_empty(),
        "no quote-bearing units under {}",
        root.display()
    );

    let human = match std::env::var("SMYSL_GE_T4_LABELS") {
        Ok(p) => read_labels(Path::new(&p)).expect("the labels file reads"),
        Err(_) => BTreeMap::new(),
    };
    // Every passage of the corpus, for the rule that asks whether a quote belongs to a
    // different one. Loaded once; the passages are the material, so there are tens of them.
    let mut corpus: BTreeMap<String, String> = texts
        .iter()
        .filter_map(|(n, p)| std::fs::read_to_string(p).ok().map(|t| (n.clone(), t)))
        .collect();
    // The editions, where they were given. A passage file cannot witness a quote from the
    // chapter before it, which is the shape every refusal in the spike's sample turned out to
    // have: a model quoting past the window it was shown, from the book rather than the passage.
    if let Ok(more) = std::env::var("SMYSL_GE_T4_CORPUS") {
        for (name, path) in sources(Path::new(&more)) {
            if let Ok(text) = std::fs::read_to_string(&path) {
                corpus.insert(name, text);
            }
        }
    }
    let rules = decided(&by_lang, &corpus);
    let labels = rulings(&human, &rules);

    let out = PathBuf::from(
        std::env::var("SMYSL_GE_T4_OUT").unwrap_or_else(|_| "target/ge_t4/worklist.tsv".into()),
    );
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).expect("the worklist directory");
    }
    std::fs::write(&out, worklist_tsv(&by_lang, &rules)).expect("the worklist is written");

    println!(
        "GE-T4 over {} staged document(s){}{}",
        staged_files.len(),
        if unparsed > 0 {
            format!(", {unparsed} unparsed")
        } else {
            String::new()
        },
        if unresolved.is_empty() {
            String::new()
        } else {
            format!(", {} source(s) not found", unresolved.len())
        }
    );
    println!("lang  n    short  disagree  v1 P/L/A      v2 P/L/A      false-absent v1  v2");

    let mut blocked = Vec::new();
    let mut provisional = false;
    for (lang, units) in &by_lang {
        let row = tally(units, &labels);
        // A language with a refusal nobody has ruled on has a rate that can still move, and in
        // the one direction that matters: were all five of the spike's English residue ruled
        // verbatim, its rate would be 0.0250 and the phase would be blocked. Printing that as
        // a settled 0.0000 is how a provisional figure becomes a passed gate.
        let left = worklist(units)
            .iter()
            .filter(|s| !labels.contains_key(&s.uid))
            .count();
        provisional |= left > 0;
        let rate = |r: Option<f64>| match r {
            Some(v) if left > 0 => format!("{v:.4}*"),
            Some(v) => format!("{v:.4}"),
            None => "owed".to_string(),
        };
        println!(
            "{:<5} {:<4} {:<6} {:<9} {:<13} {:<13} {:<16} {}",
            lang,
            row.n,
            SAMPLE.saturating_sub(row.n),
            row.disagreements,
            format!("{}/{}/{}", row.v1.present, row.v1.loose, row.v1.absent),
            format!("{}/{}/{}", row.v2.present, row.v2.loose, row.v2.absent),
            rate(row.v1_rate()),
            rate(row.v2_rate()),
        );
        if let Some(v2) = row.v2_rate() {
            if v2 > BAR {
                blocked.push(format!("{lang}: {v2:.4}"));
            }
        }
    }
    if provisional {
        println!(
            "* provisional: the language has refusals nobody has ruled on, and the rate can only \
             rise"
        );
    }

    // Every unit the two forms disagree about, with both spans. S0 reported this set empty over
    // 5,560 quotes, so a row here is a case the pilot did not see — and the span is half of
    // what makes it one: two forms can agree on `Present` and still disagree on *where*, which
    // is the defect F-3 was raised for and the reason the span is printed rather than the
    // verdict alone.
    for (lang, units) in &by_lang {
        for u in units.iter().filter(|u| u.v1 != u.v2) {
            println!(
                "disagreement {lang}: v1 {} {} / v2 {} {} — {}",
                verdict(u.v1),
                span(&u.v1_span),
                verdict(u.v2),
                span(&u.v2_span),
                u.quote.chars().take(60).collect::<String>(),
            );
        }
    }

    let disputed: usize = by_lang.values().map(|u| worklist(u).len()).sum();
    let unruled: usize = by_lang
        .values()
        .map(|u| {
            worklist(u)
                .iter()
                .filter(|s| !labels.contains_key(&s.uid))
                .count()
        })
        .sum();
    println!("worklist: {}", out.display());
    println!(
        "{disputed} unit(s) the check refused: {} ruled by hand, {} decided by rule, {unruled} \
         left to read",
        human.len(),
        rules.keys().filter(|k| !human.contains_key(*k)).count(),
    );
    if labels.is_empty() {
        println!(
            "GE-T4: the corpus is scored and the clause is owed. Fill in `label` on the \
             worklist and pass it back as SMYSL_GE_T4_LABELS to measure the rate."
        );
        return;
    }
    if unruled > 0 {
        println!("the rate below is over what is ruled; {unruled} unit(s) are not");
    }
    assert!(
        blocked.is_empty(),
        "GE-T4: v2 false-`Absent` above {} blocks TX-P5 — {}",
        BAR,
        blocked.join(", ")
    );
}
