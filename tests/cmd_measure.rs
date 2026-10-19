//! `smysl measure` (SMYSL-2.6 §4.4, TX-P8 M7).
//!
//! The step's exit names a run of `measure T1,T2,T3,T4` over the five-Bible fixture that adds
//! no unit the second time, and that is asserted here against the store itself rather than
//! against the report: the library is opened after each run and its units counted. The fixture
//! is `fixtures/library/bibles/slice/`, Ruth and Jude in all five editions, added with
//! `text add` as `bible/<edition>/<book>` so that `from collection:bible` is all ten books.
//!
//! Five languages are five analyzer chains, which a scope is refused for unless the recipe
//! names one (`SMY-E489`); the runs name the plain chain, as the M2 to M6 library tests do.
#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use smysl::text::library::Library;
use smysl::Record;

const BIN: &str = env!("CARGO_BIN_EXE_smysl");
const PLAIN: &str = "smysl/an/1:surface";
const SCOPE: &str = "from collection:bible";

const EDITIONS: [(&str, &str, &str); 5] = [
    ("deu1912", "deu1912", "de"),
    ("eng-kjv", "eng-kjv", "en"),
    ("fraLSG", "fralsg", "fr"),
    ("russyn", "russyn", "ru"),
    ("spaRV1909", "sparv1909", "es"),
];

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-cmd-measure-{name}-{}", std::process::id()));
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

/// The five-Bible fixture as a library: ten books under `bible/`.
fn bibles(name: &str) -> (PathBuf, String) {
    let dir = scratch(name).join("lib");
    let root = dir.to_string_lossy().to_string();
    for (dir_name, alias, lang) in EDITIONS {
        for book in ["08-RUT", "65-JUD"] {
            let file = repo(&format!(
                "fixtures/library/bibles/slice/{dir_name}/{book}.usfm"
            ));
            let alias = format!("bible/{alias}/{}", book.to_lowercase());
            let o = run(&[
                "text",
                "add",
                &file.to_string_lossy(),
                "--reader",
                "usfm/1",
                "--alias",
                &alias,
                "--lang",
                lang,
                "--licence",
                "public-domain",
                "--carry",
                "text",
                "-s",
                &root,
            ]);
            assert!(o.status.success(), "text add {alias}: {}", stderr(&o));
        }
    }
    (dir, root)
}

/// Units and attestations in the library's store.
fn census(root: &Path) -> (usize, usize) {
    let library = Library::open(root).expect("a library");
    let mut units = 0;
    let mut attestations = 0;
    for r in library.store().iter() {
        match r {
            Record::Unit(_) => units += 1,
            Record::Attestation(_) => attestations += 1,
            _ => {}
        }
    }
    (units, attestations)
}

fn measure(root: &str, extra: &[&str]) -> Output {
    let mut args = vec![
        "measure",
        "T1,T2,T3,T4",
        "--scope",
        SCOPE,
        "--analyzer",
        PLAIN,
        // T3's default asks 50,000 characters of every text and a book of Ruth has 13k; set
        // for T3 alone, which leaves T1's, T2's and T4's recipes at their defaults.
        "--min-n",
        "T3=2",
    ];
    args.extend_from_slice(extra);
    args.push(root);
    run(&args)
}

/// The exit: four measures over the five Bibles, written once, and a second run that adds no
/// unit — the store holds the same units after it, and gains only the second run's
/// attestations, one per result.
#[test]
fn a_second_run_adds_no_unit() {
    let (dir, root) = bibles("twice");
    let before = census(&dir);

    let first = measure(&root, &["--at", "1000"]);
    assert!(first.status.success(), "{}", stderr(&first));
    assert!(
        stderr(&first).contains("4 unit(s): 4 new, 0 already in the store"),
        "{}",
        stderr(&first)
    );
    let after_first = census(&dir);
    // Four results and the saved scope they are grounded on.
    assert_eq!(after_first.0, before.0 + 5, "{}", stdout(&first));
    assert_eq!(after_first.1, before.1 + 4);

    let second = measure(&root, &["--at", "2000"]);
    assert!(second.status.success(), "{}", stderr(&second));
    assert!(
        stderr(&second).contains("4 unit(s): 0 new, 4 already in the store"),
        "{}",
        stderr(&second)
    );
    let after_second = census(&dir);
    assert_eq!(after_second.0, after_first.0, "a second run added a unit");
    assert_eq!(after_second.1, after_first.1 + 4, "the run's attestations");
    for id in ["T1", "T2", "T3", "T4"] {
        assert!(
            stdout(&second).contains(&format!("{id} v1  b3:")),
            "{id}: {}",
            stdout(&second)
        );
    }
}

/// `--dry-run` prints the values and writes nothing; `--seed-check` computes each result twice
/// and passes for every pure id this build has; `--json` is one object a parser reads.
#[test]
fn dry_run_seed_check_and_json() {
    let (dir, root) = bibles("dry");
    let before = census(&dir);
    let o = run(&[
        "--seed-check",
        "--json",
        "measure",
        "T1,T2,T3,T4,T5,T6,T7",
        "--scope",
        SCOPE,
        "--analyzer",
        PLAIN,
        "--min-n",
        "2",
        "--dry-run",
        &root,
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(census(&dir), before, "--dry-run wrote");
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).expect("one JSON object");
    assert_eq!(v["dry_run"], true);
    assert_eq!(v["new"], 7);
    let results = v["results"].as_array().unwrap();
    let ids: Vec<&str> = results.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["T1", "T2", "T3", "T4", "T5", "T6", "T7"]);
    for r in results {
        assert_eq!(r["outcome"], "value", "{r}");
        assert!(r["uid"].as_str().unwrap().starts_with("b3:"));
    }
    // T7 over five languages read as one vocabulary: Latin, Cyrillic, and the three Latin-script
    // editions' shared spellings.
    let t7 = &results[6]["values"];
    assert_eq!(t7["jsd:bible/eng-kjv/08-rut|bible/russyn/08-rut"], "1.0000");
}

/// A parameter goes to the ids that take it, or to the one it names; one that no id asked for
/// takes is refused before anything runs, as is a scoped one for an id not asked for. An id
/// the build does not have is a result, not run, beside the ones that ran.
#[test]
fn parameters_are_routed_per_id() {
    let (_dir, root) = bibles("params");
    let o = run(&[
        "--json",
        "measure",
        "T5,T7,T9",
        "--scope",
        SCOPE,
        "--analyzer",
        PLAIN,
        "--level",
        "verse",
        "--min-n",
        "2",
        "--param",
        "top:k=1",
        "--param",
        "T5.order=2",
        "--dry-run",
        &root,
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    let r = v["results"].as_array().unwrap();
    let tops = |x: &serde_json::Value, p: &str| {
        x["values"]
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| k.starts_with(p))
            .count()
    };
    assert_eq!(tops(&r[0], "top:"), 1, "top:k went to T5");
    assert_eq!(tops(&r[1], "g2:"), 1, "and to T7");
    assert_eq!(r[2]["outcome"], "not-run");
    assert_eq!(r[2]["code"], "SMY-E489");

    // T5's recipe says order 2, and T7's does not mention an order at all.
    let order_2 = r[0]["recipe"].as_str().unwrap().to_string();
    let o = run(&[
        "--json",
        "measure",
        "T5",
        "--scope",
        SCOPE,
        "--analyzer",
        PLAIN,
        "--level",
        "verse",
        "--min-n",
        "2",
        "--param",
        "top:k=1",
        "--dry-run",
        &root,
    ]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_ne!(v["results"][0]["recipe"].as_str().unwrap(), order_2);

    for (args, why) in [
        (
            vec!["--param", "symbols=terms"],
            "none of the ids asked for takes `symbols`",
        ),
        (
            vec!["--param", "T1.order=2"],
            "T1 is not one of the ids asked for",
        ),
        (vec!["--param", "T5.symbols=terms"], "T5 takes no `symbols`"),
        (
            vec!["--min-n", "T3=2"],
            "T3 is not one of the ids asked for",
        ),
        (vec!["--level", "stanza"], "--level is part, chapter"),
    ] {
        let mut a = vec!["measure", "T5,T7", "--scope", SCOPE, "--dry-run"];
        a.extend(args);
        a.push(&root);
        let o = run(&a);
        assert_eq!(o.status.code(), Some(2), "{a:?}");
        assert!(stderr(&o).contains(why), "{a:?}: {}", stderr(&o));
    }
}

/// A scope whose texts choose five analyzer chains is refused unless one is named, and a path
/// that is not a library is refused before anything is read.
#[test]
fn refusals_before_anything_runs() {
    let (_dir, root) = bibles("refuse");
    let o = run(&["measure", "T1", "--scope", SCOPE, "--dry-run", &root]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("SMY-E489"), "{}", stderr(&o));

    let plain = scratch("plain").join("store.smy");
    std::fs::write(&plain, "").unwrap();
    let o = run(&["measure", "T1", "--scope", SCOPE, &plain.to_string_lossy()]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("not a library"), "{}", stderr(&o));
}
