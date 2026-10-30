//! `migrate` (SMYSL-2.5 §3.11, TX-P12 step 6): re-derive a schema's units under a newer
//! revision, deterministically, as a staged batch.
//!
//! A lens is a reading (§3.9) and never writes; `migrate` is the one place a reading becomes
//! records, and it does so the way §3.11 says, never in place and never automatically:
//!
//! - each unit of the schema written at an older revision is read forward through the chain
//!   from its inferred revision to the target, and a new `UnitCore` is built from the lensed
//!   type and payload — everything else (gist, status, grounds, deps, source, lang) as it was;
//! - each new unit is attested `op: Transformed`, its parent the old unit, its recipe the chain
//!   hash, and a `supersedes` edge runs new → old;
//! - the batch goes through [`crate::stage::prepare_declared`], so rule M caps every new unit at
//!   what its grounds support and the batch is staged (rule S), not appended;
//! - a unit the target revision requires a key of, which no `default` on the path supplies,
//!   needs information the unit does not carry: `SMY-E473`, and nothing is staged;
//! - a unit the chain cannot carry (an unusable step, a value outside every map) is left alone
//!   and counted (`SMY-W420`);
//! - commitments and datings of an old unit are **proposed** for its new uid, attributed to the
//!   migrating agent, and staged with the batch for review rather than transferred
//!   (`SMY-W472`, with counts);
//! - references are not touched: `relink` re-points them once the batch is accepted, and the
//!   supersession pass prefers heads meanwhile.

use std::collections::BTreeMap;

use smysl_core::schema::{LensChain, Mapped, SchemaSet, UnitView};
use smysl_core::types::{Commit, Dating, DatingTarget, Record};
use smysl_core::{
    canonical_uid, AgentId, Attestation, Code, Diagnostic, Hlc, Op, RelKind, Relation, Report,
    Rung, SchemaId, Uid, UnitCore,
};
use smysl_graph::Store;

use crate::stage::{Attesting, Staged};

/// What to migrate, and as whom.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MigrateOptions {
    pub schema: SchemaId,
    pub to: u32,
    pub agent: AgentId,
    /// Supplied, never read, so a replayed migration stages the same records.
    pub now: Hlc,
}

impl MigrateOptions {
    pub fn new(schema: SchemaId, to: u32, agent: AgentId, now: Hlc) -> MigrateOptions {
        MigrateOptions {
            schema,
            to,
            agent,
            now,
        }
    }
}

/// A migration, staged.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Migration {
    /// The batch: new units, `supersedes` edges, their attestations, and the proposed
    /// commitments and datings.
    pub staged: Staged,
    /// `(old, new)` for every unit migrated, in old-uid order.
    pub pairs: Vec<(Uid, Uid)>,
    /// Units of the schema already at the target revision, left alone.
    pub current: usize,
    /// Units the chain could not carry, left alone (`SMY-W420`).
    pub unmappable: Vec<Uid>,
    /// Commitments and datings proposed for review (`SMY-W472`).
    pub proposed_commitments: usize,
    pub proposed_datings: usize,
}

/// Why a migration could not be prepared.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MigrateError {
    /// The store does not declare the schema at the target revision (`SMY-E466`'s case: there
    /// is nothing to migrate to), or the id is forked.
    NoTarget(String),
    /// No lens path, or an ill-formed step on it (`SMY-E470`, `SMY-E471`).
    Lens(Diagnostic),
    /// The target requires keys these units lack and no `default` supplies (`SMY-E473`).
    NeedsInformation(Vec<(Uid, Vec<String>)>),
}

impl MigrateError {
    pub fn diagnostic(&self) -> Diagnostic {
        match self {
            MigrateError::NoTarget(why) => Diagnostic::new(Code::E466).with_message(why.clone()),
            MigrateError::Lens(d) => d.clone(),
            MigrateError::NeedsInformation(units) => {
                Diagnostic::new(Code::E473).with_message(format!(
                    "{} unit(s) lack keys the target revision requires and no default on the \
                     path supplies ({}); a model recipe would be needed",
                    units.len(),
                    units
                        .iter()
                        .take(3)
                        .map(|(u, ks)| format!("{} needs {}", u, ks.join(", ")))
                        .collect::<Vec<_>>()
                        .join("; ")
                ))
            }
        }
    }
}

/// Who attests a migrated record: the migrating agent, `op: Transformed` from the unit it
/// replaces, at the `computed` rung, the chain hash as recipe.
struct Transform<'a> {
    agent: &'a AgentId,
    now: &'a Hlc,
    parents: BTreeMap<Uid, (Uid, [u8; 32])>,
    family: [u8; 32],
}

impl Attesting for Transform<'_> {
    fn for_unit(&self, uid: Uid) -> Attestation {
        let mut a = Attestation::new(
            uid,
            self.agent.clone(),
            Op::Transformed,
            Rung::Computed,
            self.now.clone(),
        );
        if let Some((old, chain)) = self.parents.get(&uid) {
            a.parents.insert(*old);
            a = a.with_recipe(*chain, self.family);
        }
        a
    }

    fn for_relation(&self, rid: Uid) -> Attestation {
        Attestation::new(
            rid,
            self.agent.clone(),
            Op::Authored,
            Rung::Computed,
            self.now.clone(),
        )
    }
}

/// The recipe family every migration shares: the tool's, not the chain's.
fn family(id: &SchemaId) -> [u8; 32] {
    let mut h = smysl_core::hash::Rolling::new();
    h.update(b"smysl/migrate/1\0")
        .update(id.as_str().as_bytes());
    h.finish()
}

/// Prepare the migration of every unit of `opts.schema` written below `opts.to`.
pub fn migrate(store: &Store, opts: &MigrateOptions) -> Result<Migration, MigrateError> {
    let mut decoding = Report::new();
    let set = SchemaSet::from_decls(store.schema_decls(), &mut decoding);
    let id = &opts.schema;
    let Some(target) = set.revision(id, opts.to) else {
        return Err(MigrateError::NoTarget(format!(
            "the store does not declare `{id}` at revision {} (or it is forked)",
            opts.to
        )));
    };
    let required: Vec<(String, Option<smysl_core::schema::Cond>)> = target
        .fields
        .typed()
        .map(|t| {
            t.iter()
                .filter(|(_, f)| f.required_from.is_some_and(|r| r <= opts.to))
                .map(|(k, f)| (k.clone(), f.required_when.clone()))
                .collect()
        })
        .unwrap_or_default();

    let mut chains: BTreeMap<u32, LensChain> = BTreeMap::new();
    let mut new_units: Vec<UnitCore> = Vec::new();
    let mut parents: BTreeMap<Uid, (Uid, [u8; 32])> = BTreeMap::new();
    let mut pairs = Vec::new();
    let mut lacking = Vec::new();
    let mut unmappable = Vec::new();
    let mut current = 0;
    for (uid, unit) in store.units() {
        let core = &unit.core;
        if set.governing_unit(core).as_ref() != Some(id) {
            continue;
        }
        let Some(guess) = set.infer_revision(core) else {
            unmappable.push(*uid);
            continue;
        };
        if guess.hi >= opts.to {
            current += 1;
            continue;
        }
        let chain = match chains.get(&guess.hi) {
            Some(c) => c.clone(),
            None => {
                let c = set
                    .chain(id, guess.hi, opts.to)
                    .map_err(|e| MigrateError::Lens(e.diagnostic()))?;
                chains.insert(guess.hi, c.clone());
                c
            }
        };
        let mut view = UnitView::of(core);
        if !matches!(chain.forward(&mut view), Mapped::Whole) {
            unmappable.push(*uid);
            continue;
        }
        let missing: Vec<String> = required
            .iter()
            .filter(|(k, when)| {
                when.as_ref().is_none_or(|c| c.holds(&view.payload))
                    && !view.payload.contains_key(k)
            })
            .map(|(k, _)| k.clone())
            .collect();
        if !missing.is_empty() {
            lacking.push((*uid, missing));
            continue;
        }
        let mut next = core.clone();
        next.schema = view.ty.clone();
        next.payload = view.payload_bytes();
        if &next == core {
            current += 1;
            continue;
        }
        let n = canonical_uid(&next);
        parents.insert(n, (*uid, chain.hash));
        pairs.push((*uid, n));
        new_units.push(next);
    }
    if !lacking.is_empty() {
        return Err(MigrateError::NeedsInformation(lacking));
    }

    let relations: Vec<Relation> = pairs
        .iter()
        .map(|(old, new)| Relation::new(RelKind::Supersedes, *new, *old))
        .collect();
    let attest = Transform {
        agent: &opts.agent,
        now: &opts.now,
        parents,
        family: family(id),
    };
    let mut staged = crate::stage::prepare_attested(
        store,
        new_units,
        relations,
        BTreeMap::new(),
        Vec::new(),
        &attest,
    );

    // Rule M may have moved a new unit's identity; proposals follow the staged uid.
    let staged_uid: BTreeMap<Uid, Uid> = {
        let kept: std::collections::BTreeSet<Uid> =
            staged.units.iter().map(canonical_uid).collect();
        let mut m = BTreeMap::new();
        for (old, new) in &pairs {
            if kept.contains(new) {
                m.insert(*old, *new);
            }
        }
        for w in &staged.weakened {
            if let Some((old, _)) = pairs.iter().find(|(_, n)| *n == w.before) {
                m.insert(*old, w.after);
            }
        }
        m
    };
    let (mut commitments, mut datings) = (0, 0);
    for r in store.iter() {
        match r {
            Record::Commit(c) => {
                if let Some(new) = staged_uid.get(&c.unit) {
                    staged.proposals.push(Record::Commit(Commit::new(
                        *new,
                        c.level,
                        opts.agent.clone(),
                        opts.now.clone(),
                    )));
                    commitments += 1;
                }
            }
            Record::Dating(d) => {
                if let DatingTarget::Unit(u) = &d.target {
                    if let Some(new) = staged_uid.get(u) {
                        let mut p: Dating = d.clone();
                        p.target = DatingTarget::Unit(*new);
                        p.agent = opts.agent.clone();
                        p.ts = opts.now.clone();
                        staged.proposals.push(Record::Dating(p));
                        datings += 1;
                    }
                }
            }
            _ => {}
        }
    }
    if commitments + datings > 0 {
        staged
            .report
            .push(Diagnostic::new(Code::W472).with_message(format!(
            "{commitments} commitment(s) and {datings} dating(s) of migrated units proposed for \
             their new uids, staged for review rather than transferred"
        )));
    }
    if !unmappable.is_empty() {
        staged
            .report
            .push(Diagnostic::new(Code::W420).with_message(format!(
                "{} unit(s) of {id} could not be carried to revision {} and were left as they are",
                unmappable.len(),
                opts.to
            )));
    }
    Ok(Migration {
        staged,
        pairs,
        current,
        unmappable,
        proposed_commitments: commitments,
        proposed_datings: datings,
    })
}
