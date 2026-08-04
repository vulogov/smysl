//! Pass 6 - rule M, epistemic monotonicity (§1.4, §17).
//!
//! ```text
//! ∀u : status(u) ∈ {Derived, Inferred} ⇒ status(u) ≤ min { status(g) : g ∈ grounds(u) }
//! ```
//!
//! This is the guarantee that makes hallucination laundering structurally impossible
//! inside the graph. Prose has no type for *measured* versus *guessed*, so hedges are the
//! first casualty of summarisation and by hop three a speculation is a fact. Here the cap
//! is mechanical: a claim cannot be stronger than the weakest thing it rests on.
//!
//! `measured` and `cited` are exempt because they ground out externally - their support is
//! a source reference, not another unit. That exemption is what rule T closes at the
//! ingestion boundary (§9.3).
//!
//! The check is a single pass in topological order over `grounds`, so it is `O(V+E)`. The
//! diagnostic names the **weakest ground**, not just the violation: that is the actionable
//! part, and it is free to compute.

use smysl_core::diag::{Code, Diagnostic, Report};
use smysl_core::{Status, Uid};
use smysl_graph::{topo, EdgeKind, EdgeSet, Store};

pub fn run(store: &Store, report: &mut Report) {
    let g = store.adjacency();
    let order = topo(g, &EdgeSet::one(EdgeKind::Grounds));

    // Dependencies before dependents, so a ground's own status is settled before anything
    // that rests on it is judged.
    for node in order.order.iter().chain(order.cyclic.iter()) {
        let Some(uid) = g.uid(*node) else { continue };
        let Some(unit) = store.get(uid) else { continue };
        let status = unit.core.status;
        if !status.is_rule_m_constrained() {
            continue;
        }

        let Some((cap, weakest)) = weakest_ground(store, uid) else {
            // Every ground is absent. That is `SMY-E060`'s business, not this pass's -
            // reporting it twice would tell the repair loop to fix one thing in two
            // places.
            continue;
        };

        if status > cap {
            report.push(
                Diagnostic::on(Code::E030, *uid)
                    .with_message(format!(
                        "{status} exceeds the cap of {cap} set by its weakest ground {weakest}"
                    ))
                    .with_suggestion(format!(
                        "weaken this unit to {cap}, or strengthen {weakest}"
                    )),
            );
        }
    }
}

/// The lowest status among a unit's present grounds, and which ground it was.
///
/// Ties break by ascending uid, so the named ground is a function of the graph rather than
/// of iteration order (rule D).
fn weakest_ground(store: &Store, uid: &Uid) -> Option<(Status, Uid)> {
    let unit = store.get(uid)?;
    unit.core
        .grounds
        .iter()
        .filter_map(|g| store.get(g).map(|u| (u.core.status, *g)))
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{
        canonical_uid, KernelType, Record, SourceKind, SourceRef, Status, UnitCore, UnitCoreBuilder,
    };

    fn check(records: Vec<Record>) -> Report {
        let store = Store::from_records(records);
        let mut r = Report::new();
        run(&store, &mut r);
        r
    }

    fn measured(gist: &str) -> UnitCore {
        UnitCoreBuilder::new(KernelType::Evidence, gist, Status::Measured)
            .source(SourceRef::new(SourceKind::Metric, "m"))
            .build()
            .unwrap()
    }

    fn cited(gist: &str) -> UnitCore {
        UnitCoreBuilder::new(KernelType::Evidence, gist, Status::Cited)
            .source(SourceRef::new(SourceKind::Doc, "d"))
            .build()
            .unwrap()
    }

    fn speculative(gist: &str) -> UnitCore {
        UnitCoreBuilder::new(KernelType::Hypothesis, gist, Status::Speculative)
            .build()
            .unwrap()
    }

    fn grounded(gist: &str, status: Status, grounds: Vec<Uid>) -> UnitCore {
        UnitCoreBuilder::new(KernelType::Claim, gist, status)
            .grounds(grounds)
            .build()
            .unwrap()
    }

    #[test]
    fn a_claim_no_stronger_than_its_ground_passes() {
        let e = measured("a measurement");
        let c = grounded("a derivation", Status::Derived, vec![canonical_uid(&e)]);
        assert!(check(vec![Record::Unit(e), Record::Unit(c)]).is_empty());
    }

    /// The core case: `derived` resting on `speculative` is laundering, and the cap is
    /// mechanical rather than editorial.
    #[test]
    fn a_claim_stronger_than_its_ground_is_e030() {
        let s = speculative("a guess");
        let c = grounded("a derivation", Status::Derived, vec![canonical_uid(&s)]);
        let r = check(vec![Record::Unit(s), Record::Unit(c)]);
        assert_eq!(r.count(Code::E030), 1);
    }

    /// Naming the weakest ground is the actionable part, and it is free to compute.
    #[test]
    fn the_diagnostic_names_the_weakest_ground() {
        let strong = measured("a measurement");
        let weak = speculative("a guess");
        let (us, uw) = (canonical_uid(&strong), canonical_uid(&weak));
        let c = grounded("a derivation", Status::Derived, vec![us, uw]);
        let r = check(vec![
            Record::Unit(strong),
            Record::Unit(weak),
            Record::Unit(c),
        ]);
        let d = r.iter().find(|d| d.code == Code::E030).unwrap();
        assert!(d.message.contains(&uw.to_string()), "{}", d.message);
        assert!(
            !d.message.contains(&us.to_string()),
            "the strong ground is not at fault"
        );
        assert!(d.message.contains("speculative"));
        assert!(d.suggestion.is_some());
    }

    #[test]
    fn the_cap_is_the_minimum_over_every_ground() {
        let m = measured("measured");
        let c = cited("cited");
        let (um, uc) = (canonical_uid(&m), canonical_uid(&c));
        let records = vec![Record::Unit(m), Record::Unit(c)];

        // `cited` (4) is the weaker of the two, and `derived` (3) is below it.
        let ok = grounded("ok", Status::Derived, vec![um, uc]);
        let mut all = records.clone();
        all.push(Record::Unit(ok));
        assert!(check(all).is_empty());
    }

    /// Statuses that ground out externally are exempt: there is no `measured` ground for a
    /// `measured` unit to rest on.
    #[test]
    fn measured_and_cited_are_exempt() {
        let s = speculative("a guess");
        let us = canonical_uid(&s);
        for status in [Status::Measured, Status::Cited] {
            let u = UnitCoreBuilder::new(KernelType::Claim, "external", status)
                .grounds([us])
                .source(SourceRef::new(SourceKind::Metric, "m"))
                .build()
                .unwrap();
            let r = check(vec![Record::Unit(s.clone()), Record::Unit(u)]);
            assert_eq!(r.count(Code::E030), 0, "{status} must be exempt");
        }
    }

    #[test]
    fn speculative_is_never_too_strong() {
        let s = speculative("a guess");
        let c = UnitCoreBuilder::new(KernelType::Claim, "also a guess", Status::Speculative)
            .grounds([canonical_uid(&s)])
            .build()
            .unwrap();
        assert!(check(vec![Record::Unit(s), Record::Unit(c)]).is_empty());
    }

    #[test]
    fn inferred_on_speculative_is_e030_and_inferred_on_measured_is_not() {
        let s = speculative("a guess");
        let m = measured("a measurement");
        let (us, um) = (canonical_uid(&s), canonical_uid(&m));

        let bad = grounded("inference", Status::Inferred, vec![us]);
        assert_eq!(
            check(vec![Record::Unit(s), Record::Unit(bad)]).count(Code::E030),
            1
        );

        let good = grounded("inference", Status::Inferred, vec![um]);
        assert!(check(vec![Record::Unit(m), Record::Unit(good)]).is_empty());
    }

    /// Rule M is transitive by construction: each link is checked locally, and the chain
    /// falls out of the topological order.
    #[test]
    fn laundering_cannot_survive_a_chain() {
        let s = speculative("a guess");
        let us = canonical_uid(&s);
        let hop1 = UnitCoreBuilder::new(KernelType::Claim, "hop one", Status::Speculative)
            .grounds([us])
            .build()
            .unwrap();
        let u1 = canonical_uid(&hop1);
        let hop2 = grounded("hop two", Status::Derived, vec![u1]);

        let r = check(vec![
            Record::Unit(s),
            Record::Unit(hop1),
            Record::Unit(hop2),
        ]);
        assert_eq!(
            r.count(Code::E030),
            1,
            "the promotion must be caught wherever in the chain it happens"
        );
    }

    /// Every laundering attempt is reported, not just the first - the repair loop needs
    /// them all at once.
    #[test]
    fn every_violation_is_reported() {
        let s = speculative("a guess");
        let us = canonical_uid(&s);
        let a = grounded("a", Status::Derived, vec![us]);
        let b = grounded("b", Status::Inferred, vec![us]);
        let r = check(vec![Record::Unit(s), Record::Unit(a), Record::Unit(b)]);
        assert_eq!(r.count(Code::E030), 2);
    }

    /// A ground that is not in the store is pass 2's business. Reporting it here too
    /// would tell the repair loop to fix one thing in two places.
    #[test]
    fn an_absent_ground_is_not_reported_by_this_pass() {
        let mut b = UnitCoreBuilder::new(KernelType::Claim, "orphan", Status::Derived);
        b.grounds = [Uid::from_bytes([9; 32])].into_iter().collect();
        assert!(check(vec![Record::Unit(b.build().unwrap())]).is_empty());
    }

    /// A present ground caps even when a sibling ground is missing: the units that *are*
    /// there still constrain what may be claimed.
    #[test]
    fn a_present_ground_still_caps_when_a_sibling_is_absent() {
        let s = speculative("a guess");
        let mut b = UnitCoreBuilder::new(KernelType::Claim, "mixed", Status::Derived);
        b.grounds = [canonical_uid(&s), Uid::from_bytes([9; 32])]
            .into_iter()
            .collect();
        let r = check(vec![Record::Unit(s), Record::Unit(b.build().unwrap())]);
        assert_eq!(r.count(Code::E030), 1);
    }

    #[test]
    fn an_empty_store_reports_nothing() {
        assert!(check(vec![]).is_empty());
    }

    #[test]
    fn the_pass_is_deterministic() {
        let s = speculative("a guess");
        let us = canonical_uid(&s);
        let records = vec![
            Record::Unit(s),
            Record::Unit(grounded("a", Status::Derived, vec![us])),
            Record::Unit(grounded("b", Status::Inferred, vec![us])),
        ];
        let mut reversed = records.clone();
        reversed.reverse();
        let a = check(records);
        let b = check(reversed);
        assert_eq!(a.diagnostics.len(), b.diagnostics.len());
        let mut a = a;
        let mut b = b;
        a.sort();
        b.sort();
        assert_eq!(a.diagnostics, b.diagnostics);
    }

    /// Rule M's boundary: *at* the cap is legal, and only *above* it is a violation.
    ///
    /// Mutation testing in 0.11 flipped `status > cap` to `status >= cap` with nothing
    /// failing — every existing test had a unit comfortably under its cap or clearly over it,
    /// and none sat exactly on it. The mutant rejects the commonest legal shape in the format:
    /// a claim held at precisely the strength of what it rests on.
    ///
    /// Building the fixture took two tries, and both failures are worth the comment. A pair at
    /// `cited` fails in the *constructor* with `SourceRequired`, never reaching the pass. A
    /// pair at `speculative` reaches the pass and is skipped, because rule M constrains only
    /// the statuses that require grounds. So the unit under test must be `derived` or
    /// `inferred`, and its weakest ground must be too — which needs a chain, not a pair.
    #[test]
    fn a_unit_exactly_at_its_weakest_ground_is_legal() {
        let m = measured("a measurement"); // cap: measured
        let um = canonical_uid(&m);
        let mid = grounded("a derivation from it", Status::Derived, vec![um]);
        let umid = canonical_uid(&mid); // legal: derived < measured
        let top = grounded("a derivation from that", Status::Derived, vec![umid]);

        let r = check(vec![Record::Unit(m), Record::Unit(mid), Record::Unit(top)]);
        assert_eq!(
            r.count(Code::E030),
            0,
            "a unit at exactly its ground's status was reported as exceeding it"
        );
    }

    /// The control. Without it the test above passes for a pass that reports nothing at all.
    #[test]
    fn a_unit_one_step_above_its_weakest_ground_is_not() {
        let g = speculative("a speculative ground");
        let ug = canonical_uid(&g);
        let c = grounded("a claim held too strongly", Status::Inferred, vec![ug]);
        let r = check(vec![Record::Unit(g), Record::Unit(c)]);
        assert_eq!(
            r.count(Code::E030),
            1,
            "one step above the cap must still be a violation"
        );
    }
}
