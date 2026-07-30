//! The strict decoder (§15.4).
//!
//! Every rejection here is a determinism constraint, not a taste. A decoder that repaired
//! a non-shortest integer, or accepted `null` for an absent optional, would make two byte
//! sequences hash to one uid for itself and two for everyone else - which is the one
//! failure mode content addressing cannot survive.

use crate::cbor::{major, F32_HEAD, INDEFINITE, NULL};
use crate::error::{CodecError, NonDetReason};
use crate::ids::Uid;
use crate::types::is_quantised;

type Res<T> = Result<T, CodecError>;

/// A strict, position-tracking CBOR decoder.
#[derive(Debug, Clone)]
pub struct Dec<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Dec<'a> {
    pub fn new(buf: &'a [u8]) -> Dec<'a> {
        Dec { buf, pos: 0 }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    /// Rewind or advance to an absolute offset. Used only to re-read a value that was
    /// already located, never to skip validation.
    pub fn set_position(&mut self, pos: usize) {
        self.pos = pos;
    }

    /// The next byte, without consuming it.
    pub fn peek_byte(&self) -> Res<u8> {
        self.peek()
    }

    /// Consume `n` bytes that a caller has already inspected.
    pub fn advance(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.buf.len());
    }

    /// The major type of the next item, without consuming it.
    pub fn peek_major(&self) -> Res<u8> {
        Ok(self.peek()? >> 5)
    }

    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    pub fn is_done(&self) -> bool {
        self.pos >= self.buf.len()
    }

    fn nondet(&self, reason: NonDetReason) -> CodecError {
        CodecError::NonDeterministic {
            at: self.pos,
            reason,
        }
    }

    fn truncated(&self) -> CodecError {
        CodecError::Truncated { at: self.pos }
    }

    fn malformed(&self) -> CodecError {
        CodecError::MalformedEnvelope { at: self.pos }
    }

    fn peek(&self) -> Res<u8> {
        self.buf
            .get(self.pos)
            .copied()
            .ok_or_else(|| self.truncated())
    }

    fn take(&mut self, n: usize) -> Res<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or_else(|| self.malformed())?;
        let s = self
            .buf
            .get(self.pos..end)
            .ok_or_else(|| self.truncated())?;
        self.pos = end;
        Ok(s)
    }

    /// Read a head, enforcing shortest form and rejecting indefinite lengths.
    pub fn head(&mut self) -> Res<(u8, u64)> {
        let b = self.peek()?;
        let m = b >> 5;
        let ai = b & 0x1F;
        self.pos += 1;

        if ai == INDEFINITE {
            return Err(CodecError::NonDeterministic {
                at: self.pos - 1,
                reason: NonDetReason::IndefiniteLength,
            });
        }

        let arg = match ai {
            0..=23 => ai as u64,
            24 => {
                let v = self.take(1)?[0] as u64;
                if v < 24 {
                    return Err(CodecError::NonDeterministic {
                        at: self.pos - 2,
                        reason: NonDetReason::NonShortestInt,
                    });
                }
                v
            }
            25 => {
                let s = self.take(2)?;
                let v = u16::from_be_bytes([s[0], s[1]]) as u64;
                if v <= 0xFF {
                    return Err(CodecError::NonDeterministic {
                        at: self.pos - 3,
                        reason: NonDetReason::NonShortestInt,
                    });
                }
                v
            }
            26 => {
                let s = self.take(4)?;
                let v = u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as u64;
                if v <= 0xFFFF {
                    return Err(CodecError::NonDeterministic {
                        at: self.pos - 5,
                        reason: NonDetReason::NonShortestInt,
                    });
                }
                v
            }
            27 => {
                let s = self.take(8)?;
                let v = u64::from_be_bytes(s.try_into().expect("8 bytes"));
                if v <= 0xFFFF_FFFF {
                    return Err(CodecError::NonDeterministic {
                        at: self.pos - 9,
                        reason: NonDetReason::NonShortestInt,
                    });
                }
                v
            }
            _ => return Err(self.malformed()),
        };
        Ok((m, arg))
    }

    fn expect(&mut self, want: u8) -> Res<u64> {
        let at = self.pos;
        let (m, arg) = self.head()?;
        if m != want {
            self.pos = at;
            return Err(self.malformed());
        }
        Ok(arg)
    }

    pub fn uint(&mut self) -> Res<u64> {
        self.expect(major::UINT)
    }

    pub fn bytes(&mut self) -> Res<&'a [u8]> {
        let n = self.expect(major::BYTES)? as usize;
        self.take(n)
    }

    /// Read text, rejecting anything that is not NFC (constraint 5).
    pub fn text(&mut self) -> Res<&'a str> {
        let at = self.pos;
        let n = self.expect(major::TEXT)? as usize;
        let raw = self.take(n)?;
        let s = core::str::from_utf8(raw).map_err(|_| CodecError::MalformedEnvelope { at })?;
        if !unicode_normalization::is_nfc(s) {
            return Err(CodecError::NonDeterministic {
                at,
                reason: NonDetReason::NonNfcText,
            });
        }
        Ok(s)
    }

    /// Read a `binary32` float, rejecting `binary16`, `binary64`, and anything not
    /// already quantised to 1/1024 (constraint 4).
    pub fn f32q(&mut self) -> Res<f32> {
        let at = self.pos;
        let b = self.peek()?;
        if b != F32_HEAD {
            self.pos += 1;
            return Err(CodecError::Float { at });
        }
        self.pos += 1;
        let s = self.take(4)?;
        let v = f32::from_be_bytes(s.try_into().expect("4 bytes"));
        if !is_quantised(v) {
            return Err(CodecError::Float { at });
        }
        Ok(v)
    }

    /// Read a 32-byte uid. A shorter byte string is a display abbreviation and is
    /// `SMY-E071` rather than a shorter identity.
    pub fn uid(&mut self) -> Res<Uid> {
        let at = self.pos;
        let b = self.bytes()?;
        match <[u8; 32]>::try_from(b) {
            Ok(a) => Ok(Uid::from_bytes(a)),
            Err(_) => Err(CodecError::TruncatedUid { at, len: b.len() }),
        }
    }

    pub fn array_head(&mut self) -> Res<usize> {
        Ok(self.expect(major::ARRAY)? as usize)
    }

    /// Read an array of uids, rejecting an unsorted one (constraint 3).
    pub fn uid_set(&mut self) -> Res<std::collections::BTreeSet<Uid>> {
        let at = self.pos;
        let n = self.array_head()?;
        let mut out = std::collections::BTreeSet::new();
        let mut prev: Option<Uid> = None;
        for _ in 0..n {
            let u = self.uid()?;
            if let Some(p) = prev {
                if u <= p {
                    return Err(CodecError::NonDeterministic {
                        at,
                        reason: NonDetReason::UnsortedSet,
                    });
                }
            }
            prev = Some(u);
            out.insert(u);
        }
        Ok(out)
    }

    /// Read an array whose elements must be sorted by encoded bytes (constraint 3).
    pub fn sorted_array<T>(&mut self, mut f: impl FnMut(&mut Dec<'a>) -> Res<T>) -> Res<Vec<T>> {
        let at = self.pos;
        let n = self.array_head()?;
        let mut out = Vec::with_capacity(n.min(64));
        let mut prev: Option<&'a [u8]> = None;
        for _ in 0..n {
            let start = self.pos;
            let v = f(self)?;
            let raw = &self.buf[start..self.pos];
            if let Some(p) = prev {
                if raw <= p {
                    return Err(CodecError::NonDeterministic {
                        at,
                        reason: NonDetReason::UnsortedSet,
                    });
                }
            }
            prev = Some(raw);
            out.push(v);
        }
        Ok(out)
    }

    /// Read an array whose order is authored rather than derived.
    pub fn array<T>(&mut self, mut f: impl FnMut(&mut Dec<'a>) -> Res<T>) -> Res<Vec<T>> {
        let n = self.array_head()?;
        let mut out = Vec::with_capacity(n.min(64));
        for _ in 0..n {
            out.push(f(self)?);
        }
        Ok(out)
    }

    pub fn map_head(&mut self) -> Res<usize> {
        Ok(self.expect(major::MAP)? as usize)
    }

    /// Read a map key, enforcing strict ascent and rejecting duplicates (constraint 1).
    pub fn map_key(&mut self, prev: Option<u16>) -> Res<u16> {
        let at = self.pos;
        let k = self.uint()?;
        if k > u16::MAX as u64 {
            return Err(CodecError::MalformedEnvelope { at });
        }
        let k = k as u16;
        match prev {
            Some(p) if k == p => Err(CodecError::NonDeterministic {
                at,
                reason: NonDetReason::DuplicateMapKey,
            }),
            Some(p) if k < p => Err(CodecError::NonDeterministic {
                at,
                reason: NonDetReason::UnsortedMapKeys,
            }),
            _ => Ok(k),
        }
    }

    /// Reject `null` where an absent optional belongs (constraint 6). Called before every
    /// map value, so `null` never reaches a type-specific reader that might tolerate it.
    pub fn reject_null(&self) -> Res<()> {
        if self.peek()? == NULL {
            Err(self.nondet(NonDetReason::NullOptional))
        } else {
            Ok(())
        }
    }

    /// Skip one complete item and return its raw bytes, so unknown keys can be preserved
    /// verbatim. Nested items are skipped strictly - an indefinite length anywhere inside
    /// is still a rejection.
    pub fn skip_item(&mut self) -> Res<&'a [u8]> {
        let start = self.pos;
        self.skip_one(0)?;
        Ok(&self.buf[start..self.pos])
    }

    /// `depth` is carried as a parameter rather than kept on `Dec`, so it unwinds with the
    /// recursion and there is no state to forget to reset between items.
    fn skip_one(&mut self, depth: usize) -> Res<()> {
        if depth > crate::cbor::MAX_NESTING {
            return Err(CodecError::NestingTooDeep {
                at: self.pos,
                limit: crate::cbor::MAX_NESTING,
            });
        }
        let b = self.peek()?;
        if b == NULL {
            return Err(self.nondet(NonDetReason::NullOptional));
        }
        // Simple values other than null carry no payload beyond the head, except floats.
        if b >> 5 == major::SIMPLE {
            return match b {
                F32_HEAD => {
                    self.pos += 1;
                    self.take(4)?;
                    Ok(())
                }
                0xF4 | 0xF5 => {
                    self.pos += 1;
                    Ok(())
                }
                _ => Err(self.malformed()),
            };
        }
        let (m, arg) = self.head()?;
        match m {
            major::UINT | major::NEGINT => Ok(()),
            major::BYTES | major::TEXT => {
                self.take(arg as usize)?;
                Ok(())
            }
            major::ARRAY => {
                for _ in 0..arg {
                    self.skip_one(depth + 1)?;
                }
                Ok(())
            }
            major::MAP => {
                for _ in 0..arg {
                    self.skip_one(depth + 1)?;
                    self.skip_one(depth + 1)?;
                }
                Ok(())
            }
            major::TAG => Err(self.malformed()),
            _ => Err(self.malformed()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cbor::writer::Enc;

    fn dec(b: &[u8]) -> Dec<'_> {
        Dec::new(b)
    }

    fn reason(e: &CodecError) -> Option<NonDetReason> {
        match e {
            CodecError::NonDeterministic { reason, .. } => Some(*reason),
            _ => None,
        }
    }

    #[test]
    fn reads_back_what_the_encoder_wrote() {
        for v in [
            0u64,
            23,
            24,
            255,
            256,
            65535,
            65536,
            u32::MAX as u64,
            u64::MAX,
        ] {
            let mut e = Enc::new();
            e.uint(v);
            let b = e.into_bytes();
            assert_eq!(dec(&b).uint().unwrap(), v, "uint({v})");
        }
    }

    #[test]
    fn non_shortest_integers_are_rejected_at_every_width() {
        let cases: &[&[u8]] = &[
            &[0x18, 0x01],                               // uint8 for 1
            &[0x19, 0x00, 0x17],                         // uint16 for 23
            &[0x1A, 0x00, 0x00, 0x00, 0x01],             // uint32 for 1
            &[0x1B, 0, 0, 0, 0, 0, 0, 0, 1],             // uint64 for 1
            &[0x1B, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF], // uint64 for u32::MAX
        ];
        for c in cases {
            let e = dec(c).uint().unwrap_err();
            assert_eq!(reason(&e), Some(NonDetReason::NonShortestInt), "{c:?}");
        }
    }

    #[test]
    fn boundary_widths_are_accepted() {
        assert_eq!(dec(&[0x18, 0x18]).uint().unwrap(), 24);
        assert_eq!(dec(&[0x19, 0x01, 0x00]).uint().unwrap(), 256);
        assert_eq!(dec(&[0x1A, 0, 1, 0, 0]).uint().unwrap(), 65536);
    }

    #[test]
    fn indefinite_lengths_are_rejected() {
        for b in [0x5Fu8, 0x7F, 0x9F, 0xBF] {
            let e = dec(&[b]).head().unwrap_err();
            assert_eq!(reason(&e), Some(NonDetReason::IndefiniteLength));
        }
    }

    #[test]
    fn text_must_be_nfc() {
        let mut e = Enc::new();
        e.head(major::TEXT, 6);
        e.raw("cafe\u{301}".as_bytes());
        let b = e.into_bytes();
        let err = dec(&b).text().unwrap_err();
        assert_eq!(reason(&err), Some(NonDetReason::NonNfcText));
    }

    #[test]
    fn nfc_text_round_trips() {
        let mut e = Enc::new();
        e.text("caf\u{e9}");
        let b = e.into_bytes();
        assert_eq!(dec(&b).text().unwrap(), "caf\u{e9}");
    }

    #[test]
    fn invalid_utf8_is_malformed_not_a_determinism_violation() {
        let b = [0x62, 0xFF, 0xFE];
        assert!(matches!(
            dec(&b).text().unwrap_err(),
            CodecError::MalformedEnvelope { .. }
        ));
    }

    #[test]
    fn floats_must_be_binary32() {
        // binary64 encoding of 614.0 / 1024.0
        let mut b = vec![0xFB];
        b.extend_from_slice(&(614.0_f64 / 1024.0).to_be_bytes());
        assert!(matches!(
            dec(&b).f32q().unwrap_err(),
            CodecError::Float { .. }
        ));

        // binary16
        let b = [0xF9, 0x38, 0x00];
        assert!(matches!(
            dec(&b).f32q().unwrap_err(),
            CodecError::Float { .. }
        ));
    }

    #[test]
    fn floats_must_already_be_quantised() {
        let mut b = vec![0xFA];
        b.extend_from_slice(&0.6_f32.to_be_bytes());
        assert!(matches!(
            dec(&b).f32q().unwrap_err(),
            CodecError::Float { .. }
        ));

        let mut ok = vec![0xFA];
        ok.extend_from_slice(&(614.0_f32 / 1024.0).to_be_bytes());
        assert_eq!(dec(&ok).f32q().unwrap(), 614.0 / 1024.0);
    }

    #[test]
    fn uids_must_be_full_width() {
        let mut e = Enc::new();
        e.bytes(&[0x11; 17]);
        let b = e.into_bytes();
        assert!(matches!(
            dec(&b).uid().unwrap_err(),
            CodecError::TruncatedUid { len: 17, .. }
        ));

        let mut e = Enc::new();
        e.uid(&Uid::from_bytes([0x11; 32]));
        let b = e.into_bytes();
        assert_eq!(dec(&b).uid().unwrap(), Uid::from_bytes([0x11; 32]));
    }

    #[test]
    fn map_keys_must_strictly_ascend() {
        let mut d = dec(&[0x00, 0x01, 0x00]);
        let k0 = d.map_key(None).unwrap();
        let k1 = d.map_key(Some(k0)).unwrap();
        assert_eq!((k0, k1), (0, 1));
        let e = d.map_key(Some(k1)).unwrap_err();
        assert_eq!(reason(&e), Some(NonDetReason::UnsortedMapKeys));
    }

    #[test]
    fn duplicate_map_keys_are_distinguished_from_unsorted_ones() {
        let mut d = dec(&[0x05]);
        let e = d.map_key(Some(5)).unwrap_err();
        assert_eq!(reason(&e), Some(NonDetReason::DuplicateMapKey));
    }

    #[test]
    fn null_is_rejected_where_an_optional_belongs() {
        let d = dec(&[0xF6]);
        assert_eq!(
            reason(&d.reject_null().unwrap_err()),
            Some(NonDetReason::NullOptional)
        );
        assert!(dec(&[0x00]).reject_null().is_ok());
    }

    #[test]
    fn uid_sets_must_be_sorted_and_deduplicated() {
        let mut e = Enc::new();
        e.array_head(2);
        e.uid(&Uid::from_bytes([2; 32]));
        e.uid(&Uid::from_bytes([1; 32]));
        let b = e.into_bytes();
        assert_eq!(
            reason(&dec(&b).uid_set().unwrap_err()),
            Some(NonDetReason::UnsortedSet)
        );

        let mut e = Enc::new();
        e.array_head(2);
        e.uid(&Uid::from_bytes([1; 32]));
        e.uid(&Uid::from_bytes([1; 32]));
        let b = e.into_bytes();
        assert_eq!(
            reason(&dec(&b).uid_set().unwrap_err()),
            Some(NonDetReason::UnsortedSet),
            "a repeated element is not a set"
        );
    }

    #[test]
    fn sorted_uid_sets_are_accepted() {
        let mut e = Enc::new();
        e.array_head(2);
        e.uid(&Uid::from_bytes([1; 32]));
        e.uid(&Uid::from_bytes([2; 32]));
        let b = e.into_bytes();
        assert_eq!(dec(&b).uid_set().unwrap().len(), 2);
    }

    #[test]
    fn truncated_input_is_reported_as_truncation() {
        assert!(matches!(
            dec(&[]).uint().unwrap_err(),
            CodecError::Truncated { .. }
        ));
        assert!(matches!(
            dec(&[0x19, 0x01]).uint().unwrap_err(),
            CodecError::Truncated { .. }
        ));
        assert!(matches!(
            dec(&[0x45, 0x01]).bytes().unwrap_err(),
            CodecError::Truncated { .. }
        ));
    }

    #[test]
    fn a_type_mismatch_leaves_the_position_untouched() {
        let mut d = dec(&[0x65, 0x63, 0x6c, 0x61, 0x69, 0x6d]);
        assert!(d.uint().is_err());
        assert_eq!(d.position(), 0, "a failed read must not consume");
        assert_eq!(d.text().unwrap(), "claim");
    }

    #[test]
    fn skip_item_returns_the_exact_bytes_it_skipped() {
        let mut e = Enc::new();
        e.uint(7);
        e.text("claim");
        let b = e.into_bytes();
        let mut d = dec(&b);
        assert_eq!(d.skip_item().unwrap(), &[0x07]);
        assert_eq!(d.skip_item().unwrap(), &b[1..]);
        assert!(d.is_done());
    }

    #[test]
    fn skip_item_handles_nested_containers() {
        let mut e = Enc::new();
        e.array_head(2);
        e.uint(1);
        e.map(vec![(0, vec![0x01]), (1, vec![0x02])]);
        let b = e.into_bytes();
        let mut d = dec(&b);
        assert_eq!(d.skip_item().unwrap().len(), b.len());
        assert!(d.is_done());
    }

    #[test]
    fn skip_item_still_rejects_a_nested_indefinite_length() {
        let b = [0x81, 0x9F, 0xFF];
        assert_eq!(
            reason(&dec(&b).skip_item().unwrap_err()),
            Some(NonDetReason::IndefiniteLength)
        );
    }

    #[test]
    fn tags_are_rejected() {
        let b = [0xC0, 0x00];
        assert!(matches!(
            dec(&b).skip_item().unwrap_err(),
            CodecError::MalformedEnvelope { .. }
        ));
    }
}
