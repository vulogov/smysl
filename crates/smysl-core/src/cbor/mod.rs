//! The deterministic CBOR codec (§7.1, §15.4).
//!
//! Hand-rolled, with no CBOR crate: the determinism requirements here exceed what a
//! general-purpose codec guarantees, and the guarantee is the point. RFC 8949 §4.2 core
//! deterministic encoding, plus:
//!
//! 1. map keys are small unsigned integers, ascending;
//! 2. shortest-form integers, no indefinite-length items;
//! 3. sets are arrays sorted by encoded element bytes;
//! 4. floats are `binary32` **after** quantisation to 1/1024;
//! 5. text is NFC-normalised before encoding;
//! 6. absent optionals are omitted, never `null`.
//!
//! The reader **rejects** rather than normalises, so a uid always corresponds to exactly
//! one byte sequence. Normalising on read would mean two byte sequences hash to one uid
//! for the reader and two for everyone else.

pub mod envelope;
pub mod keys;
pub mod reader;
pub mod writer;

pub use envelope::{from_cbor, from_cbor_seq, to_cbor, to_cbor_seq};
pub use reader::Dec;
pub use writer::Enc;

/// CBOR major types, named so the codec never uses a bare number.
pub(crate) mod major {
    pub const UINT: u8 = 0;
    pub const NEGINT: u8 = 1;
    pub const BYTES: u8 = 2;
    pub const TEXT: u8 = 3;
    pub const ARRAY: u8 = 4;
    pub const MAP: u8 = 5;
    pub const TAG: u8 = 6;
    pub const SIMPLE: u8 = 7;
}

/// The `null` simple value. Its presence in an optional field is `SMY-E080`: an absent
/// optional is *omitted*, and admitting both spellings would make two encodings of the
/// same core.
pub(crate) const NULL: u8 = 0xF6;
/// Additional-information value marking an indefinite-length item.
pub(crate) const INDEFINITE: u8 = 31;
/// Additional-information value introducing a `binary32` float.
pub(crate) const F32_HEAD: u8 = 0xFA;
