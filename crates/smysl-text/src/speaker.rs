//! Pseudonyms: who said a thing, written so that the corpus does not say who they are.
//!
//! A chat export names people. The export's own identifier for a person is what makes the
//! messages of one person collectable — which is the whole value of the speaker field and the
//! whole privacy problem with it. SMYSL-2.4 §3.2 answers both at once: the speaker in a segment
//! row becomes `spk:` + 26 base32 characters of **keyed** BLAKE3 over the platform's user id,
//! so the same person is the same pseudonym within one library and nothing outside it.
//!
//! Keyed and not plain: a platform user id is a short string out of a small space, so an
//! unkeyed hash of it is a lookup table away from being the id itself. The key is the secret
//! that makes the mapping one-way to anyone who does not hold it, and [`crate::secrets`] is
//! where it is kept.
//!
//! # What this covers, and what it does not
//!
//! It covers the **reading**: the `speaker` of each row. It does not touch the text, and that
//! is deliberate rather than a gap — a name inside the prose (`thanks Alice`, a mention, a
//! signature) is still there afterwards. Two consequences, both worth stating:
//!
//! - **A pseudonymised library and a plain one hold the same text.** The tid of every part is
//!   the same either way, because pseudonymisation changes no byte a tid is taken over. Only
//!   the reading differs, so only the rdid moves. That is what makes `--pseudonymise` a
//!   decision about the catalog rather than a different ingest of a different document.
//! - **Names in the prose are a different instrument.** `text redact` and rule Z (TX-P2 step 4)
//!   are what remove them, and they have to be, because removing a name from the text *does*
//!   change the tid and therefore has to be recorded as a redaction rather than a setting.
//!
//! Readers help by putting every person-identifying value they carry in the `speaker` field
//! and dropping the rest. Telegram's `forwarded_from` is the case that forced the rule: it is a
//! second person's display name on somebody else's message, and there is no user id beside it
//! to key — so it is dropped and the read is marked lossy, rather than carried in a field
//! pseudonymisation does not reach.

use smysl_core::ids;

use crate::reading::Segment;

/// The prefix a pseudonymous speaker is written with.
pub const PREFIX: &str = "spk:";

/// Characters after the prefix: 130 bits, as §2.1's short form has.
pub const CHARS: usize = ids::Uid::SHORT_CHARS;

/// The key a library's pseudonyms are derived under.
///
/// Thirty-two bytes, and no way to read them back out. `from_bytes` in, nothing out: the only
/// operations are deriving a pseudonym and storing the key through [`crate::secrets`], which
/// is in this crate and uses the private field. A getter would exist for exactly one purpose —
/// printing the key somewhere — and the field being private is cheaper than a rule about it.
///
/// [`Debug`] is hand-written for the same reason. A derived one would put the key in every
/// `{:?}` of every structure that holds one, which includes an `AddSpec` in a diagnostic.
#[derive(Clone, PartialEq, Eq)]
pub struct Key([u8; 32]);

impl Key {
    /// Bytes in a key, and the length of the key file.
    pub const BYTES: usize = 32;

    pub const fn from_bytes(bytes: [u8; 32]) -> Key {
        Key(bytes)
    }

    /// The key's own bytes, for [`crate::secrets`] to write. Deliberately not public.
    pub(crate) const fn bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The pseudonym of a platform user id under this key.
    ///
    /// The id is taken as the source spelled it — `user123456`, `U024BE7LH`, a phone number —
    /// and not normalised. Normalising would be this crate deciding that two spellings are one
    /// person, which is a claim about the platform rather than about the text.
    ///
    /// Not scoped by reader. Two platforms do not share an id space in practice, and scoping
    /// would give one person two pseudonyms in a library that holds both their exports — which
    /// is the opposite of what the field is for.
    pub fn pseudonym(&self, platform_id: &str) -> String {
        let digest = blake3::keyed_hash(&self.0, platform_id.as_bytes());
        let mut out = String::with_capacity(PREFIX.len() + CHARS);
        out.push_str(PREFIX);
        out.push_str(&ids::base32(digest.as_bytes(), CHARS));
        out
    }
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Key(<{} bytes, not shown>)", Key::BYTES)
    }
}

/// Whether a speaker is already a pseudonym this crate wrote.
///
/// Checked by shape rather than remembered, because the question is asked of a reading read
/// back from a store, where there is nothing to remember. The alphabet is checked too: `spk:`
/// followed by 26 characters that are not base32 is not a pseudonym, and treating it as one
/// would mean a hostile manifest could keep a plain name out of pseudonymisation by prefixing
/// it.
pub fn is_pseudonym(speaker: &str) -> bool {
    match speaker.strip_prefix(PREFIX) {
        Some(body) => {
            body.chars().count() == CHARS
                && body
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
        }
        None => false,
    }
}

/// Replace every speaker in a reading with its pseudonym, and say how many rows moved.
///
/// Idempotent: a row whose speaker is already a pseudonym is left exactly as it is. Without
/// that, pseudonymising twice would hash the pseudonym and the second pass would break the
/// link between a person's messages — and `text append` (TX-P2 step 3) adds rows to a reading
/// that has already been through here.
pub fn pseudonymise(rows: &mut [Segment], key: &Key) -> usize {
    let mut changed = 0usize;
    for row in rows {
        if let Some(speaker) = &row.speaker {
            if is_pseudonym(speaker) {
                continue;
            }
            row.speaker = Some(key.pseudonym(speaker));
            changed += 1;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locator::Locator;
    use crate::reading::Level;

    fn key(byte: u8) -> Key {
        Key::from_bytes([byte; 32])
    }

    fn row(speaker: Option<&str>) -> Segment {
        let s = Segment::new(
            0,
            4,
            Level::new("message").expect("a level"),
            Locator::line(1).expect("L1"),
        );
        match speaker {
            Some(who) => s.with_speaker(who),
            None => s,
        }
    }

    /// The shape SMYSL-2.4 §3.2 specifies, and the shape `is_pseudonym` accepts.
    #[test]
    fn a_pseudonym_is_the_prefix_and_twenty_six_base32_characters() {
        let p = key(1).pseudonym("user123456");
        assert!(p.starts_with(PREFIX), "{p}");
        assert_eq!(p.len(), PREFIX.len() + CHARS);
        assert!(is_pseudonym(&p), "{p}");
    }

    /// One person, one pseudonym — which is the only reason the field is worth keeping.
    #[test]
    fn the_same_id_under_the_same_key_is_the_same_pseudonym() {
        assert_eq!(key(7).pseudonym("U024BE7LH"), key(7).pseudonym("U024BE7LH"));
    }

    /// Two keys, two mappings. This is what makes the key the secret rather than the algorithm.
    #[test]
    fn the_same_id_under_two_keys_is_two_pseudonyms() {
        assert_ne!(key(1).pseudonym("U024BE7LH"), key(2).pseudonym("U024BE7LH"));
    }

    #[test]
    fn two_ids_under_one_key_are_two_pseudonyms() {
        assert_ne!(key(1).pseudonym("alice"), key(1).pseudonym("bob"));
    }

    /// Pseudonymising twice is pseudonymising once.
    ///
    /// The case this protects is `text append`: the rows already in the reading have been
    /// through here, the new ones have not, and one pass over all of them has to be correct.
    #[test]
    fn pseudonymising_a_pseudonym_leaves_it_alone() {
        let mut rows = vec![row(Some("alice"))];
        assert_eq!(pseudonymise(&mut rows, &key(3)), 1);
        let once = rows[0].speaker.clone();
        assert_eq!(pseudonymise(&mut rows, &key(3)), 0);
        assert_eq!(rows[0].speaker, once);
    }

    /// A row with no speaker is not given one.
    #[test]
    fn a_row_without_a_speaker_stays_without_one() {
        let mut rows = vec![row(None)];
        assert_eq!(pseudonymise(&mut rows, &key(1)), 0);
        assert!(rows[0].speaker.is_none());
    }

    /// Only the reading moves. The text is untouched, so every tid is unchanged.
    #[test]
    fn pseudonymisation_touches_nothing_but_the_speaker() {
        let mut rows = vec![row(Some("alice")).with_id("msg", "17")];
        let before = rows[0].clone();
        pseudonymise(&mut rows, &key(9));
        let after = &rows[0];
        assert_eq!((after.start, after.end), (before.start, before.end));
        assert_eq!(after.level, before.level);
        assert_eq!(after.locator, before.locator);
        assert_eq!(after.ids, before.ids);
        assert_ne!(after.speaker, before.speaker);
    }

    /// A plain name wearing the prefix is not mistaken for a pseudonym.
    #[test]
    fn the_prefix_alone_does_not_make_a_pseudonym() {
        for not in [
            "spk:alice",
            "spk:",
            "alice",
            // 26 characters, one of them outside the alphabet: `1` is not in base32.
            "spk:abcdefghijklmnopqrstuvwxy1",
        ] {
            assert!(!is_pseudonym(not), "{not}");
        }
    }

    /// The key does not print itself.
    ///
    /// Asserted rather than trusted: a `derive(Debug)` added later would put the key into
    /// every diagnostic that formats a structure holding one, and nothing else would notice.
    #[test]
    fn a_key_does_not_appear_in_its_own_debug_output() {
        let k = Key::from_bytes([0xAB; 32]);
        let shown = format!("{k:?}");
        assert!(!shown.contains("ab"), "{shown}");
        assert!(!shown.contains("171"), "{shown}");
        assert_eq!(shown, "Key(<32 bytes, not shown>)");
    }
}
