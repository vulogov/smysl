//! The natural-language front end's check (SMYSL-2.5 §3.15, §5.6; TX-P10 step 5).
//!
//! §5.6's unit clause: model outputs that do not parse, name undeclared fields, or use a
//! disallowed stage are rejected (`SMY-E474`) and never executed. A scripted [`Translator`]
//! returns each output, so what is tested is [`accept`] and the prompt, not a model.

use std::collections::BTreeMap;

use smysl_core::schema::{encode_shape, Field, FieldType};
use smysl_core::surface::{parse_surface, HValue};
use smysl_core::types::{Record, SchemaDecl, Status, UnitCoreBuilder};
use smysl_core::SchemaId;
use smysl_graph::Store;
use smysl_query::nl::{prompt, TranslateContext, TranslateError, Translator};
use smysl_query::{accept, run, Env, PlanOptions, StoreScope};

/// Returns what it was given, whatever the question.
struct Scripted(&'static str);

impl Translator for Scripted {
    fn model_hash(&self) -> [u8; 32] {
        [9; 32]
    }
    fn translate(&self, _q: &str, _ctx: &TranslateContext) -> Result<String, TranslateError> {
        Ok(self.0.to_string())
    }
}

fn f1() -> Store {
    let src = std::fs::read_to_string("../../fixtures/corpus/F1-incident.smy").unwrap();
    Store::from_records(parse_surface(&src).unwrap().records)
}

/// A store whose one schema types its fields, so a field it does not declare is refused.
fn typed() -> Store {
    let sid = |s: &str| SchemaId::parse(s).unwrap();
    let mut d = SchemaDecl::new(sid("x.text/v1"), 1);
    d.types = vec![sid("x.text/event")];
    let mut f = BTreeMap::new();
    f.insert("text:mode".to_string(), Field::new(FieldType::Text));
    d.payload_shape = Some(encode_shape(&f));
    let view = smysl_core::schema::UnitView {
        ty: sid("x.text/event"),
        payload: [("text:mode".to_string(), HValue::Str("reported".into()))].into(),
    };
    let unit = UnitCoreBuilder::new(view.ty.clone(), "it was reported", Status::Speculative)
        .payload(view.payload_bytes().unwrap())
        .build()
        .unwrap();
    Store::from_records(vec![Record::SchemaDecl(d), Record::Unit(unit)])
}

fn check(store: &Store, name: &str, reply: &'static str) -> Result<String, (String, String)> {
    let scope = StoreScope::new(name, store);
    let env = Env::new(store, &scope);
    let ctx = TranslateContext::of(&env);
    let text = Scripted(reply).translate("anything", &ctx).unwrap();
    accept(&text, &env, &PlanOptions::default())
        .map(|a| a.text)
        .map_err(|d| (d.code.as_str().to_string(), d.message))
}

#[test]
fn every_unacceptable_translation_is_e474() {
    let store = f1();
    for (reply, why) in [
        (
            "Here are the claims: from store:F1-incident where kind = claim",
            "SMY-E460",
        ),
        (
            "from store:F1-incident\nwhere kind = claim\nexpand ->blames",
            "SMY-E462",
        ),
        ("from store:F1-incident\nfind \"pool\" ~sem", "SMY-E461"),
        ("from store:elsewhere\nwhere kind = claim", "SMY-E511"),
        (
            "from store:F1-incident\nwhere kind = claim\npack 400 tokens as view",
            "as view",
        ),
        (
            "from store:F1-incident\nwhere kind = claim\nmeasure t1",
            "`measure`",
        ),
        (
            "from store:F1-incident\nquestion \"why\"",
            "`question` source",
        ),
        (
            "from store:F1-incident\nmatch (a)-[:grounds]->(b)",
            "`match`",
        ),
        ("from store:F1-incident\nfind $topic", "parameter"),
        ("", "SMY-E460"),
    ] {
        let (code, msg) = check(&store, "F1-incident", reply).expect_err(reply);
        assert_eq!(code, "SMY-E474", "{reply:?}");
        assert!(msg.contains(why), "{reply:?}: {msg}");
        assert!(msg.ends_with("nothing was executed"), "{msg}");
    }

    // An undeclared field, where the store's declarations type every field.
    let (code, msg) = check(&typed(), "typed", "from store:typed\nwhere colour = red").unwrap_err();
    assert_eq!(code, "SMY-E474");
    assert!(msg.contains("SMY-E421"), "{msg}");
}

/// What passes is printed canonically, and runs — the plan is the one `accept` checked.
#[test]
fn an_acceptable_translation_is_accepted_in_canonical_form() {
    let store = f1();
    let got = check(
        &store,
        "F1-incident",
        "```sq\nfrom store:F1-incident\nwhere kind = claim and matches \"pool\"\n```",
    )
    .expect("accepted");
    assert_eq!(
        got,
        "from store:F1-incident\nwhere kind = claim and matches \"pool\""
    );

    let scope = StoreScope::new("F1-incident", &store);
    let env = Env::new(&store, &scope);
    let a = accept(&got, &env, &PlanOptions::default()).unwrap();
    let r = run(&a.plan, &env, &PlanOptions::default()).unwrap();
    // The pool claim, and the regression claim whose body names the pool hypothesis.
    assert_eq!(r.answer.uids().len(), 2);
}

/// The context is the store's vocabulary, and the prompt carries it with the question fenced.
#[test]
fn the_prompt_carries_the_store_and_fences_the_question() {
    let store = f1();
    let scope = StoreScope::new("F1-incident", &store);
    let env = Env::new(&store, &scope);
    let ctx = TranslateContext::of(&env);
    assert_eq!(ctx.scope, "F1-incident");
    assert_eq!(ctx.kinds, ["claim", "definition", "evidence", "finding"]);
    assert_eq!(ctx.statuses, ["inferred", "derived", "cited", "measured"]);
    for r in ["grounds", "deps", "causes", "rebuts", "warrant"] {
        assert!(ctx.relations.iter().any(|x| x == r), "{r}");
    }
    assert!(!ctx.semantic);
    let p = prompt("Why did latency rise?", &ctx);
    assert!(p.system.contains("`from store:F1-incident`"));
    assert!(p.system.contains("inferred < derived < cited < measured"));
    assert!(
        !p.system.contains("~sem"),
        "no model, so no semantic stages offered"
    );
    assert_eq!(
        p.user,
        "<<<QUESTION>>>\nWhy did latency rise?\n<<<QUESTION>>>"
    );
    // Every example is itself acceptable sq/1 — over its own store, so it parses.
    for (_, q) in &ctx.examples {
        smysl_query::parse(q).unwrap_or_else(|d| panic!("{q}: {}", d.message));
    }
    assert_eq!(
        p,
        prompt("Why did latency rise?", &ctx),
        "a function of its inputs"
    );
}

/// §3.15: a saved translated query records the translator in `query:models` and the question
/// as the unit's body, and neither moves its recipe — a rerun of it is pure and raises no
/// `SMY-W467`.
#[test]
fn a_saved_translation_records_its_question_and_translator() {
    let mut store = f1();
    let scope = StoreScope::new("F1-incident", &store);
    let env = Env::new(&store, &scope);
    let a = accept(
        "from store:F1-incident\nwhere kind = evidence",
        &env,
        &PlanOptions::default(),
    )
    .unwrap();
    let mut opts = PlanOptions::default();
    opts.question = Some("Which evidence is there?".into());
    opts.translator = Some("09".repeat(32));
    let r = run(&a.plan, &env, &opts).unwrap();
    let (unit, edges) = smysl_query::to_records(&a.query, &a.plan, &r, &opts).unwrap();
    let (plain, _) =
        smysl_query::to_records(&a.query, &a.plan, &r, &PlanOptions::default()).unwrap();
    assert_eq!(unit.body.as_deref(), Some("Which evidence is there?"));
    let uid = smysl_core::canonical_uid(&unit);
    let mut records = vec![Record::Unit(unit)];
    records.extend(edges.into_iter().map(Record::Relation));
    store.append(&records).unwrap();
    let saved = smysl_query::load(&store, &uid).unwrap();
    assert_eq!(saved.models, vec!["09".repeat(32)]);
    let plain_recipe = smysl_query::load(
        &Store::from_records(vec![Record::Unit(plain.clone())]),
        &smysl_core::canonical_uid(&plain),
    )
    .map(|s| s.recipe);
    if let Ok(recipe) = plain_recipe {
        assert_eq!(
            saved.recipe, recipe,
            "the translator does not move the recipe"
        );
    }

    let scope = StoreScope::new("F1-incident", &store);
    let env = Env::new(&store, &scope);
    let again = smysl_query::rerun(&saved, &env, &PlanOptions::default(), false).unwrap();
    assert!(again
        .run
        .warnings
        .iter()
        .all(|d| d.code.as_str() != "SMY-W467"));
}
