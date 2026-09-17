//! R10 — merge is idempotent at the record level, for every record type (rule U).
//!
//! `merge(A, A)` appended every label binding and schema declaration again, 42 records a time on a
//! real staged batch, because `Store::contains` knew nine record types and answered "absent" for
//! the rest. Each test here holds one consequence of the fix.

use std::collections::BTreeSet;

use smysl_core::{
    canonical_uid, hash_bytes, to_cbor, AgentId, Attestation, Contention, ContentionId, Detected,
    DetectionKind, Hlc, KernelType, Label, LabelBinding, Op, PackInfo, Record, RelKind, Relation,
    Resolution, ResolutionTarget, Role, Rung, SchemaDecl, SchemaId, Status, Step, Thread, ThreadId,
    ThreadSchema, UnitCore, UnitCoreBuilder, View, ViewId, Withdrawal,
};
use smysl_graph::{merge, MergeOptions, Store};

fn agent() -> AgentId {
    AgentId::new("tool:rust-smysl").unwrap()
}

fn hlc(ms: u64) -> Hlc {
    Hlc::new(ms, 0, agent())
}

fn claim(gist: &str) -> UnitCore {
    UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
        .build()
        .unwrap()
}

fn decl(version: u32, relations: &[&str]) -> SchemaDecl {
    let mut d = SchemaDecl::new(SchemaId::parse("x.verify/v1").unwrap(), version);
    d.relations = relations
        .iter()
        .map(|r| RelKind::parse(r).unwrap())
        .collect();
    d
}

/// One record of every type a store can hold, including an attestation on an edge and a record
/// type this build does not know.
fn every_record_type() -> Vec<Record> {
    let c = claim("the pool saturated");
    let r = claim("the pool never exceeded half its size");
    let (uc, ur) = (canonical_uid(&c), canonical_uid(&r));
    let edge = Relation::new(RelKind::Rebuts, ur, uc);
    let rid = edge.uid();
    vec![
        Record::Unit(c),
        Record::Unit(r),
        Record::Attestation(Attestation::new(
            uc,
            agent(),
            Op::Imported,
            Rung::Document,
            hlc(1),
        )),
        Record::Relation(edge),
        Record::Attestation(Attestation::new(
            rid,
            agent(),
            Op::Imported,
            Rung::Model,
            hlc(2),
        )),
        Record::Thread(
            Thread::new(
                ThreadId::new("t/brief").unwrap(),
                ThreadSchema::Brief,
                agent(),
                "the incident",
                hlc(3),
            )
            .with_steps([Step::new(Role::BottomLine, uc)]),
        ),
        Record::View(View::new(ViewId::new("v/incident").unwrap(), "brief").with_roots([uc])),
        Record::Contention(Contention::new(
            ContentionId::derive(DetectionKind::LiveRebuttal, &uc, &[uc, ur]),
            uc,
            vec![uc, ur],
            Detected::new(DetectionKind::LiveRebuttal, hlc(4)),
        )),
        Record::PackInfo(PackInfo::new(1000, 120, "smysl/utf8-div4")),
        Record::SchemaDecl(decl(1, &["x.verify/supports"])),
        Record::LabelBinding(LabelBinding::new(Label::new("c/pool").unwrap(), uc)),
        Record::LabelBinding(LabelBinding::new(Label::new("c/half").unwrap(), ur)),
        Record::Withdrawal(Withdrawal::new(rid, agent(), hlc(5))),
        Record::Resolution(Resolution::new(
            ResolutionTarget::Relation(rid),
            agent(),
            hlc(6),
        )),
        Record::Unknown {
            code: 99,
            payload: vec![0xA1, 0x00, 0x01],
        },
    ]
}

fn record_set(store: &Store) -> BTreeSet<[u8; 32]> {
    store.iter().map(|r| hash_bytes(&to_cbor(r))).collect()
}

/// 1. Merging a store into itself adds nothing, for every record type.
#[test]
fn a_self_merge_adds_nothing_for_any_record_type() {
    let records = every_record_type();
    let mut store = Store::from_records(records.clone());
    let copy = store.clone();
    let before = store.len();
    assert_eq!(before, records.len());

    for round in 0..3 {
        let report = merge(&mut store, &copy, MergeOptions::default()).unwrap();
        assert_eq!(report.added, 0, "round {round}: re-appended something");
        assert_eq!(report.duplicates, records.len(), "round {round}");
        assert_eq!(store.len(), before, "round {round}: the log grew");
    }

    // And each type on its own, so a failure names the type.
    for r in &records {
        let mut s = Store::from_records(vec![r.clone()]);
        let again = Store::from_records(vec![r.clone()]);
        let report = merge(&mut s, &again, MergeOptions::default()).unwrap();
        assert_eq!(report.added, 0, "{} was re-appended", r.type_name());
    }
}

/// 2. A label bound to a different uid is a different record: appended, and a collision.
#[test]
fn a_label_bound_to_a_different_uid_is_appended_and_collides() {
    let a = claim("first");
    let b = claim("second");
    let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
    let label = Label::new("d/decision").unwrap();

    let mut left = Store::from_records(vec![
        Record::Unit(a),
        Record::LabelBinding(LabelBinding::new(label.clone(), ua)),
    ]);
    let right = Store::from_records(vec![
        Record::Unit(b),
        Record::LabelBinding(LabelBinding::new(label.clone(), ub)),
    ]);
    let report = merge(&mut left, &right, MergeOptions::default()).unwrap();
    assert_eq!(report.added, 2, "the unit and its binding");
    assert_eq!(smysl_graph::label_bindings(&left, &label).len(), 2);
    assert!(
        report
            .contentions
            .iter()
            .any(|c| c.detected.kind == DetectionKind::LabelCollision),
        "{:?}",
        report.contentions
    );
}

/// 3. The same schema id with a different version or relation list is a different declaration.
#[test]
fn a_schema_declaration_that_differs_is_appended() {
    let mut store = Store::from_records(vec![Record::SchemaDecl(decl(1, &["x.verify/supports"]))]);
    for other in [
        decl(2, &["x.verify/supports"]),
        decl(1, &["x.verify/supports", "x.verify/contradicts"]),
    ] {
        let incoming = Store::from_records(vec![Record::SchemaDecl(other)]);
        let report = merge(&mut store, &incoming, MergeOptions::default()).unwrap();
        assert_eq!(report.added, 1);
    }
    assert_eq!(store.len(), 3);
}

/// 4. On disk: a reopened store merged with its own records appends nothing to the file.
#[test]
fn a_reopened_store_merged_with_itself_leaves_the_file_unchanged() {
    let dir = std::env::temp_dir().join(format!("smysl-r10-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("store.cbor");

    let mut written = Store::open(&path).unwrap();
    written.append(&every_record_type()).unwrap();
    let size = std::fs::metadata(&path).unwrap().len();
    assert!(size > 0);

    let mut reopened = Store::open(&path).unwrap();
    let own = Store::from_records(every_record_type());
    let report = merge(&mut reopened, &own, MergeOptions::default()).unwrap();
    assert_eq!(report.added, 0);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), size);

    let again = reopened.clone();
    merge(&mut reopened, &again, MergeOptions::default()).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), size);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 5. Associative and commutative at the record level, with no duplicates, for stores that share
///    label bindings and schema declarations.
#[test]
fn merge_is_associative_and_commutative_over_records() {
    let shared = |extra: &str| {
        let u = claim(extra);
        let uid = canonical_uid(&u);
        vec![
            Record::Unit(u),
            Record::LabelBinding(LabelBinding::new(Label::new("c/shared").unwrap(), uid)),
            Record::LabelBinding(LabelBinding::new(Label::new("c/common").unwrap(), uid)),
            Record::SchemaDecl(decl(1, &["x.verify/supports"])),
        ]
    };
    let mut a_records = every_record_type();
    a_records.extend(shared("a"));
    let mut b_records = shared("b");
    b_records.push(Record::SchemaDecl(decl(2, &["x.verify/supports"])));
    let mut c_records = shared("a");
    c_records.extend(every_record_type().into_iter().take(5));

    let (a, b, c) = (
        Store::from_records(a_records),
        Store::from_records(b_records),
        Store::from_records(c_records),
    );
    let opts = MergeOptions::default;

    let mut ab = a.clone();
    merge(&mut ab, &b, opts()).unwrap();
    let mut ab_c = ab;
    merge(&mut ab_c, &c, opts()).unwrap();

    let mut cb = c.clone();
    merge(&mut cb, &b, opts()).unwrap();
    let mut a_cb = a.clone();
    merge(&mut a_cb, &cb, opts()).unwrap();

    assert_eq!(record_set(&ab_c), record_set(&a_cb));
    for s in [&ab_c, &a_cb] {
        assert_eq!(s.len(), record_set(s).len(), "a record is held twice");
    }
    assert!(ab_c.converged_with(&a_cb));
}

/// 6. Reindexing after repeated self-merges reproduces the same store.
#[test]
fn reindex_after_self_merges_reproduces_the_store() {
    let original = Store::from_records(every_record_type());
    let mut merged = original.clone();
    for _ in 0..3 {
        let copy = merged.clone();
        merge(&mut merged, &copy, MergeOptions::default()).unwrap();
    }
    let maintained = merged.index();
    let rebuilt = merged.reindex();
    assert_eq!(rebuilt.to_bytes(), maintained.to_bytes());
    assert_eq!(merged.len(), original.len());
    assert_eq!(record_set(&merged), record_set(&original));
    assert!(merged.converged_with(&original));
}

/// A log written before R10 can hold the same record many times. `open` keeps it as it is on disk,
/// and `compact` is where the repeats go: counted, removed, and nothing else changed.
#[test]
fn compact_removes_the_repeats_an_old_log_holds() {
    let dir = std::env::temp_dir().join(format!("smysl-r10-compact-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("old.cbor");

    let records = every_record_type();
    let mut bytes = smysl_core::to_cbor_seq(&records);
    bytes.extend(smysl_core::to_cbor_seq(&records[10..12])); // a schema declaration and a binding
    bytes.extend(smysl_core::to_cbor_seq(&records[10..12]));
    std::fs::write(&path, &bytes).unwrap();

    let old = Store::open(&path).unwrap();
    assert_eq!(
        old.len(),
        records.len() + 4,
        "open keeps the log as written"
    );

    let out = smysl_graph::compact::compact(&old);
    assert_eq!(out.duplicates, 4);
    assert!(!out.is_empty());
    assert_eq!(out.records.len(), records.len());
    let clean = Store::from_records(out.records);
    assert_eq!(record_set(&clean), record_set(&old));
    assert!(clean.converged_with(&old));
    let _ = std::fs::remove_dir_all(&dir);
}
