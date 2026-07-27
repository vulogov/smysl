//! `smysl-pack` - budget-bounded, closure-complete selection (§8, §18).
//!
//! This is what a consuming agent calls **instead of** asking a model to summarise. Same
//! graph, same budget, same thread yields identical bytes, and no inference happens
//! anywhere - which is what makes summarisation precomputation (P4) rather than a cost
//! paid again at every hop.
//!
//! Seven constraints hold on every pack (§8). Six are closure obligations; the seventh is
//! the budget. The interesting one is C3: a selected claim's rebuttals are selected too,
//! always. If the budget cannot hold both, the claim is dropped - and if the claim is
//! pinned, packing **fails** with the minimum feasible budget rather than emitting the
//! claim alone. A one-sided pack is the failure mode prose transport already has.
//!
//! Every pack emits a `packinfo` recording budget, use, what was dropped and why, what was
//! degraded, the optimality gap, and the cost estimator that produced it. Truncation is
//! therefore self-describing; a pack without one may be assumed complete.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub mod closure;
pub mod constraints;
pub mod cost;
pub mod solve;

pub use closure::Reason;
pub use constraints::{violations, Constraints, Selection, Violation};
pub use cost::{available_levels, value, Estimator};
pub use smysl_core::error::PackError;
pub use solve::{pack, verify, Pack, PackRequest, IMPROVEMENT_PASSES};

/// Identifier of the default cost model, as recorded in every `packinfo` (D-2).
pub const DEFAULT_ESTIMATOR: &str = "smysl/utf8-div4";

/// `cost(t) = ceil(utf8_len(t)/4) + 2` (D-2).
///
/// The token count itself lives in `smysl-core`, because granularity bounds are expressed
/// in the same unit; the `+ 2` here is the per-item framing overhead a pack pays.
pub fn cost(text: &str) -> u32 {
    smysl_core::tokens(text) + 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{
        canonical_uid, KernelType, Lod, PackMode, Record, RelKind, Relation, SourceKind, SourceRef,
        Status, Uid, UnitCore, UnitCoreBuilder,
    };
    use smysl_graph::{salience, SalienceRequest, Store};

    fn evidence(gist: &str) -> UnitCore {
        UnitCoreBuilder::new(KernelType::Evidence, gist, Status::Measured)
            .source(SourceRef::new(SourceKind::Metric, "m"))
            .body("a body of moderate length, worth buying when the budget allows")
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
            .body("a body of moderate length, worth buying when the budget allows")
            .build()
            .unwrap()
    }

    /// evidence <- claim, with a rebuttal of the claim.
    fn contested() -> (Store, Uid, Uid, Uid) {
        let e = evidence("the measurement");
        let ue = canonical_uid(&e);
        let c = claim("the claim", vec![ue]);
        let uc = canonical_uid(&c);
        let r = claim("the rebuttal", vec![ue]);
        let ur = canonical_uid(&r);
        (
            Store::from_records(vec![
                Record::Unit(e),
                Record::Unit(c),
                Record::Unit(r),
                Record::Relation(Relation::new(RelKind::Rebuts, ur, uc)),
            ]),
            ue,
            uc,
            ur,
        )
    }

    fn sal(store: &Store) -> smysl_graph::SalienceReport {
        salience(store, &SalienceRequest::default())
    }

    fn run(store: &Store, req: &PackRequest) -> Pack {
        pack(store, &sal(store), req).expect("the floor fits")
    }

    #[test]
    fn the_estimator_id_is_recorded() {
        let (store, _, _, _) = contested();
        let p = run(&store, &PackRequest::budget(10_000));
        assert_eq!(p.info.estimator, DEFAULT_ESTIMATOR);
        assert_eq!(cost("abcd"), 3);
    }

    #[test]
    fn a_generous_budget_takes_everything() {
        let (store, ue, uc, ur) = contested();
        let p = run(&store, &PackRequest::budget(10_000));
        assert_eq!(p.len(), 3);
        for u in [ue, uc, ur] {
            assert_eq!(p.selection[&u], Lod::L1, "everything at full depth");
        }
        assert!(p.info.is_complete());
    }

    /// Rule R in the ordinary case: taking the claim takes the rebuttal with it.
    #[test]
    fn selecting_a_claim_brings_its_rebuttal() {
        let (store, _, uc, ur) = contested();
        let req = PackRequest::budget(10_000);
        let p = run(&store, &req);
        assert!(p.selection.contains_key(&uc) && p.selection.contains_key(&ur));
        assert!(verify(&store, &p, &req).is_empty());
    }

    /// Rule R in the hard case: a budget too small for both drops the claim rather than
    /// presenting it unopposed.
    #[test]
    fn a_budget_too_small_for_both_drops_the_claim() {
        let (store, _, uc, ur) = contested();
        let e = Estimator::default();
        let claim_only = e.unit(&store.get(&uc).unwrap().core, Lod::L0);

        let req = PackRequest::budget(claim_only + 1);
        let p = run(&store, &req);
        assert!(
            !p.selection.contains_key(&uc) || p.selection.contains_key(&ur),
            "the claim appeared without its rebuttal"
        );
        assert!(verify(&store, &p, &req).is_empty());
    }

    /// And if the claim is pinned, packing fails rather than degrading (§18.4).
    #[test]
    fn an_infeasible_floor_fails_with_the_minimum_budget() {
        let (store, _, uc, _) = contested();
        let req = PackRequest::budget(1).focusing([uc]);
        let err = pack(&store, &sal(&store), &req).unwrap_err();
        match err {
            PackError::Infeasible { budget, required } => {
                assert_eq!(budget, 1);
                assert!(required > 1);
                // The reported minimum is exactly what succeeds.
                let ok = PackRequest::budget(required).focusing([uc]);
                assert!(pack(&store, &sal(&store), &ok).is_ok());
            }
            other => panic!("expected Infeasible, got {other:?}"),
        }
    }

    #[test]
    fn the_reported_minimum_is_tight() {
        let (store, _, uc, _) = contested();
        let required =
            match pack(&store, &sal(&store), &PackRequest::budget(1).focusing([uc])).unwrap_err() {
                PackError::Infeasible { required, .. } => required,
                other => panic!("{other:?}"),
            };
        assert!(
            pack(
                &store,
                &sal(&store),
                &PackRequest::budget(required - 1).focusing([uc])
            )
            .is_err(),
            "one token less must not fit"
        );
    }

    #[test]
    fn a_focus_unit_that_is_not_in_the_store_is_an_error() {
        let (store, _, _, _) = contested();
        let ghost = Uid::from_bytes([9; 32]);
        let err = pack(
            &store,
            &sal(&store),
            &PackRequest::budget(10_000).focusing([ghost]),
        )
        .unwrap_err();
        assert!(matches!(err, PackError::FocusAbsent { .. }));
        assert_eq!(err.code(), smysl_core::Code::E201);
    }

    #[test]
    fn a_pinned_unit_reaches_l1() {
        let (store, _, uc, _) = contested();
        let req = PackRequest::budget(10_000).focusing([uc]);
        let p = run(&store, &req);
        assert_eq!(p.selection[&uc], Lod::L1);
        assert_eq!(p.why[&uc], Reason::Focus);
    }

    #[test]
    fn a_pack_never_exceeds_its_budget() {
        let (store, _, _, _) = contested();
        for budget in [10u64, 20, 40, 80, 160, 320] {
            let p = run(&store, &PackRequest::budget(budget));
            assert!(p.used() <= budget, "{} over {budget}", p.used());
        }
    }

    #[test]
    fn a_zero_budget_packs_nothing() {
        let (store, _, _, _) = contested();
        let p = run(&store, &PackRequest::budget(0));
        assert!(p.is_empty());
        assert_eq!(p.used(), 0);
    }

    #[test]
    fn the_lod_cap_is_respected() {
        let (store, _, _, _) = contested();
        let p = run(&store, &PackRequest::budget(10_000).capped(Lod::L0));
        assert!(p.selection.values().all(|l| *l == Lod::L0));
    }

    #[test]
    fn packinfo_records_what_was_dropped_and_why() {
        let (store, _, _, _) = contested();
        let p = run(&store, &PackRequest::budget(12));
        assert!(!p.info.dropped.is_empty());
        assert!(!p.info.drop_histogram().is_empty());
        assert!(!p.info.is_complete(), "a truncated pack says so");
    }

    #[test]
    fn explain_attributes_every_unit() {
        let (store, ue, uc, ur) = contested();
        let p = run(&store, &PackRequest::budget(10_000).focusing([uc]));
        assert_eq!(p.why[&uc], Reason::Focus);
        assert_eq!(p.why[&ur], Reason::Rebuts(uc));
        assert_eq!(p.why[&ue], Reason::GroundOf(uc));
        assert_eq!(p.forced().len(), 3);
    }

    #[test]
    fn the_optimality_mode_and_gap_are_recorded() {
        let (store, _, _, _) = contested();
        let p = run(&store, &PackRequest::budget(10_000));
        assert_eq!(p.info.optimality.mode, PackMode::Greedy);
        assert_eq!(p.info.optimality.gap, 0.0, "nothing was left behind");
        assert!(run(&store, &PackRequest::budget(40)).info.optimality.gap >= 0.0);
    }

    #[test]
    fn scope_restricts_what_may_be_packed() {
        let (store, ue, uc, _) = contested();
        let p = run(&store, &PackRequest::budget(10_000).scoped([ue]));
        assert!(p.selection.contains_key(&ue));
        assert!(!p.selection.contains_key(&uc), "out of scope");
    }

    #[test]
    fn packing_is_deterministic() {
        let (store, _, _, _) = contested();
        let req = PackRequest::budget(60);
        assert_eq!(run(&store, &req), run(&store, &req));
    }

    /// Record arrival order must not reach the selection - dense ids follow ascending uid,
    /// and every tie-break is total.
    #[test]
    fn record_order_does_not_change_the_pack() {
        let (store, _, _, _) = contested();
        let mut reversed: Vec<Record> = store.iter().cloned().collect();
        reversed.reverse();
        let other = Store::from_records(reversed);
        let req = PackRequest::budget(60);
        assert_eq!(run(&store, &req).selection, run(&other, &req).selection);
    }

    #[test]
    fn an_empty_store_packs_to_nothing() {
        let store = Store::new();
        let p = pack(&store, &sal(&store), &PackRequest::budget(1000)).unwrap();
        assert!(p.is_empty());
        assert!(p.info.is_complete());
    }
}
