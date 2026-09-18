//! `fixtures/wire/F12-reserved-pack.cbor` (1.6): a pack manifest that reserved part of its budget.
//!
//! §8.1 permits a new key in a record body, and states the test of "permitted" mechanically: a
//! reader written against the older revision must still round-trip a document containing the
//! addition, byte for byte. `packinfo` key 7 is that addition, and this fixture is what the
//! Python, JavaScript and Go suites check themselves against — none of them decodes a pack
//! manifest's body, so what they have to prove is that they carry it through untouched.
//!
//! It holds both encodings on purpose. A manifest that reserved nothing must not carry the key at
//! all, or every pack written before 1.6 would change its bytes for a field nobody set; one that
//! reserved something must carry it. A reader that invented a default for the first, or dropped
//! the key from the second, fails here rather than in somebody's store.
//!
//! `SMYSL_BLESS=1` rewrites the file. Doing that should be a decision: it changes what three other
//! implementations are checked against.

use std::path::Path;

use smysl_core::{to_cbor_seq, PackInfo, Record};

#[test]
fn the_reserved_pack_wire_fixture_matches_its_source() {
    let records = vec![
        // Reserved nothing: key 7 absent, which is the encoding every pack had before 1.6.
        Record::PackInfo(PackInfo::new(4096, 900, "smysl/utf8-div4")),
        // A 16k window with 2 500 tokens of system prompt, question and answer room set aside.
        Record::PackInfo(PackInfo::new(16_384, 9_120, "smysl/utf8-div4").reserving(2_500)),
        // And with a caller's own tokenizer named, since the id is free text either way.
        Record::PackInfo(PackInfo::new(8_192, 4_010, "tiktoken/cl100k").reserving(1_200)),
    ];
    let bytes = to_cbor_seq(&records);
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/wire/F12-reserved-pack.cbor");
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::write(&path, &bytes).unwrap();
        return;
    }
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "fixtures/wire/F12-reserved-pack.cbor no longer matches its source"
    );
}

/// The two encodings differ in exactly one key, and each decodes back to what it was.
#[test]
fn a_reservation_is_carried_and_its_absence_is_not_invented() {
    let plain = PackInfo::new(4096, 900, "smysl/utf8-div4");
    let reserved = PackInfo::new(4096, 900, "smysl/utf8-div4").reserving(1_200);

    let (a, b) = (
        smysl_core::to_cbor(&Record::PackInfo(plain.clone())),
        smysl_core::to_cbor(&Record::PackInfo(reserved.clone())),
    );
    assert!(b.len() > a.len(), "the key has to be somewhere");
    assert_eq!(
        smysl_core::from_cbor(&a).unwrap().0,
        Record::PackInfo(plain)
    );
    assert_eq!(
        smysl_core::from_cbor(&b).unwrap().0,
        Record::PackInfo(reserved)
    );
}
