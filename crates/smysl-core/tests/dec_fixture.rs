//! `fixtures/tel/dec/cases.json` (RFC SMYSL-3 §5): `dec` against a second reading of the RFC.
//!
//! The file is written by `generate.mjs` beside it, in JavaScript. Its binary64 cases are what
//! an engine's own `String(x)` says, which is the ECMA-262 `Number::toString` that §5.3 names,
//! so the claim in `Decimal::from_f64` that Rust's shortest formatting makes the same choice is
//! checked here rather than assumed. The token, spelling and CBOR cases are the generator's own
//! reading of §5.2–§5.3, written without looking at `dec.rs`. The three ports read the same file
//! (C-Tel-Read), so it is also what they are checked against.
//!
//! Read through the surface's HJSON reader, a superset of JSON, because `smysl-core` takes no
//! JSON dependency for a test.

use std::path::Path;

use smysl_core::surface::hjson::{parse_object, HObject, HValue};
use smysl_core::{Code, DecError, Decimal};

fn cases() -> HObject {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/tel/dec/cases.json");
    let src = std::fs::read_to_string(&path).expect("fixtures/tel/dec/cases.json");
    parse_object(&src, 0).expect("the fixture is JSON").value
}

fn list<'a>(o: &'a HObject, key: &str) -> Vec<&'a HObject> {
    match &o.get(key).unwrap_or_else(|| panic!("no `{key}`")).value {
        HValue::Array(items) => items
            .iter()
            .map(|i| match &i.value {
                HValue::Object(o) => o,
                other => panic!("{key}: {other:?}"),
            })
            .collect(),
        other => panic!("{key}: {other:?}"),
    }
}

fn text<'a>(o: &'a HObject, key: &str) -> &'a str {
    match &o.get(key).unwrap_or_else(|| panic!("no `{key}`")).value {
        HValue::Str(s) => s,
        other => panic!("{key}: {other:?}"),
    }
}

fn int(o: &HObject, key: &str) -> i64 {
    match &o.get(key).unwrap_or_else(|| panic!("no `{key}`")).value {
        HValue::Int(i) => *i,
        other => panic!("{key}: {other:?}"),
    }
}

fn expected(o: &HObject) -> Decimal {
    let m: i128 = text(o, "m").parse().expect("m is an integer");
    Decimal::new(m, int(o, "e")).expect("the fixture's pairs are in range")
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

/// Every double is the decimal a JavaScript engine's `String(x)` selects.
#[test]
fn a_binary64_is_the_decimal_ecma_262_selects() {
    let c = cases();
    let f64s = list(&c, "f64");
    assert!(f64s.len() > 2000, "{} cases", f64s.len());
    for case in f64s {
        let bits = u64::from_str_radix(text(case, "bits"), 16).expect("bits");
        let x = f64::from_bits(bits);
        assert_eq!(
            Decimal::from_f64(x),
            Ok(expected(case)),
            "{x:e} (bits {bits:016x})"
        );
    }
}

#[test]
fn tokens_are_read_as_the_rfc_says() {
    let c = cases();
    for case in list(&c, "tokens") {
        let token = text(case, "token");
        let got = Decimal::parse_token(token);
        match case.get("error").map(|v| &v.value) {
            Some(HValue::Str(kind)) => {
                let ok = match kind.as_str() {
                    "syntax" => matches!(got, Err(DecError::Syntax(_))),
                    "not-a-value" => got == Err(DecError::NotAValue),
                    "out-of-range" => got == Err(DecError::OutOfRange),
                    other => panic!("an error kind the fixture should not have: {other}"),
                };
                assert!(ok, "{token:?}: expected {kind}, got {got:?}");
            }
            _ => assert_eq!(got, Ok(expected(case)), "{token:?}"),
        }
    }
}

/// One spelling and one encoding per value, and each reads back to the value.
#[test]
fn spellings_and_encodings_are_the_rfcs() {
    let c = cases();
    for case in list(&c, "spellings") {
        let v = expected(case);
        let spelled = text(case, "text");
        assert_eq!(v.to_string(), spelled);
        assert_eq!(spelled.parse::<Decimal>(), Ok(v), "{spelled}");
        let bytes = hex(text(case, "cbor"));
        assert_eq!(v.to_cbor(), bytes, "{spelled}");
        assert_eq!(Decimal::from_cbor(&bytes), Ok(v), "{spelled}");
    }
}

#[test]
fn non_canonical_cbor_is_e602() {
    let c = cases();
    for case in list(&c, "refused") {
        let err = Decimal::from_cbor(&hex(text(case, "cbor"))).expect_err(text(case, "why"));
        assert_eq!(err.code(), Some(Code::E602), "{}", text(case, "why"));
    }
}
