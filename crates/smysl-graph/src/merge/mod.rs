//! Rule U - merge as a join-semilattice union (§5.1, §16.5).
//!
//! ```text
//! merge(A, B) = A ∪ B    component-wise over cores, attestations, relations,
//!                        contentions, and thread registers
//! ```
//!
//! Commutative, associative, idempotent. Therefore: no coordination, no vector clocks, no
//! tombstone GC, no causal-delivery requirement. Delivery MAY be out of order, duplicated,
//! or partial, and the result is the same.
//!
//! Units are immutable and content-addressed, so there is no mutable content to conflict
//! over - an edit is a new unit carrying `supersedes`. Rule M is preserved because it is a
//! local predicate and grounds are never removed.
//!
//! A general CRDT would additionally *resolve* concurrent edits. Resolution is the wrong
//! behaviour here: it destroys the disagreement, which is the most valuable signal in a
//! multi-agent corpus. This design keeps convergence and discards resolution.

pub mod contention;
pub mod policy;
pub mod retraction;

use std::collections::BTreeMap;

use smysl_core::diag::Report;
use smysl_core::{Contention, Error, Hlc, Label, MergeError, Record, Uid};

use crate::store::Store;

pub use contention::DetectionContext;
pub use policy::{RetractionAuthority, RetractionPolicy, SupersessionPolicy};
pub use retraction::{effective_status, plan_retraction, EffectiveStatus, RetractionPlan};

/// How to merge.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct MergeOptions {
    pub supersession: SupersessionPolicy,
    pub retraction: RetractionPolicy,
    pub authority: RetractionAuthority,
    /// Label bindings per source, for contention detection (c).
    pub labels: Vec<BTreeMap<Label, Uid>>,
    /// The clock stamped on newly detected contentions.
    ///
    /// Supplied rather than read, so merge is a bit-reproducible function of its inputs
    /// (rule D). Two peers merging the same stores with the same clock get identical
    /// bytes.
    pub now: Option<Hlc>,
    /// Fail rather than return when contentions are present. Exit 5 at the CLI.
    pub fail_on_contention: bool,
    /// Refuse a merge that lets one agent raise more than this many contentions - the
    /// mitigation for contention flooding (§29).
    pub max_contentions_per_agent: Option<usize>,
}

impl MergeOptions {
    pub fn with_now(mut self, now: Hlc) -> MergeOptions {
        self.now = Some(now);
        self
    }

    pub fn with_labels(mut self, labels: Vec<BTreeMap<Label, Uid>>) -> MergeOptions {
        self.labels = labels;
        self
    }
}

/// What a merge did.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct MergeReport {
    /// Records that were not already present.
    pub added: usize,
    /// Records already present. Idempotence made visible.
    pub duplicates: usize,
    /// Every contention the merged store implies - not only the new ones, because
    /// detection is a function of the union rather than of the increment.
    pub contentions: Vec<Contention>,
    /// Contentions this merge introduced.
    pub new_contentions: Vec<Contention>,
    pub report: Report,
}

impl MergeReport {
    pub fn has_contentions(&self) -> bool {
        !self.contentions.is_empty()
    }
}

/// Merge `other` into `store`.
///
/// The union itself is steps 1-4 of §16.5 and lives in `Store::append`, which is already a
/// set union keyed by content. What is added here is detection (step 5) and the retraction
/// fixpoint (step 6) - both pure functions of the merged set, which is why they inherit the
/// union's commutativity rather than threatening it.
pub fn merge(store: &mut Store, other: &Store, opts: MergeOptions) -> Result<MergeReport, Error> {
    let incoming: Vec<Record> = other.iter().cloned().collect();
    let before: Vec<Contention> = store.contentions().to_vec();

    let appended = store.append(&incoming)?;

    let ctx = DetectionContext {
        labels: opts.labels.clone(),
        now: opts.now.clone(),
    };
    let mut detected = contention::detect(store, &ctx);

    // `SupersessionPolicy::All` records the successors and says nothing; `Latest` takes
    // the newest and says nothing. Only `Contend` materialises the disagreement.
    if opts.supersession != SupersessionPolicy::Contend {
        detected.retain(|c| c.detected.kind != smysl_core::DetectionKind::SupersessionFork);
    }

    let new_contentions: Vec<Contention> = detected
        .iter()
        .filter(|c| !before.iter().any(|b| b.id == c.id))
        .cloned()
        .collect();

    // Detected contentions are **reported, not recorded**.
    //
    // Detection is not monotone: two successors that fork today become a chain the moment
    // some third store supplies the edge ordering them. Writing a detection into the log
    // would make that stale finding permanent - and, because the log only grows, would
    // break associativity outright. `merge(merge(A,B),C)` would carry a contention that
    // `merge(A,merge(B,C))` never saw.
    //
    // So the store holds only contentions somebody deliberately *recorded*, which union
    // like any other record, and detection stays a derived view of the union. A caller
    // that wants a finding to travel appends it explicitly.

    let mut report = Report::new();
    retraction::report_retractions(store, opts.retraction, &mut report);
    report.sort();

    if let Some(cap) = opts.max_contentions_per_agent {
        if let Some(n) = flooding(&detected, cap) {
            report.push(
                smysl_core::Diagnostic::new(smysl_core::Code::W055).with_message(format!(
                    "{n} contentions from one source exceeds the cap of {cap}"
                )),
            );
        }
    }

    let out = MergeReport {
        added: appended.added,
        duplicates: appended.duplicates,
        contentions: detected,
        new_contentions,
        report,
    };

    if opts.fail_on_contention && out.has_contentions() {
        return Err(Error::Merge(MergeError::ContentionsPresent {
            count: out.contentions.len(),
        }));
    }
    Ok(out)
}

/// How many contentions one merge introduced, if that exceeds the cap.
///
/// Attribution to a specific agent needs signing to mean anything adversarially (N9), so
/// until then this counts the batch rather than the accuser.
fn flooding(detected: &[Contention], cap: usize) -> Option<usize> {
    (detected.len() > cap).then_some(detected.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{
        canonical_uid, AgentId, Attestation, Code, KernelType, Op, RelKind, Relation, Rung, Status,
        Thread, ThreadId, ThreadSchema, UnitCore, UnitCoreBuilder,
    };

    fn agent(s: &str) -> AgentId {
        AgentId::new(s).unwrap()
    }

    fn claim(gist: &str) -> UnitCore {
        UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
            .build()
            .unwrap()
    }

    fn opts() -> MergeOptions {
        MergeOptions::default().with_now(Hlc::new(1, 0, agent("tool:test")))
    }

    #[test]
    fn merging_adds_what_is_missing() {
        let a = claim("a");
        let b = claim("b");
        let mut left = Store::from_records(vec![Record::Unit(a.clone())]);
        let right = Store::from_records(vec![Record::Unit(b.clone())]);

        let r = merge(&mut left, &right, opts()).unwrap();
        assert_eq!(r.added, 1);
        assert_eq!(r.duplicates, 0);
        assert!(left.contains_uid(&canonical_uid(&a)));
        assert!(left.contains_uid(&canonical_uid(&b)));
    }

    /// Idempotence: merging the same store twice costs nothing, which is what lets a
    /// delivery be duplicated without consequence.
    #[test]
    fn merging_twice_changes_nothing() {
        let mut left = Store::from_records(vec![Record::Unit(claim("a"))]);
        let right = Store::from_records(vec![Record::Unit(claim("b"))]);

        merge(&mut left, &right, opts()).unwrap();
        let digest = left.state_hash();
        let second = merge(&mut left, &right, opts()).unwrap();

        assert_eq!(second.added, 0);
        assert_eq!(second.duplicates, 1);
        assert_eq!(left.state_hash(), digest);
    }

    /// The same claim from two agents is one unit with two attestations, not two units.
    #[test]
    fn the_same_claim_from_two_agents_is_one_unit() {
        let core = claim("p95 tripled");
        let uid = canonical_uid(&core);
        let att = |who: &str| {
            let a = agent(who);
            Record::Attestation(Attestation::new(
                uid,
                a.clone(),
                Op::Authored,
                Rung::Model,
                Hlc::zero(a),
            ))
        };
        let mut left = Store::from_records(vec![Record::Unit(core.clone()), att("model:a/x")]);
        let right = Store::from_records(vec![Record::Unit(core), att("model:b/y")]);

        merge(&mut left, &right, opts()).unwrap();
        assert_eq!(left.units().count(), 1);
        assert_eq!(left.get(&uid).unwrap().attestations.len(), 2);
        assert_eq!(left.get(&uid).unwrap().corroboration_groups(), 2);
    }

    #[test]
    fn concurrent_supersession_is_materialised_not_adjudicated() {
        let target = claim("the original");
        let a = claim("revision a");
        let b = claim("revision b");
        let (ut, ua, ub) = (canonical_uid(&target), canonical_uid(&a), canonical_uid(&b));

        let mut left = Store::from_records(vec![
            Record::Unit(target.clone()),
            Record::Unit(a),
            Record::Relation(Relation::new(RelKind::Supersedes, ua, ut)),
        ]);
        let right = Store::from_records(vec![
            Record::Unit(target),
            Record::Unit(b),
            Record::Relation(Relation::new(RelKind::Supersedes, ub, ut)),
        ]);

        let r = merge(&mut left, &right, opts()).unwrap();
        assert_eq!(r.contentions.len(), 1);
        assert_eq!(r.new_contentions.len(), 1);
        assert!(
            left.contains_uid(&ua) && left.contains_uid(&ub),
            "both survive"
        );
        assert!(
            left.contentions().is_empty(),
            "a detection is reported, not written into the log"
        );
    }

    #[test]
    fn a_non_contending_supersession_policy_stays_quiet() {
        let target = claim("the original");
        let a = claim("revision a");
        let b = claim("revision b");
        let (ut, ua, ub) = (canonical_uid(&target), canonical_uid(&a), canonical_uid(&b));
        let mut left = Store::from_records(vec![
            Record::Unit(target),
            Record::Unit(a),
            Record::Unit(b),
            Record::Relation(Relation::new(RelKind::Supersedes, ua, ut)),
            Record::Relation(Relation::new(RelKind::Supersedes, ub, ut)),
        ]);
        let right = Store::new();

        let mut o = opts();
        o.supersession = SupersessionPolicy::All;
        let r = merge(&mut left, &right, o).unwrap();
        assert!(r.contentions.is_empty());
    }

    #[test]
    fn fail_on_contention_is_opt_in() {
        let target = claim("the original");
        let a = claim("revision a");
        let b = claim("revision b");
        let (ut, ua, ub) = (canonical_uid(&target), canonical_uid(&a), canonical_uid(&b));
        let records = vec![
            Record::Unit(target),
            Record::Unit(a),
            Record::Unit(b),
            Record::Relation(Relation::new(RelKind::Supersedes, ua, ut)),
            Record::Relation(Relation::new(RelKind::Supersedes, ub, ut)),
        ];

        let mut quiet = Store::from_records(records.clone());
        assert!(merge(&mut quiet, &Store::new(), opts()).is_ok());

        let mut loud = Store::from_records(records);
        let mut o = opts();
        o.fail_on_contention = true;
        let e = merge(&mut loud, &Store::new(), o).unwrap_err();
        assert_eq!(e.exit_code(), smysl_core::ExitCode::Contentions);
    }

    /// Two agents publishing the same thread id do not conflict; they publish two
    /// registers. Presentation order is an opinion, and opinions have authors.
    #[test]
    fn two_owners_of_a_thread_do_not_conflict() {
        let core = claim("a claim");
        let uid = canonical_uid(&core);
        let mk = |who: &str, gist: &str| {
            let a = agent(who);
            Record::Thread(
                Thread::new(
                    ThreadId::new("t/brief").unwrap(),
                    ThreadSchema::Brief,
                    a.clone(),
                    gist,
                    Hlc::new(1, 0, a),
                )
                .with_steps([smysl_core::Step::new(smysl_core::Role::BottomLine, uid)]),
            )
        };
        let mut left = Store::from_records(vec![Record::Unit(core.clone()), mk("human:a", "mine")]);
        let right = Store::from_records(vec![Record::Unit(core), mk("human:b", "theirs")]);

        merge(&mut left, &right, opts()).unwrap();
        assert_eq!(left.threads().count(), 2);
    }

    #[test]
    fn the_later_write_wins_within_one_register() {
        let core = claim("a claim");
        let uid = canonical_uid(&core);
        let a = agent("human:v");
        let mk = |gist: &str, t: u64| {
            Record::Thread(
                Thread::new(
                    ThreadId::new("t/brief").unwrap(),
                    ThreadSchema::Brief,
                    a.clone(),
                    gist,
                    Hlc::new(t, 0, a.clone()),
                )
                .with_steps([smysl_core::Step::new(smysl_core::Role::BottomLine, uid)]),
            )
        };
        let mut left = Store::from_records(vec![Record::Unit(core.clone()), mk("first", 1)]);
        let right = Store::from_records(vec![Record::Unit(core), mk("second", 2)]);

        merge(&mut left, &right, opts()).unwrap();
        assert_eq!(left.threads().count(), 1);
        assert_eq!(left.threads().next().unwrap().gist, "second");
    }

    #[test]
    fn retraction_is_applied_and_reported() {
        use smysl_core::{SourceKind, SourceRef};
        let e = UnitCoreBuilder::new(KernelType::Evidence, "a measurement", Status::Measured)
            .source(SourceRef::new(SourceKind::Metric, "m"))
            .build()
            .unwrap();
        let ue = canonical_uid(&e);
        let c = UnitCoreBuilder::new(KernelType::Claim, "a claim", Status::Derived)
            .grounds([ue])
            .build()
            .unwrap();

        let mut left = Store::from_records(vec![Record::Unit(e), Record::Unit(c)]);
        let right = Store::from_records(vec![Record::Relation(Relation::new(
            RelKind::Retracts,
            ue,
            ue,
        ))]);

        let r = merge(&mut left, &right, opts()).unwrap();
        assert_eq!(r.report.count(Code::E050), 1);
        assert_eq!(
            effective_status(&left, RetractionPolicy::Strict).get(&ue),
            Some(Status::Unfounded)
        );
    }

    #[test]
    fn contention_flooding_is_reported() {
        let target = claim("the original");
        let ut = canonical_uid(&target);
        let mut records = vec![Record::Unit(target)];
        for i in 0..6u8 {
            let r = claim(&format!("revision {i}"));
            let ur = canonical_uid(&r);
            records.push(Record::Unit(r));
            records.push(Record::Relation(Relation::new(RelKind::Supersedes, ur, ut)));
        }
        let mut left = Store::from_records(records);
        let mut o = opts();
        o.max_contentions_per_agent = Some(0);
        let r = merge(&mut left, &Store::new(), o).unwrap();
        assert_eq!(r.report.count(Code::W055), 1);
    }

    /// The cap is a threshold, and a threshold tested only at zero is not tested.
    ///
    /// The test above sets `max_contentions_per_agent = Some(0)`, which makes `len > cap`
    /// trivially true for any contention at all — so `flooding` replaced with "always fire",
    /// or its `>` loosened to `>=`, passes it unchanged. All three mutants survived, and the
    /// count in the message was never read either.
    ///
    /// Three separate targets, each with a supersession fork, give three contentions to put a
    /// cap either side of. Six revisions of *one* target give one contention, not six —
    /// contention is per target — which is what the first draft of this test got wrong.
    #[test]
    fn the_contention_cap_is_a_threshold_and_not_a_switch() {
        fn merged_with_cap(cap: usize) -> Report {
            let mut records = Vec::new();
            for t in 0..3u8 {
                let target = claim(&format!("target {t}"));
                let ut = canonical_uid(&target);
                records.push(Record::Unit(target));
                for i in 0..2u8 {
                    let r = claim(&format!("target {t} revision {i}"));
                    let ur = canonical_uid(&r);
                    records.push(Record::Unit(r));
                    records.push(Record::Relation(Relation::new(RelKind::Supersedes, ur, ut)));
                }
            }
            let mut left = Store::from_records(records);
            let mut o = opts();
            o.max_contentions_per_agent = Some(cap);
            merge(&mut left, &Store::new(), o).unwrap().report
        }

        // What the fixture actually produces, read from the warning rather than assumed.
        let detected: usize = merged_with_cap(0)
            .iter()
            .find(|d| d.code == Code::W055)
            .and_then(|d| d.message.split_whitespace().next()?.parse().ok())
            .expect("the warning reports how many it saw");
        assert!(
            detected > 1,
            "the fixture must flood by more than one, saw {detected}"
        );

        // At the cap: not over it, so silent. This is the assertion `Some(0)`, `Some(1)` and
        // `>=` each fail — none of them can stay quiet.
        assert_eq!(
            merged_with_cap(detected).count(Code::W055),
            0,
            "a rate exactly at the cap is not over it"
        );
        // One below: over it, so reported.
        assert_eq!(merged_with_cap(detected - 1).count(Code::W055), 1);

        // And the number in the message is the number seen, not the cap and not a constant.
        let report = merged_with_cap(detected - 1);
        let msg = &report
            .iter()
            .find(|d| d.code == Code::W055)
            .expect("a message")
            .message;
        assert!(
            msg.starts_with(&format!("{detected} contentions")),
            "the warning must say how many it saw, not a constant: {msg:?}"
        );
    }

    #[test]
    fn merging_an_empty_store_changes_nothing() {
        let mut left = Store::from_records(vec![Record::Unit(claim("a"))]);
        let digest = left.state_hash();
        let r = merge(&mut left, &Store::new(), opts()).unwrap();
        assert_eq!(r.added, 0);
        assert_eq!(left.state_hash(), digest);
    }
}
