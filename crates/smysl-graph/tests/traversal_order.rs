//! The claim at the top of `traverse.rs`, tested for all four rather than two.
//!
//! > Four patterns, one property: every result is a `Vec` in dense-id order, so a caller never
//! > has to sort and a traversal is a pure function of the graph (rule D).
//!
//! That sentence is now corrected in the module, because writing this test showed it is false
//! of `topo`: a topological order lists dependencies before dependents and uses dense id only
//! to break ties, so it is canonical without being sorted. The first run of the test below
//! reported `topo` returning `[0, 6, 4, 1, 3, 5, 2, 7]`, which is the function behaving exactly
//! as its own doc comment describes and exactly as the module header denied.
//!
//! And the claim justifying it, in `adjacency.rs`:
//!
//! > ordering is structural, so insertion order cannot leak into a traversal.
//!
//! Both are load-bearing — rule D is bit-reproducibility, and `topo` feeds thread derivation,
//! whose output is hashed. Neither was covered for all four. `closure` and `topo` each had a
//! test named for the property; `reverse_closure` asserted `vec![0, 1, 2]` on a chain, which is
//! sorted by accident of the shape, and `rebuttals_of` asserted `vec![1, 2]` on relations that
//! had been *inserted* in that order. Nothing anywhere built one graph two ways.
//!
//! Found by sweeping `smysl-graph` and `smysl-check` for claims of comprehensiveness with no
//! evidence beside them — the method that found `skip_item` earlier in 0.10, applied to the
//! next two crates.
//!
//! This matters more than usual right now: `topo` was rewritten this cycle, from a sorted
//! `Vec` to a `BinaryHeap`, precisely on the grounds that the order is unchanged.

use smysl_core::{
    canonical_uid, KernelType, Record, RelKind, Relation, Status, Uid, UnitCoreBuilder,
};
use smysl_graph::traverse::{closure, cycles, rebuttals_of, reverse_closure, topo};
use smysl_graph::{Adjacency, EdgeSet, NodeId, Store};

/// Eight units in a shape with both depth and fan-out, plus rebuttals, so every traversal has
/// something to order. Returned as records so the caller can decide what order to insert them.
fn records() -> Vec<Record> {
    let mut records = Vec::new();
    let mut uids: Vec<Uid> = Vec::new();
    for i in 0..8u8 {
        let grounds: Vec<Uid> = match i {
            0 | 1 => vec![],
            _ => vec![uids[(i - 1) as usize], uids[(i / 2) as usize]],
        };
        let core = UnitCoreBuilder::new(
            KernelType::Claim,
            format!("unit number {i} in the ordering fixture"),
            if grounds.is_empty() {
                Status::Speculative
            } else {
                Status::Inferred
            },
        )
        .grounds(grounds)
        .build()
        .unwrap();
        uids.push(canonical_uid(&core));
        records.push(Record::Unit(core));
    }
    // Several units rebut one, so `rebuttals_of` returns more than a single element and its
    // order is a claim rather than a coincidence.
    for from in [3usize, 5, 7] {
        records.push(Record::Relation(Relation::new(
            RelKind::Rebuts,
            uids[from],
            uids[2],
        )));
    }
    records.push(Record::Relation(Relation::new(
        RelKind::Causes,
        uids[6],
        uids[2],
    )));
    records
}

/// The same records in several orders. A store keyed by content holds the same graph however
/// they arrive, which is the property `adjacency.rs` claims and nothing tested.
fn orderings() -> Vec<Vec<Record>> {
    let base = records();
    let mut reversed = base.clone();
    reversed.reverse();
    let mut relations_first = base.clone();
    relations_first.sort_by_key(|r| !matches!(r, Record::Relation(_)));
    let mut rotated = base.clone();
    rotated.rotate_left(5);
    vec![base, reversed, relations_first, rotated]
}

fn adjacency_of(records: &[Record]) -> Adjacency {
    Store::from_records(records.to_vec()).adjacency().clone()
}

fn is_sorted(v: &[NodeId]) -> bool {
    v.windows(2).all(|w| w[0] < w[1])
}

/// Every traversal the module names, on the same graph, checked for the property the module
/// claims for all of them.
fn all_traversals(g: &Adjacency) -> Vec<(&'static str, Vec<NodeId>)> {
    let roots: Vec<NodeId> = vec![0, 3];
    let rebutted = rebutted_node(g);
    vec![
        ("closure", closure(g, &roots, &EdgeSet::support())),
        (
            "reverse_closure",
            reverse_closure(g, &roots, &EdgeSet::support()),
        ),
        ("topo", topo(g, &EdgeSet::support()).order),
        ("topo.cyclic", topo(g, &EdgeSet::support()).cyclic),
        ("rebuttals_of", rebuttals_of(g, rebutted)),
        ("cycles.flat", cycles(g, &EdgeSet::support()).concat()),
    ]
}

/// The node three others rebut. Found by uid rather than assumed to be a dense id: dense ids
/// are assigned by the store, not by the order this file happens to build records in, and
/// guessing one gave `rebuttals_of` an empty result that read as a passing sorted check.
fn rebutted_node(g: &Adjacency) -> NodeId {
    let uid = *rebutted_uid();
    g.id(&uid).expect("the rebutted unit is in the store")
}

fn rebutted_uid() -> &'static Uid {
    use std::sync::OnceLock;
    static U: OnceLock<Uid> = OnceLock::new();
    U.get_or_init(|| {
        let recs = records();
        match &recs[2] {
            Record::Unit(u) => canonical_uid(u),
            _ => unreachable!("record 2 is a unit by construction"),
        }
    })
}

/// The traversals that return a *set*, and therefore sort. `topo` is excluded on purpose.
const SORTED: &[&str] = &[
    "closure",
    "reverse_closure",
    "rebuttals_of",
    "topo.cyclic",
    "cycles.flat",
];

#[test]
fn every_traversal_returns_dense_id_order() {
    let g = adjacency_of(&records());
    for (name, out) in all_traversals(&g) {
        if !SORTED.contains(&name) {
            continue;
        }
        assert!(
            is_sorted(&out),
            "{name} returned {out:?}, which is not ascending dense id — a set-shaped \
             traversal sorts so that a caller never has to"
        );
    }
}

/// `topo` is not sorted, and must not be "fixed" into being. What it owes is a valid
/// dependency order: everything a node depends on appears before it.
#[test]
fn topo_returns_a_dependency_order_rather_than_a_sorted_one() {
    let g = adjacency_of(&records());
    let order = topo(&g, &EdgeSet::support()).order;
    assert!(
        !is_sorted(&order),
        "the fixture is meant to have a topological order that differs from dense id; \
         if this passes, the test below proves nothing"
    );
    let position: std::collections::BTreeMap<NodeId, usize> =
        order.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    for &node in &order {
        for dep in g.out(node, &EdgeSet::support()) {
            assert!(
                position[&dep] < position[&node],
                "{dep} is a dependency of {node} and came after it"
            );
        }
    }
}

/// The control. `is_sorted` is trivially true of an empty or one-element result, so a graph
/// whose traversals all came back empty would pass the test above while checking nothing.
#[test]
fn the_traversals_actually_return_something_to_order() {
    let g = adjacency_of(&records());
    for (name, out) in all_traversals(&g) {
        if name == "topo.cyclic" || name == "cycles.flat" {
            continue; // acyclic by construction; their emptiness is the point
        }
        assert!(
            out.len() >= 2,
            "{name} returned {} element(s); ordering is not being exercised",
            out.len()
        );
    }
}

#[test]
fn insertion_order_cannot_leak_into_a_traversal() {
    let expected = all_traversals(&adjacency_of(&records()));
    for (i, ordering) in orderings().into_iter().enumerate() {
        let got = all_traversals(&adjacency_of(&ordering));
        for ((name, want), (_, have)) in expected.iter().zip(got.iter()) {
            assert_eq!(
                want, have,
                "{name} differs under insertion ordering {i}; rule D says a traversal is a \
                 pure function of the graph, and a store keyed by content is the same graph"
            );
        }
    }
}

/// The control for the one above: the orderings must genuinely differ, or it compares a store
/// with itself four times. Two of them coincide on a symmetric record list, which is exactly
/// the kind of accident that makes a loop look like coverage.
#[test]
fn the_orderings_are_actually_different() {
    let all = orderings();
    let shapes: Vec<Vec<String>> = all
        .iter()
        .map(|o| o.iter().map(|r| r.type_name().to_string()).collect())
        .collect();
    let distinct = shapes
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    assert!(
        distinct >= 3,
        "only {distinct} distinct insertion orders; the test above is weaker than it reads"
    );
}
