//! Identifier scaffolding.
//!
//! SM-P0 defines only what the diagnostic machinery needs: the [`Uid`] newtype and its
//! display forms (§1.2). Hashing (`canonical_uid`), prefix resolution, and the remaining
//! identifier types land with the codec in SM-P1.

use core::fmt;

/// RFC 4648 base32, lowercased. Not base32hex - the alphabet is the standard one.
const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// `uid = BLAKE3-256(det_cbor(core))` (rule P1).
///
/// A `Uid` always carries all 256 bits. The 26-character short form is for interactive
/// display and prefix resolution only; a canonical record carrying a truncated uid is
/// `SMY-E071`, and comparison is always over the full 256 bits.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uid([u8; 32]);

impl Uid {
    /// Characters in the short display form: the first 130 bits, exactly 26 base32 chars.
    pub const SHORT_CHARS: usize = 26;
    /// Characters in the canonical display form: all 256 bits, zero-padded to 260.
    pub const FULL_CHARS: usize = 52;

    pub const fn from_bytes(b: [u8; 32]) -> Uid {
        Uid(b)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub const fn to_bytes(self) -> [u8; 32] {
        self.0
    }

    /// `b3:` + 26 base32 characters (the first 130 bits). Display form; not canonical.
    pub fn short(&self) -> String {
        self.encode(Uid::SHORT_CHARS)
    }

    /// `b3:` + 52 base32 characters (all 256 bits). The canonical text form.
    pub fn canonical(&self) -> String {
        self.encode(Uid::FULL_CHARS)
    }

    fn encode(&self, chars: usize) -> String {
        let mut s = String::with_capacity(3 + chars);
        s.push_str("b3:");
        for i in 0..chars {
            s.push(ALPHABET[five_bits_at(&self.0, i * 5)] as char);
        }
        s
    }
}

/// The 5 bits starting at bit offset `off`, MSB-first, zero-padded past the end.
fn five_bits_at(bytes: &[u8; 32], off: usize) -> usize {
    let mut v = 0usize;
    for k in 0..5 {
        let bit = off + k;
        let set = bit < 256 && (bytes[bit / 8] >> (7 - (bit % 8))) & 1 == 1;
        v = (v << 1) | usize::from(set);
    }
    v
}

/// Display is the short form - the one a human reads in a diagnostic.
impl fmt::Display for Uid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.short())
    }
}

impl fmt::Debug for Uid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Uid({})", self.canonical())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_form_is_prefixed_and_26_chars() {
        let u = Uid::from_bytes([0xAB; 32]);
        let s = u.short();
        assert!(s.starts_with("b3:"));
        assert_eq!(s.len(), 3 + 26);
        assert!(s[3..].bytes().all(|b| ALPHABET.contains(&b)));
    }

    #[test]
    fn canonical_form_is_52_chars() {
        let u = Uid::from_bytes([0x5C; 32]);
        let s = u.canonical();
        assert_eq!(s.len(), 3 + 52);
        assert!(
            s.starts_with(&u.short()),
            "short form is a prefix of canonical"
        );
    }

    #[test]
    fn all_zero_and_all_one_encode_at_the_alphabet_extremes() {
        assert_eq!(
            Uid::from_bytes([0x00; 32]).short(),
            format!("b3:{}", "a".repeat(26))
        );
        assert_eq!(
            Uid::from_bytes([0xFF; 32]).short(),
            format!("b3:{}", "7".repeat(26))
        );
    }

    #[test]
    fn distinct_bytes_give_distinct_short_forms() {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        a[0] = 0x01;
        b[0] = 0x02;
        assert_ne!(Uid::from_bytes(a).short(), Uid::from_bytes(b).short());
    }

    #[test]
    fn short_form_covers_exactly_the_first_130_bits() {
        let base = [0u8; 32];
        let mut flipped = base;
        flipped[17] |= 0b0100_0000; // bit 137, past the 130-bit boundary
        assert_eq!(
            Uid::from_bytes(base).short(),
            Uid::from_bytes(flipped).short()
        );
        assert_ne!(
            Uid::from_bytes(base).canonical(),
            Uid::from_bytes(flipped).canonical()
        );

        let mut inside = base;
        inside[16] |= 0b0100_0000; // bit 129, the last bit inside the boundary
        assert_ne!(
            Uid::from_bytes(base).short(),
            Uid::from_bytes(inside).short()
        );
    }

    #[test]
    fn ordering_is_over_raw_bytes() {
        let a = Uid::from_bytes([0x00; 32]);
        let mut hi = [0u8; 32];
        hi[0] = 0x01;
        assert!(a < Uid::from_bytes(hi));
    }

    #[test]
    fn round_trips_through_bytes() {
        let b = [0x3A; 32];
        assert_eq!(Uid::from_bytes(b).to_bytes(), b);
        assert_eq!(Uid::from_bytes(b).as_bytes(), &b);
    }
}
