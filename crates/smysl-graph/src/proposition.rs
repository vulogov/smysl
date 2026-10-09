//! Proposition classes: which units are saying the same thing (SMYSL-2.3 A-12.4, SMYSL-2.4 §3.6).
//!
//! A `x.text/same-as` edge is a claim that two units are the same proposition. A **class** is
//! what a set of those edges adds up to, and `strict` is the one policy whose answer is
//! normative: two implementations that disagree about the classes of one store disagree about
//! how many propositions it holds, which is the number every corpus measure is divided by.
//!
//! # Why this is in `smysl-graph` and not in `smysl-text`
//!
//! SMYSL-2.4 §4.1's tree puts `proposition/` in `smysl-text`, beside the engines that *propose*
//! the edges — and those belong there: `lexical` needs retrieval and `anchored` needs a
//! `Library`. Counting the classes needs neither. It reads units, relations, withdrawals and
//! attestations, which are `Store`'s, and it has to be reachable from **`smysl-check`**: A-13
//! gives the `C-Library` conformance class the obligation to "derive `strict` classes per
//! A-12.4", and `smysl-check` may not depend on `smysl-text` (§4.3.3 allows exactly one such
//! edge, the `Time` pass's). A module a conformance class needs and a checker cannot reach
//! would be an obligation nothing can discharge.
//!
//! # What `strict` is, and what it is not
//!
//! A-12.4's procedure, verbatim in effect: edges undirected, units in ascending uid order, the
//! smallest unassigned unit opens a class, and every later unassigned unit joins if it is
//! adjacent to **every** current member. It is a greedy clique partition — seed, then grow —
//! and it is deterministic because the order is the uid order and nothing else.
//!
//! It is **not** a minimum clique cover and does not claim to be. A path `a—b—c` yields `{a, b}`
//! and `{c}`: `c` is adjacent to `b` but not to `a`, so it cannot join the class `a` opened, and
//! it is in the vertex set because it has an edge — so it opens a class of its own. A singleton
//! class is therefore a normal outcome and says something true: that unit is claimed to be the
//! same proposition as something, and the claims do not agree closely enough to put them
//! together.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use smysl_core::ids::{AgentId, Uid};
use smysl_core::types::RelKind;

use crate::Store;

/// The relation kind a class is built from.
///
/// `x.text/same-as`, as SMYSL-2.4 §3.6 and A-11's `x.text/v1` name it. A constant rather than a
/// string literal at each use: it is the one piece of vocabulary this module and the engines
/// that fill it have to agree on.
pub const SAME_AS: &str = "x.text/same-as";

/// How a class is formed.
///
/// `Strict` is normative (A-12.4). The other two are **tool-level** and say so: they answer
/// different questions, and a corpus figure derived from either is a figure about this
/// implementation's choice rather than about the format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[non_exhaustive]
pub enum Policy {
    /// A-12.4's greedy clique partition. The only policy two implementations must agree on.
    #[default]
    Strict,
    /// Connected components of the edge graph, reported **with a diameter**.
    ///
    /// The diameter is the point. A component is as large as the transitive closure of every
    /// claim in it, so one wrong edge merges two propositions and nothing says so — except the
    /// diameter, which grows. A component of diameter 1 is a clique and means what `strict`
    /// means; a component of diameter 6 is a chain of six claims nobody made together.
    Component,
    /// `Strict`, over edges carrying at least `n` distinct attesting agents.
    ///
    /// SMYSL-2.4 §3.6 defaults corpus measures to `attested:2`, because S0's proposers reached
    /// no cell of the precision bar under either provisional gold: one agent's opinion about
    /// two units being one proposition is a proposal, and the measures are divided by classes.
    Attested(u32),
}

/// One class of units claimed to be the same proposition.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct Class {
    /// The class's identity: its smallest member, which is the unit that opened it (A-12.4).
    pub id: Uid,
    /// Every member, including `id`.
    pub members: BTreeSet<Uid>,
    /// The longest shortest path inside the class, in edges.
    ///
    /// `Some` only under [`Policy::Component`], where it is the thing being reported. Under
    /// `Strict` and `Attested` every class is a clique by construction, so a diameter would be
    /// 1 for every class larger than one and 0 for a singleton — a column of constants, which
    /// is why it is `None` rather than computed.
    pub diameter: Option<u32>,
}

impl Class {
    pub fn len(&self) -> usize {
        self.members.len()
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }
}

/// The classes of a store under a policy, in class-id order.
///
/// `scope` restricts the units considered: `Some(set)` is those units, `None` is every unit the
/// store holds. **An `Option` and not SMYSL-2.4 §4.2's bare `&BTreeSet<Uid>`**, because with a
/// bare set the empty one has to mean something, and both meanings are wrong: "every unit" makes
/// a caller whose computed scope came out empty count the whole store, and "no unit" makes
/// `classes(&store, Strict, &BTreeSet::new())` — the obvious way to ask for all of them — return
/// nothing. `None` says which one is meant.
///
/// Pure, and independent of the order records arrived in: the vertex set is sorted by uid, the
/// adjacency is a `BTreeMap` of `BTreeSet`s, and nothing reads the log's order.
pub fn classes(store: &Store, policy: Policy, scope: Option<&BTreeSet<Uid>>) -> Vec<Class> {
    let edges = adjacency(store, policy, scope);
    match policy {
        Policy::Component => components(&edges),
        // `Attested` differs from `Strict` only in which edges reached `adjacency`.
        Policy::Strict | Policy::Attested(_) => strict(&edges),
    }
}

/// The undirected adjacency a policy admits.
///
/// Three filters, and each is a sentence from §3.6: the edge is `x.text/same-as`; it is **live**
/// — not withdrawn, both endpoints present — and both endpoints are in scope; and under
/// `Attested(n)` it carries at least `n` distinct attesting agents.
fn adjacency(
    store: &Store,
    policy: Policy,
    scope: Option<&BTreeSet<Uid>>,
) -> BTreeMap<Uid, BTreeSet<Uid>> {
    let present: BTreeSet<Uid> = store.units().map(|(uid, _)| *uid).collect();
    let in_scope = |uid: &Uid| present.contains(uid) && scope.is_none_or(|s| s.contains(uid));
    let needed = match policy {
        Policy::Attested(n) => n,
        _ => 0,
    };

    let mut out: BTreeMap<Uid, BTreeSet<Uid>> = BTreeMap::new();
    for rel in store.relations() {
        match &rel.kind {
            RelKind::Extension(name) if name == SAME_AS => {}
            _ => continue,
        }
        if rel.from == rel.to {
            // A unit is the same proposition as itself, which is true and is not a class.
            // Admitting it would make every self-edged unit a vertex with no neighbour, and
            // A-12.4's vertex set is "units with at least one edge".
            continue;
        }
        if !in_scope(&rel.from) || !in_scope(&rel.to) || store.is_withdrawn(rel) {
            continue;
        }
        if needed > 0 {
            let agents: BTreeSet<&AgentId> = store
                .attestations_of(&rel.uid())
                .iter()
                .map(|a| &a.agent)
                .collect();
            if (agents.len() as u32) < needed {
                continue;
            }
        }
        out.entry(rel.from).or_default().insert(rel.to);
        out.entry(rel.to).or_default().insert(rel.from);
    }
    out
}

/// A-12.4: seed with the smallest unassigned unit, then grow in uid order.
fn strict(edges: &BTreeMap<Uid, BTreeSet<Uid>>) -> Vec<Class> {
    let vertices: Vec<Uid> = edges
        .iter()
        .filter(|(_, ns)| !ns.is_empty())
        .map(|(u, _)| *u)
        .collect();
    let mut assigned: BTreeSet<Uid> = BTreeSet::new();
    let mut out = Vec::new();
    for seed in &vertices {
        if assigned.contains(seed) {
            continue;
        }
        let mut members: BTreeSet<Uid> = BTreeSet::new();
        members.insert(*seed);
        assigned.insert(*seed);
        // "Every later unassigned unit, in ascending uid order": later than the seed, which the
        // ordered vertex list gives for free.
        for candidate in vertices.iter().filter(|c| *c > seed) {
            if assigned.contains(candidate) {
                continue;
            }
            let neighbours = &edges[candidate];
            if members.iter().all(|m| neighbours.contains(m)) {
                members.insert(*candidate);
                assigned.insert(*candidate);
            }
        }
        out.push(Class {
            id: *seed,
            members,
            diameter: None,
        });
    }
    out
}

/// Connected components, each with its diameter.
fn components(edges: &BTreeMap<Uid, BTreeSet<Uid>>) -> Vec<Class> {
    let mut seen: BTreeSet<Uid> = BTreeSet::new();
    let mut out = Vec::new();
    for start in edges.keys() {
        if seen.contains(start) || edges[start].is_empty() {
            continue;
        }
        let mut members: BTreeSet<Uid> = BTreeSet::new();
        let mut queue = VecDeque::from([*start]);
        while let Some(u) = queue.pop_front() {
            if !members.insert(u) {
                continue;
            }
            for n in &edges[&u] {
                if !members.contains(n) {
                    queue.push_back(*n);
                }
            }
        }
        seen.extend(members.iter().copied());
        let diameter = diameter_of(edges, &members);
        out.push(Class {
            id: *members.iter().next().expect("a component has a member"),
            members,
            diameter: Some(diameter),
        });
    }
    out
}

/// The longest shortest path inside a component, in edges.
///
/// A breadth-first search from every member, which is `O(V·E)` over the component and is what a
/// diameter costs without an approximation. Components are small — a component large enough for
/// this to matter is a component whose diameter has already said it should not exist.
fn diameter_of(edges: &BTreeMap<Uid, BTreeSet<Uid>>, members: &BTreeSet<Uid>) -> u32 {
    let mut worst = 0u32;
    for start in members {
        let mut depth: BTreeMap<Uid, u32> = BTreeMap::new();
        depth.insert(*start, 0);
        let mut queue = VecDeque::from([*start]);
        while let Some(u) = queue.pop_front() {
            let d = depth[&u];
            for n in &edges[&u] {
                if !depth.contains_key(n) {
                    depth.insert(*n, d + 1);
                    queue.push_back(*n);
                }
            }
        }
        worst = worst.max(depth.values().copied().max().unwrap_or(0));
    }
    worst
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::types::provenance::{Attestation, Hlc, Op, Rung};
    use smysl_core::{canonical_uid, KernelType, Record, Relation, Status, UnitCoreBuilder};

    fn agent(name: &str) -> AgentId {
        AgentId::new(name).expect("an agent id")
    }

    /// A unit whose gist is `gist`, and its uid.
    fn unit(gist: &str) -> (Record, Uid) {
        let u = UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
            .build()
            .expect("a unit");
        let uid = canonical_uid(&u);
        (Record::Unit(u), uid)
    }

    fn same_as(from: Uid, to: Uid) -> Relation {
        Relation::new(RelKind::Extension(SAME_AS.to_string()), from, to)
    }

    /// `n` units named by a single letter, so the test can talk about them.
    ///
    /// Returned sorted by uid, because A-12.4's procedure is in uid order and a test that
    /// thought in insertion order would be asserting something else. The names are therefore
    /// positions, not labels: `u[0]` is the smallest uid whatever its gist.
    fn units(n: usize) -> (Vec<Record>, Vec<Uid>) {
        let mut pairs: Vec<(Record, Uid)> = (0..n)
            .map(|i| unit(&format!("proposition number {i}")))
            .collect();
        pairs.sort_by_key(|(_, uid)| *uid);
        (
            pairs.iter().map(|(r, _)| r.clone()).collect(),
            pairs.iter().map(|(_, u)| *u).collect(),
        )
    }

    fn store_of(records: Vec<Record>) -> Store {
        Store::from_records(records)
    }

    /// A triangle is one class under every policy, and its id is its smallest member.
    #[test]
    fn a_clique_is_one_class_identified_by_its_smallest_member() {
        let (mut records, u) = units(3);
        for (a, b) in [(0, 1), (1, 2), (0, 2)] {
            records.push(Record::Relation(same_as(u[a], u[b])));
        }
        let store = store_of(records);

        let strict = classes(&store, Policy::Strict, None);
        assert_eq!(strict.len(), 1);
        assert_eq!(strict[0].id, u[0], "the smallest member opened it");
        assert_eq!(strict[0].members, u.iter().copied().collect());
        assert_eq!(strict[0].diameter, None, "strict reports no diameter");

        let component = classes(&store, Policy::Component, None);
        assert_eq!(component.len(), 1);
        assert_eq!(component[0].diameter, Some(1), "a clique has diameter 1");
    }

    /// **A path is two classes under `strict` and one under `component`.**
    ///
    /// The case that shows the two policies are different questions rather than two
    /// implementations of one. `a—b—c`: `a` opens a class, `b` joins it, `c` is adjacent to `b`
    /// and not to `a` so it cannot — and `c` has an edge, so it opens a class of one. A
    /// component says the three are connected and that its diameter is 2, which is the number
    /// that tells you the three were never claimed to be one thing together.
    #[test]
    fn a_path_is_two_strict_classes_and_one_component_of_diameter_two() {
        let (mut records, u) = units(3);
        records.push(Record::Relation(same_as(u[0], u[1])));
        records.push(Record::Relation(same_as(u[1], u[2])));
        let store = store_of(records);

        let strict = classes(&store, Policy::Strict, None);
        assert_eq!(strict.len(), 2, "{strict:?}");
        assert_eq!(strict[0].id, u[0]);
        assert_eq!(strict[0].members, [u[0], u[1]].into_iter().collect());
        assert_eq!(strict[1].id, u[2]);
        assert_eq!(strict[1].members, [u[2]].into_iter().collect());

        let component = classes(&store, Policy::Component, None);
        assert_eq!(component.len(), 1);
        assert_eq!(component[0].members.len(), 3);
        assert_eq!(component[0].diameter, Some(2));
    }

    /// A unit with no edge forms no class, which is A-12.4's last sentence.
    #[test]
    fn a_unit_with_no_edge_is_in_no_class() {
        let (mut records, u) = units(3);
        records.push(Record::Relation(same_as(u[0], u[1])));
        let store = store_of(records);
        let found = classes(&store, Policy::Strict, None);
        assert_eq!(found.len(), 1);
        assert!(!found[0].members.contains(&u[2]));
    }

    /// The classes do not depend on the order the records arrived in.
    ///
    /// Half of this step's exit, and the half that a greedy algorithm has to be asked about:
    /// the growth order is the uid order, so a store that received its edges backwards has to
    /// come out the same.
    #[test]
    fn the_classes_are_the_same_whatever_order_the_records_arrived_in() {
        let (mut records, u) = units(5);
        for (a, b) in [(0, 1), (1, 2), (0, 2), (3, 4), (2, 3)] {
            records.push(Record::Relation(same_as(u[a], u[b])));
        }
        let forward = classes(&store_of(records.clone()), Policy::Strict, None);
        records.reverse();
        let backward = classes(&store_of(records), Policy::Strict, None);
        assert_eq!(forward, backward);
        assert!(
            forward.len() > 1,
            "a partition worth comparing: {forward:?}"
        );
    }

    /// A withdrawn edge is not an edge.
    #[test]
    fn a_withdrawn_edge_does_not_hold_a_class_together() {
        let (mut records, u) = units(2);
        let rel = same_as(u[0], u[1]);
        let rid = rel.uid();
        records.push(Record::Relation(rel));
        let store = store_of(records.clone());
        assert_eq!(classes(&store, Policy::Strict, None).len(), 1);

        let who = agent("human:vu");
        records.push(Record::Withdrawal(smysl_core::Withdrawal::new(
            rid,
            who.clone(),
            Hlc::new(1, 0, who),
        )));
        let store = store_of(records);
        assert!(
            classes(&store, Policy::Strict, None).is_empty(),
            "the edge was withdrawn, so neither unit has one"
        );
    }

    /// An edge to a unit the store does not hold is not an edge either.
    #[test]
    fn an_edge_with_an_absent_endpoint_is_not_live() {
        let (mut records, u) = units(2);
        let (_, missing) = unit("a unit this store never received");
        records.push(Record::Relation(same_as(u[0], missing)));
        records.push(Record::Relation(same_as(u[0], u[1])));
        let store = store_of(records);
        let found = classes(&store, Policy::Strict, None);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].members, [u[0], u[1]].into_iter().collect());
    }

    /// `attested:n` counts **distinct agents**, not attestations.
    #[test]
    fn attested_counts_distinct_agents_on_the_edge() {
        let (mut records, u) = units(2);
        let rel = same_as(u[0], u[1]);
        let rid = rel.uid();
        records.push(Record::Relation(rel));
        // Two attestations by one agent is one agent.
        for i in 0..2 {
            records.push(Record::Attestation(Attestation::new(
                rid,
                agent("tool:smysl-same-as-lexical"),
                Op::Attested,
                Rung::Computed,
                Hlc::new(i, 0, agent("tool:smysl-same-as-lexical")),
            )));
        }
        let store = store_of(records.clone());
        assert_eq!(classes(&store, Policy::Strict, None).len(), 1);
        assert_eq!(classes(&store, Policy::Attested(1), None).len(), 1);
        assert!(
            classes(&store, Policy::Attested(2), None).is_empty(),
            "one agent twice is one agent"
        );

        records.push(Record::Attestation(Attestation::new(
            rid,
            agent("human:vu"),
            Op::Attested,
            // `Document`, not a human rung: `Rung` names where the *content* came from, and
            // there is no `Human` — the agent id is what says who. A detail worth meeting here
            // rather than in the engine that will write these for real.
            Rung::Document,
            Hlc::new(2, 0, agent("human:vu")),
        )));
        let store = store_of(records);
        assert_eq!(
            classes(&store, Policy::Attested(2), None).len(),
            1,
            "two agents is two agents"
        );
    }

    /// A scope of `Some` is those units; `None` is all of them.
    #[test]
    fn a_scope_restricts_the_units_and_none_means_every_unit() {
        let (mut records, u) = units(4);
        for (a, b) in [(0, 1), (2, 3)] {
            records.push(Record::Relation(same_as(u[a], u[b])));
        }
        let store = store_of(records);
        assert_eq!(classes(&store, Policy::Strict, None).len(), 2);

        let only_first: BTreeSet<Uid> = [u[0], u[1]].into_iter().collect();
        let scoped = classes(&store, Policy::Strict, Some(&only_first));
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].members, only_first);

        // And an empty scope is no units, which is the reading the `Option` exists to make
        // sayable: a caller whose scope came out empty asked for nothing, not for everything.
        assert!(classes(&store, Policy::Strict, Some(&BTreeSet::new())).is_empty());
    }

    /// A self-edge is true and is not a class.
    #[test]
    fn a_unit_is_not_classed_with_itself() {
        let (mut records, u) = units(1);
        records.push(Record::Relation(same_as(u[0], u[0])));
        let store = store_of(records);
        assert!(classes(&store, Policy::Strict, None).is_empty());
    }

    /// Only `x.text/same-as` builds a class.
    #[test]
    fn another_relation_kind_is_not_a_sameness_claim() {
        let (mut records, u) = units(2);
        records.push(Record::Relation(Relation::new(
            RelKind::Elaborates,
            u[0],
            u[1],
        )));
        records.push(Record::Relation(Relation::new(
            RelKind::Extension("x.text/translates".to_string()),
            u[0],
            u[1],
        )));
        let store = store_of(records);
        assert!(classes(&store, Policy::Strict, None).is_empty());
    }
}
