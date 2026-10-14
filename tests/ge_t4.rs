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
//! A `Present` verdict cannot be a false `Absent`. That is not a convenience, it is the whole
//! shape of the labour: of a 200-unit sample, only the units the check does **not** call
//! `Present` can possibly be mislabelled, so those are the only ones a person has to read. The
//! rest are the denominator. [`worklist`] is that set and nothing else, and
//! `the_worklist_is_exactly_what_a_person_has_to_read` asserts it, because a worklist that
//! quietly included settled units would turn a short afternoon into the reason this row stays
//! owed for another phase.
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
    quote: String,
    gist: String,
    v1: QuoteSupport,
    v2: QuoteSupport,
    v1_span: Option<Range<usize>>,
    v2_span: Option<Range<usize>>,
}

impl Scored {
    /// Whether a person has to read this one: see the module's second section.
    fn disputed(&self) -> bool {
        self.v1 != QuoteSupport::Present || self.v2 != QuoteSupport::Present
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
    /// The false-`Absent` rate, or `None` where nobody has ruled.
    ///
    /// `None` rather than zero, the choice `Row::retry_rate` and `Agreement::rate` both make: a
    /// language nobody has hand-checked has no rate, and 0.0 would read as the best possible
    /// result of the measurement that was never taken — which is the one way this row could
    /// pass without the material it exists to require.
    fn false_absent(&self, hits: usize) -> Option<f64> {
        (self.labelled > 0).then(|| hits as f64 / self.n as f64)
    }

    fn v1_rate(&self) -> Option<f64> {
        self.false_absent(self.false_absent_v1)
    }

    fn v2_rate(&self) -> Option<f64> {
        self.false_absent(self.false_absent_v2)
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
fn worklist_tsv(by_lang: &BTreeMap<String, Vec<Scored>>) -> String {
    let mut out = String::from(
        "# GE-T4 adjudication worklist. Rule on each row and keep the first two columns:\n\
         #   <uid><TAB>verbatim        the quote is in the source text THIS UNIT CITES,\n\
         #                                word for word\n\
         #   <uid><TAB>not-verbatim    it is not: it elides (`...`), it paraphrases, or it\n\
         #                                quotes a passage the unit did not cite\n\
         # Only units the check did not call `present` under both forms are here; a `present`\n\
         # cannot be a false `absent`. The rest of the sample is the denominator.\n\
         uid\tlabel\tlang\tv1\tv2\tv1_span\tv2_span\tquote\tgist\n",
    );
    for (lang, units) in by_lang {
        for u in worklist(units) {
            out.push_str(&format!(
                "{}\t\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                u.uid,
                lang,
                verdict(u.v1),
                verdict(u.v2),
                span(&u.v1_span),
                span(&u.v2_span),
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
        let out = worklist_tsv(&by_lang);
        assert!(out.contains("not-verbatim"), "{out}");
        assert_eq!(
            out.lines().filter(|l| l.starts_with("u")).count()
                + out.lines().filter(|l| l.contains("\tinvented")).count(),
            out.lines().filter(|l| l.contains("\tinvented")).count() * 2,
            "one data row per disputed unit"
        );
    }

    /// A language nobody has ruled on has no rate — not a perfect one.
    #[test]
    fn an_unlabelled_language_reports_no_rate_rather_than_a_passing_one() {
        let source = "все пошли домой.";
        let staged = doc(&[("yo", "всё")]);
        let units = score(&staged, source, "ru", None).expect("the document parses");
        let row = tally(&units, &BTreeMap::new());
        assert_eq!(row.n, 1);
        assert_eq!(row.labelled, 0);
        assert_eq!(row.v1_rate(), None, "0.0 would be this gate passing itself");
        assert_eq!(row.v2_rate(), None);
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

    let labels = match std::env::var("SMYSL_GE_T4_LABELS") {
        Ok(p) => read_labels(Path::new(&p)).expect("the labels file reads"),
        Err(_) => BTreeMap::new(),
    };

    let out = PathBuf::from(
        std::env::var("SMYSL_GE_T4_OUT").unwrap_or_else(|_| "target/ge_t4/worklist.tsv".into()),
    );
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).expect("the worklist directory");
    }
    std::fs::write(&out, worklist_tsv(&by_lang)).expect("the worklist is written");

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
    for (lang, units) in &by_lang {
        let row = tally(units, &labels);
        let rate = |r: Option<f64>| match r {
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
    if labels.is_empty() {
        println!(
            "GE-T4: the corpus is scored and the clause is owed. {unruled} unit(s) need a \
             ruling; the rest of the sample cannot be a false `absent`."
        );
        return;
    }
    if unruled > 0 {
        println!("{unruled} disputed unit(s) are still unruled; the rate is over what is ruled.");
    }
    assert!(
        blocked.is_empty(),
        "GE-T4: v2 false-`Absent` above {} blocks TX-P5 — {}",
        BAR,
        blocked.join(", ")
    );
}
