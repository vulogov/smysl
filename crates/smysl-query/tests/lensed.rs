//! Field resolution, pins and compile-forward (SMYSL-2.5 §3.9, TX-P12 step 3).
//!
//! The exit is §19.5's rewrite printed exactly as §4.4 shows it:
//!
//! ```text
//! rewrites:  mode = reported  =>  mode = reported and text:verbatim = false   (widening, paired key)
//! ```
//!
//! The store is §19.5's: `x.text/v1` at revision 1 as Appendix C declares it, and a revision 2
//! that splits `quoted` into `reported` plus `text:verbatim = true`. `reported` existed at
//! revision 1, so the split is a **widening**: a revision-1 `reported` is a revision-2
//! `reported` with `text:verbatim = false`, the step's paired default.

use std::collections::BTreeMap;

use smysl_core::schema::{encode_lenses, encode_shape, Cond, Field, FieldType, LensOp, LensStep};
use smysl_core::surface::HValue;
use smysl_core::types::{Record, SchemaDecl, Status, UnitCore, UnitCoreBuilder};
use smysl_core::{SchemaId, Uid};
use smysl_graph::Store;
use smysl_query::{explain, lens_note, parse, plan, run, Env, PlanOptions, StoreScope};

const NAME: &str = "s195";

fn sid(s: &str) -> SchemaId {
    SchemaId::parse(s).unwrap()
}

fn s(v: &str) -> HValue {
    HValue::Str(v.into())
}

const MODES: [&str; 7] = [
    "asserted",
    "reported",
    "hypothetical",
    "negated",
    "interrogative",
    "imperative",
    "quoted",
];

fn v1_fields() -> BTreeMap<String, Field> {
    let mut f = BTreeMap::new();
    let mut holder = Field::new(FieldType::Uid);
    holder.literals = vec!["text".into()];
    f.insert("text:holder".into(), holder);
    f.insert("text:mode".into(), Field::enumeration(MODES));
    f.insert("text:speaker".into(), Field::new(FieldType::Uid));
    f.insert("text:locator".into(), Field::new(FieldType::Text));
    f.insert("text:when".into(), Field::new(FieldType::Edtf));
    f.insert(
        "text:kind".into(),
        Field::enumeration(["person", "place", "group", "work", "object", "concept"]),
    );
    f.insert("meta:field".into(), Field::new(FieldType::Text));
    f.insert("meta:value".into(), Field::new(FieldType::Text));
    f
}

/// Revision 1 (Appendix C) and §19.5's revision 2.
fn decls() -> Vec<SchemaDecl> {
    let mut d1 = smysl_core::schemas::text::decl();
    d1.payload_shape = Some(encode_shape(&v1_fields()));

    let when = Some(Cond::new("text:mode", s("reported")));
    let step = LensStep::new(
        1,
        2,
        vec![
            LensOp::SplitValue {
                key: "text:mode".into(),
                from: "quoted".into(),
                to: "reported".into(),
                add: [("text:verbatim".to_string(), HValue::Bool(true))].into(),
            },
            LensOp::Require {
                key: "text:verbatim".into(),
                when: when.clone(),
            },
            LensOp::Default {
                key: "text:verbatim".into(),
                value: HValue::Bool(false),
                when,
            },
        ],
    );
    let mut f2 = v1_fields();
    f2.insert(
        "text:mode".into(),
        Field::enumeration(MODES.iter().filter(|m| **m != "quoted").copied()),
    );
    let mut verbatim = Field::new(FieldType::Bool);
    verbatim.required_from = Some(2);
    verbatim.required_when = Some(Cond::new("text:mode", s("reported")));
    f2.insert("text:verbatim".into(), verbatim);
    let mut d2 = d1.clone();
    d2.version = 2;
    // A type revision 2 introduces: a unit of it can only have been written at 2 or later.
    d2.types.push(sid("x.text/excerpt"));
    d2.payload_shape = Some(encode_shape(&f2));
    d2.lenses = Some(encode_lenses(&[step]));
    vec![d1, d2]
}

fn unit(gist: &str, payload: &[(&str, HValue)]) -> UnitCore {
    typed_unit("x.text/event", gist, payload)
}

fn typed_unit(ty: &str, gist: &str, payload: &[(&str, HValue)]) -> UnitCore {
    let view = smysl_core::schema::UnitView {
        ty: sid(ty),
        payload: payload
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    };
    let mut b = UnitCoreBuilder::new(view.ty.clone(), gist, Status::Speculative);
    if let Some(p) = view.payload_bytes() {
        b = b.payload(p);
    }
    b.build().unwrap()
}

struct Fixture {
    store: Store,
    /// Written at revision 1.
    asserted: Uid,
    reported: Uid,
    quoted: Uid,
    /// Written at revision 2.
    reported_plain: Uid,
    reported_verbatim: Uid,
    /// Of a type only revision 2 declares.
    excerpt: Uid,
}

fn fixture() -> Fixture {
    let us = [
        unit("they said it", &[("text:mode", s("asserted"))]),
        unit("it was reported", &[("text:mode", s("reported"))]),
        unit("it was quoted", &[("text:mode", s("quoted"))]),
        unit(
            "reported, in other words",
            &[
                ("text:mode", s("reported")),
                ("text:verbatim", HValue::Bool(false)),
            ],
        ),
        unit(
            "reported, word for word",
            &[
                ("text:mode", s("reported")),
                ("text:verbatim", HValue::Bool(true)),
            ],
        ),
        typed_unit(
            "x.text/excerpt",
            "an excerpt",
            &[("text:mode", s("asserted"))],
        ),
    ];
    let uids: Vec<Uid> = us.iter().map(smysl_core::hash::canonical_uid).collect();
    let mut records: Vec<Record> = decls().into_iter().map(Record::SchemaDecl).collect();
    records.extend(us.into_iter().map(Record::Unit));
    Fixture {
        store: Store::from_records(records),
        asserted: uids[0],
        reported: uids[1],
        quoted: uids[2],
        reported_plain: uids[3],
        reported_verbatim: uids[4],
        excerpt: uids[5],
    }
}

fn explained(store: &Store, q: &str) -> String {
    let scope = StoreScope::new(NAME, store);
    let env = Env::new(store, &scope);
    let parsed = parse(q).unwrap_or_else(|d| panic!("{q}: {}", d.message));
    let p = plan(&parsed.query, &env, &PlanOptions::default())
        .unwrap_or_else(|d| panic!("{q}: {:?}", d));
    explain(&p)
}

fn answer(store: &Store, q: &str) -> (Vec<Uid>, Vec<String>) {
    let scope = StoreScope::new(NAME, store);
    let env = Env::new(store, &scope);
    let parsed = parse(q).unwrap_or_else(|d| panic!("{q}: {}", d.message));
    let opts = PlanOptions::default();
    let p = plan(&parsed.query, &env, &opts).unwrap_or_else(|d| panic!("{q}: {:?}", d));
    let r = run(&p, &env, &opts).unwrap_or_else(|d| panic!("{q}: {:?}", d));
    let mut uids = r.answer.uids();
    uids.sort();
    (
        uids,
        r.warnings
            .iter()
            .map(|w| format!("{}: {}", w.code.as_str(), w.message))
            .collect(),
    )
}

fn refused(store: &Store, q: &str) -> (String, String) {
    let scope = StoreScope::new(NAME, store);
    let env = Env::new(store, &scope);
    let parsed = parse(q).unwrap_or_else(|d| panic!("{q}: {}", d.message));
    let d = plan(&parsed.query, &env, &PlanOptions::default())
        .err()
        .unwrap_or_else(|| panic!("{q} planned"));
    (d[0].code.as_str().to_string(), d[0].message.clone())
}

fn sorted(mut v: Vec<Uid>) -> Vec<Uid> {
    v.sort();
    v
}

/// The exit: §19.5's rewrite, printed as §4.4 prints it.
#[test]
fn the_section_19_5_rewrite_is_printed_as_section_4_4_shows_it() {
    let f = fixture();
    let text = explained(&f.store, "using x.text/v1@1\nwhere mode = reported");
    let line = text
        .lines()
        .find(|l| l.trim_start().starts_with("rewrites:"))
        .unwrap_or_else(|| panic!("no rewrite line:\n{text}"));
    assert_eq!(
        line.trim_start(),
        "rewrites:  mode = reported  =>  mode = reported and text:verbatim = false   (widening, paired key)",
        "{text}"
    );
    let pins = text
        .lines()
        .find(|l| l.trim_start().starts_with("pins:"))
        .unwrap();
    assert_eq!(
        pins.trim_start(),
        "pins:      x.text/v1@1 (compiled forward to @2)"
    );
    // Three units are consistent only with revision 1 (each holds a `text:mode` a step would
    // change, or lacks the key revision 2 requires); the two written at 2 are not lensed.
    let lenses = text
        .lines()
        .find(|l| l.trim_start().starts_with("lenses:"))
        .unwrap();
    assert!(
        lenses
            .trim_start()
            .starts_with("lenses:    x.text/v1 1->2  chain "),
        "{lenses}"
    );
    assert!(lenses.ends_with("units read, 0 unmappable"), "{lenses}");
    assert!(text.contains("fields text:mode, text:verbatim\n"), "{text}");
}

/// OQ-24: a run states the chain whenever a lens touched a unit, and says nothing when none
/// did — a query over raw fields, or over a store with no older revision.
#[test]
fn a_lens_that_touched_a_unit_is_stated_and_none_is_silent() {
    let note = |store: &Store, q: &str| {
        let scope = StoreScope::new(NAME, store);
        let env = Env::new(store, &scope);
        let parsed = parse(q).unwrap_or_else(|d| panic!("{q}: {}", d.message));
        let p = plan(&parsed.query, &env, &PlanOptions::default())
            .unwrap_or_else(|d| panic!("{q}: {:?}", d));
        lens_note(&p)
    };
    let f = fixture();
    let line =
        note(&f.store, "where mode = reported").expect("units written at revision 1 are lensed");
    assert!(
        line.starts_with("lenses: 2 unit(s) read through x.text/v1 1->2 (chain "),
        "{line}"
    );
    assert!(line.ends_with(", 0 unmappable)"), "{line}");
    assert_eq!(note(&f.store, "find \"pool\""), None);
    let u = unit("plain", &[("colour", s("red"))]);
    let store = Store::from_records(vec![Record::Unit(u)]);
    assert_eq!(note(&store, "where colour = red"), None);
}

/// The pinned query keeps revision 1's narrow meaning: a revision-1 `reported`, and a
/// revision-2 `reported` that is not verbatim. The unpinned query asks today's question.
#[test]
fn a_pinned_query_answers_the_old_question_over_lensed_units() {
    let f = fixture();
    let (pinned, warnings) = answer(&f.store, "using x.text/v1@1\nwhere mode = reported");
    assert_eq!(pinned, sorted(vec![f.reported, f.reported_plain]));
    assert!(warnings.is_empty(), "{warnings:?}");

    let (current, _) = answer(&f.store, "where mode = reported");
    assert_eq!(
        current,
        sorted(vec![
            f.reported,
            f.quoted,
            f.reported_plain,
            f.reported_verbatim
        ])
    );

    let (quoted, _) = answer(&f.store, "using x.text/v1@1\nwhere mode = quoted");
    assert_eq!(quoted, sorted(vec![f.quoted, f.reported_verbatim]));
    let text = explained(&f.store, "using x.text/v1@1\nwhere mode = quoted");
    assert!(
        text.contains(
            "rewrites:  mode = quoted  =>  mode = reported and text:verbatim = true   (split)"
        ),
        "{text}"
    );

    let (asserted, _) = answer(&f.store, "using x.text/v1@1\nwhere text:mode = asserted");
    assert_eq!(asserted, sorted(vec![f.asserted, f.excerpt]));
}

/// A qualified reference is the same compile, scoped to one predicate.
#[test]
fn a_qualified_field_compiles_forward_by_itself() {
    let f = fixture();
    let (got, _) = answer(&f.store, "where x.text/v1@1:text:mode = reported");
    assert_eq!(got, sorted(vec![f.reported, f.reported_plain]));
    let text = explained(&f.store, "where x.text/v1@1:mode = reported");
    assert!(
        text.contains(
            "rewrites:  x.text/v1@1:mode = reported  =>  text:mode = reported and text:verbatim = false   (widening, paired key)"
        ),
        "{text}"
    );
}

/// `with lens none` reads a qualified reference literally at the revision it names: units
/// consistent with that revision answer raw, and the rest are unmappable on the field.
///
/// A-12.1's clues place a unit *before* a step that would change it, or before a key became
/// required; nothing places a unit after a revision except its type. So the two units written
/// at revision 2 with `text:verbatim` are consistent with revision 1 too (a payload may carry
/// a key its revision does not declare) and read raw, and only the excerpt, whose type
/// revision 2 introduced, is unmappable.
#[test]
fn with_lens_none_reads_the_revision_named_and_counts_the_rest() {
    let f = fixture();
    let (got, warnings) = answer(
        &f.store,
        "where x.text/v1@1:text:mode = quoted with lens none",
    );
    assert_eq!(got, vec![f.quoted]);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].starts_with("SMY-W420: 1 unit(s) unmappable on `text:mode`"),
        "{warnings:?}"
    );
    let (got, _) = answer(
        &f.store,
        "where x.text/v1@1:text:mode = reported with lens none",
    );
    assert_eq!(
        got,
        sorted(vec![f.reported, f.reported_plain, f.reported_verbatim])
    );
}

/// `SMY-E421`: a field no schema declares when every unit is governed by one that types its
/// fields, and a short name two keys share.
#[test]
fn an_undeclared_or_ambiguous_field_is_e421() {
    let f = fixture();
    let (code, msg) = refused(&f.store, "where colour = red");
    assert_eq!(code, "SMY-E421");
    assert!(
        msg.contains("no schema in scope declares a field `colour`"),
        "{msg}"
    );

    let (code, msg) = refused(&f.store, "where x.text/v1@1:verbatim = true");
    assert_eq!(code, "SMY-E421", "{msg}");
    assert!(
        msg.contains("`x.text/v1@1` declares no field `verbatim`"),
        "{msg}"
    );

    // `field` is `meta:field` and nothing else; `text:` and `meta:` share no short name in
    // Appendix C, so ambiguity needs a second schema.
    let mut records: Vec<Record> = f.store.iter().cloned().collect();
    let mut other = SchemaDecl::new(sid("x.other/v1"), 1);
    other.types = vec![sid("x.other/thing")];
    let mut t = BTreeMap::new();
    t.insert("other:mode".to_string(), Field::new(FieldType::Text));
    other.payload_shape = Some(encode_shape(&t));
    records.push(Record::SchemaDecl(other));
    let store = Store::from_records(records);
    let (code, msg) = refused(&store, "where mode = reported");
    assert_eq!(code, "SMY-E421");
    assert!(
        msg.contains("x.other/v1:other:mode") && msg.contains("x.text/v1:text:mode"),
        "{msg}"
    );
}

/// `SMY-E422`: a pinned predicate on a value no lens carries forward.
#[test]
fn a_retired_value_with_no_mapping_is_e422() {
    let mut ds = decls();
    // Revision 3 drops `imperative` from the values, with no op to carry it.
    let mut d3 = ds[1].clone();
    d3.version = 3;
    let mut f3 = v1_fields();
    f3.insert(
        "text:mode".into(),
        Field::enumeration(
            MODES
                .iter()
                .filter(|m| **m != "quoted" && **m != "imperative")
                .copied(),
        ),
    );
    let mut verbatim = Field::new(FieldType::Bool);
    verbatim.required_from = Some(2);
    f3.insert("text:verbatim".into(), verbatim);
    d3.payload_shape = Some(encode_shape(&f3));
    d3.lenses = Some(encode_lenses(&[LensStep::new(2, 3, vec![])]));
    ds.push(d3);
    let store = Store::from_records(ds.into_iter().map(Record::SchemaDecl).collect());
    let (code, msg) = refused(&store, "using x.text/v1@1\nwhere mode = imperative");
    assert_eq!(code, "SMY-E422");
    assert!(
        msg.contains("`imperative` is not a value of `text:mode`"),
        "{msg}"
    );
}

/// A predicate on a key the chain retires is `SMY-E422`.
#[test]
fn a_retired_key_is_e422() {
    let mut ds = decls();
    let mut d3 = ds[1].clone();
    d3.version = 3;
    let mut f3 = v1_fields();
    let mut gone = Field::new(FieldType::Text);
    gone.retired_in = Some(3);
    f3.insert("text:locator".into(), gone);
    d3.payload_shape = Some(encode_shape(&f3));
    d3.lenses = Some(encode_lenses(&[LensStep::new(
        2,
        3,
        vec![LensOp::Retire {
            key: "text:locator".into(),
        }],
    )]));
    ds.push(d3);
    let store = Store::from_records(ds.into_iter().map(Record::SchemaDecl).collect());
    let (code, msg) = refused(&store, "using x.text/v1@1\nwhere locator = \"Gen 1:1\"");
    assert_eq!(code, "SMY-E422");
    assert!(msg.contains("`locator` is retired at x.text/v1@3"), "{msg}");
}

/// A store with no typed declaration reads every payload name raw, as before FC-9, and says
/// so; one with some undeclared unit types still reads an undeclared name raw.
#[test]
fn undeclared_payloads_are_read_raw_and_said_to_be() {
    let u = unit("plain", &[("colour", s("red"))]);
    let uid = smysl_core::hash::canonical_uid(&u);
    let store = Store::from_records(vec![Record::Unit(u.clone())]);
    let (got, _) = answer(&store, "where colour = red");
    assert_eq!(got, vec![uid]);
    let text = explained(&store, "where colour = red");
    assert!(
        text.contains("fields colour (raw) (no declaration in this store types a field)"),
        "{text}"
    );

    let f = fixture();
    let mut records: Vec<Record> = f.store.iter().cloned().collect();
    let kernel = UnitCoreBuilder::new(sid("claim"), "a kernel claim", Status::Speculative)
        .payload(
            smysl_core::schema::UnitView {
                ty: sid("claim"),
                payload: [("colour".to_string(), s("red"))].into(),
            }
            .payload_bytes()
            .unwrap(),
        )
        .build()
        .unwrap();
    let k = smysl_core::hash::canonical_uid(&kernel);
    records.push(Record::Unit(kernel));
    let store = Store::from_records(records);
    let (got, _) = answer(&store, "where colour = red");
    assert_eq!(got, vec![k]);
}

/// A raw dotted key reads the nested value its path names when no top-level key is spelled so,
/// as `payload_strings` keys it for `find`; a literal dotted key wins over the path.
#[test]
fn a_raw_dotted_key_reads_a_nested_value_and_a_literal_one_wins() {
    let src = r#"
@claim c/nested { status: speculative, "meta": { colour: "red" } }
~ nested red

@claim c/flat { status: speculative, "meta.colour": "blue", "meta": { colour: "red" } }
~ flat blue, nested red

@claim c/other { status: speculative, "meta": { colour: "green" } }
~ nested green
"#;
    let out = smysl_core::surface::parse_surface(src).unwrap();
    let uid = |l: &str| out.labels[&smysl_core::Label::new(l).unwrap()];
    let store = Store::from_records(out.records.clone());
    let (red, _) = answer(&store, "where meta.colour = red");
    assert_eq!(red, vec![uid("c/nested")]);
    let (blue, _) = answer(&store, "where meta.colour = blue");
    assert_eq!(blue, vec![uid("c/flat")]);
    let (none, _) = answer(&store, "where meta.colour.shade = red");
    assert!(none.is_empty());
}

/// A sort key compiles forward too: a pinned `order by` on a renamed key sorts by the key
/// today's revision names.
#[test]
fn ordering_reads_through_the_lens() {
    let f = fixture();
    let (got, _) = answer(&f.store, "where mode = reported\ngroup by verbatim");
    assert_eq!(got.len(), 4);
    let text = explained(
        &f.store,
        "using x.text/v1@1\nwhere mode = reported\norder by mode",
    );
    assert!(text.contains("rewrites:"), "{text}");
}

/// A key declared `int` has an order, so `>` means something on it; the same key undeclared
/// is still refused (`SMY-E512`).
#[test]
fn a_key_declared_int_is_ordered() {
    let mut d = SchemaDecl::new(sid("x.n/v1"), 1);
    d.types = vec![sid("x.n/item")];
    let mut t = BTreeMap::new();
    t.insert("n:count".to_string(), Field::new(FieldType::Int));
    d.payload_shape = Some(encode_shape(&t));
    let us: Vec<UnitCore> = [3, 10, 1]
        .iter()
        .map(|n| {
            typed_unit(
                "x.n/item",
                &format!("item {n}"),
                &[("n:count", HValue::Int(*n))],
            )
        })
        .collect();
    let uids: Vec<Uid> = us.iter().map(smysl_core::hash::canonical_uid).collect();
    let mut records = vec![Record::SchemaDecl(d)];
    records.extend(us.into_iter().map(Record::Unit));
    let store = Store::from_records(records);
    let (got, _) = answer(&store, "where count > 2");
    assert_eq!(
        got,
        sorted(vec![uids[0], uids[1]]),
        "10 > 2 as a number, not as text"
    );
    let scope = StoreScope::new(NAME, &store);
    let env = Env::new(&store, &scope);
    let opts = PlanOptions::default();
    let p = plan(
        &parse("where count > 0\norder by count descending")
            .unwrap()
            .query,
        &env,
        &opts,
    )
    .unwrap();
    assert_eq!(
        run(&p, &env, &opts).unwrap().answer.uids(),
        vec![uids[1], uids[0], uids[2]]
    );
}

/// §5.4's §19.5 reproduction (TX-P12 step 4): a `mode = reported` query saved over revision 1,
/// then revision 2's widening declared and data appended — a plain `reported` written at 2 and
/// the injected quotations, one written at 1 as `quoted` and one at 2 as `reported` with
/// `text:verbatim = true`. Rerun pinned, it shows only the data effect; `--current` reports a
/// schema effect of exactly the injected quotations.
#[test]
fn the_schema_effect_is_exactly_the_injected_quotations() {
    use smysl_query::{bind, load, rerun, to_records};

    let ds = decls();
    let reported = unit("it was reported", &[("text:mode", s("reported"))]);
    let mut store = Store::from_records(vec![
        Record::SchemaDecl(ds[0].clone()),
        Record::Unit(reported.clone()),
    ]);
    let opts = PlanOptions::default();
    let q = "where mode = reported";
    let (saved_uid, records) = {
        let scope = StoreScope::new(NAME, &store);
        let env = Env::new(&store, &scope);
        let bound = bind(&parse(q).unwrap().query, &opts.params).unwrap();
        let p = plan(&bound, &env, &opts).unwrap();
        assert_eq!(p.revisions.len(), 1, "the revision in force is recorded");
        let r = run(&p, &env, &opts).unwrap();
        assert_eq!(
            r.answer.uids(),
            vec![smysl_core::hash::canonical_uid(&reported)]
        );
        let (u, edges) = to_records(&bound, &p, &r, &opts).unwrap();
        let mut recs = vec![Record::Unit(u.clone())];
        recs.extend(edges.into_iter().map(Record::Relation));
        (smysl_core::hash::canonical_uid(&u), recs)
    };
    store.append(&records).unwrap();

    let plain = unit(
        "reported, in other words",
        &[
            ("text:mode", s("reported")),
            ("text:verbatim", HValue::Bool(false)),
        ],
    );
    let quoted_at_1 = unit("it was quoted", &[("text:mode", s("quoted"))]);
    let quoted_at_2 = unit(
        "reported, word for word",
        &[
            ("text:mode", s("reported")),
            ("text:verbatim", HValue::Bool(true)),
        ],
    );
    let uid = smysl_core::hash::canonical_uid;
    store
        .append(&[
            Record::SchemaDecl(ds[1].clone()),
            Record::Unit(plain.clone()),
            Record::Unit(quoted_at_1.clone()),
            Record::Unit(quoted_at_2.clone()),
        ])
        .unwrap();

    let scope = StoreScope::new(NAME, &store);
    let env = Env::new(&store, &scope);
    let saved = load(&store, &saved_uid).unwrap();
    assert_eq!(saved.schemas, vec!["x.text/v1@1".to_string()]);

    let pinned = rerun(&saved, &env, &opts, false).unwrap();
    assert_eq!(pinned.data_effect.added, vec![uid(&plain)]);
    assert!(pinned.data_effect.removed.is_empty());
    assert_eq!(pinned.schema_effect, None);
    assert!(pinned.recipe_matches);

    let current = rerun(&saved, &env, &opts, true).unwrap();
    assert_eq!(
        current.data_effect, pinned.data_effect,
        "the data effect is the pinned one"
    );
    let effect = current.schema_effect.expect("asked for");
    assert_eq!(
        effect.added,
        sorted(vec![uid(&quoted_at_1), uid(&quoted_at_2)])
    );
    assert!(effect.removed.is_empty());
}

/// A recorded lens chain the declarations no longer reproduce is `SMY-E466`: the query would
/// answer from part of its meaning.
#[test]
fn a_chain_the_store_no_longer_reproduces_cannot_be_pinned() {
    use smysl_query::{bind, load, rerun, to_records};

    let f = fixture();
    let mut store = f.store;
    let opts = PlanOptions::default();
    let q = "using x.text/v1@1\nwhere mode = reported";
    let (saved_uid, records) = {
        let scope = StoreScope::new(NAME, &store);
        let env = Env::new(&store, &scope);
        let bound = bind(&parse(q).unwrap().query, &opts.params).unwrap();
        let p = plan(&bound, &env, &opts).unwrap();
        assert_eq!(p.lens_chains.len(), 1);
        let r = run(&p, &env, &opts).unwrap();
        let (u, edges) = to_records(&bound, &p, &r, &opts).unwrap();
        let mut recs = vec![Record::Unit(u.clone())];
        recs.extend(edges.into_iter().map(Record::Relation));
        (smysl_core::hash::canonical_uid(&u), recs)
    };
    store.append(&records).unwrap();
    // The same saved query in a store that never received revision 2: its pin still
    // resolves, but the chain it was compiled through cannot be built.
    let without: Vec<Record> = store
        .iter()
        .filter(|r| !matches!(r, Record::SchemaDecl(d) if d.version == 2))
        .cloned()
        .collect();
    let store = Store::from_records(without);
    let scope = StoreScope::new(NAME, &store);
    let env = Env::new(&store, &scope);
    let saved = load(&store, &saved_uid).unwrap();
    let d = rerun(&saved, &env, &opts, false).expect_err("refused");
    assert_eq!(d[0].code.as_str(), "SMY-E466", "{}", d[0].message);
}
