//! Traversal primitives (§16.3).
//!
//! Four patterns, one property: every result is a `Vec` in dense-id order, so a caller
//! never has to sort and a traversal is a pure function of the graph (rule D).
//!
//! | Function | Used by |
//! |---|---|
//! | [`closure`] | view membership, `bundle`, retraction blast radius |
//! | [`topo`] | thread ordering |
//! | [`reverse_closure`] | `trace`, dependents of a retraction |
//! | [`rebuttals_of`] | rule R closure in packing |

use crate::adjacency::{Adjacency, EdgeKind, EdgeSet, NodeId};

/// A reusable visited-set. Traversals in hot paths thread one of these through rather
/// than allocating a hash set per node (guarantee A6).
#[derive(Debug, Clone, Default)]
pub struct Scratch {
    seen: Vec<bool>,
    stack: Vec<NodeId>,
}

impl Scratch {
    pub fn with_capacity(n: usize) -> Scratch {
        Scratch {
            seen: vec![false; n],
            stack: Vec::new(),
        }
    }

    fn reset(&mut self, n: usize) {
        self.seen.clear();
        self.seen.resize(n, false);
        self.stack.clear();
    }
}

/// Everything reachable from `roots` along `kinds`, including the roots themselves.
///
/// This is what makes a view a root set rather than a container: membership is computed,
/// never stored.
pub fn closure(g: &Adjacency, roots: &[NodeId], kinds: &EdgeSet) -> Vec<NodeId> {
    let mut s = Scratch::with_capacity(g.len());
    closure_with(g, roots, kinds, &mut s)
}

/// [`closure`], reusing a scratch buffer.
pub fn closure_with(
    g: &Adjacency,
    roots: &[NodeId],
    kinds: &EdgeSet,
    s: &mut Scratch,
) -> Vec<NodeId> {
    walk(g, roots, kinds, s, Direction::Forward)
}

/// Everything that reaches `roots` along `kinds` - the dependents rather than the
/// dependencies.
pub fn reverse_closure(g: &Adjacency, roots: &[NodeId], kinds: &EdgeSet) -> Vec<NodeId> {
    let mut s = Scratch::with_capacity(g.len());
    walk(g, roots, kinds, &mut s, Direction::Reverse)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Forward,
    Reverse,
}

fn walk(
    g: &Adjacency,
    roots: &[NodeId],
    kinds: &EdgeSet,
    s: &mut Scratch,
    dir: Direction,
) -> Vec<NodeId> {
    s.reset(g.len());
    for &r in roots {
        if (r as usize) < g.len() && !s.seen[r as usize] {
            s.seen[r as usize] = true;
            s.stack.push(r);
        }
    }
    let mut out: Vec<NodeId> = s.stack.clone();
    while let Some(n) = s.stack.pop() {
        let next = match dir {
            Direction::Forward => g.out(n, kinds),
            Direction::Reverse => g.incoming(n, kinds),
        };
        for m in next {
            if !s.seen[m as usize] {
                s.seen[m as usize] = true;
                s.stack.push(m);
                out.push(m);
            }
        }
    }
    // Dense-id order, so the result does not depend on the walk order.
    out.sort_unstable();
    out
}

/// The result of a topological sort.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopoOrder {
    /// Nodes in dependency order.
    pub order: Vec<NodeId>,
    /// Nodes that could not be ordered because they sit on a cycle, in dense-id order.
    pub cyclic: Vec<NodeId>,
}

impl TopoOrder {
    pub fn is_acyclic(&self) -> bool {
        self.cyclic.is_empty()
    }
}

/// Kahn's algorithm over `kinds`, with the ready set popped in ascending dense id.
///
/// Popping the smallest id rather than the most recently pushed is what makes the order
/// canonical: a stack would make it depend on insertion order.
///
/// Edges point from dependent to dependency, so the returned order lists **dependencies
/// before dependents** - which is what rule M's single reverse pass needs (§17).
pub fn topo(g: &Adjacency, kinds: &EdgeSet) -> TopoOrder {
    let n = g.len();
    // Out-degree, because an edge points at what a node depends on.
    let mut pending: Vec<usize> = (0..n).map(|i| g.out(i as NodeId, kinds).len()).collect();
    let mut ready: Vec<NodeId> = (0..n as NodeId)
        .filter(|&i| pending[i as usize] == 0)
        .collect();
    let mut order = Vec::with_capacity(n);
    let mut done = vec![false; n];

    while !ready.is_empty() {
        ready.sort_unstable();
        let node = ready.remove(0);
        if done[node as usize] {
            continue;
        }
        done[node as usize] = true;
        order.push(node);
        for dependent in g.incoming(node, kinds) {
            let p = &mut pending[dependent as usize];
            *p = p.saturating_sub(1);
            if *p == 0 && !done[dependent as usize] {
                ready.push(dependent);
            }
        }
    }

    let cyclic: Vec<NodeId> = (0..n as NodeId).filter(|&i| !done[i as usize]).collect();
    TopoOrder { order, cyclic }
}

/// Every unit that rebuts `node`, in dense-id order.
///
/// Rule R pins these into any pack containing `node`, always - a budget too small to hold
/// a claim and its rebuttals must drop the claim rather than present it unopposed.
pub fn rebuttals_of(g: &Adjacency, node: NodeId) -> Vec<NodeId> {
    let Some(rebuts) = EdgeKind::kernel(smysl_core::RelKind::Rebuts) else {
        return Vec::new();
    };
    g.incoming(node, &EdgeSet::one(rebuts))
}

/// Cycles over `kinds`, as sets of mutually reachable nodes.
///
/// Reported rather than repaired: a `deps` cycle is `SMY-E061` and a `causes` cycle is
/// only `SMY-W062`, and only the caller knows which it is looking at.
pub fn cycles(g: &Adjacency, kinds: &EdgeSet) -> Vec<Vec<NodeId>> {
    let t = topo(g, kinds);
    if t.is_acyclic() {
        return Vec::new();
    }
    // Group the unordered nodes into mutually reachable sets.
    let mut remaining: Vec<NodeId> = t.cyclic;
    let mut out = Vec::new();
    while let Some(&start) = remaining.first() {
        let forward = closure(g, &[start], kinds);
        let backward = reverse_closure(g, &[start], kinds);
        let mut group: Vec<NodeId> = forward
            .iter()
            .copied()
            .filter(|n| backward.contains(n) && remaining.contains(n))
            .collect();
        group.sort_unstable();
        if group.is_empty() {
            group.push(start);
        }
        remaining.retain(|n| !group.contains(n));
        out.push(group);
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{KernelType, RelKind, Relation, Status, Uid, Unit, UnitCore, UnitCoreBuilder};
    use std::collections::BTreeMap;

    fn uid(n: u8) -> Uid {
        Uid::from_bytes([n; 32])
    }

    fn core(gist: &str, deps: Vec<Uid>) -> UnitCore {
        UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
            .deps(deps)
            .build()
            .unwrap()
    }

    fn graph(units: Vec<(Uid, Vec<Uid>)>, rels: Vec<Relation>) -> Adjacency {
        let map: BTreeMap<Uid, Unit> = units
            .into_iter()
            .map(|(u, deps)| (u, Unit::new(core(&format!("{u}"), deps))))
            .collect();
        Adjacency::build(&map, &rels)
    }

    /// 1 <- 2 <- 3, plus an unrelated 4.
    fn chain() -> Adjacency {
        graph(
            vec![
                (uid(1), vec![]),
                (uid(2), vec![uid(1)]),
                (uid(3), vec![uid(2)]),
                (uid(4), vec![]),
            ],
            vec![],
        )
    }

    #[test]
    fn closure_includes_the_roots_and_everything_below() {
        let g = chain();
        assert_eq!(closure(&g, &[2], &EdgeSet::support()), vec![0, 1, 2]);
        assert_eq!(closure(&g, &[0], &EdgeSet::support()), vec![0]);
        assert_eq!(closure(&g, &[3], &EdgeSet::support()), vec![3]);
    }

    #[test]
    fn closure_from_several_roots_is_the_union() {
        let g = chain();
        assert_eq!(closure(&g, &[1, 3], &EdgeSet::support()), vec![0, 1, 3]);
    }

    #[test]
    fn closure_returns_dense_id_order_regardless_of_walk_order() {
        let g = chain();
        assert_eq!(
            closure(&g, &[2, 0], &EdgeSet::support()),
            closure(&g, &[0, 2], &EdgeSet::support())
        );
    }

    #[test]
    fn closure_of_no_roots_is_empty() {
        assert!(closure(&chain(), &[], &EdgeSet::support()).is_empty());
    }

    #[test]
    fn closure_ignores_out_of_range_roots() {
        assert!(closure(&chain(), &[99], &EdgeSet::support()).is_empty());
    }

    #[test]
    fn reverse_closure_walks_dependents() {
        let g = chain();
        assert_eq!(
            reverse_closure(&g, &[0], &EdgeSet::support()),
            vec![0, 1, 2]
        );
        assert_eq!(reverse_closure(&g, &[2], &EdgeSet::support()), vec![2]);
    }

    #[test]
    fn closure_respects_the_edge_set() {
        let g = graph(
            vec![(uid(1), vec![]), (uid(2), vec![])],
            vec![Relation::new(RelKind::Causes, uid(2), uid(1))],
        );
        assert_eq!(closure(&g, &[1], &EdgeSet::support()), vec![1]);
        assert_eq!(closure(&g, &[1], &EdgeSet::all()), vec![0, 1]);
    }

    #[test]
    fn topo_lists_dependencies_before_dependents() {
        let t = topo(&chain(), &EdgeSet::support());
        assert!(t.is_acyclic());
        // 0 and 3 are both ready at the start; the smallest ready id wins each round, so
        // the unrelated node 3 comes last rather than second.
        assert_eq!(t.order, vec![0, 1, 2, 3]);
        let pos = |n| t.order.iter().position(|&x| x == n).unwrap();
        assert!(pos(0) < pos(1) && pos(1) < pos(2));
    }

    /// The ready set is popped by ascending dense id, so the order cannot depend on
    /// insertion order - which is exactly what rule D asks of thread derivation.
    #[test]
    fn topo_breaks_ties_by_dense_id() {
        let g = graph(
            vec![
                (uid(1), vec![]),
                (uid(2), vec![]),
                (uid(3), vec![]),
                (uid(4), vec![uid(1), uid(2), uid(3)]),
            ],
            vec![],
        );
        let t = topo(&g, &EdgeSet::support());
        assert_eq!(t.order, vec![0, 1, 2, 3]);
    }

    #[test]
    fn topo_is_deterministic_across_runs() {
        let g = chain();
        assert_eq!(topo(&g, &EdgeSet::support()), topo(&g, &EdgeSet::support()));
    }

    #[test]
    fn topo_reports_cyclic_nodes_rather_than_hanging() {
        let g = graph(vec![(uid(1), vec![uid(2)]), (uid(2), vec![uid(1)])], vec![]);
        let t = topo(&g, &EdgeSet::support());
        assert!(!t.is_acyclic());
        assert_eq!(t.cyclic, vec![0, 1]);
        assert!(t.order.is_empty());
    }

    #[test]
    fn a_cycle_does_not_stop_the_rest_being_ordered() {
        let g = graph(
            vec![
                (uid(1), vec![uid(2)]),
                (uid(2), vec![uid(1)]),
                (uid(3), vec![]),
            ],
            vec![],
        );
        let t = topo(&g, &EdgeSet::support());
        assert_eq!(t.order, vec![2]);
        assert_eq!(t.cyclic, vec![0, 1]);
    }

    #[test]
    fn cycles_are_grouped() {
        let g = graph(
            vec![
                (uid(1), vec![uid(2)]),
                (uid(2), vec![uid(1)]),
                (uid(3), vec![uid(4)]),
                (uid(4), vec![uid(3)]),
                (uid(5), vec![]),
            ],
            vec![],
        );
        let c = cycles(&g, &EdgeSet::support());
        assert_eq!(c, vec![vec![0, 1], vec![2, 3]]);
    }

    #[test]
    fn an_acyclic_graph_reports_no_cycles() {
        assert!(cycles(&chain(), &EdgeSet::support()).is_empty());
    }

    #[test]
    fn a_self_loop_is_a_cycle() {
        let g = graph(vec![(uid(1), vec![uid(1)])], vec![]);
        assert_eq!(cycles(&g, &EdgeSet::support()), vec![vec![0]]);
    }

    #[test]
    fn rebuttals_are_found_by_walking_backwards_along_rebuts() {
        let g = graph(
            vec![(uid(1), vec![]), (uid(2), vec![]), (uid(3), vec![])],
            vec![
                Relation::new(RelKind::Rebuts, uid(2), uid(1)),
                Relation::new(RelKind::Rebuts, uid(3), uid(1)),
                Relation::new(RelKind::Causes, uid(3), uid(1)),
            ],
        );
        assert_eq!(rebuttals_of(&g, 0), vec![1, 2]);
        assert!(rebuttals_of(&g, 1).is_empty());
    }

    #[test]
    fn scratch_can_be_reused_across_traversals() {
        let g = chain();
        let mut s = Scratch::with_capacity(g.len());
        let a = closure_with(&g, &[2], &EdgeSet::support(), &mut s);
        let b = closure_with(&g, &[2], &EdgeSet::support(), &mut s);
        let c = closure_with(&g, &[3], &EdgeSet::support(), &mut s);
        assert_eq!(a, b);
        assert_eq!(c, vec![3]);
    }

    #[test]
    fn traversal_over_an_empty_graph_is_empty() {
        let g = Adjacency::default();
        assert!(closure(&g, &[0], &EdgeSet::all()).is_empty());
        assert!(reverse_closure(&g, &[0], &EdgeSet::all()).is_empty());
        assert!(rebuttals_of(&g, 0).is_empty());
        let t = topo(&g, &EdgeSet::all());
        assert!(t.order.is_empty() && t.is_acyclic());
    }
}
