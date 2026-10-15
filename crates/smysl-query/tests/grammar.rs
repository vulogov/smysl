//! The grammar goldens (RFC SMYSL-2.9 §5, refining SMYSL-2.5 §5.1).
//!
//! Four things are asserted over `fixtures/sq/`, and the division matters:
//!
//! - **accept** fixtures parse, and their tree is compared against a committed dump. The dump
//!   leaves spans out: a span is a byte offset, so every fixture would churn on an edit to the
//!   whitespace above it, and spans are asserted where they are actually read — in the reject
//!   goldens, which carry one.
//! - **reject** fixtures fail with exactly one code, one span and one message. The message is
//!   compared because a grammar meant to read like English owes a diagnostic that reads like
//!   it too.
//! - **QG-7**, both equations, over every accept fixture.
//! - The reject set holds a **near-miss of every accept fixture** by construction — the risk
//!   in a hand-written recursive-descent parser is never ambiguity, it is unintended
//!   acceptance, which no accept fixture can detect.
//!
//! `SMYSL_BLESS=1` rewrites the expectations, which should be a decision rather than a habit.

use std::path::{Path, PathBuf};

fn dir(which: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/sq")
        .join(which)
}

fn cases(which: &str) -> Vec<(String, String, PathBuf)> {
    let d = dir(which);
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&d).expect("fixture directory") {
        let p = entry.expect("entry").path();
        if p.extension().and_then(|e| e.to_str()) != Some("sq") {
            continue;
        }
        let name = p
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("utf-8 name")
            .to_string();
        let src = std::fs::read_to_string(&p).expect("fixture is readable");
        out.push((name, src, p));
    }
    // `read_dir` order is the filesystem's, which is not an order. Sorting makes a failing run
    // reproducible and the determinism matrix meaningful.
    out.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(!out.is_empty(), "no fixtures in {}", d.display());
    out
}

fn blessing() -> bool {
    std::env::var("SMYSL_BLESS").is_ok_and(|v| v == "1")
}

fn compare(path: &Path, got: &str, what: &str) {
    if blessing() {
        std::fs::write(path, got).expect("expectation is writable");
        return;
    }
    let want = std::fs::read_to_string(path).unwrap_or_else(|_| {
        panic!(
            "{} is missing for {}; run with SMYSL_BLESS=1 to write it",
            what,
            path.display()
        )
    });
    assert_eq!(
        want.trim_end(),
        got.trim_end(),
        "{what} differs for {}",
        path.display()
    );
}

/// `{:#?}` with the four lines of every `span: Span { … }` removed. Span is a closed
/// two-field struct, so the shape is fixed and the filter cannot drift.
fn dump(q: &smysl_query::Query) -> String {
    let full = format!("{q:#?}");
    let lines: Vec<&str> = full.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim_start().starts_with("span: Span {") {
            i += 4;
            continue;
        }
        out.push(lines[i]);
        i += 1;
    }
    out.join("\n")
}

#[test]
fn accept_fixtures_parse_to_their_committed_tree() {
    for (name, src, path) in cases("accept") {
        let parsed = smysl_query::parse(&src)
            .unwrap_or_else(|d| panic!("{name} should parse, but: {} {}", d.code, d.message));
        compare(
            &path.with_extension("ast"),
            &dump(&parsed.query),
            "the tree",
        );
    }
}

#[test]
fn accept_fixtures_print_to_their_committed_canonical_form() {
    for (name, src, path) in cases("accept") {
        let parsed = smysl_query::parse(&src).unwrap_or_else(|d| panic!("{name}: {}", d.message));
        let printed = smysl_query::print(&parsed.query);
        compare(&path.with_extension("sqc"), &printed, "the canonical form");
    }
}

/// QG-7, first equation: printing loses nothing the tree holds.
#[test]
fn printing_then_parsing_gives_the_same_tree() {
    for (name, src, _) in cases("accept") {
        let once = smysl_query::parse(&src).unwrap_or_else(|d| panic!("{name}: {}", d.message));
        let text = smysl_query::print(&once.query);
        let twice = smysl_query::parse(&text)
            .unwrap_or_else(|d| panic!("{name} did not re-parse: {} on `{text}`", d.message));
        assert_eq!(
            dump(&once.query),
            dump(&twice.query),
            "{name} does not round trip"
        );
    }
}

/// QG-7, second equation: printing is idempotent, so a query has one canonical text.
#[test]
fn printing_is_idempotent() {
    for (name, src, _) in cases("accept") {
        let once = smysl_query::parse(&src).unwrap_or_else(|d| panic!("{name}: {}", d.message));
        let first = smysl_query::print(&once.query);
        let again = smysl_query::parse(&first).expect("canonical text parses");
        assert_eq!(
            first,
            smysl_query::print(&again.query),
            "{name} is not idempotent"
        );
    }
}

#[test]
fn reject_fixtures_fail_with_their_committed_diagnostic() {
    for (name, src, path) in cases("reject") {
        match smysl_query::parse(&src) {
            Ok(p) => panic!(
                "{name} should not parse, but produced:\n{}",
                smysl_query::print(&p.query)
            ),
            Err(d) => {
                let span = d
                    .span()
                    .unwrap_or_else(|| panic!("{name}: a syntax diagnostic carries a span"));
                let got = format!("{} {}..{} {}", d.code, span.start, span.end, d.message);
                compare(&path.with_extension("diag"), &got, "the diagnostic");
            }
        }
    }
}

/// Every accepted query carries at least one stage; an empty pipeline is refused rather than
/// silently standing for "everything".
#[test]
fn an_empty_query_is_refused() {
    assert!(smysl_query::parse("   \n  # nothing but a comment\n").is_err());
}

/// QG-5: a literal is kept byte-exact, and the parse says so rather than normalising it.
#[test]
fn a_non_nfc_literal_is_kept_and_reported() {
    // "e" followed by a combining acute, which NFC would fold to a single code point.
    let parsed = smysl_query::parse("from library\nfind \"cafe\u{301}\"").expect("parses");
    assert_eq!(parsed.warnings.len(), 1);
    assert_eq!(parsed.warnings[0].code.as_str(), "SMY-W510");
    assert!(
        smysl_query::print(&parsed.query).contains('\u{301}'),
        "the combining mark survives printing"
    );
}

/// QG-4: keyword folding is ASCII-only, so a name carrying a character that *Unicode* folding
/// would map onto a keyword letter stays a name. The Kelvin sign is the standing example.
#[test]
fn keyword_folding_is_ascii_only() {
    let err = smysl_query::parse("from library\nLIMIT 5").err();
    assert!(
        err.is_none(),
        "an ASCII-uppercase keyword is still a keyword"
    );
    assert!(smysl_query::parse("from library\n\u{212A}imit 5").is_err());
}

/// QG-3, in the form available before step 7.
///
/// The determinism matrix (`xtask determinism`) runs a *command* under permuted locale,
/// timezone and hash seed, and there is no `smysl q` to run until TX-P6 step 7 — so the matrix
/// entry lands with it. What can be asserted now is stronger than it sounds and is the whole
/// of what the matrix would catch here: this crate never reads the environment, so there is
/// nothing for a permutation to change. A parse that consulted the clock, the locale or an
/// environment variable would have to name one of these.
#[test]
fn the_parse_reads_nothing_from_its_environment() {
    for (name, src) in sources() {
        for forbidden in [
            "std::env",
            "env::var",
            "SystemTime",
            "Instant::now",
            "RandomState",
        ] {
            assert!(
                !src.contains(forbidden),
                "{name} names `{forbidden}`; a parse must not depend on where or when it runs"
            );
        }
    }
}

/// QG-4: keyword folding is ASCII-only, because Unicode's case-folding tables change between
/// Unicode versions and the four implementations of this grammar will not ship the same one.
/// `to_lowercase` would be that dependency, so it may not appear.
#[test]
fn case_folding_is_ascii_only_in_the_source_too() {
    for (name, src) in sources() {
        assert!(
            !src.contains(".to_lowercase()") && !src.contains(".to_uppercase()"),
            "{name} folds case with Unicode tables; use the ASCII forms (QG-4)"
        );
    }
}

fn sources() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("src is readable") {
        let p = entry.expect("entry").path();
        if p.extension().and_then(|e| e.to_str()) == Some("rs") {
            let name = p
                .file_name()
                .and_then(|s| s.to_str())
                .expect("utf-8")
                .to_string();
            out.push((name, std::fs::read_to_string(&p).expect("readable")));
        }
    }
    out.sort();
    assert!(out.len() >= 4, "expected the crate's modules");
    out
}
