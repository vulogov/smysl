//! The `Library` check pass (pass 12, RFC SMYSL-2.4 §4.3.3), TX-P1 step 5.
//!
//! `fixtures/library/check/` is driven here the way `fixtures/conformance/check/` is driven in
//! `conformance_fixtures.rs`: one document per defect, one expected code set, exactly.
//!
//! `SMY-E452` cannot be reached from a `.smy` file at all — records 15 and 18 have no surface
//! form, because a megabyte of someone else's prose inside a quoted string is neither readable
//! nor diffable — so it is tested below, as rule T's `SMY-E033` is, and for the same kind of
//! reason.
//!
//! `SMY-E446` and `SMY-E401` are the pass's other half and are **not here**: they are about
//! bytes, a log holds none (OQ-39), and the only `PartResolver` implementation is
//! `smysl-text`'s object store. They live in `crates/smysl-text/tests/object_check.rs`, which
//! says why.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use smysl::{check, CheckOptions, Code, Store};
// The facade does not re-export the library types yet — `Record::Manifest`'s payload is
// unnameable through `smysl` — so these come from the crates that define them. TX-P1 step 6
// owns the facade surface; see the note in SMYSL-2.4 §6.
use smysl_check::passes::library::SKIPPED;
use smysl_core::types::PartText;

fn check_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/library/check")
}

fn fixtures(ext: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(check_dir())
        .expect("the library check tree is missing")
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

/// Every code a fixture produces, parsing and checking together — the same helper the other
/// trees use, and for the same reason: which layer catches a defect is an implementation
/// detail, and what a fixture pins is the code set.
fn all_codes(src: &str) -> BTreeSet<Code> {
    let out = match smysl::parse_surface(src) {
        Ok(o) => o,
        Err(e) => return BTreeSet::from([e.code()]),
    };
    let mut codes: BTreeSet<Code> = out.diagnostics.iter().map(|d| d.code).collect();
    let store = Store::from_records(out.records.clone());
    let opts = CheckOptions::default().with_labels(out.labels.clone());
    codes.extend(check(&store, opts).iter().map(|d| d.code));
    codes
}

#[test]
fn every_fixture_has_an_expected_set() {
    let files = fixtures("smy");
    assert!(!files.is_empty(), "the library check tree is empty");
    for f in &files {
        assert!(
            f.with_extension("expected").is_file(),
            "{} has no .expected sibling",
            f.display()
        );
    }
}

#[test]
fn no_orphaned_expectation_files() {
    for e in fixtures("expected") {
        assert!(
            e.with_extension("smy").is_file(),
            "{} has no fixture",
            e.display()
        );
    }
}

/// The step's exit: exactly the expected codes, no more and no fewer.
#[test]
fn every_fixture_produces_exactly_its_expected_codes() {
    for f in fixtures("smy") {
        let src = std::fs::read_to_string(&f).unwrap();
        let observed = all_codes(&src);
        let expected = expected_codes(&f);
        assert_eq!(
            observed,
            expected,
            "{}: observed {observed:?}, expected {expected:?}",
            f.file_name().unwrap().to_string_lossy()
        );
    }
}

/// Two controls, and the second is the one that distinguishes a working `heads` from one that
/// returns every manifest under an alias.
#[test]
fn the_tree_has_both_controls() {
    for name in ["clean-control", "superseded-chain"] {
        let src = std::fs::read_to_string(check_dir().join(format!("{name}.smy"))).unwrap();
        let codes = all_codes(&src);
        assert!(codes.is_empty(), "{name} must be clean, got {codes:?}");
    }
}

/// Every code the tree can reach is reached by it.
///
/// The list is written out rather than derived from the pass, because deriving it would make
/// the assertion "the fixtures cover whatever the pass happens to raise" — which is true of an
/// empty tree and a pass that raises nothing.
#[test]
fn the_tree_covers_every_code_a_document_can_reach() {
    let mut seen: BTreeSet<Code> = BTreeSet::new();
    for f in fixtures("smy") {
        seen.extend(expected_codes(&f));
    }
    assert_eq!(seen, BTreeSet::from([Code::E403, Code::W418]));
}

// ---------------------------------------------------------------------------
// `SMY-E452`: no surface form, so no fixture
// ---------------------------------------------------------------------------

/// A log that holds text, which `append` refuses and `from_records` does not.
///
/// The second half is the point. `Store::append` raises `SMY-E452` and refuses the whole
/// batch, so nothing this build *writes* can reach the pass — but `from_records` and
/// `Store::open` absorb records directly, deliberately, because F-12's lesson is that one bad
/// record must not stop `open`. So a log written by a producer that did not refuse it decodes
/// and is checkable, and this is the path that makes the code worth registering.
#[test]
fn a_log_holding_text_is_reported_rather_than_unopenable() {
    use smysl::Record;

    let part = PartText::new(b"a part text that should never be in a log\n".to_vec());
    let tid = part.tid;

    // `append` refuses it, whole batch, before a byte is written.
    let mut store = Store::new();
    let err = store
        .append(&[Record::PartText(part.clone())])
        .expect_err("a log does not hold text");
    assert!(
        err.to_string().contains("record 15"),
        "the refusal must say what was offered: {err}"
    );

    // Absorbed directly, it is there and the pass says so.
    let store = Store::from_records(vec![Record::PartText(part)]);
    let report = check(&store, CheckOptions::default());
    assert_eq!(report.count(Code::E452), 1, "{report}");
    let d = report
        .iter()
        .find(|d| d.code == Code::E452)
        .expect("the diagnostic");
    assert!(
        d.message.contains(&tid.canonical()),
        "the diagnostic must name the tid so the bytes can be moved: {d}"
    );
    assert!(d.suggestion.is_some(), "{d}");
}

/// And C-Library refuses such a store, which §4.3.3's forbidden list does not say.
#[test]
fn text_in_a_log_blocks_c_library() {
    use smysl::{conformance, ConformanceClass, Record};
    let part = PartText::new(b"text in the wrong place\n".to_vec());
    let store = Store::from_records(vec![Record::PartText(part)]);
    let report = check(&store, CheckOptions::default());

    let verdict = conformance(&report, ConformanceClass::Library);
    assert!(!verdict.passed, "{verdict}");
    assert_eq!(verdict.blocking, vec![Code::E452]);
    // Still readable: C-Read only has to parse and verify hashes.
    assert!(conformance(&report, ConformanceClass::Read).passed);
}

// ---------------------------------------------------------------------------
// What the pass does not check
// ---------------------------------------------------------------------------

/// The two codes the plan allocated and this build cannot raise, named in one place.
///
/// The assertion is that they are **not** in the registry, which is the repository's own rule:
/// a code a reader can grep for and find nothing behind is worse than a missing one. The day
/// `SourceRef.span` lands, this test is what says the list has to move.
#[test]
fn the_unreachable_codes_are_named_and_unregistered() {
    let names: Vec<&str> = SKIPPED.iter().map(|(code, _)| *code).collect();
    assert_eq!(names, ["SMY-E404", "SMY-W405"]);
    for (code, reason) in SKIPPED {
        assert!(
            Code::parse(code).is_none(),
            "{code} is registered; it belongs in the pass rather than in SKIPPED"
        );
        assert!(
            reason.contains("TX-P5"),
            "{code} must say what it waits for"
        );
    }
}
