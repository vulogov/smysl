//! `fixtures/tel/window/cases.json` (RFC SMYSL-3 §9.3): exact sums and means against an
//! independent oracle.
//!
//! The cases were computed by `gen_cases.py` with Python's exact rationals: the sum of an
//! integer series exactly, any other sum and every mean as the rational rounded once to the
//! nearest binary64, ties to even, spelt by §5.3. `null` is a sum outside `dec`'s range or a
//! value past binary64's. Four hundred windows, from one sample to forty, with exponents from
//! −400 to 400 and mantissas to 2⁶⁴ − 1.

use std::path::Path;

use smysl_core::Decimal;
use smysl_series::Sample;
use smysl_tel::evidence::summarize;
use smysl_tel::json::{self, Json};

#[test]
fn sums_and_means_agree_with_exact_rationals() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/tel/window/cases.json");
    let doc = json::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
    let Some(Json::Array(cases)) = doc.get("cases") else {
        panic!("cases.json has no cases")
    };
    let spelt = |v: Option<&Json>| match v {
        Some(Json::String(s)) => Some(Decimal::parse_token(s).unwrap()),
        _ => None,
    };
    let mut checked = 0;
    for (n, c) in cases.iter().enumerate() {
        let Some(Json::Array(values)) = c.get("values") else {
            panic!("case {n}")
        };
        let samples: Vec<Sample> = values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let t = v.as_str().unwrap();
                Sample::new(i as u64, None, Decimal::parse_token(t).unwrap(), 0).unwrap()
            })
            .collect();
        let s = summarize(&samples);
        assert_eq!(s.count, values.len() as u64);
        assert_eq!(s.sum, spelt(c.get("sum")), "case {n}: sum of {values:?}");
        assert_eq!(s.mean, spelt(c.get("mean")), "case {n}: mean of {values:?}");
        checked += 1;
    }
    assert_eq!(checked, 400);
}
