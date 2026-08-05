//! Rule C - contention detection (§5.4).
//!
//! Merge **MUST NOT adjudicate**. When two agents disagree, an auto-merging transport picks
//! a winner and destroys the disagreement - the most valuable signal in a multi-agent
//! corpus (F7). Here the disagreement becomes an object: a consumer reasons about it, a
//! resolver supersedes both positions, the renderer surfaces it, and rule R pins both sides
//! into any pack touching either.
//!
//! Detection is a pure function of the merged set, which is what lets merge stay
//! commutative and associative: the same union produces the same contentions however it
//! was assembled. That includes the identifier - a contention's id is derived from its
//! content, never allocated.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::{
    hash_bytes, Contention, ContentionId, Detected, DetectionKind, Hlc, Label, RelKind, Uid,
};

use crate::store::Store;

/// Everything detection needs that is not in the store.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct DetectionContext {
    /// Label bindings per source, for detection (c). Labels have no wire record, so only a
    /// caller that parsed surface text can supply them.
    pub labels: Vec<BTreeMap<Label, Uid>>,
    /// The clock stamped on newly detected contentions.
    ///
    /// Supplied rather than read, so merge stays a bit-reproducible function of its inputs
    /// (rule D). The CLI passes a real clock; tests and replays pass a fixed one.
    pub now: Option<Hlc>,
}

/// Detect every contention implied by a store (§5.4).
pub fn detect(store: &Store, ctx: &DetectionContext) -> Vec<Contention> {
    let mut out = Vec::new();
    out.extend(supersession_forks(store, ctx));
    out.extend(live_rebuttals(store, ctx));
    out.extend(label_collisions(store, ctx));
    out.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    out.dedup_by(|a, b| a.id == b.id);
    out
}

/// (a) Two distinct uids supersede the same target and neither supersedes the other.
fn supersession_forks(store: &Store, ctx: &DetectionContext) -> Vec<Contention> {
    let mut by_target: BTreeMap<Uid, BTreeSet<Uid>> = BTreeMap::new();
    for r in store.relations_of_kind(&RelKind::Supersedes) {
        by_target.entry(r.to).or_default().insert(r.from);
    }

    by_target
        .into_iter()
        .filter(|(_, successors)| successors.len() > 1)
        .filter_map(|(target, successors)| {
            // A chain is not a fork: if the successors are totally ordered by
            // `supersedes` among themselves, there is a latest and no disagreement.
            let positions: Vec<Uid> = successors.iter().copied().collect();
            if totally_ordered(store, &positions) {
                return None;
            }
            Some(contention(
                DetectionKind::SupersessionFork,
                target,
                positions,
                ctx,
            ))
        })
        .collect()
}

/// Whether every pair among `uids` is related by `supersedes` in one direction or the
/// other - that is, whether they form a chain rather than a fork.
fn totally_ordered(store: &Store, uids: &[Uid]) -> bool {
    for (i, a) in uids.iter().enumerate() {
        for b in &uids[i + 1..] {
            let ab = store.has_relation(&RelKind::Supersedes, a, b);
            let ba = store.has_relation(&RelKind::Supersedes, b, a);
            if !ab && !ba {
                return false;
            }
        }
    }
    true
}

/// (b) A `rebuts` edge between two units both selected in a common thread.
///
/// A rebuttal that nobody has threaded together is a disagreement in waiting; one that a
/// thread presents as a single line of argument is a disagreement in progress.
fn live_rebuttals(store: &Store, ctx: &DetectionContext) -> Vec<Contention> {
    let mut out = Vec::new();
    for rel in store.relations_of_kind(&RelKind::Rebuts) {
        let together = store.threads().any(|t| {
            let units: BTreeSet<&Uid> = t.units().collect();
            units.contains(&rel.from) && units.contains(&rel.to)
        });
        if together {
            out.push(contention(
                DetectionKind::LiveRebuttal,
                rel.to,
                vec![rel.from, rel.to],
                ctx,
            ));
        }
    }
    out
}

/// (c) One label bound to different uids across views in scope.
fn label_collisions(store: &Store, ctx: &DetectionContext) -> Vec<Contention> {
    let mut bindings: BTreeMap<Label, BTreeSet<Uid>> = BTreeMap::new();
    for map in &ctx.labels {
        for (l, u) in map {
            bindings.entry(l.clone()).or_default().insert(*u);
        }
    }
    // Labels carried on units themselves count too, where a store has them.
    for (uid, unit) in store.units() {
        for l in &unit.labels {
            bindings.entry(l.clone()).or_default().insert(*uid);
        }
    }

    bindings
        .into_iter()
        .filter(|(_, uids)| uids.len() > 1)
        .map(|(_, uids)| {
            let positions: Vec<Uid> = uids.into_iter().collect();
            let over = positions[0];
            contention(DetectionKind::LabelCollision, over, positions, ctx)
        })
        .collect()
}

/// Build a contention whose identifier is a function of its content.
///
/// Deriving the id rather than allocating one is what keeps merge commutative: two peers
/// that detect the same disagreement name it the same thing, so unioning their stores does
/// not produce two records for one argument.
fn contention(
    kind: DetectionKind,
    over: Uid,
    mut positions: Vec<Uid>,
    ctx: &DetectionContext,
) -> Contention {
    positions.sort();
    positions.dedup();

    let mut bytes = Vec::with_capacity(1 + 32 * (positions.len() + 1));
    bytes.push(kind.as_u8());
    bytes.extend_from_slice(over.as_bytes());
    for p in &positions {
        bytes.extend_from_slice(p.as_bytes());
    }
    let digest = Uid::from_bytes(hash_bytes(&bytes));
    // `k/c…` - the leading letter keeps the identifier a well-formed label, whose first
    // character must be alphabetic.
    let id = ContentionId::new(format!("k/c{}", &digest.short()[3..]))
        .expect("a derived identifier is always well-formed");

    let ts = ctx
        .now
        .clone()
        .unwrap_or_else(|| Hlc::new(0, 0, merge_agent()));

    Contention::new(id, over, positions, Detected::new(kind, ts))
}

/// The agent a contention is attributed to when the caller supplied no clock. Merge is not
/// an author, so it claims to be a tool rather than borrowing anyone's identity.
fn merge_agent() -> smysl_core::AgentId {
    smysl_core::AgentId::new("tool:smysl-merge").expect("a valid literal")
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{
        canonical_uid, AgentId, KernelType, Record, Relation, Role, Status, Step, Thread, ThreadId,
        ThreadSchema, UnitCore, UnitCoreBuilder,
    };

    fn agent() -> AgentId {
        AgentId::new("human:vladimir").unwrap()
    }

    fn claim(gist: &str) -> UnitCore {
        UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
            .build()
            .unwrap()
    }

    fn ctx() -> DetectionContext {
        DetectionContext::default()
    }

    #[test]
    fn a_clean_store_has_nothing_to_contend() {
        let store = Store::from_records(vec![Record::Unit(claim("a"))]);
        assert!(detect(&store, &ctx()).is_empty());
    }

    /// (a) Two successors, neither superseding the other, is a fork.
    #[test]
    fn concurrent_supersession_is_a_contention() {
        let target = claim("the original");
        let a = claim("revision a");
        let b = claim("revision b");
        let (ut, ua, ub) = (canonical_uid(&target), canonical_uid(&a), canonical_uid(&b));
        let store = Store::from_records(vec![
            Record::Unit(target),
            Record::Unit(a),
            Record::Unit(b),
            Record::Relation(Relation::new(RelKind::Supersedes, ua, ut)),
            Record::Relation(Relation::new(RelKind::Supersedes, ub, ut)),
        ]);

        let c = detect(&store, &ctx());
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].detected.kind, DetectionKind::SupersessionFork);
        assert_eq!(c[0].over, ut);
        assert_eq!(c[0].positions.len(), 2);
        assert!(c[0].is_open());
    }

    /// A chain is not a fork: if the successors supersede each other there is a latest,
    /// and nobody disagrees.
    #[test]
    fn a_supersession_chain_is_not_a_fork() {
        let target = claim("the original");
        let a = claim("revision a");
        let b = claim("revision b");
        let (ut, ua, ub) = (canonical_uid(&target), canonical_uid(&a), canonical_uid(&b));
        let store = Store::from_records(vec![
            Record::Unit(target),
            Record::Unit(a),
            Record::Unit(b),
            Record::Relation(Relation::new(RelKind::Supersedes, ua, ut)),
            Record::Relation(Relation::new(RelKind::Supersedes, ub, ut)),
            Record::Relation(Relation::new(RelKind::Supersedes, ub, ua)),
        ]);
        assert!(detect(&store, &ctx()).is_empty());
    }

    #[test]
    fn a_single_successor_is_not_a_fork() {
        let target = claim("the original");
        let a = claim("revision a");
        let (ut, ua) = (canonical_uid(&target), canonical_uid(&a));
        let store = Store::from_records(vec![
            Record::Unit(target),
            Record::Unit(a),
            Record::Relation(Relation::new(RelKind::Supersedes, ua, ut)),
        ]);
        assert!(detect(&store, &ctx()).is_empty());
    }

    /// (b) A rebuttal only contends once a thread presents both sides as one argument.
    #[test]
    fn a_rebuttal_inside_a_thread_is_a_contention() {
        let a = claim("the claim");
        let b = claim("the rebuttal");
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
        let thread = Thread::new(
            ThreadId::new("t/brief").unwrap(),
            ThreadSchema::Brief,
            agent(),
            "g",
            Hlc::zero(agent()),
        )
        .with_steps([Step::new(Role::BottomLine, ua), Step::new(Role::Risk, ub)]);
        let store = Store::from_records(vec![
            Record::Unit(a),
            Record::Unit(b),
            Record::Relation(Relation::new(RelKind::Rebuts, ub, ua)),
            Record::Thread(thread),
        ]);

        let c = detect(&store, &ctx());
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].detected.kind, DetectionKind::LiveRebuttal);
        assert_eq!(c[0].over, ua);
    }

    #[test]
    fn a_rebuttal_outside_any_thread_is_not_yet_a_contention() {
        let a = claim("the claim");
        let b = claim("the rebuttal");
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
        let store = Store::from_records(vec![
            Record::Unit(a),
            Record::Unit(b),
            Record::Relation(Relation::new(RelKind::Rebuts, ub, ua)),
        ]);
        assert!(detect(&store, &ctx()).is_empty());
    }

    #[test]
    fn a_thread_holding_only_one_side_is_not_a_contention() {
        let a = claim("the claim");
        let b = claim("the rebuttal");
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
        let thread = Thread::new(
            ThreadId::new("t/brief").unwrap(),
            ThreadSchema::Brief,
            agent(),
            "g",
            Hlc::zero(agent()),
        )
        .with_steps([Step::new(Role::BottomLine, ua)]);
        let store = Store::from_records(vec![
            Record::Unit(a),
            Record::Unit(b),
            Record::Relation(Relation::new(RelKind::Rebuts, ub, ua)),
            Record::Thread(thread),
        ]);
        assert!(detect(&store, &ctx()).is_empty());
    }

    /// (c) One label, two uids, is a disagreement about what a name means.
    #[test]
    fn a_label_bound_to_two_uids_is_a_contention() {
        let a = claim("a");
        let b = claim("b");
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
        let store = Store::from_records(vec![Record::Unit(a), Record::Unit(b)]);
        let label = Label::new("c/p95").unwrap();
        let c = detect(
            &store,
            &DetectionContext {
                labels: vec![
                    BTreeMap::from([(label.clone(), ua)]),
                    BTreeMap::from([(label, ub)]),
                ],
                now: None,
            },
        );
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].detected.kind, DetectionKind::LabelCollision);
    }

    #[test]
    fn the_same_label_bound_to_the_same_uid_is_agreement() {
        let a = claim("a");
        let ua = canonical_uid(&a);
        let store = Store::from_records(vec![Record::Unit(a)]);
        let label = Label::new("c/p95").unwrap();
        let c = detect(
            &store,
            &DetectionContext {
                labels: vec![
                    BTreeMap::from([(label.clone(), ua)]),
                    BTreeMap::from([(label, ua)]),
                ],
                now: None,
            },
        );
        assert!(c.is_empty());
    }

    /// The identifier is a function of the content, so two peers that see the same
    /// disagreement name it the same thing - which is what stops a union producing two
    /// records for one argument.
    #[test]
    fn contention_ids_are_derived_from_content() {
        let target = claim("the original");
        let a = claim("revision a");
        let b = claim("revision b");
        let (ut, ua, ub) = (canonical_uid(&target), canonical_uid(&a), canonical_uid(&b));
        let build = |order: Vec<Record>| Store::from_records(order);

        let forward = build(vec![
            Record::Unit(target.clone()),
            Record::Unit(a.clone()),
            Record::Unit(b.clone()),
            Record::Relation(Relation::new(RelKind::Supersedes, ua, ut)),
            Record::Relation(Relation::new(RelKind::Supersedes, ub, ut)),
        ]);
        let backward = build(vec![
            Record::Unit(b),
            Record::Unit(a),
            Record::Unit(target),
            Record::Relation(Relation::new(RelKind::Supersedes, ub, ut)),
            Record::Relation(Relation::new(RelKind::Supersedes, ua, ut)),
        ]);

        let one = detect(&forward, &ctx());
        let two = detect(&backward, &ctx());
        assert_eq!(one[0].id, two[0].id);
        assert_eq!(one[0].positions, two[0].positions);
    }

    #[test]
    fn a_derived_id_is_a_well_formed_label() {
        let c = contention(
            DetectionKind::LiveRebuttal,
            Uid::from_bytes([0xFF; 32]),
            vec![Uid::from_bytes([1; 32])],
            &ctx(),
        );
        assert!(c.id.as_str().starts_with("k/c"));
        assert_eq!(ContentionId::new(c.id.as_str()).unwrap(), c.id);
    }

    /// Position order must not leak into the identifier.
    #[test]
    fn positions_are_canonicalised_before_hashing() {
        let a = Uid::from_bytes([1; 32]);
        let b = Uid::from_bytes([2; 32]);
        let over = Uid::from_bytes([3; 32]);
        let one = contention(DetectionKind::LiveRebuttal, over, vec![a, b], &ctx());
        let two = contention(DetectionKind::LiveRebuttal, over, vec![b, a, b], &ctx());
        assert_eq!(one.id, two.id);
        assert_eq!(one.positions, two.positions);
    }

    #[test]
    fn detection_is_deterministic() {
        let target = claim("the original");
        let a = claim("revision a");
        let b = claim("revision b");
        let (ut, ua, ub) = (canonical_uid(&target), canonical_uid(&a), canonical_uid(&b));
        let store = Store::from_records(vec![
            Record::Unit(target),
            Record::Unit(a),
            Record::Unit(b),
            Record::Relation(Relation::new(RelKind::Supersedes, ua, ut)),
            Record::Relation(Relation::new(RelKind::Supersedes, ub, ut)),
        ]);
        assert_eq!(detect(&store, &ctx()), detect(&store, &ctx()));
    }
}
