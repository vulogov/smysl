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
    //
    // Modulo spans, which is what QG-7 means and now says (RFC SMYSL-2.9 §3.3). A span is an
    // offset into the text it was parsed from, and `text` is a different string from `src` by
    // construction — the printer discards the whitespace and the comments the offsets index
    // past. Requiring equal spans would be requiring two different layouts to be the same
    // layout, which holds only when the input was already canonical. A dropped node still
    // shows up here; only the offsets are excused.
    assert_eq!(
        modulo_spans(&once.query),
        modulo_spans(&again.query),
        "print/parse changed the tree"
    );
    assert_eq!(
        text,
        smysl_query::print(&again.query),
        "printing is not idempotent"
    );
    // What spans are held to instead, and it is sharper than equality: canonical text is all
    // content, so the query covers the whole of it. The first run of this target found a
    // trailing newline inside the span — `from library\n` closed at 13 — which is the shape
    // of defect that puts a diagnostic's caret a line below the word it is about.
    assert_eq!(
        (again.query.span.start, again.query.span.end),
        (0, text.len()),
        "the canonical text is not exactly the span of the query parsed from it"
    );
});

/// The tree's `Debug` with every `span:` field removed. `{:#?}` prints a `Span` over four
/// lines, which is the only thing this relies on; the grammar goldens strip them the same way
/// and for the same reason.
fn modulo_spans(q: &smysl_query::Query) -> String {
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
