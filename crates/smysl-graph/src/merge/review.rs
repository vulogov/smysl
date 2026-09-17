//! What is open for review (1.4).
//!
//! Merge detects disagreements and must not adjudicate them, so somebody has to. This is the
//! list they work from: every contention a store records or implies, and every live `rebuts`
//! edge no contention covers — a rebuttal nobody threaded is a disagreement in waiting, and
//! detection alone would never show it to anyone.
//!
//! An item leaves the list when a resolution names it, or when it stops being a disagreement at
//! all: its rebuttal withdrawn or retracted, or its claim retracted. What the reviewer concluded
//! is recorded by retraction and withdrawal; the resolution records only that they looked.

use std::collections::BTreeSet;

use smysl_core::{
    AgentKind, Contention, ContentionStatus, DetectionKind, RelKind, Relation, ResolutionTarget,
};

use crate::merge::contention::{detect, DetectionContext};
use crate::store::Store;

/// What a review item is about.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ReviewSubject {
    /// A contention the store records, or that detection finds in it.
    Contention(Contention),
    /// A live `rebuts` edge that no open contention covers.
    Rebuttal(Relation),
    /// An edge of a kind the caller says needs confirming, which nobody has confirmed (1.5).
    Unconfirmed(Relation),
}

/// One thing a reviewer may need to look at.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ReviewItem {
    pub subject: ReviewSubject,
    /// Whether it has been dealt with: a resolution naming it, or — for an edge awaiting
    /// confirmation — an attestation from an agent of the accepted kind.
    pub resolved: bool,
}

/// What counts as an item, and as having dealt with one (1.5).
///
/// The default is 1.4's queue: contentions and live rebuttals, closed by a resolution.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ReviewOptions {
    /// What detection needs beyond the store.
    pub detection: DetectionContext,
    /// Edge kinds a tool may propose and a person is expected to confirm — `backs`, or an
    /// extension kind such as `x.code/exercises` for a model linking test evidence to a claim.
    /// An edge of one of these kinds is an item until it is confirmed or withdrawn.
    pub confirm: Vec<RelKind>,
    /// Whose attestation confirms one. `Human` by default: a model attesting its own proposal is
    /// not a review. Reached only when `confirm` names a kind.
    pub confirmed_by: Option<AgentKind>,
}

impl ReviewOptions {
    /// Ask for edges of these kinds to be confirmed by a person.
    pub fn confirming(kinds: impl IntoIterator<Item = RelKind>) -> ReviewOptions {
        ReviewOptions {
            confirm: kinds.into_iter().collect(),
            confirmed_by: Some(AgentKind::Human),
            ..ReviewOptions::default()
        }
    }

    pub fn with_detection(mut self, ctx: DetectionContext) -> ReviewOptions {
        self.detection = ctx;
        self
    }

    /// Accept an attestation from this kind of agent as confirmation.
    pub fn confirmed_by(mut self, kind: AgentKind) -> ReviewOptions {
        self.confirmed_by = Some(kind);
        self
    }
}

impl ReviewItem {
    /// What a resolution of this item names.
    pub fn target(&self) -> ResolutionTarget {
        match &self.subject {
            ReviewSubject::Contention(c) => ResolutionTarget::Contention(c.id.clone()),
            ReviewSubject::Rebuttal(r) | ReviewSubject::Unconfirmed(r) => {
                ResolutionTarget::Relation(r.uid())
            }
        }
    }
}

/// Every review item in a store, resolved or not: contentions by id, then rebuttals by rid.
///
/// Stale contentions are left out, and so are rebuttals that are no longer live or whose claim
/// is `unfounded` — there is nothing left to review. A caller wanting the open queue filters on
/// `resolved`.
pub fn review(store: &Store, ctx: &DetectionContext) -> Vec<ReviewItem> {
    review_with(
        store,
        &ReviewOptions {
            detection: ctx.clone(),
            ..ReviewOptions::default()
        },
    )
}

/// [`review`], with the edges a caller expects a person to confirm (1.5).
///
/// A tool that proposes edges — a model linking a test to the claim it verifies — needs them in
/// the queue until somebody stands behind them, and an attestation on an edge is how the format
/// records who did (§2.4). A withdrawn edge is not an item: it has been dealt with.
pub fn review_with(store: &Store, opts: &ReviewOptions) -> Vec<ReviewItem> {
    let ctx = &opts.detection;
    let mut contentions: Vec<Contention> = Vec::new();
    for c in store
        .contentions()
        .iter()
        .cloned()
        .chain(detect(store, ctx))
    {
        if contentions.iter().any(|k| k.id == c.id) {
            continue;
        }
        contentions.push(c);
    }
    contentions.retain(|c| store.contention_status(c) != ContentionStatus::Stale);
    contentions.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));

    // A rebuttal an unresolved contention already presents is reviewed there, not twice.
    let covered: BTreeSet<(smysl_core::Uid, smysl_core::Uid)> = contentions
        .iter()
        .filter(|c| c.detected.kind == DetectionKind::LiveRebuttal)
        .filter(|c| store.contention_status(c) == ContentionStatus::Open)
        .flat_map(|c| c.positions.iter().map(move |p| (*p, c.over)))
        .collect();

    let mut out: Vec<ReviewItem> = contentions
        .into_iter()
        .map(|c| {
            let resolved = store.contention_status(&c) == ContentionStatus::Resolved;
            ReviewItem {
                subject: ReviewSubject::Contention(c),
                resolved,
            }
        })
        .collect();

    let mut rebuttals: Vec<&Relation> = store
        .relations_of_kind(&RelKind::Rebuts)
        .into_iter()
        .filter(|r| store.is_live_rebuttal(r) && !store.is_unfounded(&r.to))
        .filter(|r| !covered.contains(&(r.from, r.to)))
        .collect();
    rebuttals.sort_by_key(|r| r.uid());
    for r in rebuttals {
        out.push(ReviewItem {
            resolved: store.is_resolved(&ResolutionTarget::Relation(r.uid())),
            subject: ReviewSubject::Rebuttal(r.clone()),
        });
    }

    // Edges a tool proposed and a person is expected to stand behind.
    let mut awaiting: Vec<&Relation> = opts
        .confirm
        .iter()
        .filter(|k| **k != RelKind::Rebuts)
        .flat_map(|k| store.relations_of_kind(k))
        .filter(|r| store.contains_uid(&r.from) && store.contains_uid(&r.to))
        .collect();
    awaiting.sort_by_key(|r| r.uid());
    awaiting.dedup_by_key(|r| r.uid());
    for r in awaiting {
        let confirmed = opts
            .confirmed_by
            .is_some_and(|kind| r.attestations.iter().any(|a| a.agent.kind() == kind))
            || store.is_resolved(&ResolutionTarget::Relation(r.uid()));
        out.push(ReviewItem {
            resolved: confirmed,
            subject: ReviewSubject::Unconfirmed(r.clone()),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{
        canonical_uid, AgentId, Hlc, KernelType, Record, Resolution, Role, Status, Step, Thread,
        ThreadId, ThreadSchema, UnitCoreBuilder, Withdrawal,
    };

    fn reviewer() -> AgentId {
        AgentId::new("human:reviewer").unwrap()
    }

    fn claim(gist: &str) -> smysl_core::UnitCore {
        UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
            .build()
            .unwrap()
    }

    fn base() -> (Vec<Record>, smysl_core::Uid, smysl_core::Uid, Relation) {
        let c = claim("the pool saturated");
        let r = claim("the pool never exceeded half its size");
        let (uc, ur) = (canonical_uid(&c), canonical_uid(&r));
        let edge = Relation::new(RelKind::Rebuts, ur, uc);
        (
            vec![
                Record::Unit(c),
                Record::Unit(r),
                Record::Relation(edge.clone()),
            ],
            uc,
            ur,
            edge,
        )
    }

    fn items(records: Vec<Record>) -> Vec<ReviewItem> {
        review(&Store::from_records(records), &DetectionContext::default())
    }

    #[test]
    fn an_unthreaded_rebuttal_is_open_until_resolved_or_withdrawn() {
        let (records, _, _, edge) = base();
        let open = items(records.clone());
        assert_eq!(open.len(), 1);
        assert!(!open[0].resolved);
        assert_eq!(open[0].target(), ResolutionTarget::Relation(edge.uid()));

        let mut resolved = records.clone();
        resolved.push(Record::Resolution(Resolution::new(
            ResolutionTarget::Relation(edge.uid()),
            reviewer(),
            Hlc::new(1, 0, reviewer()),
        )));
        let after = items(resolved);
        assert_eq!(after.len(), 1);
        assert!(after[0].resolved);

        let mut withdrawn = records;
        withdrawn.push(Record::Withdrawal(Withdrawal::new(
            edge.uid(),
            reviewer(),
            Hlc::new(1, 0, reviewer()),
        )));
        assert!(items(withdrawn).is_empty(), "nothing left to review");
    }

    /// An edge a tool proposed is an item until a person stands behind it, and confirming is an
    /// attestation on the edge rather than a resolution (1.5). The model's own attestation is not
    /// confirmation: a proposal is not a review of itself.
    #[test]
    fn an_edge_awaiting_confirmation_is_an_item_until_a_person_attests_it() {
        use smysl_core::{Attestation, Op, Rung};
        let (records, uc, ur, _) = base();
        let backs = Relation::new(RelKind::Backs, ur, uc);
        let mut with_edge = records.clone();
        with_edge.push(Record::Relation(backs.clone()));
        let opts = || ReviewOptions::confirming([RelKind::Backs]);
        let items = |recs: Vec<Record>| review_with(&Store::from_records(recs), &opts());

        let found = items(with_edge.clone());
        let edge_item = found
            .iter()
            .find(|i| matches!(&i.subject, ReviewSubject::Unconfirmed(r) if r.uid() == backs.uid()))
            .expect("the backs edge is an item");
        assert!(!edge_item.resolved);

        // Without `confirm` it is not an item at all: 1.4's queue is unchanged.
        assert!(!review(
            &Store::from_records(with_edge.clone()),
            &DetectionContext::default()
        )
        .iter()
        .any(|i| matches!(i.subject, ReviewSubject::Unconfirmed(_))));

        let attest = |agent: &str| {
            Record::Attestation(Attestation::new(
                backs.uid(),
                AgentId::new(agent).unwrap(),
                Op::Imported,
                Rung::Model,
                Hlc::new(1, 0, AgentId::new(agent).unwrap()),
            ))
        };
        let mut by_model = with_edge.clone();
        by_model.push(attest("model:linker"));
        assert!(
            !items(by_model.clone())
                .iter()
                .any(|i| matches!(&i.subject, ReviewSubject::Unconfirmed(_)) && i.resolved),
            "a model confirmed its own proposal"
        );

        let mut by_person = by_model;
        by_person.push(attest("human:reviewer"));
        assert!(items(by_person)
            .iter()
            .any(|i| matches!(&i.subject, ReviewSubject::Unconfirmed(_)) && i.resolved));

        // Withdrawing it deals with it too: the edge is no longer followed.
        let mut withdrawn = with_edge;
        withdrawn.push(Record::Withdrawal(Withdrawal::new(
            backs.uid(),
            reviewer(),
            Hlc::new(2, 0, reviewer()),
        )));
        assert!(!items(withdrawn)
            .iter()
            .any(|i| matches!(i.subject, ReviewSubject::Unconfirmed(_))));
    }

    /// A threaded rebuttal is one item — the contention — not two.
    #[test]
    fn a_threaded_rebuttal_is_reviewed_as_its_contention_only() {
        let (mut records, uc, ur, _) = base();
        records.push(Record::Thread(
            Thread::new(
                ThreadId::new("t/brief").unwrap(),
                ThreadSchema::Brief,
                reviewer(),
                "the incident",
                Hlc::new(1, 0, reviewer()),
            )
            .with_steps([Step::new(Role::BottomLine, uc), Step::new(Role::Risk, ur)]),
        ));
        let found = items(records);
        assert_eq!(found.len(), 1);
        assert!(matches!(found[0].subject, ReviewSubject::Contention(_)));
    }
}
