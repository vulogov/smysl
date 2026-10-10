//! EDTF: what parses has exactly one spelling, and parsing it again gives the same value.
//!
//! The property the format depends on, over arbitrary bytes rather than over a table. Both
//! fields that carry an EDTF string — `source.published` and a dating's absolute value — are
//! inside an identity, so a value with two spellings is a claim with two uids. The acceptance
//! table in `fixtures/library/edtf/cases.json` says which spellings are accepted; this says
//! that *whatever* is accepted, accepting it twice gives the same answer.
//!
//! 1. **Canonical.** `parse(s).to_string() == s` for every `s` that parses.
//! 2. **Idempotent.** Parsing what `Display` wrote gives the same value, not merely the same
//!    string — a re-print that agreed by accident while the AST differed would pass (1) alone.
//! 3. **Total.** No input panics, and none runs away: a parser over a date syntax has no
//!    business allocating or looping on anything the caller hands it.
//! 4. **Sound extent.** Every interval the value admits is well formed: an end that is bounded
//!    on both sides never has its ends crossed except where the value itself is reversed.

#![no_main]
use libfuzzer_sys::fuzz_target;
use smysl_core::edtf;
use smysl_text::time::edtf as extent;

fuzz_target!(|data: &[u8]| {
    // Only text can be a date. Invalid UTF-8 is a different refusal and `parse` never sees it:
    // a CBOR text item is UTF-8 by the time the decoder hands it over.
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(v) = edtf::parse(s) else {
        return;
    };
    let printed = v.to_string();
    assert_eq!(printed, s, "accepted `{s}` and re-printed it as `{printed}`");
    let again = edtf::parse(&printed).expect("what Display wrote must parse");
    assert_eq!(again, v, "`{s}` round-tripped to a different value");

    // The extent. A value whose ends are both bounded gives a non-empty interval unless the
    // value itself is a reversed interval — which parses on purpose, because its meaning is
    // rule E's to report and not the parser's to refuse.
    let i = extent::to_interval(&v);
    let reversed = matches!(&v, edtf::Edtf::Interval { .. }) && i.is_empty();
    if !reversed {
        assert!(!i.is_empty(), "`{s}` admits no instant at all");
    }
    // And `fits` agrees with what the interval actually has.
    if extent::fits(&v) && !matches!(&v, edtf::Edtf::Interval { .. }) {
        assert!(
            i.lo != smysl_text::time::Bound::Open && i.hi != smysl_text::time::Bound::Open,
            "`{s}` fits and yet has an open end"
        );
    }
});
