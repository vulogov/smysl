//! Record 17 end to end: surface, CBOR, identity (SMYSL-2.3 A-5, A-3; TX-P3 step 1).
//!
//! A dating is the first library record whose *target* can be four different things and whose
//! *value* can be three, so the combinations are what this file is for. The wire fixture
//! (`gen_library_fixtures.rs`) pins three of them for the other implementations; these are the
//! ones a reader of this crate would otherwise have to take on trust.

use smysl_core::cbor::envelope::dating_bytes;
use smysl_core::ids::{AgentId, Did, Mid, Tid, Uid};
use smysl_core::surface::{parse_surface, write_surface, WriteContext};
use smysl_core::types::library::{Allen, Axis, Dating, DatingTarget, DatingValue};
use smysl_core::types::provenance::Hlc;
use smysl_core::{from_cbor, to_cbor, Code, Record};

fn tid(byte: u8) -> Tid {
    Tid::from_normalised_bytes(&[byte])
}

fn agent() -> AgentId {
    AgentId::new("human:vu").expect("an agent")
}

fn ts() -> Hlc {
    Hlc::new(1726500000000, 0, agent())
}

fn datings_in(src: &str) -> Vec<Dating> {
    let out = parse_surface(src).expect("a document");
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    out.records
        .iter()
        .filter_map(|r| match r {
            Record::Dating(d) => Some(d.clone()),
            _ => None,
        })
        .collect()
}

/// The surface form is a fixed point over every combination the grammar can spell.
///
/// Parse, write, parse again, and compare the *records* rather than the text: the writer picks
/// its own quoting, so comparing strings would test the quoter and this tests the grammar.
#[test]
fn every_spellable_dating_survives_a_round_trip_through_text() {
    let p1 = tid(1).canonical();
    let p2 = tid(2).canonical();
    let mid = Mid::of(b"a manifest body").canonical();
    let cases = [
        format!("@date {p1} {{ axis: said, when: 1611, agent: human:vu, ts: [1, 0] }}"),
        format!("@date {p1} {{ axis: composed, when: \"1611?\", agent: human:vu, ts: [1, 0] }}"),
        format!("@date {p1} {{ axis: about, when: \"1920/1930\", agent: human:vu, ts: [1, 0] }}"),
        format!("@date {mid} {{ axis: said, when: \"-0044-03-15\", agent: human:vu, ts: [1, 0] }}"),
        format!("@date {p1} {{ axis: said, offset: -10800000, agent: human:vu, ts: [1, 0] }}"),
        format!("@date {p1} {{ axis: said, offset: 3600000, agent: human:vu, ts: [1, 0] }}"),
        format!(
            "@date {p1} {{ axis: said, window: [100, 200], offset: -1, \
             agent: human:vu, ts: [1, 0] }}"
        ),
        format!("@date {p2} {{ axis: said, after: {p1}, agent: human:vu, ts: [1, 0] }}"),
        format!("@date {p2} {{ axis: said, during: {mid}, agent: human:vu, ts: [1, 0] }}"),
    ];
    for src in cases {
        let first = datings_in(&src);
        assert_eq!(first.len(), 1, "{src}");
        let text = write_surface(
            None,
            &[Record::Dating(first[0].clone())],
            &WriteContext::default(),
        );
        let again = datings_in(&text);
        assert_eq!(first, again, "{src} -> {text}");
    }
}

/// Every Allen relation is a key, and each parses to the relation it is named after.
#[test]
fn the_seven_allen_relations_are_each_a_key() {
    let p1 = tid(1).canonical();
    let p2 = tid(2).canonical();
    for allen in Allen::ALL {
        let src =
            format!("@date {p2} {{ axis: said, {allen}: {p1}, agent: human:vu, ts: [1, 0] }}");
        let d = datings_in(&src);
        assert_eq!(d.len(), 1, "{src}");
        let DatingValue::Relative { allen: got, .. } = &d[0].value else {
            panic!("{src} is not relative");
        };
        assert_eq!(got, allen);
    }
    // And `starts` is one of the six Allen relations this format does not use, so it is an
    // unknown key rather than a relation nobody meant.
    let out = parse_surface(&format!(
        "@date {p2} {{ axis: said, starts: {p1}, agent: human:vu, ts: [1, 0] }}"
    ))
    .expect("a document");
    assert!(out.diagnostics.iter().any(|d| d.code == Code::E001));
}

/// A window target writes its part positionally and its range in `window:`.
#[test]
fn a_window_narrows_a_part_and_keeps_its_half_open_range() {
    let d = datings_in(&format!(
        "@date {} {{ axis: said, window: [1726500000000, 1726586400000], offset: -10800000, \
         agent: human:vu, ts: [1, 0] }}",
        tid(1).canonical()
    ));
    assert_eq!(
        d[0].target,
        DatingTarget::Window {
            tid: tid(1),
            from_ms: 1726500000000,
            to_ms: 1726586400000,
        }
    );
    assert!(d[0].target.selects_anything());
}

/// A `window:` on anything but a part is refused rather than silently ignored.
#[test]
fn a_window_on_a_manifest_is_not_a_window() {
    let out = parse_surface(&format!(
        "@date {} {{ axis: said, window: [1, 2], when: 1611, agent: human:vu, ts: [1, 0] }}",
        Mid::of(b"m").canonical()
    ))
    .expect("a document");
    assert!(out.diagnostics.iter().any(|d| d.code == Code::E001));
}

/// A malformed EDTF value is `SMY-E410` wherever it is written.
///
/// Two places, and both matter: `@date` is new in this step and `source { published: … }` is
/// the field FC-2 adds. A value the parser dropped instead would give the unit a uid its
/// author did not write.
#[test]
fn a_malformed_edtf_value_is_refused_in_both_places() {
    for src in [
        format!(
            "@date {} {{ axis: said, when: \"1984-13\", agent: human:vu, ts: [1, 0] }}",
            tid(1).canonical()
        ),
        "@claim c/x { source: { kind: doc, ref: \"a book\", published: \"1984-13\" } }\n~ A claim.\n"
            .to_string(),
        "@claim c/x { source: { kind: doc, ref: \"a book\", published: 920 } }\n~ A claim.\n"
            .to_string(),
    ] {
        let out = parse_surface(&src).expect("a document");
        assert!(
            out.diagnostics.iter().any(|d| d.code == Code::E410),
            "{src} produced {:?}",
            out.diagnostics
        );
    }
}

/// A well-formed `published` survives into the unit, and into its uid.
#[test]
fn a_published_date_reaches_the_unit_and_its_bytes() {
    let src = "@claim c/x { source: { kind: doc, ref: \"a book\", published: \"1920?\" } }\n\
               ~ A claim.\n";
    let out = parse_surface(src).expect("a document");
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let unit = out
        .records
        .iter()
        .find_map(|r| match r {
            Record::Unit(u) => Some(u.clone()),
            _ => None,
        })
        .expect("a unit");
    assert_eq!(
        unit.source.as_ref().and_then(|s| s.published.as_deref()),
        Some("1920?")
    );
    // Inside identity: the same claim with no `published` is a different unit.
    let bare = parse_surface("@claim c/x { source: { kind: doc, ref: \"a book\" } }\n~ A claim.\n")
        .expect("a document");
    let other = bare
        .records
        .iter()
        .find_map(|r| match r {
            Record::Unit(u) => Some(u.clone()),
            _ => None,
        })
        .expect("a unit");
    assert_ne!(
        smysl_core::canonical_uid(&unit),
        smysl_core::canonical_uid(&other),
        "`published` is inside `source`, which is inside the uid"
    );
    // And it comes back out of the surface writer, so a document is not quietly shortened —
    // asserted by re-parsing rather than by matching the text, since the quoting is the
    // writer's own business and the identity is not.
    let text = write_surface(None, &out.records, &WriteContext::default());
    assert!(text.contains("published: 1920?"), "{text}");
    let again = parse_surface(&text).expect("a document");
    assert!(again.diagnostics.is_empty(), "{:?}", again.diagnostics);
    assert!(again.records.contains(&Record::Unit(unit)));
}

/// Every target kind and every value kind round-trips through CBOR byte for byte.
#[test]
fn every_dating_round_trips_through_cbor() {
    let targets = [
        DatingTarget::Unit(Uid::from_bytes([7; 32])),
        DatingTarget::Part(tid(1)),
        DatingTarget::Manifest(Mid::of(b"m")),
        DatingTarget::Window {
            tid: tid(1),
            from_ms: 100,
            to_ms: 200,
        },
    ];
    let values = [
        DatingValue::Absolute("1611".to_string()),
        DatingValue::Offset(-10800000),
        DatingValue::Offset(i64::MIN),
        DatingValue::Offset(0),
        DatingValue::Relative {
            allen: Allen::During,
            target: DatingTarget::Part(tid(2)),
        },
        DatingValue::Relative {
            allen: Allen::Before,
            target: DatingTarget::Window {
                tid: tid(2),
                from_ms: 1,
                to_ms: 2,
            },
        },
    ];
    for target in &targets {
        for value in &values {
            for axis in Axis::ALL {
                let d = Dating::new(target.clone(), *axis, value.clone(), agent(), ts())
                    .with_basis(Uid::from_bytes([3; 32]));
                let bytes = to_cbor(&Record::Dating(d.clone()));
                let (back, used) = from_cbor(&bytes).expect("it decodes");
                assert_eq!(used, bytes.len());
                assert_eq!(back, Record::Dating(d.clone()));
                assert_eq!(to_cbor(&back), bytes, "re-encoded differently");
            }
        }
    }
}

/// The did is BLAKE3 over the domain byte and the body, computed here without the identity type.
#[test]
fn a_did_is_blake3_over_its_domain_byte_and_the_body() {
    let d = Dating::absolute(
        DatingTarget::Part(tid(1)),
        Axis::Said,
        &smysl_core::edtf::parse("1611").expect("edtf"),
        agent(),
        ts(),
    );
    let mut h = blake3::Hasher::new();
    h.update(&[Did::DOMAIN]);
    h.update(&dating_bytes(&d));
    assert_eq!(d.did().as_bytes(), h.finalize().as_bytes());
    assert_eq!(Did::DOMAIN, 0x11, "the record code it names");
    // A dating that differs in any hashed field is a different dating.
    let other = Dating::absolute(
        DatingTarget::Part(tid(1)),
        Axis::Composed,
        &smysl_core::edtf::parse("1611").expect("edtf"),
        agent(),
        ts(),
    );
    assert_ne!(d.did(), other.did());
}

/// A relative dating whose own target is a window cannot be spelled, and says so.
///
/// Representable in CBOR and not in text: a window is written as a `window:` key beside the
/// positional target, and there is one positional slot. The writer counts it among the records
/// surface cannot hold rather than writing it back as something else.
#[test]
fn a_relative_dating_onto_a_window_has_no_surface_form() {
    let d = Dating::new(
        DatingTarget::Part(tid(1)),
        Axis::Said,
        DatingValue::Relative {
            allen: Allen::During,
            target: DatingTarget::Window {
                tid: tid(2),
                from_ms: 1,
                to_ms: 2,
            },
        },
        agent(),
        ts(),
    );
    assert!(!smysl_core::surface::dating_has_surface_form(&d));
    let text = write_surface(None, &[Record::Dating(d)], &WriteContext::default());
    assert!(text.is_empty(), "{text}");
}

/// `Dating::offset` cannot be built on the wrong axis, because it does not take one.
#[test]
fn an_offset_is_only_meaningful_on_the_said_axis() {
    let d = Dating::offset(DatingTarget::Part(tid(1)), -1, agent(), ts());
    assert_eq!(d.axis, Axis::Said);
}

/// A window that selects nothing decodes and is reportable, rather than being refused.
#[test]
fn an_empty_window_decodes_and_says_it_selects_nothing() {
    let d = Dating::offset(
        DatingTarget::Window {
            tid: tid(1),
            from_ms: 200,
            to_ms: 200,
        },
        -1,
        agent(),
        ts(),
    );
    let bytes = to_cbor(&Record::Dating(d.clone()));
    assert!(
        from_cbor(&bytes).is_ok(),
        "a record that arrived must decode"
    );
    assert!(!d.target.selects_anything());
}

/// An Allen relation reads the same from either end, which rule E's graph needs.
#[test]
fn an_allen_relation_knows_its_own_inverse() {
    for a in Allen::ALL {
        assert_eq!(a.inverse().inverse(), *a);
    }
    assert_eq!(Allen::Before.inverse(), Allen::After);
    assert_eq!(Allen::During.inverse(), Allen::Contains);
    assert_eq!(Allen::Equals.inverse(), Allen::Equals);
    assert_eq!(Allen::Meets.inverse(), Allen::Meets);
}
