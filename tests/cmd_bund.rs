//! `smysl bund` (RFC SMYSL-4 §10, BQ-P4): a Bund2 script over a store or a library, end to end
//! against the binary.
//!
//! Every test that runs a script needs `BUND2_BIN`, a `bund2` built with `--features smysl`,
//! and passes trivially without it. `SMYSL_BUND_REQUIRED` turns its absence into a failure, as
//! in the `bund2-bridge` CI job. The refusals that come before any process starts run always.

#![cfg(all(feature = "cli", feature = "bund"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

fn f1() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/corpus/F1-incident.smy")
        .display()
        .to_string()
}

fn bund2() -> Option<PathBuf> {
    let b = std::env::var_os("BUND2_BIN").map(PathBuf::from);
    assert!(
        b.is_some() || std::env::var_os("SMYSL_BUND_REQUIRED").is_none(),
        "SMYSL_BUND_REQUIRED is set and BUND2_BIN is not"
    );
    b
}

fn smysl(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("the binary runs")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// `smysl bund` with `bund2` given and the sandbox where there is one.
fn bund(bin: &Path, args: &[&str]) -> Output {
    let mut all = vec!["bund", "--bund2", bin.to_str().unwrap()];
    if smysl_sandbox_missing() {
        all.push("--trusted");
    }
    all.extend_from_slice(args);
    smysl(&all)
}

fn smysl_sandbox_missing() -> bool {
    smysl::bund::sandbox::available().is_err()
}

#[test]
fn a_script_is_given_once() {
    let o = smysl(&["bund", "-s", &f1()]);
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert!(stderr(&o).contains("no script given"), "{}", stderr(&o));
    let o = smysl(&["bund", "-s", &f1(), "--eval", "1", "--script", "x.bund"]);
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
}

/// The gate runs before `bund2` is looked for, so a refused word is refused with no `bund2`.
#[test]
fn a_refused_word_is_named() {
    let o = smysl(&[
        "bund",
        "--bund2",
        "/no/such/bund2",
        "-s",
        &f1(),
        "--eval",
        "\"ls\" sh",
    ]);
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert!(stderr(&o).contains("refused `sh`"), "{}", stderr(&o));
}

#[test]
fn probe_names_the_binary_and_the_sandbox() {
    let Some(bin) = bund2() else { return };
    let o = smysl(&[
        "bund",
        "--probe",
        "--bund2",
        bin.to_str().unwrap(),
        "--json",
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    let out: serde_json::Value = serde_json::from_str(&stdout(&o)).expect("--json is JSON");
    assert_eq!(out["smysl"], true, "{out}");
    assert!(out["sandbox"].is_string(), "{out}");
}

#[test]
fn a_query_answers_as_json_and_as_text() {
    let Some(bin) = bund2() else { return };
    let script = ":lib var? \"from library\" smysl.kb [ [ unit U claim S ] [ gist U G ] ] kb.query";
    let o = bund(&bin, &["-s", &f1(), "--eval", script, "--json"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let json = stdout(&o);
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("--json is JSON");
    let rows = parsed.as_array().expect("a list of bindings");
    assert_eq!(rows.len(), 3, "{json}");
    assert!(rows
        .iter()
        .all(|r| r["U"].as_str().is_some_and(|u| u.starts_with("b3:"))));

    let o = bund(&bin, &["-s", &f1(), "--eval", script]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("S: \"inferred\""), "{}", stdout(&o));

    // Rule P: the same invocation prints the same bytes.
    let again = bund(&bin, &["-s", &f1(), "--eval", script, "--json"]);
    assert_eq!(stdout(&again), json);
}

#[test]
fn sq_seeds_the_script() {
    let Some(bin) = bund2() else { return };
    let o = bund(
        &bin,
        &[
            "-s",
            &f1(),
            "--sq",
            "from library | where kind = claim",
            "--eval",
            ":seed var? len",
        ],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "3");
}

#[test]
fn a_script_file_over_a_library() {
    let Some(bin) = bund2() else { return };
    let dir = std::env::temp_dir().join(format!("smysl-cmd-bund-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let root = dir.join("lib");
    let mut lib = smysl::text::library::Library::create(&root).unwrap();
    let src = std::fs::read_to_string(f1()).unwrap();
    lib.append_records(&smysl::parse_surface(&src).unwrap().records)
        .unwrap();
    drop(lib);
    let script = dir.join("q.bund");
    std::fs::write(
        &script,
        ":lib var? \"from library | where kind = claim\" smysl.query :units get len\n",
    )
    .unwrap();
    let o = bund(
        &bin,
        &[
            "-s",
            root.to_str().unwrap(),
            "--script",
            script.to_str().unwrap(),
        ],
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "3");
}

#[test]
fn a_runaway_is_stopped() {
    let Some(bin) = bund2() else { return };
    let o = bund(
        &bin,
        &[
            "-s",
            &f1(),
            "--eval",
            "true { true } while",
            "--timeout-ms",
            "300",
        ],
    );
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("stopped after 300 ms"),
        "{}",
        stderr(&o)
    );
    let spin = ":s [ [ [ spin ] ← [ spin ] ] ] kb! var :s var? [ spin ] kb.holds";
    let o = bund(
        &bin,
        &["-s", &f1(), "--eval", spin, "--max-inferences", "500"],
    );
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("inference cap of 500"),
        "{}",
        stderr(&o)
    );
}
