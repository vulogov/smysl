//! Lineage: where a unit came from, and what changed between hops (§16.6).
//!
//! This is the answer to F3 - provenance evaporation. After three hops of prose nothing
//! distinguishes source-derived content from model priors. Here every unit carries the
//! attestations that produced it, so "who changed this, and at which hop" is a walk rather
//! than an archaeology.
//!
//! The distinction `--recipe` draws is the one that matters in practice: **the output
//! changed because the prompt changed** versus **the output changed because the content
//! changed**. A recipe hash covers the full conditions of a model call; `recipe_family`
//! excludes the provider and model, so the same logical ingest across two vendors shares a
//! family and differs in recipe (D-8).

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::{AgentId, RelKind, Uid};

use crate::adjacency::{EdgeKind, EdgeSet};
use crate::store::Store;
use crate::traverse;

// ---------------------------------------------------------------------------
// trace
// ---------------------------------------------------------------------------

/// Which lineage a `trace` follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum TraceKind {
    /// Causal: attestation `parents` and `supersedes`. Where this unit came from.
    Parents,
    /// Evidential: `grounds` and `deps`. What this unit rests on.
    Grounds,
    /// Both at once.
    Both,
}

impl TraceKind {
    pub const ALL: &'static [TraceKind] =
        &[TraceKind::Parents, TraceKind::Grounds, TraceKind::Both];

    pub const fn as_str(self) -> &'static str {
        match self {
            TraceKind::Parents => "parents",
            TraceKind::Grounds => "grounds",
            TraceKind::Both => "both",
        }
    }

    pub fn parse(s: &str) -> Option<TraceKind> {
        TraceKind::ALL.iter().copied().find(|k| k.as_str() == s)
    }

    const fn follows_parents(self) -> bool {
        matches!(self, TraceKind::Parents | TraceKind::Both)
    }

    const fn follows_grounds(self) -> bool {
        matches!(self, TraceKind::Grounds | TraceKind::Both)
    }
}

/// How a unit was reached during a trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Via {
    /// The unit the trace started from.
    Root,
    /// An attestation named it as a parent.
    Parent,
    /// It was superseded by the unit that reached it.
    Supersedes,
    Grounds,
    Deps,
}

impl Via {
    pub const fn as_str(self) -> &'static str {
        match self {
            Via::Root => "root",
            Via::Parent => "parent",
            Via::Supersedes => "supersedes",
            Via::Grounds => "grounds",
            Via::Deps => "deps",
        }
    }
}

/// One step of a lineage walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineageNode {
    pub uid: Uid,
    /// Steps from the unit the trace started at.
    pub depth: u32,
    pub via: Via,
    /// Every agent that attested this unit.
    pub agents: BTreeSet<AgentId>,
    /// The earliest hop any agent attested it at.
    pub hop: Option<u32>,
}

/// Where a unit came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lineage {
    pub root: Uid,
    pub kind: TraceKind,
    /// Reached units, in `(depth, uid)` order - so the output is a function of the graph
    /// rather than of the walk (rule D).
    pub nodes: Vec<LineageNode>,
}

impl Lineage {
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Every agent that had a hand in this unit's ancestry.
    pub fn agents(&self) -> BTreeSet<&AgentId> {
        self.nodes.iter().flat_map(|n| n.agents.iter()).collect()
    }

    /// How far back the walk reached.
    pub fn max_depth(&self) -> u32 {
        self.nodes.iter().map(|n| n.depth).max().unwrap_or(0)
    }
}

/// Walk a unit's ancestry.
///
/// `depth` bounds how far back to go; `None` walks to the roots. The result is sorted by
/// `(depth, uid)`, so two runs over the same store agree exactly.
pub fn trace(store: &Store, root: Uid, kind: TraceKind, depth: Option<u32>) -> Lineage {
    let mut nodes: Vec<LineageNode> = Vec::new();
    let mut seen: BTreeSet<Uid> = BTreeSet::new();
    let mut frontier: Vec<(Uid, Via)> = vec![(root, Via::Root)];
    seen.insert(root);

    let mut d = 0u32;
    while !frontier.is_empty() {
        if let Some(limit) = depth {
            if d > limit {
                break;
            }
        }
        // Canonical order within a depth level.
        frontier.sort();

        let mut next: Vec<(Uid, Via)> = Vec::new();
        for (uid, via) in frontier {
            let (agents, hop) = attribution(store, &uid);
            nodes.push(LineageNode {
                uid,
                depth: d,
                via,
                agents,
                hop,
            });

            if kind.follows_parents() {
                if let Some(unit) = store.get(&uid) {
                    for a in &unit.attestations {
                        for p in &a.parents {
                            if seen.insert(*p) {
                                next.push((*p, Via::Parent));
                            }
                        }
                    }
                }
                // A unit that supersedes something came from it.
                for rel in store.relations_of_kind(&RelKind::Supersedes) {
                    if rel.from == uid && seen.insert(rel.to) {
                        next.push((rel.to, Via::Supersedes));
                    }
                }
            }

            if kind.follows_grounds() {
                if let Some(unit) = store.get(&uid) {
                    for g in &unit.core.grounds {
                        if seen.insert(*g) {
                            next.push((*g, Via::Grounds));
                        }
                    }
                    for dep in &unit.core.deps {
                        if seen.insert(*dep) {
                            next.push((*dep, Via::Deps));
                        }
                    }
                }
            }
        }
        frontier = next;
        d += 1;
    }

    nodes.sort_by_key(|n| (n.depth, n.uid));
    Lineage { root, kind, nodes }
}

/// Who attested a unit, and at what hop it first appeared.
fn attribution(store: &Store, uid: &Uid) -> (BTreeSet<AgentId>, Option<u32>) {
    match store.get(uid) {
        Some(u) => (
            u.attestations.iter().map(|a| a.agent.clone()).collect(),
            u.attestations.iter().map(|a| a.hop).min(),
        ),
        None => (BTreeSet::new(), None),
    }
}

/// Everything that depends on a unit - the direction `trace` does not go.
///
/// Used for a retraction's blast radius and for "what breaks if this is wrong".
pub fn dependents(store: &Store, uid: Uid) -> Vec<Uid> {
    let g = store.adjacency();
    let Some(id) = g.id(&uid) else {
        return Vec::new();
    };
    traverse::reverse_closure(g, &[id], &EdgeSet::support())
        .into_iter()
        .filter_map(|n| g.uid(n))
        .copied()
        .filter(|u| *u != uid)
        .collect()
}

// ---------------------------------------------------------------------------
// diff between stores
// ---------------------------------------------------------------------------

/// What two stores disagree about, by presence.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreDiff {
    pub only_in_a: Vec<Uid>,
    pub only_in_b: Vec<Uid>,
    pub common: Vec<Uid>,
}

impl StoreDiff {
    pub fn is_identical(&self) -> bool {
        self.only_in_a.is_empty() && self.only_in_b.is_empty()
    }
}

/// Compare two stores by unit membership.
pub fn diff(a: &Store, b: &Store) -> StoreDiff {
    let ua: BTreeSet<Uid> = a.units().map(|(u, _)| *u).collect();
    let ub: BTreeSet<Uid> = b.units().map(|(u, _)| *u).collect();
    StoreDiff {
        only_in_a: ua.difference(&ub).copied().collect(),
        only_in_b: ub.difference(&ua).copied().collect(),
        common: ua.intersection(&ub).copied().collect(),
    }
}

// ---------------------------------------------------------------------------
// diff across hops
// ---------------------------------------------------------------------------

/// Why a unit's provenance changed between hops (D-8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum RecipeChangeKind {
    /// Same `recipe_family`, different `recipe`: the same logical ingest ran against a
    /// different provider or model.
    ProviderChanged,
    /// Different `recipe_family`: the prompt itself changed.
    TemplateChanged,
}

impl RecipeChangeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            RecipeChangeKind::ProviderChanged => "provider-changed",
            RecipeChangeKind::TemplateChanged => "template-changed",
        }
    }
}

/// A unit whose provenance conditions moved between hops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeChange {
    pub uid: Uid,
    pub kind: RecipeChangeKind,
}

/// What one agent did across a hop range.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentActivity {
    pub added: usize,
    pub superseded: usize,
    pub retracted: usize,
}

impl AgentActivity {
    pub fn total(&self) -> usize {
        self.added + self.superseded + self.retracted
    }
}

/// The partition of §16.6.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HopDiff {
    pub from: u32,
    pub to: u32,
    /// Present at `from` and untouched since.
    pub survived: Vec<Uid>,
    /// Present at `from` and superseded by something at or before `to`.
    pub superseded: Vec<(Uid, Uid)>,
    /// Present at `from` and retracted at or before `to`.
    pub retracted: Vec<Uid>,
    /// First attested after `from`.
    pub added: Vec<Uid>,
    pub by_agent: BTreeMap<AgentId, AgentActivity>,
    /// Populated only when recipes are requested.
    pub recipe_changes: Vec<RecipeChange>,
}

impl HopDiff {
    /// The fraction of `from`'s units still standing unmodified at `to`.
    ///
    /// This is E2 - claim survival - measured directly rather than judged.
    pub fn survival_rate(&self) -> f64 {
        let before = self.survived.len() + self.superseded.len() + self.retracted.len();
        if before == 0 {
            return 1.0;
        }
        self.survived.len() as f64 / before as f64
    }

    /// Every unit accounted for, in either partition.
    pub fn total(&self) -> usize {
        self.survived.len() + self.superseded.len() + self.retracted.len() + self.added.len()
    }
}

/// Partition a store's units across a hop range (§16.6).
///
/// A unit's hop is the earliest any agent attested it. Units with no attestation cannot be
/// placed in time and are excluded - a store that never recorded provenance cannot be asked
/// what changed when.
pub fn hop_diff(store: &Store, from: u32, to: u32, recipes: bool) -> HopDiff {
    let mut out = HopDiff {
        from,
        to,
        ..HopDiff::default()
    };

    let hops: BTreeMap<Uid, u32> = store
        .units()
        .filter_map(|(u, unit)| {
            unit.attestations
                .iter()
                .map(|a| a.hop)
                .min()
                .map(|h| (*u, h))
        })
        .collect();

    let before: BTreeSet<Uid> = hops
        .iter()
        .filter(|(_, h)| **h <= from)
        .map(|(u, _)| *u)
        .collect();
    let after_window: BTreeSet<Uid> = hops
        .iter()
        .filter(|(_, h)| **h > from && **h <= to)
        .map(|(u, _)| *u)
        .collect();

    // A supersession or retraction counts only if the edge itself arrived in the window.
    //
    // An edge is dated by its own attestation where it has one, and otherwise by the unit
    // that issued it - a successor dates the supersession that produced it. An edge with
    // neither cannot be placed in time and is treated as always present, which is the
    // conservative reading: an undated withdrawal is still a withdrawal.
    let edge_hop = |rel: &smysl_core::Relation| -> Option<u32> {
        rel.hop().or_else(|| {
            (rel.from != rel.to)
                .then(|| hops.get(&rel.from).copied())
                .flatten()
        })
    };
    let acted_in_window = |rel: &smysl_core::Relation| -> bool {
        match edge_hop(rel) {
            Some(h) => h > from && h <= to,
            None => true,
        }
    };

    let mut superseded_by: BTreeMap<Uid, Uid> = BTreeMap::new();
    for rel in store.relations_of_kind(&RelKind::Supersedes) {
        if before.contains(&rel.to) && acted_in_window(rel) {
            superseded_by.insert(rel.to, rel.from);
        }
    }
    let mut retracted: BTreeSet<Uid> = BTreeSet::new();
    for rel in store.relations_of_kind(&RelKind::Retracts) {
        if before.contains(&rel.to) && acted_in_window(rel) {
            retracted.insert(rel.to);
        }
    }

    for uid in &before {
        if retracted.contains(uid) {
            out.retracted.push(*uid);
        } else if let Some(successor) = superseded_by.get(uid) {
            out.superseded.push((*uid, *successor));
        } else {
            out.survived.push(*uid);
        }
    }
    out.added = after_window.iter().copied().collect();

    // Attribution: who did each of those things.
    for uid in &out.added {
        for agent in agents_at(store, uid) {
            out.by_agent.entry(agent).or_default().added += 1;
        }
    }
    for (_, successor) in &out.superseded {
        for agent in agents_at(store, successor) {
            out.by_agent.entry(agent).or_default().superseded += 1;
        }
    }
    for uid in &out.retracted {
        for agent in agents_at(store, uid) {
            out.by_agent.entry(agent).or_default().retracted += 1;
        }
    }

    if recipes {
        out.recipe_changes = recipe_changes(store, &before, &after_window);
    }

    out.survived.sort();
    out.retracted.sort();
    out.superseded.sort();
    out.added.sort();
    out
}

fn agents_at(store: &Store, uid: &Uid) -> Vec<AgentId> {
    store
        .get(uid)
        .map(|u| u.attestations.iter().map(|a| a.agent.clone()).collect())
        .unwrap_or_default()
}

/// Separate "the prompt changed" from "the content changed" (D-8).
///
/// A unit whose successor shares its `recipe_family` but not its `recipe` was re-run
/// against a different provider or model. A successor with a different family came from a
/// different prompt. Both look identical in the output; only the attestation tells them
/// apart.
fn recipe_changes(
    store: &Store,
    before: &BTreeSet<Uid>,
    after: &BTreeSet<Uid>,
) -> Vec<RecipeChange> {
    let families = |uid: &Uid| -> (BTreeSet<[u8; 32]>, BTreeSet<[u8; 32]>) {
        match store.get(uid) {
            Some(u) => (
                u.attestations.iter().filter_map(|a| a.recipe).collect(),
                u.attestations.iter().filter_map(|a| a.family).collect(),
            ),
            None => (BTreeSet::new(), BTreeSet::new()),
        }
    };

    let mut out = Vec::new();
    for rel in store.relations_of_kind(&RelKind::Supersedes) {
        if !before.contains(&rel.to) || !after.contains(&rel.from) {
            continue;
        }
        let (old_recipe, old_family) = families(&rel.to);
        let (new_recipe, new_family) = families(&rel.from);
        if old_family.is_empty() && new_family.is_empty() {
            continue;
        }
        let kind = if old_family == new_family && old_recipe != new_recipe {
            RecipeChangeKind::ProviderChanged
        } else if old_family != new_family {
            RecipeChangeKind::TemplateChanged
        } else {
            continue;
        };
        out.push(RecipeChange {
            uid: rel.from,
            kind,
        });
    }
    out.sort_by_key(|c| c.uid);
    out
}

/// Units reachable from a set of roots, in dense-id order - the membership of a view.
pub fn membership(store: &Store, roots: &BTreeSet<Uid>) -> Vec<Uid> {
    let g = store.adjacency();
    let ids: Vec<_> = roots.iter().filter_map(|u| g.id(u)).collect();
    traverse::closure(g, &ids, &EdgeSet::all())
        .into_iter()
        .filter_map(|n| g.uid(n))
        .copied()
        .filter(|u| store.contains_uid(u))
        .collect()
}

/// Whether an edge kind carries lineage rather than argument.
pub fn is_lineage_edge(k: EdgeKind) -> bool {
    matches!(k, EdgeKind::Deps | EdgeKind::Grounds)
        || k.rel_kind()
            .is_some_and(|r| matches!(r, RelKind::Supersedes | RelKind::Retracts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{
        canonical_uid, Attestation, Hlc, KernelType, Op, Record, Relation, Rung, SourceKind,
        SourceRef, Status, UnitCore, UnitCoreBuilder,
    };

    fn agent(s: &str) -> AgentId {
        AgentId::new(s).unwrap()
    }

    fn evidence(gist: &str) -> UnitCore {
        UnitCoreBuilder::new(KernelType::Evidence, gist, Status::Measured)
            .source(SourceRef::new(SourceKind::Metric, "m"))
            .build()
            .unwrap()
    }

    fn claim(gist: &str, grounds: Vec<Uid>) -> UnitCore {
        UnitCoreBuilder::new(KernelType::Claim, gist, Status::Inferred)
            .grounds(grounds)
            .build()
            .unwrap()
    }

    fn attest(uid: Uid, who: &str, hop: u32) -> Record {
        let a = agent(who);
        Record::Attestation(
            Attestation::new(
                uid,
                a.clone(),
                Op::Authored,
                Rung::Model,
                Hlc::new(hop as u64, 0, a),
            )
            .at_hop(hop),
        )
    }

    /// evidence <- claim <- finding, each attested one hop later.
    fn chain() -> (Store, Uid, Uid, Uid) {
        let e = evidence("a measurement");
        let ue = canonical_uid(&e);
        let c = claim("a claim", vec![ue]);
        let uc = canonical_uid(&c);
        let f = claim("a finding", vec![uc]);
        let uf = canonical_uid(&f);
        let store = Store::from_records(vec![
            Record::Unit(e),
            attest(ue, "human:v", 0),
            Record::Unit(c),
            attest(uc, "model:a/x", 1),
            Record::Unit(f),
            attest(uf, "model:b/y", 2),
        ]);
        (store, ue, uc, uf)
    }

    // --- trace ------------------------------------------------------------

    #[test]
    fn a_grounds_trace_reaches_the_evidence() {
        let (store, ue, uc, uf) = chain();
        let l = trace(&store, uf, TraceKind::Grounds, None);
        assert_eq!(l.len(), 3);
        assert_eq!(l.nodes[0].uid, uf);
        assert_eq!(l.nodes[0].depth, 0);
        assert_eq!(l.nodes[0].via, Via::Root);
        assert!(l.nodes.iter().any(|n| n.uid == uc && n.depth == 1));
        assert!(l.nodes.iter().any(|n| n.uid == ue && n.depth == 2));
        assert_eq!(l.max_depth(), 2);
    }

    /// F3, answered: every agent that had a hand in a unit's ancestry, named.
    #[test]
    fn a_trace_attributes_every_step() {
        let (store, _, _, uf) = chain();
        let l = trace(&store, uf, TraceKind::Grounds, None);
        let agents: BTreeSet<String> = l.agents().into_iter().map(ToString::to_string).collect();
        assert_eq!(agents.len(), 3);
        assert!(agents.contains("human:v"));
        assert!(agents.contains("model:a/x"));
        assert!(agents.contains("model:b/y"));
    }

    #[test]
    fn a_trace_reports_the_hop_each_unit_entered_at() {
        let (store, ue, _, uf) = chain();
        let l = trace(&store, uf, TraceKind::Grounds, None);
        assert_eq!(l.nodes.iter().find(|n| n.uid == uf).unwrap().hop, Some(2));
        assert_eq!(l.nodes.iter().find(|n| n.uid == ue).unwrap().hop, Some(0));
    }

    #[test]
    fn depth_bounds_the_walk() {
        let (store, ue, uc, uf) = chain();
        let l = trace(&store, uf, TraceKind::Grounds, Some(1));
        assert_eq!(l.len(), 2);
        assert!(l.nodes.iter().any(|n| n.uid == uc));
        assert!(!l.nodes.iter().any(|n| n.uid == ue));

        let zero = trace(&store, uf, TraceKind::Grounds, Some(0));
        assert_eq!(zero.len(), 1);
    }

    #[test]
    fn a_parents_trace_follows_attestation_ancestry() {
        let old = evidence("the first version");
        let uo = canonical_uid(&old);
        let new = evidence("the second version");
        let un = canonical_uid(&new);
        let a = agent("model:a/x");
        let store = Store::from_records(vec![
            Record::Unit(old),
            attest(uo, "human:v", 0),
            Record::Unit(new),
            Record::Attestation(
                Attestation::new(
                    un,
                    a.clone(),
                    Op::Transformed,
                    Rung::Model,
                    Hlc::new(1, 0, a),
                )
                .at_hop(1)
                .with_parents([uo].into_iter().collect()),
            ),
        ]);

        let l = trace(&store, un, TraceKind::Parents, None);
        assert_eq!(l.len(), 2);
        let parent = l.nodes.iter().find(|n| n.uid == uo).unwrap();
        assert_eq!(parent.via, Via::Parent);
        assert_eq!(parent.depth, 1);
    }

    #[test]
    fn a_parents_trace_follows_supersession() {
        let old = evidence("the first version");
        let uo = canonical_uid(&old);
        let new = evidence("the second version");
        let un = canonical_uid(&new);
        let store = Store::from_records(vec![
            Record::Unit(old),
            Record::Unit(new),
            Record::Relation(Relation::new(RelKind::Supersedes, un, uo)),
        ]);
        let l = trace(&store, un, TraceKind::Parents, None);
        assert_eq!(
            l.nodes.iter().find(|n| n.uid == uo).unwrap().via,
            Via::Supersedes
        );
    }

    /// A causal walk and an evidential walk answer different questions, so `grounds` must
    /// not wander into supersession and vice versa.
    #[test]
    fn the_two_walks_are_independent() {
        let (store, ue, _, uf) = chain();
        assert!(trace(&store, uf, TraceKind::Parents, None)
            .nodes
            .iter()
            .all(|n| n.uid != ue));
        assert_eq!(trace(&store, uf, TraceKind::Parents, None).len(), 1);
        assert_eq!(trace(&store, uf, TraceKind::Both, None).len(), 3);
    }

    #[test]
    fn a_trace_is_deterministic() {
        let (store, _, _, uf) = chain();
        assert_eq!(
            trace(&store, uf, TraceKind::Both, None),
            trace(&store, uf, TraceKind::Both, None)
        );
    }

    #[test]
    fn tracing_an_absent_unit_yields_only_the_root() {
        let (store, _, _, _) = chain();
        let l = trace(&store, Uid::from_bytes([9; 32]), TraceKind::Both, None);
        assert_eq!(l.len(), 1);
        assert!(l.nodes[0].agents.is_empty());
    }

    #[test]
    fn dependents_walks_the_other_way() {
        let (store, ue, uc, uf) = chain();
        let mut expected = vec![uc, uf];
        expected.sort();
        assert_eq!(dependents(&store, ue), expected);
        assert!(dependents(&store, uf).is_empty());
    }

    #[test]
    fn trace_kinds_round_trip() {
        for k in TraceKind::ALL {
            assert_eq!(TraceKind::parse(k.as_str()), Some(*k));
        }
        assert_eq!(TraceKind::parse("sideways"), None);
    }

    // --- diff -------------------------------------------------------------

    #[test]
    fn two_identical_stores_diff_to_nothing() {
        let (store, _, _, _) = chain();
        let d = diff(&store, &store);
        assert!(d.is_identical());
        assert_eq!(d.common.len(), 3);
    }

    #[test]
    fn diff_partitions_by_membership() {
        let a = evidence("only in a");
        let b = evidence("only in b");
        let both = evidence("in both");
        let (ua, ub, uboth) = (canonical_uid(&a), canonical_uid(&b), canonical_uid(&both));

        let left = Store::from_records(vec![Record::Unit(a), Record::Unit(both.clone())]);
        let right = Store::from_records(vec![Record::Unit(b), Record::Unit(both)]);
        let d = diff(&left, &right);
        assert_eq!(d.only_in_a, vec![ua]);
        assert_eq!(d.only_in_b, vec![ub]);
        assert_eq!(d.common, vec![uboth]);
        assert!(!d.is_identical());
    }

    // --- hop diff ---------------------------------------------------------

    #[test]
    fn a_hop_diff_reports_additions() {
        let (store, ue, uc, uf) = chain();
        let d = hop_diff(&store, 0, 2, false);
        assert_eq!(d.survived, vec![ue]);
        let mut added = vec![uc, uf];
        added.sort();
        assert_eq!(d.added, added);
        assert!(d.superseded.is_empty() && d.retracted.is_empty());
    }

    #[test]
    fn a_hop_diff_reports_supersession_with_its_successor() {
        let old = evidence("the first version");
        let uo = canonical_uid(&old);
        let new = evidence("the second version");
        let un = canonical_uid(&new);
        let store = Store::from_records(vec![
            Record::Unit(old),
            attest(uo, "human:v", 0),
            Record::Unit(new),
            attest(un, "model:a/x", 1),
            Record::Relation(Relation::new(RelKind::Supersedes, un, uo)),
        ]);

        let d = hop_diff(&store, 0, 1, false);
        assert_eq!(d.superseded, vec![(uo, un)]);
        assert_eq!(d.added, vec![un]);
        assert!(d.survived.is_empty());
        assert_eq!(d.survival_rate(), 0.0);
    }

    #[test]
    fn a_hop_diff_reports_retraction() {
        let e = evidence("a measurement");
        let ue = canonical_uid(&e);
        let store = Store::from_records(vec![
            Record::Unit(e),
            attest(ue, "human:v", 0),
            Record::Relation(Relation::new(RelKind::Retracts, ue, ue)),
        ]);
        let d = hop_diff(&store, 0, 1, false);
        assert_eq!(d.retracted, vec![ue]);
        assert!(d.survived.is_empty());
    }

    #[test]
    fn hop_activity_is_attributed_by_agent() {
        let (store, _, _, _) = chain();
        let d = hop_diff(&store, 0, 2, false);
        assert_eq!(d.by_agent.len(), 2, "two agents added something");
        assert_eq!(d.by_agent[&agent("model:a/x")].added, 1);
        assert_eq!(d.by_agent[&agent("model:b/y")].added, 1);
        assert_eq!(d.by_agent[&agent("model:a/x")].total(), 1);
    }

    /// A unit nobody attested cannot be placed in time, so it is left out rather than
    /// guessed at.
    #[test]
    fn units_without_attestations_are_excluded() {
        let e = evidence("unattested");
        let store = Store::from_records(vec![Record::Unit(e)]);
        let d = hop_diff(&store, 0, 5, false);
        assert_eq!(d.total(), 0);
        assert_eq!(d.survival_rate(), 1.0, "vacuously");
    }

    /// D-8: the same prompt against a different vendor, versus a different prompt.
    #[test]
    fn recipe_changes_separate_a_provider_swap_from_a_prompt_change() {
        let build = |old_family: [u8; 32], new_family: [u8; 32], new_recipe: [u8; 32]| {
            let old = evidence("the first version");
            let uo = canonical_uid(&old);
            let new = evidence("the second version");
            let un = canonical_uid(&new);
            let a = agent("model:a/x");
            let b = agent("model:b/y");
            Store::from_records(vec![
                Record::Unit(old),
                Record::Attestation(
                    Attestation::new(uo, a.clone(), Op::Authored, Rung::Model, Hlc::new(0, 0, a))
                        .at_hop(0)
                        .with_recipe([1; 32], old_family),
                ),
                Record::Unit(new),
                Record::Attestation(
                    Attestation::new(un, b.clone(), Op::Authored, Rung::Model, Hlc::new(1, 0, b))
                        .at_hop(1)
                        .with_recipe(new_recipe, new_family),
                ),
                Record::Relation(Relation::new(RelKind::Supersedes, un, uo)),
            ])
        };

        // Same family, different recipe: a different provider ran the same prompt.
        let provider = build([7; 32], [7; 32], [2; 32]);
        let d = hop_diff(&provider, 0, 1, true);
        assert_eq!(d.recipe_changes.len(), 1);
        assert_eq!(d.recipe_changes[0].kind, RecipeChangeKind::ProviderChanged);

        // Different family: the prompt itself moved.
        let template = build([7; 32], [8; 32], [2; 32]);
        let d = hop_diff(&template, 0, 1, true);
        assert_eq!(d.recipe_changes[0].kind, RecipeChangeKind::TemplateChanged);

        // Identical conditions: nothing to report.
        let same = build([7; 32], [7; 32], [1; 32]);
        assert!(hop_diff(&same, 0, 1, true).recipe_changes.is_empty());
    }

    #[test]
    fn recipe_changes_are_only_computed_when_asked_for() {
        let (store, _, _, _) = chain();
        assert!(hop_diff(&store, 0, 2, false).recipe_changes.is_empty());
    }

    #[test]
    fn survival_rate_measures_what_e2_measures() {
        let a = evidence("survives");
        let b = evidence("gets retracted");
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
        let store = Store::from_records(vec![
            Record::Unit(a),
            attest(ua, "human:v", 0),
            Record::Unit(b),
            attest(ub, "human:v", 0),
            Record::Relation(Relation::new(RelKind::Retracts, ub, ub)),
        ]);
        let d = hop_diff(&store, 0, 5, false);
        assert_eq!(d.survival_rate(), 0.5);
    }

    #[test]
    fn a_hop_diff_is_deterministic() {
        let (store, _, _, _) = chain();
        assert_eq!(hop_diff(&store, 0, 2, true), hop_diff(&store, 0, 2, true));
    }

    // --- membership -------------------------------------------------------

    #[test]
    fn membership_is_the_reachable_closure() {
        let (store, ue, uc, uf) = chain();
        let m = membership(&store, &[uf].into_iter().collect());
        assert_eq!(m.len(), 3);
        assert!(m.contains(&ue) && m.contains(&uc) && m.contains(&uf));

        let narrow = membership(&store, &[ue].into_iter().collect());
        assert_eq!(narrow, vec![ue]);
    }

    #[test]
    fn membership_of_no_roots_is_empty() {
        let (store, _, _, _) = chain();
        assert!(membership(&store, &BTreeSet::new()).is_empty());
    }

    #[test]
    fn lineage_edges_are_the_ones_that_carry_history() {
        assert!(is_lineage_edge(EdgeKind::Grounds));
        assert!(is_lineage_edge(EdgeKind::Deps));
        assert!(is_lineage_edge(
            EdgeKind::kernel(RelKind::Supersedes).unwrap()
        ));
        assert!(is_lineage_edge(
            EdgeKind::kernel(RelKind::Retracts).unwrap()
        ));
        assert!(!is_lineage_edge(EdgeKind::kernel(RelKind::Rebuts).unwrap()));
    }
}
