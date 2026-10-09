//! `fixtures/library/wire/` — records 14, 15 and 18, their identities, and their bodies.
//!
//! The library records are the first ones whose identity is **not** a uid. A tid is a hash of
//! bytes, a mid and an rdid are hashes of record bodies, and each is domain-separated by the
//! record code it names (SMYSL-2.3 A-3). None of that is reachable through C-Read: decoding a
//! manifest never requires deriving a mid, so the Python, JavaScript and Go suites could
//! round-trip every record here byte for byte without computing one — the same gap
//! `fixtures/wire/uid/cases.json` was written to close for uids.
//!
//! So the fixture carries three things per record, for the same reason that one does: the
//! canonical body bytes, the identity derived from them, and the record envelope. A reader that
//! disagrees can tell whether it encoded differently or hashed differently.
//!
//! Unlike `gen_uid_fixtures.rs`, the check here is not `#[ignore]`. The generator is the test:
//! it rebuilds the fixture from source and compares, so a change to the encoder fails CI
//! instead of leaving three other implementations checked against bytes this build no longer
//! produces. `SMYSL_BLESS=1` rewrites the files, and that should be a decision — it changes
//! what those implementations are checked against.

use std::path::{Path, PathBuf};

use smysl_core::cbor::envelope::{manifest_bytes, part_reading_bytes};
use smysl_core::cbor::writer::MapBuilder;
use smysl_core::cbor::Enc;
use smysl_core::surface::{parse_surface, write_surface, WriteContext};
use smysl_core::types::library::Redaction;
use smysl_core::{from_cbor_seq, to_cbor_seq, Manifest, PartReading, PartText, Record, Tid, Uid};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/library/wire")
}

/// The two parts of the fixture expression.
///
/// Short, public domain, and NFC already — a fixture is not the place to test the normaliser,
/// which is `smysl-text`'s and arrives in TX-P1 step 2. What these bytes have to be is
/// *normalised*, because `Tid::from_normalised_bytes` is a promise the caller makes.
const PART_ONE: &str = "In the beginning God created the heaven and the earth.\n\
                        And the earth was without form, and void.\n";
const PART_TWO: &str = "В начале сотворил Бог небо и землю.\n";

/// The bytes of the part the fixture's redaction names — and does **not** carry.
///
/// Here only so that the tid is derived from something rather than invented: a fixture holding
/// a 32-byte constant nobody can reproduce is a fixture whose first mismatch is unattributable.
const GONE: &str = "A third part, redacted before this fixture was written.\n";

/// One segment row, as `smysl-text` will build it: `{0: start, 1: end, 2: level, 3: locator}`.
///
/// Hand-built here rather than imported, because the builder does not exist yet and the
/// structure hash in a manifest's part entry is taken over exactly these bytes. That is the
/// point of carrying `segments` opaquely in `PartReading`: the table's encoder lives in the
/// crate that understands locators, and `smysl-core` hashes what it is given.
fn segment_table(rows: &[(u64, u64, &str, &str)]) -> Vec<u8> {
    let mut e = Enc::new();
    e.array_head(rows.len());
    for (start, end, level, locator) in rows {
        let mut m = MapBuilder::new();
        m.put(0, |e| e.uint(*start));
        m.put(1, |e| e.uint(*end));
        m.put(2, |e| e.text(level));
        m.put(3, |e| e.text(locator));
        m.finish(&mut e);
    }
    e.into_bytes()
}

struct Built {
    manifest: Manifest,
    texts: Vec<PartText>,
    readings: Vec<PartReading>,
    /// The redaction, over a part this fixture deliberately does **not** hold.
    ///
    /// Which is the state rule Z leaves behind: the record remains and the bytes are gone. A
    /// redaction naming one of the two parts here would make the fixture a record sequence that
    /// `Store::append` filters — a conformance fixture whose records a conforming store drops is
    /// a fixture nobody can check against.
    redaction: Redaction,
    /// The surface document the manifest was parsed from, with the real identities in it.
    surface: String,
}

fn build() -> Built {
    let texts = vec![
        PartText::new(PART_ONE.as_bytes().to_vec()),
        PartText::new(PART_TWO.as_bytes().to_vec()),
    ];
    let readings = vec![
        PartReading::new(
            texts[0].tid,
            "osis/1",
            // Real byte offsets into `PART_ONE`: the first verse runs 0..55 including its
            // newline, the second 55..97. Wrong-but-plausible offsets in a conformance
            // fixture are worse than none, because three other implementations would agree
            // with them.
            segment_table(&[(0, 55, "verse", "Gen.1.1"), (55, 97, "verse", "Gen.1.2")]),
        ),
        PartReading::new(
            texts[1].tid,
            "osis/1",
            segment_table(&[(0, PART_TWO.len() as u64, "verse", "Gen.1.1")]),
        ),
    ];

    // The identities go into the surface text, so the document and the records cannot drift
    // apart: a change to any of the three hashes rewrites the source the manifest is parsed
    // from, and the committed `.smy` shows it.
    let surface = format!(
        "@manifest kjv/1769 {{ lang: en, reader: osis/1, licence: public-domain, carry: text, \
         part-policy: \"top/64Ki-4Mi\", title: \"The Holy Bible: King James Version\", \
         creators: [\"The Translators\"], published: 1769, \
         identifiers: {{ url: \"https://example.invalid/kjv\" }}, \
         origin: {{ kind: url, ref: \"https://example.invalid/kjv.osis.xml\", \
         captured: \"2026-10-08\" }}, versification: kjv, calendar: julian, \
         parts: [{}, {}] }}\n",
        part_row(&texts[0], &readings[0], None),
        part_row(&texts[1], &readings[1], Some("ru")),
    );

    // The redaction goes in the same document, so that the record and the text it is parsed
    // from cannot drift: a change to either rewrites the committed `.smy`.
    let gone = Tid::of(GONE.as_bytes());
    let surface = format!(
        "{surface}\n@redact {} {{ agent: human:vu, ts: [1726500000000, 0] }}\n",
        gone.canonical()
    );

    let out = parse_surface(&surface).unwrap();
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let manifest = out
        .records
        .iter()
        .find_map(|r| match r {
            Record::Manifest(m) => Some(m.clone()),
            _ => None,
        })
        .expect("the document declares a manifest");
    let redaction = out
        .records
        .iter()
        .find_map(|r| match r {
            Record::Redaction(r) => Some(r.clone()),
            _ => None,
        })
        .expect("the document declares a redaction");

    Built {
        manifest,
        texts,
        readings,
        redaction,
        surface,
    }
}

fn part_row(t: &PartText, r: &PartReading, lang: Option<&str>) -> String {
    let mut s = format!(
        "{{ tid: {}, length: {}, structure: {}, rdid: {} ",
        t.tid.canonical(),
        t.text.len(),
        Uid::from_bytes(r.structure_hash()).canonical(),
        r.rdid().canonical(),
    );
    if let Some(l) = lang {
        s = s.trim_end().to_string();
        s.push_str(&format!(", lang: {l} "));
    }
    s.push('}');
    s
}

/// Records this build does not decode, to prove they survive it.
///
/// 16 is A-5's reserved telemetry slot, which no amendment has defined; 17 is allocated and
/// lands in TX-P3. Both must decode to `Record::Unknown`, re-encode byte for byte and be
/// reported as `SMY-W014`. That is the whole argument for why adding records 14, 15, 18 and 19
/// is an addition rather than a version break, and it is worth asserting from the side that
/// will actually meet them: a 1.10 build reading a 1.11 store.
///
/// **19 left this list in TX-P2 step 4.** It is a redaction now, and the fixture carries a real
/// one — which is the better test of the same property anyway: three other implementations have
/// to decode it, and until step 4 they were being checked against a map with one key in it.
fn forward_records() -> Vec<Record> {
    [16u64, 17]
        .into_iter()
        .map(|code| {
            let mut m = MapBuilder::new();
            m.put(0, |e| e.uint(code));
            Record::Unknown {
                code,
                payload: m.into_bytes(),
            }
        })
        .collect()
}

fn records(b: &Built) -> Vec<Record> {
    let mut v = vec![Record::Manifest(b.manifest.clone())];
    for t in &b.texts {
        v.push(Record::PartText(t.clone()));
    }
    for r in &b.readings {
        v.push(Record::PartReading(r.clone()));
    }
    v.push(Record::Redaction(b.redaction.clone()));
    v.extend(forward_records());
    v
}

fn ids_json(b: &Built) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str(
        "  \"purpose\": \"Records 14, 15, 18 and 19 with their canonical body bytes and the \
         identities derived from them (SMYSL-2.3 A-3). Each identity is BLAKE3-256 over a \
         one-byte domain prefix and a preimage: tid over the part's normalised bytes (0x0f), \
         mid over the manifest body (0x0e), rdid over the reading body (0x12). The body bytes \
         are included so a mismatch says whether the encoding or the hashing disagreed. \
         `structure_hex` is BLAKE3-256 of the segment table alone, with no domain byte, \
         because it names a table and not a record.\",\n",
    );
    s.push_str(&format!(
        "  \"manifest\": {{ \"alias\": \"{}\", \"mid_hex\": \"{}\", \"body_hex\": \"{}\" }},\n",
        b.manifest.alias,
        hex(b.manifest.mid().as_bytes()),
        hex(&manifest_bytes(&b.manifest)),
    ));
    s.push_str("  \"parts\": [\n");
    for (i, (t, r)) in b.texts.iter().zip(&b.readings).enumerate() {
        s.push_str(&format!(
            "    {{ \"tid_hex\": \"{}\", \"length\": {}, \"text_hex\": \"{}\", \
             \"rdid_hex\": \"{}\", \"reading_body_hex\": \"{}\", \"structure_hex\": \"{}\" }}{}\n",
            hex(t.tid.as_bytes()),
            t.text.len(),
            hex(&t.text),
            hex(r.rdid().as_bytes()),
            hex(&part_reading_bytes(r)),
            hex(&r.structure_hash()),
            if i + 1 == b.texts.len() { "" } else { "," },
        ));
    }
    s.push_str("  ],\n");
    // The redaction (record 19, rule Z). No identity of its own — a redaction is a statement
    // about a part, named by the part's tid — so what the ports have to agree on is the body
    // bytes and the tid inside them. The part itself is deliberately absent from the fixture:
    // that is the state a honoured redaction leaves behind.
    s.push_str(&format!(
        "  \"redaction\": {{ \"tid_hex\": \"{}\", \"agent\": \"{}\", \
         \"ts_ms\": {}, \"body_hex\": \"{}\", \
         \"note\": \"the part this names is not in the fixture, which is what honouring a \
         redaction leaves behind\" }},\n",
        hex(b.redaction.tid.as_bytes()),
        b.redaction.agent.as_str(),
        b.redaction.ts.wall_ms,
        hex(&smysl_core::cbor::envelope::redaction_bytes(&b.redaction)),
    ));
    s.push_str(&format!(
        "  \"domain_bytes\": {{ \"tid\": {}, \"mid\": {}, \"did\": {}, \"rdid\": {} }}\n",
        Tid::DOMAIN,
        smysl_core::ids::Mid::DOMAIN,
        smysl_core::ids::Did::DOMAIN,
        smysl_core::ids::Rdid::DOMAIN,
    ));
    s.push_str("}\n");
    s
}

fn check(name: &str, want: &[u8]) {
    let path = dir().join(name);
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::create_dir_all(dir()).unwrap();
        std::fs::write(&path, want).unwrap();
        return;
    }
    let have = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(
        have, want,
        "fixtures/library/wire/{name} no longer matches what this build produces; \
         `SMYSL_BLESS=1 cargo test -p smysl-core --test gen_library_fixtures` rewrites it, \
         which changes what the Python, JavaScript and Go suites are checked against"
    );
}

#[test]
fn the_library_wire_fixture_matches_this_build() {
    let b = build();
    check("records.cbor", &to_cbor_seq(&records(&b)));
    check("ids.json", ids_json(&b).as_bytes());
    check("manifest.smy", b.surface.as_bytes());
}

#[test]
fn every_library_record_round_trips_byte_identically() {
    let b = build();
    let all = records(&b);
    let bytes = to_cbor_seq(&all);
    let (back, consumed) = from_cbor_seq(&bytes).unwrap();
    assert_eq!(consumed, bytes.len(), "the sequence decoded in full");
    assert_eq!(back, all, "a record decoded to something else");
    assert_eq!(to_cbor_seq(&back), bytes, "a record re-encoded differently");
}

/// The identities are what the amendment says they are, derived here from the preimage.
///
/// Not a restatement of the implementation: the hash is recomputed from the domain byte and
/// the preimage with no help from the identity types, so a change to `digest_id!` that broke
/// the domain separation would fail here rather than agree with itself.
#[test]
fn each_identity_is_blake3_over_its_domain_byte_and_preimage() {
    let b = build();

    let by_hand = |domain: u8, preimage: &[u8]| {
        let mut h = blake3::Hasher::new();
        h.update(&[domain]);
        h.update(preimage);
        *h.finalize().as_bytes()
    };

    assert_eq!(
        b.manifest.mid().to_bytes(),
        by_hand(0x0E, &manifest_bytes(&b.manifest)),
        "a mid is BLAKE3(0x0e ‖ the manifest body)"
    );
    for (t, r) in b.texts.iter().zip(&b.readings) {
        assert_eq!(
            t.tid.to_bytes(),
            by_hand(0x0F, &t.text),
            "a tid is BLAKE3(0x0f ‖ the normalised bytes)"
        );
        assert_eq!(
            r.rdid().to_bytes(),
            by_hand(0x12, &part_reading_bytes(r)),
            "an rdid is BLAKE3(0x12 ‖ the reading body)"
        );
    }

    // The property the domain bytes exist for. A uid's preimage is a CBOR map, so its first
    // byte is 0xa0-0xbf; none of the four domain bytes is, which is what makes it impossible
    // for a library identity to equal a uid however the preimages collide.
    for d in [0x0Eu8, 0x0F, 0x11, 0x12] {
        assert!(
            !(0xA0..=0xBF).contains(&d),
            "{d:#x} could collide with a uid"
        );
    }
}

/// A manifest survives `parse -> write -> parse` as the same manifest, and the second write
/// produces the same text as the first.
///
/// The fixed point is the property, not the round trip: a writer that emitted `parts: []` where
/// the parser reads an absent `parts`, or `lossy: false` where the wire has no key, would pass
/// a single round trip and move the document on every save after that.
#[test]
fn the_surface_form_is_a_fixed_point() {
    let b = build();
    let ctx = WriteContext::default();
    let once = write_surface(None, &[Record::Manifest(b.manifest.clone())], &ctx);
    let reparsed = parse_surface(&once).unwrap();
    assert!(
        reparsed.diagnostics.is_empty(),
        "{:?}",
        reparsed.diagnostics
    );
    let twice = write_surface(None, &reparsed.records, &ctx);
    assert_eq!(once, twice, "the second write moved the document");

    let m2 = reparsed
        .records
        .iter()
        .find_map(|r| match r {
            Record::Manifest(m) => Some(m.clone()),
            _ => None,
        })
        .expect("a manifest came back");
    assert_eq!(
        m2, b.manifest,
        "the manifest changed through the round trip"
    );
    assert_eq!(m2.mid(), b.manifest.mid(), "and so would its mid have");
}

/// An import manifest: no parts, and no text ever carried.
///
/// `parts` is required on the wire and may be empty (A-5), which is the one key whose empty
/// form has to encode. The surface omits it, so this is also where the two defaults are checked
/// against each other.
#[test]
fn an_import_manifest_has_no_parts_and_still_round_trips() {
    let src = "@manifest import:worldcat/42 { lang: und, reader: smysl/1, licence: unknown, \
               carry: none, part-policy: \"none\" }\n";
    let out = parse_surface(src).unwrap();
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let Some(Record::Manifest(m)) = out.records.first() else {
        panic!("expected a manifest, got {:?}", out.records.first());
    };
    assert!(m.parts.is_empty());
    assert_eq!(m.length(), 0);

    let bytes = to_cbor_seq(&out.records);
    let (back, _) = from_cbor_seq(&bytes).unwrap();
    assert_eq!(back, out.records);
    assert_eq!(to_cbor_seq(&back), bytes);

    let ctx = WriteContext::default();
    let text = write_surface(None, &out.records, &ctx);
    assert!(
        !text.contains("parts"),
        "an empty `parts` is omitted, not written as `[]`: {text}"
    );
    assert_eq!(
        parse_surface(&text).unwrap().records,
        out.records,
        "and the omission still reads back as an import manifest"
    );
}
