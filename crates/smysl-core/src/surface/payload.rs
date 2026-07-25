//! Unknown header keys, carried as deterministic CBOR in `payload` (rule X).
//!
//! §6 requires that unknown header keys be preserved rather than rejected, and rule X
//! requires a consumer to re-emit them verbatim. That means a lossless mapping in both
//! directions between the HJSON value model and deterministic CBOR.
//!
//! The kernel's own maps use small unsigned integer keys (§7.1 constraint 1). A payload
//! map cannot: its keys are author-chosen names. Text keys are therefore permitted
//! **inside a payload**, sorted by encoded key bytes so the determinism guarantee still
//! holds. That extends §7.1 rather than contradicting it - the kernel records themselves
//! are unaffected.

use crate::cbor::reader::Dec;
use crate::cbor::writer::{enc, Enc};
use crate::cbor::{major, NULL};
use crate::error::CodecError;
use crate::surface::hjson::{HObject, HValue, Spanned};
use crate::types::quantise;

/// Simple values used by payload encoding. `false`/`true` follow RFC 8949.
const FALSE: u8 = 0xF4;
const TRUE: u8 = 0xF5;

/// Encode leftover header keys as a deterministic CBOR map.
///
/// Returns `None` for an empty object, so a unit with no unknown keys carries no payload
/// at all rather than an empty one - two encodings of the same core would otherwise exist.
pub fn object_to_payload(o: &HObject) -> Option<Vec<u8>> {
    if o.is_empty() {
        return None;
    }
    let mut e = Enc::new();
    write_object(&mut e, o);
    Some(e.into_bytes())
}

fn write_object(e: &mut Enc, o: &HObject) {
    let mut entries: Vec<(Vec<u8>, Vec<u8>)> = o
        .iter()
        .map(|(k, v)| (enc(|e| e.text(&k.value)), enc(|e| write_value(e, &v.value))))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries.dedup_by(|a, b| a.0 == b.0);
    e.head(major::MAP, entries.len() as u64);
    for (k, v) in entries {
        e.raw(&k);
        e.raw(&v);
    }
}

fn write_value(e: &mut Enc, v: &HValue) {
    match v {
        HValue::Null => e.raw(&[NULL]),
        HValue::Bool(false) => e.raw(&[FALSE]),
        HValue::Bool(true) => e.raw(&[TRUE]),
        HValue::Int(i) if *i >= 0 => e.uint(*i as u64),
        HValue::Int(i) => e.head(major::NEGINT, (-1 - *i) as u64),
        HValue::Float(f) => e.f32q(*f as f32),
        HValue::Str(s) => e.text(s),
        HValue::Array(items) => {
            e.head(major::ARRAY, items.len() as u64);
            for i in items {
                write_value(e, &i.value);
            }
        }
        HValue::Object(o) => write_object(e, o),
    }
}

/// Decode a payload back into header keys, for re-emission to surface syntax.
pub fn payload_to_object(bytes: &[u8]) -> Result<HObject, CodecError> {
    let mut d = Dec::new(bytes);
    let o = read_object(&mut d)?;
    Ok(o)
}

fn read_object(d: &mut Dec<'_>) -> Result<HObject, CodecError> {
    let at = d.position();
    let n = d.map_head()?;
    let mut o = HObject::new();
    let mut prev: Option<String> = None;
    for _ in 0..n {
        let key = d.text()?.to_string();
        if let Some(p) = &prev {
            // Text keys sort by encoded bytes, which is length first then content.
            let ordered = (p.len(), p.as_str()) < (key.len(), key.as_str());
            if !ordered {
                return Err(CodecError::NonDeterministic {
                    at,
                    reason: crate::error::NonDetReason::UnsortedMapKeys,
                });
            }
        }
        prev = Some(key.clone());
        let v = read_value(d)?;
        let span = crate::diag::Span::new(0, 0);
        o.insert(Spanned::new(key, span), Spanned::new(v, span));
    }
    Ok(o)
}

fn read_value(d: &mut Dec<'_>) -> Result<HValue, CodecError> {
    let at = d.position();
    match d.peek_byte()? {
        NULL => {
            d.advance(1);
            Ok(HValue::Null)
        }
        FALSE => {
            d.advance(1);
            Ok(HValue::Bool(false))
        }
        TRUE => {
            d.advance(1);
            Ok(HValue::Bool(true))
        }
        b if b == crate::cbor::F32_HEAD => Ok(HValue::Float(d.f32q()? as f64)),
        b => match b >> 5 {
            major::UINT => {
                let v = d.uint()?;
                i64::try_from(v)
                    .map(HValue::Int)
                    .map_err(|_| CodecError::MalformedEnvelope { at })
            }
            major::NEGINT => {
                let (_, arg) = d.head()?;
                i64::try_from(arg)
                    .map(|a| HValue::Int(-1 - a))
                    .map_err(|_| CodecError::MalformedEnvelope { at })
            }
            major::TEXT => Ok(HValue::Str(d.text()?.to_string())),
            major::ARRAY => {
                let items = d.array(read_value)?;
                let span = crate::diag::Span::new(0, 0);
                Ok(HValue::Array(
                    items.into_iter().map(|v| Spanned::new(v, span)).collect(),
                ))
            }
            major::MAP => Ok(HValue::Object(read_object(d)?)),
            _ => Err(CodecError::MalformedEnvelope { at }),
        },
    }
}

/// Whether a value survives the payload round trip unchanged.
///
/// Floats do not: they are quantised to 1/1024 on the way in, because a payload is hash
/// input and hash stability must not depend on the float path that produced the number.
pub fn is_lossless(v: &HValue) -> bool {
    match v {
        HValue::Float(f) => quantise(*f as f32) as f64 == *f,
        HValue::Array(a) => a.iter().all(|i| is_lossless(&i.value)),
        HValue::Object(o) => o.iter().all(|(_, v)| is_lossless(&v.value)),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::hjson::parse_object;

    fn obj(src: &str) -> HObject {
        parse_object(src, 0).unwrap().value
    }

    fn round_trip(src: &str) -> HObject {
        let o = obj(src);
        payload_to_object(&object_to_payload(&o).unwrap()).unwrap()
    }

    #[test]
    fn an_empty_object_carries_no_payload() {
        assert!(object_to_payload(&obj("{}")).is_none());
    }

    #[test]
    fn scalars_round_trip() {
        let o = round_trip(r#"{ s: hello, i: 42, neg: -7, t: true, f: false, n: null }"#);
        assert_eq!(o.get("s").unwrap().value.as_str(), Some("hello"));
        assert_eq!(o.get("i").unwrap().value.as_int(), Some(42));
        assert_eq!(o.get("neg").unwrap().value.as_int(), Some(-7));
        assert_eq!(o.get("t").unwrap().value.as_bool(), Some(true));
        assert_eq!(o.get("f").unwrap().value.as_bool(), Some(false));
        assert_eq!(o.get("n").unwrap().value, HValue::Null);
    }

    #[test]
    fn arrays_and_nested_objects_round_trip() {
        let o = round_trip("{ a: [1, 2, 3], nested: { x: y, deep: [{ k: v }] } }");
        assert_eq!(o.get("a").unwrap().value.as_array().unwrap().len(), 3);
        let n = o.get("nested").unwrap().value.as_object().unwrap();
        assert_eq!(n.get("x").unwrap().value.as_str(), Some("y"));
        assert_eq!(n.get("deep").unwrap().value.as_array().unwrap().len(), 1);
    }

    #[test]
    fn keys_are_emitted_in_encoded_byte_order() {
        let bytes = object_to_payload(&obj("{ zz: 1, a: 2, mmm: 3 }")).unwrap();
        let o = payload_to_object(&bytes).unwrap();
        // Encoded byte order is length first, then content: `a` < `zz` < `mmm`.
        assert_eq!(o.keys().collect::<Vec<_>>(), ["a", "zz", "mmm"]);
    }

    /// Authored order must not leak into the bytes, because the payload is hash input.
    #[test]
    fn authored_order_does_not_change_the_encoding() {
        let a = object_to_payload(&obj("{ x: 1, y: 2 }")).unwrap();
        let b = object_to_payload(&obj("{ y: 2, x: 1 }")).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn duplicate_keys_collapse_to_one_entry() {
        let bytes = object_to_payload(&obj("{ x: 1, x: 2 }")).unwrap();
        assert_eq!(payload_to_object(&bytes).unwrap().len(), 1);
    }

    #[test]
    fn floats_are_quantised_on_the_way_in() {
        let o = round_trip("{ f: 0.6 }");
        assert_eq!(o.get("f").unwrap().value.as_f64(), Some(614.0 / 1024.0));
        assert!(!is_lossless(&HValue::Float(0.6)));
        assert!(is_lossless(&HValue::Float(614.0 / 1024.0)));
    }

    #[test]
    fn losslessness_is_reported_recursively() {
        assert!(is_lossless(
            &obj("{ a: [1, hello] }").get("a").unwrap().value
        ));
        let o = obj("{ a: [0.6] }");
        assert!(!is_lossless(&o.get("a").unwrap().value));
    }

    #[test]
    fn an_unsorted_payload_map_is_rejected() {
        let mut e = Enc::new();
        e.head(major::MAP, 2);
        e.text("zz");
        e.uint(1);
        e.text("a");
        e.uint(2);
        let bytes = e.into_bytes();
        assert!(payload_to_object(&bytes).is_err());
    }

    #[test]
    fn a_non_map_payload_is_rejected() {
        assert!(payload_to_object(&[0x01]).is_err());
        assert!(payload_to_object(&[]).is_err());
    }

    #[test]
    fn multibyte_text_survives() {
        let o = round_trip("{ s: \"caf\u{e9} \u{4f60}\u{597d}\" }");
        assert_eq!(
            o.get("s").unwrap().value.as_str(),
            Some("caf\u{e9} \u{4f60}\u{597d}")
        );
    }

    #[test]
    fn negative_integers_round_trip_at_the_boundary() {
        for v in [-1i64, -24, -25, -256, -65_537, i32::MIN as i64] {
            let o = round_trip(&format!("{{ n: {v} }}"));
            assert_eq!(o.get("n").unwrap().value.as_int(), Some(v), "{v}");
        }
    }
}
