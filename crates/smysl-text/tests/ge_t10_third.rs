//! GE-T10's third question set (SMYSL-2.5 §5.5, OQ-22's owed spelling fixes and subject
//! morphology): the front of `ask` alone — the intent the rule table reads and the subject the
//! entity port finds — over `fixtures/dossier/third/questions.tsv`, sixteen questions in five
//! languages written before either fix and scored first without them
//! (`fixtures/dossier/third/baseline.txt`); and over the fourth set, which does the same for
//! the intent rules the third set's losses named (`fixtures/dossier/fourth/`), and the fifth
//! for the rules the fourth's named (`fixtures/dossier/fifth/`).
//!
//! It is here and not beside `smysl-query`'s GE-T10 because the subject's stems are this
//! crate's: [`Stemmed`] reads them through the analyzer chain that indexes a library, and the
//! figure is meant to be the production path's, not a test's copy of it. Two columns for the
//! subject: as spelled, the port `ask` had before, and through the stems.

#![cfg(feature = "stem")]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use smysl_core::ids::{AgentId, Uid};
use smysl_core::surface::parse_surface;
use smysl_graph::Store;
use smysl_query::dossier::Intent;
use smysl_query::ports::Entities;
use smysl_query::{ask, AskError, AskOptions, Env, StoreScope};
use smysl_text::scope::Stemmed;

/// The qid prefix's subject, as GE-T10's `SUBJECTS`.
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

#[derive(Default)]
struct Front {
    asked: usize,
    intent: usize,
    spelled: usize,
    stemmed: usize,
}

/// Whether `ask` found the subject `want` through `port`, and the line saying why not.
fn subject(text: &str, env: &Env<'_>, opts: &AskOptions, want: Uid) -> Result<(), String> {
    match ask(text, env, opts) {
        Ok(d) if d.entities.contains(&want) => Ok(()),
        Ok(d) => Err(format!("subject `{}`", d.subject)),
        Err(AskError::NoSubject { .. }) => Err("no subject".to_string()),
        Err(AskError::Refused(d)) => Err(d.code.as_str().to_string()),
        Err(e) => Err(format!("{e:?}")),
    }
}

fn front(questions: &str) -> (BTreeMap<String, Front>, Vec<String>) {
    let src = std::fs::read_to_string(repo("fixtures/dossier/gold/corpus.smy")).unwrap();
    let parsed = parse_surface(&src).unwrap();
    let labels: BTreeMap<String, Uid> = parsed
        .labels
        .iter()
        .map(|(l, u)| (l.to_string(), *u))
        .collect();
    let store = Store::from_records(parsed.records);
    let scope = StoreScope::new("ge-t10", &store);
    let stemmed = Stemmed::new(&store, &scope);
    let spelled_env = Env::new(&store, &scope).with_entities(&scope as &dyn Entities);
    let stemmed_env = Env::new(&store, &scope).with_entities(&stemmed as &dyn Entities);
    let opts = AskOptions::new(AgentId::new("tool:ge-t10").unwrap());
    let subjects: BTreeMap<&str, &str> = SUBJECTS.iter().copied().collect();

    let mut by_lang: BTreeMap<String, Front> = BTreeMap::new();
    let mut lost = Vec::new();
    let text = std::fs::read_to_string(repo(questions)).unwrap();
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let r: Vec<&str> = line.split('\t').collect();
        let (qid, intent, lang, question) = (r[0], Intent::parse(r[1]).unwrap(), r[2], r[3]);
        let want = labels[subjects[qid.split('-').next().unwrap()]];
        let f = by_lang.entry(lang.to_string()).or_default();
        f.asked += 1;
        match smysl_query::dossier::classify(question, None) {
            Ok(c) if c.intent == intent => f.intent += 1,
            Ok(c) => lost.push(format!(
                "    {qid:<14} {lang}  read as {}: {question}",
                c.intent
            )),
            Err(d) => lost.push(format!(
                "    {qid:<14} {lang}  {}: {question}",
                d.code.as_str()
            )),
        }
        let spelled = subject(question, &spelled_env, &opts, want);
        let stem = subject(question, &stemmed_env, &opts, want);
        f.spelled += usize::from(spelled.is_ok());
        f.stemmed += usize::from(stem.is_ok());
        match (&spelled, &stem) {
            (Err(_), Ok(())) => lost.push(format!(
                "    {qid:<14} {lang}  found by stems only: {question}"
            )),
            (_, Err(why)) if !why.starts_with("SMY-E468") => {
                lost.push(format!("    {qid:<14} {lang}  {why}: {question}"))
            }
            _ => {}
        }
    }
    (by_lang, lost)
}

fn table(title: &str, questions: &str) -> (String, Front) {
    let (by_lang, lost) = front(questions);
    let mut out = format!(
        "GE-T10, {title}: the front\n  lang     intent   subject spelled   subject stemmed\n"
    );
    let mut all = Front::default();
    for (lang, f) in &by_lang {
        let _ = writeln!(
            out,
            "  {lang:<8} {:>3}/{:<3}   {:>3}/{:<3}           {:>3}/{:<3}",
            f.intent, f.asked, f.spelled, f.asked, f.stemmed, f.asked
        );
        all.asked += f.asked;
        all.intent += f.intent;
        all.spelled += f.spelled;
        all.stemmed += f.stemmed;
    }
    let _ = writeln!(
        out,
        "  {:<8} {:>3}/{:<3}   {:>3}/{:<3}           {:>3}/{:<3}",
        "all", all.intent, all.asked, all.spelled, all.asked, all.stemmed, all.asked
    );
    out.push_str("  every question lost, and every subject only the stems found:\n");
    for l in &lost {
        let _ = writeln!(out, "{l}");
    }
    (out, all)
}

/// Each of the third, fourth and fifth sets out of sample for the rules declared before it was
/// written and in-sample for the ones its own losses named; and sets 1 and fresh beside them, in-sample for
/// the spellings (they found them) and for the stems (they found the inflections). Pinned:
/// a change to the table, the stemmer or the port that moves one is a change to record.
#[test]
fn ge_t10_third_set() {
    let mut moved = Vec::new();
    for (title, file, want) in [
        (
            "third set",
            "fixtures/dossier/third/questions.tsv",
            (80, 80, 69, 80),
        ),
        (
            "fourth set",
            "fixtures/dossier/fourth/questions.tsv",
            (80, 79, 70, 78),
        ),
        (
            "fifth set",
            "fixtures/dossier/fifth/questions.tsv",
            (80, 43, 41, 43),
        ),
        (
            "set 1 (in-sample)",
            "fixtures/dossier/gold/questions.tsv",
            (150, 150, 143, 150),
        ),
        (
            "fresh set (in-sample)",
            "fixtures/dossier/fresh/questions.tsv",
            (85, 85, 77, 83),
        ),
    ] {
        let (report, all) = table(title, file);
        println!("{report}");
        let got = (all.asked, all.intent, all.spelled, all.stemmed);
        if got != want {
            moved.push(format!(
                "{title}: (asked, intent, spelled, stemmed) {want:?} is now {got:?}"
            ));
        }
    }
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}
