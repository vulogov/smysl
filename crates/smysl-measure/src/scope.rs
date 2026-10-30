//! A measure's scope, resolved, and what its result is grounded on (SMYSL-2.6 §3.1, §3.4).
//!
//! # The basis is the saved scope query (OQ-55, decided in TX-P8 M1)
//!
//! SMYSL-2.6 §3.1 grounds a measure on a **basis**: not on everything it counted — a measure
//! over 10⁶ units would carry 32 MB of uids, and rule M would cap "41% of these are
//! speculative" at `speculative` — but on a few units that say what was measured. The RFC
//! named the catalog units (`x.text/meta`) of the manifests in scope. **They do not exist.**
//! SMYSL-2.4 keeps a manifest as a record, identified by a `Mid`, and a record is not a unit
//! and cannot be a ground; nothing in the build writes `x.text/meta`. Under the rule as written
//! every measure would be computed and never persisted.
//!
//! OQ-55's own alternative is what this crate does: the scope is an sq query, and TX-P8 step 1
//! made a query that ran into a unit — `x.query/saved`, `cited`, its text, its recipe and the
//! hash of its result set in the payload, and an `x.query/answers` edge to each unit it
//! answered with. [`resolve`] runs the scope and builds that unit; its uid is the basis, the
//! measure's one ground. Rule M holds, because the saved query is `cited`. The question "what
//! was this computed over" is answered by the store, edge by edge, rather than by a digest
//! alone, and two measures over one scope ground on one unit.
//!
//! What it costs is the edges: a scope of 10⁶ units writes 10⁶ answer edges once, which is
//! what saving that query costs anyway, and they sit outside every measure's core.
//!
//! # The digest
//!
//! [`digest`] is `hash_bytes("smysl/measure-scope/1\0" ‖ ids)` over the 32-byte ids of the
//! scope, sorted and deduplicated. It is in the recipe, so a result names what it was computed
//! over even when read without the store it was written to — and it is what a scope built
//! without sq ([`Resolved::of_uids`]) still has when it has no basis.

use std::collections::BTreeSet;

use smysl_core::diag::Diagnostic;
use smysl_core::types::Relation;
use smysl_core::{canonical_uid, Uid, UnitCore};
use smysl_query::{Env, PlanOptions};

/// The domain a scope digest is taken under.
pub const DOMAIN: &[u8] = b"smysl/measure-scope/1\0";

/// The digest of a scope: its ids, sorted and deduplicated, after [`DOMAIN`].
pub fn digest(ids: impl IntoIterator<Item = [u8; 32]>) -> [u8; 32] {
    let sorted: BTreeSet<[u8; 32]> = ids.into_iter().collect();
    let mut bytes = Vec::with_capacity(DOMAIN.len() + 32 * sorted.len());
    bytes.extend_from_slice(DOMAIN);
    for id in &sorted {
        bytes.extend_from_slice(id);
    }
    smysl_core::hash_bytes(&bytes)
}

/// The saved scope query a measure grounds on: the unit and its answer edges, to be appended
/// with the result — and, when the store has none, with the `x.query/v1` declaration, which is
/// the writer's job as it is `q --save`'s.
#[derive(Debug, Clone, PartialEq)]
pub struct Basis {
    pub uid: Uid,
    pub unit: UnitCore,
    pub edges: Vec<Relation>,
}

/// A scope, resolved.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Resolved {
    /// The scope as written.
    pub text: String,
    pub uids: BTreeSet<Uid>,
    /// [`digest`] of `uids`.
    pub digest: [u8; 32],
    /// What a result over this scope is grounded on. `None` for a scope that was not an sq
    /// query, whose result is reported and not persisted (`SMY-W483`).
    pub basis: Option<Basis>,
}

impl Resolved {
    /// A scope given as a set rather than as a query — a harness, a test. It has a digest and
    /// no basis: nothing in a store says what it was.
    pub fn of_uids(text: impl Into<String>, uids: BTreeSet<Uid>) -> Resolved {
        Resolved {
            text: text.into(),
            digest: digest(uids.iter().map(|u| u.to_bytes())),
            uids,
            basis: None,
        }
    }

    /// The basis's uid, if there is one.
    pub fn basis(&self) -> Option<Uid> {
        self.basis.as_ref().map(|b| b.uid)
    }
}

/// Resolve an sq scope over `env`: parse, plan, run, and save it as the basis.
///
/// The scope is a whole query — `from texts where alias = kjv-1769`, or anything else sq
/// accepts — and the units it answers with are the scope. A query with `$parameters` is
/// refused here, unbound, by the planner (`SMY-E460`), since a recipe records the scope as
/// written and a parameter would make two different scopes one recipe.
pub fn resolve(text: &str, env: &Env<'_>, opts: &PlanOptions) -> Result<Resolved, Vec<Diagnostic>> {
    let parsed = smysl_query::parse(text).map_err(|d| vec![d])?;
    let plan = smysl_query::plan(&parsed.query, env, opts)?;
    let run = smysl_query::run(&plan, env, opts)?;
    let (unit, edges) =
        smysl_query::saved::to_records(&parsed.query, &plan, &run, opts).map_err(|d| vec![d])?;
    // An empty scope resolves. Whether a measure has anything to say about nothing is the
    // measure's to decide, by its `min:n` (`SMY-W485`), and not the scope's.
    let uids: BTreeSet<Uid> = run.answer.uids().into_iter().collect();
    Ok(Resolved {
        text: text.to_string(),
        digest: digest(uids.iter().map(|u| u.to_bytes())),
        uids,
        basis: Some(Basis {
            uid: canonical_uid(&unit),
            unit,
            edges,
        }),
    })
}
