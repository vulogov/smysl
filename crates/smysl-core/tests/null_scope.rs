//! Where `null` is forbidden, and where it is data.
//!
//! `Dec::reject_null` carried the claim "called before every map value, so `null` never
//! reaches a type-specific reader that might tolerate it". The second half is not true, and
//! is not meant to be: `surface::payload::read_value` tolerates `null` deliberately, because a
//! payload is user data and an explicit JSON null is a value rather than an absent optional.
//!
//! Nothing was wrong with the code. What was wrong is that the boundary between the two
//! existed only in a sentence, and the sentence described the wrong scope. §3 constraint 5
//! reads "No `null` for an absent optional" — the qualifier is load bearing, and a reader
//! skimming for the rule will take away "no null".
//!
//! So the boundary is pinned here instead of asserted there. If someone later makes the
//! payload reader strict, or the kernel reader lax, one of these fails and says which
//! decision they have actually made.

use smysl_core::cbor::Dec;
use smysl_core::surface::{object_to_payload, parse_object, payload_to_object};

/// A unit core `{0:"s", 1:"g", 6:0, 200:<value>}`. Key 200 is above `keys::unit::HIGHEST`, so
/// it is an unknown key and takes the generic path — the one `reject_null` guards.
fn core_with_unknown(value: &[u8]) -> Vec<u8> {
    let mut v = vec![
        0xA4, 0x00, 0x61, b's', 0x01, 0x61, b'g', 0x06, 0x00, 0x18, 0xC8,
    ];
    v.extend_from_slice(value);
    v
}

fn walker_accepts(bytes: &[u8]) -> bool {
    let mut d = Dec::new(bytes);
    d.skip_item().is_ok() && d.remaining() == 0
}

#[test]
fn the_kernel_refuses_null_as_a_map_value() {
    // 0xF6 is null. An absent optional is omitted, so this is a violation rather than a value.
    assert!(
        !walker_accepts(&core_with_unknown(&[0xF6])),
        "a null reached a kernel map value"
    );
}

#[test]
fn the_kernel_still_accepts_the_same_shape_with_a_real_value() {
    // The control: without it the test above would pass if the walker refused everything.
    assert!(
        walker_accepts(&core_with_unknown(&[0x01])),
        "the same core with a non-null value should decode"
    );
}

#[test]
fn a_payload_admits_null_as_data() {
    // Deliberate, and the reason the claim on `reject_null` had to be narrowed: a payload is
    // user data, and `{"n": null}` is distinguishable from `{}` on purpose. They are different
    // payloads, hash to different uids, and both are canonical.
    let src = r#"{ "absent": null, "present": 1 }"#;
    let object = parse_object(src, 0).expect("a payload object with an explicit null parses");
    let bytes = object_to_payload(&object.value).expect("and encodes");
    assert!(
        bytes.contains(&0xF6),
        "the encoded payload should carry a real CBOR null"
    );

    let back = payload_to_object(&bytes).expect("and decodes again");
    assert_eq!(
        back.get("absent").map(|v| v.value.type_name()),
        Some("null"),
        "an explicit payload null must survive the round trip as null, not vanish"
    );
    assert!(
        back.get("present").is_some(),
        "and must not take its neighbours with it"
    );
}

/// The distinction that makes the two above consistent rather than contradictory: a payload
/// is stored in the unit core as a **byte string**, so the kernel decoder never walks into it.
/// A reader at C-Read sees opaque bytes. That is why the two rules can differ without two
/// implementations disagreeing about what a document is.
#[test]
fn a_payload_is_opaque_to_the_kernel_walker() {
    let src = r#"{ "absent": null }"#;
    let object = parse_object(src, 0).expect("parses");
    let payload = object_to_payload(&object.value).expect("encodes");

    // {8: <payload as a byte string>} — key 8 is `keys::unit::PAYLOAD`.
    let mut core = vec![0xA1, 0x08];
    assert!(
        payload.len() < 24,
        "test payload is short enough for a one-byte head"
    );
    core.push(0x40 | payload.len() as u8);
    core.extend_from_slice(&payload);

    assert!(
        walker_accepts(&core),
        "a payload carrying a null is opaque bytes to the kernel and must decode"
    );
}
