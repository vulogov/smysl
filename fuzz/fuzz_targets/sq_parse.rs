//! Guarantee A1 for the query language: no panics on untrusted input, and QG-7 on anything
//! accepted (RFC SMYSL-2.9 §5; SMYSL-2.5 §5.3 asks for this target by name).
//!
//! A query arrives from a person, a dossier template or — through the natural-language front
//! end — a model, so "the parser must not panic" is a requirement rather than hygiene. The
//! second assertion is the one a corpus of fixtures cannot make: whatever the fuzzer finds
//! that parses must also print and re-parse to the same tree, which is where a lexer mode that
//! loses bytes or a printer that drops a node will show up.

#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(once) = smysl_query::parse(src) else {
        return;
    };
    let text = smysl_query::print(&once.query);
    let again = smysl_query::parse(&text)
        .expect("the canonical form of an accepted query is itself an accepted query");
    // Trees, not texts: equality of the printed form is the second law and follows from this
    // one, but a mismatch here points at the node that was lost.
    assert_eq!(once.query, again.query, "print/parse changed the tree");
    assert_eq!(
        text,
        smysl_query::print(&again.query),
        "printing is not idempotent"
    );
});
