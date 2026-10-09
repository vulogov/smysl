//! `text redact`: rule Z through the binary (SMYSL-2.4 §5.2, TX-P2 step 4).
//!
//! Redact a part; the objects are gone; the manifests, the units and their spans remain. That
//! last clause is the one worth a test rather than a comment: a redaction is **not** a
//! retraction, and a corpus that quietly dropped the claims made from a text would be
//! answering a legal demand by falsifying its own history.
//!
//! The record goes into the catalog before the bytes are unlinked, so the crash window leaves a
//! catalog that has said what it is doing and an object store that has not caught up — and
//! opening the library is where that is put right. That half is asserted by deleting nothing and
//! letting `Library::open` find an object a redaction already names.

#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "smysl-redact-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("the binary under test must run")
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn field(out: &str, key: &str) -> String {
    out.lines()
        .find_map(|l| l.trim().strip_prefix(key))
        .unwrap_or_else(|| panic!("no `{key}` line in:\n{out}"))
        .trim()
        .to_string()
}

/// A library holding one text, and the tid of its only part.
fn library(name: &str) -> (PathBuf, String) {
    let dir = scratch(name);
    let root = dir.to_string_lossy().into_owned();
    let out = run(&[
        "text",
        "add",
        "fixtures/library/readers/gen1.usfm",
        "--reader",
        "usfm/1",
        "--alias",
        "kjv",
        "--lang",
        "en",
        "--licence",
        "public-domain",
        "--carry",
        "text",
        "-s",
        &root,
    ]);
    assert!(out.status.success(), "{}", text(&out));
    let tid = field(&text(&out), "part 0");
    (dir, tid)
}

/// Every object file a library holds, excluding the staging directory.
fn objects(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.join("objects")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == "tmp") {
                    continue;
                }
                stack.push(p);
            } else {
                out.push(p.to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    out
}

/// The whole of §5.2's redaction case: the bytes go, and nothing else does.
#[test]
fn redacting_a_part_removes_its_objects_and_nothing_else() {
    let (dir, tid) = library("basic");
    let root = dir.to_string_lossy().into_owned();
    assert_eq!(objects(&dir).len(), 2, "a part and a reading");

    let log_before = std::fs::read(dir.join("log/catalog.cbor")).expect("the log");
    let out = run(&[
        "text",
        "redact",
        &tid,
        "--as",
        "human:vu",
        "--at",
        "1726500000000",
        "-s",
        &root,
    ]);
    assert!(out.status.success(), "{}", text(&out));
    let shown = text(&out);
    assert_eq!(field(&shown, "redacted"), tid);
    assert_eq!(field(&shown, "objects"), "2 unlinked");
    assert_eq!(field(&shown, "manifests"), "1 still name this part");

    assert!(objects(&dir).is_empty(), "the bytes are gone");
    // The log **grew**: a redaction is appended, never a rewrite. That is the whole of OQ-39's
    // answer, and the one property an append-only log cannot be given back once it is lost.
    let log_after = std::fs::read(dir.join("log/catalog.cbor")).expect("the log");
    assert!(log_after.len() > log_before.len());
    assert!(
        log_after.starts_with(&log_before),
        "the log was rewritten rather than appended to"
    );

    // The catalog still lists the expression, and `check` is still clean.
    let ls = run(&["text", "ls", "-s", &root]);
    assert!(ls.status.success(), "{}", text(&ls));
    assert!(text(&ls).contains("kjv"), "{}", text(&ls));
    let check = run(&["check", "-s", &root]);
    assert!(check.status.success(), "{}", text(&check));

    // And asking for the passage says why there is none, rather than naming a missing file.
    let show = run(&["text", "show", "kjv#Gen.1.1", "-s", &root]);
    assert!(!show.status.success());
    let refused = text(&show);
    assert!(refused.contains("redacted"), "{refused}");
    assert!(!refused.contains("No such file"), "{refused}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Re-adding the source file does not bring the bytes back, and says so.
#[test]
fn adding_the_file_again_withholds_the_objects() {
    let (dir, tid) = library("readd");
    let root = dir.to_string_lossy().into_owned();
    assert!(run(&[
        "text",
        "redact",
        &tid,
        "--as",
        "human:vu",
        "--at",
        "1726500000000",
        "-s",
        &root
    ])
    .status
    .success());

    let out = run(&[
        "text",
        "add",
        "fixtures/library/readers/gen1.usfm",
        "--reader",
        "usfm/1",
        "--alias",
        "kjv-again",
        "--lang",
        "en",
        "--licence",
        "public-domain",
        "--carry",
        "text",
        "-s",
        &root,
    ]);
    // Not refused: the spec's rule is drop-and-count, because a refusal would make one
    // redaction anywhere a permanent failure for everybody who still holds the file.
    assert!(out.status.success(), "{}", text(&out));
    let shown = text(&out);
    assert_eq!(field(&shown, "objects"), "0 written");
    assert!(shown.contains("2 object(s) withheld"), "{shown}");
    assert!(objects(&dir).is_empty(), "the bytes came back");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A crash between the record and the unlink is repaired by opening the library.
///
/// Simulated the only honest way: write the objects back under the library's own layout, which
/// is the state a process killed after the append would leave, and then open it.
#[test]
fn an_object_a_redaction_names_is_unlinked_on_the_next_open() {
    let (dir, tid) = library("repair");
    let root = dir.to_string_lossy().into_owned();
    let before = objects(&dir);
    assert_eq!(before.len(), 2);
    let saved: Vec<(String, Vec<u8>)> = before
        .iter()
        .map(|p| (p.clone(), std::fs::read(p).expect("an object")))
        .collect();

    assert!(run(&[
        "text",
        "redact",
        &tid,
        "--as",
        "human:vu",
        "--at",
        "1726500000000",
        "-s",
        &root
    ])
    .status
    .success());
    assert!(objects(&dir).is_empty());

    // The crash window: the record is in the log and the bytes are back on disk.
    for (path, bytes) in &saved {
        std::fs::write(path, bytes).expect("putting the object back");
    }
    assert_eq!(objects(&dir).len(), 2, "the window is open");

    // Any command that opens the library closes it. `ls` reads and writes nothing of its own.
    let out = run(&["text", "ls", "-s", &root]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(
        objects(&dir).is_empty(),
        "opening the library left a redacted part in it"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A redaction for a part this library never held is still recorded.
///
/// Which is the case that makes a redaction worth being a record at all: it is what stops the
/// part arriving later from a peer that has not heard.
#[test]
fn a_part_the_library_never_held_can_still_be_redacted() {
    let (dir, _) = library("absent");
    let root = dir.to_string_lossy().into_owned();
    // A tid of the right shape naming nothing here.
    let absent = "t3:w75spjohk7y23jgohkujj56vefvcxaxsox5c6lh6odeomuecjhta";
    let out = run(&[
        "text",
        "redact",
        absent,
        "--as",
        "human:vu",
        "--at",
        "1726500000000",
        "-s",
        &root,
    ]);
    assert!(out.status.success(), "{}", text(&out));
    let shown = text(&out);
    assert_eq!(field(&shown, "objects"), "0 unlinked");
    assert_eq!(field(&shown, "manifests"), "0 still name this part");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--as` is required, and an abbreviated tid is refused.
#[test]
fn a_redaction_names_an_agent_and_a_whole_tid() {
    let (dir, tid) = library("args");
    let root = dir.to_string_lossy().into_owned();

    let no_agent = run(&["text", "redact", &tid, "-s", &root]);
    assert!(!no_agent.status.success());
    assert!(text(&no_agent).contains("--as"), "{}", text(&no_agent));

    // The 26-character short form, which `Tid::parse` refuses everywhere.
    let short = &tid[..3 + 26];
    let abbreviated = run(&["text", "redact", short, "--as", "human:vu", "-s", &root]);
    assert!(!abbreviated.status.success());
    let shown = text(&abbreviated);
    assert!(shown.contains("not a tid"), "{shown}");
    assert!(shown.contains("52-character"), "{shown}");

    // And nothing was written by either refusal.
    assert_eq!(objects(&dir).len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The surface form of a redaction round-trips through `fmt`.
///
/// A part text has no surface form and a redaction does, which is the asymmetry rule Z is made
/// of: the bytes never travel as text, and the statement that they are to be held no longer
/// always can — including into a store that never had them.
#[test]
fn a_redaction_has_a_surface_form_that_round_trips() {
    let dir = scratch("surface");
    let doc = dir.join("r.smy");
    let src = "@redact t3:w75spjohk7y23jgohkujj56vefvcxaxsox5c6lh6odeomuecjhta \
               { agent: human:vu, ts: [1726500000000, 0] }\n";
    std::fs::write(&doc, src).expect("the document");
    let path = doc.to_string_lossy().into_owned();

    let out = run(&["fmt", "--check", &path]);
    assert!(out.status.success(), "{}", text(&out));
    let printed = run(&["fmt", &path]);
    assert!(printed.status.success(), "{}", text(&printed));
    assert!(
        String::from_utf8_lossy(&printed.stdout).contains("@redact t3:w75spjoh"),
        "{}",
        text(&printed)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
