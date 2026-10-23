//! Rule N fixtures (SMYSL-2.3 A-12.1, Appendix D `schema/rule-n/`; SMYSL-2.5 §3.11, §5.7;
//! TX-P12 step 5).
//!
//! The exit is the rule N fixtures: one declaration series per violation — a name given a new
//! meaning, a retired name reintroduced, a widening without its paired key, a fork — beside a
//! widening that is declared, each with the codes `schema check` raises for it. The files are
//! what this test writes (`SMYSL_BLESS=1` rewrites them), and `cases.json` records the codes.

use std::collections::BTreeMap;
use std::path::PathBuf;

use smysl_core::schema::{
    encode_lenses, encode_shape, Cond, Field, FieldType, LensOp, LensStep, SchemaSet,
};
use smysl_core::surface::HValue;
use smysl_core::types::{Record, SchemaDecl};
use smysl_core::{from_cbor_seq, to_cbor_seq, Report, SchemaId};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/schema/rule-n")
}

fn id(s: &str) -> SchemaId {
    SchemaId::parse(s).unwrap()
}

fn s(v: &str) -> HValue {
    HValue::Str(v.into())
}

fn decl(v: u32, fields: &BTreeMap<String, Field>, steps: &[LensStep]) -> SchemaDecl {
    let mut d = SchemaDecl::new(id("x.rule/v1"), v);
    d.types = vec![id("x.rule/item")];
    d.payload_shape = Some(encode_shape(fields));
    if !steps.is_empty() {
        d.lenses = Some(encode_lenses(steps));
    }
    d
}

fn base() -> BTreeMap<String, Field> {
    let mut f = BTreeMap::new();
    f.insert(
        "rule:mode".into(),
        Field::enumeration(["plain", "loud", "quiet"]),
    );
    f.insert("rule:note".into(), Field::new(FieldType::Text));
    f.insert("rule:size".into(), Field::new(FieldType::Int));
    f
}

/// `quiet` split into `loud` plus `rule:hushed = true`; `loud` existed, so this widens it.
fn widening(paired: bool) -> Vec<SchemaDecl> {
    let when = Some(Cond::new("rule:mode", s("loud")));
    let mut ops = vec![LensOp::SplitValue {
        key: "rule:mode".into(),
        from: "quiet".into(),
        to: "loud".into(),
        add: [("rule:hushed".to_string(), HValue::Bool(true))].into(),
    }];
    if paired {
        ops.push(LensOp::Require {
            key: "rule:hushed".into(),
            when: when.clone(),
        });
        ops.push(LensOp::Default {
            key: "rule:hushed".into(),
            value: HValue::Bool(false),
            when: when.clone(),
        });
    }
    let mut f2 = base();
    f2.insert("rule:mode".into(), Field::enumeration(["plain", "loud"]));
    let mut hushed = Field::new(FieldType::Bool);
    if paired {
        hushed.required_from = Some(2);
        hushed.required_when = when.as_ref().cloned();
    }
    f2.insert("rule:hushed".into(), hushed);
    vec![
        decl(1, &base(), &[]),
        decl(2, &f2, &[LensStep::new(1, 2, ops)]),
    ]
}

/// `rule:note` retired at 2, brought back at 3.
fn reintroduced_key() -> Vec<SchemaDecl> {
    let mut f2 = base();
    let mut gone = Field::new(FieldType::Text);
    gone.retired_in = Some(2);
    f2.insert("rule:note".into(), gone);
    let f3 = base();
    vec![
        decl(1, &base(), &[]),
        decl(
            2,
            &f2,
            &[LensStep::new(
                1,
                2,
                vec![LensOp::Retire {
                    key: "rule:note".into(),
                }],
            )],
        ),
        decl(3, &f3, &[LensStep::new(2, 3, vec![])]),
    ]
}

/// `quiet` dropped at 2 and back at 3.
fn reintroduced_value() -> Vec<SchemaDecl> {
    let mut f2 = base();
    f2.insert("rule:mode".into(), Field::enumeration(["plain", "loud"]));
    vec![
        decl(1, &base(), &[]),
        decl(2, &f2, &[LensStep::new(1, 2, vec![])]),
        decl(3, &base(), &[LensStep::new(2, 3, vec![])]),
    ]
}

/// `quiet` mapped onto `plain`, which revision 1 already has.
fn mapped_onto() -> Vec<SchemaDecl> {
    let mut f2 = base();
    f2.insert("rule:mode".into(), Field::enumeration(["plain", "loud"]));
    vec![
        decl(1, &base(), &[]),
        decl(
            2,
            &f2,
            &[LensStep::new(
                1,
                2,
                vec![LensOp::MapValue {
                    key: "rule:mode".into(),
                    map: [("quiet".to_string(), "plain".to_string())].into(),
                }],
            )],
        ),
    ]
}

/// A key renamed onto one that exists, and a value mapped onto one that exists: either way
/// the survivor now means two things.
fn new_meaning() -> Vec<SchemaDecl> {
    let mut f2 = base();
    f2.remove("rule:note");
    vec![
        decl(1, &base(), &[]),
        decl(
            2,
            &f2,
            &[LensStep::new(
                1,
                2,
                vec![LensOp::RenameKey {
                    from: "rule:note".into(),
                    to: "rule:size".into(),
                }],
            )],
        ),
    ]
}

/// `rule:size` is `int` at 1 and `text` at 2.
fn type_change() -> Vec<SchemaDecl> {
    let mut f2 = base();
    f2.insert("rule:size".into(), Field::new(FieldType::Text));
    vec![
        decl(1, &base(), &[]),
        decl(2, &f2, &[LensStep::new(1, 2, vec![])]),
    ]
}

/// Two declarations of version 2 with different content.
fn fork() -> Vec<SchemaDecl> {
    let mut f2 = base();
    f2.insert("rule:extra".into(), Field::new(FieldType::Text));
    let mut f2b = base();
    f2b.insert("rule:other".into(), Field::new(FieldType::Text));
    vec![
        decl(1, &base(), &[]),
        decl(2, &f2, &[LensStep::new(1, 2, vec![])]),
        decl(2, &f2b, &[LensStep::new(1, 2, vec![])]),
    ]
}

fn files() -> Vec<(&'static str, Vec<SchemaDecl>)> {
    vec![
        ("widening-paired.cbor", widening(true)),
        ("widening-unpaired.cbor", widening(false)),
        ("reintroduced-key.cbor", reintroduced_key()),
        ("reintroduced-value.cbor", reintroduced_value()),
        ("new-meaning.cbor", new_meaning()),
        ("type-change.cbor", type_change()),
        ("fork.cbor", fork()),
    ]
}

/// The codes `schema check` raises over a file, sorted, one entry per diagnostic.
fn codes(records: &[Record]) -> Vec<String> {
    let mut report = Report::new();
    let set = SchemaSet::from_records(records, &mut report);
    let mut check = Report::new();
    set.check_rule_n(&id("x.rule/v1"), &mut check);
    let mut v: Vec<String> = check.iter().map(|d| d.code.as_str().to_string()).collect();
    v.sort();
    v
}

fn cases() -> String {
    let mut out = String::from("{\n  \"files\": [\n");
    let fs = files();
    for (i, (name, decls)) in fs.iter().enumerate() {
        let recs: Vec<Record> = decls.iter().cloned().map(Record::SchemaDecl).collect();
        let c = codes(&recs)
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "    {{\"file\": \"{name}\", \"codes\": [{c}]}}{}\n",
            if i + 1 < fs.len() { "," } else { "" }
        ));
    }
    out.push_str("  ]\n}\n");
    out
}

#[test]
fn the_fixtures_are_what_this_build_writes() {
    let bless = std::env::var("SMYSL_BLESS").is_ok_and(|v| v == "1");
    let d = dir();
    if bless {
        std::fs::create_dir_all(&d).unwrap();
    }
    for (name, decls) in files() {
        let recs: Vec<Record> = decls.into_iter().map(Record::SchemaDecl).collect();
        let bytes = to_cbor_seq(&recs);
        let path = d.join(name);
        if bless {
            std::fs::write(&path, &bytes).unwrap();
        } else {
            let want = std::fs::read(&path).unwrap_or_else(|_| {
                panic!("{} is missing; SMYSL_BLESS=1 writes it", path.display())
            });
            assert_eq!(want, bytes, "{name}");
        }
    }
    let path = d.join("cases.json");
    if bless {
        std::fs::write(&path, cases()).unwrap();
    } else {
        assert_eq!(std::fs::read_to_string(&path).unwrap(), cases());
    }
}

/// Each file raises exactly the codes its name promises, read from the committed bytes.
#[test]
fn every_violation_is_named_and_the_declared_widening_is_clean() {
    let expect: &[(&str, &[&str])] = &[
        ("widening-paired.cbor", &[]),
        ("widening-unpaired.cbor", &["SMY-E426", "SMY-E471"]),
        ("reintroduced-key.cbor", &["SMY-E426"]),
        ("reintroduced-value.cbor", &["SMY-E426"]),
        ("new-meaning.cbor", &["SMY-E426"]),
        ("type-change.cbor", &["SMY-E426"]),
        ("fork.cbor", &["SMY-W429"]),
    ];
    for (name, want) in expect {
        let bytes = std::fs::read(dir().join(name)).unwrap();
        let (recs, _) = from_cbor_seq(&bytes).unwrap();
        let got = codes(&recs);
        let want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
        assert_eq!(got, want, "{name}");
    }
}

/// A map onto an existing value gives it a second meaning.
#[test]
fn a_value_mapped_onto_an_existing_one_is_a_new_meaning() {
    let recs: Vec<Record> = mapped_onto().into_iter().map(Record::SchemaDecl).collect();
    assert_eq!(codes(&recs), vec!["SMY-E426".to_string()]);
}
