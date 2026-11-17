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
use smysl_tel::evidence::{summarize, DEFAULT_QS};
use smysl_tel::json::{self, Json};
use smysl_tel::sketch::{Mapping, Sketch};

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

/// Every case's sketch, at three accuracies, quantiles at the default ranks and the extremes,
/// pinned in `sketch.tsv`. Written on one architecture and compared on every other CI runs:
/// the cross-architecture half of GE-M3 for the one detector TL-P4 ships. `SMYSL_BLESS=1`
/// rewrites it.
#[test]
fn sketch_quantiles_are_the_same_bytes_everywhere() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/tel/window");
    let doc = json::parse(&std::fs::read_to_string(dir.join("cases.json")).unwrap()).unwrap();
    let Some(Json::Array(cases)) = doc.get("cases") else {
        panic!("cases.json has no cases")
    };
    let mut ranks: Vec<f64> = vec![0.0];
    ranks.extend(DEFAULT_QS.iter().map(|q| q.parse::<f64>().unwrap()));
    ranks.push(1.0);
    let mut out = String::from(
        "# RFC SMYSL-3 §9.3: DDSketch quantiles of fixtures/tel/window/cases.json, per case and alpha:\n\
         # ranks 0, 0.5, 0.9, 0.95, 0.99, 1. Written by crates/smysl-tel/tests/window_cases.rs.\n",
    );
    for alpha in ["0.001", "0.01", "0.05"] {
        let m = Mapping::new(Decimal::parse_token(alpha).unwrap()).unwrap();
        for (n, c) in cases.iter().enumerate() {
            let Some(Json::Array(values)) = c.get("values") else {
                panic!("case {n}")
            };
            let mut s = Sketch::new(m.clone());
            for v in values {
                s.add(Decimal::parse_token(v.as_str().unwrap()).unwrap());
            }
            let qs: Vec<String> = ranks
                .iter()
                .map(|r| {
                    s.quantile(*r)
                        .unwrap()
                        .map_or("-".into(), |d| d.to_string())
                })
                .collect();
            out.push_str(&format!("{alpha}\t{n}\t{}\n", qs.join("\t")));
        }
    }
    let path = dir.join("sketch.tsv");
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::write(&path, &out).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).expect("sketch.tsv; SMYSL_BLESS=1 writes it");
    assert!(out == want, "fixtures/tel/window/sketch.tsv moved");
}
