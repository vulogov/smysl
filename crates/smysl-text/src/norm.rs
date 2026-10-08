//! Normalisation: the one gate between bytes somebody gave us and bytes a tid may name.
//!
//! Four transformations, in this order (SMYSL-2.4 §3.1): UTF-8 validation, BOM strip, CRLF and
//! lone CR to LF, NFC. Order matters in one place — the BOM is stripped before NFC, because
//! U+FEFF is not combining and NFC would keep it, and a leading BOM is a byte-order mark in
//! one file and a zero-width no-break space in the middle of another. Only the leading one is
//! removed, so a document that uses U+FEFF as text keeps it.
//!
//! # Why a newtype
//!
//! A tid is `BLAKE3-256(0x0F ‖ the part's normalised bytes)`. Nothing in that formula says
//! what happens if the bytes are not normalised: the hash still computes, the identity is
//! still well formed, and two libraries that received the same document with different line
//! endings name it differently and never find out. [`Normalised`] is the answer, and it is a
//! type rather than a convention because a convention is something a future reader forgets at
//! four in the afternoon.
//!
//! `smysl-core` cannot carry this type: `Tid::from_normalised_bytes` takes bytes, because the
//! decoder has to be able to build a part text from whatever was on the wire, including a
//! part text whose bytes are *wrong* (that is what `SMY-E446` reports). So the guarantee is
//! structural rather than enforced by the signature, and
//! `tests/norm_is_the_only_gate.rs` keeps it: inside this crate the bytes-taking constructor
//! is named in one file, this one, and the test fails on the next call site wherever somebody
//! adds it.

use smysl_core::error::LibError;
use smysl_core::ids::Tid;
use unicode_normalization::{is_nfc, UnicodeNormalization};

/// Text that has been through all four normalisations, and therefore may name a tid.
///
/// Clone is cheap enough (one `String`) and the type is immutable: every way to obtain one
/// goes through [`Normalised::new`] or [`Normalised::of`], and there is no way to edit
/// one afterwards. A slice of one is not one — see [`Normalised::slice`] for what is and is
/// not safe to carve out.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Normalised(String);

/// The leading byte-order mark, as UTF-8.
const BOM: &str = "\u{FEFF}";

impl Normalised {
    /// Normalise bytes, or refuse them.
    ///
    /// The only refusal is invalid UTF-8, reported with the byte offset where the decoder
    /// gave up — which is the offset a human needs to look at the file, and the one piece of
    /// information "invalid encoding" never carries.
    pub fn new(input: &[u8]) -> Result<Normalised, LibError> {
        let s = std::str::from_utf8(input).map_err(|e| LibError::NotText {
            at: e.valid_up_to(),
        })?;
        Ok(Normalised::of(s))
    }

    /// Normalise text that is already valid UTF-8, which cannot fail.
    ///
    /// Named `of` rather than `from_str`, which would read as [`std::str::FromStr`] and is
    /// not: that trait is fallible and this is not, and a reader who saw `from_str` would
    /// reasonably expect `"x".parse()` to work.
    pub fn of(input: &str) -> Normalised {
        let stripped = input.strip_prefix(BOM).unwrap_or(input);
        let lf = newlines_to_lf(stripped);
        let nfc = if is_nfc(&lf) {
            lf
        } else {
            lf.nfc().collect::<String>()
        };
        Normalised(nfc)
    }

    /// Wrap text that is normalised already.
    ///
    /// For decoded part texts: the bytes came off the wire, the tid is what names them, and
    /// re-normalising would be a second opinion about bytes that are already addressed by
    /// content. `None` when the text is not in fact normalised, so this is a check and not a
    /// promise the caller makes.
    pub fn already(text: &str) -> Option<Normalised> {
        let n = Normalised::of(text);
        if n.as_str() == text {
            Some(n)
        } else {
            None
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn into_string(self) -> String {
        self.0
    }

    /// The identity of these bytes as a part (A-3).
    pub fn tid(&self) -> Tid {
        Tid::from_normalised_bytes(self.as_bytes())
    }

    /// A byte range of this text, as its own normalised value.
    ///
    /// `None` when the range is out of bounds, lands inside a character, or **is not itself
    /// normalised**. The last one is not paranoia: NFC is not closed under slicing. Cutting
    /// between a base character and its combining mark leaves two pieces, each valid UTF-8,
    /// where the second begins with a combining mark that NFC would have composed into the
    /// character now in the first piece. Parts are cut at structure boundaries, where this
    /// never happens — so the check costs nothing in practice and refuses exactly the case
    /// that would give two libraries different tids for the same part.
    pub fn slice(&self, range: std::ops::Range<usize>) -> Option<Normalised> {
        let text = self.0.get(range)?;
        Normalised::already(text)
    }
}

impl std::fmt::Display for Normalised {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// CRLF and lone CR to LF, in one pass.
///
/// A lone CR is a line ending too — classic Mac OS files, and more often a file that has been
/// through a tool that half-converted it. Mapping it to LF rather than refusing it is the
/// choice that costs nothing: there is no document in which a bare CR means something other
/// than "line break", and refusing would reject readable text over a byte nobody chose.
fn newlines_to_lf(s: &str) -> String {
    if !s.contains('\r') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leading_bom_is_removed_and_a_later_one_is_not() {
        assert_eq!(Normalised::of("\u{FEFF}text").as_str(), "text");
        assert_eq!(
            Normalised::of("a\u{FEFF}b").as_str(),
            "a\u{FEFF}b",
            "U+FEFF inside a document is a zero-width no-break space, not a mark"
        );
    }

    #[test]
    fn every_line_ending_becomes_lf() {
        assert_eq!(Normalised::of("a\r\nb").as_str(), "a\nb");
        assert_eq!(Normalised::of("a\rb").as_str(), "a\nb");
        assert_eq!(Normalised::of("a\nb").as_str(), "a\nb");
        assert_eq!(Normalised::of("a\r\n\r\nb").as_str(), "a\n\nb");
        assert_eq!(Normalised::of("a\r").as_str(), "a\n");
    }

    #[test]
    fn decomposed_input_is_composed() {
        // "é" as e + U+0301.
        let nfd = "cafe\u{301}";
        let n = Normalised::of(nfd);
        assert_eq!(n.as_str(), "café");
        assert!(is_nfc(n.as_str()));
        assert_eq!(n.len(), 5, "composed é is two bytes, not three");
    }

    #[test]
    fn invalid_utf8_is_refused_at_the_byte_it_fails_on() {
        let bad = [b'o', b'k', 0xFF, b'!'];
        assert_eq!(Normalised::new(&bad), Err(LibError::NotText { at: 2 }));
    }

    #[test]
    fn normalisation_is_idempotent() {
        for input in [
            "\u{FEFF}a\r\nb\u{301}",
            "plain",
            "",
            "\r",
            "ru\u{0438}\u{0306}",
        ] {
            let once = Normalised::of(input);
            let twice = Normalised::of(once.as_str());
            assert_eq!(once, twice, "{input:?}");
            assert_eq!(Normalised::already(once.as_str()), Some(once));
        }
    }

    #[test]
    fn already_refuses_text_that_is_not_normalised() {
        assert_eq!(Normalised::already("a\r\nb"), None);
        assert_eq!(Normalised::already("cafe\u{301}"), None);
        assert!(Normalised::already("a\nb").is_some());
    }

    #[test]
    fn a_slice_that_would_cut_a_combining_mark_off_its_base_is_refused() {
        let n = Normalised::of("x\u{1100}\u{1161}y");
        // The Hangul jamo pair composes under NFC, so the composed text is x + 가 + y.
        assert_eq!(n.as_str(), "x\u{AC00}y");
        assert!(n.slice(0..1).is_some());
        assert!(n.slice(0..2).is_none(), "cuts into 가");
        assert_eq!(n.slice(1..4).unwrap().as_str(), "\u{AC00}");
    }

    #[test]
    fn a_tid_is_over_the_normalised_bytes() {
        let a = Normalised::of("\u{FEFF}Mark 1:1\r\n");
        let b = Normalised::of("Mark 1:1\n");
        assert_eq!(a.tid(), b.tid());
        assert_eq!(a.tid(), Tid::from_normalised_bytes(b"Mark 1:1\n"));
    }
}
