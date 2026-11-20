//! SMYSL-3's reservations (TL-P1), found in a store that holds them.
//!
//! `SMY-E611`: a record 6 of a detection kind SMYSL-3 derives. A store refuses one at the door,
//! so this is a store built without it — a document checked as written, or a log from a peer
//! that predates TL-P1. `SMY-E612`: record 16 or `source` key 7, which no writer may emit and
//! every reader carries.

use smysl_check::{check, CheckOptions};
use smysl_core::types::{Contention, Detected, DetectionKind};
use smysl_core::{
    AgentId, Code, ContentionId, Hlc, KernelType, Record, SourceKind, SourceRef, Status, Uid,
    UnitCoreBuilder,
};
use smysl_graph::Store;

fn agent() -> AgentId {
    AgentId::new("human:vu").unwrap()
}

fn contention(kind: DetectionKind) -> Record {
    let uid = |n: u8| Uid::from_bytes([n; 32]);
    Record::Contention(Contention::new(
        ContentionId::derive(kind, &uid(1), &[uid(2), uid(3)]),
        uid(1),
        vec![uid(2), uid(3)],
        Detected::new(kind, Hlc::new(1, 0, agent())),
    ))
}

#[test]
fn a_written_rival_explanation_or_witness_conflict_is_e611() {
    for kind in [
        DetectionKind::RivalExplanation,
        DetectionKind::WitnessConflict,
    ] {
        let store = Store::from_records(vec![contention(kind)]);
        let report = check(&store, CheckOptions::strict());
        let e611: Vec<_> = report.iter().filter(|d| d.code == Code::E611).collect();
        assert_eq!(e611.len(), 1, "{kind}: {report:?}");
        assert!(
            e611[0].message.contains(kind.as_str()),
            "{}",
            e611[0].message
        );
        assert!(
            !report.iter().any(|d| d.code == Code::W409),
            "{kind} is a kind this build names, not an unknown code"
        );
    }
}

/// Rule E's two have no code, and keep none: the door is their whole enforcement, and §15
/// allocates `E611` for SMYSL-3's own two. A kind merge writes is not a finding at all.
#[test]
fn only_smysl_3s_kinds_are_e611() {
    for kind in [
        DetectionKind::SupersessionFork,
        DetectionKind::CommitmentFork,
        DetectionKind::DatingNotApplied,
        DetectionKind::TemporalInconsistency,
    ] {
        let store = Store::from_records(vec![contention(kind)]);
        let report = check(&store, CheckOptions::strict());
        assert!(!report.iter().any(|d| d.code == Code::E611), "{kind}");
    }
}

/// Record 16 is reserved, not merely unknown: one `E612`, and no `W014` describing it twice.
#[test]
fn a_record_16_is_e612_and_not_w014() {
    let store = Store::from_records(vec![Record::Unknown {
        code: smysl_core::types::record::code::HOLD,
        payload: vec![0xa0],
    }]);
    let report = check(&store, CheckOptions::strict());
    assert_eq!(
        report.iter().filter(|d| d.code == Code::E612).count(),
        1,
        "{report:?}"
    );
    assert!(!report.iter().any(|d| d.code == Code::W014), "{report:?}");

    // Any other unknown record is what it was: W014, and no E612.
    let store = Store::from_records(vec![Record::Unknown {
        code: 40,
        payload: vec![0xa0],
    }]);
    let report = check(&store, CheckOptions::strict());
    assert!(report.iter().any(|d| d.code == Code::W014));
    assert!(!report.iter().any(|d| d.code == Code::E612));
}

/// `source` key 7 is carried — it is inside the uid — and named on the unit that carries it.
#[test]
fn a_source_key_7_is_e612_on_its_unit() {
    let mut s = SourceRef::new(SourceKind::Metric, "series:prom:up");
    s.extra.insert(7, vec![0x19, 0x01, 0xf4]); // 500, a sub-millisecond remainder
    let u = UnitCoreBuilder::new(KernelType::Observation, "up is 1", Status::Measured)
        .source(s)
        .build()
        .unwrap();
    let uid = smysl_core::canonical_uid(&u);

    // Carried: the key survives an encode and decode, and the uid with it.
    let bytes = smysl_core::to_cbor(&Record::Unit(u.clone()));
    let (back, _) = smysl_core::from_cbor(&bytes).unwrap();
    match &back {
        Record::Unit(b) => {
            assert_eq!(smysl_core::canonical_uid(b), uid);
            assert!(b.source.as_ref().unwrap().extra.contains_key(&7));
        }
        other => panic!("{other:?}"),
    }

    let store = Store::from_records(vec![Record::Unit(u)]);
    let report = check(&store, CheckOptions::strict());
    let e612: Vec<_> = report.iter().filter(|d| d.code == Code::E612).collect();
    assert_eq!(e612.len(), 1, "{report:?}");
    assert_eq!(e612[0].uid(), Some(&uid));
}
