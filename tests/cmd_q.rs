//! `smysl q` (SMYSL-2.5 §4.4, TX-P6 step 7).
//!
//! The step's exit is *"every pure query passes `--seed-check`, plans stable"*, and both
//! halves are asserted against things that already exist rather than against this file's own
//! expectations: the plans `--explain` prints are compared byte for byte with the planner
//! goldens in `fixtures/sq/plans/`, which step 2 blessed from the library directly, so the
//! command is shown to add nothing and lose nothing on the way to the terminal; and
//! `--seed-check` is shown to run a pure query twice and agree with itself, which is the
//! assertion §3.2 specifies and the one the global flag could not make before this command.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-cmd-q-{name}-{}", std::process::id()));
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

fn f1() -> String {
    repo("fixtures/corpus/F1-incident.smy")
        .to_string_lossy()
        .to_string()
}

/// `--explain` over every corpus fixture prints the plan step 2 blessed for it.
///
/// This is the "plans stable" half of the exit, and it is what makes the store's name matter:
/// `q` names the scope after the file's stem, so `from store:F1-incident` over
/// `F1-incident.smy` resolves and the golden written under that name is the golden printed.
#[test]
fn explain_prints_the_planner_goldens_byte_for_byte() {
    let corpus = repo("fixtures/corpus");
    let mut seen = 0;
    for entry in std::fs::read_dir(&corpus).expect("the corpus").flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|x| x != "smy") {
            continue;
        }
        let stem = path
            .file_stem()
            .expect("a stem")
            .to_string_lossy()
            .to_string();
        let golden = repo("fixtures/sq/plans").join(format!("{stem}.explain"));
        let Ok(want) = std::fs::read_to_string(&golden) else {
            continue;
        };
        let o = run(&[
            "q",
            "--explain",
            &format!("from store:{stem}"),
            &path.to_string_lossy(),
        ]);
        assert!(o.status.success(), "{stem}: {}", stderr(&o));
        assert_eq!(
            stdout(&o),
            want,
            "{stem}: the plan `q` prints is not the golden"
        );
        seen += 1;
    }
    assert!(seen >= 13, "only {seen} fixtures had a plan golden");
}

/// A pure query under `--seed-check` runs twice, agrees with itself, and answers.
#[test]
fn seed_check_runs_a_pure_query_twice_and_passes() {
    let q = "from store:F1-incident\nfind \"connection pool\"";
    let once = run(&["--seed-check", "q", q, &f1()]);
    assert!(once.status.success(), "{}", stderr(&once));
    assert!(
        stdout(&once).lines().count() >= 3,
        "a ranking was expected:\n{}",
        stdout(&once)
    );
    // Stable across invocations as well as within one: the same bytes from a second process.
    let twice = run(&["--seed-check", "q", q, &f1()]);
    assert_eq!(stdout(&once), stdout(&twice));
}

/// A model-dependent stage is refused by the planner before `--seed-check` has anything to
/// judge, and the refusal names what the stage needs.
#[test]
fn a_semantic_source_is_refused_before_seed_check_runs_anything() {
    let o = run(&[
        "--seed-check",
        "q",
        "from store:F1-incident\nfind \"pool\" ~sem",
        &f1(),
    ]);
    assert!(!o.status.success());
    let err = stderr(&o);
    assert!(err.contains("SMY-E461"), "{err}");
    assert!(err.contains("~sem"), "{err}");
    assert!(stdout(&o).is_empty(), "nothing ran");
}

/// `--why` prints the fusion's arithmetic on stderr, as `find --why` does, and leaves stdout
/// to the ranking.
#[test]
fn why_goes_to_stderr_and_the_ranking_stays_on_stdout() {
    let o = run(&[
        "q",
        "--why",
        "from store:F1-incident\nfind \"connection pool\"",
        &f1(),
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    let err = stderr(&o);
    assert!(err.contains("rrf"), "{err}");
    assert!(err.contains("rank 1"), "{err}");
    assert!(!stdout(&o).contains("rrf"));
}

/// `--json` is one object with the answer and the warnings.
#[test]
fn json_is_one_object_with_an_answer_and_warnings() {
    let o = run(&[
        "--json",
        "q",
        "from store:F1-incident\nfind \"connection pool\"\nlimit 2",
        &f1(),
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    let out = stdout(&o);
    assert!(
        out.starts_with("{\"answer\":{\"kind\":\"ranked\",\"hits\":["),
        "{out}"
    );
    assert!(out.trim_end().ends_with("]},\"warnings\":[]}"), "{out}");
    assert_eq!(out.matches("\"uid\":").count(), 2);

    let o = run(&["--json", "q", "from store:F1-incident\ncount", &f1()]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).starts_with("{\"answer\":{\"kind\":\"count\",\"count\":"));
}

/// A parameter is bound by `--param` and its absence is `SMY-E460` at execution.
#[test]
fn params_bind_and_an_unbound_one_is_refused_when_run() {
    let q = "from store:F1-incident\nfind $what";
    let bound = run(&["q", "--param", "what=pool", q, &f1()]);
    assert!(bound.status.success(), "{}", stderr(&bound));
    assert!(!stdout(&bound).is_empty());

    let unbound = run(&["q", q, &f1()]);
    assert!(!unbound.status.success());
    assert!(
        stderr(&unbound).contains("SMY-E460"),
        "{}",
        stderr(&unbound)
    );

    let malformed = run(&["q", "--param", "what", q, &f1()]);
    assert!(!malformed.status.success());
    assert!(stderr(&malformed).contains("K=V"), "{}", stderr(&malformed));
}

/// `pack … as view` writes a `View` record, which is the one thing the pure crate refused to
/// do and the CLI exists to do.
#[test]
fn pack_as_view_appends_a_view_record() {
    let dir = scratch("as-view");
    let store = dir.join("F1.cbor");
    let store = store.to_string_lossy().to_string();
    // A CBOR store, because a view has no surface form and `persist` says so for `.smy`.
    let converted = run(&["merge", &f1(), "-o", &store]);
    assert!(converted.status.success(), "{}", stderr(&converted));

    let before = std::fs::metadata(&store).expect("the store").len();
    let o = run(&[
        "q",
        "--view",
        "v/q-pool",
        "from store:F1\nfind \"connection pool\"\npack 2000 tokens as view",
        &store,
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(!stdout(&o).is_empty(), "the packed units are still printed");
    let after = std::fs::metadata(&store).expect("the store").len();
    assert!(after > before, "the store did not grow");

    // `bundle --view` emits the closure of a view the store declares, and fails on one it
    // does not — so it is the reader that can tell the record was written.
    let bundled = run(&["bundle", "--view", "v/q-pool", &store]);
    assert!(bundled.status.success(), "{}", stderr(&bundled));
    assert!(
        !stdout(&bundled).is_empty(),
        "the bundle of the written view is empty"
    );
    let missing = run(&["bundle", "--view", "v/q-nothing", &store]);
    assert!(
        !missing.status.success(),
        "a view nothing wrote must not bundle"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The two ways of giving a query are exclusive, and giving none is a usage error.
#[test]
fn the_query_is_given_once_or_not_at_all() {
    let dir = scratch("file");
    let file = dir.join("q.sq");
    std::fs::write(&file, "from store:F1-incident\ncount\n").expect("written");
    let file = file.to_string_lossy().to_string();

    // `-s` is the global store flag and `q` has a positional of the same name, so the global
    // goes before the subcommand, as it does for every command with a positional store.
    let from_file = run(&["-s", &f1(), "q", "--file", &file]);
    assert!(from_file.status.success(), "{}", stderr(&from_file));
    assert!(stdout(&from_file).trim().parse::<usize>().is_ok());

    let both = run(&["q", "--file", &file, "from store:F1-incident", &f1()]);
    assert!(!both.status.success());
    assert!(stderr(&both).contains("once"), "{}", stderr(&both));

    let none = run(&["q", &f1()]);
    // One positional: clap reads the path as the query, and the command then has no store.
    assert!(!none.status.success());
    assert!(
        stderr(&none).contains("no store given"),
        "{}",
        stderr(&none)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--library` binds the catalogue: the six items `StoreScope` refuses resolve, and the plan
/// names the library. No units are drawn here — `text add` writes parts and readings, and
/// units come from ingest — so the honest answer is a scope of zero, and the point is that it
/// is an *answer* rather than `SMY-E511`.
#[cfg(feature = "text")]
#[test]
fn library_binds_the_catalogue_and_its_scope_items_resolve() {
    let dir = scratch("library");
    let root = dir.to_string_lossy().to_string();
    let added = run(&[
        "text",
        "add",
        &repo("fixtures/library/readers/gen1.usfm").to_string_lossy(),
        "--reader",
        "usfm/1",
        "--alias",
        "kjv/gen",
        "--lang",
        "en",
        "--licence",
        "public-domain",
        "--carry",
        "text",
        "-s",
        &root,
    ]);
    assert!(added.status.success(), "{}", stderr(&added));

    for scope in ["texts", "library", "collection:kjv"] {
        let o = run(&["q", "--library", &format!("from {scope}\ncount"), &root]);
        assert!(o.status.success(), "{scope}: {}", stderr(&o));
        assert_eq!(stdout(&o).trim(), "0", "{scope}");
    }
    let explained = run(&[
        "q",
        "--library",
        "--explain",
        "from collection:kjv\nfind \"beginning\"",
        &root,
    ]);
    assert!(explained.status.success(), "{}", stderr(&explained));
    let plan = stdout(&explained);
    assert!(plan.contains("3 scope: collection:kjv via"), "{plan}");
    assert!(plan.contains("5 source: lexical"), "{plan}");

    let refused = run(&["q", "--library", "from channel:nowhere\ncount", &root]);
    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains("SMY-E511"),
        "{}",
        stderr(&refused)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--save` writes the query after it ran; `--rerun` runs it again and reports the data effect
/// (SMYSL-2.5 §3.6; TX-P8 step 1). The store grows between the two by one claim the query
/// finds, merged in through `merge` because that is the one pure way the CLI has of adding a
/// unit, and the effect reported is exactly that claim.
#[test]
fn save_then_rerun_reports_the_data_effect() {
    let dir = scratch("save-rerun");
    let store = dir.join("save.cbor").to_string_lossy().to_string();
    let converted = run(&["merge", &f1(), "-o", &store]);
    assert!(converted.status.success(), "{}", stderr(&converted));

    let q = "from store:save\nfind \"connection pool\"";
    let saved = run(&["q", "--save", q, &store]);
    assert!(saved.status.success(), "{}", stderr(&saved));
    let err = stderr(&saved);
    let uid = err
        .lines()
        .find_map(|l| l.strip_prefix("smysl q: saved "))
        .unwrap_or_else(|| panic!("no `saved <uid>` line in:\n{err}"))
        .trim()
        .to_string();
    assert!(uid.starts_with("b3:"), "{uid}");

    // Nothing changed: no effect.
    let same = run(&["q", "--rerun", &uid, &store]);
    assert!(same.status.success(), "{}", stderr(&same));
    assert!(
        stderr(&same).contains("data effect: 0 added, 0 removed"),
        "{}",
        stderr(&same)
    );
    assert_eq!(
        stdout(&same),
        stdout(&saved),
        "a pinned rerun over unchanged data is the run"
    );

    // One claim the query finds, merged in after the save.
    let extra = dir.join("extra.smy");
    std::fs::write(
        &extra,
        "@claim c/extra { status: speculative }\n~ The connection pool was starved by a retry storm.\n",
    )
    .expect("written");
    // Under the same stem: the saved text says `from store:save`, and a store is named by
    // the file's stem, so the grown copy keeps the name in a directory of its own.
    std::fs::create_dir_all(dir.join("grown")).expect("a directory");
    let grown = dir.join("grown/save.cbor").to_string_lossy().to_string();
    let merged = run(&["merge", &store, &extra.to_string_lossy(), "-o", &grown]);
    assert!(merged.status.success(), "{}", stderr(&merged));

    let again = run(&["q", "--rerun", &uid, &grown]);
    assert!(again.status.success(), "{}", stderr(&again));
    let err = stderr(&again);
    assert!(err.contains("data effect: 1 added, 0 removed"), "{err}");
    assert!(err.lines().any(|l| l.starts_with("  + b3:")), "{err}");

    // `--explain` on a rerun plans the saved text and runs nothing.
    let explained = run(&["q", "--rerun", &uid, "--explain", &grown]);
    assert!(explained.status.success(), "{}", stderr(&explained));
    assert!(stdout(&explained).starts_with("plan sq/1\n"));

    // The schema effect waits for the lens engine, and a rerun saved again would need it.
    let current = run(&["q", "--rerun", &uid, "--current", &grown]);
    assert!(!current.status.success());
    assert!(
        stderr(&current).contains("SMY-E461"),
        "{}",
        stderr(&current)
    );
    let resave = run(&["q", "--rerun", &uid, "--save", &grown]);
    assert!(!resave.status.success());
    assert!(
        stderr(&resave).contains("supersedes"),
        "{}",
        stderr(&resave)
    );

    // A uid that is not a saved query is named for what it is.
    let one = run(&["q", "from store:save\nfind \"canary\"\nlimit 1", &store]);
    assert!(one.status.success(), "{}", stderr(&one));
    let claim_uid = stdout(&one)
        .split_whitespace()
        .find(|w| w.starts_with("b3:"))
        .expect("a hit")
        .to_string();
    let wrong = run(&["q", "--rerun", &claim_uid, &store]);
    assert!(!wrong.status.success());
    assert!(
        stderr(&wrong).contains("x.query/saved"),
        "{}",
        stderr(&wrong)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
