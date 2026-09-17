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
    Contention, ContentionStatus, DetectionKind, RelKind, Relation, ResolutionTarget,
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
}

/// One thing a reviewer may need to look at.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ReviewItem {
    pub subject: ReviewSubject,
    /// Whether a resolution already names it.
    pub resolved: bool,
}

impl ReviewItem {
    /// What a resolution of this item names.
    pub fn target(&self) -> ResolutionTarget {
        match &self.subject {
            ReviewSubject::Contention(c) => ResolutionTarget::Contention(c.id.clone()),
            ReviewSubject::Rebuttal(r) => ResolutionTarget::Relation(r.uid()),
        }
    }
}

/// Every review item in a store, resolved or not: contentions by id, then rebuttals by rid.
///
/// Stale contentions are left out, and so are rebuttals that are no longer live or whose claim
/// is `unfounded` — there is nothing left to review. A caller wanting the open queue filters on
/// `resolved`.
pub fn review(store: &Store, ctx: &DetectionContext) -> Vec<ReviewItem> {
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
