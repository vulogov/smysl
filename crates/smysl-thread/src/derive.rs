//! Deterministic thread derivation (§19).
//!
//! Four stages: assign roles by the schema's rule table, select within each role by
//! salience, order by Kahn over the ordering edges, then repair coherence.
//!
//! Every stage is a pure function of the graph. Role assignment takes the first matching
//! rule, selection breaks ties by ascending uid, Kahn pops the smallest ready id, and
//! repair inserts at a determined position - so the same store and schema always produce
//! the same thread (rule D).
//!
//! `synth_gist` is deliberately unambitious. It concatenates the two most salient opening
//! steps and truncates; anything better needs a model, and that is what `--refine` is for -
//! which records `op: Transformed`, so a refined thread is distinguishable from a derived
//! one for ever after.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::{
    tokens, AgentId, Hlc, RelKind, Role, Status, Step, Thread, ThreadId, ThreadSchema, Uid,
};
use smysl_graph::{salience, topo, EdgeSet, SalienceReport, SalienceRequest, Store};

use crate::schema::{definition, Matcher, Position, SchemaDef};

/// Default gist bound, matching every granularity profile's `l0_max` (§1.6).
pub const GIST_TOKENS: u32 = 30;

/// How to derive.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DeriveOptions {
    pub id: ThreadId,
    pub owner: AgentId,
    /// Supplied rather than read, so derivation is a bit-reproducible function of its
    /// inputs (rule D). The CLI passes a real clock.
    pub ts: Option<Hlc>,
    /// Per-role arity overrides, for `thread --arity ROLE=N`.
    pub arity: BTreeMap<Role, usize>,
    /// Restrict derivation to these units; empty means the whole store.
    pub scope: BTreeSet<Uid>,
    pub gist_tokens: u32,
}

impl Default for DeriveOptions {
    fn default() -> DeriveOptions {
        DeriveOptions {
            id: ThreadId::new("t/derived").expect("a valid literal"),
            owner: AgentId::new("tool:smysl-thread").expect("a valid literal"),
            ts: None,
            arity: BTreeMap::new(),
            scope: BTreeSet::new(),
            gist_tokens: GIST_TOKENS,
        }
    }
}

impl DeriveOptions {
    pub fn new(id: ThreadId, owner: AgentId) -> DeriveOptions {
        DeriveOptions {
            id,
            owner,
            ..DeriveOptions::default()
        }
    }

    pub fn with_ts(mut self, ts: Hlc) -> DeriveOptions {
        self.ts = Some(ts);
        self
    }

    pub fn scoped(mut self, s: impl IntoIterator<Item = Uid>) -> DeriveOptions {
        self.scope = s.into_iter().collect();
        self
    }

    pub fn with_arity(mut self, role: Role, n: usize) -> DeriveOptions {
        self.arity.insert(role, n);
        self
    }
}

/// What derivation did, beyond the thread itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeriveReport {
    /// Units inserted by coherence repair, with the step that needed them.
    pub repaired: Vec<(Uid, Uid)>,
    /// Roles the schema requires that no unit could fill.
    pub unfilled: Vec<Role>,
    /// Units considered but not selected.
    pub unselected: usize,
}

/// Derive a thread over a store (§19).
pub fn derive_thread(
    store: &Store,
    schema: ThreadSchema,
    opts: &DeriveOptions,
) -> (Thread, DeriveReport) {
    let def = definition(schema);
    let scope: Vec<Uid> = if opts.scope.is_empty() {
        store.units().map(|(u, _)| *u).collect()
    } else {
        opts.scope
            .iter()
            .copied()
            .filter(|u| store.contains_uid(u))
            .collect()
    };

    let sal = salience(
        store,
        &SalienceRequest::default().seeded(seed(store, &scope)),
    );
    let order = ordering(store, &scope);

    // 1. role assignment - first matching rule wins.
    let roles = assign(store, def, &scope, &sal, &order);

    // 2. selection - the most salient within each role, up to its arity.
    let (picked, mut report) = select(def, opts, &roles, &sal);

    // 3. ordering - Kahn over the ordering edges, ready set popped by role then salience.
    let mut steps = order_steps(def, &picked, &order, &sal);

    // 4. coherence repair.
    repair(store, &mut steps, &mut report);

    let gist = synth_gist(store, def, &steps, opts.gist_tokens);
    let ts = opts
        .ts
        .clone()
        .unwrap_or_else(|| Hlc::new(0, 0, opts.owner.clone()));

    let thread =
        Thread::new(opts.id.clone(), schema, opts.owner.clone(), gist, ts).with_steps(steps);
    report.unselected = scope.len().saturating_sub(thread.steps.len());
    (thread, report)
}

/// The personalisation vector for derivation: what the schema's most important role is
/// about, or the view's roots when nothing matches.
fn seed(store: &Store, scope: &[Uid]) -> BTreeSet<Uid> {
    let focal: BTreeSet<Uid> = scope
        .iter()
        .copied()
        .filter(|u| {
            store.get(u).is_some_and(|unit| {
                matches!(
                    unit.core.schema.kernel(),
                    Some(smysl_core::KernelType::Question) | Some(smysl_core::KernelType::Finding)
                )
            })
        })
        .collect();
    if focal.is_empty() {
        let roots = smysl_graph::view_roots(store);
        if roots.is_empty() {
            scope.iter().copied().collect()
        } else {
            roots
        }
    } else {
        focal
    }
}

/// Position of each unit in the ordering chain, and the chain itself.
struct Ordering {
    /// Index in the topological order; absent units sort last.
    index: BTreeMap<Uid, usize>,
    /// Rank along the chain *within the scope*, so a band does not shift when units
    /// outside the scope are added to the store.
    rank: BTreeMap<Uid, usize>,
    first: BTreeSet<Uid>,
    last: BTreeSet<Uid>,
}

impl Ordering {
    /// Whether a unit falls in band `i` of `n` along the chain.
    fn in_band(&self, uid: &Uid, i: usize, n: usize) -> bool {
        if n == 0 || self.rank.is_empty() {
            return false;
        }
        match self.rank.get(uid) {
            Some(r) => r * n / self.rank.len() == i,
            None => false,
        }
    }
}

fn ordering(store: &Store, scope: &[Uid]) -> Ordering {
    let g = store.adjacency();
    let kinds = EdgeSet::ordering();
    let t = topo(g, &kinds);

    let mut index = BTreeMap::new();
    for (i, node) in t.order.iter().chain(t.cyclic.iter()).enumerate() {
        if let Some(uid) = g.uid(*node) {
            index.insert(*uid, i);
        }
    }

    let in_scope: BTreeSet<Uid> = scope.iter().copied().collect();
    let mut first = BTreeSet::new();
    let mut last = BTreeSet::new();
    for uid in &in_scope {
        let Some(id) = g.id(uid) else { continue };
        // Ordering edges point from the later unit to the earlier one, so a unit with no
        // outgoing edge starts the chain and one with nothing pointing at it ends it.
        if g.out(id, &kinds).is_empty() {
            first.insert(*uid);
        }
        if g.incoming(id, &kinds).is_empty() {
            last.insert(*uid);
        }
    }
    // A unit with no ordering edges at all is neither the start nor the end of anything.
    let isolated: Vec<Uid> = first.intersection(&last).copied().collect();
    for u in isolated {
        first.remove(&u);
        last.remove(&u);
    }

    // Rank within the scope, by chain index then uid - a total order, so a band is a
    // function of the graph rather than of iteration.
    let mut ordered: Vec<Uid> = in_scope.iter().copied().collect();
    ordered.sort_by_key(|u| (index.get(u).copied().unwrap_or(usize::MAX), *u));
    let rank: BTreeMap<Uid, usize> = ordered.iter().enumerate().map(|(i, u)| (*u, i)).collect();

    Ordering {
        index,
        rank,
        first,
        last,
    }
}

fn assign(
    store: &Store,
    def: &SchemaDef,
    scope: &[Uid],
    sal: &SalienceReport,
    order: &Ordering,
) -> BTreeMap<Uid, Role> {
    let ranked: Vec<Uid> = sal.top(scope.len()).into_iter().map(|(u, _)| u).collect();

    let mut out = BTreeMap::new();
    for uid in scope {
        for (matcher, role) in def.rules {
            if matches(store, matcher, uid, order, &ranked) {
                out.insert(*uid, *role);
                break;
            }
        }
    }
    out
}

fn matches(store: &Store, m: &Matcher, uid: &Uid, order: &Ordering, ranked: &[Uid]) -> bool {
    let Some(unit) = store.get(uid) else {
        return false;
    };
    match m {
        Matcher::Any => true,
        Matcher::Type(k) => unit.core.schema.kernel() == Some(*k),
        Matcher::StatusAtLeast(s) => unit.core.status >= *s,
        Matcher::SourceOf(k) => store
            .relations_of_kind(k)
            .into_iter()
            .any(|r| r.from == *uid),
        Matcher::TargetOf(k) => store.relations_of_kind(k).into_iter().any(|r| r.to == *uid),
        Matcher::SalienceTop(n) => ranked.iter().take(*n).any(|u| u == uid),
        Matcher::At(Position::First) => order.first.contains(uid),
        Matcher::At(Position::Last) => order.last.contains(uid),
        Matcher::At(Position::Middle) => !order.first.contains(uid) && !order.last.contains(uid),
        Matcher::At(Position::Band(i, n)) => order.in_band(uid, *i, *n),
    }
}

fn select(
    def: &SchemaDef,
    opts: &DeriveOptions,
    roles: &BTreeMap<Uid, Role>,
    sal: &SalienceReport,
) -> (BTreeMap<Role, Vec<Uid>>, DeriveReport) {
    let mut picked: BTreeMap<Role, Vec<Uid>> = BTreeMap::new();
    let mut report = DeriveReport::default();

    for role in def.roles {
        let mut candidates: Vec<Uid> = roles
            .iter()
            .filter(|(_, r)| *r == role)
            .map(|(u, _)| *u)
            .collect();
        // Most salient first; ties by ascending uid, so the choice is a function of the
        // graph rather than of iteration order.
        candidates.sort_by(|a, b| {
            sal.get(b)
                .partial_cmp(&sal.get(a))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.cmp(b))
        });

        let arity = def.arity_of(*role);
        let take = opts
            .arity
            .get(role)
            .copied()
            .unwrap_or_else(|| *arity.end())
            .min(candidates.len());
        candidates.truncate(take);

        if candidates.len() < *arity.start() {
            report.unfilled.push(*role);
        }
        if !candidates.is_empty() {
            picked.insert(*role, candidates);
        }
    }
    (picked, report)
}

/// Kahn over the ordering edges, with the ready set popped by role order, then salience,
/// then uid - a total order, so the sequence is reproducible.
fn order_steps(
    def: &SchemaDef,
    picked: &BTreeMap<Role, Vec<Uid>>,
    order: &Ordering,
    sal: &SalienceReport,
) -> Vec<Step> {
    let mut steps: Vec<(usize, usize, Uid, Role)> = Vec::new();
    for (role, units) in picked {
        let role_pos = def.position_of(*role).unwrap_or(usize::MAX);
        for uid in units {
            let chain = order.index.get(uid).copied().unwrap_or(usize::MAX);
            steps.push((role_pos, chain, *uid, *role));
        }
    }

    steps.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then_with(|| {
                sal.get(&b.2)
                    .partial_cmp(&sal.get(&a.2))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then(a.2.cmp(&b.2))
    });

    steps
        .into_iter()
        .map(|(_, _, uid, role)| Step::new(role, uid))
        .collect()
}

/// Coherence repair (§19 step 4).
///
/// A step whose deps are absent is not interpretable at L0, which is rule L's whole claim.
/// The missing dep is inserted immediately before the step that needs it, taking that
/// step's role - so it always precedes its dependent and the position is determined rather
/// than chosen.
///
/// Repair iterates to a fixpoint, because an inserted dep may itself have deps.
fn repair(store: &Store, steps: &mut Vec<Step>, report: &mut DeriveReport) {
    loop {
        let present: BTreeSet<Uid> = steps.iter().map(|s| s.unit).collect();
        let mut insertion: Option<(usize, Step, Uid)> = None;

        for (i, step) in steps.iter().enumerate() {
            let Some(unit) = store.get(&step.unit) else {
                continue;
            };
            // Ascending uid, so which missing dep is repaired first is determined.
            let missing = unit
                .core
                .deps
                .iter()
                .find(|d| !present.contains(d) && store.contains_uid(d));
            if let Some(d) = missing {
                insertion = Some((i, Step::new(step.role, *d), step.unit));
                break;
            }
        }

        match insertion {
            Some((at, step, needed_by)) => {
                report.repaired.push((step.unit, needed_by));
                steps.insert(at, step);
            }
            None => break,
        }
    }
}

/// A gist for the thread, from its opening steps.
///
/// Deliberately unambitious: it concatenates and truncates. Anything better needs a model,
/// which is what `--refine` is for.
fn synth_gist(store: &Store, def: &SchemaDef, steps: &[Step], budget: u32) -> String {
    // The heaviest roles, not the first ones. Weight is the table's own statement of what
    // a role contributes, so a gist built from the opening roles would summarise an
    // analysis by its definitions rather than by its finding.
    let mut order: Vec<Role> = def.roles.to_vec();
    order.sort_by(|a, b| {
        def.weight_of(*b)
            .partial_cmp(&def.weight_of(*a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(def.position_of(*a).cmp(&def.position_of(*b)))
    });

    let mut parts: Vec<&str> = Vec::new();
    for role in order.iter().take(2) {
        if let Some(step) = steps.iter().find(|s| s.role == *role) {
            if let Some(unit) = store.get(&step.unit) {
                parts.push(unit.core.gist.trim());
            }
        }
    }
    if parts.is_empty() {
        if let Some(first) = steps.first().and_then(|s| store.get(&s.unit)) {
            parts.push(first.core.gist.trim());
        }
    }

    let joined = parts.join("; ");
    if tokens(&joined) <= budget {
        return joined;
    }

    // Truncate on a word boundary, so the result reads as a shortened sentence rather
    // than a severed one.
    let limit = (budget as usize) * 4;
    let mut out = String::new();
    for word in joined.split_whitespace() {
        if out.len() + word.len() + 1 > limit.saturating_sub(1) {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    if out.is_empty() {
        out.push_str(joined.split_whitespace().next().unwrap_or("thread"));
    }
    out.push('\u{2026}');
    out
}

/// Whether a thread satisfies rule L: no step references a unit whose deps are absent.
///
/// The SM-P11 gate asserts this over generated graphs - the repair either works always or
/// it is not a repair.
pub fn satisfies_rule_l(store: &Store, thread: &Thread) -> Vec<(Uid, Uid)> {
    let present: BTreeSet<Uid> = thread.units().copied().collect();
    let mut out = Vec::new();
    for uid in &present {
        let Some(unit) = store.get(uid) else { continue };
        for d in &unit.core.deps {
            if store.contains_uid(d) && !present.contains(d) {
                out.push((*uid, *d));
            }
        }
    }
    out.sort();
    out
}

/// Role weights a derived thread implies, for the `w_t` term of salience (§1.5).
pub fn schema_role_weight(schema: ThreadSchema, role: Role) -> f32 {
    definition(schema).weight_of(role)
}

/// Whether a status is strong enough for a matcher. Exposed for the CLI's `--show`.
pub fn status_at_least(status: Status, floor: Status) -> bool {
    status >= floor
}

/// The ordering edge kinds derivation walks (§19).
pub fn ordering_kinds() -> Vec<RelKind> {
    RelKind::KERNEL
        .iter()
        .filter(|k| k.is_ordering())
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{
        canonical_uid, KernelType, Record, Relation, SourceKind, SourceRef, UnitCore,
        UnitCoreBuilder,
    };

    fn unit(t: KernelType, gist: &str, status: Status) -> UnitCore {
        let mut b = UnitCoreBuilder::new(t, gist, status);
        if status == Status::Measured {
            b = b.source(SourceRef::new(SourceKind::Metric, "m"));
        }
        b.build().unwrap()
    }

    fn evidence(gist: &str) -> UnitCore {
        unit(KernelType::Evidence, gist, Status::Measured)
    }

    fn opts() -> DeriveOptions {
        DeriveOptions::new(
            ThreadId::new("t/derived").unwrap(),
            AgentId::new("tool:test").unwrap(),
        )
    }

    fn derive(store: &Store, schema: ThreadSchema) -> (Thread, DeriveReport) {
        derive_thread(store, schema, &opts())
    }

    #[test]
    fn a_question_and_an_answer_become_a_qa_thread() {
        let q = unit(
            KernelType::Question,
            "why did latency rise",
            Status::Speculative,
        );
        let uq = canonical_uid(&q);
        let a = UnitCoreBuilder::new(KernelType::Finding, "the cache was cold", Status::Derived)
            .grounds([uq])
            .build()
            .unwrap();
        let ua = canonical_uid(&a);
        let store = Store::from_records(vec![
            Record::Unit(q),
            Record::Unit(a),
            Record::Relation(Relation::new(RelKind::Answers, ua, uq)),
        ]);

        let (t, r) = derive(&store, ThreadSchema::Qa);
        assert_eq!(t.schema, ThreadSchema::Qa);
        assert_eq!(
            t.steps.iter().find(|s| s.unit == uq).map(|s| s.role),
            Some(Role::Question)
        );
        assert_eq!(
            t.steps.iter().find(|s| s.unit == ua).map(|s| s.role),
            Some(Role::Answer)
        );
        assert!(r.unfilled.is_empty(), "both required roles were filled");
    }

    /// A rebuttal is the unit doing the rebutting. Putting the rebutted claim in the
    /// caveat slot would tell the reader the opposite of what happened.
    #[test]
    fn the_rebutting_unit_takes_the_rebuttal_role() {
        let claim = unit(KernelType::Finding, "the fix worked", Status::Speculative);
        let uc = canonical_uid(&claim);
        let reb = unit(
            KernelType::Observation,
            "it regressed on tuesday",
            Status::Measured,
        );
        let ur = canonical_uid(&reb);
        let store = Store::from_records(vec![
            Record::Unit(claim),
            Record::Unit(reb),
            Record::Relation(Relation::new(RelKind::Rebuts, ur, uc)),
        ]);

        let (t, _) = derive(&store, ThreadSchema::Analysis);
        assert_eq!(
            t.steps.iter().find(|s| s.unit == ur).map(|s| s.role),
            Some(Role::Rebuttal)
        );
        assert_ne!(
            t.steps.iter().find(|s| s.unit == uc).map(|s| s.role),
            Some(Role::Rebuttal),
            "the rebutted claim is not the rebuttal"
        );
    }

    /// The narrative table is positional: the unit nothing is ordered before opens, the
    /// unit nothing follows closes.
    #[test]
    fn narrative_assigns_by_position_in_the_chain() {
        let a = evidence("first this happened");
        let ua = canonical_uid(&a);
        let b = evidence("then this happened");
        let ub = canonical_uid(&b);
        let c = evidence("and finally this");
        let uc = canonical_uid(&c);
        // Sequences points from the later unit to the earlier one.
        let store = Store::from_records(vec![
            Record::Unit(a),
            Record::Unit(b),
            Record::Unit(c),
            Record::Relation(Relation::new(RelKind::Sequences, ub, ua)),
            Record::Relation(Relation::new(RelKind::Sequences, uc, ub)),
        ]);

        let (t, _) = derive(&store, ThreadSchema::Narrative);
        assert_eq!(
            t.steps.iter().find(|s| s.unit == ua).map(|s| s.role),
            Some(Role::Setup)
        );
        assert_eq!(
            t.steps.iter().find(|s| s.unit == uc).map(|s| s.role),
            Some(Role::Coda)
        );
        assert_eq!(
            t.steps.iter().find(|s| s.unit == ub).map(|s| s.role),
            Some(Role::Complication)
        );
    }

    #[test]
    fn a_role_never_exceeds_its_arity() {
        let mut records = Vec::new();
        for i in 0..8 {
            records.push(Record::Unit(evidence(&format!("supporting point {i}"))));
        }
        records.push(Record::Unit(unit(
            KernelType::Finding,
            "the bottom line",
            Status::Speculative,
        )));
        let store = Store::from_records(records);

        let (t, report) = derive(&store, ThreadSchema::Brief);
        let n = t.steps.iter().filter(|s| s.role == Role::Support).count();
        assert_eq!(n, 3, "brief allows at most three supporting points");
        assert!(report.unselected >= 5);
    }

    #[test]
    fn an_arity_override_widens_a_role() {
        let mut records = vec![Record::Unit(unit(
            KernelType::Finding,
            "the bottom line",
            Status::Speculative,
        ))];
        for i in 0..8 {
            records.push(Record::Unit(evidence(&format!("supporting point {i}"))));
        }
        let store = Store::from_records(records);

        let o = opts().with_arity(Role::Support, 6);
        let (t, _) = derive_thread(&store, ThreadSchema::Brief, &o);
        assert_eq!(
            t.steps.iter().filter(|s| s.role == Role::Support).count(),
            6
        );
    }

    /// A role the graph cannot fill is reported rather than faked. A thread that silently
    /// omits its required role is a thread that lies about its schema.
    #[test]
    fn an_unfillable_required_role_is_reported() {
        let store = Store::from_records(vec![Record::Unit(evidence("only evidence here"))]);
        let (_, r) = derive(&store, ThreadSchema::Qa);
        assert!(r.unfilled.contains(&Role::Question));
    }

    /// Coherence repair: a selected unit whose dep was not selected pulls it in.
    #[test]
    fn repair_pulls_in_a_missing_dep() {
        let base = unit(
            KernelType::Definition,
            "what p99 means here",
            Status::Speculative,
        );
        let ub = canonical_uid(&base);
        let f = UnitCoreBuilder::new(KernelType::Finding, "p99 doubled", Status::Speculative)
            .deps([ub])
            .build()
            .unwrap();
        let uf = canonical_uid(&f);
        let store = Store::from_records(vec![Record::Unit(base), Record::Unit(f)]);

        // Only the finding is in scope, so its dep can arrive by repair alone.
        let o = opts().scoped([uf]);
        let (t, r) = derive_thread(&store, ThreadSchema::Brief, &o);
        assert!(t.units().any(|u| *u == ub), "the dep was not repaired in");
        assert_eq!(r.repaired, vec![(ub, uf)]);
        assert!(satisfies_rule_l(&store, &t).is_empty());
    }

    /// The dep is inserted *before* the step that needs it, so a reader meets the
    /// definition before the claim that rests on it.
    #[test]
    fn a_repaired_dep_precedes_its_dependent() {
        let base = unit(
            KernelType::Definition,
            "the definition",
            Status::Speculative,
        );
        let ub = canonical_uid(&base);
        let f = UnitCoreBuilder::new(KernelType::Finding, "the finding", Status::Speculative)
            .deps([ub])
            .build()
            .unwrap();
        let uf = canonical_uid(&f);
        let store = Store::from_records(vec![Record::Unit(base), Record::Unit(f)]);

        let (t, _) = derive_thread(&store, ThreadSchema::Brief, &opts().scoped([uf]));
        let pos = |u: Uid| t.steps.iter().position(|s| s.unit == u).unwrap();
        assert!(pos(ub) < pos(uf));
    }

    /// Repair runs to a fixpoint: an inserted dep may itself have deps.
    #[test]
    fn repair_is_transitive() {
        let a = unit(
            KernelType::Definition,
            "the ground floor",
            Status::Speculative,
        );
        let ua = canonical_uid(&a);
        let b = UnitCoreBuilder::new(KernelType::Definition, "the middle", Status::Speculative)
            .deps([ua])
            .build()
            .unwrap();
        let ub = canonical_uid(&b);
        let c = UnitCoreBuilder::new(KernelType::Finding, "the top", Status::Speculative)
            .deps([ub])
            .build()
            .unwrap();
        let uc = canonical_uid(&c);
        let store = Store::from_records(vec![Record::Unit(a), Record::Unit(b), Record::Unit(c)]);

        let (t, r) = derive_thread(&store, ThreadSchema::Brief, &opts().scoped([uc]));
        assert_eq!(t.steps.len(), 3);
        assert_eq!(r.repaired.len(), 2);
        assert!(satisfies_rule_l(&store, &t).is_empty());
    }

    /// A dep the store does not hold cannot be repaired in, and must not be reported as a
    /// rule L violation either - the unit is simply not here.
    #[test]
    fn an_absent_dep_is_not_a_rule_l_violation() {
        let dangling = Uid::from_bytes([7; 32]);
        let f = UnitCoreBuilder::new(
            KernelType::Finding,
            "rests on elsewhere",
            Status::Speculative,
        )
        .deps([dangling])
        .build()
        .unwrap();
        let store = Store::from_records(vec![Record::Unit(f)]);
        let (t, r) = derive(&store, ThreadSchema::Brief);
        assert!(r.repaired.is_empty());
        assert!(satisfies_rule_l(&store, &t).is_empty());
    }

    #[test]
    fn derivation_is_reproducible() {
        let q = unit(KernelType::Question, "the question", Status::Speculative);
        let e = evidence("the evidence");
        let store = Store::from_records(vec![Record::Unit(q), Record::Unit(e)]);
        assert_eq!(
            derive(&store, ThreadSchema::Qa).0,
            derive(&store, ThreadSchema::Qa).0
        );
    }

    /// Derivation takes its timestamp rather than reading a clock, so the same store
    /// derives the same bytes on any machine at any time (rule D).
    #[test]
    fn the_timestamp_is_supplied_not_read() {
        let store = Store::from_records(vec![Record::Unit(evidence("a"))]);
        let ts = Hlc::new(42, 1, AgentId::new("tool:test").unwrap());
        let (t, _) = derive_thread(&store, ThreadSchema::Brief, &opts().with_ts(ts.clone()));
        assert_eq!(t.ts, ts);
    }

    #[test]
    fn an_empty_store_derives_an_empty_thread() {
        let store = Store::new();
        let (t, r) = derive(&store, ThreadSchema::Analysis);
        assert!(t.steps.is_empty());
        assert!(!r.unfilled.is_empty(), "nothing can fill a required role");
        assert!(satisfies_rule_l(&store, &t).is_empty());
    }

    /// The heaviest roles, not the opening ones: an analysis summarised by its definitions
    /// would tell a reader nothing about what it found.
    #[test]
    fn the_gist_comes_from_the_heaviest_roles() {
        let d = unit(
            KernelType::Definition,
            "what p99 means",
            Status::Speculative,
        );
        let f = unit(
            KernelType::Finding,
            "the cache was cold",
            Status::Speculative,
        );
        let store = Store::from_records(vec![Record::Unit(d), Record::Unit(f)]);
        let (t, _) = derive(&store, ThreadSchema::Analysis);
        assert!(t.gist.contains("the cache was cold"), "{}", t.gist);
        assert!(!t.gist.contains("what p99 means"), "{}", t.gist);
    }

    /// A five-unit chain fills all five narrative roles. Before the table banded its
    /// middle, `turn` and `resolution` were unreachable and three units piled into
    /// `complication`, of which the arity then threw one away.
    #[test]
    fn a_five_unit_chain_fills_every_narrative_role() {
        let mut records = Vec::new();
        let mut uids = Vec::new();
        for i in 0..5 {
            let u = evidence(&format!("step {i} of the story"));
            uids.push(canonical_uid(&u));
            records.push(Record::Unit(u));
        }
        for w in uids.windows(2) {
            records.push(Record::Relation(Relation::new(
                RelKind::Sequences,
                w[1],
                w[0],
            )));
        }
        let store = Store::from_records(records);

        let (t, _) = derive(&store, ThreadSchema::Narrative);
        let roles: Vec<Role> = t.steps.iter().map(|s| s.role).collect();
        for want in ThreadSchema::Narrative.roles() {
            assert!(roles.contains(want), "{want} was never assigned: {roles:?}");
        }
        assert_eq!(t.steps.len(), 5);
    }

    #[test]
    fn the_gist_is_truncated_to_its_budget() {
        let long = "a gist that goes on and on and on ".repeat(12);
        let f = unit(KernelType::Finding, &long, Status::Speculative);
        let store = Store::from_records(vec![Record::Unit(f)]);
        let (t, _) = derive(&store, ThreadSchema::Brief);
        assert!(t.gist.ends_with('\u{2026}'));
        assert!(tokens(&t.gist) <= GIST_TOKENS + 1, "{}", tokens(&t.gist));
    }

    #[test]
    fn scope_restricts_what_derivation_considers() {
        let a = evidence("in scope");
        let ua = canonical_uid(&a);
        let b = evidence("out of scope");
        let ub = canonical_uid(&b);
        let store = Store::from_records(vec![Record::Unit(a), Record::Unit(b)]);
        let (t, _) = derive_thread(&store, ThreadSchema::Brief, &opts().scoped([ua]));
        assert!(t.units().any(|u| *u == ua));
        assert!(!t.units().any(|u| *u == ub));
    }

    #[test]
    fn every_schema_derives_something_from_the_same_store() {
        let q = unit(KernelType::Question, "the question", Status::Speculative);
        let uq = canonical_uid(&q);
        let f = UnitCoreBuilder::new(KernelType::Finding, "the finding", Status::Derived)
            .grounds([uq])
            .build()
            .unwrap();
        let p = unit(KernelType::Procedure, "the procedure", Status::Speculative);
        let store = Store::from_records(vec![Record::Unit(q), Record::Unit(f), Record::Unit(p)]);

        for &s in ThreadSchema::ALL {
            let (t, _) = derive(&store, s);
            assert!(!t.steps.is_empty(), "{s} derived nothing");
            assert!(satisfies_rule_l(&store, &t).is_empty(), "{s} broke rule L");
        }
    }

    #[test]
    fn ordering_kinds_are_the_ordering_kinds() {
        let k = ordering_kinds();
        assert!(k.contains(&RelKind::Sequences));
        assert!(!k.contains(&RelKind::Rebuts));
    }

    #[test]
    fn status_comparison_is_by_strength() {
        assert!(status_at_least(Status::Measured, Status::Cited));
        assert!(!status_at_least(Status::Speculative, Status::Measured));
    }

    #[test]
    fn schema_role_weights_come_from_the_table() {
        assert_eq!(
            schema_role_weight(ThreadSchema::Brief, Role::BottomLine),
            1.0
        );
    }
}
