//! The conformance tree is data, not code, so nothing checks it at compile time. These
//! tests keep it honest: every fixture must be paired with an expected-diagnostic file,
//! and every code named there must exist in the registry.
//!
//! SM-P0 validates the *shape* of the suite. SM-P1 adds the decoding half - feeding each
//! fixture to the reader and asserting the exact diagnostic set comes back.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use smysl::{Code, Severity};

fn codec_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/conformance/codec")
}

fn fixtures(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("fixture directory is missing")
        .map(|e| e.expect("unreadable fixture entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == ext))
        .collect();
    out.sort();
    out
}

fn expected_codes(fixture: &Path) -> BTreeSet<Code> {
    let path = fixture.with_extension("expected");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            Code::parse(l).unwrap_or_else(|| {
                panic!(
                    "{}: `{l}` is not a registered diagnostic code",
                    path.display()
                )
            })
        })
        .collect()
}

#[test]
fn every_codec_fixture_has_an_expected_set() {
    let dir = codec_dir();
    let files = fixtures(&dir, "cbor");
    assert!(!files.is_empty(), "the codec conformance tree is empty");
    for f in &files {
        assert!(
            f.with_extension("expected").is_file(),
            "{} has no .expected sibling",
            f.display()
        );
    }
}

#[test]
fn every_expected_code_is_registered() {
    for f in fixtures(&codec_dir(), "cbor") {
        // Panics inside `expected_codes` if a code is unknown.
        let _ = expected_codes(&f);
    }
}

#[test]
fn no_orphaned_expectation_files() {
    let dir = codec_dir();
    for e in fixtures(&dir, "expected") {
        assert!(
            e.with_extension("cbor").is_file(),
            "{} has no fixture",
            e.display()
        );
    }
}

/// The §15.4 table names six distinct ways to be non-deterministic and one way to be a
/// bad float. All of them must be represented, or the reader could pass the suite while
/// silently normalising one of them.
#[test]
fn codec_tree_covers_the_section_15_4_table() {
    let dir = codec_dir();
    let names: BTreeSet<String> = fixtures(&dir, "cbor")
        .iter()
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();

    for required in [
        "nonshortest-int",
        "indefinite-length-map",
        "indefinite-length-text",
        "unsorted-map-keys",
        "duplicate-map-key",
        "null-optional",
        "non-nfc-text",
        "float-f64",
        "float-unquantised",
        "unknown-envelope-code",
    ] {
        assert!(names.contains(required), "missing fixture `{required}`");
    }
}

/// A control that decodes cleanly is what distinguishes "the reader rejects everything"
/// from "the reader rejects the right things".
#[test]
fn the_tree_contains_clean_controls() {
    let clean: Vec<PathBuf> = fixtures(&codec_dir(), "cbor")
        .into_iter()
        .filter(|f| expected_codes(f).is_empty())
        .collect();
    assert!(
        clean.len() >= 2,
        "expected at least two fixtures that must decode cleanly, found {}",
        clean.len()
    );
}

/// An unknown envelope code is forward compatibility, not corruption: it MUST be a
/// warning, so a store written by a later minor version stays readable (rule X).
#[test]
fn unknown_envelope_code_is_a_warning_not_an_error() {
    let f = codec_dir().join("unknown-envelope-code.cbor");
    let codes = expected_codes(&f);
    assert_eq!(codes, BTreeSet::from([Code::W014]));
    assert_eq!(Code::W014.severity(), Severity::Warn);
}

/// Everything else in the tree is an error. A codec fixture that expected only warnings
/// would mean the reader is allowed to accept a non-deterministic encoding.
#[test]
fn every_defective_fixture_expects_at_least_one_error() {
    for f in fixtures(&codec_dir(), "cbor") {
        let codes = expected_codes(&f);
        let name = f.file_stem().unwrap().to_string_lossy().into_owned();
        if codes.is_empty() || name == "unknown-envelope-code" {
            continue;
        }
        assert!(
            codes.iter().any(|c| c.severity() == Severity::Error),
            "{name} expects only warnings; a non-deterministic encoding must be rejected"
        );
    }
}
