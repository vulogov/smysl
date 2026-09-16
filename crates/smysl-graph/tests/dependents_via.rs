//! `dependents_via`: what depends on a unit, over the edges a caller chooses.

use smysl_core::surface::parse_surface;
use smysl_core::{Label, RelKind, Uid};
use smysl_graph::{dependents, dependents_via, reverse_closure, EdgeKind, EdgeSet, Store};

/// A decision with two prerequisites: one it is grounded on, one that `conditions` it.
const SRC: &str = "\
@claim p/grounding { status: speculative }
~ A prerequisite the decision is grounded on.

@claim p/condition { status: speculative }
~ A prerequisite linked by conditions.

@decision d/choice { status: inferred, grounds: [p/grounding] }
~ The decision.

@claim c/downstream { status: inferred, grounds: [d/choice] }
~ Something resting on the decision.

@rel p/condition --conditions--> d/choice
";

fn setup() -> (Store, impl Fn(&str) -> Uid) {
    let out = parse_surface(SRC).unwrap();
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let labels = out.labels.clone();
    (Store::from_records(out.records), move |l: &str| {
        labels[&Label::new(l).unwrap()]
    })
}

fn with_conditions() -> EdgeSet {
    EdgeSet::of([
        EdgeKind::Deps,
        EdgeKind::Grounds,
        EdgeKind::kernel(RelKind::Conditions).unwrap(),
    ])
}

#[test]
fn over_support_edges_it_is_dependents() {
    let (store, uid) = setup();
    for l in ["p/grounding", "p/condition", "d/choice", "c/downstream"] {
        let mut a = dependents(&store, uid(l));
        let mut b = dependents_via(&store, uid(l), &EdgeSet::support());
        a.sort();
        b.sort();
        assert_eq!(a, b, "{l}");
    }
}

#[test]
fn a_decision_conditioned_on_a_prerequisite_is_among_its_dependents() {
    let (store, uid) = setup();
    let deps = dependents_via(&store, uid("p/condition"), &with_conditions());
    assert!(
        deps.contains(&uid("d/choice")),
        "the conditioned decision is missing"
    );
    assert!(
        deps.contains(&uid("c/downstream")),
        "and what rests on the decision rests, through it, on the condition"
    );
    assert!(
        !deps.contains(&uid("p/grounding")),
        "a sibling prerequisite is not a dependent"
    );
    assert!(
        !deps.contains(&uid("p/condition")),
        "a unit is not its own dependent"
    );
}

/// The trap `dependents_via` exists to close. A `grounds` edge is stored from the dependent to
/// what it rests on; a relation `p --conditions--> d` is stored from `p` to `d`, the other way
/// round. So a plain reverse closure over grounds, deps and conditions — the obvious
/// implementation, and the one a caller holding `NodeId`s would write — never reaches the
/// conditioned decision.
#[test]
fn a_plain_reverse_closure_over_the_same_edges_misses_it() {
    let (store, uid) = setup();
    let g = store.adjacency();
    let from = g.id(&uid("p/condition")).unwrap();
    let reached: Vec<Uid> = reverse_closure(g, &[from], &with_conditions())
        .into_iter()
        .filter_map(|n| g.uid(n).copied())
        .collect();
    assert!(
        !reached.contains(&uid("d/choice")),
        "if this now finds it, the adjacency changed direction and dependents_via must follow"
    );
}
