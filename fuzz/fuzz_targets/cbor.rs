//! Guarantee A1 for the codec, plus the determinism invariant.
//!
//! Anything the reader accepts must re-encode to the exact bytes it was read from -
//! otherwise two byte sequences would map to one record, and a uid would stop being an
//! identity.

#![no_main]
use libfuzzer_sys::fuzz_target;
use smysl_core::{from_cbor, to_cbor};

fuzz_target!(|data: &[u8]| {
    if let Ok((record, n)) = from_cbor(data) {
        assert_eq!(
            to_cbor(&record),
            &data[..n],
            "an accepted encoding did not re-encode to itself"
        );
    }
});
