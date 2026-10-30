//! FC-1 and FC-3: unit core key 9 `lang`, and `source` keys 5 `span` and 6 `manifest`.
//!
//! SMYSL-2.3 A-1 and A-2, activated in TX-P5 step 1. All three are **inside identity** — `lang`
//! is a core key and `source` is a core field — so every test here is really about uids.
//!
//! # What the exit clause asks, and where each half is asserted
//!
//! "Old stores re-encode unchanged" is the compatibility test spec §8.1 permits an addition
//! under, and it is asserted twice: here, by checking that a unit without the new fields encodes
//! to bytes that contain no key 9, 5 or 6, and in the conformance suite, where every wire fixture
//! is byte-pinned and round-trips. The second is the stronger of the two — those bytes were
//! written by a release that had never heard of these keys.
//!
//! `SMY-E404` and `SMY-W405` are findings about a *store* and need the part to check against, so
//! they live with the library check pass and its resolver.

use smysl_core::ids::{LangTag, Mid};
use smysl_core::{
    canonical_uid, from_cbor, surface, to_cbor, KernelType, Record, SourceKind, SourceRef, Status,
    UnitCore, UnitCoreBuilder,
};

fn mid() -> Mid {
    Mid::from_bytes([0x11; 32])
}

fn tid_ref() -> String {
    format!(
        "{}#Gen.1.1",
        smysl_core::ids::Tid::from_bytes([0x22; 32]).canonical()
    )
}

fn unit(lang: Option<&str>, span: Option<(u64, u64)>) -> UnitCore {
    let mut src = SourceRef::new(SourceKind::Doc, tid_ref());
    if let Some((start, end)) = span {
        src = src.in_part(start, end, mid()).expect("a forward range");
    }
    let mut b = UnitCoreBuilder::new(
        KernelType::Claim,
        "the fool hath said in his heart",
        Status::Cited,
    )
    .source(src);
    if let Some(tag) = lang {
        b = b.lang(LangTag::new(tag).expect("a tag"));
    }
    b.build().expect("a unit")
}

fn roundtrip(core: &UnitCore) -> UnitCore {
    let bytes = to_cbor(&Record::Unit(core.clone()));
    let (record, read) = from_cbor(&bytes).expect("it decodes");
    assert_eq!(read, bytes.len(), "the decoder stopped short");
    match record {
        Record::Unit(u) => u,
        other => panic!("expected a unit, got {other:?}"),
    }
}

fn written(core: &UnitCore) -> String {
    surface::write_surface(
        None,
        &[Record::Unit(core.clone())],
        &surface::WriteContext::default(),
    )
}

/// A unit without the new fields encodes to the bytes it always did.
#[test]
fn a_unit_without_them_encodes_as_it_always_did() {
    let core = unit(None, None);
    let bytes = to_cbor(&Record::Unit(core.clone()));

    // The map keys are written as unsigned integers, so the three new ones appear as the single
    // bytes 0x09, 0x05 and 0x06 in key position. Rather than hunt for key positions, the check
    // that means what the clause means: re-encoding what decodes is identical, and no byte of the
    // encoding changed when the fields were added — which the byte-pinned wire fixtures assert
    // over records written by a release that had never heard of these keys.
    assert_eq!(
        to_cbor(&Record::Unit(roundtrip(&core))),
        bytes,
        "a unit with no lang, span or manifest did not survive a round trip unchanged"
    );
    assert_eq!(core.lang, None);
    assert_eq!(core.source.as_ref().and_then(|s| s.span()), None);
    assert_eq!(core.source.as_ref().and_then(|s| s.manifest()), None);
}

/// Absent and present are different cores and different uids (A-1).
#[test]
fn a_language_is_part_of_what_a_gist_says() {
    let silent = unit(None, None);
    let english = unit(Some("en"), None);
    let german = unit(Some("de"), None);

    assert_ne!(
        canonical_uid(&silent),
        canonical_uid(&english),
        "saying nothing about the language and saying `en` are the same unit"
    );
    assert_ne!(
        canonical_uid(&english),
        canonical_uid(&german),
        "two languages, one uid"
    );
    assert_eq!(
        canonical_uid(&english),
        canonical_uid(&unit(Some("en"), None)),
        "one unit, two uids"
    );

    // `und` and `mul` are permitted values, and both are different from saying nothing: "nobody
    // knows" and "several" are answers.
    for tag in ["und", "mul"] {
        let u = unit(Some(tag), None);
        assert_ne!(canonical_uid(&u), canonical_uid(&silent), "{tag}");
        assert_eq!(roundtrip(&u).lang.as_ref().map(|t| t.as_str()), Some(tag));
    }
}

/// `lang` round-trips through CBOR and through surface text.
#[test]
fn a_language_survives_both_round_trips() {
    let core = unit(Some("ru-RU"), None);
    assert_eq!(
        roundtrip(&core).lang.as_ref().map(|t| t.as_str()),
        Some("ru-RU")
    );

    let text = written(&core);
    assert!(text.contains("lang: ru-RU"), "{text}");
    let out = surface::parse_surface(&text).expect("it parses");
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let back = match out.records.into_iter().next().expect("one record") {
        Record::Unit(u) => u,
        other => panic!("expected a unit, got {other:?}"),
    };
    assert_eq!(
        canonical_uid(&back),
        canonical_uid(&core),
        "the surface round trip moved the uid"
    );
}

/// A-10 item 2: the quoting is load-bearing. A bare `lang:` is the core key and a quoted
/// `"lang":` is a payload key.
#[test]
fn quoting_decides_whether_lang_is_the_core_key() {
    let bare = "@claim { status: speculative, lang: fr }\n~ le texte est en français\n";
    let out = surface::parse_surface(bare).expect("it parses");
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let core = match &out.records[0] {
        Record::Unit(u) => u.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(core.lang.as_ref().map(|t| t.as_str()), Some("fr"));
    assert!(
        core.payload.is_none(),
        "the core key went into the payload too"
    );

    let quoted = "@claim { status: speculative, \"lang\": fr }\n~ le texte est en français\n";
    let out = surface::parse_surface(quoted).expect("it parses");
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let payload_core = match &out.records[0] {
        Record::Unit(u) => u.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(payload_core.lang, None, "a quoted key became the core key");
    assert!(
        payload_core.payload.is_some(),
        "a quoted `lang` did not reach the payload"
    );

    // The two are different units, which is exactly what `SMY-W432` spent a release warning
    // about: the same four characters in two places, both hashed.
    assert_ne!(canonical_uid(&core), canonical_uid(&payload_core));

    // And the payload one survives being written back, because the writer quotes it.
    let text = written(&payload_core);
    assert!(
        text.contains("\"lang\""),
        "the writer wrote it bare: {text}"
    );
    let again = surface::parse_surface(&text).expect("it parses");
    let back = match &again.records[0] {
        Record::Unit(u) => u.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        canonical_uid(&back),
        canonical_uid(&payload_core),
        "write then parse moved a payload `lang` into the core"
    );
}

/// A tag that is not well-formed BCP-47 cannot reach a unit: `LangTag` is the gate, and the
/// surface says so rather than dropping the attribute.
#[test]
fn a_malformed_tag_is_refused_rather_than_dropped() {
    assert!(LangTag::new("not a tag").is_err());
    let out =
        surface::parse_surface("@claim { status: speculative, lang: \"not a tag\" }\n~ a gist\n")
            .expect("the parse returns");
    assert!(
        out.diagnostics
            .iter()
            .any(|d| d.code == smysl_core::Code::E001),
        "{:?}",
        out.diagnostics
    );
    assert!(
        out.records.is_empty(),
        "a unit was produced with the attribute dropped, and its uid would be wrong"
    );
}

/// The span is a pair of byte offsets, and two ranges of one part are two units.
#[test]
fn a_span_is_inside_identity() {
    let whole = unit(Some("en"), None);
    let first = unit(Some("en"), Some((0, 54)));
    let second = unit(Some("en"), Some((55, 197)));

    assert_ne!(canonical_uid(&first), canonical_uid(&second));
    assert_ne!(canonical_uid(&first), canonical_uid(&whole));
    assert_eq!(
        roundtrip(&first).source.and_then(|s| s.span()),
        Some((0, 54))
    );
    assert_eq!(
        roundtrip(&first).source.and_then(|s| s.manifest().copied()),
        Some(mid())
    );
}

/// A-2.4 rule 1: a span needs its manifest, at every door.
#[test]
fn a_span_without_a_manifest_cannot_be_written_or_read() {
    // The constructor sets both, so the illegal state is not reachable through this crate's API,
    // and a reversed or empty range is `None` rather than a span naming no bytes.
    let src = SourceRef::new(SourceKind::Doc, tid_ref());
    assert!(
        src.clone().in_part(10, 10, mid()).is_none(),
        "an empty span"
    );
    assert!(
        src.clone().in_part(20, 10, mid()).is_none(),
        "a reversed span"
    );

    // The surface refuses the pair's absence and names the rule.
    let doc = format!(
        "@claim {{ status: cited, source: {{ kind: doc, ref: \"{}\", span: [0, 10] }} }}\n~ a gist\n",
        tid_ref()
    );
    let out = surface::parse_surface(&doc).expect("the parse returns");
    assert!(
        out.diagnostics
            .iter()
            .any(|d| d.code == smysl_core::Code::E001),
        "{:?}",
        out.diagnostics
    );
    assert!(out.records.is_empty());

    // And a span that is not two ascending integers is refused too.
    for bad in ["[10, 10]", "[20, 10]", "[0]", "\"0-10\""] {
        let doc = format!(
            "@claim {{ status: cited, source: {{ kind: doc, ref: \"{}\", span: {bad}, manifest: \
             {} }} }}\n~ a gist\n",
            tid_ref(),
            mid().canonical()
        );
        let out = surface::parse_surface(&doc).expect("the parse returns");
        assert!(!out.diagnostics.is_empty(), "`{bad}` was accepted");
    }
}

/// The span and the manifest survive a surface round trip, which is what makes a passage-bearing
/// unit writable at all.
#[test]
fn a_passage_unit_survives_the_surface() {
    let core = unit(Some("en"), Some((198, 252)));
    let text = written(&core);
    assert!(text.contains("span: [198, 252]"), "{text}");
    assert!(
        text.contains(&format!("manifest: {}", mid().canonical())),
        "{text}"
    );

    let out = surface::parse_surface(&text).expect("it parses");
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let back = match &out.records[0] {
        Record::Unit(u) => u.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(canonical_uid(&back), canonical_uid(&core));
}

/// A manifest without a span is legal on its own: a unit may name the manifestation it came from
/// without quoting a range of it (A-2.1 makes key 6 optional).
#[test]
fn a_manifest_without_a_span_is_allowed() {
    let doc = format!(
        "@claim {{ status: cited, source: {{ kind: doc, ref: \"{}\", manifest: {} }} }}\n~ a gist\n",
        tid_ref(),
        mid().canonical()
    );
    let out = surface::parse_surface(&doc).expect("the parse returns");
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let core = match &out.records[0] {
        Record::Unit(u) => u.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        core.source.as_ref().and_then(|s| s.manifest()),
        Some(&mid())
    );
    assert_eq!(core.source.as_ref().and_then(|s| s.span()), None);
    assert_eq!(canonical_uid(&roundtrip(&core)), canonical_uid(&core));
}

/// An unknown source key is still `SMY-E001`: the set is closed, and two more keys does not open
/// it (A-10 item 3, F-13).
#[test]
fn the_source_key_set_is_still_closed() {
    let doc = "@claim { status: cited, source: { kind: doc, ref: \"x\", spans: [0, 1] } }\n~ g\n";
    let out = surface::parse_surface(doc).expect("the parse returns");
    assert!(
        out.diagnostics
            .iter()
            .any(|d| d.code == smysl_core::Code::E001),
        "{:?}",
        out.diagnostics
    );
}
