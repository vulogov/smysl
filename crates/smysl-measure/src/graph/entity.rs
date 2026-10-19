//! A9, the entity graph (SMYSL-2.6 §3.8): who a scope's units name, which of them are named
//! together, how central each is, which fall into groups, and how long its runs of events are.
//!
//! # The graph
//!
//! **Mentions.** A live `x.text/mentions` edge between a unit of the scope and an
//! `x.text/entity` unit, read in whichever direction puts the entity at one end — the schema
//! does not fix it, as `smysl_text::link` reads it. The entity need not be in the scope: it is
//! what the scope's units name, and a name is an entity whether or not a query selected it.
//! An entity's **degree** is how many of the scope's units mention it.
//!
//! **The entity graph** joins two entities that one unit of the scope mentions both of, with a
//! weight of how many units do. Its nodes are every entity mentioned at all, so one mentioned
//! alone is a node of its own.
//!
//! # What is reported
//!
//! - **degree** and **centrality** of the `top:k` entities (10), keyed by name.
//! - **Eigenvector centrality** by power iteration, 100 fixed steps, in `f64`, over `A + I`
//!   rather than `A`: the identity shifts every eigenvalue by one without moving an
//!   eigenvector, and it is what keeps the iteration from oscillating on a bipartite graph,
//!   where `A`'s largest and smallest eigenvalues are equal in magnitude. Each step is summed in
//!   uid order and scaled to sum 1, the result to a maximum of 1. Deterministic, as every IEEE
//!   double is under a fixed order of operations: no square root, no transcendental.
//! - **Communities** by synchronous label propagation. Every entity starts labelled with its
//!   own uid; in each round, all at once, each takes the label of greatest weight among its
//!   neighbours and **itself** (weight 1), ties to the smallest label. Counting itself is what
//!   stops two entities with one edge between them swapping labels for ever, which the plain
//!   synchronous rule does. At most 100 rounds; a graph that has not settled by then is flagged
//!   `lpa-unsettled` and reported as it stands. A community is named by its smallest uid.
//! - **Event chains**: the longest path in the DAG of the scope's `sequences` and
//!   `x.text/precedes` edges, both ends in scope, counted in units. A unit on a cycle — which the
//!   two kinds do not forbid — is in no chain, and is counted.
//! - **Betweenness** only when `params.betweenness` is true: Brandes, O(VE), on the entity
//!   graph unweighted, each pair counted once and normalised by `(n − 1)(n − 2)/2`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use smysl_core::diag::Code;
use smysl_core::schemas::text::{ENTITY, MENTIONS, PRECEDES};
use smysl_core::types::RelKind;
use smysl_core::Uid;
use smysl_graph::Store;

use crate::graph;
use crate::num::{self, q20};
use crate::recipe::Param;
use crate::{Input, Measure, Needs, Num, Outcome, Purity, Recipe, Refusal, Value};

/// A9.
#[derive(Debug, Clone, Copy, Default)]
pub struct A9;

/// Power-iteration steps and label-propagation rounds, both §3.8's.
pub const STEPS: usize = 100;

/// `params.top:k`'s default.
pub const TOP_K: i64 = 10;

/// The flag on a value whose communities had not settled after [`STEPS`] rounds.
pub const LPA_UNSETTLED: &str = "lpa-unsettled";

/// The entity graph of a scope: its entities, each one's degree, and the weighted co-mention
/// edges, both directions held.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntityGraph {
    /// Entity → how many of the scope's units mention it.
    pub degree: BTreeMap<Uid, u64>,
    /// Entity → neighbour → how many units mention both.
    pub edges: BTreeMap<Uid, BTreeMap<Uid, u64>>,
    /// Distinct (unit, entity) mentions.
    pub mentions: u64,
}

fn is_entity(store: &Store, uid: &Uid) -> bool {
    store
        .get(uid)
        .is_some_and(|u| u.core.schema.as_str() == ENTITY)
}

/// The entity graph of `uids`.
pub fn entity_graph(store: &Store, uids: &[Uid]) -> EntityGraph {
    let scope: BTreeSet<Uid> = uids.iter().copied().collect();
    let kind = RelKind::Extension(MENTIONS.to_string());
    let mut named: BTreeMap<Uid, BTreeSet<Uid>> = BTreeMap::new();
    for rel in store.relations_of_kind(&kind) {
        if store.is_withdrawn(rel) {
            continue;
        }
        for (unit, entity) in [(rel.from, rel.to), (rel.to, rel.from)] {
            if scope.contains(&unit) && !is_entity(store, &unit) && is_entity(store, &entity) {
                named.entry(unit).or_default().insert(entity);
            }
        }
    }
    let mut g = EntityGraph::default();
    for entities in named.values() {
        for e in entities {
            *g.degree.entry(*e).or_default() += 1;
            g.edges.entry(*e).or_default();
            g.mentions += 1;
            for f in entities {
                if f != e {
                    *g.edges.entry(*e).or_default().entry(*f).or_default() += 1;
                }
            }
        }
    }
    g
}

/// Eigenvector centrality over `A + I`, [`STEPS`] steps, scaled to a maximum of 1.
pub fn centrality(g: &EntityGraph) -> BTreeMap<Uid, f64> {
    let n = g.edges.len();
    if n == 0 {
        return BTreeMap::new();
    }
    let mut x: BTreeMap<Uid, f64> = g.edges.keys().map(|u| (*u, 1.0 / n as f64)).collect();
    for _ in 0..STEPS {
        let next: BTreeMap<Uid, f64> = g
            .edges
            .iter()
            .map(|(u, nbrs)| {
                let s = num::sum(
                    std::iter::once(x[u]).chain(nbrs.iter().map(|(v, w)| *w as f64 * x[v])),
                );
                (*u, s)
            })
            .collect();
        let total = num::sum(next.values().copied());
        x = next.into_iter().map(|(u, s)| (u, s / total)).collect();
    }
    let max = x.values().copied().fold(0.0, f64::max);
    x.into_iter().map(|(u, s)| (u, s / max)).collect()
}

/// Communities by synchronous label propagation: each entity's label, and whether the rounds
/// settled.
pub fn communities(g: &EntityGraph) -> (BTreeMap<Uid, Uid>, bool) {
    let mut label: BTreeMap<Uid, Uid> = g.edges.keys().map(|u| (*u, *u)).collect();
    for _ in 0..STEPS {
        let next: BTreeMap<Uid, Uid> = g
            .edges
            .iter()
            .map(|(u, nbrs)| {
                let mut weight: BTreeMap<Uid, u64> = BTreeMap::new();
                *weight.entry(label[u]).or_default() += 1;
                for (v, w) in nbrs {
                    *weight.entry(label[v]).or_default() += w;
                }
                // Greatest weight; the BTreeMap's order makes the first of equals the smallest.
                let best =
                    weight.iter().fold(
                        (label[u], 0),
                        |acc, (l, w)| if *w > acc.1 { (*l, *w) } else { acc },
                    );
                (*u, best.0)
            })
            .collect();
        if next == label {
            return (label, true);
        }
        label = next;
    }
    (label, false)
}

/// Betweenness by Brandes over the entity graph unweighted, normalised to `[0, 1]`.
pub fn betweenness(g: &EntityGraph) -> BTreeMap<Uid, f64> {
    let ids: Vec<Uid> = g.edges.keys().copied().collect();
    let at: BTreeMap<Uid, usize> = ids.iter().enumerate().map(|(i, u)| (*u, i)).collect();
    let adj: Vec<Vec<usize>> = ids
        .iter()
        .map(|u| g.edges[u].keys().map(|v| at[v]).collect())
        .collect();
    let n = ids.len();
    let mut cb = vec![0.0f64; n];
    for s in 0..n {
        let mut stack = Vec::with_capacity(n);
        let mut pred: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut sigma = vec![0.0f64; n];
        let mut dist = vec![-1i64; n];
        sigma[s] = 1.0;
        dist[s] = 0;
        let mut q = VecDeque::from([s]);
        while let Some(v) = q.pop_front() {
            stack.push(v);
            for &w in &adj[v] {
                if dist[w] < 0 {
                    dist[w] = dist[v] + 1;
                    q.push_back(w);
                }
                if dist[w] == dist[v] + 1 {
                    sigma[w] += sigma[v];
                    pred[w].push(v);
                }
            }
        }
        let mut delta = vec![0.0f64; n];
        while let Some(w) = stack.pop() {
            for &v in &pred[w] {
                delta[v] += sigma[v] / sigma[w] * (1.0 + delta[w]);
            }
            if w != s {
                cb[w] += delta[w];
            }
        }
    }
    // Each unordered pair was counted from both ends.
    let norm = if n > 2 {
        ((n - 1) * (n - 2)) as f64
    } else {
        1.0
    };
    ids.into_iter()
        .zip(cb)
        .map(|(u, c)| (u, if n > 2 { c / norm } else { 0.0 }))
        .collect()
}

/// `(longest, chained, cyclic)` over `uids`: the longest chain of `sequences` and
/// `x.text/precedes` edges, in units; how many units are on such an edge; and how many of
/// those are on a cycle, or after one, and so in no chain. All 0 with no edges.
pub fn chains(store: &Store, uids: &[Uid]) -> (u64, u64, u64) {
    let scope: BTreeSet<Uid> = uids.iter().copied().collect();
    let mut out: BTreeMap<Uid, BTreeSet<Uid>> = BTreeMap::new();
    let mut indeg: BTreeMap<Uid, u64> = BTreeMap::new();
    let precedes = RelKind::Extension(PRECEDES.to_string());
    for kind in [RelKind::Sequences, precedes] {
        for rel in store.relations_of_kind(&kind) {
            if store.is_withdrawn(rel)
                || rel.from == rel.to
                || !scope.contains(&rel.from)
                || !scope.contains(&rel.to)
            {
                continue;
            }
            if out.entry(rel.from).or_default().insert(rel.to) {
                *indeg.entry(rel.to).or_default() += 1;
            }
            indeg.entry(rel.from).or_default();
            out.entry(rel.to).or_default();
        }
    }
    let on_edges = out.len() as u64;
    // Kahn's order, smallest uid first; `len[u]` is the longest chain ending at u.
    let mut ready: BTreeSet<Uid> = indeg
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(u, _)| *u)
        .collect();
    let mut len: BTreeMap<Uid, u64> = ready.iter().map(|u| (*u, 1)).collect();
    let mut done = 0u64;
    let mut longest = 0;
    while let Some(u) = ready.pop_first() {
        done += 1;
        let l = len[&u];
        longest = longest.max(l);
        for v in &out[&u] {
            let e = len.entry(*v).or_default();
            *e = (*e).max(l + 1);
            let d = indeg.get_mut(v).expect("every edge's end has a degree");
            *d -= 1;
            if *d == 0 {
                ready.insert(*v);
            }
        }
    }
    (longest, on_edges, on_edges - done)
}

fn top_k(recipe: &Recipe) -> Result<usize, Refusal> {
    match recipe.params.get("top:k") {
        Some(Param::Int(k)) if *k >= 0 => Ok(*k as usize),
        _ => Err(Refusal::new(
            Code::E489,
            "A9's `top:k` is a non-negative integer",
        )),
    }
}

fn wants_betweenness(recipe: &Recipe) -> Result<bool, Refusal> {
    match recipe.params.get("betweenness") {
        Some(Param::Bool(b)) => Ok(*b),
        _ => Err(Refusal::new(
            Code::E489,
            "A9's `betweenness` is true or false",
        )),
    }
}

/// Each entity's name for a value key: its gist, and the gist with the start of its uid where
/// two entities share one.
fn names(store: &Store, entities: impl Iterator<Item = Uid>) -> BTreeMap<Uid, String> {
    let gist = |u: &Uid| {
        store
            .get(u)
            .map_or_else(String::new, |x| x.core.gist.clone())
    };
    let all: Vec<Uid> = entities.collect();
    let mut count: BTreeMap<String, u32> = BTreeMap::new();
    for u in &all {
        *count.entry(gist(u)).or_default() += 1;
    }
    all.into_iter()
        .map(|u| {
            let g = gist(&u);
            let name = if count[&g] > 1 {
                let c = u.canonical();
                format!("{g} {}", &c[..c.len().min(12)])
            } else {
                g
            };
            (u, name)
        })
        .collect()
}

impl Measure for A9 {
    fn id(&self) -> &'static str {
        "A9"
    }

    fn version(&self) -> u16 {
        1
    }

    fn purity(&self) -> Purity {
        Purity::Pure
    }

    fn needs(&self) -> Needs {
        Needs::UNITS
    }

    fn complete(&self, recipe: &mut Recipe) {
        graph::complete(recipe);
        recipe
            .params
            .entry("top:k".to_string())
            .or_insert(Param::Int(TOP_K));
        recipe
            .params
            .entry("betweenness".to_string())
            .or_insert(Param::Bool(false));
    }

    fn admit(&self, recipe: &Recipe, input: &Input<'_>) -> Result<(), Refusal> {
        top_k(recipe)?;
        wants_betweenness(recipe)?;
        graph::admitted(recipe, input).map(|_| ())
    }

    fn run(&self, recipe: &Recipe, input: &Input<'_>) -> Outcome {
        let run = || -> Result<Value, Refusal> {
            let k = top_k(recipe)?;
            let between = wants_betweenness(recipe)?;
            let uids = graph::admitted(recipe, input)?;
            let store = input.store;
            let g = entity_graph(store, &uids);
            let fixed = |x: f64| {
                q20(x)
                    .ok_or_else(|| Refusal::new(Code::E489, "a centrality was not a finite number"))
            };
            let (label, settled) = communities(&g);
            let mut sizes: BTreeMap<Uid, u64> = BTreeMap::new();
            for l in label.values() {
                *sizes.entry(*l).or_default() += 1;
            }
            let mut by_size: Vec<u64> = sizes.values().copied().collect();
            by_size.sort_unstable_by(|a, b| b.cmp(a));
            let (longest, chained, cyclic) = chains(store, &uids);
            let entities = g.edges.len() as u64;
            let pairs: u64 = g.edges.values().map(|n| n.len() as u64).sum::<u64>() / 2;

            let mut v = Value::new()
                .with_n("units", uids.len() as u64)
                .with_n("entities", entities)
                .with_n("mentions", g.mentions)
                .with_n("pairs", pairs)
                .with_n("chained", chained)
                .with_n("cyclic", cyclic)
                .with_value("communities", Num::Count(sizes.len() as u64))
                .with_value("chain:max", Num::Count(longest))
                .with_series("sizes", by_size.iter().map(|s| Num::Count(*s)).collect())
                .with_headline_in("communities", "entity communities", "");
            if let Some(largest) = by_size.first() {
                v = v.with_value("community:largest", Num::Ratio(*largest, entities));
            }
            if !settled {
                v = v.with_flag(LPA_UNSETTLED);
            }

            let name = names(store, g.edges.keys().copied());
            let mut by_degree: Vec<(&Uid, &u64)> = g.degree.iter().collect();
            by_degree.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            for (u, d) in by_degree.into_iter().take(k) {
                v = v.with_value(format!("degree:{}", name[u]), Num::Count(*d));
            }
            let mut ranked: Vec<(Uid, i64)> = Vec::new();
            for (u, c) in centrality(&g) {
                ranked.push((u, fixed(c)?));
            }
            ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            for (u, q) in ranked.into_iter().take(k) {
                v = v.with_value(format!("centrality:{}", name[&u]), Num::Fixed(q));
            }
            if between {
                let mut ranked: Vec<(Uid, i64)> = Vec::new();
                for (u, c) in betweenness(&g) {
                    ranked.push((u, fixed(c)?));
                }
                ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                for (u, q) in ranked.into_iter().take(k) {
                    v = v.with_value(format!("betweenness:{}", name[&u]), Num::Fixed(q));
                }
            }
            Ok(v)
        };
        match run() {
            Ok(v) => Outcome::Value(v),
            Err(r) => Outcome::refused(r),
        }
    }
}
