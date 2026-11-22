//! The decoder against what Bund2 0.56.0 printed, captured in BQ-P1 (RFC SMYSL-4 §7), and the
//! probe against a real binary when `BUND2_BIN` names one.
//!
//! `fixtures/render-0.56.0.txt` is a script's whole output under `--report trace --nocolor`:
//! `smysl.texts` and `smysl.status` of a one-text library, a LIST of every scalar type with a
//! string of quotes, backslashes, a box-drawing bar and a non-ASCII mark in it, and a STRING on
//! top, between the encoder's two sentinels for the tag `0123456789abcdef`.

use smysl_bund::decode::{parse_value, read_output};
use smysl_bund::{BundError, QueryValue};

const RENDER: &str = include_str!("fixtures/render-0.56.0.txt");
const REFUSAL: &str = include_str!("fixtures/refusal-0.56.0.txt");
const TAG: &str = "0123456789abcdef";

fn rows() -> Vec<QueryValue> {
    RENDER
        .lines()
        .filter_map(|l| l.strip_prefix("│ "))
        .map(|l| parse_value(l.trim_end().trim_end_matches('│').trim_end()).unwrap())
        .collect()
}

#[test]
fn the_top_of_the_stack_is_the_answer() {
    let top = read_output(RENDER, TAG).unwrap();
    let text = match top {
        QueryValue::Text(s) => s,
        QueryValue::Uid(u) => u.to_string(),
        other => panic!("{other:?}"),
    };
    assert_eq!(text, "b3:jx3m2x5dm5ptzdj3pxpgmgafhq");
}

#[test]
fn every_row_reads() {
    let rows = rows();
    assert_eq!(rows.len(), 4);
    let QueryValue::Map(texts) = &rows[0] else {
        panic!("{:?}", rows[0])
    };
    assert_eq!(
        texts["notes"],
        QueryValue::List(vec![QueryValue::Text(
            "m3:njjx4rwuyulrwmnghh2fvbrehhiuarlhb6nfe2qgzrcv3exlf2nq".into()
        )])
    );
    // Bund2's literal syntax keeps a backslash and the character after it. And inside `[ … ]`
    // a word is not run: `true` and `nodata` there are the STRINGs of their names.
    assert_eq!(
        rows[2],
        QueryValue::List(vec![
            QueryValue::Int(1),
            QueryValue::Int(-7),
            QueryValue::Text("q\\\"uo\\\\te │ ✓".into()),
            QueryValue::Float(0.1 + 0.2),
            QueryValue::Float(1e300),
            QueryValue::Float(-0.0),
            QueryValue::Text("true".into()),
            QueryValue::Text("nodata".into()),
            QueryValue::List(vec![QueryValue::Int(2)]),
        ])
    );
}

#[test]
fn a_refusal_reaches_the_caller_classified() {
    match read_output(REFUSAL, TAG) {
        Err(BundError::InvalidScriptOrArgs { code, message }) => {
            assert_eq!(code, "SM03");
            assert_eq!(message, "SMYSL.GET: uid b3:x is not a uid");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_probe_against_a_real_binary() {
    let Some(bin) = std::env::var_os("BUND2_BIN") else {
        eprintln!("BUND2_BIN not set; skipped");
        return;
    };
    let info = smysl_bund::probe::probe_bund_binary(Some(std::path::Path::new(&bin))).unwrap();
    assert!(info.has_smysl);
}
