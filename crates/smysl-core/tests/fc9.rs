//! FC-9: schema declaration keys 4 and 5 decoded, and written back byte for byte (RFC SMYSL-2.5
//! §3.8, SMYSL-2.3 A-7 and Appendix B; TX-P12 step 1).
//!
//! The exit is L9 on fixtures — `encode_lenses(decode(bytes)) == bytes` — and the round-trip
//! suite green. `fixtures/schema/fc9/` is what the Python, JavaScript and Go suites are held to
//! (§5.8), so the generator is the test, as `gen_library_fixtures.rs` is: it rebuilds every file
//! from source and compares, and `SMYSL_BLESS=1` rewrites them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use smysl_core::schema::{
    decode, encode_lenses, encode_shape, Cond, Field, FieldType, Fields, LensOp, LensStep,
    SchemaSet,
};
use smysl_core::surface::HValue;
use smysl_core::types::SchemaDecl;
use smysl_core::{from_cbor_seq, to_cbor, to_cbor_seq, Code, Record, Report, SchemaId};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/schema/fc9")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn id(s: &str) -> SchemaId {
    SchemaId::parse(s).unwrap()
}

fn s(v: &str) -> HValue {
    HValue::Str(v.into())
}

/// `x.text/v1` as SMYSL-2.3 Appendix C declares it, `fields:` included — the declaration
/// `smysl_core::schemas::text::decl` cannot carry until A-7, because its table is in code.
fn text_v1() -> SchemaDecl {
    let mut d = smysl_core::schemas::text::decl();
    let mut f = BTreeMap::new();
    let mut holder = Field::new(FieldType::Uid);
    holder.literals = vec!["text".into()];
    f.insert("text:holder".into(), holder);
    f.insert(
        "text:mode".into(),
        Field::enumeration([
            "asserted",
            "reported",
            "hypothetical",
            "negated",
            "interrogative",
            "imperative",
            "quoted",
        ]),
    );
    f.insert("text:speaker".into(), Field::new(FieldType::Uid));
    f.insert("text:locator".into(), Field::new(FieldType::Text));
    f.insert("text:when".into(), Field::new(FieldType::Edtf));
    f.insert(
        "text:kind".into(),
        Field::enumeration(["person", "place", "group", "work", "object", "concept"]),
    );
    f.insert("meta:field".into(), Field::new(FieldType::Text));
    f.insert("meta:value".into(), Field::new(FieldType::Text));
    d.payload_shape = Some(encode_shape(&f));
    d
}

/// A three-revision history of `x.demo/v1` exercising every closed op:
/// 1→2 renames a key and a type, maps a value, and adds a required key with a default;
/// 2→3 widens a value with its paired key, and retires a key.
pub fn demo_series() -> Vec<SchemaDecl> {
    let mut f1 = BTreeMap::new();
    f1.insert("demo:name".into(), Field::new(FieldType::Text));
    f1.insert(
        "demo:state".into(),
        Field::enumeration(["open", "shut", "ajar"]),
    );
    f1.insert("demo:old".into(), Field::new(FieldType::Text));
    let mut d1 = SchemaDecl::new(id("x.demo/v1"), 1);
    d1.types = vec![id("x.demo/item")];
    d1.payload_shape = Some(encode_shape(&f1));

    let step12 = LensStep::new(
        1,
        2,
        vec![
            LensOp::RenameType {
                from: id("x.demo/item"),
                to: id("x.demo/thing"),
            },
            LensOp::RenameKey {
                from: "demo:name".into(),
                to: "demo:title".into(),
            },
            LensOp::MapValue {
                key: "demo:state".into(),
                map: [("shut".to_string(), "closed".to_string())].into(),
            },
            LensOp::Require {
                key: "demo:rank".into(),
                when: None,
            },
            LensOp::Default {
                key: "demo:rank".into(),
                value: HValue::Int(0),
                when: None,
            },
        ],
    );
    let mut f2 = BTreeMap::new();
    f2.insert("demo:title".into(), Field::new(FieldType::Text));
    let mut name = Field::new(FieldType::Text);
    name.retired_in = Some(2);
    f2.insert("demo:name".into(), name);
    f2.insert(
        "demo:state".into(),
        Field::enumeration(["open", "closed", "ajar"]),
    );
    f2.insert("demo:old".into(), Field::new(FieldType::Text));
    let mut rank = Field::new(FieldType::Int);
    rank.required_from = Some(2);
    f2.insert("demo:rank".into(), rank);
    let mut d2 = SchemaDecl::new(id("x.demo/v1"), 2);
    d2.types = vec![id("x.demo/thing")];
    d2.payload_shape = Some(encode_shape(&f2));
    d2.lenses = Some(encode_lenses(&[step12]));

    let when = Some(Cond::new("demo:state", s("open")));
    let step23 = LensStep::new(
        2,
        3,
        vec![
            LensOp::SplitValue {
                key: "demo:state".into(),
                from: "ajar".into(),
                to: "open".into(),
                add: [("demo:gap".to_string(), s("some"))].into(),
            },
            LensOp::Require {
                key: "demo:gap".into(),
                when: when.clone(),
            },
            LensOp::Default {
                key: "demo:gap".into(),
                value: s("none"),
                when,
            },
            LensOp::Retire {
                key: "demo:old".into(),
            },
        ],
    );
    let mut f3 = f2.clone();
    f3.insert("demo:state".into(), Field::enumeration(["open", "closed"]));
    let mut gap = Field::enumeration(["none", "some"]);
    gap.required_from = Some(3);
    gap.required_when = Some(Cond::new("demo:state", s("open")));
    f3.insert("demo:gap".into(), gap);
    let mut old = Field::new(FieldType::Text);
    old.retired_in = Some(3);
    f3.insert("demo:old".into(), old);
    let mut d3 = SchemaDecl::new(id("x.demo/v1"), 3);
    d3.types = vec![id("x.demo/thing")];
    d3.payload_shape = Some(encode_shape(&f3));
    d3.lenses = Some(encode_lenses(&[step23]));

    vec![d1, d2, d3]
}

/// An op tag no build knows, between two known ones.
fn unknown_op() -> SchemaDecl {
    let known = LensStep::new(
        1,
        2,
        vec![
            LensOp::RenameKey {
                from: "u:a".into(),
                to: "u:b".into(),
            },
            LensOp::Unknown {
                tag: "transpose".into(),
                // {0: "transpose", 1: "u:b"}
                bytes: vec![
                    0xa2, 0x00, 0x69, b't', b'r', b'a', b'n', b's', b'p', b'o', b's', b'e', 0x01,
                    0x63, b'u', b':', b'b',
                ],
            },
        ],
    );
    let mut d = SchemaDecl::new(id("x.unknown/v1"), 2);
    d.types = vec![id("x.unknown/item")];
    d.lenses = Some(encode_lenses(&[known]));
    d
}

/// A key 4 that is CBOR but not A-7's structure, and a key 5 that is not an array of steps.
fn opaque() -> SchemaDecl {
    let mut d = SchemaDecl::new(id("x.opaque/v1"), 1);
    d.payload_shape = Some(vec![0xa1, 0x00, 0x01]); // {0: 1}
    d.lenses = Some(vec![0x82, 0x01, 0x02]); // [1, 2]
    d
}

fn files() -> Vec<(&'static str, Vec<SchemaDecl>)> {
    vec![
        ("text-v1.cbor", vec![text_v1()]),
        ("series.cbor", demo_series()),
        ("unknown-op.cbor", vec![unknown_op()]),
        ("opaque.cbor", vec![opaque()]),
    ]
}

fn records(bytes: &[u8]) -> Vec<Record> {
    let (v, used) = from_cbor_seq(bytes).unwrap();
    assert_eq!(used, bytes.len());
    v
}

fn seq(decls: &[SchemaDecl]) -> Vec<u8> {
    let records: Vec<Record> = decls.iter().cloned().map(Record::SchemaDecl).collect();
    to_cbor_seq(&records)
}

/// What a reader of each file must find: per declaration, the field count or `opaque`, the op
/// tags of each step, and the codes decoding reports; and the chain hashes of the series.
fn cases() -> String {
    let mut out = String::from("{\n  \"files\": [\n");
    let all = files();
    for (i, (name, decls)) in all.iter().enumerate() {
        out.push_str(&format!("    {{\"file\": \"{name}\", \"decls\": [\n"));
        for (j, d) in decls.iter().enumerate() {
            let mut r = Report::new();
            let rev = decode(d, &mut r);
            let fields = match &rev.fields {
                Fields::Typed(m) => m.len().to_string(),
                Fields::Opaque => "\"opaque\"".into(),
                _ => "null".into(),
            };
            let steps: Vec<String> = rev
                .lenses
                .iter()
                .map(|s| {
                    let tags: Vec<String> =
                        s.ops.iter().map(|o| format!("\"{}\"", o.tag())).collect();
                    format!(
                        "{{\"from\": {}, \"to\": {}, \"ops\": [{}], \"bytes\": \"{}\"}}",
                        s.from,
                        s.to,
                        tags.join(", "),
                        hex(s.bytes())
                    )
                })
                .collect();
            let codes: Vec<String> = r.iter().map(|d| format!("\"{}\"", d.code)).collect();
            out.push_str(&format!(
                "      {{\"id\": \"{}\", \"version\": {}, \"fields\": {fields}, \"lenses_opaque\": {}, \
                 \"steps\": [{}], \"codes\": [{}]}}{}\n",
                d.id.as_str(),
                d.version,
                rev.lenses_opaque,
                steps.join(", "),
                codes.join(", "),
                if j + 1 < decls.len() { "," } else { "" }
            ));
        }
        out.push_str(&format!(
            "    ]}}{}\n",
            if i + 1 < all.len() { "," } else { "" }
        ));
    }
    out.push_str("  ],\n  \"chains\": [\n");
    let mut r = Report::new();
    let set = SchemaSet::from_decls(&demo_series(), &mut r);
    let pairs = [(1, 2), (2, 3), (1, 3), (2, 2)];
    for (i, (a, b)) in pairs.iter().enumerate() {
        let c = set.chain(&id("x.demo/v1"), *a, *b).unwrap();
        out.push_str(&format!(
            "    {{\"id\": \"x.demo/v1\", \"from\": {a}, \"to\": {b}, \"hash\": \"{}\"}}{}\n",
            hex(&c.hash),
            if i + 1 < pairs.len() { "," } else { "" }
        ));
    }
    out.push_str("  ]\n}\n");
    out
}

#[test]
fn the_fixtures_are_what_this_build_writes() {
    let bless = std::env::var_os("SMYSL_BLESS").is_some();
    let mut want: Vec<(String, Vec<u8>)> = files()
        .into_iter()
        .map(|(n, d)| (n.to_string(), seq(&d)))
        .collect();
    want.push(("cases.json".into(), cases().into_bytes()));
    if bless {
        std::fs::create_dir_all(dir()).unwrap();
    }
    for (name, bytes) in want {
        let path = dir().join(&name);
        if bless {
            std::fs::write(&path, &bytes).unwrap();
            continue;
        }
        let have = std::fs::read(&path)
            .unwrap_or_else(|_| panic!("fixtures/schema/fc9/{name} missing; SMYSL_BLESS=1"));
        assert!(
            have == bytes,
            "fixtures/schema/fc9/{name} differs from what this build writes"
        );
    }
}

/// L9 on the fixtures: every file decodes, re-encodes to its own bytes, and each declaration's
/// key 5 is exactly `encode_lenses` of its decoded steps.
#[test]
fn l9_byte_stability_on_every_fixture() {
    for (name, decls) in files() {
        let bytes = seq(&decls);
        let back = records(&bytes);
        assert_eq!(to_cbor_seq(&back), bytes, "{name} round trip");
        for r in &back {
            let Record::SchemaDecl(d) = r else { panic!() };
            let rev = decode(d, &mut Report::new());
            if let (Some(raw), false) = (&d.lenses, rev.lenses_opaque) {
                assert_eq!(&encode_lenses(&rev.lenses), raw, "{name} {}", d.version);
            }
        }
    }
}

#[test]
fn appendix_c_fields_decode_typed() {
    let mut r = Report::new();
    let rev = decode(&text_v1(), &mut r);
    assert!(r.is_empty(), "{r:?}");
    let t = rev.fields.typed().unwrap();
    assert_eq!(t.len(), 8);
    assert_eq!(t["text:mode"].ty, FieldType::Enum);
    assert_eq!(t["text:mode"].values.len(), 7);
    assert_eq!(t["text:holder"].literals, vec!["text".to_string()]);
    assert!(t["text:holder"].admits(&s("text")));
    assert_eq!(t["text:when"].ty, FieldType::Edtf);
}

#[test]
fn a_list_type_parses_and_prints() {
    let t = FieldType::parse("list<text>");
    assert_eq!(t, FieldType::List(Box::new(FieldType::Text)));
    assert_eq!(t.as_string(), "list<text>");
    assert_eq!(
        FieldType::parse("decimal"),
        FieldType::Other("decimal".into())
    );
}

#[test]
fn every_closed_op_decodes_as_itself() {
    let decls = demo_series();
    let mut r = Report::new();
    let tags: Vec<String> = decls
        .iter()
        .flat_map(|d| decode(d, &mut r).lenses)
        .flat_map(|s| s.ops)
        .map(|o| o.tag().to_string())
        .collect();
    assert!(r.is_empty(), "{r:?}");
    for t in [
        "rename-type",
        "rename-key",
        "map-value",
        "split-value",
        "require",
        "default",
        "retire",
    ] {
        assert!(tags.iter().any(|x| x == t), "{t}");
    }
    // The steps built by `LensStep::new` and the steps decoded are the same values.
    let built = decode(&decls[2], &mut r).lenses;
    let again = decode(
        &match records(&seq(&decls[2..])).remove(0) {
            Record::SchemaDecl(d) => d,
            _ => unreachable!(),
        },
        &mut r,
    )
    .lenses;
    assert_eq!(built, again);
}

#[test]
fn an_unknown_op_makes_its_step_unusable_and_survives() {
    let d = unknown_op();
    let mut r = Report::new();
    let rev = decode(&d, &mut r);
    assert_eq!(r.count(Code::W469), 1);
    assert_eq!(rev.lenses.len(), 1);
    assert!(!rev.lenses[0].is_usable());
    assert!(matches!(&rev.lenses[0].ops[1], LensOp::Unknown { tag, .. } if tag == "transpose"));
    let rec = Record::SchemaDecl(d);
    assert_eq!(to_cbor(&records(&to_cbor(&rec))[0]), to_cbor(&rec));
}

#[test]
fn a_known_tag_with_a_key_this_build_does_not_know_is_unknown() {
    // {0: "retire", 1: "k", 2: true}: a later `retire` with an option is not this `retire`.
    let op = vec![
        0xa3, 0x00, 0x66, b'r', b'e', b't', b'i', b'r', b'e', 0x01, 0x61, b'k', 0x02, 0xf5,
    ];
    let step = LensStep::new(
        1,
        2,
        vec![LensOp::Unknown {
            tag: "retire".into(),
            bytes: op,
        }],
    );
    let mut d = SchemaDecl::new(id("x.k/v1"), 2);
    d.lenses = Some(encode_lenses(&[step]));
    let mut r = Report::new();
    let rev = decode(&d, &mut r);
    assert_eq!(r.count(Code::W469), 1);
    assert!(matches!(&rev.lenses[0].ops[0], LensOp::Unknown { tag, .. } if tag == "retire"));
}

#[test]
fn a_shape_or_lens_array_that_does_not_match_is_opaque_and_said() {
    let d = opaque();
    let mut r = Report::new();
    let rev = decode(&d, &mut r);
    assert_eq!(r.count(Code::W430), 2);
    assert_eq!(rev.fields, Fields::Opaque);
    assert!(rev.lenses_opaque);
    assert!(rev.lenses.is_empty());
    // Opaque is not lost: the record re-encodes to what it was.
    let rec = Record::SchemaDecl(d);
    assert_eq!(to_cbor(&records(&to_cbor(&rec))[0]), to_cbor(&rec));
}

#[test]
fn a_step_that_does_not_go_up_is_not_a_step() {
    let mut d = SchemaDecl::new(id("x.down/v1"), 2);
    d.lenses = Some(encode_lenses(&[LensStep::new(2, 1, vec![])]));
    let mut r = Report::new();
    assert!(decode(&d, &mut r).lenses_opaque);
    assert_eq!(r.count(Code::W430), 1);
}

#[test]
fn the_highest_version_is_current_and_a_fork_has_none() {
    let mut r = Report::new();
    let set = SchemaSet::from_decls(&demo_series(), &mut r);
    assert!(r.is_empty());
    assert_eq!(set.current(&id("x.demo/v1")).unwrap().version, 3);
    assert_eq!(set.revisions(&id("x.demo/v1")).len(), 3);
    assert_eq!(set.governing(&id("x.demo/thing")), Some(&id("x.demo/v1")));

    // The same declaration twice is one declaration.
    let mut twice = demo_series();
    twice.push(demo_series()[1].clone());
    let set = SchemaSet::from_decls(&twice, &mut r);
    assert!(r.is_empty());
    assert_eq!(set.revisions(&id("x.demo/v1")).len(), 3);

    // Two different declarations of version 2 are a fork.
    let mut forked = demo_series();
    let mut other = forked[1].clone();
    other.types.push(id("x.demo/extra"));
    forked.push(other);
    let set = SchemaSet::from_decls(&forked, &mut r);
    assert_eq!(r.count(Code::W429), 1);
    assert!(set.current(&id("x.demo/v1")).is_none());
    assert_eq!(set.forks(), vec![(id("x.demo/v1"), 2)]);
    assert!(set.revision(&id("x.demo/v1"), 2).is_none());
    assert!(set.revision(&id("x.demo/v1"), 3).is_some());
}

#[test]
fn a_store_lists_its_declarations() {
    let mut store = smysl_graph_free_store();
    store.retain(|r| matches!(r, Record::SchemaDecl(_)));
    let mut r = Report::new();
    let set = SchemaSet::from_records(&store, &mut r);
    assert_eq!(set.ids().collect::<Vec<_>>(), vec!["x.demo/v1"]);
}

/// `smysl-core` cannot depend on `smysl-graph`; `Store::schema_decls` is tested there. Here a
/// record list stands in for the log.
fn smysl_graph_free_store() -> Vec<Record> {
    demo_series().into_iter().map(Record::SchemaDecl).collect()
}

#[test]
fn a_declaration_with_lenses_has_no_surface_form_yet() {
    let d = &demo_series()[1];
    assert!(!smysl_core::surface::schema_decl_has_surface_form(d));
    let bare = SchemaDecl::new(id("x.bare/v1"), 1);
    assert!(smysl_core::surface::schema_decl_has_surface_form(&bare));
}
