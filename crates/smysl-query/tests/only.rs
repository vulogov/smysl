//! `expand … only` and the walk into history (SMYSL-2.9 §4.6; SMYSL-2.5 OQ-86, OQ-87).
//!
//! Over one small store: an account of tea a later one replaced, a rebuttal of each, a third
//! claim with two rebuttals — one of which names tea itself, which is OQ-86's case — and a
//! ground under the third claim.

use std::collections::BTreeSet;

use smysl_core::ids::Uid;
use smysl_core::surface::parse_surface;
use smysl_graph::Store;
use smysl_query::{explain, parse, plan, run, Env, PlanOptions, StoreScope};

const TEA: &str = r#"@schema x.text/v1 { version: 1, types: ["x.text/entity"], relations: [x.text/mentions, x.text/same-as] }

@x.text/entity e/tea { status: speculative, source: { kind: doc, ref: "alias:en" } }
~ tea

@claim t/oldest { status: cited, source: { kind: doc, ref: "tea:0" } }
~ Tea is a cure for every fever.

@claim t/old { status: cited, source: { kind: doc, ref: "tea:1" } }
~ Tea cures the ague.

@claim t/new { status: cited, source: { kind: doc, ref: "tea:2" } }
~ Tea eases the ague.

@data t/survey { status: cited, source: { kind: doc, ref: "tea:3" } }
~ Nine drinkers in ten said they lay awake.

@claim t/awake { status: inferred, grounds: [t/survey] }
~ Tea keeps a drinker awake.

@claim r/old { status: cited, source: { kind: doc, ref: "tea:4" } }
~ The fevers came back whatever was drunk.

@claim r/awake { status: cited, source: { kind: doc, ref: "tea:5" } }
~ The monks drank it at night and slept well.

@claim r/named { status: cited, source: { kind: doc, ref: "tea:6" } }
~ Green tea taken late did not stop anyone sleeping.

@rel t/old --supersedes--> t/oldest
@rel t/new --supersedes--> t/old
@rel r/old --rebuts--> t/old
@rel r/awake --rebuts--> t/awake
@rel r/named --rebuts--> t/awake
@rel t/oldest --x.text/mentions--> e/tea
@rel t/old --x.text/mentions--> e/tea
@rel t/new --x.text/mentions--> e/tea
@rel t/awake --x.text/mentions--> e/tea
@rel r/named --x.text/mentions--> e/tea
"#;

struct Fx {
    store: Store,
    labels: Vec<(String, Uid)>,
}

impl Fx {
    fn new() -> Fx {
        let parsed = parse_surface(TEA).expect("the store parses");
        Fx {
            labels: parsed
                .labels
                .iter()
                .map(|(l, u)| (l.to_string(), *u))
                .collect(),
            store: Store::from_records(parsed.records),
        }
    }

    fn uid(&self, label: &str) -> Uid {
        self.labels.iter().find(|(l, _)| l == label).expect(label).1
    }

    fn set(&self, labels: &[&str]) -> BTreeSet<Uid> {
        labels.iter().map(|l| self.uid(l)).collect()
    }

    /// The query's answer, and its plan as `--explain` prints it.
    fn answer(&self, text: &str, opts: &PlanOptions) -> (BTreeSet<Uid>, String) {
        let scope = StoreScope::new("tea", &self.store);
        let env = Env::new(&self.store, &scope).with_entities(&scope);
        let q = parse(text).expect("parses").query;
        let p = plan(&q, &env, opts).expect("plans");
        let r = run(&p, &env, opts).expect("runs");
        (r.answer.uids().into_iter().collect(), explain(&p))
    }
}

#[test]
fn only_keeps_what_the_walk_reached_and_not_the_seeds() {
    let f = Fx::new();
    let (got, plan) = f.answer(
        "about entity(\"tea\")\nexpand ->rebuts depth 1 only",
        &PlanOptions::default(),
    );
    // r/named is a seed (it names tea) and is reached from t/awake, so it stays; r/old is the
    // rebuttal of a replaced account, which this plain query still walks from (the pass runs
    // after the walk unless a caller asks otherwise).
    assert_eq!(got, f.set(&["r/awake", "r/named", "r/old"]));
    assert!(plan.contains("expand: ->rebuts only"), "{plan}");
}

#[test]
fn without_only_the_seeds_stay() {
    let f = Fx::new();
    let (got, _) = f.answer(
        "about entity(\"tea\")\nexpand ->rebuts depth 1",
        &PlanOptions::default(),
    );
    // The seeds' heads (the pass replaced t/oldest and t/old by t/new), the rebuttals.
    assert_eq!(
        got,
        f.set(&["t/new", "t/awake", "r/awake", "r/named", "r/old"])
    );
}

#[test]
fn seeds_stay_when_any_other_stage_of_the_phase_keeps_them() {
    let f = Fx::new();
    let (got, _) = f.answer(
        "about entity(\"tea\")\nexpand ->rebuts depth 1 only\nexpand ->grounds depth 1",
        &PlanOptions::default(),
    );
    assert!(got.contains(&f.uid("t/awake")), "the seeds stay");
    assert!(got.contains(&f.uid("t/survey")), "the ground");
    assert!(got.contains(&f.uid("r/awake")), "the rebuttal");

    let (got, _) = f.answer(
        "about entity(\"tea\")\nexpand ->rebuts depth 1 only\nwith rebuttals",
        &PlanOptions::default(),
    );
    assert!(got.contains(&f.uid("t/awake")), "a closure keeps the seeds");
}

#[test]
fn a_walk_from_the_current_account_leaves_the_replaced_one_out() {
    let f = Fx::new();
    let mut opts = PlanOptions::default();
    opts.supersede_before_walk = true;
    let (got, plan) = f.answer("about entity(\"tea\")\nexpand ->rebuts depth 1 only", &opts);
    assert_eq!(got, f.set(&["r/awake", "r/named"]));
    assert!(plan.contains("seeds give way"), "{plan}");
}

#[test]
fn the_walk_into_history_is_kept_as_written() {
    let f = Fx::new();
    let (got, plan) = f.answer(
        "about entity(\"tea\")\nexpand <-supersedes only",
        &PlanOptions::default(),
    );
    // Every version something replaced, at any depth, and not the head the pass would have
    // turned each of them back into.
    assert_eq!(got, f.set(&["t/old", "t/oldest"]));
    assert!(plan.contains("kept as written"), "{plan}");

    let (got, _) = f.answer(
        "about entity(\"tea\")\nexpand <-supersedes depth 1 only",
        &PlanOptions::default(),
    );
    assert_eq!(
        got,
        f.set(&["t/old", "t/oldest"]),
        "t/old is reached from t/new, t/oldest from t/old"
    );
}

#[test]
fn a_walk_the_other_way_is_not_history() {
    let f = Fx::new();
    let (got, plan) = f.answer(
        "about entity(\"tea\")\nexpand ->supersedes only",
        &PlanOptions::default(),
    );
    // ->supersedes walks from a replaced version to what replaced it, which is current; the
    // pass runs over it as over anything else.
    assert_eq!(got, f.set(&["t/new"]));
    assert!(!plan.contains("kept as written"), "{plan}");
}
