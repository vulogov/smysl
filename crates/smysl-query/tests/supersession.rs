//! The supersession pass (TX-P6 step 5; SMYSL-2.5 §3.2, §3.3).
//!
//! §3.2: *"a supersession pass runs after sources and traversals: units with a single newest
//! successor are replaced by it, as relink resolves chains; a fork keeps every successor and
//! raises `SMY-W464`. `--all-versions` turns the pass off."*
//!
//! The step's exit is in two halves and they guard different things. *"Existing relink tests
//! unchanged"* guards the refactor: the head computation was `relink`'s private `successors`
//! and is now `smysl-graph`'s public `supersession_heads`, and a visibility change that moved
//! behaviour would be the worst possible outcome — a repair and a query disagreeing about
//! which unit is current, which is the exact disagreement making it public was meant to
//! prevent. *"`W464` on a fork fixture"* guards the pass itself.

use smysl_core::surface::parse_surface;
use smysl_core::{Label, Uid};
use smysl_graph::Store;
use smysl_query::{plan, run, Answer, Env, PlanOptions, StoreScope};

const NAME: &str = "supersession";

fn fixture() -> (Store, impl Fn(&str) -> Uid) {
    let path = "../../fixtures/sq/plans/supersession.smy";
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let out = parse_surface(&src).expect("the fixture parses");
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let labels = out.labels.clone();
    (Store::from_records(out.records), move |l: &str| {
        labels[&Label::new(l).expect("a label")]
    })
}

fn answer(store: &Store, query: &str, opts: &PlanOptions) -> (Answer, Vec<String>) {
    let scope = StoreScope::new(NAME, store);
    let env = Env::new(store, &scope);
    let q = format!("from store:{NAME}\n{query}");
    let parsed = smysl_query::parse(&q).unwrap_or_else(|d| panic!("{q}: {}", d.message));
    let p = plan(&parsed.query, &env, opts)
        .unwrap_or_else(|d| panic!("{q}: {} {}", d[0].code, d[0].message));
    let outcome =
        run(&p, &env, opts).unwrap_or_else(|d| panic!("{q}: {} {}", d[0].code, d[0].message));
    (
        outcome.answer,
        outcome
            .warnings
            .iter()
            .map(|d| d.code.as_str().to_string())
            .collect(),
    )
}

/// A chain resolves to its newest head, all the way, as relink resolves it.
#[test]
fn a_superseded_unit_gives_way_to_its_newest_head() {
    let (store, uid) = fixture();
    let (answer, _) = answer(&store, "where kind = claim", &PlanOptions::default());
    let got: Vec<Uid> = answer.uids();

    assert!(!got.contains(&uid("c/first")), "superseded twice over");
    assert!(!got.contains(&uid("c/second")), "superseded once");
    assert!(got.contains(&uid("c/third")), "the head of the chain");
    assert!(got.contains(&uid("c/settled")), "nobody revised it");
}

/// A fork keeps every successor and says so.
///
/// Not "keep the old one": the single case replaces, so the fork case replaces too — with
/// both successors rather than one, because choosing would be adjudicating a contention the
/// store has recorded and not settled.
#[test]
fn a_fork_keeps_every_successor_and_raises_w464() {
    let (store, uid) = fixture();
    let (answer, codes) = answer(&store, "where kind = claim", &PlanOptions::default());
    let got: Vec<Uid> = answer.uids();

    assert!(
        !got.contains(&uid("c/disputed")),
        "the forked unit is replaced"
    );
    assert!(got.contains(&uid("c/revision-a")));
    assert!(got.contains(&uid("c/revision-b")));
    assert!(codes.contains(&"SMY-W464".to_string()), "{codes:?}");
}

/// `--all-versions` turns the pass off, and then every version is there.
#[test]
fn all_versions_keeps_the_history() {
    let (store, uid) = fixture();
    let mut opts = PlanOptions::default();
    opts.all_versions = true;
    let (answer, codes) = answer(&store, "where kind = claim", &opts);
    let got: Vec<Uid> = answer.uids();

    for label in [
        "c/first",
        "c/second",
        "c/third",
        "c/disputed",
        "c/revision-a",
        "c/revision-b",
        "c/settled",
    ] {
        assert!(got.contains(&uid(label)), "{label} should survive");
    }
    assert!(
        codes.is_empty(),
        "a pass that did not run warns about nothing"
    );
}

/// A store with no supersession at all is left exactly as it was.
///
/// The pass returns early rather than walking the result for nothing, and this is the
/// assertion that the early return is not also a behaviour change.
#[test]
fn a_store_without_supersession_is_untouched() {
    let path = "../../fixtures/corpus/F1-incident.smy";
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let store = Store::from_records(parse_surface(&src).expect("parses").records);

    let scope = StoreScope::new("F1-incident", &store);
    let env = Env::new(&store, &scope);
    let mut all = PlanOptions::default();
    all.all_versions = true;

    let run_one = |opts: &PlanOptions| {
        let parsed =
            smysl_query::parse("from store:F1-incident\nwhere kind = claim").expect("parses");
        let p = plan(&parsed.query, &env, opts).expect("plans");
        run(&p, &env, opts).expect("runs")
    };
    assert_eq!(
        run_one(&PlanOptions::default()).answer,
        run_one(&all).answer,
        "with nothing superseded, the pass and no pass are the same answer"
    );
}

/// The pass runs **after** traversals, which is what §3.2 says and what makes it useful: a
/// traversal that reaches a superseded unit hands the pass its replacement.
#[test]
fn the_pass_runs_after_the_traversal_that_reaches_a_superseded_unit() {
    let (store, uid) = fixture();
    let (answer, _) = answer(
        &store,
        "where kind = claim\nexpand ->supersedes",
        &PlanOptions::default(),
    );
    let got: Vec<Uid> = answer.uids();
    // `->supersedes` walks from each revision back to what it replaced, so the traversal pulls
    // `c/first`, `c/second` and `c/disputed` in — and the pass then sends each on to its head
    // again. A pass that ran before the traversal would leave all three in the answer.
    assert!(!got.contains(&uid("c/first")));
    assert!(!got.contains(&uid("c/second")));
    assert!(!got.contains(&uid("c/disputed")));
}
