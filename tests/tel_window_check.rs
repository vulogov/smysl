//! `check` on window evidence (RFC SMYSL-3 §9.2, §9.4, TL-P4 step 3): rule TW (`SMY-E603`) and
//! the windows of one key (`SMY-W614`), on units the materialiser built.

#![cfg(feature = "tel")]

use smysl::series::{MemBackend, Sample, SeriesInfo, Substrate};
use smysl::tel::evidence::{self, DEFAULT_ALPHA, DEFAULT_QS};
use smysl::{
    canonical_uid, AgentId, Attestation, Code, Hlc, Op, Record, Rung, Store, Uid, UnitCore,
};
use smysl_core::Decimal;

fn d(s: &str) -> Decimal {
    Decimal::parse_token(s).unwrap()
}

/// A substrate with one series, its exact and sketched windows, and the same exact window
/// again after a late sample.
fn windows() -> (UnitCore, UnitCore, UnitCore) {
    let mut sub = Substrate::new(MemBackend::default());
    let (series, inst) = (Uid::from_bytes([1; 32]), Uid::from_bytes([2; 32]));
    let info = SeriesInfo {
        scheme: "prom".into(),
        key: "lat".into(),
        class: "gauge".into(),
    };
    let samples: Vec<Sample> = (0..50u64)
        .map(|i| Sample::new(i * 1_000, None, d(&format!("{}e-3", i * 37 % 100 + 1)), 0).unwrap())
        .collect();
    sub.add(&series, &inst, &info, &samples).unwrap();
    let exact = evidence::materialise(&sub, &series, &inst, 0, 60_000)
        .unwrap()
        .unit;
    let qs: Vec<Decimal> = DEFAULT_QS.iter().map(|q| d(q)).collect();
    let sketched =
        evidence::materialise_sketched(&sub, &series, &inst, 0, 60_000, d(DEFAULT_ALPHA), &qs)
            .unwrap()
            .unit;
    sub.add(
        &series,
        &inst,
        &info,
        &[Sample::new(55_000, None, d("9"), 0).unwrap()],
    )
    .unwrap();
    let late = evidence::materialise(&sub, &series, &inst, 0, 60_000)
        .unwrap()
        .unit;
    (exact, sketched, late)
}

fn attest(u: &UnitCore, op: Op, rung: Rung) -> Record {
    let agent = AgentId::new("tool:smysl-tel").unwrap();
    Record::Attestation(Attestation::new(
        canonical_uid(u),
        agent.clone(),
        op,
        rung,
        Hlc::new(1, 0, agent),
    ))
}

fn codes(records: Vec<Record>) -> Vec<(Code, String)> {
    let mut all = vec![Record::SchemaDecl(smysl::schemas::tel::decl())];
    all.extend(records);
    let store = Store::from_records(all);
    smysl::check(&store, smysl::CheckOptions::default())
        .diagnostics
        .into_iter()
        .filter(|d| [Code::E603, Code::W614].contains(&d.code))
        .map(|d| (d.code, d.message.clone()))
        .collect()
}

#[test]
fn windows_attested_as_rule_tw_says_are_clean() {
    let (exact, sketched, _) = windows();
    let r = codes(vec![
        Record::Unit(exact.clone()),
        attest(&exact, Op::Imported, Rung::Computed),
        Record::Unit(sketched.clone()),
        attest(&sketched, Op::Transformed, Rung::Computed),
    ]);
    assert!(r.is_empty(), "{r:?}");
}

#[test]
fn a_window_attested_otherwise_is_e603() {
    let (exact, sketched, _) = windows();
    // A sketch attested as an import: it claims to have transcribed what it computed.
    let r = codes(vec![
        Record::Unit(sketched.clone()),
        attest(&sketched, Op::Imported, Rung::Computed),
    ]);
    assert_eq!(r.len(), 1, "{r:?}");
    assert_eq!(r[0].0, Code::E603);
    assert!(r[0].1.contains("Transformed"), "{}", r[0].1);

    // An exact window at a status it cannot have.
    let mut inflated = exact.clone();
    inflated.status = smysl::Status::Cited;
    let r = codes(vec![Record::Unit(inflated)]);
    assert_eq!(r.len(), 1, "{r:?}");
    assert!(r[0].1.contains("is measured, not cited"), "{}", r[0].1);

    // An exact window from a document: rung wrong.
    let r = codes(vec![
        Record::Unit(exact.clone()),
        attest(&exact, Op::Imported, Rung::Document),
    ]);
    assert_eq!(r.len(), 1, "{r:?}");
}

#[test]
fn windows_of_one_key_that_disagree_are_w614_and_both_kept() {
    let (exact, _, late) = windows();
    assert_ne!(canonical_uid(&exact), canonical_uid(&late));
    let r = codes(vec![Record::Unit(exact), Record::Unit(late)]);
    assert_eq!(r.len(), 1, "{r:?}");
    assert_eq!(r[0].0, Code::W614);
    assert!(r[0].1.starts_with("2 windows"), "{}", r[0].1);
}
