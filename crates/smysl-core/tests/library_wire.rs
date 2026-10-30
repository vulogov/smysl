//! Records 14, 15 and 18 on the wire (SMYSL-2.3 A-3, A-5; SMYSL-2.4 §5.1).
//!
//! The round-trip and identity properties live in `gen_library_fixtures.rs`, beside the
//! fixture they are checked against. What is here is the half a fixture cannot carry: the
//! inputs that must be **refused**, and the ones that must be preserved although this build
//! does not understand them.

use smysl_core::cbor::writer::MapBuilder;
use smysl_core::cbor::Enc;
use smysl_core::types::record_code;
use smysl_core::{
    from_cbor, to_cbor, Carry, LangTag, Manifest, Mid, ParentKind, PartEntry, PartReading,
    PartText, Rdid, Record, Tid, Uid,
};

fn manifest() -> Manifest {
    Manifest::new(
        "kjv/1769",
        LangTag::new("en").unwrap(),
        "osis/1",
        "public-domain",
        "top/64Ki-4Mi",
    )
    .with_carry(Carry::Text)
}

fn part() -> PartText {
    PartText::new(b"a part, normalised\n".to_vec())
}

fn reading() -> PartReading {
    PartReading::new(part().tid, "txt/1", Vec::new())
}

fn entry() -> PartEntry {
    let r = reading();
    PartEntry::new(
        part().tid,
        part().text.len() as u64,
        r.structure_hash(),
        r.rdid(),
    )
}

// ---------------------------------------------------------------------------
// Rule X: keys and codes this build does not know
// ---------------------------------------------------------------------------

/// A key above each new body's highest survives verbatim (§8.1).
///
/// The same test the relation, withdrawal and resolution bodies get in `roundtrip.rs`, applied
/// to the three bodies 1.10 adds. A manifest is the one that matters most: it is nineteen keys
/// wide and the likeliest of all the records to grow a twentieth.
#[test]
fn an_unknown_key_survives_in_every_new_record_body() {
    let cases = [
        (Record::Manifest(manifest()), 7u8, 19u8),
        (Record::PartText(part()), 2, 2),
        (Record::PartReading(reading()), 3, 4),
    ];
    for (record, known, extra_key) in cases {
        let mut bytes = to_cbor(&record);
        assert_eq!(
            bytes[2],
            0xA0 | known,
            "{}: expected {known} keys",
            record.type_name()
        );
        bytes[2] = 0xA0 | (known + 1);
        bytes.extend_from_slice(&[extra_key, 0x01]); // {…, extra_key: 1}

        let (back, n) = from_cbor(&bytes).unwrap_or_else(|e| panic!("{}: {e}", record.type_name()));
        assert_eq!(n, bytes.len());
        assert_ne!(back, record, "{}: the key was dropped", record.type_name());
        assert_eq!(
            to_cbor(&back),
            bytes,
            "{}: not verbatim",
            record.type_name()
        );
    }
}

/// A key a part entry does not know survives too, one level down inside the manifest.
///
/// Worth its own test because the entry is an element of an array inside a map: `read_map`
/// reaches it only because `dec_part_entry` calls it, and an encoder that rebuilt the entry
/// from its named fields alone would drop the key silently.
#[test]
fn an_unknown_part_entry_key_survives() {
    let mut e = entry();
    e.extra.insert(9, vec![0x18, 0x2a]); // key 9: the CBOR uint 42
    let m = manifest().with_parts(vec![e]);

    let bytes = to_cbor(&Record::Manifest(m.clone()));
    let (back, _) = from_cbor(&bytes).unwrap();
    let Record::Manifest(m2) = &back else {
        panic!("not a manifest")
    };
    assert_eq!(m2.parts[0].extra.get(&9), Some(&vec![0x18, 0x2a]));
    assert_eq!(to_cbor(&back), bytes, "re-encoded differently");
    assert_eq!(
        m2.mid(),
        m.mid(),
        "and the mid is over the bytes, so it held"
    );
}

/// Code 16 is reserved and MUST NOT be decoded (A-5).
///
/// Reserved for telemetry by an amendment that does not define its layout, so this build has
/// nothing to decode it *into*. It behaves exactly as code 9 has since 0.1: `Record::Unknown`,
/// preserved verbatim, reported as `SMY-W014`.
#[test]
fn the_reserved_hold_code_decodes_to_unknown() {
    let mut body = MapBuilder::new();
    body.put(0, |e| e.uint(1));
    let payload = body.into_bytes();

    let mut e = Enc::new();
    e.array_head(2);
    e.uint(record_code::HOLD);
    e.raw(&payload);
    let bytes = e.into_bytes();

    let (back, n) = from_cbor(&bytes).unwrap();
    assert_eq!(n, bytes.len());
    assert_eq!(
        back,
        Record::Unknown {
            code: 16,
            payload: payload.clone()
        }
    );
    assert_eq!(
        to_cbor(&back),
        bytes,
        "an unknown record re-encodes verbatim"
    );
    assert!(!record_code::KNOWN.contains(&16));
}

// ---------------------------------------------------------------------------
// What must be refused
// ---------------------------------------------------------------------------

/// Keys 11 and 12 are one fact. Either half alone is refused, not dropped.
///
/// Dropping the kind would produce a manifest that claims a parent and will not say how it
/// derives from it; dropping the parent would leave a kind naming nothing. Both re-encode to
/// different bytes than arrived, and a mid is over those bytes.
///
/// Built key by key rather than cut out of a valid record: the 32 bytes of a mid can hold
/// `0x0b`, so locating a key by scanning for its byte is a test that passes for the wrong
/// reason as soon as the hash changes.
#[test]
fn a_parent_without_its_kind_is_refused_in_both_directions() {
    let parent = Mid::of(b"parent");

    /// The five required keys, plus whatever `extra` adds.
    fn body(extra: impl FnOnce(&mut MapBuilder)) -> Vec<u8> {
        let mut e = Enc::new();
        e.array_head(2);
        e.uint(record_code::MANIFEST);
        let mut b = MapBuilder::new();
        b.put(0, |e| e.text("kjv/1769"));
        b.put(1, |e| e.array(vec![]));
        b.put(2, |e| e.text("en"));
        b.put(3, |e| e.text("osis/1"));
        b.put(4, |e| e.text("public-domain"));
        b.put(5, |e| e.uint(2));
        b.put(17, |e| e.text("top/64Ki-4Mi"));
        extra(&mut b);
        e.raw(&b.into_bytes());
        e.into_bytes()
    }

    let both = body(|b| {
        b.put(11, |e| e.bytes(parent.as_bytes()));
        b.put(12, |e| e.text("translation"));
    });
    let (back, _) = from_cbor(&both).expect("the pair decodes");
    let Record::Manifest(m) = &back else {
        panic!("not a manifest")
    };
    assert_eq!(m.parent, Some((parent, ParentKind::Translation)));
    assert_eq!(to_cbor(&back), both);

    assert!(
        from_cbor(&body(|b| {
            b.put(11, |e| e.bytes(parent.as_bytes()));
        }))
        .is_err(),
        "a `parent` with no `parent-kind` must be refused"
    );
    assert!(
        from_cbor(&body(|b| {
            b.put(12, |e| e.text("translation"));
        }))
        .is_err(),
        "a `parent-kind` naming no `parent` must be refused"
    );
    // And a kind this build cannot name is refused rather than dropped: the key is required
    // with 11, so defaulting it would invent a derivation nobody stated.
    assert!(
        from_cbor(&body(|b| {
            b.put(11, |e| e.bytes(parent.as_bytes()));
            b.put(12, |e| e.text("adaptation"));
        }))
        .is_err(),
        "an unknown parent kind must be refused"
    );
}

/// `lossy` is written only as `true`, so `false` has no encoding at all (A-5, key 15).
///
/// Admitting `0xF4` would give one manifest two byte strings and therefore two mids, which is
/// the whole failure content addressing cannot survive. The absence of the key *is* `false`.
#[test]
fn a_false_lossy_flag_has_no_encoding() {
    let mut m = manifest();
    m.lossy = true;
    let bytes = to_cbor(&Record::Manifest(m));
    // Exactly one, so the byte flipped below is the flag and not a coincidence inside a text
    // or a hash. The assertion is the test's own scaffolding, which is worth checking.
    assert_eq!(
        bytes.iter().filter(|b| **b == 0xF5).count(),
        1,
        "`true` appears once in these bytes"
    );
    let pos = bytes.iter().position(|b| *b == 0xF5).unwrap();

    let mut as_false = bytes.clone();
    as_false[pos] = 0xF4;
    assert!(
        from_cbor(&as_false).is_err(),
        "`lossy: false` is not a second spelling of a lossless manifest"
    );
}

/// A text-keyed map is ordered by its **encoded** keys, which is not string order.
///
/// A text head carries its length first, so `"a"` sorts before `"doi"` whatever the letters
/// are. The decoder has to enforce the order the generic `skip_item` enforces when the same
/// map arrives under a key this build does not know — otherwise one reader accepts a store
/// that a reader one version older rejects.
#[test]
fn an_identifiers_map_out_of_encoded_key_order_is_refused() {
    let mut m = manifest();
    m.identifiers.insert("doi".into(), "10.0/x".into());
    m.identifiers.insert("a".into(), "y".into());
    let bytes = to_cbor(&Record::Manifest(m));
    let (back, _) = from_cbor(&bytes).unwrap();
    assert_eq!(to_cbor(&back), bytes);

    // `"a"` is `61 61`, `"doi"` is `63 64 6f 69`: the shorter key must come first. Hand-build
    // the pair both ways round, so the refusal below is attributable to the order and not to
    // something else about a hand-built record.
    let hand = |pairs: [(&str, &str); 2]| {
        let mut e = Enc::new();
        e.array_head(2);
        e.uint(record_code::MANIFEST);
        let mut body = MapBuilder::new();
        body.put(0, |e| e.text("kjv/1769"));
        body.put(1, |e| e.array(vec![]));
        body.put(2, |e| e.text("en"));
        body.put(3, |e| e.text("osis/1"));
        body.put(4, |e| e.text("public-domain"));
        body.put(5, |e| e.uint(2));
        body.put(9, |e| {
            e.head(5, 2); // a two-entry map
            for (k, v) in pairs {
                e.text(k);
                e.text(v);
            }
        });
        body.put(17, |e| e.text("top/64Ki-4Mi"));
        e.raw(&body.into_bytes());
        e.into_bytes()
    };

    let sorted = hand([("a", "y"), ("doi", "10.0/x")]);
    let (back, _) = from_cbor(&sorted).expect("encoded-key order decodes");
    assert_eq!(to_cbor(&back), sorted, "and re-encodes to the same bytes");
    assert!(
        from_cbor(&hand([("doi", "10.0/x"), ("a", "y")])).is_err(),
        "a text-keyed map out of encoded-key order must be refused"
    );
}

/// A part text whose bytes do not hash to its tid **decodes**, and says so when asked.
///
/// The decode deliberately does not verify (SMYSL-2.4 §4.3.1). One corrupt record must not
/// stop a store from opening — that is F-12, and the cost of relearning it was a release. The
/// layer above reports `SMY-E446` and names the part, which it can only do if the record
/// reached it.
#[test]
fn a_part_text_that_does_not_hash_to_its_tid_decodes_and_reports_itself() {
    let good = part();
    assert!(good.verify());

    let liar = PartText::with_claimed_tid(Tid::of(b"not these bytes"), good.text.clone());
    assert!(!liar.verify(), "the claim is false");

    let bytes = to_cbor(&Record::PartText(liar.clone()));
    let (back, _) = from_cbor(&bytes).expect("a bad tid must not fail the decode");
    assert_eq!(back, Record::PartText(liar));
    assert_eq!(to_cbor(&back), bytes, "and it re-encodes verbatim");
}

/// An empty segment table is an empty array, not zero bytes.
///
/// A reading's `segments` is required and may be empty (A-5), and a `Vec::new()` from a caller
/// is an ordinary statement of that. Writing nothing would emit a map with a key and no value
/// and corrupt every record after it in the log.
#[test]
fn an_empty_segment_table_encodes_as_an_empty_array() {
    let r = reading();
    assert_eq!(
        r.segments,
        PartReading::EMPTY_SEGMENTS,
        "the constructor spells the empty table, so nothing downstream has to"
    );
    let bytes = to_cbor(&Record::PartReading(r.clone()));
    assert!(
        bytes.windows(1).any(|w| w[0] == 0x80),
        "the empty array is in the bytes: {bytes:02x?}"
    );

    let (back, n) = from_cbor(&bytes).unwrap();
    assert_eq!(n, bytes.len());
    let Record::PartReading(r2) = &back else {
        panic!("not a reading")
    };
    assert_eq!(r2.segments, vec![0x80], "it decodes as the array it is");
    assert_eq!(
        to_cbor(&back),
        bytes,
        "and the second encoding is the first"
    );
    assert_eq!(r2.rdid(), r.rdid(), "so the rdid did not move");
    assert_eq!(
        r2.structure_hash(),
        r.structure_hash(),
        "nor did the structure hash, which a manifest entry records and `SMY-E401` compares"
    );

    // The same, for a field assigned directly rather than through the constructor. `segments`
    // is public, so the substitution cannot live in `new` alone — and this is the case that was
    // wrong first: the encoder guarded it, `structure_hash` did not, and a correct corpus would
    // have failed `SMY-E401` with the text intact and the reading unchanged.
    let mut direct = reading();
    direct.segments = Vec::new();
    assert_eq!(
        direct.structure_hash(),
        r.structure_hash(),
        "an empty field and an empty table are the same table"
    );
    assert_eq!(direct.rdid(), r.rdid());
    assert_eq!(to_cbor(&Record::PartReading(direct)), bytes);
}

// ---------------------------------------------------------------------------
// The identities themselves
// ---------------------------------------------------------------------------

/// Each kind has its own prefix, and a text form of one kind is not a text form of another.
///
/// The prefixes are not decoration. A mid pasted where a tid belongs is a lookup that would
/// otherwise find nothing and report it as a missing part, which is a true statement about the
/// wrong question.
#[test]
fn the_text_forms_are_distinct_per_kind() {
    let t = Tid::of(b"x");
    let m = Mid::of(b"x");
    let r = Rdid::of(b"x");

    assert!(t.canonical().starts_with("t3:"));
    assert!(m.canonical().starts_with("m3:"));
    assert!(r.canonical().starts_with("r3:"));

    assert_eq!(Tid::parse(&t.canonical()).unwrap(), t);
    assert_eq!(Mid::parse(&m.canonical()).unwrap(), m);
    assert_eq!(Rdid::parse(&r.canonical()).unwrap(), r);

    assert!(Mid::parse(&t.canonical()).is_err(), "a tid is not a mid");
    assert!(Tid::parse(&m.canonical()).is_err(), "nor the reverse");
    assert!(Uid::parse(&t.canonical()).is_err(), "nor is a tid a uid");

    // The same preimage under three domain bytes is three different digests, which is the
    // property the prefixes only *announce*.
    assert_ne!(t.to_bytes(), m.to_bytes());
    assert_ne!(m.to_bytes(), r.to_bytes());
    assert_ne!(t.to_bytes(), r.to_bytes());

    // A short form is a display abbreviation and is refused in a record, as a uid's is.
    assert!(Tid::parse(&t.short()).is_err());
    assert_eq!(t.short().len(), "t3:".len() + 26);
    assert_eq!(t.canonical().len(), "t3:".len() + 52);
}

/// `as_uid` is the same 256 bits in the slot a record has for a uid (A-6).
///
/// For a withdrawal or a commitment naming a dating, and for a contention position. It is safe
/// only because the domain byte is already inside the digest.
#[test]
fn an_identity_fits_the_slot_a_record_keeps_for_a_uid() {
    let t = Tid::of(b"a part");
    assert_eq!(t.as_uid().as_bytes(), t.as_bytes());
    assert_eq!(
        t.as_uid().canonical()["b3:".len()..],
        t.canonical()["t3:".len()..]
    );
}

/// A reading's structure hash and its rdid answer different questions.
///
/// The structure hash is over the segment table alone, so a reading that gains the reader's raw
/// metadata keeps its structure hash and changes its rdid — which is why a manifest's part
/// entry carries both. Asserting it here is what stops a later edit collapsing them.
#[test]
fn the_structure_hash_is_over_the_table_and_the_rdid_over_the_record() {
    let plain = PartReading::new(part().tid, "txt/1", vec![0x81, 0xA0]); // [ {} ]
    let with_raw = plain.clone().with_raw(vec![0xA0]);

    assert_eq!(
        plain.structure_hash(),
        with_raw.structure_hash(),
        "raw metadata is not part of the structure"
    );
    assert_ne!(
        plain.rdid(),
        with_raw.rdid(),
        "but it is part of the record"
    );
}

/// Aliases are lowercase ASCII segments, at most 128 bytes (A-3).
#[test]
fn the_alias_grammar_is_what_the_amendment_says() {
    for good in [
        "kjv",
        "kjv/1769",
        "import:worldcat/42",
        "a.b-c_d/9",
        &"a".repeat(128),
    ] {
        assert!(smysl_core::is_alias(good), "{good} should be an alias");
    }
    for bad in [
        "",
        "KJV",
        "kjv//1769",
        "kjv/",
        "/kjv",
        "kjv 1769",
        "kjv+1769",
        &"a".repeat(129),
    ] {
        assert!(!smysl_core::is_alias(bad), "{bad} should not be an alias");
    }

    // The decoder does not apply the grammar — a later version may widen it, and refusing a
    // store over an alias is refusing to open it. The check is the surface parser's and the
    // library pass's, and `alias_is_valid` is where both get it.
    let mut m = manifest();
    m.alias = "NOT AN ALIAS".into();
    assert!(!m.alias_is_valid());
    let bytes = to_cbor(&Record::Manifest(m.clone()));
    assert!(from_cbor(&bytes).is_ok(), "but it still decodes");

    // And it has no surface form, which is the consequence that is easy to miss. The alias is
    // the header word, written unquoted because every conforming alias is safe unquoted, so a
    // manifest the decoder accepted and this parser would not would be emitted as a
    // `@manifest` line that the next parse rejects — `fmt` dropping a record and saying
    // nothing.
    assert!(
        !smysl_core::surface::manifest_has_surface_form(&m),
        "an alias this parser will not accept cannot be written as surface text"
    );
    assert_eq!(
        smysl_core::surface::write_surface(
            None,
            &[Record::Manifest(m)],
            &smysl_core::surface::WriteContext::default()
        ),
        "",
        "so the writer emits nothing rather than something it cannot read back"
    );
}
