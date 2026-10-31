//! GE-T10's third question set (SMYSL-2.5 §5.5, OQ-22's owed spelling fixes and subject
//! morphology): the front of `ask` alone — the intent the rule table reads and the subject the
//! entity port finds — over `fixtures/dossier/third/questions.tsv`, sixteen questions in five
//! languages written before either fix and scored first without them
//! (`fixtures/dossier/third/baseline.txt`); and over the fourth set, which does the same for
//! the intent rules the third set's losses named (`fixtures/dossier/fourth/`), and the fifth
//! for the rules the fourth's named (`fixtures/dossier/fifth/`), and the sixth for the fifth's
//! (`fixtures/dossier/sixth/`).
//!
//! It is here and not beside `smysl-query`'s GE-T10 because the subject's stems are this
//! crate's: [`Stemmed`] reads them through the analyzer chain that indexes a library, and the
//! figure is meant to be the production path's, not a test's copy of it. Two columns for the
//! subject: as spelled, the port `ask` had before, and through the production port — stems,
//! then a word as a part of a name (`fixtures/dossier/parts/`, a German compound and a Russian
//! adjective), with the words that only look like one scored alone (`false-friends.tsv`).

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
                "    {qid:<14} {lang}  found by stems or parts only: {question}"
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
        "GE-T10, {title}: the front\n  lang     intent   subject spelled   subject found\n"
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

/// Each of the third to sixth sets out of sample for the rules declared before it was
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
            (80, 80, 74, 80),
        ),
        (
            "fourth set",
            "fixtures/dossier/fourth/questions.tsv",
            (80, 80, 72, 80),
        ),
        (
            "fifth set",
            "fixtures/dossier/fifth/questions.tsv",
            (80, 76, 70, 76),
        ),
        (
            "sixth set",
            "fixtures/dossier/sixth/questions.tsv",
            (80, 52, 46, 52),
        ),
        (
            "yes-or-no set",
            "fixtures/dossier/yesno/questions.tsv",
            (50, 44, 41, 44),
        ),
        (
            "parts set",
            "fixtures/dossier/parts/questions.tsv",
            (32, 32, 0, 32),
        ),
        (
            "set 1 (in-sample)",
            "fixtures/dossier/gold/questions.tsv",
            (150, 150, 144, 150),
        ),
        (
            "fresh set (in-sample)",
            "fixtures/dossier/fresh/questions.tsv",
            (85, 85, 79, 85),
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

/// Rules 22 and 23 directly: a German compound's first part and a Russian adjective find the
/// name, a compound's later part does not, and nothing is found in a language without a rule.
#[test]
fn a_word_names_a_subject_as_its_first_part_or_its_adjective() {
    let src = std::fs::read_to_string(repo("fixtures/dossier/gold/corpus.smy")).unwrap();
    let parsed = parse_surface(&src).unwrap();
    let labels: BTreeMap<String, Uid> = parsed
        .labels
        .iter()
        .map(|(l, u)| (l.to_string(), *u))
        .collect();
    let store = Store::from_records(parsed.records);
    let finds = |word: &str, lang: &str| -> Vec<Uid> {
        smysl_text::scope::derived_candidates(&store, word, lang)
            .into_iter()
            .flatten()
            .collect()
    };

    assert!(finds("Cholerabazillus", "de").contains(&labels["e/cholera"]));
    assert!(finds("Listersche", "de").contains(&labels["e/lister"]));
    assert!(
        finds("Kuhpocken", "de").is_empty(),
        "a later part is not the subject"
    );
    assert!(
        finds("Cholera", "de").is_empty(),
        "nothing after the name, no compound"
    );
    assert!(finds("холерный", "ru").contains(&labels["e/cholera"]));
    assert!(finds("дженнеровский", "ru").contains(&labels["e/jenner"]));
    assert!(finds("cholerabacillus", "en").is_empty());
}

/// The false-friend list (`fixtures/dossier/parts/false-friends.tsv`, rules 24 and 25): each word
/// scored alone by the parts pass, as `false hits` (a word naming no entity that finds one) and
/// `guards` (a word naming one that finds it). Pinned like the sets.
#[test]
fn ge_t10_false_friends() {
    let src = std::fs::read_to_string(repo("fixtures/dossier/gold/corpus.smy")).unwrap();
    let parsed = parse_surface(&src).unwrap();
    let labels: BTreeMap<String, Uid> = parsed
        .labels
        .iter()
        .map(|(l, u)| (l.to_string(), *u))
        .collect();
    let store = Store::from_records(parsed.records);
    let list = std::fs::read_to_string(repo("fixtures/dossier/parts/false-friends.tsv")).unwrap();
    let (mut friends, mut hits, mut guards, mut found) = (0, 0, 0, 0);
    let mut out = String::from("GE-T10, false-friend list: the parts pass alone\n");
    for line in list
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let [id, lang, word, want] = line.split('\t').collect::<Vec<_>>()[..] else {
            panic!("a row has four columns: {line}");
        };
        let got: Vec<Uid> = smysl_text::scope::derived_candidates(&store, word, lang)
            .into_iter()
            .flatten()
            .collect();
        let named: Vec<&str> = labels
            .iter()
            .filter(|(_, u)| got.contains(u))
            .map(|(l, _)| l.as_str())
            .collect();
        let ok = if want == "-" {
            friends += 1;
            hits += usize::from(!got.is_empty());
            got.is_empty()
        } else {
            guards += 1;
            let hit = got.contains(&labels[want]);
            found += usize::from(hit);
            hit
        };
        if !ok {
            let _ = writeln!(
                out,
                "    {id:<10} {lang}  {word}: wanted {want}, found [{}]",
                named.join(" ")
            );
        }
    }
    let _ = writeln!(
        out,
        "  false hits {hits}/{friends}, guards found {found}/{guards}"
    );
    println!("{out}");
    assert_eq!((friends, hits, guards, found), (14, 1, 16, 16), "{out}");
}

/// `guess_lang` over every question set, whose rows carry their language: what `ask --misses`
/// would mark as each question's language with no `--lang`, and how often it is right. A
/// heuristic's figure, pinned so a change to its word lists is a change to record.
#[test]
fn guess_lang_over_the_question_sets() {
    let (mut asked, mut right, mut none) = (0, 0, 0);
    let mut wrong: Vec<String> = Vec::new();
    for set in [
        "gold", "fresh", "third", "fourth", "fifth", "sixth", "parts", "yesno", "forms",
    ] {
        let src = std::fs::read_to_string(repo(&format!("fixtures/dossier/{set}/questions.tsv")))
            .unwrap();
        for line in src
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        {
            let cols: Vec<&str> = line.split('\t').collect();
            let [id, _, lang, question, ..] = cols[..] else {
                continue;
            };
            asked += 1;
            match smysl_query::dossier::intent::guess_lang(question) {
                Some(g) if g == lang => right += 1,
                Some(g) => wrong.push(format!("    {set}/{id} {lang} as {g}: {question}")),
                None => {
                    none += 1;
                    wrong.push(format!("    {set}/{id} {lang} as none: {question}"));
                }
            }
        }
    }
    println!(
        "guess_lang: {right}/{asked} right, {} wrong, {none} no guess\n{}",
        asked - right - none,
        wrong.join("\n")
    );
    // Before the lists were completed (1.12.0): 563/587 over the first seven sets, 1 wrong and
    // 23 unguessed; 634/662 with the yes-or-no and forms sets, 1 wrong and 27 unguessed.
    assert_eq!((asked, right, none), (662, 637, 25));
}

/// Precision and recall of `counter` and `reported` over (question, slot) pairs.
#[derive(Default)]
struct Narrowed {
    found: usize,
    right: usize,
    wanted: usize,
}

impl Narrowed {
    fn figures(&self) -> (String, String) {
        let f = |n: usize, d: usize| {
            if d == 0 {
                "—".to_string()
            } else {
                format!("{:.3}", n as f64 / d as f64)
            }
        };
        (f(self.right, self.found), f(self.right, self.wanted))
    }
}

/// The forms set (`fixtures/dossier/forms/`, rule 26): each English verify and explain question
/// of the gold and fresh sets with its topic's words inflected otherwise, scored on the two
/// narrowed slots against the original's gold, with the originals beside them as guards.
#[test]
fn ge_t10_forms() {
    let src = std::fs::read_to_string(repo("fixtures/dossier/gold/corpus.smy")).unwrap();
    let parsed = parse_surface(&src).unwrap();
    let labels: BTreeMap<String, Uid> = parsed
        .labels
        .iter()
        .map(|(l, u)| (l.to_string(), *u))
        .collect();
    let names: BTreeMap<Uid, String> = labels.iter().map(|(l, u)| (*u, l.clone())).collect();
    let store = Store::from_records(parsed.records);
    let scope = StoreScope::new("ge-t10", &store);
    let stemmed = Stemmed::new(&store, &scope);
    let env = Env::new(&store, &scope).with_entities(&stemmed as &dyn Entities);
    let opts = AskOptions::new(AgentId::new("tool:ge-t10").unwrap()).with_lang("en");

    let rows = |rel: &str| -> Vec<Vec<String>> {
        std::fs::read_to_string(repo(rel))
            .unwrap()
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .map(|l| l.split('\t').map(str::to_string).collect())
            .collect()
    };
    let mut gold: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for r in rows("fixtures/dossier/gold/slots.tsv")
        .into_iter()
        .chain(rows("fixtures/dossier/fresh/slots.tsv"))
    {
        if r[1] == "counter" || r[1] == "reported" {
            let units = if r[2] == "-" {
                Vec::new()
            } else {
                r[2].split(' ').map(str::to_string).collect()
            };
            gold.insert((r[0].clone(), r[1].clone()), units);
        }
    }
    let originals: BTreeMap<String, String> = rows("fixtures/dossier/gold/questions.tsv")
        .into_iter()
        .chain(rows("fixtures/dossier/fresh/questions.tsv"))
        .filter(|r| r[2] == "en")
        .map(|r| (r[0].clone(), r[3].clone()))
        .collect();

    let mut out = String::from("GE-T10, forms set: the narrowed slots\n");
    let mut score = |set: &str, qid: &str, question: &str, t: &mut Narrowed| {
        let d = ask(question, &env, &opts);
        for slot in ["counter", "reported"] {
            let Some(want) = gold.get(&(qid.to_string(), slot.to_string())) else {
                continue;
            };
            let got: Vec<String> = match &d {
                Ok(d) => d
                    .slots
                    .iter()
                    .filter(|s| s.name == slot)
                    .flat_map(|s| s.units.iter())
                    .map(|u| names.get(u).cloned().unwrap_or_else(|| u.canonical()))
                    .collect(),
                Err(_) => Vec::new(),
            };
            t.found += got.len();
            t.wanted += want.len();
            t.right += got.iter().filter(|u| want.contains(u)).count();
            for u in got.iter().filter(|u| !want.contains(u)) {
                let _ = writeln!(out, "    {set:<9} {qid:<12} {slot:<9} + {u}");
            }
            for u in want.iter().filter(|u| !got.contains(u)) {
                let _ = writeln!(out, "    {set:<9} {qid:<12} {slot:<9} - {u}");
            }
        }
        match &d {
            Ok(d) => {
                let _ = writeln!(out, "    {set:<9} {qid:<12} [{}]", d.topic.join(" "));
            }
            Err(e) => {
                let _ = writeln!(out, "    {set:<9} {qid:<12} {e:?}");
            }
        }
    };
    let (mut variants, mut guards) = (Narrowed::default(), Narrowed::default());
    for r in rows("fixtures/dossier/forms/questions.tsv") {
        score("variant", &r[0], &r[3], &mut variants);
        score("guard", &r[0], &originals[&r[0]], &mut guards);
    }
    let (vp, vr) = variants.figures();
    let (gp, gr) = guards.figures();
    let _ = writeln!(out, "  variants P {vp} R {vr}; guards P {gp} R {gr}");
    println!("{out}");
    assert_eq!(
        (vp.as_str(), vr.as_str(), gp.as_str(), gr.as_str()),
        ("1.000", "0.733", "1.000", "0.733"),
        "{out}"
    );
}

/// A set whose rows want an intent or `none` (`fixtures/dossier/collide/`, `misread/`): counts
/// (asked, intent right, misread, subjects wanted, subjects found), where a `none` row is right
/// when it is not read as a verification, and a row is misread when it is read as an intent it
/// does not want.
fn wanted_set(title: &str, file: &str) -> ((usize, usize, usize, usize, usize), String) {
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
    let env = Env::new(&store, &scope).with_entities(&stemmed as &dyn Entities);
    let opts = AskOptions::new(AgentId::new("tool:ge-t10").unwrap());
    let subjects: BTreeMap<&str, &str> = SUBJECTS.iter().copied().collect();

    let (mut asked, mut right, mut misread, mut wanted, mut found) = (0, 0, 0, 0, 0);
    let mut out = format!("GE-T10, {title}: the front\n");
    let text = std::fs::read_to_string(repo(file)).unwrap();
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let r: Vec<&str> = line.split('\t').collect();
        let (qid, want, lang, question) = (r[0], r[1], r[2], r[3]);
        asked += 1;
        let read = smysl_query::dossier::classify(question, None);
        let ok = match (want, &read) {
            ("none", Ok(c)) => c.intent != Intent::Verify,
            ("none", Err(_)) => true,
            (w, Ok(c)) => Intent::parse(w) == Some(c.intent),
            _ => false,
        };
        right += usize::from(ok);
        misread += usize::from(!ok && read.is_ok());
        if !ok {
            let how = match &read {
                Ok(c) => format!("read as {}", c.intent),
                Err(d) => d.code.as_str().to_string(),
            };
            let _ = writeln!(
                out,
                "    {qid:<11} {lang}  wanted {want}, {how}: {question}"
            );
        }
        if want != "none" {
            wanted += 1;
            let uid = labels[subjects[qid.split('-').next().unwrap()]];
            match subject(question, &env, &opts, uid) {
                Ok(()) => found += 1,
                Err(why) => {
                    let _ = writeln!(out, "    {qid:<11} {lang}  {why}: {question}");
                }
            }
        }
    }
    let _ = writeln!(
        out,
        "  intent {right}/{asked}, misread {misread}, subject {found}/{wanted}"
    );
    ((asked, right, misread, wanted, found), out)
}

/// The collision set (rules 28 and 29): questions no rule should misread as a verification,
/// verifications the last-resort tier should still read, and German names in the genitive.
#[test]
fn ge_t10_collisions() {
    let (got, out) = wanted_set("collision set", "fixtures/dossier/collide/questions.tsv");
    println!("{out}");
    assert_eq!(got, (20, 19, 0, 15, 13), "{out}");
}

/// The misread set (rules 30 to 32): origins asked as traces, terms asked to be explained as
/// definitions, and a last-resort opener another language shares.
#[test]
fn ge_t10_misread() {
    let (got, out) = wanted_set("misread set", "fixtures/dossier/misread/questions.tsv");
    println!("{out}");
    assert_eq!(got, (40, 39, 0, 37, 36), "{out}");
}
