//! A pinned case fold for text a person types (QG-4, extended by TX-P8 step 3).
//!
//! sq keywords fold ASCII-only, and that stays so. A dossier subject and an intent rule are
//! not keywords: they are names and phrases in five languages, and "naomi" must find "Naomi"
//! as "ноеминь" must find "Ноеминь". `char::to_lowercase` would do it with Unicode's tables,
//! which change between Unicode versions and are not what the other implementations of this
//! layer will ship, so the fold is written out here instead: the scripts the intent table's
//! languages use, as fixed code-point arithmetic. Anything outside them is left as it is, so a
//! name in another script still matches itself exactly.
//!
//! Covered: ASCII; Latin-1 Supplement capitals U+00C0–U+00DE (not U+00D7 ×); Latin
//! Extended-A U+0100–U+017F (the pairs, with U+0130 İ to `i` and U+0178 Ÿ to ÿ); Greek
//! U+0391–U+03AB (not U+03A2); Cyrillic U+0400–U+042F. One char in, one char out, so a fold
//! never changes a string's char count.

/// `c`, lowercased by the pinned table; `c` itself when the table does not cover it.
pub fn lower(c: char) -> char {
    let u = c as u32;
    let l = match u {
        0x41..=0x5A => u + 0x20,
        0xC0..=0xDE if u != 0xD7 => u + 0x20,
        0x0130 => 0x69,
        0x0178 => 0xFF,
        0x0139..=0x0148 | 0x0179..=0x017E if u % 2 == 1 => u + 1,
        0x0100..=0x0137 | 0x014A..=0x0177 if u % 2 == 0 => u + 1,
        0x0391..=0x03AB if u != 0x03A2 => u + 0x20,
        0x0400..=0x040F => u + 0x50,
        0x0410..=0x042F => u + 0x20,
        _ => u,
    };
    char::from_u32(l).unwrap_or(c)
}

/// `s`, every char through [`lower`].
pub fn fold(s: &str) -> String {
    s.chars().map(lower).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_five_languages_fold_and_nothing_else_moves() {
        assert_eq!(fold("Naomi"), "naomi");
        assert_eq!(fold("ÉTÉ À"), "été à");
        assert_eq!(fold("Ноеминь ЁЖ"), "ноеминь ёж");
        assert_eq!(fold("ΣΩ"), "σω");
        assert_eq!(fold("Łódź Ŝ"), "łódź ŝ");
        assert_eq!(fold("İ Ÿ ×"), "i ÿ ×");
        // Already lower, digits, punctuation and other scripts are fixed points.
        for s in ["naomi", "ß", "ĸ", "ŉ", "日本", "١٢", "?¿"] {
            assert_eq!(fold(s), s);
        }
    }

    #[test]
    fn a_fold_is_idempotent_and_keeps_the_char_count() {
        for c in (0u32..0x0500).filter_map(char::from_u32) {
            assert_eq!(lower(lower(c)), lower(c), "U+{:04X}", c as u32);
        }
        let s = "ÀÉÎÕÜ ĀĒĪŌŪ ΑΒΓ АБВ";
        assert_eq!(fold(s).chars().count(), s.chars().count());
    }
}
