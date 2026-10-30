//! The canonical book vocabulary the scripture readers agree on.
//!
//! Three of the six TX-P1 readers read scripture, and each source names its books its own way:
//! USFM writes `\id EXO`, Zefania numbers them `bnumber="2"`, and an OSIS file carries
//! `osisID="Exod.20.1"`. If each reader emitted its source's own spelling, the same verse in
//! three editions would get three locators, and nothing downstream could align them — which is
//! GE-T1's whole measurement.
//!
//! So there is one vocabulary and two mappings into it. The vocabulary is **OSIS**: `Gen`,
//! `Exod`, `1John`, `Ps`. It is chosen because it is the only one of the four that is
//! published and standardised rather than local to a file format, because the locator table in
//! SMYSL-2.4 §3.2 is already written in it (`1John.3`, `Ps.136`), and because it is what the
//! `osis/1` reader gets for free — one reader needing no mapping is one fewer table to be
//! wrong.
//!
//! # The spike's alignment table is keyed in something else
//!
//! §3.2 says the canonical form is OSIS-shaped and cites the spike's `in/align.tsv` as keyed
//! by it, "(`Ex.20.1`)". Those two statements do not agree: the spike keys its rows `Ex.20.1`,
//! `Ecc.1`, `1Ki.3` and `Jas.2`, which is an ad-hoc abbreviation set — OSIS writes `Exod`,
//! `Eccl`, `1Kgs` and `Jas`, so three of those four differ. The spike is an input to an
//! experiment and not a corpus, so the table is the thing that moves: GE-T1 either re-keys it
//! or carries a mapping, and either way the readers are not the place to encode one
//! experiment's spelling. Recorded here because the claim in §3.2 reads as though the question
//! were already settled.

/// Every book, in canonical order: its OSIS name and the USFM code that means it.
///
/// The index is the Zefania book number minus one, which is what makes this one table rather
/// than three — a second table would be a second place for the 66 rows to disagree.
pub const BOOKS: &[(&str, &str)] = &[
    // The Old Testament, 39.
    ("Gen", "GEN"),
    ("Exod", "EXO"),
    ("Lev", "LEV"),
    ("Num", "NUM"),
    ("Deut", "DEU"),
    ("Josh", "JOS"),
    ("Judg", "JDG"),
    ("Ruth", "RUT"),
    ("1Sam", "1SA"),
    ("2Sam", "2SA"),
    ("1Kgs", "1KI"),
    ("2Kgs", "2KI"),
    ("1Chr", "1CH"),
    ("2Chr", "2CH"),
    ("Ezra", "EZR"),
    ("Neh", "NEH"),
    ("Esth", "EST"),
    ("Job", "JOB"),
    ("Ps", "PSA"),
    ("Prov", "PRO"),
    ("Eccl", "ECC"),
    ("Song", "SNG"),
    ("Isa", "ISA"),
    ("Jer", "JER"),
    ("Lam", "LAM"),
    ("Ezek", "EZK"),
    ("Dan", "DAN"),
    ("Hos", "HOS"),
    ("Joel", "JOL"),
    ("Amos", "AMO"),
    ("Obad", "OBA"),
    ("Jonah", "JON"),
    ("Mic", "MIC"),
    ("Nah", "NAM"),
    ("Hab", "HAB"),
    ("Zeph", "ZEP"),
    ("Hag", "HAG"),
    ("Zech", "ZEC"),
    ("Mal", "MAL"),
    // The New Testament, 27.
    ("Matt", "MAT"),
    ("Mark", "MRK"),
    ("Luke", "LUK"),
    ("John", "JHN"),
    ("Acts", "ACT"),
    ("Rom", "ROM"),
    ("1Cor", "1CO"),
    ("2Cor", "2CO"),
    ("Gal", "GAL"),
    ("Eph", "EPH"),
    ("Phil", "PHP"),
    ("Col", "COL"),
    ("1Thess", "1TH"),
    ("2Thess", "2TH"),
    ("1Tim", "1TI"),
    ("2Tim", "2TI"),
    ("Titus", "TIT"),
    ("Phlm", "PHM"),
    ("Heb", "HEB"),
    ("Jas", "JAS"),
    ("1Pet", "1PE"),
    ("2Pet", "2PE"),
    ("1John", "1JN"),
    ("2John", "2JN"),
    ("3John", "3JN"),
    ("Jude", "JUD"),
    ("Rev", "REV"),
];

/// The OSIS name for a USFM book code, uppercase as USFM writes it.
///
/// `None` for a code this table does not hold — the deuterocanon, a custom book, or a typo.
/// A reader turns that into a refusal naming the code rather than guessing, because a guessed
/// book name is a locator that points somewhere real and wrong.
pub fn from_usfm(code: &str) -> Option<&'static str> {
    BOOKS
        .iter()
        .find(|(_, usfm)| *usfm == code)
        .map(|(osis, _)| *osis)
}

/// The OSIS name for a 1-based book number, as Zefania's `bnumber` gives it.
pub fn from_number(n: u64) -> Option<&'static str> {
    let index = usize::try_from(n.checked_sub(1)?).ok()?;
    BOOKS.get(index).map(|(osis, _)| *osis)
}

/// Whether this is a book name in the canonical vocabulary.
///
/// Case-sensitive, like every other identity comparison in this format: `GEN` and `Gen` are
/// not the same book name, and accepting both would put both into a corpus.
pub fn is_osis(name: &str) -> bool {
    BOOKS.iter().any(|(osis, _)| *osis == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locator::Locator;

    #[test]
    fn the_table_is_the_protestant_canon_in_order() {
        assert_eq!(BOOKS.len(), 66);
        assert_eq!(from_number(1), Some("Gen"));
        assert_eq!(from_number(39), Some("Mal"), "the Old Testament ends at 39");
        assert_eq!(
            from_number(40),
            Some("Matt"),
            "the New Testament starts at 40"
        );
        assert_eq!(from_number(66), Some("Rev"));
        assert_eq!(from_number(0), None, "bnumber is 1-based");
        assert_eq!(from_number(67), None);
    }

    /// No two rows claim the same name or the same code.
    ///
    /// A duplicate would make one of the two unreachable through `from_usfm`, and which one
    /// depends on table order — the kind of defect that shows up as a single missing book in a
    /// corpus years later.
    #[test]
    fn every_name_and_every_code_appears_once() {
        let mut names: Vec<&str> = BOOKS.iter().map(|(osis, _)| *osis).collect();
        let mut codes: Vec<&str> = BOOKS.iter().map(|(_, usfm)| *usfm).collect();
        names.sort_unstable();
        codes.sort_unstable();
        let (n, c) = (names.len(), codes.len());
        names.dedup();
        codes.dedup();
        assert_eq!(names.len(), n, "duplicate OSIS name");
        assert_eq!(codes.len(), c, "duplicate USFM code");
    }

    #[test]
    fn usfm_codes_map_to_the_names_the_rfc_uses() {
        assert_eq!(from_usfm("GEN"), Some("Gen"));
        assert_eq!(from_usfm("EXO"), Some("Exod"));
        assert_eq!(from_usfm("1JN"), Some("1John"));
        assert_eq!(from_usfm("PSA"), Some("Ps"));
        assert_eq!(from_usfm("gen"), None, "USFM codes are uppercase");
        assert_eq!(
            from_usfm("TOB"),
            None,
            "the deuterocanon is not in this table"
        );
    }

    /// Every book name is a locator head this crate's grammar accepts.
    ///
    /// The vocabulary and the grammar were decided in different steps, and the names that
    /// would catch a disagreement are the awkward ones: `1John` starts with a digit, which is
    /// what `L412` also does, and `Song` is a word that could have been a level.
    #[test]
    fn every_book_name_parses_as_a_canonical_locator() {
        for (osis, _) in BOOKS {
            let text = format!("{osis}.1.1");
            let parsed: Locator = text.parse().unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!(parsed.to_string(), text);
        }
    }

    #[test]
    fn the_vocabulary_check_is_case_sensitive() {
        assert!(is_osis("Gen"));
        assert!(is_osis("1John"));
        assert!(!is_osis("gen"));
        assert!(!is_osis("GEN"));
        assert!(!is_osis("Tob"));
    }
}
