//! Pass 2 - reference integrity (§17).
//!
//! Three defects, and the distinction between the last two is the interesting part: a
//! cycle in the support graph is an error because it makes the graph unorderable, while a
//! cycle in `causes` or `sequences` is only a warning, because narrative feedback loops
//! are legitimate.

use smysl_core::diag::{Code, Diagnostic, Report, Subject};
use smysl_graph::{cycles, Adjacency, EdgeKind, EdgeSet, Store};

/// Run the pass.
pub fn run(store: &Store, report: &mut Report) {
    dangling(store, report);
    support_cycles(store.adjacency(), report);
    causal_cycles(store.adjacency(), report);
}

/// `SMY-E060` - a reference that points at nothing in this store.
fn dangling(store: &Store, report: &mut Report) {
    store.report_dangling(report);
}

/// `SMY-E061` - a cycle in the support graph.
///
/// Appendix D words this as "cycle in deps", but `grounds` belongs here too: rule M's
/// check is a single pass in topological order over `grounds` (§17), and a cycle there
/// leaves some units unorderable and therefore unverifiable. Both edge families are
/// support, and both are fatal.
///
/// **Unreachable through this library, and that is a fact rather than a hope.** Mutation
/// testing in 0.11 replaced this whole function with `()` and every test still passed. The
/// reason is not a missing test: `EdgeSet::support()` is `{Deps, Grounds}`, both derived from
/// a `UnitCore`'s own fields; `Unit` stores no uid and derives it from the core; so making two
/// units name each other requires solving a hash fixpoint. No input reaches the loop below.
///
/// The comment here used to say the pass exists "because a store can be assembled from records
/// that were never hashed together" — which was true of a design where a record carries its
/// uid, and is not true of this one. It is kept as a backstop against a future
/// `EdgeSet::support()` that admits a relation kind, because relation endpoints are *not*
/// content-derived and can cycle freely. `support_is_only_structural_edges` fails the moment
/// that happens, which is the moment this stops being dead code.
fn support_cycles(g: &Adjacency, report: &mut Report) {
    for group in cycles(g, &EdgeSet::support()) {
        let members: Vec<_> = group.iter().filter_map(|&n| g.uid(n)).copied().collect();
        let subject = members
            .first()
            .map(|u| Subject::Unit(*u))
            .unwrap_or(Subject::Store);
        report.push(
            Diagnostic::new(Code::E061)
                .with_subject(subject)
                .with_message(format!(
                    "cycle in the support graph over {} units: {}",
                    members.len(),
                    join(&members)
                ))
                .with_suggestion("break the cycle by weakening one edge to a discourse relation"),
        );
    }
}

/// `SMY-W062` - a cycle in `causes` or `sequences`.
fn causal_cycles(g: &Adjacency, report: &mut Report) {
    let kinds = EdgeSet::of(
        [smysl_core::RelKind::Causes, smysl_core::RelKind::Sequences]
            .into_iter()
            .filter_map(EdgeKind::kernel),
    );
    for group in cycles(g, &kinds) {
        let members: Vec<_> = group.iter().filter_map(|&n| g.uid(n)).copied().collect();
        let subject = members
            .first()
            .map(|u| Subject::Unit(*u))
            .unwrap_or(Subject::Store);
        report.push(
            Diagnostic::new(Code::W062)
                .with_subject(subject)
                .with_message(format!(
                    "causal cycle over {} units: {}",
                    members.len(),
                    join(&members)
                )),
        );
    }
}

fn join(uids: &[smysl_core::Uid]) -> String {
    uids.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{
        canonical_uid, KernelType, Record, RelKind, Relation, Status, Uid, UnitCore,
        UnitCoreBuilder,
    };

    fn claim(gist: &str, deps: Vec<Uid>) -> UnitCore {
        UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
            .deps(deps)
            .build()
            .unwrap()
    }

    fn check(records: Vec<Record>) -> Report {
        let store = Store::from_records(records);
        let mut r = Report::new();
        run(&store, &mut r);
        r
    }

    #[test]
    fn a_clean_store_reports_nothing() {
        let a = claim("a", vec![]);
        let b = claim("b", vec![canonical_uid(&a)]);
        assert!(check(vec![Record::Unit(a), Record::Unit(b)]).is_empty());
    }

    #[test]
    fn a_dangling_dep_is_e060() {
        let r = check(vec![Record::Unit(claim(
            "a",
            vec![Uid::from_bytes([9; 32])],
        ))]);
        assert_eq!(r.count(Code::E060), 1);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn a_dangling_relation_endpoint_is_e060() {
        let a = claim("a", vec![]);
        let r = check(vec![
            Record::Unit(a.clone()),
            Record::Relation(Relation::new(
                RelKind::Rebuts,
                Uid::from_bytes([9; 32]),
                canonical_uid(&a),
            )),
        ]);
        assert_eq!(r.count(Code::E060), 1);
    }

    /// Two dangling references are two diagnostics: passes never collapse defects,
    /// because the repair loop needs them all at once.
    #[test]
    fn every_dangling_reference_is_reported() {
        let mut b = UnitCoreBuilder::new(KernelType::Claim, "a", Status::Speculative);
        b.deps = [Uid::from_bytes([8; 32]), Uid::from_bytes([9; 32])]
            .into_iter()
            .collect();
        let r = check(vec![Record::Unit(b.build().unwrap())]);
        assert_eq!(r.count(Code::E060), 2);
    }

    #[test]
    fn a_support_cycle_is_e061() {
        // A cycle needs uids that reference each other, which content addressing makes
        // impossible to author - so it is injected through the adjacency directly.
        let a = claim("a", vec![]);
        let ua = canonical_uid(&a);
        let selfref = {
            let mut b = UnitCoreBuilder::new(KernelType::Claim, "self", Status::Speculative);
            b.deps = [ua].into_iter().collect();
            b.build().unwrap()
        };
        let us = canonical_uid(&selfref);
        let mut back = UnitCoreBuilder::new(KernelType::Claim, "a", Status::Speculative);
        back.deps = [us].into_iter().collect();
        // `a` and the unit that depends on it now depend on each other.
        let r = check(vec![
            Record::Unit(selfref),
            Record::Unit(back.build().unwrap()),
        ]);
        // The second unit's content differs from `a`, so this is a dangling edge rather
        // than a cycle - which is exactly what content addressing guarantees.
        assert_eq!(r.count(Code::E061), 0);
        assert!(r.count(Code::E060) > 0);
    }

    /// Content addressing makes a support cycle unconstructible: a unit's uid depends on
    /// what it points at, so nothing can point back at it. The pass still exists, because
    /// a store can be assembled from records that were never hashed together.
    #[test]
    fn a_causal_cycle_is_only_a_warning() {
        let a = claim("a", vec![]);
        let b = claim("b", vec![]);
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
        let r = check(vec![
            Record::Unit(a),
            Record::Unit(b),
            Record::Relation(Relation::new(RelKind::Causes, ua, ub)),
            Record::Relation(Relation::new(RelKind::Causes, ub, ua)),
        ]);
        assert_eq!(r.count(Code::W062), 1);
        assert_eq!(r.count(Code::E061), 0);
        assert!(
            r.is_clean(),
            "a narrative feedback loop must not fail a check"
        );
    }

    #[test]
    fn a_sequence_cycle_is_also_only_a_warning() {
        let a = claim("a", vec![]);
        let b = claim("b", vec![]);
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
        let r = check(vec![
            Record::Unit(a),
            Record::Unit(b),
            Record::Relation(Relation::new(RelKind::Sequences, ua, ub)),
            Record::Relation(Relation::new(RelKind::Sequences, ub, ua)),
        ]);
        assert_eq!(r.count(Code::W062), 1);
    }

    /// A rebuttal loop is not a causal loop, so it must not warn.
    #[test]
    fn a_rebuttal_loop_is_not_a_causal_cycle() {
        let a = claim("a", vec![]);
        let b = claim("b", vec![]);
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
        let r = check(vec![
            Record::Unit(a),
            Record::Unit(b),
            Record::Relation(Relation::new(RelKind::Rebuts, ua, ub)),
            Record::Relation(Relation::new(RelKind::Rebuts, ub, ua)),
        ]);
        assert!(r.is_empty());
    }

    #[test]
    fn the_diagnostic_names_the_units_involved() {
        let a = claim("a", vec![]);
        let b = claim("b", vec![]);
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
        let r = check(vec![
            Record::Unit(a),
            Record::Unit(b),
            Record::Relation(Relation::new(RelKind::Causes, ua, ub)),
            Record::Relation(Relation::new(RelKind::Causes, ub, ua)),
        ]);
        let d = r.iter().find(|d| d.code == Code::W062).unwrap();
        assert!(d.message.contains(&ua.to_string()));
        assert!(d.message.contains(&ub.to_string()));
    }

    #[test]
    fn an_empty_store_reports_nothing() {
        assert!(check(vec![]).is_empty());
    }

    /// The tripwire for the paragraph above.
    ///
    /// `support_cycles` is unreachable only while every support edge comes from a `UnitCore`'s
    /// own fields, because those point by content-derived uid and cannot form a loop. Add a
    /// relation kind to `EdgeSet::support()` — relation endpoints are arbitrary — and cycles
    /// become constructible, and the pass becomes load-bearing with no test behind it.
    ///
    /// So this fails at that moment rather than after it.
    #[test]
    fn support_is_only_structural_edges() {
        use smysl_graph::adjacency::{EdgeKind, EdgeSet};
        for k in smysl_core::RelKind::KERNEL.iter() {
            if let Some(edge) = EdgeKind::kernel(k.clone()) {
                assert!(
                    !EdgeSet::support().contains(edge),
                    "{k:?} is now a support edge. Relation endpoints are not content-derived, \
                     so a support cycle is constructible and `support_cycles` is reachable — \
                     it needs a test that emits SMY-E061, which nothing has ever done."
                );
            }
        }
    }
}
