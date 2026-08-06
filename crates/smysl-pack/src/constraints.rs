//! The seven constraints of §8, as a checker.
//!
//! Written as a predicate over a finished selection rather than only as an invariant the
//! solver maintains, for two reasons. The property test needs to assert C1-C7 over
//! generated graphs, and every local-improvement move is validated against it and reverted
//! if it breaks anything - so an optimisation cannot quietly cost correctness.
//!
//! C3 is the one that matters most. If a selected claim's rebuttals cannot fit, the claim
//! is dropped; if the claim is pinned, packing **fails**. It must never emit the claim
//! alone, because a one-sided pack is exactly the failure prose transport has (F7).

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::{Contention, Lod, RelKind, Uid};
use smysl_graph::Store;

/// A selection: which units are in, and at what level.
pub type Selection = BTreeMap<Uid, Lod>;

/// Which constraint a selection breaks, and where.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Violation {
    /// C1: a unit at L1+ whose dep is absent.
    Dep { unit: Uid, missing: Uid },
    /// C2: a unit at L1+ whose ground is absent.
    Ground { unit: Uid, missing: Uid },
    /// C3, rule R: a selected unit whose rebuttal is absent.
    Rebuttal { unit: Uid, missing: Uid },
    /// C4: an open contention with a position absent.
    Contention { unit: Uid, missing: Uid },
    /// C5: a pinned unit below L1.
    Pin { unit: Uid, level: Lod },
    /// C6: a unit at L1+ whose warrant is absent.
    Warrant { unit: Uid, missing: Uid },
    /// C7: over budget.
    Budget { used: u64, budget: u64 },
}

impl Violation {
    pub const fn constraint(&self) -> &'static str {
        match self {
            Violation::Dep { .. } => "C1",
            Violation::Ground { .. } => "C2",
            Violation::Rebuttal { .. } => "C3",
            Violation::Contention { .. } => "C4",
            Violation::Pin { .. } => "C5",
            Violation::Warrant { .. } => "C6",
            Violation::Budget { .. } => "C7",
        }
    }
}

impl core::fmt::Display for Violation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let c = self.constraint();
        match self {
            Violation::Dep { unit, missing } => write!(f, "{c}: {unit} needs dep {missing}"),
            Violation::Ground { unit, missing } => write!(f, "{c}: {unit} needs ground {missing}"),
            Violation::Rebuttal { unit, missing } => {
                write!(f, "{c}: {unit} is unopposed - {missing} rebuts it")
            }
            Violation::Contention { unit, missing } => {
                write!(
                    f,
                    "{c}: {unit} is contested - {missing} is the other position"
                )
            }
            Violation::Pin { unit, level } => {
                write!(f, "{c}: {unit} is pinned but only at {level}")
            }
            Violation::Warrant { unit, missing } => {
                write!(f, "{c}: {unit} needs warrant {missing}")
            }
            Violation::Budget { used, budget } => {
                write!(f, "{c}: {used} over a budget of {budget}")
            }
        }
    }
}

/// Everything the checker needs beyond the store.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Constraints {
    /// C5: units that must reach L1 - the focus set and whatever a thread pins.
    pub pinned: BTreeSet<Uid>,
    /// C7.
    pub budget: u64,
}

/// Check a selection against C1-C7.
///
/// Returns every violation, not the first: a solver that fixed one at a time would loop.
pub fn violations(
    store: &Store,
    selection: &Selection,
    used: u64,
    c: &Constraints,
) -> Vec<Violation> {
    let mut out = Vec::new();
    let contentions: Vec<&Contention> =
        store.contentions().iter().filter(|k| k.is_open()).collect();

    for (uid, level) in selection {
        let Some(unit) = store.get(uid) else { continue };

        if *level >= Lod::L1 {
            for d in &unit.core.deps {
                if store.contains_uid(d) && !selection.contains_key(d) {
                    out.push(Violation::Dep {
                        unit: *uid,
                        missing: *d,
                    });
                }
            }
            for g in &unit.core.grounds {
                if store.contains_uid(g) && !selection.contains_key(g) {
                    out.push(Violation::Ground {
                        unit: *uid,
                        missing: *g,
                    });
                }
            }
            for w in warrants_of(store, uid) {
                if !selection.contains_key(&w) {
                    out.push(Violation::Warrant {
                        unit: *uid,
                        missing: w,
                    });
                }
            }
        }

        // C3 binds at every level, not only L1: a gist presented without its rebuttal is
        // still a claim presented unopposed.
        for r in store.rebuttals_of(uid) {
            if store.contains_uid(&r) && !selection.contains_key(&r) {
                out.push(Violation::Rebuttal {
                    unit: *uid,
                    missing: r,
                });
            }
        }

        for k in &contentions {
            if !k.pins(uid) {
                continue;
            }
            for p in &k.positions {
                if store.contains_uid(p) && !selection.contains_key(p) {
                    out.push(Violation::Contention {
                        unit: *uid,
                        missing: *p,
                    });
                }
            }
        }
    }

    for uid in &c.pinned {
        if !store.contains_uid(uid) {
            continue;
        }
        match selection.get(uid) {
            Some(l) if *l >= Lod::L1 => {}
            // A unit with no body cannot reach L1; L0 is its ceiling and satisfies the pin.
            Some(l) if store.get(uid).is_some_and(|u| u.core.body.is_none()) => {
                let _ = l;
            }
            Some(l) => out.push(Violation::Pin {
                unit: *uid,
                level: *l,
            }),
            None => out.push(Violation::Pin {
                unit: *uid,
                level: Lod::L0,
            }),
        }
    }

    if used > c.budget {
        out.push(Violation::Budget {
            used,
            budget: c.budget,
        });
    }

    out.sort();
    out.dedup();
    out
}

/// The warrants a unit declares: the targets of its `warrant` edges (§3.2).
pub fn warrants_of(store: &Store, uid: &Uid) -> Vec<Uid> {
    store
        .relations_of_kind(&RelKind::Warrant)
        .into_iter()
        .filter(|r| r.from == *uid)
        .map(|r| r.to)
        .filter(|t| store.contains_uid(t))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{
        canonical_uid, ContentionId, Detected, DetectionKind, Hlc, KernelType, Record, Relation,
        SourceKind, SourceRef, Status, UnitCore, UnitCoreBuilder,
    };

    fn evidence(gist: &str) -> UnitCore {
        UnitCoreBuilder::new(KernelType::Evidence, gist, Status::Measured)
            .source(SourceRef::new(SourceKind::Metric, "m"))
            .body("a body long enough to be worth buying at L1")
            .build()
            .unwrap()
    }

    fn claim(gist: &str, grounds: Vec<Uid>) -> UnitCore {
        let status = if grounds.is_empty() {
            Status::Speculative
        } else {
            Status::Inferred
        };
        UnitCoreBuilder::new(KernelType::Claim, gist, status)
            .grounds(grounds)
            .body("a body long enough to be worth buying at L1")
            .build()
            .unwrap()
    }

    fn constraints(pinned: Vec<Uid>) -> Constraints {
        Constraints {
            pinned: pinned.into_iter().collect(),
            budget: u64::MAX,
        }
    }

    #[test]
    fn an_empty_selection_violates_nothing() {
        let store = Store::from_records(vec![Record::Unit(evidence("e"))]);
        assert!(violations(&store, &Selection::new(), 0, &constraints(vec![])).is_empty());
    }

    /// C2: a claim at L1 has to bring its evidence, at least as a gist.
    #[test]
    fn a_unit_at_l1_needs_its_grounds() {
        let e = evidence("the evidence");
        let ue = canonical_uid(&e);
        let c = claim("the claim", vec![ue]);
        let uc = canonical_uid(&c);
        let store = Store::from_records(vec![Record::Unit(e), Record::Unit(c)]);

        let alone = Selection::from([(uc, Lod::L1)]);
        let v = violations(&store, &alone, 0, &constraints(vec![]));
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].constraint(), "C2");

        let with_ground = Selection::from([(uc, Lod::L1), (ue, Lod::L0)]);
        assert!(violations(&store, &with_ground, 0, &constraints(vec![])).is_empty());
    }

    /// At L0 the closure obligation does not bind - a gist is interpretable from the L0 of
    /// its deps, and those are not required to be present (rule L).
    #[test]
    fn a_unit_at_l0_does_not_need_its_grounds() {
        let e = evidence("the evidence");
        let ue = canonical_uid(&e);
        let c = claim("the claim", vec![ue]);
        let uc = canonical_uid(&c);
        let store = Store::from_records(vec![Record::Unit(e), Record::Unit(c)]);
        let sel = Selection::from([(uc, Lod::L0)]);
        assert!(violations(&store, &sel, 0, &constraints(vec![])).is_empty());
    }

    #[test]
    fn a_unit_at_l1_needs_its_deps() {
        let d = evidence("the definition");
        let ud = canonical_uid(&d);
        let c = UnitCoreBuilder::new(KernelType::Claim, "the claim", Status::Speculative)
            .deps([ud])
            .body("a body")
            .build()
            .unwrap();
        let uc = canonical_uid(&c);
        let store = Store::from_records(vec![Record::Unit(d), Record::Unit(c)]);
        let v = violations(
            &store,
            &Selection::from([(uc, Lod::L1)]),
            0,
            &constraints(vec![]),
        );
        assert_eq!(v[0].constraint(), "C1");
    }

    /// Rule R, the one that matters: a claim never travels without its rebuttals.
    #[test]
    fn a_selected_unit_must_bring_its_rebuttals() {
        let c = claim("the claim", vec![]);
        let uc = canonical_uid(&c);
        let r = claim("the rebuttal", vec![]);
        let ur = canonical_uid(&r);
        let store = Store::from_records(vec![
            Record::Unit(c),
            Record::Unit(r),
            Record::Relation(Relation::new(RelKind::Rebuts, ur, uc)),
        ]);

        let alone = Selection::from([(uc, Lod::L0)]);
        let v = violations(&store, &alone, 0, &constraints(vec![]));
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].constraint(), "C3");

        let both = Selection::from([(uc, Lod::L0), (ur, Lod::L0)]);
        assert!(violations(&store, &both, 0, &constraints(vec![])).is_empty());
    }

    /// C3 binds at L0 too. A gist presented without its rebuttal is still a claim
    /// presented unopposed.
    #[test]
    fn the_rebuttal_obligation_binds_at_every_level() {
        let c = claim("the claim", vec![]);
        let uc = canonical_uid(&c);
        let r = claim("the rebuttal", vec![]);
        let ur = canonical_uid(&r);
        let store = Store::from_records(vec![
            Record::Unit(c),
            Record::Unit(r),
            Record::Relation(Relation::new(RelKind::Rebuts, ur, uc)),
        ]);
        for level in [Lod::L0, Lod::L1] {
            let sel = Selection::from([(uc, level)]);
            assert!(
                !violations(&store, &sel, 0, &constraints(vec![])).is_empty(),
                "unopposed at {level}"
            );
        }
    }

    /// The rebuttal itself does not drag the claim in - the obligation is directional.
    #[test]
    fn selecting_only_the_rebuttal_is_fine() {
        let c = claim("the claim", vec![]);
        let uc = canonical_uid(&c);
        let r = claim("the rebuttal", vec![]);
        let ur = canonical_uid(&r);
        let store = Store::from_records(vec![
            Record::Unit(c),
            Record::Unit(r),
            Record::Relation(Relation::new(RelKind::Rebuts, ur, uc)),
        ]);
        assert!(violations(
            &store,
            &Selection::from([(ur, Lod::L0)]),
            0,
            &constraints(vec![])
        )
        .is_empty());
    }

    #[test]
    fn an_open_contention_pins_every_position() {
        let a = claim("position a", vec![]);
        let ua = canonical_uid(&a);
        let b = claim("position b", vec![]);
        let ub = canonical_uid(&b);
        let k = Contention::new(
            ContentionId::new("k/x").unwrap(),
            ua,
            vec![ua, ub],
            Detected::new(
                DetectionKind::SupersessionFork,
                Hlc::new(0, 0, smysl_core::AgentId::new("tool:t").unwrap()),
            ),
        );
        let store = Store::from_records(vec![
            Record::Unit(a),
            Record::Unit(b),
            Record::Contention(k),
        ]);

        let one = Selection::from([(ua, Lod::L0)]);
        let v = violations(&store, &one, 0, &constraints(vec![]));
        assert_eq!(v[0].constraint(), "C4");

        let both = Selection::from([(ua, Lod::L0), (ub, Lod::L0)]);
        assert!(violations(&store, &both, 0, &constraints(vec![])).is_empty());
    }

    #[test]
    fn a_resolved_contention_pins_nothing() {
        let a = claim("position a", vec![]);
        let ua = canonical_uid(&a);
        let b = claim("position b", vec![]);
        let ub = canonical_uid(&b);
        let mut k = Contention::new(
            ContentionId::new("k/x").unwrap(),
            ua,
            vec![ua, ub],
            Detected::new(
                DetectionKind::SupersessionFork,
                Hlc::new(0, 0, smysl_core::AgentId::new("tool:t").unwrap()),
            ),
        );
        k.status = smysl_core::ContentionStatus::Resolved;
        let store = Store::from_records(vec![
            Record::Unit(a),
            Record::Unit(b),
            Record::Contention(k),
        ]);
        assert!(violations(
            &store,
            &Selection::from([(ua, Lod::L0)]),
            0,
            &constraints(vec![])
        )
        .is_empty());
    }

    #[test]
    fn a_pinned_unit_must_reach_l1() {
        let c = claim("the focus", vec![]);
        let uc = canonical_uid(&c);
        let store = Store::from_records(vec![Record::Unit(c)]);

        let absent = violations(&store, &Selection::new(), 0, &constraints(vec![uc]));
        assert_eq!(absent[0].constraint(), "C5");

        let shallow = violations(
            &store,
            &Selection::from([(uc, Lod::L0)]),
            0,
            &constraints(vec![uc]),
        );
        assert_eq!(shallow[0].constraint(), "C5");

        let ok = violations(
            &store,
            &Selection::from([(uc, Lod::L1)]),
            0,
            &constraints(vec![uc]),
        );
        assert!(ok.is_empty());
    }

    /// A gist-only unit cannot reach L1, so L0 satisfies the pin. Demanding otherwise
    /// would make some focus sets unsatisfiable for no reason.
    #[test]
    fn a_pinned_gist_only_unit_is_satisfied_at_l0() {
        let c = UnitCoreBuilder::new(KernelType::Claim, "gist only", Status::Speculative)
            .build()
            .unwrap();
        let uc = canonical_uid(&c);
        let store = Store::from_records(vec![Record::Unit(c)]);
        assert!(violations(
            &store,
            &Selection::from([(uc, Lod::L0)]),
            0,
            &constraints(vec![uc])
        )
        .is_empty());
    }

    #[test]
    fn a_unit_at_l1_needs_its_warrant() {
        let w = evidence("the warrant");
        let uw = canonical_uid(&w);
        let c = claim("the claim", vec![]);
        let uc = canonical_uid(&c);
        let store = Store::from_records(vec![
            Record::Unit(w),
            Record::Unit(c),
            Record::Relation(Relation::new(RelKind::Warrant, uc, uw)),
        ]);
        let v = violations(
            &store,
            &Selection::from([(uc, Lod::L1)]),
            0,
            &constraints(vec![]),
        );
        assert_eq!(v[0].constraint(), "C6");
        assert_eq!(warrants_of(&store, &uc), vec![uw]);
    }

    #[test]
    fn going_over_budget_is_c7() {
        let store = Store::new();
        let c = Constraints {
            pinned: BTreeSet::new(),
            budget: 100,
        };
        assert!(violations(&store, &Selection::new(), 100, &c).is_empty());
        let v = violations(&store, &Selection::new(), 101, &c);
        assert_eq!(v[0].constraint(), "C7");
    }

    #[test]
    fn every_violation_is_reported_not_just_the_first() {
        let e = evidence("the evidence");
        let ue = canonical_uid(&e);
        let c = claim("the claim", vec![ue]);
        let uc = canonical_uid(&c);
        let r = claim("the rebuttal", vec![]);
        let ur = canonical_uid(&r);
        let store = Store::from_records(vec![
            Record::Unit(e),
            Record::Unit(c),
            Record::Unit(r),
            Record::Relation(Relation::new(RelKind::Rebuts, ur, uc)),
        ]);
        let v = violations(
            &store,
            &Selection::from([(uc, Lod::L1)]),
            0,
            &constraints(vec![]),
        );
        let kinds: Vec<&str> = v.iter().map(|x| x.constraint()).collect();
        assert!(kinds.contains(&"C2") && kinds.contains(&"C3"), "{kinds:?}");
    }

    /// A reference to something the store does not have is pass 2's business, not the
    /// packer's - it cannot select what is not there.
    #[test]
    fn an_absent_reference_is_not_a_pack_violation() {
        let c = UnitCoreBuilder::new(KernelType::Claim, "the claim", Status::Inferred)
            .grounds([Uid::from_bytes([9; 32])])
            .body("a body")
            .build()
            .unwrap();
        let uc = canonical_uid(&c);
        let store = Store::from_records(vec![Record::Unit(c)]);
        assert!(violations(
            &store,
            &Selection::from([(uc, Lod::L1)]),
            0,
            &constraints(vec![])
        )
        .is_empty());
    }
}
