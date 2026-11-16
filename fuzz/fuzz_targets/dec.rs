//! `dec` (RFC SMYSL-3 §5): one value, one pair, one encoding, one spelling.
//!
//! Over arbitrary bytes, as CBOR: anything the decoder accepts re-encodes to the bytes it was
//! read from, so two encodings never name one value. Over the same bytes as text: anything the
//! token reader accepts spells canonically, and the spelling reads back to the same value and
//! the same encoding. Neither path may panic, which is the first thing a reader of untrusted
//! telemetry owes its caller.

#![no_main]
use libfuzzer_sys::fuzz_target;
use smysl_core::Decimal;

fuzz_target!(|data: &[u8]| {
    if let Ok(v) = Decimal::from_cbor(data) {
        assert_eq!(v.to_cbor(), data, "an accepted dec did not re-encode to itself");
        let spelled = v.to_string();
        assert_eq!(spelled.parse::<Decimal>(), Ok(v), "{spelled}");
    }
    if let Ok(token) = core::str::from_utf8(data) {
        if let Ok(v) = Decimal::parse_token(token) {
            let spelled = v.to_string();
            assert_eq!(spelled.parse::<Decimal>(), Ok(v), "{token:?} -> {spelled}");
            assert_eq!(Decimal::from_cbor(&v.to_cbor()), Ok(v), "{token:?}");
        }
    }
});
