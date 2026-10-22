//! `smysl lib` (SMYSL-2.7 §4.4, TX-P11 step 5).
//!
//! The step's exit: `lib export … && lib verify` exits 0 on F1–F13, here with F10 and F14 from
//! the wire as well, under both layouts and both closure modes. The rest holds the exit codes
//! §4.4 gives each action, and the `--format` refusals that came with widening the global flag
//! to `pack`.
#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-cmd-lib-{name}-{}", std::process::id()));
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

fn code(o: &Output) -> i32 {
    o.status.code().expect("an exit code")
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

/// F1–F13, F10 from the wire, and F14's unknown codes: the corpus `smysl-lib`'s own tests use.
fn corpus() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(repo("fixtures/corpus"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "smy"))
        .collect();
    out.push(repo("fixtures/wire/F10-lifecycle.cbor"));
    out.push(repo("fixtures/wire/F14-unknown-codes.cbor"));
    out.sort();
    out
}

fn s(p: &Path) -> &str {
    p.to_str().expect("a UTF-8 path")
}

/// Export `store` into `dir`, and say how it went.
fn export(store: &Path, dir: &Path, extra: &[&str]) -> Output {
    let mut args = vec![
        "lib",
        "export",
        "-s",
        s(store),
        "--scope",
        "all",
        "-o",
        s(dir),
    ];
    args.extend_from_slice(extra);
    run(&args)
}

#[test]
fn export_then_verify_exits_0_on_the_corpus() {
    let root = scratch("corpus");
    let mut failures = Vec::new();
    for fixture in corpus() {
        let stem = fixture.file_stem().unwrap().to_string_lossy().to_string();
        for layout in ["single", "by-view"] {
            for closure in ["full", "ref"] {
                let dir = root.join(format!("{stem}-{layout}-{closure}"));
                let e = export(&fixture, &dir, &["--layout", layout, "--closure", closure]);
                if code(&e) != 0 {
                    failures.push(format!(
                        "{stem} {layout} {closure}: export {}: {}",
                        code(&e),
                        stderr(&e)
                    ));
                    continue;
                }
                let v = run(&["lib", "verify", s(&dir)]);
                if code(&v) != 0 {
                    failures.push(format!(
                        "{stem} {layout} {closure}: verify {}: {}",
                        code(&v),
                        stdout(&v)
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_flipped_byte_exits_9() {
    let root = scratch("flip");
    let dir = root.join("out");
    assert_eq!(
        code(&export(&repo("fixtures/corpus/F13-canon.smy"), &dir, &[])),
        0
    );
    let file = dir.join("all.cbor");
    let mut bytes = std::fs::read(&file).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    std::fs::write(&file, bytes).unwrap();
    let v = run(&["lib", "verify", s(&dir.join("export-index.cbor"))]);
    assert_eq!(code(&v), 9, "{}", stdout(&v));
    assert!(stderr(&v).contains("SMY-E494"), "{}", stderr(&v));
}

#[test]
fn export_refuses_an_occupied_directory_without_force() {
    let root = scratch("occupied");
    let dir = root.join("out");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.txt"), "mine").unwrap();
    let store = repo("fixtures/corpus/F13-canon.smy");
    let e = export(&store, &dir, &[]);
    assert_eq!(code(&e), 2, "{}", stderr(&e));
    assert!(!dir.join("export-index.cbor").exists());
    assert_eq!(code(&export(&store, &dir, &["--force"])), 0);
    assert_eq!(
        std::fs::read_to_string(dir.join("notes.txt")).unwrap(),
        "mine"
    );
    assert_eq!(code(&run(&["lib", "verify", s(&dir)])), 0);
}

#[test]
fn export_needs_a_scope_and_a_directory() {
    let root = scratch("needs");
    let store = repo("fixtures/corpus/F13-canon.smy");
    let o = run(&["lib", "export", "-s", s(&store), "-o", s(&root.join("a"))]);
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("--scope"), "{}", stderr(&o));
    let o = run(&["lib", "export", "-s", s(&store), "--scope", "all"]);
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("-o DIR"), "{}", stderr(&o));
    let o = run(&[
        "lib",
        "export",
        "-s",
        s(&store),
        "--scope",
        "b3:aaaaaaaaaaaaaaaaaaaaaaaaaa",
        "-o",
        s(&root.join("b")),
    ]);
    assert_ne!(code(&o), 0);
    assert!(!root.join("b").exists());
}

#[test]
fn import_stages_by_default_and_accepts_on_request() {
    let root = scratch("import");
    let dir = root.join("out");
    assert_eq!(
        code(&export(&repo("fixtures/corpus/F13-canon.smy"), &dir, &[])),
        0
    );

    let staged = root.join("staged");
    let o = run(&["lib", "import", "-s", s(&staged), s(&dir)]);
    assert_eq!(code(&o), 10, "{}{}", stdout(&o), stderr(&o));
    let st = run(&["--json", "lib", "status", "-s", s(&staged)]);
    assert_eq!(code(&st), 0);
    assert!(
        stdout(&st).contains("\"state\":\"staged\""),
        "{}",
        stdout(&st)
    );

    // The same collection again changes nothing, and says so.
    let again = run(&["lib", "import", "-s", s(&staged), s(&dir)]);
    assert_eq!(code(&again), 0, "{}", stderr(&again));
    assert!(
        stdout(&again).contains("already imported"),
        "{}",
        stdout(&again)
    );

    let accepted = root.join("accepted");
    let o = run(&["lib", "import", "--accept", "-s", s(&accepted), s(&dir)]);
    assert_eq!(code(&o), 0, "{}{}", stdout(&o), stderr(&o));
    let st = run(&[
        "--json",
        "lib",
        "status",
        "-s",
        s(&accepted),
        "--import",
        "out",
    ]);
    assert!(
        stdout(&st).contains("\"state\":\"accepted\""),
        "{}",
        stdout(&st)
    );
}

#[test]
fn accept_and_discard_a_staged_import() {
    let root = scratch("accept");
    let dir = root.join("out");
    assert_eq!(
        code(&export(&repo("fixtures/corpus/F13-canon.smy"), &dir, &[])),
        0
    );
    let lib = root.join("lib");
    assert_eq!(
        code(&run(&[
            "lib",
            "import",
            "-s",
            s(&lib),
            "--name",
            "first",
            s(&dir)
        ])),
        10
    );
    let a = run(&["lib", "accept", "-s", s(&lib), "first"]);
    assert_eq!(code(&a), 0, "{}", stderr(&a));
    // Accepted is not staged: a second accept, or a discard, has nothing to act on.
    assert_eq!(code(&run(&["lib", "accept", "-s", s(&lib), "first"])), 2);
    assert_eq!(code(&run(&["lib", "discard", "-s", s(&lib), "first"])), 2);
    assert_eq!(
        code(&run(&["lib", "accept", "-s", s(&lib), "no-such-import"])),
        2
    );

    let other = root.join("other");
    assert_eq!(
        code(&run(&[
            "lib",
            "import",
            "-s",
            s(&other),
            "--name",
            "second",
            s(&dir)
        ])),
        10
    );
    let d = run(&["lib", "discard", "-s", s(&other), "second"]);
    assert_eq!(code(&d), 0, "{}", stderr(&d));
    let st = run(&["--json", "lib", "status", "-s", s(&other)]);
    assert!(
        stdout(&st).contains("\"state\":\"discarded\""),
        "{}",
        stdout(&st)
    );
}

#[test]
fn an_input_that_does_not_decode_exits_1() {
    let root = scratch("truncated");
    let dir = root.join("out");
    assert_eq!(
        code(&export(&repo("fixtures/corpus/F13-canon.smy"), &dir, &[])),
        0
    );
    let bytes = std::fs::read(dir.join("all.cbor")).unwrap();
    let cut = root.join("cut.cbor");
    std::fs::write(&cut, &bytes[..bytes.len() - 3]).unwrap();
    let o = run(&["lib", "import", "-s", s(&root.join("lib")), s(&cut)]);
    assert_eq!(code(&o), 1, "{}{}", stdout(&o), stderr(&o));
    assert!(stderr(&o).contains("SMY-E493"), "{}", stderr(&o));
}

#[test]
fn format_is_refused_where_it_is_not_written() {
    let root = scratch("format");
    let store = repo("fixtures/corpus/F13-canon.smy");
    // `pack` is `lib export`'s alone.
    let o = run(&["fmt", "--format", "pack", s(&store)]);
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("only `lib export`"), "{}", stderr(&o));
    // `lib export` writes all three; `pack` needs a budget, and a budget needs `pack`.
    for (form, extra) in [
        ("cbor", &[][..]),
        ("surface", &[][..]),
        ("pack", &["--budget", "4000"][..]),
    ] {
        let dir = root.join(form);
        let mut args = vec!["--format", form];
        args.extend_from_slice(extra);
        let o = export(&store, &dir, &args);
        assert_eq!(code(&o), 0, "{form}: {}", stderr(&o));
        assert_eq!(code(&run(&["lib", "verify", s(&dir)])), 0, "{form}");
    }
    assert!(root.join("surface").join("all.smy").is_file());
    assert!(root.join("pack").join("all.pack.cbor").is_file());
    assert_eq!(
        code(&export(&store, &root.join("p1"), &["--format", "pack"])),
        2
    );
    assert_eq!(
        code(&export(&store, &root.join("p2"), &["--budget", "10"])),
        2
    );
    // The other actions write no document at all.
    let o = run(&[
        "lib",
        "status",
        "--format",
        "cbor",
        "-s",
        s(&root.join("lib")),
    ]);
    assert_eq!(code(&o), 2);
    assert!(
        stderr(&o).contains("does not write a document"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn status_needs_a_library() {
    let root = scratch("status");
    let o = run(&["lib", "status", "-s", s(&root)]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(
        !root.join("LIBRARY").exists(),
        "status must not create a library"
    );
}

#[test]
fn the_library_flags_are_refused_on_a_plain_store() {
    let root = scratch("plain");
    let store = repo("fixtures/corpus/F13-canon.smy");
    for flag in [
        "--with-text",
        "--with-provenance",
        "--with-staged",
        "--with-quarantine",
    ] {
        let dir = root.join(flag.trim_start_matches('-'));
        let o = export(&store, &dir, &[flag]);
        assert_eq!(code(&o), 2, "{flag}: {}", stderr(&o));
        assert!(!dir.exists(), "{flag}");
    }
}

#[test]
fn by_expression_and_by_collection_round_trip_on_the_corpus() {
    let root = scratch("by-alias");
    for fixture in corpus() {
        let stem = fixture.file_stem().unwrap().to_string_lossy().to_string();
        for layout in ["by-expression", "by-collection"] {
            let dir = root.join(format!("{stem}-{layout}"));
            let e = export(&fixture, &dir, &["--layout", layout]);
            assert_eq!(code(&e), 0, "{stem} {layout}: {}", stderr(&e));
            let v = run(&["lib", "verify", s(&dir)]);
            assert_eq!(code(&v), 0, "{stem} {layout}: {}", stdout(&v));
        }
    }
}

#[test]
fn by_time_and_as_of_on_the_command_line() {
    let root = scratch("time");
    let store = repo("fixtures/corpus/F13-canon.smy");
    let dir = root.join("by-time");
    let e = export(&store, &dir, &["--layout", "by-time", "--window", "6h"]);
    assert_eq!(code(&e), 0, "{}", stderr(&e));
    assert_eq!(code(&run(&["lib", "verify", s(&dir)])), 0);
    // A window belongs to `by-time`, and is a number and a unit.
    assert_eq!(
        code(&export(&store, &root.join("w1"), &["--window", "6h"])),
        2
    );
    let o = export(
        &store,
        &root.join("w2"),
        &["--layout", "by-time", "--window", "6x"],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    // An `--as-of` long after every clock in the fixture keeps what has an attestation.
    let dir = root.join("as-of");
    let e = export(&store, &dir, &["--as-of", "2999"]);
    assert_eq!(code(&e), 0, "{}", stderr(&e));
    assert_eq!(code(&run(&["lib", "verify", s(&dir)])), 0);
    let o = export(&store, &root.join("bad"), &["--as-of", "yesterday"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
}

#[test]
fn an_sq_scope_and_by_query_on_the_command_line() {
    let root = scratch("sq");
    let store = repo("fixtures/corpus/F13-canon.smy");
    let dir = root.join("claims");
    let o = run(&[
        "lib",
        "export",
        "-s",
        s(&store),
        "--scope",
        "from store:F13-canon\nwhere kind = claim",
        "-o",
        s(&dir),
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(code(&run(&["lib", "verify", s(&dir)])), 0);
    let o = run(&[
        "lib",
        "export",
        "-s",
        s(&store),
        "--scope",
        "from nowhere at all",
        "-o",
        s(&root.join("bad")),
    ]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));

    let dir = root.join("by-kind");
    let e = export(
        &store,
        &dir,
        &["--layout", "by-query", "--group-by", "kind"],
    );
    assert_eq!(code(&e), 0, "{}", stderr(&e));
    assert_eq!(code(&run(&["lib", "verify", s(&dir)])), 0);
    assert_eq!(
        code(&export(&store, &root.join("g1"), &["--layout", "by-query"])),
        2
    );
    assert_eq!(
        code(&export(&store, &root.join("g2"), &["--group-by", "kind"])),
        2
    );
}

#[test]
fn from_import_gives_the_collection_back_byte_for_byte() {
    let root = scratch("from-import");
    let first = root.join("first");
    assert_eq!(
        code(&export(
            &repo("fixtures/corpus/F13-canon.smy"),
            &first,
            &["--layout", "by-view"]
        )),
        0
    );
    let lib = root.join("lib");
    let o = run(&[
        "lib",
        "import",
        "--accept",
        "--name",
        "coll",
        "-s",
        s(&lib),
        s(&first),
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let again = root.join("again");
    let o = run(&[
        "lib",
        "export",
        "-s",
        s(&lib),
        "--scope",
        "from import:coll",
        "--layout",
        "by-view",
        "-o",
        s(&again),
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    for e in std::fs::read_dir(&first).unwrap() {
        let name = e.unwrap().file_name();
        if name == "export-index.cbor" {
            continue;
        }
        assert_eq!(
            std::fs::read(first.join(&name)).unwrap(),
            std::fs::read(again.join(&name)).unwrap(),
            "{name:?}"
        );
    }
}

#[test]
fn verify_p2_checks_files_through_a_scratch_library() {
    let o = run(&[
        "lib",
        "verify",
        "--p2",
        s(&repo("fixtures/corpus/F13-canon.smy")),
        "--p2",
        s(&repo("fixtures/wire/F12-reserved-pack.cbor")),
    ]);
    assert_eq!(code(&o), 0, "{}{}", stdout(&o), stderr(&o));
    assert_eq!(stdout(&o).matches("P2 holds").count(), 2, "{}", stdout(&o));
    // An index and --p2 are two questions.
    let root = scratch("p2");
    let dir = root.join("out");
    assert_eq!(
        code(&export(&repo("fixtures/corpus/F13-canon.smy"), &dir, &[])),
        0
    );
    let o = run(&[
        "lib",
        "verify",
        s(&dir),
        "--p2",
        s(&repo("fixtures/corpus/F13-canon.smy")),
    ]);
    assert_eq!(code(&o), 2);
}
