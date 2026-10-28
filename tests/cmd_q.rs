//! `smysl q` (SMYSL-2.5 §4.4, TX-P6 step 7).
//!
//! The step's exit is *"every pure query passes `--seed-check`, plans stable"*, and both
//! halves are asserted against things that already exist rather than against this file's own
//! expectations: the plans `--explain` prints are compared byte for byte with the planner
//! goldens in `fixtures/sq/plans/`, which step 2 blessed from the library directly, so the
//! command is shown to add nothing and lose nothing on the way to the terminal; and
//! `--seed-check` is shown to run a pure query twice and agree with itself, which is the
//! assertion §3.2 specifies and the one the global flag could not make before this command.
#![cfg(feature = "cli")]

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

/// OQ-84: a save records the per-partition limit it ran under, and a rerun without
/// `--per-partition` runs under that limit, so its recipe is the saved one. A limit given on
/// the command line is the caller's choice and is labelled as another question.
#[test]
fn a_rerun_keeps_the_per_partition_limit_it_was_saved_under() {
    let dir = scratch("save-per-partition");
    let store = dir.join("pp.cbor").to_string_lossy().to_string();
    let converted = run(&["merge", &f1(), "-o", &store]);
    assert!(converted.status.success(), "{}", stderr(&converted));

    let q = "from store:pp\nfind \"connection pool\"";
    let saved = run(&["q", "--save", "--per-partition", "2", q, &store]);
    assert!(saved.status.success(), "{}", stderr(&saved));
    let err = stderr(&saved);
    let uid = err
        .lines()
        .find_map(|l| l.strip_prefix("smysl q: saved "))
        .unwrap_or_else(|| panic!("no `saved <uid>` line in:\n{err}"))
        .trim()
        .to_string();

    let same = run(&["q", "--rerun", &uid, &store]);
    assert!(same.status.success(), "{}", stderr(&same));
    assert!(
        !stderr(&same).contains("recipe differs"),
        "{}",
        stderr(&same)
    );
    assert_eq!(stdout(&same), stdout(&saved), "the same two hits");

    let other = run(&["q", "--rerun", &uid, "--per-partition", "10", &store]);
    assert!(other.status.success(), "{}", stderr(&other));
    assert!(
        stderr(&other).contains("recipe differs from the saved one"),
        "{}",
        stderr(&other)
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

    // `--current` runs under current revisions and reports the schema effect, which over a
    // store with no typed declarations is nothing (TX-P12 step 4; §19.5's is in
    // `crates/smysl-query/tests/lensed.rs`). A pinned rerun saved again is refused: it would
    // be the unit it reran.
    let current = run(&["q", "--rerun", &uid, "--current", &grown]);
    assert!(current.status.success(), "{}", stderr(&current));
    assert!(
        stderr(&current).contains("schema effect: 0 added, 0 removed"),
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
    // `--current --save` writes a new saved query superseding the old one (§3.6).
    let superseded = run(&["q", "--rerun", &uid, "--current", "--save", &grown]);
    assert!(superseded.status.success(), "{}", stderr(&superseded));
    assert!(
        stderr(&superseded).contains(&format!("superseding {uid}")),
        "{}",
        stderr(&superseded)
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

/// A Model2Vec directory small enough to write here (TX-P10 step 4): a word-level tokenizer over
/// F1's vocabulary and a random-free table — each word's vector is its index's bits spread over
/// eight dimensions — so `q --engine-model` runs with no download. What it embeds means nothing;
/// what is tested is the command around it.
#[cfg(feature = "semantic")]
fn tiny_s0(dir: &Path) {
    let words = [
        "pool",
        "connection",
        "saturated",
        "canary",
        "latency",
        "auth",
        "traces",
        "regression",
        "wait",
        "rollout",
        "configuration",
        "cause",
        "the",
        "eu",
        "west",
    ];
    let mut vocab = serde_json::Map::new();
    vocab.insert("[UNK]".into(), 0.into());
    for (i, w) in words.iter().enumerate() {
        vocab.insert((*w).into(), (i + 1).into());
    }
    let tokenizer = serde_json::json!({
        "version": "1.0", "truncation": null, "padding": null,
        "added_tokens": [{"id": 0, "content": "[UNK]", "single_word": false, "lstrip": false,
                          "rstrip": false, "normalized": false, "special": true}],
        "normalizer": {"type": "Lowercase"},
        "pre_tokenizer": {"type": "Whitespace"},
        "post_processor": null, "decoder": null,
        "model": {"type": "WordLevel", "vocab": vocab, "unk_token": "[UNK]"}
    });
    let (rows, dim) = (words.len() + 1, 8usize);
    let mut data = Vec::new();
    for r in 0..rows {
        for d in 0..dim {
            let v: f32 = if (r >> (d % 4)) & 1 == 1 {
                1.0
            } else {
                0.1 * d as f32
            };
            data.extend_from_slice(&v.to_le_bytes());
        }
    }
    // safetensors by hand: an 8-byte little-endian header length, the JSON header, the bytes.
    let header = serde_json::json!({
        "embeddings": {"dtype": "F32", "shape": [rows, dim], "data_offsets": [0, data.len()]}
    })
    .to_string();
    let mut st = (header.len() as u64).to_le_bytes().to_vec();
    st.extend_from_slice(header.as_bytes());
    st.extend_from_slice(&data);
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("model.safetensors"), st).unwrap();
    std::fs::write(dir.join("tokenizer.json"), tokenizer.to_string()).unwrap();
    std::fs::write(dir.join("config.json"), r#"{"normalize": true}"#).unwrap();
}

/// `q` with a model: a semantic plan runs, `--explain` labels it model-dependent and names
/// the model, `--seed-check` refuses it with `SMY-E416` before it runs, and a lexical plan with
/// the same model bound is still pure and still passes.
#[cfg(feature = "semantic")]
#[test]
fn a_semantic_plan_runs_and_seed_check_refuses_it_with_e416() {
    let dir = scratch("s0");
    let model = dir.join("model");
    tiny_s0(&model);
    let model = model.to_string_lossy().to_string();
    let store = f1();

    let o = run(&[
        "q",
        "find \"connection pool\" ~sem",
        &store,
        "--engine-model",
        &model,
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(!stdout(&o).trim().is_empty(), "a ranking");

    let o = run(&[
        "q",
        "find \"connection pool\" ~sem",
        &store,
        "--engine-model",
        &model,
        "--explain",
    ]);
    assert!(
        stdout(&o).contains("purity: model-dependent (~sem)"),
        "{}",
        stdout(&o)
    );
    assert!(stdout(&o).contains(", model "), "{}", stdout(&o));

    let o = run(&[
        "--seed-check",
        "q",
        "find \"connection pool\" ~sem",
        &store,
        "--engine-model",
        &model,
    ]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("SMY-E416"), "{}", stderr(&o));
    assert!(stdout(&o).is_empty(), "refused before anything ran");

    let o = run(&[
        "--seed-check",
        "q",
        "find \"connection pool\"",
        &store,
        "--engine-model",
        &model,
    ]);
    assert!(o.status.success(), "{}", stderr(&o));

    let o = run(&[
        "q",
        "find \"x\" ~sem",
        &store,
        "--engine-model",
        &model,
        "--candle-model",
        &model,
    ]);
    assert_eq!(o.status.code(), Some(2), "one model at a time");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Without a model a semantic stage is refused by name, and in a build without the feature the
/// flag is too — rather than accepted and ignored.
#[test]
fn a_semantic_stage_without_a_model_is_refused_by_name() {
    let o = run(&["q", "find \"pool\" ~sem", &f1()]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("SMY-E461"), "{}", stderr(&o));
    #[cfg(not(feature = "semantic"))]
    {
        let o = run(&[
            "q",
            "find \"pool\" ~sem",
            &f1(),
            "--engine-model",
            "nowhere",
        ]);
        assert_eq!(o.status.code(), Some(2));
        assert!(
            stderr(&o).contains("needs a build with --features semantic"),
            "{}",
            stderr(&o)
        );
    }
    #[cfg(not(feature = "semantic-candle"))]
    {
        let o = run(&[
            "q",
            "find \"pool\" ~sem",
            &f1(),
            "--candle-model",
            "nowhere",
        ]);
        assert_eq!(o.status.code(), Some(2));
        assert!(
            stderr(&o).contains("needs a build with --features semantic-candle"),
            "{}",
            stderr(&o)
        );
    }
}

/// `--from-question` without a model to ask: a configuration that cannot be read refuses before
/// any provider is built, `--seed-check` refuses the invocation (a translation is not
/// reproducible), and `--yes` alone is a usage error, since there is nothing for it to confirm.
/// None of these reaches a model — `--offline` would not do, since it leaves a local one.
#[test]
fn from_question_is_refused_where_it_cannot_ask() {
    let dir = scratch("no-config");
    let missing = dir.join("absent.toml").to_string_lossy().to_string();
    let o = run(&[
        "--config",
        &missing,
        "q",
        "--from-question",
        "Which claims are there?",
        &f1(),
    ]);
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert!(stdout(&o).is_empty(), "nothing ran: {}", stdout(&o));
    let _ = std::fs::remove_dir_all(&dir);

    let o = run(&[
        "--seed-check",
        "q",
        "--from-question",
        "Which claims are there?",
        &f1(),
    ]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("--from-question"), "{}", stderr(&o));

    let o = run(&[
        "q",
        "from store:F1-incident\nwhere kind = claim",
        &f1(),
        "--yes",
    ]);
    assert_eq!(o.status.code(), Some(2));
}
