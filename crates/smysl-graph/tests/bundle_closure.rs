//! A bundle is the artifact designed to travel alone, so nothing in it may name what is absent.
//!
//! `Store::emit` ended in `_ => false`, and five classes of record fell through it: commitments,
//! schema declarations, attestations on relations, units named by a note or a position, and
//! records of a type this build cannot interpret. Each was a silent closure failure in the one
//! artifact whose whole purpose is to be readable by a recipient with nothing else to read it
//! against.

use smysl_core::{
    canonical_uid, from_cbor_seq, AgentId, Commit, Commitment, Hlc, KernelType, Record, Status,
    Uid, UnitCoreBuilder, View, ViewId,
};
use smysl_graph::Store;

fn agent() -> AgentId {
    AgentId::new("human:vu").unwrap()
}

fn claim(gist: &str) -> smysl_core::UnitCore {
    UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
        .build()
        .unwrap()
}

/// Every uid any bundled record names is itself in the bundle.
///
/// The property the RFC asks for, and the one that makes a bundle portable rather than merely
/// smaller than the store it came from.
#[test]
fn no_bundled_record_names_an_absent_unit() {
    let root = claim("the root claim a view points at, long enough to be a real body");
    let root_uid = canonical_uid(&root);
    // A unit reachable only through a commitment's note — not an edge, so the closure walk
    // cannot see it.
    let why = claim("the unit a commitment points at to say why it was settled");
    let why_uid = canonical_uid(&why);
    let a = agent();

    let store = Store::from_records(vec![
        Record::Unit(root.clone()),
        Record::Unit(why),
        Record::Commit(
            Commit::new(
                root_uid,
                Commitment::Canonical,
                a.clone(),
                Hlc::new(1, 0, a),
            )
            .with_note(why_uid),
        ),
    ]);

    let view = View::new(ViewId::new("v/b").unwrap(), "bundle").with_roots([root_uid]);
    let (bytes, report) = store.bundle_with_report(&view);
    let (records, _) = from_cbor_seq(&bytes).expect("a bundle must decode");

    let present: std::collections::BTreeSet<Uid> = records
        .iter()
        .filter_map(|r| match r {
            Record::Unit(u) => Some(canonical_uid(u)),
            _ => None,
        })
        .collect();

    assert!(
        present.contains(&why_uid),
        "a unit named by a commitment's note must travel with it"
    );
    assert_eq!(
        report.pulled_in_by_reference, 1,
        "and the report says it was not reachable by edges"
    );

    for r in &records {
        if let Record::Commit(c) = r {
            assert!(
                present.contains(&c.unit),
                "a commitment names an absent unit"
            );
            if let Some(n) = c.note {
                assert!(
                    present.contains(&n),
                    "a commitment's note names an absent unit"
                );
            }
        }
    }
}

/// A commitment travels with the unit it settles.
///
/// 1.7 added the axis; this arm of `emit` was never written, so every bundle silently dropped
/// how settled anything was — the one thing a ledger exists to carry.
#[test]
fn a_bundle_carries_its_commitments() {
    let u = claim("a settled claim, with a body long enough to be realistic");
    let uid = canonical_uid(&u);
    let a = agent();
    let store = Store::from_records(vec![
        Record::Unit(u),
        Record::Commit(Commit::new(
            uid,
            Commitment::Canonical,
            a.clone(),
            Hlc::new(1, 0, a),
        )),
    ]);

    let view = View::new(ViewId::new("v/c").unwrap(), "bundle").with_roots([uid]);
    let (bytes, _) = store.bundle_with_report(&view);
    let (records, _) = from_cbor_seq(&bytes).unwrap();

    assert!(
        records.iter().any(|r| matches!(r, Record::Commit(_))),
        "the commitment must be in the bundle"
    );
}

/// A record this build cannot interpret is kept, and counted.
#[test]
fn unknown_records_travel_and_are_reported() {
    let u = claim("an ordinary claim beside a record from a later release");
    let uid = canonical_uid(&u);
    let store = Store::from_records(vec![
        Record::Unit(u),
        Record::Unknown {
            code: 42,
            // One CBOR item: the framing is [code, body], so a body of three bare bytes
            // would be three items and the record would not decode.
            payload: vec![0xa0],
        },
    ]);

    let view = View::new(ViewId::new("v/u").unwrap(), "bundle").with_roots([uid]);
    let (bytes, report) = store.bundle_with_report(&view);
    let (records, _) = from_cbor_seq(&bytes).unwrap();

    assert_eq!(report.unknown_records, 1, "the sender is told how many");
    assert!(
        records
            .iter()
            .any(|r| matches!(r, Record::Unknown { code: 42, .. })),
        "rule X: a record this build cannot name is kept, not dropped"
    );
}
