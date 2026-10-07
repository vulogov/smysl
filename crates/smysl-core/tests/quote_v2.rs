//! F-3: the V2 comparison form, and the promise that V1 did not move.
//!
//! Five probes in SMYSL-2.1 §2.1 are quotes a reader would call verbatim and the checker called
//! `Loose` or `Absent`: French guillemets with spaces inside them, German low-high quotation
//! marks, `всё` against `все`, `Straße` against `STRASSE`, and a modifier-letter apostrophe in
//! `lʼhomme`. None of them is a fabrication, and `E307` on an honest quote is the expensive
//! direction of that error — it refuses a correct attribution.
//!
//! Two claims here, and they pull against each other, which is why both are asserted over the
//! same rows:
//!
//! 1. **V2 answers the probes.** Each reads `Present`.
//! 2. **V1 did not move.** `support` and its three siblings have been public contract since 1.3,
//!    so a verdict somebody stored is still that verdict. Every row states the V1 answer too.

use smysl_core::quote::{
    support, support_span, support_span_with, support_with, Normaliser, Support,
};

const V2_TSV: &str = include_str!("../../../fixtures/quote/v2.tsv");
const COMMIT: &str = include_str!("../../../fixtures/quote/commit-4968383.md");

fn verdict(name: &str) -> Support {
    match name {
        "present" => Support::Present,
        "loose" => Support::Loose,
        "absent" => Support::Absent,
        other => panic!("`{other}` is not a verdict"),
    }
}

fn rows() -> Vec<(String, String, Support, Support)> {
    V2_TSV
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            assert_eq!(f.len(), 4, "a row is quote, source, V1, V2: {l:?}");
            (
                f[0].to_string(),
                f[1].to_string(),
                verdict(f[2]),
                verdict(f[3]),
            )
        })
        .collect()
}

/// Every row of the conformance set, under both normalisers.
#[test]
fn the_conformance_set_holds_for_both_normalisers() {
    let rows = rows();
    assert!(rows.len() >= 18, "the set lost rows: {}", rows.len());
    for (q, s, v1, v2) in rows {
        assert_eq!(
            support_with(Normaliser::V1, &q, &s),
            v1,
            "V1: {q:?} in {s:?}"
        );
        assert_eq!(
            support_with(Normaliser::V2, &q, &s),
            v2,
            "V2: {q:?} in {s:?}"
        );
        // The 1.3 entry point is V1, which is the whole of the compatibility promise.
        assert_eq!(support(&q, &s), v1, "`support` must be V1: {q:?} in {s:?}");
    }
}

/// The five probes, named, so a failure says which one.
///
/// Redundant with the row sweep above and kept anyway: the sweep says "row 7 failed", and these
/// say "the German quotation marks regressed", which is the sentence somebody needs at 2 a.m.
#[test]
fn every_probe_from_the_rfc_reads_present_under_v2() {
    let probes: &[(&str, &str, &str)] = &[
        (
            "French guillemets, spaces inside",
            "\u{00AB}Libert\u{00E9}\u{00BB}",
            "Il a dit \u{00AB} Libert\u{00E9} \u{00BB} hier.",
        ),
        (
            "German low-high quotation marks",
            "\u{00AB}Freiheit\u{00BB}",
            "\u{201E}Freiheit\u{201C}",
        ),
        (
            "\u{0451} against \u{0435}",
            "\u{0432}\u{0441}\u{0451}",
            "\u{0432}\u{0441}\u{0435}",
        ),
        ("the German case-fold pair", "Stra\u{00DF}e", "STRASSE"),
        ("modifier-letter apostrophe", "l'homme", "l\u{02BC}homme"),
    ];
    for (what, q, s) in probes {
        assert_eq!(
            support_with(Normaliser::V2, q, s),
            Support::Present,
            "{what}"
        );
    }
}

/// V1 is byte for byte what it was, over a document that exercises it.
///
/// The fixture is a real commit message with backticks, elisions and typographic punctuation —
/// the text the 1.5 span work was verified against. Asserting the verdicts here rather than
/// trusting that "V1 is untouched" means a refactor of the shared walk cannot quietly move them.
#[test]
fn v1_is_byte_for_byte_unchanged() {
    for invented in [
        "Rust was rewritten in Go to match the Python implementation.",
        "blake3.js is a hand-rolled binding to the same C library as Rust",
        "It was decided that tests are not needed for the Node port.",
    ] {
        assert_eq!(support(invented, COMMIT), Support::Absent, "{invented}");
    }
    assert_eq!(
        support(
            "nodejs/ reaches C-Produce, a gate ties the format's constants to the",
            COMMIT
        ),
        Support::Present
    );
    assert_eq!(
        support("the status integers ... the base32 alphabet", COMMIT),
        Support::Loose
    );

    // And the spans: a range into the source as given, which is what a caller points a reader at.
    let (verdict, span) = support_span("a gate ties the format's constants", COMMIT);
    assert_eq!(verdict, Support::Present);
    let span = span.expect("a present quote has a range");
    assert_eq!(&COMMIT[span], "a gate ties the format's constants");
}

/// Whether a character survives `n`'s comparison form.
///
/// Asked through the public surface rather than the private fold table, so the question is "does
/// this character reach the comparison" and not "is it in a list I also wrote". A character that
/// normalises away compares `Absent` against itself, because the quote side becomes empty.
fn productive(n: Normaliser, c: char) -> bool {
    let one = c.to_string();
    support_with(n, &one, &one) == Support::Present
}

/// **MS-4.** A `Present` span ends at the match, not past a mark the fold dropped.
///
/// The spike measured this on 5,560 spans: V2's range was wider than V1's in 10 of them, every one
/// the same shape — a verse ending before a closing `”` whose range swallowed it. 0.18%, and the
/// reason the second normaliser was not worth defaulting to while it held.
///
/// Both normalisers are asserted because **both were wrong**, which the finding did not know. V2
/// drops quotation marks, so the corpus found it there; V1 folds them to `"` and keeps them, but
/// V1 drops `` ` `` and `*` and overshoots on those in exactly the same way. One defect in two
/// walkers, from one cause: an upper bound read off the table of where characters *begin*.
#[test]
fn a_present_span_does_not_swallow_a_dropped_mark() {
    // (source, quote, the text the range must cover)
    let cases: &[(&str, &str, &str)] = &[
        // The measured shape, in the script it was measured in.
        (
            "Он сказал: “доколе, Господи” И забудешь меня.",
            "доколе, Господи",
            "доколе, Господи",
        ),
        // The same shape in ASCII, so nothing about it is Cyrillic.
        (
            "He said \"hello there\" and left.",
            "hello there",
            "hello there",
        ),
        // The marks V1 drops rather than folds. These are the V1 half of the defect.
        (
            "He said `hello there` and left.",
            "hello there",
            "hello there",
        ),
        (
            "He said *hello there* and left.",
            "hello there",
            "hello there",
        ),
        // A dropped mark before the match must not widen the lower bound either.
        (
            "He said \"hello there and left.",
            "hello there",
            "hello there",
        ),
        // Several dropped marks in a row.
        (
            "He said ““hello there”” and left.",
            "hello there",
            "hello there",
        ),
        // The match ending at the very end of the source, with a mark after it.
        ("he said “hello there”", "hello there", "hello there"),
    ];

    for (source, quote, want) in cases {
        for n in [Normaliser::V1, Normaliser::V2] {
            let (verdict, range) = support_span_with(n, quote, source);
            assert_eq!(verdict, Support::Present, "{n}: {quote:?} in {source:?}");
            let r = range.expect("a present quote has a range");
            assert_eq!(
                &source[r.clone()],
                *want,
                "{n}: {quote:?} in {source:?} — range {r:?} is not the match"
            );
        }
    }
}

/// The same for `Loose`: the region ends on the last word matched, not past a mark dropped
/// after it.
///
/// Each normaliser is given a mark **it** drops, because that is the defect's shape and the two
/// tables differ. V2 drops quotation marks; V1 folds them to `"` and keeps them as word content,
/// so V1's own blind spot is the backtick. Asserting a shared expectation here would be asserting
/// that the two normalisers agree about typography, which they deliberately do not.
#[test]
fn a_loose_span_ends_on_the_last_word_matched() {
    let cases: &[(Normaliser, &str)] = &[
        (
            Normaliser::V2,
            "He said hello to the wide world” and then left.",
        ),
        (
            Normaliser::V1,
            "He said hello to the wide world` and then left.",
        ),
    ];
    for (n, source) in cases {
        let (verdict, range) = support_span_with(*n, "hello ... world", source);
        assert_eq!(verdict, Support::Loose, "{n}");
        let r = range.expect("a loose quote has a range");
        assert_eq!(
            &source[r.clone()],
            "hello to the wide world",
            "{n}: range {r:?} is not the region the quote was drawn from"
        );
    }
}

/// A span is a region a caller slices and shows to a reader, so trailing whitespace is an
/// overshoot too — the same class as the dropped mark, and the sentinel already handled it. This
/// is the control that says the MS-4 fix did not cost it.
#[test]
fn a_span_does_not_reach_into_trailing_whitespace() {
    let source = "hello there   \n\n";
    for n in [Normaliser::V1, Normaliser::V2] {
        let (_, range) = support_span_with(n, "hello there", source);
        let r = range.expect("a present quote has a range");
        assert_eq!(&source[r], "hello there", "{n}");
    }
}

/// The verdict and the span agree, for both normalisers, over generated input.
///
/// `support_with` finds a substring in a string the character fold produced; `support_span_with`
/// walks the same text again to map that index back. Two walks are two chances to disagree, and
/// the RFC asked for this property rather than a example of it. The generator is the fold's own
/// table: every character it treats specially, in combinations a hand-written case would miss.
#[test]
fn a_span_and_a_verdict_never_disagree() {
    const INTERESTING: &[&str] = &[
        "a",
        " ",
        "\u{00AB}",
        "\u{00BB}",
        "\u{201C}",
        "\u{201D}",
        "\u{201E}",
        "\u{2018}",
        "\u{2019}",
        "\u{02BC}",
        "'",
        "\"",
        "\u{00BF}",
        "\u{00A1}",
        "\u{0451}",
        "\u{0435}",
        "\u{0435}\u{0308}",
        "\u{00DF}",
        "ss",
        "\u{202F}",
        "\u{00A0}",
        "`",
        "*",
        "-",
        "\u{2014}",
        "\u{2026}",
        "b",
    ];

    // A xorshift, so a failure is reproducible from its seed.
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let build = |len: usize, next: &mut dyn FnMut() -> u64| -> String {
        (0..len)
            .map(|_| INTERESTING[(next() % INTERESTING.len() as u64) as usize])
            .collect()
    };

    for _ in 0..4_000 {
        let source = build(1 + (next() % 12) as usize, &mut next);
        let quote = if next() % 2 == 0 {
            // A substring of the source, which is where `Present` and the span both matter.
            let chars: Vec<char> = source.chars().collect();
            let from = (next() as usize) % chars.len();
            let to = from + 1 + (next() as usize) % (chars.len() - from);
            chars[from..to].iter().collect()
        } else {
            build(1 + (next() % 6) as usize, &mut next)
        };

        for n in [Normaliser::V1, Normaliser::V2] {
            let plain = support_with(n, &quote, &source);
            let (spanned, range) = support_span_with(n, &quote, &source);
            assert_eq!(
                plain, spanned,
                "{n}: {quote:?} in {source:?} \u{2014} verdict and span disagree"
            );
            match spanned {
                Support::Absent => assert!(range.is_none()),
                _ => {
                    let r = range.expect("a match has a range");
                    assert!(
                        r.start <= r.end && r.end <= source.len(),
                        "{n}: {quote:?} in {source:?} \u{2014} range {r:?} is not inside the source"
                    );
                    assert!(
                        source.is_char_boundary(r.start) && source.is_char_boundary(r.end),
                        "{n}: {quote:?} in {source:?} \u{2014} range {r:?} splits a character"
                    );
                    // And **tight**: neither end may be a character the comparison form throws
                    // away. A valid range is not enough — MS-4's overshoot was in bounds, on a
                    // character boundary, and still wrong, because it reached past the match to
                    // take in a closing quote mark the fold had dropped. Without this assertion
                    // the generator above passes with the defect in place, which is how it
                    // survived to be measured on a corpus instead of caught here.
                    let slice = &source[r.clone()];
                    if let Some(c) = slice.chars().next() {
                        assert!(
                            productive(n, c),
                            "{n}: {quote:?} in {source:?} — range {r:?} starts on {c:?}, dropped by this form"
                        );
                    }
                    if let Some(c) = slice.chars().next_back() {
                        assert!(
                            productive(n, c),
                            "{n}: {quote:?} in {source:?} — range {r:?} ends on {c:?}, dropped by this form"
                        );
                    }
                }
            }
        }
    }
}

/// V2 is a comparison form, not a rewrite: the stored text is never touched.
#[test]
fn v2_does_not_alter_the_text_it_compares() {
    let source = "\u{0412}\u{0441}\u{0451} \u{0441}\u{043C}\u{0435}\u{0448}\u{0430}\u{043B}\u{043E}\u{0441}\u{044C}";
    let (verdict, span) = support_span_with(Normaliser::V2, "\u{0432}\u{0441}\u{0435}", source);
    assert_eq!(verdict, Support::Present);
    let span = span.expect("a range");
    // The range points at the text as written, with its `ё` and its capital.
    assert_eq!(&source[span], "\u{0412}\u{0441}\u{0451}");
}
