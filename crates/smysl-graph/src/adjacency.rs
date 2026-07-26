//! The purpose-built adjacency store (D-1, §16.2).
//!
//! No `petgraph`. Three edge families, four traversal patterns, and a hard requirement
//! for deterministic iteration order - a general library yields insertion-order iteration,
//! and every traversal would need a wrapping sort to satisfy rule D.
//!
//! Dense integer ids are assigned in **ascending uid order**, which is what makes every
//! traversal canonical without an explicit sort at each step. Determinism is structural
//! here rather than defensive.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::{RelKind, Relation, Uid, Unit};

/// A dense node id. Position in the ascending-uid ordering.
pub type NodeId = u32;

/// What kind of edge connects two units.
///
/// `Copy` and totally ordered, so edge lists sort without allocating. Extension relation
/// kinds are interned per store rather than carried by text, which keeps the whole
/// adjacency structure fixed-width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum EdgeKind {
    /// An interpretive prerequisite (`UnitCore.deps`).
    Deps,
    /// Evidential support (`UnitCore.grounds`).
    Grounds,
    /// One of the fourteen kernel relation kinds, by its Appendix B code.
    Kernel(u8),
    /// An extension relation kind, interned per store.
    Extension(u16),
}

impl EdgeKind {
    pub fn kernel(k: RelKind) -> Option<EdgeKind> {
        k.code().map(EdgeKind::Kernel)
    }

    /// The kernel relation kind this edge carries, if it is one.
    pub fn rel_kind(self) -> Option<RelKind> {
        match self {
            EdgeKind::Kernel(c) => RelKind::from_code(c),
            _ => None,
        }
    }

    /// Whether rank flows along this edge in the salience computation (§16.4).
    ///
    /// `deps` and `grounds` always do; among relations only `causes` and `answers` do.
    pub fn carries_support(self) -> bool {
        match self {
            EdgeKind::Deps | EdgeKind::Grounds => true,
            EdgeKind::Kernel(_) => self.rel_kind().is_some_and(|k| k.carries_support()),
            EdgeKind::Extension(_) => false,
        }
    }

    /// Whether a cycle over this edge kind is an error rather than a warning.
    ///
    /// A cycle in `deps` is `SMY-E061`; a cycle in `causes` or `sequences` is only
    /// `SMY-W062`, because feedback loops are legitimate in a narrative.
    pub fn cycle_is_fatal(self) -> bool {
        matches!(self, EdgeKind::Deps)
    }

    /// The wire code used in the index sidecar.
    pub const fn code(self) -> u16 {
        match self {
            EdgeKind::Deps => 0,
            EdgeKind::Grounds => 1,
            EdgeKind::Kernel(c) => 2 + c as u16,
            EdgeKind::Extension(i) => 1024 + i,
        }
    }

    pub const fn from_code(c: u16) -> Option<EdgeKind> {
        match c {
            0 => Some(EdgeKind::Deps),
            1 => Some(EdgeKind::Grounds),
            2..=15 => Some(EdgeKind::Kernel((c - 2) as u8)),
            1024..=u16::MAX => Some(EdgeKind::Extension(c - 1024)),
            _ => None,
        }
    }
}

/// A set of edge kinds to follow. Traversals take one of these rather than a closure, so
/// the walk order stays a pure function of the graph.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EdgeSet {
    kinds: BTreeSet<EdgeKind>,
    all: bool,
}

impl EdgeSet {
    /// Every edge in the graph.
    pub fn all() -> EdgeSet {
        EdgeSet {
            kinds: BTreeSet::new(),
            all: true,
        }
    }

    pub fn of(kinds: impl IntoIterator<Item = EdgeKind>) -> EdgeSet {
        EdgeSet {
            kinds: kinds.into_iter().collect(),
            all: false,
        }
    }

    /// `deps` and `grounds` - what rule L closure and view membership walk.
    pub fn support() -> EdgeSet {
        EdgeSet::of([EdgeKind::Deps, EdgeKind::Grounds])
    }

    /// What thread ordering runs over: `sequences`, `causes`, `enables` (§19).
    pub fn ordering() -> EdgeSet {
        EdgeSet::of(
            RelKind::KERNEL
                .iter()
                .filter(|k| k.is_ordering())
                .filter_map(|k| EdgeKind::kernel(k.clone())),
        )
    }

    /// Where rank flows for salience: `deps`, `grounds`, `causes`, `answers` (§16.4).
    pub fn support_rank() -> EdgeSet {
        let mut s = EdgeSet::support();
        for k in RelKind::KERNEL.iter().filter(|k| k.carries_support()) {
            if let Some(e) = EdgeKind::kernel(k.clone()) {
                s.kinds.insert(e);
            }
        }
        s
    }

    pub fn one(k: EdgeKind) -> EdgeSet {
        EdgeSet::of([k])
    }

    pub fn contains(&self, k: EdgeKind) -> bool {
        self.all || self.kinds.contains(&k)
    }

    pub fn is_empty(&self) -> bool {
        !self.all && self.kinds.is_empty()
    }
}

/// One outgoing or incoming edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Edge {
    pub kind: EdgeKind,
    pub target: NodeId,
}

/// The adjacency store.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Adjacency {
    /// Ascending uid order. A node's dense id is its position here.
    order: Vec<Uid>,
    index: BTreeMap<Uid, NodeId>,
    /// Which nodes have a unit behind them. A referenced-but-absent uid is still a node,
    /// so dangling references are visible rather than silently dropped.
    present: Vec<bool>,
    fwd: Vec<Vec<Edge>>,
    rev: Vec<Vec<Edge>>,
    /// Interned extension relation kinds, index = `EdgeKind::Extension` payload.
    extensions: Vec<String>,
}

impl Adjacency {
    /// Build from the units and relations of a store.
    ///
    /// Every uid mentioned anywhere becomes a node, present or not, so the dense id space
    /// covers the whole reference graph and dangling edges stay observable.
    pub fn build(units: &BTreeMap<Uid, Unit>, relations: &[Relation]) -> Adjacency {
        let mut all: BTreeSet<Uid> = BTreeSet::new();
        for (uid, u) in units {
            all.insert(*uid);
            all.extend(u.core.references().copied());
        }
        for r in relations {
            all.insert(r.from);
            all.insert(r.to);
            if let Some(n) = r.note {
                all.insert(n);
            }
        }

        let order: Vec<Uid> = all.into_iter().collect();
        let index: BTreeMap<Uid, NodeId> = order
            .iter()
            .enumerate()
            .map(|(i, u)| (*u, i as NodeId))
            .collect();
        let n = order.len();
        let present: Vec<bool> = order.iter().map(|u| units.contains_key(u)).collect();

        let mut extensions: Vec<String> = relations
            .iter()
            .filter(|r| !r.kind.is_kernel())
            .map(|r| r.kind.as_str().to_string())
            .collect();
        extensions.sort();
        extensions.dedup();

        let mut fwd: Vec<Vec<Edge>> = vec![Vec::new(); n];
        let mut rev: Vec<Vec<Edge>> = vec![Vec::new(); n];
        let mut link = |from: NodeId, to: NodeId, kind: EdgeKind| {
            fwd[from as usize].push(Edge { kind, target: to });
            rev[to as usize].push(Edge { kind, target: from });
        };

        for (uid, u) in units {
            let from = index[uid];
            for d in &u.core.deps {
                link(from, index[d], EdgeKind::Deps);
            }
            for g in &u.core.grounds {
                link(from, index[g], EdgeKind::Grounds);
            }
        }
        for r in relations {
            let kind = match EdgeKind::kernel(r.kind.clone()) {
                Some(k) => k,
                None => {
                    let i = extensions
                        .binary_search(&r.kind.as_str().to_string())
                        .expect("interned above");
                    EdgeKind::Extension(i as u16)
                }
            };
            link(index[&r.from], index[&r.to], kind);
        }

        // Sorting by (kind, target) is what makes every traversal canonical without a
        // sort at each step.
        for v in fwd.iter_mut().chain(rev.iter_mut()) {
            v.sort();
            v.dedup();
        }

        Adjacency {
            order,
            index,
            present,
            fwd,
            rev,
            extensions,
        }
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn id(&self, uid: &Uid) -> Option<NodeId> {
        self.index.get(uid).copied()
    }

    pub fn uid(&self, id: NodeId) -> Option<&Uid> {
        self.order.get(id as usize)
    }

    /// Whether a unit record exists for this node, as opposed to it being referenced only.
    pub fn is_present(&self, id: NodeId) -> bool {
        self.present.get(id as usize).copied().unwrap_or(false)
    }

    /// Every node, in dense-id order.
    pub fn nodes(&self) -> impl Iterator<Item = NodeId> {
        0..self.len() as NodeId
    }

    /// Nodes referenced by something but backed by no unit - `SMY-E060` candidates.
    pub fn dangling(&self) -> Vec<NodeId> {
        self.nodes().filter(|&n| !self.is_present(n)).collect()
    }

    pub fn out_edges(&self, id: NodeId) -> &[Edge] {
        self.fwd.get(id as usize).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn in_edges(&self, id: NodeId) -> &[Edge] {
        self.rev.get(id as usize).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Outgoing neighbours along `kinds`, in dense-id order, deduplicated.
    pub fn out(&self, id: NodeId, kinds: &EdgeSet) -> Vec<NodeId> {
        let mut v: Vec<NodeId> = self
            .out_edges(id)
            .iter()
            .filter(|e| kinds.contains(e.kind))
            .map(|e| e.target)
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Incoming neighbours along `kinds`, in dense-id order, deduplicated.
    pub fn incoming(&self, id: NodeId, kinds: &EdgeSet) -> Vec<NodeId> {
        let mut v: Vec<NodeId> = self
            .in_edges(id)
            .iter()
            .filter(|e| kinds.contains(e.kind))
            .map(|e| e.target)
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// The extension kind name behind an interned id.
    pub fn extension_name(&self, i: u16) -> Option<&str> {
        self.extensions.get(i as usize).map(String::as_str)
    }

    pub fn edge_count(&self) -> usize {
        self.fwd.iter().map(Vec::len).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{KernelType, Status, UnitCore, UnitCoreBuilder};

    fn uid(n: u8) -> Uid {
        Uid::from_bytes([n; 32])
    }

    fn core(gist: &str, deps: Vec<Uid>, grounds: Vec<Uid>) -> UnitCore {
        let status = if grounds.is_empty() {
            Status::Speculative
        } else {
            Status::Inferred
        };
        UnitCoreBuilder::new(KernelType::Claim, gist, status)
            .deps(deps)
            .grounds(grounds)
            .build()
            .unwrap()
    }

    /// Build a store-shaped map keyed by the uid the caller wants, rather than by the
    /// content hash, so tests can pin the dense-id ordering.
    fn units(items: Vec<(Uid, UnitCore)>) -> BTreeMap<Uid, Unit> {
        items.into_iter().map(|(u, c)| (u, Unit::new(c))).collect()
    }

    #[test]
    fn dense_ids_follow_ascending_uid_order() {
        let a = Adjacency::build(
            &units(vec![
                (uid(3), core("c", vec![], vec![])),
                (uid(1), core("a", vec![], vec![])),
                (uid(2), core("b", vec![], vec![])),
            ]),
            &[],
        );
        assert_eq!(a.len(), 3);
        assert_eq!(a.uid(0), Some(&uid(1)));
        assert_eq!(a.uid(1), Some(&uid(2)));
        assert_eq!(a.uid(2), Some(&uid(3)));
        assert_eq!(a.id(&uid(2)), Some(1));
    }

    /// This is the whole justification for D-1: ordering is structural, so insertion
    /// order cannot leak into a traversal.
    #[test]
    fn insertion_order_does_not_change_the_structure() {
        let forward = Adjacency::build(
            &units(vec![
                (uid(1), core("a", vec![], vec![])),
                (uid(2), core("b", vec![uid(1)], vec![])),
            ]),
            &[],
        );
        let backward = Adjacency::build(
            &units(vec![
                (uid(2), core("b", vec![uid(1)], vec![])),
                (uid(1), core("a", vec![], vec![])),
            ]),
            &[],
        );
        assert_eq!(forward, backward);
    }

    #[test]
    fn deps_and_grounds_become_edges_in_both_directions() {
        let a = Adjacency::build(
            &units(vec![
                (uid(1), core("e", vec![], vec![])),
                (uid(2), core("c", vec![], vec![uid(1)])),
            ]),
            &[],
        );
        assert_eq!(a.out(1, &EdgeSet::support()), vec![0]);
        assert_eq!(a.incoming(0, &EdgeSet::support()), vec![1]);
        assert_eq!(a.out_edges(1)[0].kind, EdgeKind::Grounds);
    }

    #[test]
    fn edge_sets_filter() {
        let a = Adjacency::build(
            &units(vec![
                (uid(1), core("a", vec![], vec![])),
                (uid(2), core("b", vec![uid(1)], vec![])),
            ]),
            &[],
        );
        assert_eq!(a.out(1, &EdgeSet::one(EdgeKind::Deps)), vec![0]);
        assert!(a.out(1, &EdgeSet::one(EdgeKind::Grounds)).is_empty());
        assert_eq!(a.out(1, &EdgeSet::all()), vec![0]);
        assert!(a.out(1, &EdgeSet::default()).is_empty());
    }

    #[test]
    fn relations_become_edges_keyed_by_their_kernel_code() {
        let a = Adjacency::build(
            &units(vec![
                (uid(1), core("a", vec![], vec![])),
                (uid(2), core("b", vec![], vec![])),
            ]),
            &[Relation::new(RelKind::Rebuts, uid(2), uid(1))],
        );
        let e = a.out_edges(1)[0];
        assert_eq!(e.kind, EdgeKind::Kernel(9));
        assert_eq!(e.kind.rel_kind(), Some(RelKind::Rebuts));
    }

    #[test]
    fn extension_kinds_are_interned() {
        let k = RelKind::parse("x.sre/mitigates").unwrap();
        let a = Adjacency::build(
            &units(vec![
                (uid(1), core("a", vec![], vec![])),
                (uid(2), core("b", vec![], vec![])),
            ]),
            &[Relation::new(k, uid(2), uid(1))],
        );
        let e = a.out_edges(1)[0];
        assert_eq!(e.kind, EdgeKind::Extension(0));
        assert_eq!(a.extension_name(0), Some("x.sre/mitigates"));
        assert_eq!(e.kind.rel_kind(), None);
    }

    #[test]
    fn a_referenced_but_absent_uid_is_a_node_without_a_unit() {
        let a = Adjacency::build(&units(vec![(uid(2), core("b", vec![uid(9)], vec![]))]), &[]);
        assert_eq!(a.len(), 2);
        let missing = a.id(&uid(9)).unwrap();
        assert!(!a.is_present(missing));
        assert!(a.is_present(a.id(&uid(2)).unwrap()));
        assert_eq!(a.dangling(), vec![missing]);
    }

    #[test]
    fn edge_lists_are_sorted_and_deduplicated() {
        let a = Adjacency::build(
            &units(vec![
                (uid(1), core("a", vec![], vec![])),
                (uid(2), core("b", vec![], vec![])),
                (uid(3), core("c", vec![uid(2), uid(1)], vec![uid(1)])),
            ]),
            &[],
        );
        let edges = a.out_edges(2);
        let mut sorted = edges.to_vec();
        sorted.sort();
        assert_eq!(edges, sorted.as_slice());
        // `uid(1)` is both a dep and a ground, so it appears twice under distinct kinds.
        assert_eq!(a.out(2, &EdgeSet::support()), vec![0, 1]);
    }

    #[test]
    fn duplicate_relations_collapse_to_one_edge() {
        let a = Adjacency::build(
            &units(vec![
                (uid(1), core("a", vec![], vec![])),
                (uid(2), core("b", vec![], vec![])),
            ]),
            &[
                Relation::new(RelKind::Causes, uid(2), uid(1)),
                Relation::new(RelKind::Causes, uid(2), uid(1)),
            ],
        );
        assert_eq!(a.edge_count(), 1);
    }

    #[test]
    fn edge_kind_codes_round_trip() {
        let mut kinds = vec![EdgeKind::Deps, EdgeKind::Grounds];
        for c in 0..14u8 {
            kinds.push(EdgeKind::Kernel(c));
        }
        kinds.push(EdgeKind::Extension(0));
        kinds.push(EdgeKind::Extension(7));
        for k in kinds {
            assert_eq!(EdgeKind::from_code(k.code()), Some(k), "{k:?}");
        }
        assert_eq!(EdgeKind::from_code(16), None);
    }

    #[test]
    fn support_rank_covers_exactly_the_section_16_4_edges() {
        let s = EdgeSet::support_rank();
        assert!(s.contains(EdgeKind::Deps));
        assert!(s.contains(EdgeKind::Grounds));
        assert!(s.contains(EdgeKind::kernel(RelKind::Causes).unwrap()));
        assert!(s.contains(EdgeKind::kernel(RelKind::Answers).unwrap()));
        assert!(!s.contains(EdgeKind::kernel(RelKind::Rebuts).unwrap()));
        assert!(!s.contains(EdgeKind::kernel(RelKind::Elaborates).unwrap()));
    }

    #[test]
    fn ordering_covers_exactly_the_section_19_edges() {
        let s = EdgeSet::ordering();
        for k in [RelKind::Sequences, RelKind::Causes, RelKind::Enables] {
            assert!(s.contains(EdgeKind::kernel(k).unwrap()));
        }
        assert!(!s.contains(EdgeKind::kernel(RelKind::Rebuts).unwrap()));
        assert!(!s.contains(EdgeKind::Deps));
    }

    /// Only a `deps` cycle is fatal: narrative feedback loops over `causes` and
    /// `sequences` are legitimate and only warn.
    #[test]
    fn only_a_deps_cycle_is_fatal() {
        assert!(EdgeKind::Deps.cycle_is_fatal());
        assert!(!EdgeKind::Grounds.cycle_is_fatal());
        assert!(!EdgeKind::kernel(RelKind::Causes).unwrap().cycle_is_fatal());
    }

    #[test]
    fn support_carrying_edges_are_identified() {
        assert!(EdgeKind::Deps.carries_support());
        assert!(EdgeKind::Grounds.carries_support());
        assert!(EdgeKind::kernel(RelKind::Causes).unwrap().carries_support());
        assert!(!EdgeKind::kernel(RelKind::Rebuts).unwrap().carries_support());
        assert!(!EdgeKind::Extension(0).carries_support());
    }

    #[test]
    fn an_empty_graph_is_well_formed() {
        let a = Adjacency::build(&BTreeMap::new(), &[]);
        assert!(a.is_empty());
        assert_eq!(a.len(), 0);
        assert_eq!(a.id(&uid(1)), None);
        assert_eq!(a.uid(0), None);
        assert!(a.out_edges(0).is_empty());
        assert!(a.dangling().is_empty());
    }
}
