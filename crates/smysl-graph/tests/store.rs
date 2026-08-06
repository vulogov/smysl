//! The SM-P3 gate.
//!
//! - An index rebuilt from the log alone is **byte-identical** to the one maintained
//!   while appending. If the two paths ever disagree, they disagree about the graph.
//! - A truncated log parses up to the last complete record, because the log is
//!   append-only and may be read while a writer is mid-append.
//! - Altering a stored unit is caught, because a unit cannot be changed without changing
//!   its uid.

use std::collections::BTreeSet;

use smysl_core::{
    canonical_uid, AgentId, Attestation, Contention, ContentionId, Detected, DetectionKind, Hlc,
    KernelType, Op, Record, RelKind, Relation, Rung, SourceKind, SourceRef, Status, Thread,
    ThreadId, ThreadSchema, Uid, UnitCore, UnitCoreBuilder, View, ViewId,
};
use smysl_graph::{EdgeSet, Store, StoreOptions};

fn agent() -> AgentId {
    AgentId::new("human:vladimir").unwrap()
}

fn claim(gist: &str, grounds: Vec<Uid>) -> UnitCore {
    let status = if grounds.is_empty() {
        Status::Speculative
    } else {
        Status::Inferred
    };
    UnitCoreBuilder::new(KernelType::Claim, gist, status)
        .grounds(grounds)
        .build()
        .unwrap()
}

fn evidence(gist: &str) -> UnitCore {
    UnitCoreBuilder::new(KernelType::Evidence, gist, Status::Measured)
        .source(SourceRef::new(SourceKind::Metric, "pool.wait_ms"))
        .build()
        .unwrap()
}

/// evidence <- claim, with a rebuttal, an attestation, a thread, a view and a contention.
fn corpus() -> Vec<Record> {
    let e = evidence("Pool wait rose from 2 ms to 310 ms.");
    let e_uid = canonical_uid(&e);
    let c = claim("The eu-west pool is saturated.", vec![e_uid]);
    let c_uid = canonical_uid(&c);
    let r = claim(
        "The canary ran the same configuration cleanly.",
        vec![e_uid],
    );
    let r_uid = canonical_uid(&r);

    vec![
        Record::Unit(e),
        Record::Unit(c),
        Record::Unit(r),
        Record::Attestation(Attestation::new(
            c_uid,
            agent(),
            Op::Authored,
            Rung::Document,
            Hlc::new(1, 0, agent()),
        )),
        Record::Relation(Relation::new(RelKind::Rebuts, r_uid, c_uid).with_weight(0.6)),
        Record::Thread(
            Thread::new(
                ThreadId::new("t/brief").unwrap(),
                ThreadSchema::Brief,
                agent(),
                "pool saturation, contested",
                Hlc::new(2, 0, agent()),
            )
            .with_steps([smysl_core::Step::new(smysl_core::Role::BottomLine, c_uid)]),
        ),
        Record::View(
            View::new(ViewId::new("v/incident").unwrap(), "incident-brief")
                .with_roots([c_uid])
                .with_threads([ThreadId::new("t/brief").unwrap()]),
        ),
        Record::Contention(Contention::new(
            ContentionId::new("k/pool").unwrap(),
            c_uid,
            vec![c_uid, r_uid],
            Detected::new(DetectionKind::LiveRebuttal, Hlc::new(3, 0, agent())),
        )),
    ]
}

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-p3-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("store.smy")
}

// ---------------------------------------------------------------------------
// Log
// ---------------------------------------------------------------------------

#[test]
fn an_appended_store_reads_back_identically() {
    let path = tmp("append");
    let mut s = Store::open(&path).unwrap();
    let report = s.append(&corpus()).unwrap();
    assert_eq!(report.added, 8);
    assert_eq!(report.duplicates, 0);
    assert!(report.bytes_written > 0);

    let (back, open) = Store::open_with(&path, StoreOptions::default()).unwrap();
    assert_eq!(back.len(), s.len());
    assert_eq!(back.log_len(), s.log_len());
    assert_eq!(back.log_hash(), s.log_hash());
    assert_eq!(open.trailing_bytes, 0);
    assert_eq!(
        back.iter().cloned().collect::<Vec<_>>(),
        s.iter().cloned().collect::<Vec<_>>()
    );
}

/// The store is a grow-only set, so a duplicated delivery must cost nothing (rule U).
#[test]
fn appending_the_same_records_twice_is_idempotent() {
    let mut s = Store::from_records(corpus());
    let before = s.len();
    let report = s.append(&corpus()).unwrap();
    assert_eq!(report.added, 0);
    assert_eq!(report.duplicates, 8);
    assert_eq!(s.len(), before);
}

#[test]
fn opening_a_missing_log_yields_an_empty_store() {
    let path = tmp("missing").with_file_name("nope.smy");
    let s = Store::open(&path).unwrap();
    assert!(s.is_empty());
    assert_eq!(s.log_len(), 0);
}

/// A writer may be mid-append. Everything up to the last complete record must survive.
#[test]
fn a_truncated_log_parses_up_to_the_last_complete_record() {
    let path = tmp("truncated");
    let mut s = Store::open(&path).unwrap();
    s.append(&corpus()).unwrap();

    let full = std::fs::read(&path).unwrap();
    let complete =
        smysl_core::to_cbor(&corpus()[0]).len() + smysl_core::to_cbor(&corpus()[1]).len();

    for extra in 1..12 {
        std::fs::write(&path, &full[..complete + extra]).unwrap();
        let (s, open) = Store::open_with(&path, StoreOptions::default()).unwrap();
        assert_eq!(s.len(), 2, "cut at +{extra} lost a complete record");
        assert_eq!(open.trailing_bytes as usize, extra);
        assert_eq!(open.report.count(smysl_core::Code::W110), 1);
    }
}

#[test]
fn log_bytes_are_the_canonical_encoding_of_the_records() {
    let s = Store::from_records(corpus());
    assert_eq!(
        s.log_bytes(),
        smysl_core::to_cbor_seq(&s.iter().cloned().collect::<Vec<_>>())
    );
    assert_eq!(s.log_len() as usize, s.log_bytes().len());
}

// ---------------------------------------------------------------------------
// Index
// ---------------------------------------------------------------------------

/// The SM-P3 gate.
#[test]
fn a_rebuilt_index_is_byte_identical_to_the_maintained_one() {
    let mut incremental = Store::new();
    for r in corpus() {
        incremental.append(&[r]).unwrap();
    }
    let maintained = incremental.index().to_bytes();

    let mut rebuilt_store = Store::from_records(corpus());
    let rebuilt = rebuilt_store.reindex().to_bytes();

    assert_eq!(
        maintained.len(),
        rebuilt.len(),
        "the two index paths disagree about size"
    );
    assert_eq!(maintained, rebuilt, "the two index paths disagree");
}

#[test]
fn the_index_survives_a_write_and_read() {
    let path = tmp("sidecar");
    let mut s = Store::open(&path).unwrap();
    s.append(&corpus()).unwrap();
    s.write_index().unwrap();

    let sidecar = std::fs::read(Store::index_path(&path)).unwrap();
    let back = smysl_graph::Index::from_bytes(&sidecar).unwrap();
    assert_eq!(back, s.index());
    assert!(back.matches(s.log_len(), s.log_hash()));
}

#[test]
fn a_current_sidecar_is_not_rebuilt() {
    let path = tmp("current");
    let mut s = Store::open(&path).unwrap();
    s.append(&corpus()).unwrap();
    s.write_index().unwrap();

    let (_, open) = Store::open_with(&path, StoreOptions::default()).unwrap();
    assert!(!open.index_rebuilt);
    assert!(open.report.is_empty());
}

/// A stale index is a rebuild, never a failure: the log is the authority.
#[test]
fn a_stale_sidecar_triggers_a_rebuild_rather_than_an_error() {
    let path = tmp("stale");
    let mut s = Store::open(&path).unwrap();
    s.append(&corpus()[..2]).unwrap();
    s.write_index().unwrap();
    s.append(&corpus()[2..]).unwrap();

    let (back, open) = Store::open_with(&path, StoreOptions::default()).unwrap();
    assert!(open.index_rebuilt);
    assert_eq!(open.report.count(smysl_core::Code::W110), 1);
    assert_eq!(back.len(), 8, "the rebuild sees the whole log");
}

#[test]
fn a_corrupt_sidecar_triggers_a_rebuild_rather_than_an_error() {
    let path = tmp("corrupt");
    let mut s = Store::open(&path).unwrap();
    s.append(&corpus()).unwrap();
    s.write_index().unwrap();
    std::fs::write(Store::index_path(&path), b"garbage").unwrap();

    let (back, open) = Store::open_with(&path, StoreOptions::default()).unwrap();
    assert!(open.index_rebuilt);
    assert_eq!(back.len(), 8);
}

#[test]
fn reindexing_preserves_the_store() {
    let mut s = Store::from_records(corpus());
    let before: Vec<Record> = s.iter().cloned().collect();
    let uids: BTreeSet<Uid> = s.units().map(|(u, _)| *u).collect();
    s.reindex();
    assert_eq!(s.iter().cloned().collect::<Vec<_>>(), before);
    assert_eq!(s.units().map(|(u, _)| *u).collect::<BTreeSet<_>>(), uids);
}

#[test]
fn the_index_records_offsets_that_locate_each_unit() {
    let s = Store::from_records(corpus());
    let ix = s.index();
    let bytes = s.log_bytes();
    for (uid, e) in &ix.entries {
        let slice = &bytes[e.offset as usize..e.offset as usize + e.len as usize];
        let (record, n) = smysl_core::from_cbor(slice).unwrap();
        assert_eq!(n, slice.len());
        assert_eq!(canonical_uid(record.as_unit().unwrap()), *uid);
        assert_eq!(e.type_code, 1);
    }
    assert_eq!(ix.entries.len(), 3);
}

#[test]
fn the_index_records_threads_and_contentions() {
    let ix = Store::from_records(corpus()).index();
    assert_eq!(ix.threads.len(), 1);
    assert_eq!(ix.contentions.len(), 1);
    assert_eq!(ix.unit_count(), 3);
}

// ---------------------------------------------------------------------------
// Integrity
// ---------------------------------------------------------------------------

/// A unit cannot be altered without changing its uid, so tampering shows up as an index
/// that no longer describes the log.
#[test]
fn altering_a_stored_unit_is_caught() {
    let path = tmp("tamper");
    let mut s = Store::open(&path).unwrap();
    s.append(&corpus()).unwrap();
    s.write_index().unwrap();

    // Flip a byte inside the first unit's gist, then restore the header the index
    // checks, so the tamper has to be caught by content rather than by length.
    let mut bytes = std::fs::read(&path).unwrap();
    let at = bytes
        .windows(4)
        .position(|w| w == b"Pool")
        .expect("the gist text is in the log");
    bytes[at] = b'W';
    std::fs::write(&path, &bytes).unwrap();

    let (_, open) = Store::open_with(&path, StoreOptions::strict()).unwrap();
    assert!(open.index_rebuilt, "the log no longer matches its index");

    // With the sidecar rewritten to match, the altered unit has a different uid, so
    // everything that referenced the original now dangles.
    let (tampered, _) = Store::open_with(&path, StoreOptions::default()).unwrap();
    let mut report = smysl_core::Report::new();
    tampered.report_dangling(&mut report);
    assert!(
        report.count(smysl_core::Code::E060) > 0,
        "content addressing did not surface the alteration"
    );
}

#[test]
fn verify_hashes_reports_a_unit_the_index_does_not_know() {
    let s = Store::from_records(corpus());
    let mut ix = s.index();
    let victim = *ix.entries.keys().next().unwrap();
    ix.entries.remove(&victim);

    let mut report = smysl_core::Report::new();
    s.verify_against(&ix, &mut report);
    assert!(report.count(smysl_core::Code::E070) > 0);
}

#[test]
fn a_clean_store_verifies_without_diagnostics() {
    let s = Store::from_records(corpus());
    let ix = s.index();
    let mut report = smysl_core::Report::new();
    s.verify_against(&ix, &mut report);
    assert!(report.is_empty(), "{report}");
}

#[test]
fn a_dangling_reference_is_reported() {
    let orphan = claim("grounded on nothing here", vec![Uid::from_bytes([9; 32])]);
    let s = Store::from_records(vec![Record::Unit(orphan)]);
    let mut report = smysl_core::Report::new();
    s.report_dangling(&mut report);
    assert_eq!(report.count(smysl_core::Code::E060), 1);
}

// ---------------------------------------------------------------------------
// Derived structure
// ---------------------------------------------------------------------------

#[test]
fn units_are_addressable_by_uid() {
    let s = Store::from_records(corpus());
    let e = evidence("Pool wait rose from 2 ms to 310 ms.");
    let uid = canonical_uid(&e);
    assert!(s.contains_uid(&uid));
    assert_eq!(s.get(&uid).unwrap().core, e);
    assert!(s.get(&Uid::from_bytes([0; 32])).is_none());
}

/// Attestations are not hashed, so they attach to their unit rather than forming one.
#[test]
fn attestations_attach_to_their_unit() {
    let s = Store::from_records(corpus());
    let c = claim(
        "The eu-west pool is saturated.",
        vec![canonical_uid(&evidence(
            "Pool wait rose from 2 ms to 310 ms.",
        ))],
    );
    let u = s.get(&canonical_uid(&c)).unwrap();
    assert_eq!(u.attestations.len(), 1);
    assert_eq!(u.corroboration_groups(), 1);
}

/// Delivery may be out of order (rule U), so an attestation that arrives before its unit
/// must still find it.
#[test]
fn an_attestation_delivered_before_its_unit_still_attaches() {
    let c = claim("a claim", vec![]);
    let uid = canonical_uid(&c);
    let a = Attestation::new(
        uid,
        agent(),
        Op::Authored,
        Rung::Document,
        Hlc::new(1, 0, agent()),
    );

    let mut s = Store::new();
    s.append(&[Record::Attestation(a)]).unwrap();
    assert!(s.get(&uid).is_none());
    s.append(&[Record::Unit(c)]).unwrap();
    assert_eq!(s.get(&uid).unwrap().attestations.len(), 1);
}

#[test]
fn threads_are_keyed_by_id_and_owner_with_last_write_winning() {
    let c = claim("a claim", vec![]);
    let uid = canonical_uid(&c);
    let other = AgentId::new("model:openai/gpt").unwrap();
    let mk = |owner: &AgentId, gist: &str, t: u64| {
        Record::Thread(Thread::new(
            ThreadId::new("t/x").unwrap(),
            ThreadSchema::Brief,
            owner.clone(),
            gist,
            Hlc::new(t, 0, owner.clone()),
        ))
    };

    let mut s = Store::from_records(vec![Record::Unit(c)]);
    s.append(&[
        mk(&agent(), "first", 1),
        mk(&other, "theirs", 1),
        mk(&agent(), "second", 2),
        mk(&agent(), "stale", 0),
    ])
    .unwrap();

    assert_eq!(s.threads().count(), 2, "two owners are two registers");
    let mine = s.threads().find(|t| t.owner == agent()).unwrap();
    assert_eq!(mine.gist, "second", "the later write wins within a key");
    assert!(s.contains_uid(&uid));
}

#[test]
fn relations_of_the_same_edge_merge_their_attestations() {
    let a = claim("a", vec![]);
    let b = claim("b", vec![]);
    let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
    let att = |n: u64| {
        Attestation::new(
            ua,
            agent(),
            Op::Authored,
            Rung::Document,
            Hlc::new(n, 0, agent()),
        )
    };

    let mut s = Store::from_records(vec![Record::Unit(a), Record::Unit(b)]);
    s.append(&[
        Record::Relation(Relation::new(RelKind::Rebuts, ua, ub).with_attestation(att(1))),
        Record::Relation(Relation::new(RelKind::Rebuts, ua, ub).with_attestation(att(2))),
    ])
    .unwrap();

    assert_eq!(s.relations().count(), 1, "one edge, not two");
    assert_eq!(s.relations().next().unwrap().attestations.len(), 2);
}

#[test]
fn rebuttals_are_reachable_by_uid() {
    let s = Store::from_records(corpus());
    let e_uid = canonical_uid(&evidence("Pool wait rose from 2 ms to 310 ms."));
    let c_uid = canonical_uid(&claim("The eu-west pool is saturated.", vec![e_uid]));
    let r_uid = canonical_uid(&claim(
        "The canary ran the same configuration cleanly.",
        vec![e_uid],
    ));
    assert_eq!(s.rebuttals_of(&c_uid), vec![r_uid]);
    assert!(s.rebuttals_of(&r_uid).is_empty());
}

#[test]
fn the_adjacency_reflects_the_grounds_graph() {
    let s = Store::from_records(corpus());
    let g = s.adjacency();
    let e_uid = canonical_uid(&evidence("Pool wait rose from 2 ms to 310 ms."));
    let c_uid = canonical_uid(&claim("The eu-west pool is saturated.", vec![e_uid]));

    let e = g.id(&e_uid).unwrap();
    let c = g.id(&c_uid).unwrap();
    assert_eq!(g.out(c, &EdgeSet::support()), vec![e]);
    assert!(smysl_graph::closure(g, &[c], &EdgeSet::support()).contains(&e));
}

#[test]
fn relations_are_selectable_by_kind() {
    let s = Store::from_records(corpus());
    assert_eq!(s.relations_of_kind(&RelKind::Rebuts).len(), 1);
    assert!(s.relations_of_kind(&RelKind::Causes).is_empty());
}

// ---------------------------------------------------------------------------
// bundle
// ---------------------------------------------------------------------------

/// A view references rather than owns, so `bundle` is the only way to make one portable.
#[test]
fn bundle_emits_the_reachable_closure() {
    let s = Store::from_records(corpus());
    let view = s.views().next().unwrap().clone();
    let bytes = s.bundle(&view);
    let (records, n) = smysl_core::from_cbor_seq(&bytes).unwrap();
    assert_eq!(n, bytes.len());

    let bundled = Store::from_records(records);
    let e_uid = canonical_uid(&evidence("Pool wait rose from 2 ms to 310 ms."));
    let c_uid = canonical_uid(&claim("The eu-west pool is saturated.", vec![e_uid]));
    assert!(bundled.contains_uid(&c_uid), "the root is missing");
    assert!(bundled.contains_uid(&e_uid), "its grounds are missing");

    let mut report = smysl_core::Report::new();
    bundled.report_dangling(&mut report);
    assert!(
        report.is_empty(),
        "a bundle must be self-contained: {report}"
    );
}

#[test]
fn bundle_of_an_empty_view_is_empty() {
    let s = Store::from_records(corpus());
    let view = View::new(ViewId::new("v/empty").unwrap(), "nothing");
    assert!(s.bundle(&view).is_empty());
}

#[test]
fn bundling_is_deterministic() {
    let s = Store::from_records(corpus());
    let view = s.views().next().unwrap().clone();
    assert_eq!(s.bundle(&view), s.bundle(&view));
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

/// Record arrival order must not change the derived graph - that is the property merge
/// depends on (rule U), and the store is where it first has to hold.
#[test]
fn arrival_order_does_not_change_the_derived_graph() {
    let forward = Store::from_records(corpus());
    let mut reversed_records = corpus();
    reversed_records.reverse();
    let reversed = Store::from_records(reversed_records);

    assert_eq!(forward.adjacency(), reversed.adjacency());
    assert_eq!(
        forward.units().map(|(u, _)| *u).collect::<BTreeSet<_>>(),
        reversed.units().map(|(u, _)| *u).collect::<BTreeSet<_>>()
    );

    // Index bytes differ only in record offsets, which follow the log; the graph itself
    // does not.
    assert_eq!(forward.index().fwd_adj, reversed.index().fwd_adj);
    assert_eq!(forward.index().cache, reversed.index().cache);
}

#[test]
fn the_index_is_byte_stable_across_repeated_derivation() {
    let s = Store::from_records(corpus());
    assert_eq!(s.index().to_bytes(), s.index().to_bytes());
}
