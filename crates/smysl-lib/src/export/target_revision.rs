//! `--target-revision ID@N` (RFC SMYSL-2.7 §3.11, TX-P12): the selection written as revision `N`
//! of one schema would have written it.
//!
//! Pure, and nothing is written to the library. The rewrite builds a new store and the normal
//! pipeline exports it, which is how `--as-of` works too (§3.10): the layouts, the closure, the
//! formats and the index are not told that the store they read is a rewrite.
//!
//! 1. Each unit's revision is inferred ([`SchemaSet::infer_revision`]) and its chain to `N`
//!    resolved. A path that runs **backward** through a step with no inverse — a `retire`, or a
//!    `map-value` that is not one to one — refuses the export (`SMY-E422`); no path is
//!    `SMY-E470`, and an ill-formed step `SMY-E471`.
//! 2. Units are rewritten in dependency order (`traverse::topo` over the support edges), so a
//!    unit's `deps` and `grounds` are mapped before it is: a cycle among them is `SMY-E492`.
//! 3. A unit whose core is unchanged keeps its uid and its attestations. One that changes is a
//!    new unit, `canonical_uid(core')`, with one new attestation: `Op::Transformed`, the agent
//!    given (`tool:smysl-lib` by default), `parents = {u}`, at the export clock (OQ-63).
//! 4. A relation with a mapped end, or a kind the chain renames, is a new relation with a new
//!    rid, attested the same way with `parents = {old rid}`. Label bindings, thread steps and
//!    view roots are re-pointed.
//! 5. Commitments, datings, withdrawals, resolutions and contentions that name a mapped uid or
//!    rid are dropped and counted (`SMY-W491`): each is a statement about the original, and
//!    re-pointing it would put words in its author's mouth.
//!
//! Two readings of §3.11 are this module's, and recorded in the RFC:
//!
//! - **U is the selection closed along `deps` and `grounds`.** §3.11 rewrites "each `u` in
//!   order, restricted to `U`", and step 13's exit asks that every rewritten core reference
//!   only mapped uids. A selected unit's ground outside the selection would be exported (the
//!   closure takes it) unrewritten and keep its old uid, so the two cannot both hold unless
//!   the rewrite covers what the closure will take.
//! - **The refusal is for backward paths.** §3.11 says any step without an inverse refuses,
//!   and an inverse is what a backward reading needs; a unit older than `N` is read forward,
//!   where a `retire` is simply what revision `N` says (the key reads as absent).

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::schema::{LensChain, LensError, Mapped, SchemaSet, UnitView};
use smysl_core::types::library::DatingTarget;
use smysl_core::types::lifecycle::ResolutionTarget;
use smysl_core::types::{Attestation, Hlc, Op, Record, Relation, Rung, UnitCore};
use smysl_core::{canonical_uid, AgentId, Code, Diagnostic, Report, SchemaId, Uid};
use smysl_graph::{traverse, EdgeSet, Store};

use super::LibError;

/// What `--target-revision` asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TargetRevision {
    pub id: SchemaId,
    pub version: u32,
    /// The agent of the attestations the rewrite adds; `tool:smysl-lib` by default.
    pub agent: AgentId,
    /// `(wall_ms, counter)` of those attestations; derived from the selection when absent.
    pub clock: Option<(u64, u32)>,
}

/// The agent a rewrite attests as when none is given (§3.11).
pub const DEFAULT_AGENT: &str = "tool:smysl-lib";

impl TargetRevision {
    pub fn new(id: SchemaId, version: u32) -> TargetRevision {
        TargetRevision {
            id,
            version,
            agent: AgentId::new(DEFAULT_AGENT).expect("an agent id"),
            clock: None,
        }
    }

    /// `ID@N`, as the flag spells it.
    pub fn parse(spec: &str) -> Result<TargetRevision, LibError> {
        let bad = || {
            LibError::TargetRevision(
                Code::E470,
                format!("`{spec}` is not ID@N, a schema id and a revision"),
            )
        };
        let (id, n) = spec.rsplit_once('@').ok_or_else(bad)?;
        let id = SchemaId::parse(id).map_err(|_| bad())?;
        let n: u32 = n.parse().map_err(|_| bad())?;
        Ok(TargetRevision::new(id, n))
    }

    pub fn agent(mut self, agent: AgentId) -> TargetRevision {
        self.agent = agent;
        self
    }

    pub fn clock(mut self, wall_ms: u64, counter: u32) -> TargetRevision {
        self.clock = Some((wall_ms, counter));
        self
    }

    /// The index's `derived` text (§4.2 key 13).
    pub fn derived(&self) -> String {
        format!("target-revision {}@{}", self.id.as_str(), self.version)
    }
}

/// A rewritten store and what the rewrite did.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Rewrite {
    pub store: Store,
    /// Each unit of U to what it became, in the order they were rewritten (dependency order).
    pub pairs: Vec<(Uid, Uid)>,
    /// Old rid to new rid, for every relation rewritten.
    pub relations: BTreeMap<Uid, Uid>,
    /// Records dropped because they named an original (`SMY-W491`).
    pub dropped: usize,
    /// Units holding a name or value the chain could not carry, kept as written (`SMY-W420`).
    pub unmappable: usize,
    /// The hash of the chain from `N` to the current revision (§4.2 key 5).
    pub chain: [u8; 32],
    pub report: Report,
}

impl Rewrite {
    pub fn map(&self) -> BTreeMap<Uid, Uid> {
        self.pairs.iter().copied().collect()
    }
}

fn lens_error(e: LensError) -> LibError {
    LibError::TargetRevision(e.code(), e.to_string())
}

/// Rewrite `sel`, closed along `deps` and `grounds`, at revision `t.version` of `t.id`.
pub fn rewrite(
    store: &Store,
    sel: &BTreeSet<Uid>,
    t: &TargetRevision,
) -> Result<Rewrite, LibError> {
    let mut report = Report::new();
    let set = SchemaSet::from_decls(store.schema_decls(), &mut report);
    let id = &t.id;
    if set.revisions(id).is_empty() {
        return Err(LibError::TargetRevision(
            Code::E470,
            format!("the store declares no revision of {}", id.as_str()),
        ));
    }
    let Some(current) = set.current(id).map(|r| r.version) else {
        return Err(LibError::TargetRevision(
            Code::W429,
            format!(
                "{} is forked and has no current revision to rewrite from",
                id.as_str()
            ),
        ));
    };
    if set.revision(id, t.version).is_none() {
        return Err(LibError::TargetRevision(
            Code::E470,
            format!(
                "the store declares no revision {} of {}",
                t.version,
                id.as_str()
            ),
        ));
    }
    let to_current = set
        .chain(id, t.version.min(current), t.version.max(current))
        .map_err(lens_error)?;

    // U: the selection closed along the support edges.
    let mut u: BTreeSet<Uid> = BTreeSet::new();
    let mut stack: Vec<Uid> = sel.iter().copied().collect();
    while let Some(x) = stack.pop() {
        if !u.insert(x) {
            continue;
        }
        if let Some(unit) = store.get(&x) {
            stack.extend(unit.core.deps.iter().chain(&unit.core.grounds).copied());
        }
    }
    u.retain(|x| store.contains_uid(x));

    let adj = store.adjacency();
    let order = traverse::topo(adj, &EdgeSet::support());
    if let Some(x) = order
        .cyclic
        .iter()
        .filter_map(|n| adj.uid(*n))
        .find(|x| u.contains(x))
    {
        return Err(LibError::TargetRevision(
            Code::E492,
            format!(
                "{} sits on a cycle of deps and grounds, so the units cannot be rewritten in \
                 dependency order",
                x.short()
            ),
        ));
    }

    // The export clock (OQ-63): after everything the rewrite reads.
    let ts = match t.clock {
        Some((w, c)) => Hlc::new(w, c, t.agent.clone()),
        None => {
            let (w, c) = u
                .iter()
                .flat_map(|x| store.attestations_of(x))
                .map(|a| (a.ts.wall_ms, a.ts.counter))
                .max()
                .unwrap_or((0, 0));
            Hlc::new(w, c.saturating_add(1), t.agent.clone())
        }
    };
    let attest = |subject: Uid, parent: Uid| {
        let hop = store
            .attestations_of(&parent)
            .iter()
            .map(|a| a.hop)
            .max()
            .map_or(1, |h| h.saturating_add(1));
        Attestation::new(
            subject,
            t.agent.clone(),
            Op::Transformed,
            Rung::Computed,
            ts.clone(),
        )
        .at_hop(hop)
        .with_parents([parent].into_iter().collect())
    };

    // Each unit, dependencies first.
    let mut chains: BTreeMap<u32, LensChain> = BTreeMap::new();
    let mut map: BTreeMap<Uid, Uid> = BTreeMap::new();
    let mut pairs = Vec::new();
    let mut rewritten: Vec<(UnitCore, Attestation)> = Vec::new();
    let mut unmappable = 0;
    for x in order.order.iter().filter_map(|n| adj.uid(*n)) {
        if !u.contains(x) {
            continue;
        }
        let core = &store.get(x).expect("U is held").core;
        let mut next = core.clone();
        if set.governing(&core.schema) == Some(id) {
            let rev = set.infer_revision(core).map_or(current, |g| g.hi);
            if rev != t.version {
                let chain = match chains.get(&rev) {
                    Some(c) => c.clone(),
                    None => {
                        let c = set
                            .chain(id, rev.min(t.version), rev.max(t.version))
                            .map_err(lens_error)?;
                        chains.insert(rev, c.clone());
                        c
                    }
                };
                let before = UnitView::of(core);
                let mut view = before.clone();
                let mapped = if rev > t.version {
                    chain.backward(&mut view).map_err(lens_error)?
                } else {
                    chain.forward(&mut view)
                };
                if !matches!(mapped, Mapped::Whole) {
                    unmappable += 1;
                }
                if view != before {
                    next.schema = view.ty.clone();
                    next.payload = view.payload_bytes();
                }
            }
        }
        let remap = |set: &BTreeSet<Uid>| -> BTreeSet<Uid> {
            set.iter().map(|d| *map.get(d).unwrap_or(d)).collect()
        };
        next.deps = remap(&core.deps);
        next.grounds = remap(&core.grounds);
        if &next == core {
            map.insert(*x, *x);
            pairs.push((*x, *x));
        } else {
            let n = canonical_uid(&next);
            map.insert(*x, n);
            pairs.push((*x, n));
            rewritten.push((next, attest(n, *x)));
        }
    }
    let moved = |x: &Uid| map.get(x).is_some_and(|n| n != x);
    let point = |x: &Uid| *map.get(x).unwrap_or(x);

    // Relation kinds the chain renames, read as revision N names them. Units at revisions
    // other than current are a matter for their own chain; an edge kind has no revision of its
    // own, so the current one is the reading.
    let kind_at_target = |k: &smysl_core::types::RelKind| {
        if t.version < current {
            to_current.relation_backward(k)
        } else {
            to_current.relation_forward(k)
        }
    };

    let mut records: Vec<Record> = Vec::new();
    let mut relations: BTreeMap<Uid, Uid> = BTreeMap::new();
    let mut dropped = 0;
    let rid_moved = |r: &Uid, relations: &BTreeMap<Uid, Uid>| relations.contains_key(r);
    // Relations first, so that a withdrawal or resolution naming one can be judged.
    for r in store.iter() {
        if let Record::Relation(rel) = r {
            let mut next =
                Relation::new(kind_at_target(&rel.kind), point(&rel.from), point(&rel.to));
            next.weight = rel.weight;
            next.note = rel.note.as_ref().map(&point);
            next.extra = rel.extra.clone();
            let old = rel.uid();
            if next.kind == rel.kind
                && next.from == rel.from
                && next.to == rel.to
                && next.note == rel.note
            {
                records.push(r.clone());
                continue;
            }
            // The attestation is a record of its own, by rid, as the log holds every edge's:
            // an inline one is the store's in-memory join, and the wire does not carry it.
            let rid = next.uid();
            relations.insert(old, rid);
            records.push(Record::Relation(next));
            records.push(Record::Attestation(attest(rid, old)));
        }
    }
    // Contentions naming an original go, and so do the resolutions of the ones that went:
    // a resolution of nothing is not a record any reader can place.
    let names = |uids: &[&Uid], relations: &BTreeMap<Uid, Uid>| {
        uids.iter().any(|x| moved(x) || rid_moved(x, relations))
    };
    let gone: BTreeSet<_> = store
        .iter()
        .filter_map(|r| match r {
            Record::Contention(c) => {
                let mut n: Vec<&Uid> = vec![&c.over];
                n.extend(c.positions.iter());
                names(&n, &relations).then(|| c.id.clone())
            }
            _ => None,
        })
        .collect();
    for r in store.iter() {
        let names_original = |uids: &[&Uid]| names(uids, &relations);
        match r {
            Record::Relation(_) => {}
            Record::Unit(c) => {
                if !moved(&canonical_uid(c)) {
                    records.push(r.clone());
                }
            }
            Record::Attestation(a) => {
                if !moved(&a.uid) && !relations.contains_key(&a.uid) {
                    records.push(r.clone());
                }
            }
            Record::Thread(th) => {
                let mut th = th.clone();
                for s in &mut th.steps {
                    s.unit = point(&s.unit);
                }
                records.push(Record::Thread(th));
            }
            Record::View(v) => {
                let mut v = v.clone();
                v.roots = v.roots.iter().map(&point).collect();
                records.push(Record::View(v));
            }
            Record::LabelBinding(b) => {
                let mut b = b.clone();
                b.uid = point(&b.uid);
                records.push(Record::LabelBinding(b));
            }
            Record::Commit(c) => {
                let mut n: Vec<&Uid> = vec![&c.unit];
                n.extend(c.note.as_ref());
                if names_original(&n) {
                    dropped += 1;
                } else {
                    records.push(r.clone());
                }
            }
            Record::Dating(d) => {
                let mut n: Vec<&Uid> = Vec::new();
                if let DatingTarget::Unit(x) = &d.target {
                    n.push(x);
                }
                n.extend(d.basis.as_ref());
                if names_original(&n) {
                    dropped += 1;
                } else {
                    records.push(r.clone());
                }
            }
            Record::Withdrawal(w) => {
                let mut n: Vec<&Uid> = vec![&w.relation];
                n.extend(w.reason.as_ref());
                if names_original(&n) {
                    dropped += 1;
                } else {
                    records.push(r.clone());
                }
            }
            Record::Resolution(res) => {
                let mut n: Vec<&Uid> = Vec::new();
                let of_gone = match &res.target {
                    ResolutionTarget::Relation(x) => {
                        n.push(x);
                        false
                    }
                    ResolutionTarget::Contention(c) => gone.contains(c),
                    _ => false,
                };
                n.extend(res.note.as_ref());
                if of_gone || names_original(&n) {
                    dropped += 1;
                } else {
                    records.push(r.clone());
                }
            }
            Record::Contention(c) => {
                let mut n: Vec<&Uid> = vec![&c.over];
                n.extend(c.positions.iter());
                if names_original(&n) {
                    dropped += 1;
                } else {
                    records.push(r.clone());
                }
            }
            _ => records.push(r.clone()),
        }
    }
    for (core, a) in rewritten {
        records.push(Record::Unit(core));
        records.push(Record::Attestation(a));
    }
    let store = Store::from_records(records);
    if dropped > 0 {
        report.push(Diagnostic::new(Code::W491).with_message(format!(
            "{dropped} commitment(s), dating(s), withdrawal(s), resolution(s) or contention(s) \
             named an original and were dropped"
        )));
    }
    if unmappable > 0 {
        report.push(Diagnostic::new(Code::W420).with_message(format!(
            "{unmappable} unit(s) hold a name or value revision {} cannot carry; kept as written",
            t.version
        )));
    }
    Ok(Rewrite {
        store,
        pairs,
        relations,
        dropped,
        unmappable,
        chain: to_current.hash,
        report,
    })
}
