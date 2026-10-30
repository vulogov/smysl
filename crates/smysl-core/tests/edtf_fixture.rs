//! `fixtures/library/edtf/cases.json` — what this build accepts as EDTF, and what it refuses.
//!
//! The acceptance criterion of TX-P3 step 1, in the form another implementation can read. Two
//! halves, and the second is the one that is easy to leave out: **refusals**. EDTF level 1 has
//! more than one way to spell some values and this format accepts exactly one of each, because
//! `published` and a dating's absolute value are both inside an identity — so a producer that
//! wrote `+1984` or `1984-6-2` would hash a different byte string for the same date. A fixture
//! that listed only the accepted forms would let a second implementation pass while accepting
//! all of them.
//!
//! The generator is the test, as in `gen_library_fixtures.rs`: it rebuilds the file from the
//! parser's own tables and compares, so the fixture cannot drift from the code. `SMYSL_BLESS=1`
//! rewrites it, which should be a decision.

use std::path::{Path, PathBuf};

use smysl_core::edtf;

fn path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/library/edtf/cases.json")
}

/// Every form this build accepts, with the level that defines it and what the value means.
///
/// The `kind` column is what a reader has to agree about beyond "it parses": a date, a
/// date-time or an interval, and for an interval whether an end is open (`..`) or unknown
/// (empty). Those two are different claims and a parser that collapsed them would still round
/// trip every string here.
const ACCEPTED: &[(&str, u8, &str, &str)] = &[
    ("1984", 0, "date", "a year"),
    ("1984-06", 0, "date", "a year and a month"),
    ("1984-06-02", 0, "date", "a full date"),
    ("0001-01-01", 0, "date", "the first year of the era"),
    ("2024-02-29", 0, "date", "a leap day"),
    (
        "0000-02-29",
        0,
        "date",
        "year zero exists and is a leap year",
    ),
    (
        "1984-06-02T09:30:01",
        0,
        "datetime",
        "a local time, naming no instant",
    ),
    ("1984-06-02T09:30:01Z", 0, "datetime", "an instant in UTC"),
    (
        "1984-06-02T09:30:01+05:30",
        0,
        "datetime",
        "an offset east of UTC",
    ),
    (
        "1984-06-02T09:30:01-04:00",
        0,
        "datetime",
        "an offset west of UTC",
    ),
    ("1984/1985", 0, "interval", "both ends given"),
    (
        "1984-06-02/1984-06-20",
        0,
        "interval",
        "both ends at day precision",
    ),
    ("1984?", 1, "date", "uncertain: the source is not sure"),
    (
        "1984-06~",
        1,
        "date",
        "approximate: the date is near this one",
    ),
    ("1984-06-02%", 1, "date", "both uncertain and approximate"),
    ("1984~/2004~", 1, "interval", "a qualifier on each end"),
    ("201X", 1, "date", "a decade: one unspecified digit"),
    ("20XX", 1, "date", "a century"),
    ("2XXX", 1, "date", "a millennium"),
    ("XXXX", 1, "date", "a year with no known digit"),
    ("1984-XX", 1, "date", "a month the source does not give"),
    ("1984-XX-02", 1, "date", "the second of an unknown month"),
    ("1984-06-XX", 1, "date", "a day the source does not give"),
    ("1984-XX-XX", 1, "date", "some day in 1984"),
    (
        "201X-06-02",
        1,
        "date",
        "a masked year with a full month and day",
    ),
    ("1984/..", 1, "interval", "open end: no bound"),
    ("../1984", 1, "interval", "open start"),
    (
        "1984/",
        1,
        "interval",
        "unknown end: there is one, unrecorded",
    ),
    ("/1984", 1, "interval", "unknown start"),
    ("-0999", 1, "date", "a negative year"),
    ("-0999-06-02", 1, "date", "a negative year at day precision"),
    ("Y170000002", 1, "date", "a year four digits cannot hold"),
    ("Y-170000002", 1, "date", "a negative long year"),
    ("2001-21", 1, "date", "spring"),
    ("2001-24", 1, "date", "winter"),
    (
        "2005/1984",
        0,
        "interval",
        "reversed: well formed, and empty as a meaning",
    ),
];

/// What this build refuses, and why. Three kinds, and the third is the point of the file.
///
/// **Malformed** (`1984-13`), **level 2** (`[1667,1668]`, `1X34`, `2004-06~-11`) and **a second
/// spelling of a value already accepted** (`+1984`, `Y1984`, `1984-6-2`, `-00:00`). The third
/// kind would parse perfectly well under any EDTF library. Accepting it would mean one date
/// with two byte strings, and therefore one unit with two uids.
const REFUSED: &[(&str, &str)] = &[
    ("", "a value with no characters"),
    ("1984-13", "month 13"),
    ("1984-00", "month 00"),
    ("1984-20", "a level-2 sub-year division"),
    ("1984-06-00", "day 00"),
    ("1984-06-32", "day 32"),
    ("2019-02-30", "February has no 30th"),
    ("2023-02-29", "2023 is not a leap year"),
    ("1900-02-29", "1900 is not a leap year"),
    ("1984-06-02-03", "four fields"),
    ("1984--06", "an empty field"),
    ("198", "three digits"),
    ("19845", "five digits with no `Y`"),
    ("+1984", "a leading `+`: a second spelling of `1984`"),
    (
        "1984-6-2",
        "not zero-padded: a second spelling of `1984-06-02`",
    ),
    (
        "Y1984",
        "four digits need no `Y`: a second spelling of `1984`",
    ),
    ("Y0170000002", "a leading zero in a long year"),
    ("Y1700", "fewer than five digits after `Y`"),
    ("1X34", "an interior `X` is level 2"),
    ("1984-X6", "an interior `X` in a month"),
    ("2001-21-05", "a season has no days"),
    ("1984-06-02T09:30", "a time of day with no seconds"),
    ("1984-06-02t09:30:01", "a lowercase `t`"),
    ("1984-06-02T09:30:01z", "a lowercase `z`"),
    ("1984-06-02T24:00:00", "hour 24"),
    (
        "1984-06-02T23:59:60",
        "a leap second has no millisecond instant",
    ),
    ("1984-06-02T09:30:01-00:00", "`-00:00` is `Z`"),
    ("1984-06-02T09:30:01+0530", "an offset with no colon"),
    ("1984T09:30:01", "a time of day on a year"),
    ("1984?-06", "a qualifier mid-value"),
    ("2004-06~-11", "a level-2 per-component qualifier"),
    ("1984/1985/1986", "three ends"),
    ("/", "an interval with no date at either end"),
    ("../..", "two open ends"),
    ("..", "an open end on its own"),
    ("1984-06-02T09:30:01/1985", "a date-time as an interval end"),
    ("[1667,1668]", "a level-2 set"),
    ("1984-06-02^", "`^` is not a qualifier"),
    ("1984 ", "a trailing space"),
];

fn kind_of(v: &edtf::Edtf) -> String {
    match v {
        edtf::Edtf::Date(_) => "date".to_string(),
        edtf::Edtf::DateTime(_) => "datetime".to_string(),
        edtf::Edtf::Interval { .. } => "interval".to_string(),
        // `Edtf` is `#[non_exhaustive]`, so a form a later level adds lands here rather than
        // failing to compile this fixture out of existence.
        _ => "other".to_string(),
    }
}

fn json() -> String {
    // `json_escape` quotes as well as escapes, so the format strings below do not.
    let esc = |s: &str| smysl_core::types::json_escape(s);
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(
        "  \"purpose\": \"EDTF levels 0 and 1 as SMYSL-2.3 A-2.3 requires them: what \
         `source.published` (source key 4) and a dating's absolute value (record 17, key 2 \
         entry 0) may hold. `accepted` lists every form, with the level that defines it and \
         whether it is a date, a date-time or an interval; each must re-print as itself, \
         because both fields are inside an identity and one value has one spelling. `refused` \
         lists what must not parse — malformed values, level-2 syntax, and second spellings \
         of accepted values such as `+1984` or `1984-6-2`, which are refused for the same \
         reason: two byte strings for one date are two uids for one claim.\",\n",
    );
    out.push_str("  \"accepted\": [\n");
    for (i, (text, level, kind, note)) in ACCEPTED.iter().enumerate() {
        let parsed = edtf::parse(text).expect("the table only holds accepted values");
        assert_eq!(&kind_of(&parsed), kind, "{text}");
        assert_eq!(
            &parsed.to_string(),
            text,
            "{text} does not re-print as itself"
        );
        out.push_str(&format!(
            "    {{ \"text\": {}, \"level\": {level}, \"kind\": \"{kind}\", \
             \"note\": {} }}{}\n",
            esc(text),
            esc(note),
            if i + 1 == ACCEPTED.len() { "" } else { "," },
        ));
    }
    out.push_str("  ],\n");
    out.push_str("  \"refused\": [\n");
    for (i, (text, why)) in REFUSED.iter().enumerate() {
        out.push_str(&format!(
            "    {{ \"text\": {}, \"why\": {} }}{}\n",
            esc(text),
            esc(why),
            if i + 1 == REFUSED.len() { "" } else { "," },
        ));
    }
    out.push_str("  ],\n");
    out.push_str(
        "  \"diagnostic\": \"SMY-E410, raised where a malformed value is written: the surface \
         parser refuses one in `@date` or in `source { published: … }`. A CBOR decoder does \
         not — the value is inside the uid, so a record that arrived must leave again byte for \
         byte, and the `Time` check pass reports it instead.\"\n",
    );
    out.push_str("}\n");
    out
}

#[test]
fn the_edtf_fixture_matches_this_build() {
    let want = json();
    let p = path();
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::create_dir_all(p.parent().expect("a parent")).expect("a directory");
        std::fs::write(&p, want.as_bytes()).expect("written");
        return;
    }
    let have = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    assert_eq!(
        have, want,
        "fixtures/library/edtf/cases.json no longer matches what this build accepts; \
         `SMYSL_BLESS=1 cargo test -p smysl-core --test edtf_fixture` rewrites it"
    );
}

/// Nothing in `refused` parses, which the generator does not check for itself.
///
/// It cannot: a generator that asserted the refusals would be asserting them against the same
/// parser that produced the file. This asserts the half the file is *for*.
#[test]
fn nothing_the_fixture_refuses_parses() {
    for (text, why) in REFUSED {
        assert!(
            edtf::parse(text).is_err(),
            "`{text}` should be refused: {why}"
        );
    }
}

/// The two lists do not overlap, and neither repeats itself.
#[test]
fn the_two_lists_are_disjoint_and_free_of_duplicates() {
    let accepted: std::collections::BTreeSet<&str> = ACCEPTED.iter().map(|c| c.0).collect();
    let refused: std::collections::BTreeSet<&str> = REFUSED.iter().map(|c| c.0).collect();
    assert_eq!(accepted.len(), ACCEPTED.len(), "a duplicate in `accepted`");
    assert_eq!(refused.len(), REFUSED.len(), "a duplicate in `refused`");
    assert!(
        accepted.is_disjoint(&refused),
        "a value is both accepted and refused"
    );
}
