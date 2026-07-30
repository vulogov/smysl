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

/// The deepest nesting the reader will walk before refusing.
///
/// The reader descends into containers recursively, so an unbounded depth means a deeply
/// nested value overflows the stack and **aborts the process** — measured at roughly 20 000
/// levels, reached through an unknown key, which is to say through rule X. An abort is worse
/// than an error: it cannot be caught, so an embedder cannot contain it, and rule A1
/// promises no panics on untrusted input.
///
/// 128 is far above anything a real document produces — the deepest structure the kernel
/// defines is a source reference inside a unit header, three levels — and far below the
/// depth that threatens the stack. A bound that refuses honestly beats a walk that aborts.
pub const MAX_NESTING: usize = 128;

pub mod envelope;
pub mod keys;
pub mod reader;
pub mod writer;

pub use envelope::{from_cbor, from_cbor_seq, to_cbor, to_cbor_seq};
pub use reader::Dec;
pub use writer::Enc;

/// CBOR major types, named so the codec never uses a bare number.
pub mod major {
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
pub const NULL: u8 = 0xF6;
/// Additional-information value marking an indefinite-length item.
pub(crate) const INDEFINITE: u8 = 31;
/// Additional-information value introducing a `binary32` float.
pub const F32_HEAD: u8 = 0xFA;
