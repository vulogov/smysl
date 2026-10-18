//! `smysl ask` (SMYSL-2.5 §4.4, TX-P8 step 3).
//!
//! The library half of the step is `crates/smysl-query/tests/dossier.rs` and the answer step's is
//! `crates/smysl-ingest/tests/answer.rs`. What is asserted here is what the command adds: the
//! dossier rendered through the profile's headings, `SMY-W417` with the candidates a person can
//! pick from, `SMY-E468` as a usage error, `--save` appending a dossier the store then checks
//! clean and can rerun a slot of, and `--seed-check` agreeing with itself.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-cmd-ask-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("the binary under test must run")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

fn fixture() -> String {
    repo("fixtures/dossier/subjects.smy")
        .to_string_lossy()
        .to_string()
}

#[test]
fn a_dossier_renders_under_the_profile_s_headings_with_its_gap_visible() {
    let o = run(&["ask", "What is Naomi?", &fixture()]);
    assert!(o.status.success(), "{}", stderr(&o));
    let out = stdout(&o);
    for heading in [
        "## question",
        "## Definition",
        "## Characteristics",
        "## Counter-evidence",
        "## Gaps",
    ] {
        assert!(out.contains(heading), "{heading} missing:\n{out}");
    }
    assert!(
        out.contains("*exemplifies: nothing in scope answers this slot*"),
        "{out}"
    );
    assert!(
        out.contains("Ноеминь"),
        "the same-as hop answers in Russian:\n{out}"
    );
    let err = stderr(&o);
    assert!(err.contains("define (en/define/what is …)"), "{err}");
    assert!(err.contains("subject `Naomi` (2 entities)"), "{err}");
}

#[test]
fn an_ambiguous_subject_is_w417_with_the_candidates_listed() {
    let o = run(&["ask", "Who is Mary?", &fixture()]);
    assert_eq!(o.status.code(), Some(2));
    let err = stderr(&o);
    assert!(err.contains("SMY-W417"), "{err}");
    assert!(
        err.contains("alias:kjv/john") && err.contains("alias:kjv/luke"),
        "{err}"
    );

    // Pick the first one by the uid the listing printed.
    let pick = err
        .lines()
        .find(|l| l.contains("alias:kjv/john"))
        .and_then(|l| l.split_whitespace().nth(2))
        .expect("a --subject line")
        .to_string();
    let o = run(&["ask", "Who is Mary?", "--subject", &pick, &fixture()]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("Mary of Magdala"));
    assert!(!stdout(&o).contains("Mary of Nazareth"));
}

#[test]
fn a_question_no_rule_matches_is_a_usage_error_naming_intent() {
    let o = run(&["ask", "Naomi, please", &fixture()]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("SMY-E468"), "{}", stderr(&o));
    let o = run(&["ask", "Naomi, please", "--intent", "define", &fixture()]);
    assert!(o.status.success(), "{}", stderr(&o));
    let o = run(&[
        "ask",
        "What is Naomi?",
        "--templates",
        "smysl/dossier/9",
        &fixture(),
    ]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("SMY-E477"), "{}", stderr(&o));
}

#[test]
fn a_saved_dossier_checks_clean_and_its_slots_rerun() {
    // CBOR, because a dossier's records are not all surface-expressible: the `x.query/v1`
    // declaration carries fields, which `@schema` has no spelling for until TX-P12 step 7, and
    // a surface store refuses what it cannot write back. `q --save` meets the same refusal.
    let dir = scratch("save");
    let store = dir.join("subjects.cbor");
    let s = store.to_string_lossy().to_string();
    let o = run(&["merge", &fixture(), "-o", &s]);
    assert!(o.status.success(), "{}", stderr(&o));

    let o = run(&["ask", "What is Naomi?", "--save", &s]);
    assert!(o.status.success(), "{}", stderr(&o));
    let saved = stderr(&o);
    let question = saved
        .split("question ")
        .nth(1)
        .and_then(|r| r.split(',').next())
        .expect("the saved question's uid")
        .trim()
        .to_string();

    let o = run(&["check", &s]);
    assert!(o.status.success(), "{}\n{}", stdout(&o), stderr(&o));

    // Asking again writes the same records, which a store holds once.
    let o = run(&["ask", "What is Naomi?", "--save", &s]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(
        stderr(&o).contains(&question),
        "the same question, the same unit"
    );
    let o = run(&[
        "q",
        "from store:subjects\nwhere schema = \"x.query/saved\"\ncount",
        &s,
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "5", "five slots, saved once each");

    // A slot is a saved query like any other, so `q --rerun` takes it and finds nothing new.
    let o = run(&[
        "q",
        "from store:subjects\nwhere schema = \"x.query/saved\"\nunits",
        &s,
    ]);
    let slot = stdout(&o)
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().next())
        .expect("a slot uid")
        .to_string();
    let o = run(&["q", "--rerun", &slot, &s]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("data effect: 0 added, 0 removed"),
        "{}",
        stderr(&o)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn seed_check_asks_twice_and_agrees() {
    let o = run(&["--seed-check", "ask", "What is Naomi?", &fixture()]);
    assert!(o.status.success(), "{}", stderr(&o));
}

#[test]
fn answer_needs_save_and_yes_needs_answer() {
    let o = run(&["ask", "What is Naomi?", "--answer", &fixture()]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("add --save"), "{}", stderr(&o));
    let o = run(&["ask", "What is Naomi?", "--yes", &fixture()]);
    assert_eq!(o.status.code(), Some(2));
    // And `--seed-check` refuses the invocation that asks a model, before it runs: over a copy,
    // so a dispatcher that did not refuse could not write to the fixture.
    let dir = scratch("seed");
    let store = dir.join("subjects.smy");
    std::fs::copy(fixture(), &store).expect("copy");
    let before = std::fs::read(&store).expect("read");
    let s = store.to_string_lossy().to_string();
    let o = run(&[
        "--seed-check",
        "ask",
        "What is Naomi?",
        "--answer",
        "--save",
        &s,
    ]);
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert!(stderr(&o).contains("--answer"), "{}", stderr(&o));
    assert_eq!(
        std::fs::read(&store).expect("read"),
        before,
        "nothing was written"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// GE-T10's kill criterion, applied (TX-P8 step 4): `ask` is listed as experimental and every
/// run says why on stderr, while stdout — the dossier a script reads — is unchanged.
#[test]
fn ask_is_labelled_experimental_with_the_figure_that_made_it_so() {
    let o = run(&["ask", "--help"]);
    assert!(o.status.success());
    assert!(stdout(&o).contains("Experimental"), "{}", stdout(&o));

    let o = run(&["ask", "What is Naomi?", &fixture()]);
    assert!(o.status.success(), "{}", stderr(&o));
    let err = stderr(&o);
    assert!(
        err.contains("experimental") && err.contains("0.750") && err.contains("GE-T10"),
        "{err}"
    );
    assert!(!stdout(&o).contains("experimental"));
}
