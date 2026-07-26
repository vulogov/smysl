//! Retraction, effective status, and the blast radius (§5.3).
//!
//! `supersedes` and `retracts` say different things and it matters which you mean:
//!
//! | | `supersedes` | `retracts` |
//! |---|---|---|
//! | Meaning | "a better version" | "should not be believed" |
//! | The target | stays valid in historical threads | is invalidated |
//! | Dependents | rebind to the successor | are orphaned (`SMY-E050`) |
//! | Status | unchanged | effective status becomes `unfounded` |
//!
//! Under `strict` a retraction is transitive over `grounds`, and its blast radius **MUST**
//! be computable in advance - `retract --dry-run` exists so nobody discovers what a
//! retraction reaches by performing it.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::{Code, Diagnostic, Report};
use smysl_core::{AgentId, RelKind, Status, Uid};

use crate::merge::policy::{RetractionAuthority, RetractionPolicy};
use crate::store::Store;

/// What a store's units are worth once retractions are taken into account.
///
/// Declared status is what an agent wrote; effective status is what survives. They differ
/// only where something has been retracted, which is why the declared value stays in the
/// unit and this stays derived.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectiveStatus {
    map: BTreeMap<Uid, Status>,
    retracted: BTreeSet<Uid>,
    orphaned: BTreeSet<Uid>,
}

impl EffectiveStatus {
    /// The status a consumer should read for this unit.
    pub fn get(&self, uid: &Uid) -> Option<Status> {
        self.map.get(uid).copied()
    }

    /// Whether this unit was retracted directly.
    pub fn is_retracted(&self, uid: &Uid) -> bool {
        self.retracted.contains(uid)
    }

    /// Whether every one of this unit's grounds has been retracted out from under it.
    pub fn is_orphaned(&self, uid: &Uid) -> bool {
        self.orphaned.contains(uid)
    }

    pub fn retracted(&self) -> impl Iterator<Item = &Uid> {
        self.retracted.iter()
    }

    pub fn orphaned(&self) -> impl Iterator<Item = &Uid> {
        self.orphaned.iter()
    }

    /// Everything a retraction reached: the targets and the units left unsupported.
    pub fn blast_radius(&self) -> BTreeSet<Uid> {
        self.retracted.union(&self.orphaned).copied().collect()
    }
}

/// Compute effective status under a policy.
///
/// A fixpoint over `grounds`, and therefore order-independent - which is what lets merge
/// run it as its last step without breaking rule U.
pub fn effective_status(store: &Store, policy: RetractionPolicy) -> EffectiveStatus {
    let mut out = EffectiveStatus::default();
    for (uid, unit) in store.units() {
        out.map.insert(*uid, unit.core.status);
    }

    if !policy.affects_status() {
        return out;
    }

    for rel in store.relations_of_kind(&RelKind::Retracts) {
        if store.contains_uid(&rel.to) {
            out.retracted.insert(rel.to);
            out.map.insert(rel.to, Status::Unfounded);
        }
    }

    if !policy.is_transitive() {
        return out;
    }

    // A unit whose support has all become unfounded is unfounded too. Iterating to a
    // fixpoint rather than walking the graph once means the result does not depend on
    // which order the units were visited in.
    loop {
        let mut changed = false;
        for (uid, unit) in store.units() {
            if unit.core.grounds.is_empty() || out.map.get(uid) == Some(&Status::Unfounded) {
                continue;
            }
            let present: Vec<&Uid> = unit
                .core
                .grounds
                .iter()
                .filter(|g| store.contains_uid(g))
                .collect();
            if present.is_empty() {
                continue;
            }
            let all_gone = present
                .iter()
                .all(|g| out.map.get(*g) == Some(&Status::Unfounded));
            if all_gone {
                out.map.insert(*uid, Status::Unfounded);
                out.orphaned.insert(*uid);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    out
}

/// Report what a retraction did (§5.3).
pub fn report_retractions(store: &Store, policy: RetractionPolicy, report: &mut Report) {
    let eff = effective_status(store, policy);

    if policy == RetractionPolicy::Advisory {
        for uid in eff.retracted() {
            report.push(
                Diagnostic::on(Code::W052, *uid)
                    .with_message("retracted, but retained under the advisory policy"),
            );
        }
    }

    for uid in eff.orphaned() {
        report.push(
            Diagnostic::on(Code::E050, *uid)
                .with_message("every ground of this unit has been retracted")
                .with_suggestion("retract this unit too, or re-ground it on something surviving"),
        );
    }
}

/// What retracting a unit would do, computed without doing it (§23.1 `--dry-run`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetractionPlan {
    pub target: Uid,
    /// The retraction itself plus everything it orphans, in ascending uid order.
    pub blast_radius: Vec<Uid>,
    /// Units left with no surviving support.
    pub orphaned: Vec<Uid>,
    /// Whether the requesting agents satisfy the authority rule.
    pub authorised: bool,
    /// Why not, when they do not.
    pub refusal: Option<String>,
}

impl RetractionPlan {
    pub fn is_empty(&self) -> bool {
        self.blast_radius.is_empty()
    }
}

/// Plan a retraction of `target` by `agents`.
///
/// Nothing is mutated. The blast radius this returns is exactly what applying the
/// retraction produces - a property the SM-P6 gate asserts directly, because a dry run
/// that disagrees with the real thing is worse than no dry run.
pub fn plan_retraction(
    store: &Store,
    target: Uid,
    agents: &[AgentId],
    policy: RetractionPolicy,
    authority: RetractionAuthority,
) -> RetractionPlan {
    let mut refusal = None;
    let distinct: BTreeSet<&AgentId> = agents.iter().collect();

    if (distinct.len() as u32) < authority.required_agents() {
        refusal = Some(format!(
            "{} requires {} distinct agents, got {}",
            authority,
            authority.required_agents(),
            distinct.len()
        ));
    } else if authority.requires_origin() {
        let attestors: BTreeSet<&AgentId> = store
            .get(&target)
            .map(|u| u.attestations.iter().map(|a| &a.agent).collect())
            .unwrap_or_default();
        if !distinct.iter().any(|a| attestors.contains(*a)) {
            refusal = Some(format!(
                "origin authority: none of the {} requesting agent(s) attested this unit",
                distinct.len()
            ));
        }
    }

    // Simulate: what would effective status be with this retraction in place?
    let mut simulated = store.clone();
    let relation = smysl_core::Relation::new(RelKind::Retracts, target, target);
    let _ = simulated.append(&[smysl_core::Record::Relation(relation)]);
    let eff = effective_status(&simulated, policy);

    let orphaned: Vec<Uid> = eff.orphaned().copied().collect();
    let mut blast_radius: Vec<Uid> = eff.blast_radius().into_iter().collect();
    blast_radius.sort();

    RetractionPlan {
        target,
        blast_radius,
        orphaned,
        authorised: refusal.is_none(),
        refusal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{
        canonical_uid, Attestation, Hlc, KernelType, Op, Record, Relation, Rung, SourceKind,
        SourceRef, UnitCore, UnitCoreBuilder,
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

    fn grounded(gist: &str, grounds: Vec<Uid>) -> UnitCore {
        UnitCoreBuilder::new(KernelType::Claim, gist, Status::Derived)
            .grounds(grounds)
            .build()
            .unwrap()
    }

    /// evidence <- claim <- finding, with a retraction of the evidence.
    fn chain() -> (Store, Uid, Uid, Uid) {
        let e = evidence("a measurement");
        let ue = canonical_uid(&e);
        let c = grounded("a claim", vec![ue]);
        let uc = canonical_uid(&c);
        let f = grounded("a finding", vec![uc]);
        let uf = canonical_uid(&f);
        let store = Store::from_records(vec![Record::Unit(e), Record::Unit(c), Record::Unit(f)]);
        (store, ue, uc, uf)
    }

    fn with_retraction(store: &Store, target: Uid) -> Store {
        let mut s = store.clone();
        s.append(&[Record::Relation(Relation::new(
            RelKind::Retracts,
            target,
            target,
        ))])
        .unwrap();
        s
    }

    #[test]
    fn without_a_retraction_effective_status_is_declared_status() {
        let (store, ue, uc, _) = chain();
        let eff = effective_status(&store, RetractionPolicy::Strict);
        assert_eq!(eff.get(&ue), Some(Status::Measured));
        assert_eq!(eff.get(&uc), Some(Status::Derived));
        assert!(eff.blast_radius().is_empty());
    }

    #[test]
    fn a_retracted_unit_reads_as_unfounded() {
        let (store, ue, _, _) = chain();
        let eff = effective_status(&with_retraction(&store, ue), RetractionPolicy::Strict);
        assert_eq!(eff.get(&ue), Some(Status::Unfounded));
        assert!(eff.is_retracted(&ue));
    }

    /// Strict retraction is transitive: a claim resting only on retracted evidence has
    /// nothing left holding it up.
    #[test]
    fn strict_retraction_propagates_along_grounds() {
        let (store, ue, uc, uf) = chain();
        let eff = effective_status(&with_retraction(&store, ue), RetractionPolicy::Strict);
        assert_eq!(eff.get(&uc), Some(Status::Unfounded));
        assert_eq!(eff.get(&uf), Some(Status::Unfounded), "and onward");
        assert!(eff.is_orphaned(&uc));
        assert!(eff.is_orphaned(&uf));
        assert!(
            !eff.is_orphaned(&ue),
            "the target is retracted, not orphaned"
        );
    }

    #[test]
    fn advisory_retraction_does_not_propagate() {
        let (store, ue, uc, _) = chain();
        let eff = effective_status(&with_retraction(&store, ue), RetractionPolicy::Advisory);
        assert_eq!(eff.get(&ue), Some(Status::Unfounded));
        assert_eq!(
            eff.get(&uc),
            Some(Status::Derived),
            "dependents are untouched"
        );
        assert!(eff.orphaned().count() == 0);
    }

    #[test]
    fn ignore_leaves_everything_alone() {
        let (store, ue, uc, _) = chain();
        let eff = effective_status(&with_retraction(&store, ue), RetractionPolicy::Ignore);
        assert_eq!(eff.get(&ue), Some(Status::Measured));
        assert_eq!(eff.get(&uc), Some(Status::Derived));
        assert!(eff.blast_radius().is_empty());
    }

    /// A unit with any surviving ground keeps standing. Retraction removes support; it
    /// does not remove units that still have some.
    #[test]
    fn a_unit_with_a_surviving_ground_is_not_orphaned() {
        let a = evidence("measurement a");
        let b = evidence("measurement b");
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
        let c = grounded("resting on both", vec![ua, ub]);
        let uc = canonical_uid(&c);
        let store = Store::from_records(vec![Record::Unit(a), Record::Unit(b), Record::Unit(c)]);

        let eff = effective_status(&with_retraction(&store, ua), RetractionPolicy::Strict);
        assert_eq!(eff.get(&uc), Some(Status::Derived));
        assert!(!eff.is_orphaned(&uc));
    }

    #[test]
    fn a_unit_with_no_grounds_is_never_orphaned() {
        let a = UnitCoreBuilder::new(KernelType::Claim, "standalone", Status::Speculative)
            .build()
            .unwrap();
        let ua = canonical_uid(&a);
        let e = evidence("unrelated");
        let ue = canonical_uid(&e);
        let store = Store::from_records(vec![Record::Unit(a), Record::Unit(e)]);
        let eff = effective_status(&with_retraction(&store, ue), RetractionPolicy::Strict);
        assert_eq!(eff.get(&ua), Some(Status::Speculative));
    }

    #[test]
    fn orphaning_is_reported_as_e050() {
        let (store, ue, _, _) = chain();
        let mut r = Report::new();
        report_retractions(
            &with_retraction(&store, ue),
            RetractionPolicy::Strict,
            &mut r,
        );
        assert_eq!(r.count(Code::E050), 2, "the claim and the finding");
        assert!(r.iter().next().unwrap().suggestion.is_some());
    }

    #[test]
    fn advisory_retention_is_reported_as_w052() {
        let (store, ue, _, _) = chain();
        let mut r = Report::new();
        report_retractions(
            &with_retraction(&store, ue),
            RetractionPolicy::Advisory,
            &mut r,
        );
        assert_eq!(r.count(Code::W052), 1);
        assert_eq!(r.count(Code::E050), 0);
        assert!(r.is_clean(), "advisory retraction must not fail a check");
    }

    /// The gate: a dry run reports exactly what applying it produces.
    #[test]
    fn the_plan_matches_what_applying_the_retraction_does() {
        let (store, ue, _, _) = chain();
        let plan = plan_retraction(
            &store,
            ue,
            &[agent("human:v")],
            RetractionPolicy::Strict,
            RetractionAuthority::Any,
        );

        let applied = effective_status(&with_retraction(&store, ue), RetractionPolicy::Strict);
        let actual: Vec<Uid> = applied.blast_radius().into_iter().collect();
        assert_eq!(plan.blast_radius, actual);
        assert_eq!(plan.orphaned.len(), 2);
        assert!(plan.blast_radius.contains(&ue));
    }

    #[test]
    fn planning_does_not_mutate_the_store() {
        let (store, ue, uc, _) = chain();
        let before = store.len();
        let _ = plan_retraction(
            &store,
            ue,
            &[agent("human:v")],
            RetractionPolicy::Strict,
            RetractionAuthority::Any,
        );
        assert_eq!(store.len(), before);
        assert_eq!(
            effective_status(&store, RetractionPolicy::Strict).get(&uc),
            Some(Status::Derived)
        );
    }

    /// The anti-censorship property: a stranger cannot retract work they had no hand in.
    #[test]
    fn origin_authority_refuses_a_stranger() {
        let e = evidence("a measurement");
        let ue = canonical_uid(&e);
        let owner = agent("human:vladimir");
        let store = Store::from_records(vec![
            Record::Unit(e),
            Record::Attestation(Attestation::new(
                ue,
                owner.clone(),
                Op::Authored,
                Rung::Document,
                Hlc::zero(owner.clone()),
            )),
        ]);

        let stranger = plan_retraction(
            &store,
            ue,
            &[agent("model:openai/gpt")],
            RetractionPolicy::Strict,
            RetractionAuthority::Origin,
        );
        assert!(!stranger.authorised);
        assert!(stranger.refusal.unwrap().contains("origin"));

        let author = plan_retraction(
            &store,
            ue,
            &[owner],
            RetractionPolicy::Strict,
            RetractionAuthority::Origin,
        );
        assert!(author.authorised);
    }

    #[test]
    fn any_authority_accepts_anyone() {
        let (store, ue, _, _) = chain();
        let plan = plan_retraction(
            &store,
            ue,
            &[agent("model:openai/gpt")],
            RetractionPolicy::Strict,
            RetractionAuthority::Any,
        );
        assert!(plan.authorised);
    }

    #[test]
    fn a_quorum_counts_distinct_agents() {
        let (store, ue, _, _) = chain();
        let one = agent("human:a");
        let two = agent("human:b");

        let short = plan_retraction(
            &store,
            ue,
            &[one.clone(), one.clone()],
            RetractionPolicy::Strict,
            RetractionAuthority::Quorum(2),
        );
        assert!(!short.authorised, "the same agent twice is one agent");

        let met = plan_retraction(
            &store,
            ue,
            &[one, two],
            RetractionPolicy::Strict,
            RetractionAuthority::Quorum(2),
        );
        assert!(met.authorised);
    }

    /// Even when refused, the plan still reports what *would* happen - which is the
    /// information a caller needs to argue for the retraction.
    #[test]
    fn a_refused_plan_still_reports_its_blast_radius() {
        let e = evidence("a measurement");
        let ue = canonical_uid(&e);
        let c = grounded("a claim", vec![ue]);
        let store = Store::from_records(vec![Record::Unit(e), Record::Unit(c)]);
        let plan = plan_retraction(
            &store,
            ue,
            &[agent("model:openai/gpt")],
            RetractionPolicy::Strict,
            RetractionAuthority::Origin,
        );
        assert!(!plan.authorised);
        assert_eq!(plan.blast_radius.len(), 2);
    }

    #[test]
    fn effective_status_is_order_independent() {
        let (store, ue, _, _) = chain();
        let retracted = with_retraction(&store, ue);
        let a = effective_status(&retracted, RetractionPolicy::Strict);
        let b = effective_status(&retracted, RetractionPolicy::Strict);
        assert_eq!(a, b);
    }
}
