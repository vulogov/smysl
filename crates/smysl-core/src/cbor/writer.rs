//! The deterministic encoder (§15.4).
//!
//! Encoding is infallible: every value that reaches here has already been validated by a
//! constructor, so there is no failure mode left to report. Violations of the
//! determinism constraints are `debug_assert`s rather than errors, because they would be
//! bugs in this crate rather than bad input.

use crate::cbor::{major, F32_HEAD};
use crate::ids::Uid;
use crate::types::is_quantised;

/// A deterministic CBOR encoder.
#[derive(Debug, Default, Clone)]
pub struct Enc {
    buf: Vec<u8>,
}

impl Enc {
    pub fn new() -> Enc {
        Enc { buf: Vec::new() }
    }

    pub fn with_capacity(n: usize) -> Enc {
        Enc {
            buf: Vec::with_capacity(n),
        }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Encode a head in shortest form. There is no path here that emits a longer form,
    /// which is what makes constraint 2 structural rather than checked.
    pub fn head(&mut self, major: u8, arg: u64) {
        let m = major << 5;
        match arg {
            0..=23 => self.buf.push(m | arg as u8),
            24..=0xFF => {
                self.buf.push(m | 24);
                self.buf.push(arg as u8);
            }
            0x100..=0xFFFF => {
                self.buf.push(m | 25);
                self.buf.extend_from_slice(&(arg as u16).to_be_bytes());
            }
            0x1_0000..=0xFFFF_FFFF => {
                self.buf.push(m | 26);
                self.buf.extend_from_slice(&(arg as u32).to_be_bytes());
            }
            _ => {
                self.buf.push(m | 27);
                self.buf.extend_from_slice(&arg.to_be_bytes());
            }
        }
    }

    pub fn uint(&mut self, v: u64) {
        self.head(major::UINT, v);
    }

    pub fn bytes(&mut self, b: &[u8]) {
        self.head(major::BYTES, b.len() as u64);
        self.buf.extend_from_slice(b);
    }

    /// Encode text. The NFC invariant is established by the constructors, so this only
    /// asserts it in debug builds rather than paying for a scan in release.
    pub fn text(&mut self, s: &str) {
        debug_assert!(
            unicode_normalization::is_nfc(s),
            "text reached the encoder without NFC normalisation: {s:?}"
        );
        self.head(major::TEXT, s.len() as u64);
        self.buf.extend_from_slice(s.as_bytes());
    }

    /// Encode a float, quantising to 1/1024 first (constraint 4).
    pub fn f32q(&mut self, v: f32) {
        let q = crate::types::quantise(v);
        debug_assert!(is_quantised(q));
        self.buf.push(F32_HEAD);
        self.buf.extend_from_slice(&q.to_be_bytes());
    }

    pub fn array_head(&mut self, n: usize) {
        self.head(major::ARRAY, n as u64);
    }

    pub fn uid(&mut self, u: &Uid) {
        self.bytes(u.as_bytes());
    }

    /// Encode a set of uids as an array. `BTreeSet<Uid>` iterates in byte order and uids
    /// are fixed width, so iteration order already *is* encoded-byte order (constraint 3).
    pub fn uid_set<'a>(&mut self, set: impl ExactSizeIterator<Item = &'a Uid>) {
        self.array_head(set.len());
        for u in set {
            self.uid(u);
        }
    }

    /// Encode a map from pre-encoded entries.
    ///
    /// Entries are sorted by key and duplicates are a bug, not an input error: the
    /// per-record encoders build their entry lists field by field.
    pub fn map(&mut self, mut entries: Vec<(u16, Vec<u8>)>) {
        entries.sort_by_key(|(k, _)| *k);
        debug_assert!(
            entries.windows(2).all(|w| w[0].0 < w[1].0),
            "duplicate map key in encoder output"
        );
        self.head(major::MAP, entries.len() as u64);
        for (k, v) in entries {
            self.uint(k as u64);
            self.buf.extend_from_slice(&v);
        }
    }

    /// Emit an array of pre-encoded elements sorted by their encoded bytes
    /// (constraint 3). Text and schema-id sets need this: CBOR byte order is not
    /// lexicographic order, because the length prefix sorts first.
    pub fn sorted_array(&mut self, mut items: Vec<Vec<u8>>) {
        items.sort();
        debug_assert!(
            items.windows(2).all(|w| w[0] < w[1]),
            "duplicate element in an encoded set"
        );
        self.array_head(items.len());
        for i in items {
            self.raw(&i);
        }
    }

    /// Emit an array of pre-encoded elements in the order given. For sequences whose
    /// order is authored rather than derived - thread steps, for instance.
    pub fn array(&mut self, items: Vec<Vec<u8>>) {
        self.array_head(items.len());
        for i in items {
            self.raw(&i);
        }
    }

    pub fn raw(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
}

/// Encode one value into a standalone buffer. Used to build map entries.
pub fn enc<F: FnOnce(&mut Enc)>(f: F) -> Vec<u8> {
    let mut e = Enc::new();
    f(&mut e);
    e.into_bytes()
}

/// Builder for a record payload map. Fields are added in whatever order is convenient;
/// the encoder sorts.
#[derive(Debug, Default)]
pub struct MapBuilder {
    entries: Vec<(u16, Vec<u8>)>,
}

impl MapBuilder {
    pub fn new() -> MapBuilder {
        MapBuilder {
            entries: Vec::new(),
        }
    }

    /// Add a required field.
    pub fn put<F: FnOnce(&mut Enc)>(&mut self, key: u16, f: F) -> &mut Self {
        self.entries.push((key, enc(f)));
        self
    }

    /// Add a field only if present. An absent optional is *omitted*, never `null`
    /// (constraint 6).
    pub fn put_opt<T, F: FnOnce(&mut Enc, &T)>(
        &mut self,
        key: u16,
        value: Option<&T>,
        f: F,
    ) -> &mut Self {
        if let Some(v) = value {
            self.entries.push((key, enc(|e| f(e, v))));
        }
        self
    }

    /// Add a set field only if non-empty. An empty set is indistinguishable from an
    /// absent one in the kernel types, so omitting it keeps one encoding per core.
    pub fn put_uid_set<'a>(
        &mut self,
        key: u16,
        set: impl ExactSizeIterator<Item = &'a Uid>,
    ) -> &mut Self {
        if set.len() > 0 {
            self.entries.push((key, enc(|e| e.uid_set(set))));
        }
        self
    }

    /// Add a set field, sorted by encoded element bytes, only if non-empty.
    pub fn put_sorted_set(&mut self, key: u16, items: Vec<Vec<u8>>) -> &mut Self {
        if !items.is_empty() {
            self.entries.push((key, enc(|e| e.sorted_array(items))));
        }
        self
    }

    /// Add a sequence field in authored order, only if non-empty.
    pub fn put_array(&mut self, key: u16, items: Vec<Vec<u8>>) -> &mut Self {
        if !items.is_empty() {
            self.entries.push((key, enc(|e| e.array(items))));
        }
        self
    }

    /// Re-emit unknown keys read from a future minor version, verbatim.
    pub fn put_extra(&mut self, extra: &crate::types::Extra) -> &mut Self {
        for (k, v) in extra {
            self.entries.push((*k, v.clone()));
        }
        self
    }

    pub fn finish(self, e: &mut Enc) {
        e.map(self.entries);
    }

    pub fn into_bytes(self) -> Vec<u8> {
        let mut e = Enc::new();
        self.finish(&mut e);
        e.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn integers_use_the_shortest_head() {
        let cases: &[(u64, &str)] = &[
            (0, "00"),
            (23, "17"),
            (24, "1818"),
            (255, "18ff"),
            (256, "190100"),
            (65535, "19ffff"),
            (65536, "1a00010000"),
            (0xFFFF_FFFF, "1affffffff"),
            (0x1_0000_0000, "1b0000000100000000"),
        ];
        for (v, want) in cases {
            let mut e = Enc::new();
            e.uint(*v);
            assert_eq!(hex(e.as_bytes()), *want, "uint({v})");
        }
    }

    #[test]
    fn text_and_bytes_carry_a_length_head() {
        let mut e = Enc::new();
        e.text("claim");
        assert_eq!(hex(e.as_bytes()), "65636c61696d");

        let mut e = Enc::new();
        e.bytes(&[0x11, 0x22]);
        assert_eq!(hex(e.as_bytes()), "421122");
    }

    #[test]
    fn floats_are_quantised_binary32() {
        let mut e = Enc::new();
        e.f32q(0.6);
        let b = e.into_bytes();
        assert_eq!(b.len(), 5, "head plus four bytes, never binary64");
        assert_eq!(b[0], 0xFA);
        // 614/1024 = 614.0 / 1024.0, the quantum 0.6 snaps to.
        assert_eq!(&b[1..], &(614.0_f32 / 1024.0).to_be_bytes());
    }

    #[test]
    fn quantisation_happens_before_encoding() {
        let mut a = Enc::new();
        a.f32q(0.6);
        let mut b = Enc::new();
        b.f32q(614.0 / 1024.0);
        assert_eq!(a.as_bytes(), b.as_bytes(), "0.6 and its quantum must agree");
    }

    #[test]
    fn maps_are_emitted_in_ascending_key_order() {
        let mut m = MapBuilder::new();
        m.put(6, |e| e.uint(1));
        m.put(0, |e| e.text("claim"));
        m.put(1, |e| e.text("g"));
        let bytes = m.into_bytes();
        assert_eq!(hex(&bytes), "a30065636c61696d016167 0601".replace(' ', ""));
    }

    #[test]
    fn absent_optionals_are_omitted_not_nulled() {
        let mut m = MapBuilder::new();
        m.put(0, |e| e.text("claim"));
        m.put_opt::<String, _>(2, None, |e, v| e.text(v));
        let bytes = m.into_bytes();
        assert_eq!(bytes[0], 0xA1, "one entry, not two");
        assert!(!bytes.contains(&0xF6), "no null anywhere");
    }

    #[test]
    fn present_optionals_are_written() {
        let body = "a body".to_string();
        let mut m = MapBuilder::new();
        m.put_opt(2, Some(&body), |e, v| e.text(v));
        let bytes = m.into_bytes();
        assert_eq!(bytes[0], 0xA1);
    }

    #[test]
    fn empty_sets_are_omitted() {
        use std::collections::BTreeSet;
        let empty: BTreeSet<Uid> = BTreeSet::new();
        let mut m = MapBuilder::new();
        m.put(0, |e| e.uint(0));
        m.put_uid_set(4, empty.iter());
        assert_eq!(m.into_bytes()[0], 0xA1);
    }

    #[test]
    fn uid_sets_encode_in_byte_order() {
        use std::collections::BTreeSet;
        let set: BTreeSet<Uid> = [
            Uid::from_bytes([3; 32]),
            Uid::from_bytes([1; 32]),
            Uid::from_bytes([2; 32]),
        ]
        .into_iter()
        .collect();
        let mut e = Enc::new();
        e.uid_set(set.iter());
        let b = e.into_bytes();
        assert_eq!(b[0], 0x83, "array of three");
        // Each element is a 32-byte string: head 0x5820 then the bytes.
        assert_eq!(b[1], 0x58);
        assert_eq!(b[3], 1);
        assert_eq!(b[3 + 34], 2);
        assert_eq!(b[3 + 68], 3);
    }

    #[test]
    fn extra_keys_are_re_emitted_verbatim() {
        let mut extra = crate::types::Extra::new();
        extra.insert(42, vec![0x01]);
        let mut m = MapBuilder::new();
        m.put(0, |e| e.uint(0));
        m.put_extra(&extra);
        let bytes = m.into_bytes();
        assert_eq!(bytes[0], 0xA2);
        assert_eq!(&bytes[bytes.len() - 3..], &[0x18, 42, 0x01]);
    }

    #[test]
    fn encoding_is_byte_stable_across_repeated_runs() {
        let build = || {
            let mut m = MapBuilder::new();
            m.put(1, |e| e.text("a gist"));
            m.put(6, |e| e.uint(1));
            m.put(0, |e| e.text("claim"));
            m.into_bytes()
        };
        assert_eq!(build(), build());
    }
}
