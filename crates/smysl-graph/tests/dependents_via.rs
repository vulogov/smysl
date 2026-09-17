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

/// The preset follows the dependency-bearing relations and nothing else.
#[test]
fn the_dependency_preset_is_the_right_way_round() {
    let src = "\
@claim p/base { status: speculative }
~ A prerequisite.

@decision d/conditioned { status: speculative }
~ Conditioned on the prerequisite.

@claim c/detail { status: speculative }
~ An elaboration of the prerequisite.

@claim c/objection { status: speculative }
~ An objection to the prerequisite.

@rel p/base --conditions--> d/conditioned
@rel c/detail --elaborates--> p/base
@rel c/objection --rebuts--> p/base
";
    let out = parse_surface(src).unwrap();
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let uid = |l: &str| out.labels[&Label::new(l).unwrap()];
    let store = Store::from_records(out.records.clone());

    let deps = dependents_via(&store, uid("p/base"), &EdgeSet::dependency());
    assert_eq!(
        deps,
        vec![uid("d/conditioned")],
        "only the conditioned decision rests on it"
    );
    assert!(EdgeSet::dependency().contains(EdgeKind::Grounds));
    assert!(!EdgeSet::dependency().contains(EdgeKind::kernel(RelKind::Elaborates).unwrap()));
    assert!(!EdgeSet::dependency().contains(EdgeKind::kernel(RelKind::Rebuts).unwrap()));
}

/// `dependency` follows `causes`; `premises` does not. An effect is a dependent of its cause
/// under the broad preset and not under the narrow one, which is the difference between
/// "what breaks" and "which conclusions lose a premise".
#[test]
fn the_premises_preset_leaves_out_causes() {
    let src = "\
@claim p/cause { status: speculative }
~ The pool saturated.

@claim c/effect { status: speculative }
~ Latency rose.

@decision d/conditioned { status: speculative }
~ Raise the pool size.

@rel p/cause --causes--> c/effect
@rel p/cause --conditions--> d/conditioned
";
    let out = parse_surface(src).unwrap();
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let uid = |l: &str| out.labels[&Label::new(l).unwrap()];
    let store = Store::from_records(out.records.clone());

    let mut broad = dependents_via(&store, uid("p/cause"), &EdgeSet::dependency());
    broad.sort();
    let mut expected = vec![uid("c/effect"), uid("d/conditioned")];
    expected.sort();
    assert_eq!(broad, expected);
    assert_eq!(
        dependents_via(&store, uid("p/cause"), &EdgeSet::premises()),
        vec![uid("d/conditioned")]
    );
    assert_eq!(EdgeSet::premises(), with_conditions());
}

/// An extension kind can be named, in whichever store holds it, and followed.
///
/// Extension edge kinds are interned per store, and only `extension_name(id)` existed — there was
/// no way from `x.verify/supports` to the `EdgeKind` that `dependents_via` needs. Two stores give
/// the same kind different ids here on purpose, so a lookup that ignored the store would fail one.
#[test]
fn an_extension_kind_is_named_per_store_and_followed() {
    let supports = RelKind::parse("x.verify/supports").unwrap();
    let base = "@claim c/claim { status: speculative }\n~ A claim.\n\n\
                @evidence e/fact { status: speculative }\n~ A fact from the code.\n\n\
                @rel e/fact --x.verify/supports--> c/claim\n";
    // `x.aaa/first` sorts before `x.verify/supports`, moving its intern id in the second store.
    let with_another = format!(
        "{base}\n@claim c/other {{ status: speculative }}\n~ Another.\n\n@rel e/fact --x.aaa/first--> c/other\n"
    );
    let mut ids = Vec::new();
    for src in [base.to_string(), with_another] {
        let out = parse_surface(&src).unwrap();
        let uid = |l: &str| out.labels[&Label::new(l).unwrap()];
        let store = Store::from_records(out.records.clone());
        let kind = store
            .adjacency()
            .edge_kind(&supports)
            .expect("the store holds this kind");
        ids.push(kind);
        let deps = dependents_via(&store, uid("e/fact"), &EdgeSet::dependency().with(kind));
        assert!(
            deps.contains(&uid("c/claim")),
            "the supported claim is a dependent of the fact"
        );
        assert!(
            src == base || !deps.contains(&uid("c/other")),
            "an unchosen extension was followed"
        );
    }
    assert_ne!(
        ids[0], ids[1],
        "the fixture must give the kind different ids in the two stores"
    );

    let (store, _) = setup();
    assert_eq!(
        store.adjacency().edge_kind(&supports),
        None,
        "a store without the kind has no id for it"
    );
    assert_eq!(
        store.adjacency().edge_kind(&RelKind::Conditions),
        EdgeKind::kernel(RelKind::Conditions),
        "kernel kinds resolve as before"
    );
}

/// `rests_on` is `dependents_via` the other way round, over the same edges and with the same
/// asymmetry: `grounds` along its direction, a relation against its own.
#[test]
fn rests_on_is_dependents_via_reversed() {
    let (store, uid) = setup();
    let (ground, condition, choice, downstream) = (
        uid("p/grounding"),
        uid("p/condition"),
        uid("d/choice"),
        uid("c/downstream"),
    );

    let mut on = smysl_graph::rests_on(&store, downstream, &with_conditions());
    on.sort();
    let mut want = vec![choice, ground, condition];
    want.sort();
    assert_eq!(
        on, want,
        "the decision, both its prerequisites, transitively"
    );

    assert_eq!(
        smysl_graph::rests_on(&store, choice, &EdgeSet::support()),
        vec![ground],
        "grounds alone stops at the grounded prerequisite"
    );
    assert!(smysl_graph::rests_on(&store, ground, &with_conditions()).is_empty());

    // The pair is symmetric: if a rests on b, b has a among its dependents.
    for start in [downstream, choice] {
        for x in smysl_graph::rests_on(&store, start, &with_conditions()) {
            assert!(
                dependents_via(&store, x, &with_conditions()).contains(&start),
                "{start} rests on {x}, which does not list it as a dependent"
            );
        }
    }
}

/// `trace_via` walks the same edges by depth, and names how each unit was reached.
#[test]
fn trace_via_walks_chosen_edges_by_depth() {
    use smysl_graph::{trace_via, Via};
    let (store, uid) = setup();
    let line = trace_via(&store, uid("c/downstream"), &with_conditions(), None);
    assert_eq!(line.kind.as_str(), "rests-on");

    let at = |u| line.nodes.iter().find(|n| n.uid == u).expect("reached");
    assert_eq!(at(uid("c/downstream")).depth, 0);
    assert_eq!(at(uid("d/choice")).depth, 1);
    assert_eq!(at(uid("d/choice")).via, Via::Grounds, "inside the unit");
    assert_eq!(at(uid("p/condition")).depth, 2);
    assert_eq!(
        at(uid("p/condition")).via,
        Via::Rests,
        "a relation the caller named"
    );

    // Depth bounds it, and grounds alone never reaches the conditioned prerequisite.
    assert_eq!(
        trace_via(&store, uid("c/downstream"), &with_conditions(), Some(1)).len(),
        2
    );
    let narrow = trace_via(&store, uid("c/downstream"), &EdgeSet::support(), None);
    assert!(!narrow.nodes.iter().any(|n| n.uid == uid("p/condition")));
}
