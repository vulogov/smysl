//! `pack --payload` (1.6): what `--query` is allowed to focus on.
//!
//! 1.6 gave `find` a filter on an extension schema's own field, and `pack --query` — which is how
//! a tool that packs by question rather than by uid actually builds a pack — still focused through
//! unrestricted retrieval. A review tool asking "what decisions does this diff touch" was focusing
//! on rejected alternatives and anticipated consequences, because all three are `claim`.

#![cfg(feature = "cli")]

use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");
const STORE: &str = "fixtures/corpus/F11-extension-kinds.smy";

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(["--format", "surface"])
        .args(args)
        .output()
        .expect("the binary under test must run")
}

fn out(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// The focus narrows to what the payload allows, and the pack follows it.
#[test]
fn payload_restricts_what_query_focuses_on() {
    let wide = run(&[
        "pack",
        "--budget",
        "300",
        "--query",
        "connection pool",
        STORE,
    ]);
    assert!(wide.status.success(), "{}", out(&wide));
    assert!(
        out(&wide).contains("focused 3 unit(s)"),
        "unrestricted, the query focuses on every claim about the pool:\n{}",
        out(&wide)
    );

    let narrow = run(&[
        "pack",
        "--budget",
        "300",
        "--query",
        "connection pool",
        "--payload",
        "code:kind=decision",
        STORE,
    ]);
    assert!(narrow.status.success(), "{}", out(&narrow));
    assert!(
        out(&narrow).contains("focused 1 unit(s)"),
        "the filter should leave the decision alone:\n{}",
        out(&narrow)
    );
    assert!(
        out(&narrow).contains("Pin the connection pool size"),
        "and that decision should be in the pack:\n{}",
        out(&narrow)
    );
}

/// A filter nothing satisfies leaves nothing to focus on, and packing says so rather than
/// quietly packing on merit — the same answer `--query` already gave when it matched nothing.
#[test]
fn a_filter_that_matches_nothing_fails_the_way_an_empty_query_does() {
    let o = run(&[
        "pack",
        "--budget",
        "300",
        "--query",
        "connection pool",
        "--payload",
        "code:kind=nonesuch",
        STORE,
    ]);
    assert!(!o.status.success());
    assert!(out(&o).contains("matched nothing"), "{}", out(&o));
}

/// It restricts `--query` and nothing else, so on its own it is a usage error rather than a
/// filter on the pack — which is what `--scope` is.
#[test]
fn payload_without_a_query_is_refused() {
    let o = run(&[
        "pack",
        "--budget",
        "300",
        "--payload",
        "code:kind=decision",
        STORE,
    ]);
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
    assert!(out(&o).contains("needs one"), "{}", out(&o));

    // And a malformed value is refused the same way `find` refuses it.
    let bad = run(&[
        "pack",
        "--budget",
        "300",
        "--query",
        "pool",
        "--payload",
        "code:kind",
        STORE,
    ]);
    assert_eq!(bad.status.code(), Some(2), "{}", out(&bad));
    assert!(out(&bad).contains("KEY=VALUE"), "{}", out(&bad));
}
