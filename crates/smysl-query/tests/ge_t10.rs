//! **GE-T10** — does a dossier's slot hold what a reader asking the question would want there?
//!
//! SMYSL-2.5 §5.5: *30 questions × 5 languages, expert-filled slots as uid sets in
//! `fixtures/dossier/gold/`; reports slot precision and recall per intent and slot. Kill
//! criterion from draft 3 §23: precision < 0.8 for* definition *or* counter-evidence *keeps
//! `ask` out of defaults.* TX-P8 step 4 is the run, and its exit is the numbers recorded and the
//! criterion applied.
//!
//! # What is measured, three ways
//!
//! A dossier can be wrong in three places, and one pooled figure would not say which:
//!
//! 1. **End to end**, per language: the question as a person typed it, through
//!    [`smysl_query::dossier::classify`] with no `--lang`, subject resolution, and the slots.
//!    A question that is not classified, or whose subject is not found, contributes an empty
//!    slot for every gold slot: it costs recall and no precision, which is what it costs the
//!    person.
//! 2. **The front**, per language: whether the classifier found the question's intent and
//!    whether a subject was found under it.
//! 3. **The slots alone**: each question once, in English, with its gold intent given and its
//!    subject picked, so that nothing but the templates is measured. The kill criterion is
//!    applied here, because it is a criterion on slots; a classifier that misses a language is
//!    a different defect with a different fix, and the end-to-end table is where it shows.
//!
//! Precision and recall are micro-averaged over (question, slot) pairs. A slot both the gold and
//! the dossier leave empty is a correct `gap`, counted separately; it adds nothing to either
//! figure, since neither has a denominator there.
//!
//! # Who wrote the gold, which is the figure's first limit
//!
//! GE-T10 asks for *expert-filled* slots. `fixtures/dossier/gold/` was written by one hand —
//! the hand that wrote the templates, after writing them — and nobody else has read it; the
//! project has no independent coders. The gold is written to criteria stated at the top of
//! `slots.tsv`, by what a unit *says*, and not to the templates, which select by kind and
//! edge; the corpus plants the cases where those come apart (`PLANT` in `corpus.smy`). So the
//! figures below say how far the templates are from one careful reading, and the planted
//! cases say why. They do not say how far they are from an expert's, and the run that would is
//! one environment variable away:
//!
//! ```text
//! SMYSL_GE_T10_CORPUS=<file.smy>        a corpus somebody else assembled
//! SMYSL_GE_T10_QUESTIONS=<file.tsv>     their questions, in questions.tsv's columns
//! SMYSL_GE_T10_GOLD=<file.tsv>          their gold, in slots.tsv's columns
//! SMYSL_GE_T10_SUBJECTS=<file.tsv>      qid-prefix → subject label, if their qids differ
//! SMYSL_GE_T10_OUT=<file>               where to write the report
//! ```
//!
//! # What is asserted
//!
//! The figures recorded in SMYSL-2.5 §6 TX-P8 step 4, to three places, over the committed
//! fixture: a change to the templates, the classifier or the resolver that moves one moves a
//! recorded number, and the test says which. Under an override nothing is asserted but that the
//! files were read; the report is the result.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use smysl_core::ids::{AgentId, Uid};
use smysl_core::surface::parse_surface;
use smysl_graph::Store;
use smysl_query::dossier::{templates, Intent};
use smysl_query::{ask, AskError, AskOptions, Env, StoreScope};

/// Draft 3 §23's bar for *definition* and *counter*.
const BAR: f64 = 0.8;

const LANGS: [&str; 5] = ["en", "de", "es", "fr", "ru"];

/// The committed questions' subjects, by qid prefix: the label of the English entity.
const SUBJECTS: &[(&str, &str)] = &[
    ("ch", "e/cholera"),
    ("sc", "e/scurvy"),
    ("mi", "e/miasma"),
    ("sp", "e/smallpox"),
    ("se", "e/semmelweis"),
    ("sn", "e/snow"),
    ("je", "e/jenner"),
    ("li", "e/lind"),
    ("ls", "e/lister"),
    ("ph", "e/phlogiston"),
];

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn input(var: &str, default: &str) -> (PathBuf, bool) {
    match std::env::var_os(var) {
        Some(p) => (PathBuf::from(p), true),
        None => (repo(default), false),
    }
}

fn rows(path: &Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| l.split('\t').map(str::to_string).collect())
        .collect()
}

struct Question {
    qid: String,
    intent: Intent,
    lang: String,
    text: String,
}

/// One (question, slot) comparison.
#[derive(Default, Clone, Copy)]
struct Tally {
    found: usize,
    gold: usize,
    hit: usize,
    gaps: usize,
    pairs: usize,
}

impl Tally {
    fn add(&mut self, found: &BTreeSet<String>, gold: &BTreeSet<String>) {
        self.pairs += 1;
        self.found += found.len();
        self.gold += gold.len();
        self.hit += found.intersection(gold).count();
        if found.is_empty() && gold.is_empty() {
            self.gaps += 1;
        }
    }

    fn merge(&mut self, o: Tally) {
        self.found += o.found;
        self.gold += o.gold;
        self.hit += o.hit;
        self.gaps += o.gaps;
        self.pairs += o.pairs;
    }

    fn precision(&self) -> Option<f64> {
        (self.found > 0).then(|| self.hit as f64 / self.found as f64)
    }

    fn recall(&self) -> Option<f64> {
        (self.gold > 0).then(|| self.hit as f64 / self.gold as f64)
    }
}

fn fmt(x: Option<f64>) -> String {
    x.map_or("  —  ".to_string(), |v| format!("{v:.3}"))
}

fn row(out: &mut String, name: &str, t: &Tally) {
    let _ = writeln!(
        out,
        "  {name:<16} {:>6} {:>6}   found {:>3}  gold {:>3}  hit {:>3}  pairs {:>3}  gaps {:>2}",
        fmt(t.precision()),
        fmt(t.recall()),
        t.found,
        t.gold,
        t.hit,
        t.pairs,
        t.gaps
    );
}

/// What a dossier put in each slot, by label (a unit the corpus does not label is kept by uid,
/// so a stray is counted against precision rather than lost).
fn slots_of(
    d: &smysl_query::Dossier,
    names: &BTreeMap<Uid, String>,
) -> BTreeMap<String, BTreeSet<String>> {
    d.slots
        .iter()
        .map(|s| {
            let units = s
                .units
                .iter()
                .map(|u| names.get(u).cloned().unwrap_or_else(|| u.canonical()))
                .collect();
            (s.name.to_string(), units)
        })
        .collect()
}

/// The run's result: the report and the figures the test pins.
struct Run {
    report: String,
    slots_alone: BTreeMap<String, Tally>,
    end_to_end: BTreeMap<String, Tally>,
    front: BTreeMap<String, (usize, usize, usize)>,
}

fn run() -> Run {
    run_over(
        input(
            "SMYSL_GE_T10_QUESTIONS",
            "fixtures/dossier/gold/questions.tsv",
        ),
        input("SMYSL_GE_T10_GOLD", "fixtures/dossier/gold/slots.tsv"),
    )
}

/// The fresh set (OQ-87, OQ-88): new questions over the committed corpus, written before set 2
/// of the templates ran on anything. No override; the committed corpus is what it asks about.
fn run_fresh() -> Run {
    run_over(
        (repo("fixtures/dossier/fresh/questions.tsv"), false),
        (repo("fixtures/dossier/fresh/slots.tsv"), false),
    )
}

fn run_over((questions, q_over): (PathBuf, bool), (gold_path, g_over): (PathBuf, bool)) -> Run {
    let (corpus, c_over) = input("SMYSL_GE_T10_CORPUS", "fixtures/dossier/gold/corpus.smy");

    let src = std::fs::read_to_string(&corpus).expect("the corpus");
    let parsed = parse_surface(&src).expect("the corpus parses");
    let labels: BTreeMap<String, Uid> = parsed
        .labels
        .iter()
        .map(|(l, u)| (l.to_string(), *u))
        .collect();
    let names: BTreeMap<Uid, String> = labels.iter().map(|(l, u)| (*u, l.clone())).collect();
    let store = Store::from_records(parsed.records);
    let scope = StoreScope::new("ge-t10", &store);
    let env = Env::new(&store, &scope).with_entities(&scope);

    let subjects: BTreeMap<String, String> = match std::env::var_os("SMYSL_GE_T10_SUBJECTS") {
        Some(p) => rows(Path::new(&p))
            .into_iter()
            .map(|r| (r[0].clone(), r[1].clone()))
            .collect(),
        None => SUBJECTS
            .iter()
            .map(|(p, l)| (p.to_string(), l.to_string()))
            .collect(),
    };

    let qs: Vec<Question> = rows(&questions)
        .into_iter()
        .map(|r| Question {
            qid: r[0].clone(),
            intent: Intent::parse(&r[1]).unwrap_or_else(|| panic!("intent `{}`", r[1])),
            lang: r[2].clone(),
            text: r[3].clone(),
        })
        .collect();

    // qid → slot → labels, in the intent's slot order.
    let mut gold: BTreeMap<String, Vec<(String, BTreeSet<String>)>> = BTreeMap::new();
    for r in rows(&gold_path) {
        let units: BTreeSet<String> = if r[2] == "-" {
            BTreeSet::new()
        } else {
            r[2].split(' ').map(str::to_string).collect()
        };
        for u in &units {
            assert!(
                labels.contains_key(u),
                "gold names `{u}`, which the corpus does not label"
            );
        }
        gold.entry(r[0].clone())
            .or_default()
            .push((r[1].clone(), units));
    }
    // The gold may name a slot the set no longer offers — `revision`, which set 2 withdrew
    // from `trace` (OQ-87) — and it is scored as a slot that found nothing: the reader still
    // wanted those units. A slot the set offers and the gold does not name is an error.
    for q in &qs {
        let want: Vec<&str> = templates::slots(q.intent).iter().map(|s| s.name).collect();
        let have: Vec<&str> = gold[&q.qid].iter().map(|(s, _)| s.as_str()).collect();
        assert!(
            want.iter().all(|w| have.contains(w)),
            "{}: the gold's slots {have:?} do not cover {}'s {want:?}",
            q.qid,
            q.intent
        );
    }

    let opts = AskOptions::new(AgentId::new("tool:ge-t10").expect("an agent"));
    let mut report = String::new();
    let _ = writeln!(
        report,
        "GE-T10 — dossier slots against gold (SMYSL-2.5 §5.5), template set {} ({})",
        templates::SET,
        templates::hash()[..6]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let _ = writeln!(
        report,
        "corpus {} ({} units), {} questions in {} rows, {} gold slots{}",
        corpus.file_name().and_then(|s| s.to_str()).unwrap_or("?"),
        store.units().count(),
        gold.len(),
        qs.len(),
        gold.values().map(Vec::len).sum::<usize>(),
        if c_over || q_over || g_over {
            "  [override]"
        } else {
            ""
        }
    );

    // --- 3. the slots alone ------------------------------------------------------------------
    let mut slots_alone: BTreeMap<String, Tally> = BTreeMap::new();
    let mut by_intent: BTreeMap<String, Tally> = BTreeMap::new();
    let mut by_pair: BTreeMap<(String, String), Tally> = BTreeMap::new();
    let mut misses: Vec<String> = Vec::new();
    let mut topics: Vec<String> = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for q in &qs {
        if !seen.insert(&q.qid) {
            continue;
        }
        let prefix = q.qid.split('-').next().unwrap_or_default();
        let label = &subjects[prefix];
        let uid = labels[label];
        let name = store
            .get(&uid)
            .expect("the subject entity")
            .core
            .gist
            .clone();
        // The English question with its intent given and its subject picked, so the templates
        // see what was asked of the subject (set 2 narrows to it) and nothing of the front is
        // measured. Set 1 asked with the subject's name alone, which was the same thing while
        // no slot read the question.
        let text = qs
            .iter()
            .find(|r| r.qid == q.qid && r.lang == "en")
            .map_or(name.as_str(), |r| r.text.as_str());
        let d = ask(
            text,
            &env,
            &opts.clone().with_intent(q.intent).with_subject(uid),
        )
        .unwrap_or_else(|e| panic!("{}: {e:?}", q.qid));
        if d.slots
            .iter()
            .any(|s| templates::narrowed(q.intent, s.name))
        {
            topics.push(format!(
                "    {:<14} [{}] {}",
                q.qid,
                d.topic.join(" "),
                if d.topic_found {
                    "narrows"
                } else {
                    "found nothing about the subject, so no slot narrows"
                }
            ));
        }
        let found = slots_of(&d, &names);
        for (slot, want) in &gold[&q.qid] {
            let got = found.get(slot).cloned().unwrap_or_default();
            slots_alone.entry(slot.clone()).or_default().add(&got, want);
            by_intent
                .entry(q.intent.to_string())
                .or_default()
                .add(&got, want);
            by_pair
                .entry((slot.clone(), q.intent.to_string()))
                .or_default()
                .add(&got, want);
            for u in got.difference(want) {
                misses.push(format!("    {:<14} {:<15} + {u}", q.qid, slot));
            }
            for u in want.difference(&got) {
                misses.push(format!("    {:<14} {:<15} - {u}", q.qid, slot));
            }
        }
    }
    let _ = writeln!(
        report,
        "\n3. the slots alone (English question, gold intent, subject picked)"
    );
    let _ = writeln!(report, "  {:<16} {:>6} {:>6}", "slot", "P", "R");
    let mut all = Tally::default();
    for (slot, t) in &slots_alone {
        row(&mut report, slot, t);
        all.merge(*t);
    }
    row(&mut report, "all slots", &all);
    let _ = writeln!(report, "  {:<16} {:>6} {:>6}", "intent", "P", "R");
    for (intent, t) in &by_intent {
        row(&mut report, intent, t);
    }
    let _ = writeln!(report, "  the criterion's two slots, by intent");
    for ((slot, intent), t) in &by_pair {
        if slot == "definition" || slot == "counter" {
            row(&mut report, &format!("{slot}/{intent}"), t);
        }
    }
    let _ = writeln!(
        report,
        "  the topic each verify and explain question narrows to:"
    );
    for t in &topics {
        let _ = writeln!(report, "{t}");
    }
    let _ = writeln!(
        report,
        "  every disagreement (+ found, not gold; - gold, not found):"
    );
    for m in &misses {
        let _ = writeln!(report, "{m}");
    }

    // --- 1 and 2. end to end, and the front, per language -------------------------------------
    let mut end_to_end: BTreeMap<String, Tally> = BTreeMap::new();
    let mut front: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    let mut failed: Vec<String> = Vec::new();
    for q in &qs {
        let f = front.entry(q.lang.clone()).or_default();
        f.0 += 1;
        let t = end_to_end.entry(q.lang.clone()).or_default();
        let empty = BTreeSet::new();
        match ask(&q.text, &env, &opts) {
            Ok(d) => {
                let right_intent = d.classified.intent == q.intent;
                if right_intent {
                    f.1 += 1;
                } else {
                    failed.push(format!(
                        "    {:<14} {}  read as {}: {}",
                        q.qid, q.lang, d.classified.intent, q.text
                    ));
                }
                let prefix = q.qid.split('-').next().unwrap_or_default();
                let right_subject = d.entities.contains(&labels[&subjects[prefix]]);
                if right_subject {
                    f.2 += 1;
                } else {
                    failed.push(format!(
                        "    {:<14} {}  subject `{}`: {}",
                        q.qid, q.lang, d.subject, q.text
                    ));
                }
                let found = if right_intent && right_subject {
                    slots_of(&d, &names)
                } else {
                    // Wrong intent: its slots answer another question. Wrong subject: about
                    // another thing. Either way the person got nothing for these gold slots.
                    BTreeMap::new()
                };
                for (slot, want) in &gold[&q.qid] {
                    t.add(found.get(slot).unwrap_or(&empty), want);
                }
            }
            Err(e) => {
                let why = match &e {
                    AskError::Refused(d) => d.code.as_str().to_string(),
                    AskError::NoSubject { .. } => {
                        f.1 += 1;
                        "no subject".to_string()
                    }
                    AskError::Ambiguous { .. } => {
                        f.1 += 1;
                        "W417".to_string()
                    }
                    other => format!("{other:?}"),
                };
                failed.push(format!("    {:<14} {}  {why}: {}", q.qid, q.lang, q.text));
                for (_, want) in &gold[&q.qid] {
                    t.add(&empty, want);
                }
            }
        }
    }
    let _ = writeln!(
        report,
        "\n1. end to end, per language (as typed, no --lang)"
    );
    let _ = writeln!(report, "  {:<16} {:>6} {:>6}", "lang", "P", "R");
    let mut all = Tally::default();
    for lang in LANGS {
        if let Some(t) = end_to_end.get(lang) {
            row(&mut report, lang, t);
            all.merge(*t);
        }
    }
    row(&mut report, "all languages", &all);
    let _ = writeln!(report, "\n2. the front, per language");
    let _ = writeln!(report, "  {:<16} {:>9} {:>9}", "lang", "intent", "subject");
    for lang in LANGS {
        if let Some((n, i, s)) = front.get(lang) {
            let _ = writeln!(report, "  {lang:<16} {i:>4}/{n:<4} {s:>4}/{n:<4}");
        }
    }
    let _ = writeln!(report, "  every question the front lost:");
    for f in &failed {
        let _ = writeln!(report, "{f}");
    }

    // --- the criterion ------------------------------------------------------------------------
    let _ = writeln!(
        report,
        "\nkill criterion (draft 3 §23): precision < {BAR} for definition or counter"
    );
    for slot in ["definition", "counter"] {
        let p = slots_alone.get(slot).and_then(Tally::precision);
        let _ = writeln!(
            report,
            "  {slot:<16} P {}  {}",
            fmt(p),
            match p {
                Some(p) if p < BAR => "below the bar",
                Some(_) => "meets the bar",
                None => "nothing found, so no precision",
            }
        );
    }

    if let Some(out) = std::env::var_os("SMYSL_GE_T10_OUT") {
        std::fs::write(&out, &report).expect("SMYSL_GE_T10_OUT is writable");
    }
    Run {
        report,
        slots_alone,
        end_to_end,
        front,
    }
}

fn overridden() -> bool {
    [
        "SMYSL_GE_T10_CORPUS",
        "SMYSL_GE_T10_QUESTIONS",
        "SMYSL_GE_T10_GOLD",
        "SMYSL_GE_T10_SUBJECTS",
    ]
    .iter()
    .any(|v| std::env::var_os(v).is_some())
}

fn three(x: Option<f64>) -> String {
    fmt(x)
}

#[test]
fn ge_t10() {
    let r = run();
    println!("{}", r.report);
    if overridden() {
        return;
    }

    // The figures SMYSL-2.5 records, each (P, R) to three places — §6 TX-P8 step 4's under
    // set 1, moved by set 2 (OQ-87, OQ-88) where the comment says; `—` is a slot that found
    // nothing, which has no precision.
    let slots: &[(&str, &str, &str)] = &[
        ("cause", "1.000", "1.000"), // sets 1, 2: 1.000, 0.667
        ("characteristic", "0.554", "1.000"),
        ("contrast", "1.000", "1.000"),
        ("counter", "1.000", "0.917"), // set 1: 0.750, 0.750; set 2: 1.000, 0.667
        ("definition", "1.000", "1.000"),
        ("dependent", "1.000", "1.000"), // sets 1, 2: 1.000, 0.833
        ("evidence", "0.667", "0.889"),
        ("exemplifies", "1.000", "1.000"), // sets 1, 2: 1.000, 0.800
        ("holder", "1.000", "1.000"),
        ("hypothesis", "1.000", "1.000"),
        ("introduction", "0.636", "1.000"),
        ("location", "1.000", "0.875"),
        ("open-question", "1.000", "1.000"),
        ("related", "1.000", "1.000"),
        ("reported", "0.857", "0.857"), // set 1: 0.636, 1.000
        ("revision", "1.000", "1.000"), // sets 1, 2: —, 0.000
        ("stance", "1.000", "0.667"),
        ("support", "0.800", "1.000"), // sets 1, 2: 1.000, 1.000
    ];
    let mut moved = Vec::new();
    for (slot, p, rc) in slots {
        let t = &r.slots_alone[*slot];
        let got = (three(t.precision()), three(t.recall()));
        if (got.0.as_str(), got.1.as_str()) != (*p, *rc) {
            moved.push(format!(
                "slot {slot}: ({p}, {rc}) is now ({}, {})",
                got.0, got.1
            ));
        }
    }
    let langs: &[(&str, &str, &str, usize, usize)] = &[
        // set 1: en 0.774/0.890, de 0.728/0.652, es 0.770/0.834, fr 0.767/0.818,
        // ru 0.773/0.751; set 2: en 0.806/0.873, de 0.738/0.652, es 0.778/0.834,
        // fr 0.772/0.807, ru 0.784/0.740
        ("en", "0.815", "0.950", 30, 30),
        ("de", "0.746", "0.713", 23, 22),
        ("es", "0.783", "0.895", 27, 27),
        ("fr", "0.775", "0.856", 26, 26),
        ("ru", "0.787", "0.796", 29, 23),
    ];
    for (lang, p, rc, intent, subject) in langs {
        let t = &r.end_to_end[*lang];
        let got = (three(t.precision()), three(t.recall()));
        if (got.0.as_str(), got.1.as_str()) != (*p, *rc) {
            moved.push(format!(
                "{lang} end to end: ({p}, {rc}) is now ({}, {})",
                got.0, got.1
            ));
        }
        let (_, i, s) = r.front[*lang];
        if (i, s) != (*intent, *subject) {
            moved.push(format!(
                "{lang} front: intent {intent}, subject {subject} is now {i}, {s}"
            ));
        }
    }
    assert!(
        moved.is_empty(),
        "GE-T10 moved from what SMYSL-2.5 §6 records — rerun, read the report, and record the \
         new figures there and here together:\n  {}\n{}",
        moved.join("\n  "),
        r.report
    );

    // The criterion no longer fires under set 2. `ask` stays experimental by decision
    // (2026-10-28, SMYSL-2.5 OQ-88: single-hand gold, and counter's recall fell on the fresh
    // questions), so the label is `ASK_EXPERIMENTAL`'s and not this test's; what is asserted
    // is that the criterion is met, so a change that fires it again fails here.
    let counter = r.slots_alone["counter"]
        .precision()
        .expect("counter found units");
    assert!(counter >= BAR, "counter-evidence is below the bar again");
}

/// The fresh set: the figures SMYSL-2.5 OQ-88 records, with set 1's over the same questions
/// beside them for the comparison the set exists to make.
#[test]
fn ge_t10_fresh() {
    let r = run_fresh();
    println!("{}", r.report);

    let slots: &[(&str, &str, &str)] = &[
        ("counter", "1.000", "0.706"), // set 1: 0.478, 0.647; set 2: 1.000, 0.471
        ("reported", "1.000", "0.500"), // set 1: 0.250, 1.000
        ("evidence", "0.435", "0.714"), // not narrowed; as set 1
    ];
    let mut moved = Vec::new();
    for (slot, p, rc) in slots {
        let t = &r.slots_alone[*slot];
        let got = (three(t.precision()), three(t.recall()));
        if (got.0.as_str(), got.1.as_str()) != (*p, *rc) {
            moved.push(format!(
                "slot {slot}: ({p}, {rc}) is now ({}, {})",
                got.0, got.1
            ));
        }
    }
    let mut all = Tally::default();
    for t in r.slots_alone.values() {
        all.merge(*t);
    }
    let got = (three(all.precision()), three(all.recall()));
    if (got.0.as_str(), got.1.as_str()) != ("0.566", "0.750") {
        moved.push(format!(
            "all slots: (0.566, 0.750) is now ({}, {})", // set 1: 0.441, 0.750; set 2: 0.565, 0.650
            got.0, got.1
        ));
    }
    let langs: &[(&str, &str, &str)] = &[
        ("en", "0.566", "0.750"),
        ("de", "0.484", "0.750"),
        ("es", "0.458", "0.825"),
        ("fr", "0.460", "0.725"),
        ("ru", "0.519", "0.675"),
    ];
    for (lang, p, rc) in langs {
        let t = &r.end_to_end[*lang];
        let got = (three(t.precision()), three(t.recall()));
        if (got.0.as_str(), got.1.as_str()) != (*p, *rc) {
            moved.push(format!(
                "{lang} end to end: ({p}, {rc}) is now ({}, {})",
                got.0, got.1
            ));
        }
    }
    assert!(
        moved.is_empty(),
        "the fresh set moved from what SMYSL-2.5 OQ-88 records:\n  {}\n{}",
        moved.join("\n  "),
        r.report
    );
    let counter = r.slots_alone["counter"]
        .precision()
        .expect("counter found units");
    assert!(
        counter >= BAR,
        "counter-evidence is below the bar on fresh questions"
    );
}
