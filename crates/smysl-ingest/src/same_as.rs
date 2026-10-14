//! Staging same-as proposals as edges (SMYSL-2.4 §3.6, TX-P7 step 2).
//!
//! The engines are in [`smysl_text::proposition`] and write nothing. This is the one place a
//! proposal becomes a record, and it is here rather than there for the reason rule S exists:
//! staging is a batch checked against a store and held for confirmation, and a pure engine
//! that could append to a store would be a pure engine that could not be trusted to be pure.
//!
//! # What a staged proposal carries
//!
//! One `x.text/same-as` [`Relation`] per proposal and one
//! [`Attestation`](smysl_core::types::provenance::Attestation) per relation, at rung `computed`
//! by the agent `tool:smysl-same-as-<engine>`
//! ([`Engine::agent`](smysl_text::proposition::Engine::agent)). The rung is the
//! claim: a computation said so, nothing signed it, and A-9's `unverified` is exactly right.
//! That is also what makes `attested:2` (§3.6's default for corpus measures) answerable —
//! two engines proposing one pair are two agents on one rid, and one engine proposing it twice
//! is still one.
//!
//! # Edges the store already holds are not staged again
//!
//! A relation's rid is derived from its kind and its endpoints, so re-running an engine over a
//! store it has already been run over produces the *same* records. Appending them would be
//! harmless and would also be a lie about what the run found, so they are skipped and counted:
//! [`already`](crate::same_as::Proposals::already) is how many proposals the store already
//! agreed with, which is the number that says whether a rerun found anything new.
//!
//! A **withdrawn** edge is not skipped. Withdrawal is a judgement that the two units are not
//! the same proposition, and an engine proposing it again is a real disagreement between a
//! person and a computation — staging it is how that disagreement becomes visible, and
//! `Store::is_withdrawn` keeps the withdrawn edge out of the classes either way.
//!
//! # The declaration, for the reason the pivot pass found it
//!
//! `x.text/same-as` is `x.text/v1`'s, and a store holding one of these edges without the
//! declaration reports `SMY-W013` on every check of it afterwards — "correct and
//! unactionable". The declaration is staged with the batch, once, and only when the store does
//! not already carry it.

use std::collections::BTreeSet;

use smysl_core::ids::Uid;
use smysl_core::types::provenance::{Hlc, Rung};
use smysl_core::types::RelKind;
use smysl_core::Relation;
use smysl_graph::Store;
use smysl_text::proposition::{Engine, Proposal, SAME_AS};

use crate::stage::{self, Attest, Staged};

/// What an engine found, and what of it was new.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Proposals {
    /// The engine's id, as `same-as propose --engine` spells it.
    pub engine: String,
    /// Every proposal the engine made, in uid order.
    pub proposals: Vec<Proposal>,
    /// How many of them the store already holds as a live edge.
    pub already: usize,
    /// The batch for the rest, or `None` when there was nothing new to stage.
    pub staged: Option<Staged>,
}

impl Proposals {
    /// How many edges this run would add.
    pub fn new_edges(&self) -> usize {
        self.staged.as_ref().map_or(0, |s| s.relations.len())
    }
}

/// Run an engine over a store and stage what it proposes.
///
/// `now` is supplied rather than read, for guarantee A2: a replayed run produces the same
/// attestations, and an engine is the kind of thing that gets replayed.
pub fn propose(
    store: &Store,
    engine: &dyn Engine,
    scope: Option<&BTreeSet<Uid>>,
    now: Hlc,
) -> Proposals {
    let proposals = engine.propose(store, scope);

    let held: BTreeSet<Uid> = store
        .relations()
        .filter(|r| matches!(&r.kind, RelKind::Extension(n) if n == SAME_AS))
        .filter(|r| !store.is_withdrawn(r))
        .map(|r| r.uid())
        .collect();

    let mut relations: Vec<Relation> = Vec::new();
    let mut already = 0usize;
    for p in &proposals {
        let rel = Relation::new(RelKind::Extension(SAME_AS.to_string()), p.a, p.b);
        if held.contains(&rel.uid()) {
            already += 1;
            continue;
        }
        relations.push(rel);
    }

    let staged = if relations.is_empty() {
        None
    } else {
        let attest = Attest::new(engine.agent(), Rung::Computed, now);
        let schemas = if declares_text(store) {
            Vec::new()
        } else {
            vec![smysl_core::schemas::text::decl()]
        };
        Some(stage::prepare_attested(
            store,
            Vec::new(),
            relations,
            Default::default(),
            schemas,
            &attest,
        ))
    };

    Proposals {
        engine: engine.id().to_string(),
        proposals,
        already,
        staged,
    }
}

/// Whether the store already carries the `x.text/v1` declaration, at any revision.
fn declares_text(store: &Store) -> bool {
    store.iter().any(|r| {
        matches!(r, smysl_core::Record::SchemaDecl(d)
            if d.id.as_str() == smysl_core::schemas::text::ID)
    })
}
