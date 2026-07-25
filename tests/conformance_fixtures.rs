//! The conformance tree is data, not code, so nothing checks it at compile time. These
//! tests keep it honest: every fixture must be paired with an expected-diagnostic file,
//! and every code named there must exist in the registry.
//!
//! SM-P0 validated the *shape* of the suite. SM-P1 adds the decoding half: every fixture
//! is fed to the reader and the exact diagnostic set is asserted.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use smysl::{from_cbor, Code, Record, Severity};

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

/// What the reader actually reports for a fixture, expressed as diagnostic codes.
///
/// A clean decode is the empty set. An unknown record type is `SMY-W014` - a warning,
/// because the record survives - and everything else is the error the reader raised.
fn observed_codes(bytes: &[u8]) -> BTreeSet<Code> {
    match from_cbor(bytes) {
        Ok((Record::Unknown { .. }, _)) => BTreeSet::from([Code::W014]),
        Ok(_) => BTreeSet::new(),
        Err(e) => BTreeSet::from([e.code()]),
    }
}

/// The SM-P1 gate: every non-conforming fixture is rejected with exactly the expected
/// code set - no more, no fewer.
#[test]
fn every_codec_fixture_produces_its_expected_diagnostics() {
    for f in fixtures(&codec_dir(), "cbor") {
        let bytes = std::fs::read(&f).unwrap();
        let name = f.file_stem().unwrap().to_string_lossy().into_owned();
        assert_eq!(
            observed_codes(&bytes),
            expected_codes(&f),
            "fixture `{name}` did not produce its expected diagnostics"
        );
    }
}

/// The controls must decode *and* re-encode to the same bytes. Without that half, a
/// reader could pass by accepting the control and silently rewriting it.
#[test]
fn clean_fixtures_re_encode_to_the_same_bytes() {
    for f in fixtures(&codec_dir(), "cbor") {
        if !expected_codes(&f).is_empty() {
            continue;
        }
        let bytes = std::fs::read(&f).unwrap();
        let (record, n) = from_cbor(&bytes).unwrap();
        assert_eq!(n, bytes.len(), "{}: trailing bytes", f.display());
        assert_eq!(
            smysl::to_cbor(&record),
            bytes,
            "{}: re-encoding changed the bytes",
            f.display()
        );
    }
}

/// The unknown-type fixture is the one that must survive rather than fail, and its
/// payload must come back byte-identical.
#[test]
fn the_unknown_type_fixture_survives_verbatim() {
    let bytes = std::fs::read(codec_dir().join("unknown-envelope-code.cbor")).unwrap();
    let (record, _) = from_cbor(&bytes).unwrap();
    assert!(record.is_unknown());
    assert_eq!(smysl::to_cbor(&record), bytes);
}

// ---------------------------------------------------------------------------
// The corpus (§27.2)
// ---------------------------------------------------------------------------

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/corpus")
}

/// Diagnostics a corpus fixture produces when parsed.
fn parse_codes(src: &str) -> BTreeSet<Code> {
    match smysl::parse_surface(src) {
        Ok(o) => o.diagnostics.iter().map(|d| d.code).collect(),
        Err(e) => BTreeSet::from([e.code()]),
    }
}

#[test]
fn every_corpus_fixture_has_an_expected_set() {
    let files = fixtures(&corpus_dir(), "smy");
    assert!(!files.is_empty(), "the corpus is empty");
    for f in &files {
        assert!(
            f.with_extension("expected").is_file(),
            "{} has no .expected sibling",
            f.display()
        );
    }
}

#[test]
fn every_corpus_fixture_produces_its_expected_diagnostics() {
    for f in fixtures(&corpus_dir(), "smy") {
        let src = std::fs::read_to_string(&f).unwrap();
        assert_eq!(
            parse_codes(&src),
            expected_codes(&f),
            "{} did not produce its expected diagnostics",
            f.display()
        );
    }
}

/// The corpus is what every later phase is measured against, so it has to survive the
/// full surface -> records -> surface -> records loop unchanged.
#[test]
fn every_corpus_fixture_round_trips() {
    use smysl::{parse_surface, write_surface, WriteContext};
    for f in fixtures(&corpus_dir(), "smy") {
        if !expected_codes(&f).is_empty() {
            continue;
        }
        let src = std::fs::read_to_string(&f).unwrap();
        let a = parse_surface(&src).unwrap();
        let ctx = WriteContext::from_labels(&a.labels).with_salience(a.salience.clone());
        let text = write_surface(a.view.as_ref(), &a.records, &ctx);
        let b = parse_surface(&text).unwrap();
        assert_eq!(b.records, a.records, "{} lost content", f.display());
        assert_eq!(b.labels, a.labels, "{} moved a uid", f.display());

        let bytes = smysl::to_cbor_seq(&a.records);
        let (back, _) = smysl::from_cbor_seq(&bytes).unwrap();
        assert_eq!(back, a.records, "{} lost content over CBOR", f.display());
    }
}

/// F1 exercises the shapes rules M and R are about: a grounds chain deep enough for the
/// monotonicity check to bind, and a rebuttal for rule R to pin.
#[test]
fn f1_carries_a_rebuttal_and_a_grounds_chain() {
    let src = std::fs::read_to_string(corpus_dir().join("F1-incident.smy")).unwrap();
    let out = smysl::parse_surface(&src).unwrap();
    assert!(out
        .records
        .iter()
        .any(|r| matches!(r, Record::Relation(rel) if rel.kind == smysl::RelKind::Rebuts)));
    assert!(out
        .records
        .iter()
        .any(|r| matches!(r, Record::Relation(rel) if rel.kind == smysl::RelKind::Warrant)));
    let grounded = out
        .records
        .iter()
        .filter_map(Record::as_unit)
        .filter(|u| !u.grounds.is_empty())
        .count();
    assert!(grounded >= 4, "only {grounded} units carry grounds");
}

/// F3 is the design's most likely falsifier (GE-2): narrative carried on a claim graph.
/// Until that experiment runs, the least it must do is survive the format intact.
#[test]
fn f3_is_coarse_and_ordered() {
    let src = std::fs::read_to_string(corpus_dir().join("F3-narrative.smy")).unwrap();
    let out = smysl::parse_surface(&src).unwrap();
    let v = out.view.as_ref().unwrap();
    assert_eq!(v.granularity.profile, "coarse");
    assert_eq!(v.granularity.admission, smysl::Admission::Topical);

    let thread = out
        .records
        .iter()
        .find_map(|r| match r {
            Record::Thread(t) => Some(t),
            _ => None,
        })
        .unwrap();
    assert_eq!(thread.schema, smysl::ThreadSchema::Narrative);
    assert_eq!(thread.steps.len(), 5);
    assert!(thread.foreign_roles().is_empty());
}
