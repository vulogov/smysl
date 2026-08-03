//! The shared rejection corpus, read by the implementation that defines the format.
//!
//! `fixtures/wire/invalid/` is consumed by all four implementations. Until 0.10 the three
//! outside this repository were checked only on *acceptance* — four valid documents, in and
//! out, byte for byte. That is the weaker half of the claim. Determinism is enforced by
//! refusal: §3 exists so that one value has exactly one encoding, and every clause in it is a
//! rule about what must be rejected. Nothing had ever checked that two implementations refuse
//! the same bytes.
//!
//! Building the corpus found that they did not. This walker accepted seven of the
//! twenty-eight that Python, JavaScript and Go all rejected — unsorted and duplicate map
//! keys, non-NFC and invalid UTF-8 text, and floats that were unquantised, infinite or NaN.
//!
//! That was not cosmetic. `skip_item` is what preserves unknown keys for rule X, its result
//! is stored verbatim in `Extra`, and `unit_core_bytes` writes `Extra` into the bytes that
//! `hash::uid` hashes. So a non-canonical extension payload reached content-addressed
//! identity: the same logical unit, with its extension map keyed in two orders, produced two
//! different uids. The comment above the call site claimed the opposite in as many words —
//! "the payload is still parsed strictly, so an unknown record cannot smuggle in a
//! non-deterministic encoding". It was the kind of claim that survives because nothing ever
//! tests it.

use smysl_core::cbor::Dec;
use std::path::{Path, PathBuf};

fn corpus() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/wire/invalid");
    let mut v: Vec<_> = std::fs::read_dir(&dir)
        .expect("the invalid corpus is present")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "cbor").unwrap_or(false))
        .collect();
    v.sort();
    v
}

/// Reject means: refuse, or consume less than the whole file. A decoder that read the first
/// valid-looking item and ignored a trailing violation would also be wrong.
fn accepts(data: &[u8]) -> bool {
    let mut d = Dec::new(data);
    matches!(d.skip_item(), Ok(_)) && d.remaining() == 0
}

#[test]
fn the_corpus_is_not_empty() {
    // Guards the whole file: a glob that matched nothing would make every other assertion
    // here vacuously true, which is the failure this project keeps finding.
    let n = corpus().len();
    assert!(n >= 28, "expected the shared corpus, found {n} files");
}

#[test]
fn every_invalid_fixture_is_rejected() {
    let accepted: Vec<_> = corpus()
        .into_iter()
        .filter(|p| accepts(&std::fs::read(p).unwrap()))
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert!(
        accepted.is_empty(),
        "these are not smysl documents, and were accepted: {accepted:?}"
    );
}

/// The control. If the walker rejected everything unconditionally the test above would pass
/// while meaning nothing, so canonical counterparts of the same shapes must be accepted.
#[test]
fn the_canonical_counterparts_are_accepted() {
    let valid: Vec<(&str, Vec<u8>)> = vec![
        ("shortest integer", vec![0x01]),
        ("definite array", vec![0x81, 0x01]),
        ("sorted map", vec![0xA2, 0x00, 0x01, 0x01, 0x02]),
        ("composed text", vec![0x62, 0xC3, 0xA9]),
        ("quantised float", vec![0xFA, 0x3F, 0x00, 0x00, 0x00]),
    ];
    for (what, bytes) in valid {
        assert!(accepts(&bytes), "{what} should be accepted: {bytes:02x?}");
    }
}

/// The defect the corpus found, kept as a regression: an extension payload must not be able
/// to carry two encodings of one value into the uid.
#[test]
fn an_extension_payload_cannot_carry_a_non_canonical_encoding() {
    // A unit core {0:"s", 1:"g", 6:0, 200:<inner>} where 200 is unknown, so its value is
    // preserved verbatim by `skip_item` and hashed by `unit_core_bytes`.
    let core = |inner: &[u8]| {
        let mut v = vec![
            0xA4, 0x00, 0x61, b's', 0x01, 0x61, b'g', 0x06, 0x00, 0x18, 0xC8,
        ];
        v.extend_from_slice(inner);
        v
    };
    let sorted = core(&[0xA2, 0x01, 0x01, 0x02, 0x02]);
    let unsorted = core(&[0xA2, 0x02, 0x02, 0x01, 0x01]);

    assert!(accepts(&sorted), "the canonical ordering is a valid core");
    assert!(
        !accepts(&unsorted),
        "an unknown key's value was accepted with its map keys out of order; these are two \
         encodings of one unit, and both would reach `hash::uid` as different bytes"
    );
}
