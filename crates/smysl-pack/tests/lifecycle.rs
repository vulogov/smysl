//! Rule R under 1.4's definition of a live rebuttal, end to end through `pack`.
//!
//! Found by rust_smysl: after the unit rebutting a claim was retracted, packing the claim still
//! pulled the retracted unit in as `C3`. Rule R obliges a selection to carry a claim's *live*
//! rebuttals, and until 1.4 nothing said what live meant.

use smysl_core::{
    canonical_uid, AgentId, Contention, ContentionId, Detected, DetectionKind, Hlc, KernelType,
    Record, RelKind, Relation, Resolution, ResolutionTarget, Status, Uid, UnitCoreBuilder,
    Withdrawal,
};
use smysl_graph::{salience, SalienceRequest, Store};
use smysl_pack::{pack, verify, PackRequest, Reason};

fn reviewer() -> AgentId {
    AgentId::new("human:reviewer").unwrap()
}

fn store(extra: impl FnOnce(Uid, Uid, &Relation) -> Vec<Record>) -> (Store, Uid, Uid) {
    let c = UnitCoreBuilder::new(KernelType::Claim, "the pool saturated", Status::Speculative)
        .build()
        .unwrap();
    let r = UnitCoreBuilder::new(
        KernelType::Claim,
        "the pool never exceeded half its size",
        Status::Speculative,
    )
    .build()
    .unwrap();
    let (uc, ur) = (canonical_uid(&c), canonical_uid(&r));
    let edge = Relation::new(RelKind::Rebuts, ur, uc);
    let mut records = vec![
        Record::Unit(c),
        Record::Unit(r),
        Record::Relation(edge.clone()),
    ];
    records.extend(extra(uc, ur, &edge));
    (Store::from_records(records), uc, ur)
}

/// Pack the claim alone: scoped to it and pinned, so anything else selected is an obligation.
fn pack_claim(s: &Store, uc: Uid) -> smysl_pack::Pack {
    let mut req = PackRequest::budget(10_000).focusing([uc]);
    req.scope = [uc].into();
    let p = pack(s, &salience(s, &SalienceRequest::default()), &req).expect("fits");
    assert!(verify(s, &p, &req).is_empty());
    p
}

#[test]
fn a_live_rebuttal_travels_with_its_claim() {
    let (s, uc, ur) = store(|_, _, _| vec![]);
    let p = pack_claim(&s, uc);
    assert_eq!(p.why.get(&ur), Some(&Reason::Rebuts(uc)));
}

#[test]
fn a_retracted_rebuttal_does_not() {
    let (s, uc, ur) =
        store(|_, ur, _| vec![Record::Relation(Relation::new(RelKind::Retracts, ur, ur))]);
    let p = pack_claim(&s, uc);
    assert!(p.selection.contains_key(&uc));
    assert!(!p.selection.contains_key(&ur), "{:?}", p.why);
}

#[test]
fn a_withdrawn_rebuttal_does_not() {
    let (s, uc, ur) = store(|_, _, edge| {
        vec![Record::Withdrawal(Withdrawal::new(
            edge.uid(),
            reviewer(),
            Hlc::new(1, 0, reviewer()),
        ))]
    });
    let p = pack_claim(&s, uc);
    assert!(!p.selection.contains_key(&ur), "{:?}", p.why);
}

/// C4: a resolved contention no longer pins the rebuttal's side in when only the rebuttal is
/// packed. (C3 still binds the other way — a resolution decides nothing.)
#[test]
fn a_resolved_contention_no_longer_pins_its_positions() {
    let contention = |uc: Uid, ur: Uid| {
        Contention::new(
            ContentionId::derive(DetectionKind::LiveRebuttal, &uc, &[uc, ur]),
            uc,
            vec![uc, ur],
            Detected::new(DetectionKind::LiveRebuttal, Hlc::new(1, 0, reviewer())),
        )
    };
    let pack_rebuttal = |s: &Store, ur: Uid| {
        let mut req = PackRequest::budget(10_000).focusing([ur]);
        req.scope = [ur].into();
        pack(s, &salience(s, &SalienceRequest::default()), &req).expect("fits")
    };

    let (open, uc, ur) = store(|uc, ur, _| vec![Record::Contention(contention(uc, ur))]);
    assert!(
        pack_rebuttal(&open, ur).selection.contains_key(&uc),
        "an open contention pins the claim"
    );

    let (resolved, uc, ur) = store(|uc, ur, _| {
        let k = contention(uc, ur);
        vec![
            Record::Resolution(Resolution::new(
                ResolutionTarget::Contention(k.id.clone()),
                reviewer(),
                Hlc::new(2, 0, reviewer()),
            )),
            Record::Contention(k),
        ]
    });
    assert!(!pack_rebuttal(&resolved, ur).selection.contains_key(&uc));
}
