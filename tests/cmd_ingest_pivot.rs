//! `--lang-policy pivot:<lang>` at the command line: accepted where there is a reading, refused
//! where there is not, and refused by name where the tag is not one (SMYSL-2.4 §4.3.4, F-6).
//!
//! What is pinned here is the half that needs no model. A pivot pass makes a call, so the pass
//! itself is measured offline against a scripted provider in
//! `crates/smysl-ingest/tests/text_gate.rs`; this file is about the three answers a caller gets
//! before anything is sent — because the failure this policy exists to prevent is a run that
//! accepts `pivot:ru` and hands back gists in the passage's language.

#![cfg(all(feature = "cli", feature = "ingest"))]

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

fn dir_for(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("smysl-pivot-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A document ingest has no reading, so the policy is a usage error and not a provider error.
///
/// Exit 2 and a sentence naming where it does work. The library refuses it too, before egress
/// (`Ingestor::ingest`), and this is the same refusal moved one step earlier so that a caller
/// who typed it on the wrong command is not told about a registry first.
#[test]
fn a_document_ingest_refuses_a_pivot_policy_and_says_where_it_belongs() {
    let dir = dir_for("document");
    let f = dir.join("doc.txt");
    std::fs::write(&f, "The pool saturated.").unwrap();

    let out = Command::new(BIN)
        .current_dir(&dir)
        .args([
            "ingest",
            "--offline",
            "--lang-policy",
            "pivot:ru",
            f.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("pivot:ru"), "{stderr}");
    assert!(stderr.contains("--text"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The flag is open now — `pivot:<lang>` names a language, so the accepted values are the
/// language tags — and what used to be clap's closed list is `LangPolicy::parse`'s refusal.
///
/// Three things must still be refused, and the message must say which part was wrong: a policy
/// nobody implements, a pivot with no language, and `und`, which is a real answer for a unit
/// whose language nobody knows and no language to write in.
#[test]
fn what_is_not_a_policy_and_what_is_not_a_language_are_both_refused_by_name() {
    let dir = dir_for("refusals");
    let f = dir.join("doc.txt");
    std::fs::write(&f, "The pool saturated.").unwrap();

    for (value, expected) in [
        ("whatever", "is not a language policy"),
        ("pivot:", "names no language"),
        ("pivot:und", "is not a language to write in"),
        ("pivot:not a tag", "is not a language tag"),
    ] {
        let out = Command::new(BIN)
            .current_dir(&dir)
            .args([
                "ingest",
                "--offline",
                "--lang-policy",
                value,
                f.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "`{value}`: {stderr}");
        assert!(
            stderr.contains(expected),
            "`{value}` should say `{expected}`: {stderr}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `ingest --text --lang-policy pivot:ru` is accepted, and refused for the library it cannot
/// find rather than for the policy.
///
/// The point is the order: a policy this build implements must not be rejected before the run
/// gets as far as its target. `--dry-run` would be the cheaper check if `ingest --text` had one
/// that skipped resolution; it does not, so the evidence is *which* refusal comes back.
#[test]
fn a_text_ingest_accepts_the_policy_and_fails_on_the_library_instead() {
    let dir = dir_for("accepted");

    let out = Command::new(BIN)
        .current_dir(&dir)
        .args([
            "ingest",
            "--offline",
            "--text",
            "nosuch",
            "--lang-policy",
            "pivot:ru",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert_ne!(out.status.code(), Some(0), "{stderr}");
    assert!(
        !stderr.contains("pivot"),
        "the policy was refused rather than the target: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
