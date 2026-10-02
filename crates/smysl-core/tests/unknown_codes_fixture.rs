//! `fixtures/wire/F14-unknown-codes.cbor` (1.9): a store carrying codes no build knows.
//!
//! 1.9 opened four enumerations — source kind, thread schema, step role and detection kind — so
//! that a code a reader cannot name is kept and re-encoded unchanged rather than failing the
//! decode. This fixture is what the Python, JavaScript and Go suites check that against: all
//! three glob `fixtures/wire` and assert every record re-encodes to the bytes it arrived in, so
//! adding the file *is* the cross-implementation test.
//!
//! It cannot be built from surface text. `kind:` and the schema and role words parse against the
//! named set, so an unknown code has no spelling — which is the point: an open enumeration is
//! the wire's business. The fixture is therefore assembled from known records and then has four
//! code bytes rewritten, which is exactly the shape a future release's store would have.
//!
//! The codes chosen are deliberate. 9 is beyond the source kinds 1.9 defines, 7 beyond the
//! thread schemas, 30 is inside the range A-8.2 reserves for roles, and 6 is beyond the
//! detection kinds. None is 255, which is reserved in all five enumerations and never assigned.
//!
//! `SMYSL_BLESS=1` rewrites the file. That should be a decision: it changes what three other
//! implementations are checked against.

use std::path::{Path, PathBuf};

use smysl_core::{
    from_cbor_seq, to_cbor_seq, AgentId, Contention, ContentionId, Detected, DetectionKind, Hlc,
    KernelType, Record, Role, SourceKind, SourceRef, Status, Step, Thread, ThreadId, ThreadSchema,
    Uid, UnitCoreBuilder,
};

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/wire/F14-unknown-codes.cbor")
}

fn build() -> Vec<u8> {
    let who = AgentId::new("human:vu").unwrap();

    // A unit whose source kind is 9.
    let unit = UnitCoreBuilder::new(
        KernelType::Claim,
        "a claim whose source kind this build cannot name",
        Status::Cited,
    )
    .source(
        SourceRef::new(SourceKind::Doc, "x://later")
            .with_unknown_kind(9)
            .expect("9 is neither reserved nor a kind this build knows"),
    )
    .build()
    .unwrap();
    let uid = smysl_core::canonical_uid(&unit);

    // A thread whose schema is 7, with a step whose role is 30.
    let mut thread = Thread::new(
        ThreadId::new("t/later").unwrap(),
        ThreadSchema::Analysis,
        who.clone(),
        "a thread whose schema this build cannot name",
        Hlc::new(1_726_500_000_000, 0, who.clone()),
    )
    .with_unknown_schema(7)
    .expect("7 is neither reserved nor a schema this build knows");
    thread.steps.push(
        Step::new(Role::Context, uid)
            .with_unknown_role(30)
            .expect("30 is neither reserved nor a role this build knows"),
    );

    // A contention whose detection kind is 6.
    let contention = Contention::new(
        ContentionId::new("k/later").unwrap(),
        uid,
        vec![Uid::from_bytes([2; 32])],
        Detected::new(
            DetectionKind::SupersessionFork,
            Hlc::new(1_726_500_000_001, 0, who),
        )
        .with_unknown_kind(6)
        .expect("6 is neither reserved nor a detection kind this build knows"),
    );

    to_cbor_seq(&[
        Record::Unit(unit),
        Record::Thread(thread),
        Record::Contention(contention),
    ])
}

/// The committed fixture is what this test builds, and every record in it round-trips.
#[test]
fn the_unknown_code_fixture_is_current_and_round_trips() {
    let built = build();
    let path = fixture_path();

    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::write(&path, &built).expect("writing the fixture");
    }

    let on_disk = std::fs::read(&path).expect("fixtures/wire/F14-unknown-codes.cbor is missing");
    assert_eq!(
        on_disk, built,
        "fixtures/wire/F14-unknown-codes.cbor no longer matches its source; \
         SMYSL_BLESS=1 rewrites it, and that changes what Python, JavaScript and Go are checked against"
    );

    // The obligation the other three suites assert, asserted here too.
    let (records, _) = from_cbor_seq(&on_disk).expect("a store of unknown codes must decode");
    assert_eq!(records.len(), 3, "a unit, a thread and a contention");
    assert_eq!(
        to_cbor_seq(&records),
        on_disk,
        "the store must re-encode to the bytes it arrived in"
    );
}

/// Every unknown code arrived as itself, and nothing was normalised to 255.
#[test]
fn every_unknown_code_is_read_as_unknown_and_kept() {
    let (records, _) = from_cbor_seq(&build()).unwrap();

    for r in &records {
        match r {
            Record::Unit(u) => {
                let s = u.source.as_ref().expect("the unit has a source");
                assert_eq!(s.kind, SourceKind::Unknown);
                assert_eq!(s.kind_code(), 9, "the source kind code is kept verbatim");
            }
            Record::Thread(t) => {
                assert_eq!(t.schema, ThreadSchema::Unknown);
                assert_eq!(t.schema_code(), 7, "the schema code is kept verbatim");
                let st = &t.steps[0];
                assert_eq!(st.role, Role::Unknown);
                assert_eq!(st.role_code(), 30, "the role code is kept verbatim");
            }
            Record::Contention(c) => {
                assert_eq!(c.detected.kind, DetectionKind::Unknown);
                assert_eq!(
                    c.detected.kind_code(),
                    6,
                    "the detection code is kept verbatim"
                );
            }
            other => panic!("unexpected record in the fixture: {other:?}"),
        }
    }
}
