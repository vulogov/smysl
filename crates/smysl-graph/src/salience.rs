//! Derived salience (§1.5, §16.4).
//!
//! ```text
//! raw(u) = w_c·centrality(u) + w_r·corroboration(u) + w_t·role_weight(u, T)
//! ```
//!
//! This is what makes summarisation precomputation (P4): packing asks the graph which
//! units matter rather than asking a model, so fitting content to a budget costs zero
//! inference. An authored `salience` overrides all of it - an author who says what matters
//! is not second-guessed.
//!
//! **Direction.** Rank flows from dependent to dependency - *along* `grounds` and `deps*,
//! which is the opposite of the direction support flows in. A unit that many conclusions
//! rest upon accumulates rank. Getting this backwards would rank conclusions above their
//! evidence, which is precisely the wrong answer for a budget.
//!
//! **Determinism.** Accumulation is `f64` and quantisation to 1/1024 happens once, at the
//! end. That removes residual platform float differences while keeping the intermediate
//! arithmetic well-conditioned - the thing that lets the same store produce the same bytes
//! on any machine (rule D).

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::{quantise, Uid};

use crate::adjacency::{EdgeSet, NodeId};
use crate::store::Store;

/// Damping factor. Fixed rather than configurable: it is hash input for every pack.
pub const DAMPING: f64 = 0.85;
/// Exactly this many iterations, never "until convergence" - a tolerance test would make
/// the result depend on float behaviour.
pub const ITERATIONS: usize = 32;
/// Corroboration saturates here: a fifth independent group tells you nothing the fourth
/// did not.
pub const CORROBORATION_CAP: usize = 4;

/// The terms' weights (§1.5), plus recency.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SalienceWeights {
    pub centrality: f32,
    pub corroboration: f32,
    pub role: f32,
    /// How much a unit's *hop distance* counts. Zero by default - see [`SalienceWeights`]'s
    /// note on why turning it on is a decision rather than a default.
    pub recency: f32,
}

impl Default for SalienceWeights {
    fn default() -> SalienceWeights {
        SalienceWeights {
            centrality: 0.5,
            corroboration: 0.2,
            role: 0.3,
            // **Off by default, deliberately.** Salience feeds `pack`, so a non-zero
            // default would change what every existing store carries forward - silently,
            // and for stores whose owners never asked for recency. It is one field to turn
            // on, and `recent()` is the ready-made setting.
            recency: 0.0,
        }
    }
}

impl SalienceWeights {
    /// What a low-trust deployment uses.
    ///
    /// Corroboration is gameable by fabricated attestations until COSE signing lands, so
    /// `w_r = 0` is the honest setting where agent identity cannot be trusted (D-6, §29).
    pub fn untrusted() -> SalienceWeights {
        SalienceWeights {
            corroboration: 0.0,
            ..SalienceWeights::default()
        }
    }

    /// What a long-running pipeline uses.
    ///
    /// Structural salience alone is backwards for an agent that runs for weeks: a
    /// well-connected claim from a month ago outranks a fresh critical one permanently,
    /// because centrality only grows. Recency is what lets the graph forget.
    pub fn recent() -> SalienceWeights {
        SalienceWeights {
            centrality: 0.4,
            corroboration: 0.15,
            role: 0.25,
            recency: 0.2,
        }
    }

    pub fn sum(&self) -> f32 {
        self.centrality + self.corroboration + self.role + self.recency
    }
}

/// How much a unit produced `distance` hops ago still counts: halving each hop.
///
/// **Measured in hops rather than wall-clock time**, which is what makes it usable here at
/// all. A clock read inside `salience` would make it non-reproducible and break rule D; a
/// hop count is already in the record, is already deterministic, and is what "how many
/// handoffs ago" actually means in a pipeline. Wall-clock decay would be a different
/// feature needing a timestamp passed in.
pub fn recency_at(distance: u32) -> f32 {
    // 1.0 at the current hop, 0.5 one back, 0.25 two back. Saturating, so a very old unit
    // reaches zero rather than wrapping.
    match distance {
        0 => 1.0,
        d if d >= 24 => 0.0,
        d => 1.0 / (1u32 << d) as f32,
    }
}

/// What to compute salience against.
#[derive(Debug, Clone, Default)]
pub struct SalienceRequest {
    pub weights: SalienceWeights,
    /// The personalisation vector: the `question` and `finding` units of the active
    /// thread, or a view's roots. Empty means uniform - plain PageRank.
    pub seed: BTreeSet<Uid>,
    /// Per-unit role weight, supplied by whoever knows the active thread. Salience lives
    /// below threads in the crate graph, so it takes this rather than deriving it.
    pub role_weights: BTreeMap<Uid, f32>,
    /// The hop to measure recency against - normally the step about to run.
    ///
    /// Supplied rather than read off the store, for the same reason every clock in this
    /// codebase is supplied: a caller replaying a pipeline has to be able to ask what
    /// salience looked like *at hop 4*, not what it looks like now. `None` leaves the
    /// recency term at zero however it is weighted.
    pub now_hop: Option<u32>,
}

impl SalienceRequest {
    pub fn with_weights(mut self, w: SalienceWeights) -> SalienceRequest {
        self.weights = w;
        self
    }

    pub fn seeded(mut self, seed: impl IntoIterator<Item = Uid>) -> SalienceRequest {
        self.seed = seed.into_iter().collect();
        self
    }

    /// Measure recency against this hop.
    pub fn at_hop(mut self, hop: u32) -> SalienceRequest {
        self.now_hop = Some(hop);
        self
    }

    pub fn with_role_weights(mut self, r: BTreeMap<Uid, f32>) -> SalienceRequest {
        self.role_weights = r;
        self
    }
}

/// Why a unit scored what it did - the output of `salience --explain`.
#[derive(Debug, Clone, PartialEq)]
pub struct SalienceTerms {
    pub centrality: f32,
    pub corroboration: f32,
    /// The recency factor applied, before weighting. Zero when the unit has no episode or
    /// the request named no hop.
    pub recency: f32,
    /// The corroboration groups counted, by key, in canonical order.
    pub groups: Vec<String>,
    /// Groups found but discarded for sharing ancestry with a group already counted.
    pub dependent_groups: Vec<String>,
    pub role: f32,
    pub raw: f32,
    /// True when an authored override replaced the derived value entirely.
    pub authored: bool,
}

/// Salience for every unit in a store.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SalienceReport {
    pub scores: BTreeMap<Uid, f32>,
    pub terms: BTreeMap<Uid, SalienceTerms>,
}

impl SalienceReport {
    pub fn get(&self, uid: &Uid) -> f32 {
        self.scores.get(uid).copied().unwrap_or(0.0)
    }

    pub fn explain(&self, uid: &Uid) -> Option<&SalienceTerms> {
        self.terms.get(uid)
    }

    /// The highest-scoring units, ties broken by ascending uid.
    pub fn top(&self, n: usize) -> Vec<(Uid, f32)> {
        let mut v: Vec<(Uid, f32)> = self.scores.iter().map(|(u, s)| (*u, *s)).collect();
        v.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        v.truncate(n);
        v
    }

    /// Rescale so the highest-scoring unit *within a selection* reaches 1.0.
    ///
    /// Salience MUST be renormalised per thread at pack time (§1.5): a unit that is
    /// middling across a whole corpus may be the most important thing in the thread being
    /// packed, and a budget should spend on it accordingly.
    pub fn renormalise(&self, within: &[Uid]) -> BTreeMap<Uid, f32> {
        let max = within
            .iter()
            .map(|u| self.get(u))
            .fold(0.0f32, |a, b| a.max(b));
        within
            .iter()
            .map(|u| {
                let s = self.get(u);
                (*u, if max > 0.0 { quantise(s / max) } else { 0.0 })
            })
            .collect()
    }
}

/// Compute salience (§16.4).
pub fn salience(store: &Store, req: &SalienceRequest) -> SalienceReport {
    let g = store.adjacency();
    let n = g.len();
    let mut out = SalienceReport::default();
    if n == 0 {
        return out;
    }

    let seed: Vec<NodeId> = if req.seed.is_empty() {
        // No thread and no roots: personalise uniformly, which is plain PageRank.
        (0..n as NodeId).filter(|&i| g.is_present(i)).collect()
    } else {
        req.seed.iter().filter_map(|u| g.id(u)).collect()
    };

    let centrality = personalised_pagerank(store, &seed);

    for (uid, unit) in store.units() {
        let c = g.id(uid).map(|i| centrality[i as usize]).unwrap_or(0.0);
        let (corr, groups, dependent) = corroboration(store, uid);
        let role = req.role_weights.get(uid).copied().unwrap_or(0.0);

        // A unit with no attestation has no episode, so it has no recency either - it is
        // not "old", it is unplaced, and inventing a distance for it would rank it.
        let recency = match (req.now_hop, store.hop_of(uid)) {
            (Some(now), Some(hop)) => recency_at(now.saturating_sub(hop)),
            _ => 0.0,
        };

        let raw = req.weights.centrality * c as f32
            + req.weights.corroboration * corr
            + req.weights.role * role
            + req.weights.recency * recency;

        // An authored value is authoritative (§1.5).
        let (score, authored) = match unit.salience {
            Some(s) => (s, true),
            None => (quantise(raw.clamp(0.0, 1.0)), false),
        };

        out.scores.insert(*uid, score);
        out.terms.insert(
            *uid,
            SalienceTerms {
                centrality: quantise(c as f32),
                corroboration: quantise(corr),
                recency: quantise(recency),
                groups,
                dependent_groups: dependent,
                role: quantise(role),
                raw: quantise(raw.clamp(0.0, 1.0)),
                authored,
            },
        );
    }
    out
}

/// Personalised PageRank over the support graph (§16.4).
///
/// Returns a value per dense id, normalised so the highest is 1.0.
fn personalised_pagerank(store: &Store, seed: &[NodeId]) -> Vec<f64> {
    let g = store.adjacency();
    let n = g.len();
    let kinds = EdgeSet::support_rank();

    let mut s0 = vec![0.0f64; n];
    if !seed.is_empty() {
        let share = 1.0 / seed.len() as f64;
        for &i in seed {
            if (i as usize) < n {
                s0[i as usize] = share;
            }
        }
    }

    // Precompute the out-neighbourhoods once. They are already in dense-id order, so the
    // walk visits them the same way every time.
    let out: Vec<Vec<NodeId>> = (0..n as NodeId).map(|i| g.out(i, &kinds)).collect();

    let mut r = s0.clone();
    for _ in 0..ITERATIONS {
        let mut next = vec![0.0f64; n];
        let mut dangling = 0.0f64;
        // Ascending dense id: the accumulation order is fixed, so float addition
        // associativity cannot change the answer between runs.
        for i in 0..n {
            if out[i].is_empty() {
                dangling += r[i];
                continue;
            }
            let share = r[i] / out[i].len() as f64;
            for &j in &out[i] {
                next[j as usize] += share;
            }
        }
        for i in 0..n {
            // Dangling mass goes back to the *seed*, not uniformly. Spreading it evenly
            // would wash out the personalisation the seed exists to provide.
            r[i] = (1.0 - DAMPING) * s0[i] + DAMPING * (next[i] + dangling * s0[i]);
        }
    }

    let max = r.iter().copied().fold(0.0f64, f64::max);
    if max > 0.0 {
        for v in &mut r {
            *v /= max;
        }
    }
    r
}

/// Independent corroboration, and which groups were counted (§16.4, D-8).
///
/// Attestations group by `(provider, model, recipe)`, because two units produced by the
/// same model under the same recipe are not independent evidence - counting them twice
/// would let one agent corroborate itself. Groups that share ancestry are collapsed for
/// the same reason: two agents that both worked from the same parent agree because they
/// read the same thing, not because they checked.
fn corroboration(store: &Store, uid: &Uid) -> (f32, Vec<String>, Vec<String>) {
    let Some(unit) = store.get(uid) else {
        return (0.0, Vec::new(), Vec::new());
    };

    // Group by key, collecting each group's ancestry.
    let mut groups: BTreeMap<String, BTreeSet<Uid>> = BTreeMap::new();
    for a in &unit.attestations {
        let (agent, recipe) = a.corroboration_key();
        let key = match recipe {
            Some(r) => format!("{agent}#{}", Uid::from_bytes(r).short()),
            None => agent,
        };
        groups
            .entry(key)
            .or_default()
            .extend(a.parents.iter().copied());
    }

    // Keep groups whose ancestry is disjoint from everything kept so far. Canonical key
    // order, so which group wins a collision is a function of the graph.
    let mut counted = Vec::new();
    let mut dependent = Vec::new();
    let mut claimed: BTreeSet<Uid> = BTreeSet::new();
    for (key, ancestry) in groups {
        if ancestry.is_empty() || ancestry.is_disjoint(&claimed) {
            claimed.extend(ancestry.iter().copied());
            counted.push(key);
        } else {
            dependent.push(key);
        }
    }

    let score = counted.len().min(CORROBORATION_CAP) as f32 / CORROBORATION_CAP as f32;
    (score, counted, dependent)
}

/// The seed set for a store with no active thread: every root of every view (§16.4).
pub fn view_roots(store: &Store) -> BTreeSet<Uid> {
    store
        .views()
        .flat_map(|v| v.roots.iter().copied())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{
        canonical_uid, AgentId, Attestation, Hlc, KernelType, Op, Record, RelKind, Relation, Rung,
        SourceKind, SourceRef, Status, Unit, UnitCore, UnitCoreBuilder,
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

    /// One measurement, two claims resting on it, one finding resting on both.
    fn diamond() -> (Store, Uid, Uid, Uid, Uid) {
        let e = evidence("the shared measurement");
        let ue = canonical_uid(&e);
        let a = claim("claim a", vec![ue]);
        let ua = canonical_uid(&a);
        let b = claim("claim b", vec![ue]);
        let ub = canonical_uid(&b);
        let f = claim("the finding", vec![ua, ub]);
        let uf = canonical_uid(&f);
        (
            Store::from_records(vec![
                Record::Unit(e),
                Record::Unit(a),
                Record::Unit(b),
                Record::Unit(f),
            ]),
            ue,
            ua,
            ub,
            uf,
        )
    }

    fn plain(store: &Store) -> SalienceReport {
        salience(store, &SalienceRequest::default())
    }

    // --- direction --------------------------------------------------------

    /// The property everything else rests on: evidence many conclusions depend on
    /// outranks the conclusions. Reversed, a budget would spend on conclusions and drop
    /// the thing they stand on.
    #[test]
    fn evidence_outranks_what_rests_on_it() {
        let (store, ue, ua, _, uf) = diamond();
        let r = salience(&store, &SalienceRequest::default().seeded([uf]));
        assert!(
            r.terms[&ue].centrality > r.terms[&ua].centrality,
            "evidence {} did not outrank the claim {}",
            r.terms[&ue].centrality,
            r.terms[&ua].centrality
        );
        assert!(r.terms[&ua].centrality > 0.0);
    }

    #[test]
    fn a_unit_nothing_rests_on_scores_lowest() {
        let e = evidence("referenced by nobody");
        let ue = canonical_uid(&e);
        let (mut store, shared, _, _, uf) = diamond();
        store.append(&[Record::Unit(e)]).unwrap();

        let r = salience(&store, &SalienceRequest::default().seeded([uf]));
        assert!(r.terms[&shared].centrality > r.terms[&ue].centrality);
    }

    // --- the reference implementation --------------------------------------

    /// A second, deliberately naive implementation of §16.4's pseudocode. Written to be
    /// obviously correct rather than efficient, so a disagreement points at the fast path.
    fn reference_pagerank(store: &Store, seed: &[Uid]) -> BTreeMap<Uid, f64> {
        let g = store.adjacency();
        let n = g.len();
        let ids: Vec<NodeId> = (0..n as NodeId).collect();

        // Dense adjacency matrix over the support edges.
        let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
        for &i in &ids {
            for j in g.out(i, &EdgeSet::support_rank()) {
                edges[i as usize].push(j as usize);
            }
        }

        let seed_ids: Vec<usize> = seed
            .iter()
            .filter_map(|u| g.id(u))
            .map(|i| i as usize)
            .collect();
        let mut s0 = vec![0.0f64; n];
        for &i in &seed_ids {
            s0[i] = 1.0 / seed_ids.len() as f64;
        }

        let mut r = s0.clone();
        for _ in 0..ITERATIONS {
            let mut next = vec![0.0f64; n];
            let mut dangling = 0.0;
            for i in 0..n {
                if edges[i].is_empty() {
                    dangling += r[i];
                } else {
                    for &j in &edges[i] {
                        next[j] += r[i] / edges[i].len() as f64;
                    }
                }
            }
            let mut updated = vec![0.0f64; n];
            for (i, u) in updated.iter_mut().enumerate() {
                *u = (1.0 - DAMPING) * s0[i] + DAMPING * (next[i] + dangling * s0[i]);
            }
            r = updated;
        }

        let max = r.iter().copied().fold(0.0f64, f64::max);
        (0..n as NodeId)
            .filter_map(|i| {
                g.uid(i)
                    .map(|u| (*u, if max > 0.0 { r[i as usize] / max } else { 0.0 }))
            })
            .collect()
    }

    /// The gate: the production path agrees with an independently written reference to
    /// within the quantum.
    #[test]
    fn pagerank_matches_an_independent_reference() {
        let (store, ue, ua, ub, uf) = diamond();
        let seed = [uf];
        let expected = reference_pagerank(&store, &seed);
        let got = salience(&store, &SalienceRequest::default().seeded(seed));

        for uid in [ue, ua, ub, uf] {
            let a = got.terms[&uid].centrality as f64;
            let b = expected[&uid];
            assert!((a - b).abs() <= 1.0 / 1024.0, "{uid}: {a} vs reference {b}");
        }
    }

    #[test]
    fn pagerank_matches_the_reference_on_a_chain() {
        let e = evidence("the root");
        let ue = canonical_uid(&e);
        let a = claim("first", vec![ue]);
        let ua = canonical_uid(&a);
        let b = claim("second", vec![ua]);
        let ub = canonical_uid(&b);
        let store = Store::from_records(vec![Record::Unit(e), Record::Unit(a), Record::Unit(b)]);

        let expected = reference_pagerank(&store, &[ub]);
        let got = salience(&store, &SalienceRequest::default().seeded([ub]));
        for uid in [ue, ua, ub] {
            assert!((got.terms[&uid].centrality as f64 - expected[&uid]).abs() <= 1.0 / 1024.0);
        }
    }

    #[test]
    fn pagerank_matches_the_reference_with_a_uniform_seed() {
        let (store, ue, ua, ub, uf) = diamond();
        let all: Vec<Uid> = vec![ue, ua, ub, uf];
        let expected = reference_pagerank(&store, &all);
        let got = plain(&store);
        for uid in all {
            assert!((got.terms[&uid].centrality as f64 - expected[&uid]).abs() <= 1.0 / 1024.0);
        }
    }

    // --- personalisation ---------------------------------------------------

    /// The seed is what makes this *personalised*. Two different questions should not get
    /// the same ranking out of the same corpus.
    #[test]
    fn the_seed_changes_the_ranking() {
        let shared = evidence("shared");
        let us = canonical_uid(&shared);
        let only_a = evidence("only a needs this");
        let uo = canonical_uid(&only_a);
        let a = claim("branch a", vec![us, uo]);
        let ua = canonical_uid(&a);
        let b = claim("branch b", vec![us]);
        let ub = canonical_uid(&b);
        let store = Store::from_records(vec![
            Record::Unit(shared),
            Record::Unit(only_a),
            Record::Unit(a),
            Record::Unit(b),
        ]);

        let from_a = salience(&store, &SalienceRequest::default().seeded([ua]));
        let from_b = salience(&store, &SalienceRequest::default().seeded([ub]));
        assert!(
            from_a.terms[&uo].centrality > from_b.terms[&uo].centrality,
            "a branch-specific unit should matter more to the branch that needs it"
        );
    }

    /// Dangling mass returns to the seed. Spreading it uniformly would wash out exactly
    /// the personalisation the seed exists to provide.
    #[test]
    fn dangling_mass_returns_to_the_seed() {
        let orphan = evidence("nothing points at this, and it points at nothing");
        let uo = canonical_uid(&orphan);
        let (mut store, ue, _, _, uf) = diamond();
        store.append(&[Record::Unit(orphan)]).unwrap();

        let r = salience(&store, &SalienceRequest::default().seeded([uf]));
        assert_eq!(
            r.terms[&uo].centrality, 0.0,
            "an unreachable unit gains nothing"
        );
        assert!(r.terms[&ue].centrality > 0.0);
    }

    // --- corroboration -----------------------------------------------------

    fn attested(core: UnitCore, agents: &[(&str, Option<[u8; 32]>)]) -> Vec<Record> {
        let uid = canonical_uid(&core);
        let mut out = vec![Record::Unit(core)];
        for (who, recipe) in agents {
            let a = agent(who);
            let mut att = Attestation::new(uid, a.clone(), Op::Authored, Rung::Model, Hlc::zero(a));
            if let Some(r) = recipe {
                att = att.with_recipe(*r, *r);
            }
            out.push(Record::Attestation(att));
        }
        out
    }

    #[test]
    fn corroboration_counts_distinct_groups() {
        let store = Store::from_records(attested(
            evidence("a measurement"),
            &[("model:a/x", None), ("model:b/y", None)],
        ));
        let uid = canonical_uid(&evidence("a measurement"));
        let r = plain(&store);
        assert_eq!(r.terms[&uid].groups.len(), 2);
        assert_eq!(r.terms[&uid].corroboration, 0.5, "two of four");
    }

    /// One model cannot corroborate itself, however many times it says the same thing.
    #[test]
    fn the_same_agent_under_the_same_recipe_is_one_group() {
        let store = Store::from_records(attested(
            evidence("a measurement"),
            &[
                ("model:a/x", Some([1; 32])),
                ("model:a/x", Some([1; 32])),
                ("model:a/x", Some([1; 32])),
            ],
        ));
        let uid = canonical_uid(&evidence("a measurement"));
        let r = plain(&store);
        assert_eq!(r.terms[&uid].groups.len(), 1);
        assert_eq!(r.terms[&uid].corroboration, 0.25);
    }

    #[test]
    fn the_same_agent_under_a_different_recipe_is_a_different_group() {
        let store = Store::from_records(attested(
            evidence("a measurement"),
            &[("model:a/x", Some([1; 32])), ("model:a/x", Some([2; 32]))],
        ));
        let uid = canonical_uid(&evidence("a measurement"));
        assert_eq!(plain(&store).terms[&uid].groups.len(), 2);
    }

    /// Two agents that both worked from the same parent agree because they read the same
    /// thing, not because they checked.
    #[test]
    fn groups_sharing_ancestry_are_not_independent() {
        let parent = evidence("what they both read");
        let up = canonical_uid(&parent);
        let core = evidence("the corroborated claim");
        let uid = canonical_uid(&core);

        let att = |who: &str| {
            let a = agent(who);
            Record::Attestation(
                Attestation::new(uid, a.clone(), Op::Transformed, Rung::Model, Hlc::zero(a))
                    .with_parents([up].into_iter().collect()),
            )
        };
        let store = Store::from_records(vec![
            Record::Unit(parent),
            Record::Unit(core),
            att("model:a/x"),
            att("model:b/y"),
        ]);

        let t = &plain(&store).terms[&uid];
        assert_eq!(t.groups.len(), 1, "one independent group, not two");
        assert_eq!(t.dependent_groups.len(), 1);
        assert_eq!(t.corroboration, 0.25);
    }

    #[test]
    fn groups_with_disjoint_ancestry_both_count() {
        let core = evidence("the claim");
        let uid = canonical_uid(&core);
        let att = |who: &str, parent: u8| {
            let a = agent(who);
            Record::Attestation(
                Attestation::new(uid, a.clone(), Op::Transformed, Rung::Model, Hlc::zero(a))
                    .with_parents([Uid::from_bytes([parent; 32])].into_iter().collect()),
            )
        };
        let store = Store::from_records(vec![
            Record::Unit(core),
            att("model:a/x", 1),
            att("model:b/y", 2),
        ]);
        assert_eq!(plain(&store).terms[&uid].groups.len(), 2);
    }

    #[test]
    fn corroboration_saturates_at_four_groups() {
        let store = Store::from_records(attested(
            evidence("a measurement"),
            &[
                ("model:a/x", None),
                ("model:b/y", None),
                ("model:c/z", None),
                ("model:d/w", None),
                ("model:e/v", None),
                ("model:f/u", None),
            ],
        ));
        let uid = canonical_uid(&evidence("a measurement"));
        assert_eq!(plain(&store).terms[&uid].corroboration, 1.0);
    }

    #[test]
    fn an_unattested_unit_has_no_corroboration() {
        let (store, ue, _, _, _) = diamond();
        assert_eq!(plain(&store).terms[&ue].corroboration, 0.0);
    }

    // --- weights and overrides ---------------------------------------------

    #[test]
    fn the_default_weights_are_the_documented_ones() {
        let w = SalienceWeights::default();
        assert_eq!((w.centrality, w.corroboration, w.role), (0.5, 0.2, 0.3));
        assert_eq!(w.sum(), 1.0);
    }

    /// Corroboration is gameable until agent identity is signed, so a low-trust
    /// deployment must be able to switch it off entirely.
    #[test]
    fn untrusted_deployments_can_zero_the_corroboration_term() {
        let store = Store::from_records(attested(
            evidence("a measurement"),
            &[("model:a/x", None), ("model:b/y", None)],
        ));
        let uid = canonical_uid(&evidence("a measurement"));
        let trusting = salience(&store, &SalienceRequest::default());
        let wary = salience(
            &store,
            &SalienceRequest::default().with_weights(SalienceWeights::untrusted()),
        );
        assert!(trusting.get(&uid) > wary.get(&uid));
        assert_eq!(SalienceWeights::untrusted().corroboration, 0.0);
    }

    #[test]
    fn role_weight_lifts_a_unit() {
        let (store, ue, _, _, uf) = diamond();
        let without = salience(&store, &SalienceRequest::default().seeded([uf]));
        let with = salience(
            &store,
            &SalienceRequest::default()
                .seeded([uf])
                .with_role_weights(BTreeMap::from([(ue, 1.0)])),
        );
        assert!(with.get(&ue) > without.get(&ue));
        assert_eq!(with.terms[&ue].role, 1.0);
    }

    /// An author who says what matters is not second-guessed.
    #[test]
    fn an_authored_salience_overrides_the_derived_value() {
        let (store, ue, _, _, _) = diamond();
        let mut with_override = Store::new();
        for r in store.iter() {
            with_override.append(std::slice::from_ref(r)).unwrap();
        }
        // Re-derive with an authored override in place.
        let derived = plain(&store).get(&ue);
        let mut unit = Unit::new(store.get(&ue).unwrap().core.clone());
        unit = unit.with_salience(0.125);
        assert_ne!(derived, 0.125, "the fixture would not prove anything");
        assert_eq!(unit.salience, Some(0.125));
    }

    #[test]
    fn scores_are_quantised_and_bounded() {
        let (store, _, _, _, uf) = diamond();
        let r = salience(&store, &SalienceRequest::default().seeded([uf]));
        for (uid, s) in &r.scores {
            assert!((0.0..=1.0).contains(s), "{uid}: {s}");
            assert_eq!(quantise(*s), *s, "{uid}: {s} is not on the quantum");
        }
    }

    // --- reporting ---------------------------------------------------------

    /// The seed ranks first, and that is correct: dangling mass returns to it every
    /// iteration, and the unit you are packing *for* is the one you certainly want. The
    /// shared evidence comes next, ahead of the intermediate claims that merely pass
    /// through.
    #[test]
    fn top_returns_the_highest_scoring_units_first() {
        let (store, ue, ua, ub, uf) = diamond();
        let r = salience(&store, &SalienceRequest::default().seeded([uf]));
        let top = r.top(4);
        assert_eq!(top.len(), 4);
        assert_eq!(top[0].0, uf, "the seed");
        assert_eq!(top[1].0, ue, "then the evidence everything rests on");
        assert!(top[0].1 >= top[1].1 && top[1].1 >= top[2].1);
        assert!(
            r.get(&ue) > r.get(&ua) && r.get(&ue) > r.get(&ub),
            "the shared evidence outranks the claims that pass through it"
        );
    }

    #[test]
    fn top_breaks_ties_by_ascending_uid() {
        let a = evidence("a");
        let b = evidence("b");
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
        let store = Store::from_records(vec![Record::Unit(a), Record::Unit(b)]);
        let r = plain(&store);
        assert_eq!(r.get(&ua), r.get(&ub), "the fixture needs a genuine tie");
        let top = r.top(2);
        assert!(top[0].0 < top[1].0);
    }

    /// A unit that is middling across a corpus may be the most important thing in the
    /// thread being packed (§1.5).
    #[test]
    fn renormalising_within_a_selection_rescales_to_one() {
        let (store, ue, ua, ub, uf) = diamond();
        let r = salience(&store, &SalienceRequest::default().seeded([uf]));
        let within = r.renormalise(&[ua, ub]);
        let max = within.values().copied().fold(0.0f32, f32::max);
        assert_eq!(max, 1.0);
        assert!(
            r.get(&ua) < r.get(&ue),
            "but ranks lower across the whole store"
        );
    }

    #[test]
    fn renormalising_an_empty_or_zero_selection_is_safe() {
        let (store, _, _, _, uf) = diamond();
        let r = salience(&store, &SalienceRequest::default().seeded([uf]));
        assert!(r.renormalise(&[]).is_empty());
        let unknown = Uid::from_bytes([9; 32]);
        assert_eq!(r.renormalise(&[unknown])[&unknown], 0.0);
    }

    #[test]
    fn explain_reports_every_term() {
        let store = Store::from_records(attested(
            evidence("a measurement"),
            &[("model:a/x", None), ("model:b/y", None)],
        ));
        let uid = canonical_uid(&evidence("a measurement"));
        let t = plain(&store).explain(&uid).cloned().unwrap();
        assert!(t.centrality > 0.0);
        assert_eq!(t.corroboration, 0.5);
        assert_eq!(t.role, 0.0);
        assert!(!t.authored);
        assert_eq!(t.groups.len(), 2);
    }

    // --- determinism -------------------------------------------------------

    #[test]
    fn salience_is_deterministic() {
        let (store, _, _, _, uf) = diamond();
        let a = salience(&store, &SalienceRequest::default().seeded([uf]));
        let b = salience(&store, &SalienceRequest::default().seeded([uf]));
        assert_eq!(a, b);
    }

    /// Record arrival order must not reach the scores - dense ids follow ascending uid,
    /// so the accumulation order is fixed whatever the log looked like.
    #[test]
    fn record_order_does_not_change_the_scores() {
        let (store, _, _, _, uf) = diamond();
        let mut reversed: Vec<Record> = store.iter().cloned().collect();
        reversed.reverse();
        let other = Store::from_records(reversed);
        assert_eq!(
            salience(&store, &SalienceRequest::default().seeded([uf])).scores,
            salience(&other, &SalienceRequest::default().seeded([uf])).scores
        );
    }

    #[test]
    fn an_empty_store_has_no_salience() {
        let r = plain(&Store::new());
        assert!(r.scores.is_empty());
        assert!(r.top(5).is_empty());
    }

    #[test]
    fn view_roots_are_the_default_seed() {
        use smysl_core::{View, ViewId};
        let (store, ue, _, _, uf) = diamond();
        let mut with_view = Store::from_records(store.iter().cloned().collect());
        with_view
            .append(&[Record::View(
                View::new(ViewId::new("v/x").unwrap(), "i").with_roots([uf]),
            )])
            .unwrap();
        assert_eq!(view_roots(&with_view), [uf].into_iter().collect());
        let _ = ue;
    }

    #[test]
    fn relations_carrying_support_contribute_rank() {
        let a = evidence("a");
        let b = evidence("b");
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
        let with_edge = Store::from_records(vec![
            Record::Unit(a.clone()),
            Record::Unit(b.clone()),
            Record::Relation(Relation::new(RelKind::Causes, ub, ua)),
        ]);
        let without = Store::from_records(vec![Record::Unit(a), Record::Unit(b)]);

        let seeded = SalienceRequest::default().seeded([ub]);
        assert!(salience(&with_edge, &seeded).terms[&ua].centrality > 0.0);
        assert_eq!(salience(&without, &seeded).terms[&ua].centrality, 0.0);
    }

    #[test]
    fn a_rebuttal_does_not_carry_rank() {
        let a = evidence("a");
        let b = evidence("b");
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
        let store = Store::from_records(vec![
            Record::Unit(a),
            Record::Unit(b),
            Record::Relation(Relation::new(RelKind::Rebuts, ub, ua)),
        ]);
        let r = salience(&store, &SalienceRequest::default().seeded([ub]));
        assert_eq!(
            r.terms[&ua].centrality, 0.0,
            "disagreeing with something is not depending on it"
        );
    }

    // -- episodes and recency ----------------------------------------------

    fn at_hop(gist: &str, hop: u32) -> (Record, Record, Uid) {
        let core = smysl_core::UnitCoreBuilder::new(
            smysl_core::KernelType::Claim,
            gist,
            smysl_core::Status::Speculative,
        )
        .build()
        .unwrap();
        let uid = smysl_core::canonical_uid(&core);
        let agent = smysl_core::AgentId::new("tool:t").unwrap();
        let att = smysl_core::Attestation::new(
            uid,
            agent.clone(),
            smysl_core::Op::Authored,
            smysl_core::Rung::Computed,
            smysl_core::Hlc::zero(agent),
        )
        .at_hop(hop);
        (Record::Unit(core), Record::Attestation(att), uid)
    }

    #[test]
    fn a_units_hop_is_the_newest_one_attested() {
        let (u, a, uid) = at_hop("carried forward", 3);
        let store = Store::from_records(vec![u, a]);
        assert_eq!(store.hop_of(&uid), Some(3));
        assert_eq!(store.latest_hop(), Some(3));
        assert_eq!(store.at_hop(3).count(), 1);
        assert_eq!(store.at_hop(2).count(), 0);
    }

    #[test]
    fn a_unit_with_no_attestation_has_no_episode() {
        let core = smysl_core::UnitCoreBuilder::new(
            smysl_core::KernelType::Claim,
            "unplaced",
            smysl_core::Status::Speculative,
        )
        .build()
        .unwrap();
        let uid = smysl_core::canonical_uid(&core);
        let store = Store::from_records(vec![Record::Unit(core)]);
        assert_eq!(store.hop_of(&uid), None);
        assert!(store.hops().is_empty());
    }

    /// Halving per hop, and saturating rather than wrapping on a very old unit.
    #[test]
    fn recency_halves_each_hop_and_bottoms_out() {
        assert_eq!(recency_at(0), 1.0);
        assert_eq!(recency_at(1), 0.5);
        assert_eq!(recency_at(2), 0.25);
        assert_eq!(recency_at(64), 0.0, "no wraparound on an ancient unit");
    }

    /// **The point of the term.** Structural salience only grows, so a well-connected old
    /// claim outranks a fresh one permanently. With recency weighted, the fresh one wins.
    #[test]
    fn a_fresh_unit_can_outrank_an_older_one() {
        let (u_old, a_old, old) = at_hop("the old claim", 0);
        let (u_new, a_new, new) = at_hop("the new claim", 5);
        let store = Store::from_records(vec![u_old, a_old, u_new, a_new]);

        // Off by default: the two are indistinguishable on structure alone.
        let flat = salience(&store, &SalienceRequest::default().at_hop(5));
        assert_eq!(
            flat.get(&old),
            flat.get(&new),
            "recency leaked in while off"
        );

        let req = SalienceRequest::default()
            .with_weights(SalienceWeights::recent())
            .at_hop(5);
        let out = salience(&store, &req);
        assert!(
            out.get(&new) > out.get(&old),
            "the fresh unit did not outrank the old one: {} vs {}",
            out.get(&new),
            out.get(&old)
        );
    }

    /// Supplied rather than read, so a replay can ask what salience looked like *then*.
    #[test]
    fn recency_is_measured_against_the_hop_the_caller_names() {
        let (u, a, uid) = at_hop("a claim", 2);
        let store = Store::from_records(vec![u, a]);
        let w = SalienceWeights::recent();

        let then = salience(
            &store,
            &SalienceRequest::default().with_weights(w).at_hop(2),
        );
        let later = salience(
            &store,
            &SalienceRequest::default().with_weights(w).at_hop(6),
        );
        assert!(
            then.get(&uid) > later.get(&uid),
            "the same unit did not decay as the pipeline moved on"
        );
    }

    /// Rule D: no clock is read, so two runs of the same request agree exactly.
    #[test]
    fn recency_does_not_make_salience_non_reproducible() {
        let (u, a, _) = at_hop("a claim", 1);
        let store = Store::from_records(vec![u, a]);
        let req = SalienceRequest::default()
            .with_weights(SalienceWeights::recent())
            .at_hop(4);
        assert_eq!(salience(&store, &req).scores, salience(&store, &req).scores);
    }

    /// A unit with no episode must not be ranked as merely old: it is unplaced, and
    /// inventing a distance for it would be inventing a fact about where it came from.
    #[test]
    fn an_unplaced_unit_gets_no_recency_rather_than_the_worst() {
        let core = smysl_core::UnitCoreBuilder::new(
            smysl_core::KernelType::Claim,
            "unplaced",
            smysl_core::Status::Speculative,
        )
        .build()
        .unwrap();
        let uid = smysl_core::canonical_uid(&core);
        let store = Store::from_records(vec![Record::Unit(core)]);
        let out = salience(
            &store,
            &SalienceRequest::default()
                .with_weights(SalienceWeights::recent())
                .at_hop(9),
        );
        assert_eq!(out.explain(&uid).map(|t| t.recency), Some(0.0));
    }
}
