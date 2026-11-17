//! `smysl-series`' chunk codec (RFC SMYSL-3 §19.2, TL-P3): no panic, and nothing misread.
//!
//! A chunk is read back from a file on disk, so whatever bytes are there are decoded or refused,
//! never a panic. What decodes is a sorted set of well-formed samples, and encoding it decodes
//! to the same samples: a chunk is a layout, and a layout that moved a sample would move a
//! window's digest.

#![no_main]
use libfuzzer_sys::fuzz_target;
use smysl_series::chunk::{decode, encode};

fuzz_target!(|data: &[u8]| {
    if let Ok(samples) = decode(data) {
        assert!(samples.windows(2).all(|w| w[0] < w[1]), "a sorted set");
        let again = decode(&encode(&samples)).expect("an encoded chunk decodes");
        assert_eq!(again, samples, "a chunk round-trips");
    }
});
