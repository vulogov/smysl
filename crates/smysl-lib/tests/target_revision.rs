//! `--target-revision ID@N` (SMYSL-2.7 §3.11), TX-P12 step 13.
//!
//! The exit is three statements over GE-T16's synthetic revisions: a `split-value` lens
//! exports; a `retire` refuses with `SMY-E422`; and a two-level grounds chain gets new uids in
//! dependency order, with every rewritten core referencing only mapped uids. GE-T16's corpus
//! is TX-P12 step 9's and does not exist yet, so the revisions are synthetic in the sense the
//! step names: a schema written here, two and three revisions deep, with units placed at each.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::schema::{
    encode_lenses, encode_shape, Field, FieldType, LensOp, LensStep, UnitView,
};
use smysl_core::surface::HValue;
use smysl_core::types::SchemaDecl;
use smysl_core::{
    canonical_uid, AgentId, Attestation, Code, Commit, Commitment, Hlc, Label, LabelBinding, Op,
    Record, RelKind, Relation, Rung, SchemaId, Status, Uid, UnitCore, UnitCoreBuilder,
};
use smysl_graph::Store;
use smysl_lib::export::target_revision::rewrite;
use smysl_lib::{
    export_store, verify_collection, ExportOptions, LibError, Property, Selection, TargetRevision,
};

const ID: &str = "x.ge16/v1";

fn sid(s: &str) -> SchemaId {
    SchemaId::parse(s).unwrap()
}

fn s(v: &str) -> HValue {
    HValue::Str(v.into())
}

fn agent() -> AgentId {
    AgentId::new("human:ge16").unwrap()
}

/// Revision 1: a state of `open`, `shut` or `ajar`, a note and a key that revision 3 retires.
/// Revision 2 splits `ajar` into `half` with a `gap` of `some`; revision 3 retires `old`.
fn decls(revisions: u32) -> Vec<Record> {
    let mut f1 = BTreeMap::new();
    f1.insert(
        "ge:state".to_string(),
        Field::enumeration(["open", "shut", "ajar"]),
    );
    f1.insert("ge:note".to_string(), Field::new(FieldType::Text));
    f1.insert("ge:old".to_string(), Field::new(FieldType::Text));
    let mut d1 = SchemaDecl::new(sid(ID), 1);
    d1.types = vec![sid("x.ge16/claim")];
    d1.payload_shape = Some(encode_shape(&f1));

    let mut f2 = f1.clone();
    f2.insert(
        "ge:state".into(),
        Field::enumeration(["open", "shut", "half"]),
    );
    f2.insert("ge:gap".into(), Field::new(FieldType::Text));
    let mut d2 = SchemaDecl::new(sid(ID), 2);
    d2.types = vec![sid("x.ge16/claim")];
    d2.payload_shape = Some(encode_shape(&f2));
    d2.lenses = Some(encode_lenses(&[LensStep::new(
        1,
        2,
        vec![LensOp::SplitValue {
            key: "ge:state".into(),
            from: "ajar".into(),
            to: "half".into(),
            add: [("ge:gap".to_string(), s("some"))].into(),
        }],
    )]));

    let mut f3 = f2.clone();
    let mut old = Field::new(FieldType::Text);
    old.retired_in = Some(3);
    f3.insert("ge:old".into(), old);
    let mut d3 = SchemaDecl::new(sid(ID), 3);
    d3.types = vec![sid("x.ge16/claim")];
    d3.payload_shape = Some(encode_shape(&f3));
    d3.lenses = Some(encode_lenses(&[LensStep::new(
        2,
        3,
        vec![LensOp::Retire {
            key: "ge:old".into(),
        }],
    )]));

    [d1, d2, d3]
        .into_iter()
        .take(revisions as usize)
        .map(Record::SchemaDecl)
        .collect()
}

fn unit(gist: &str, payload: &[(&str, &str)], grounds: &[Uid]) -> UnitCore {
    let view = UnitView {
        ty: sid("x.ge16/claim"),
        payload: payload.iter().map(|(k, v)| (k.to_string(), s(v))).collect(),
    };
    let status = if grounds.is_empty() {
        Status::Speculative
    } else {
        Status::Inferred
    };
    let mut b =
        UnitCoreBuilder::new(view.ty.clone(), gist, status).grounds(grounds.iter().copied());
    if let Some(p) = view.payload_bytes() {
        b = b.payload(p);
    }
    b.build().unwrap()
}

fn authored(u: Uid, ms: u64) -> Record {
    Record::Attestation(Attestation::new(
        u,
        agent(),
        Op::Authored,
        Rung::Document,
        Hlc::new(ms, 0, agent()),
    ))
}

/// A two-revision store: `a` is half-open (revision 2's split), `b` grounds on `a`, `c` on `b`,
/// and `d` is shut and stands alone. A commitment to `a`, an edge from `a` to `d`, and a label
/// for `a` are what the rewrite must drop, re-attest and re-point.
struct Fixture {
    store: Store,
    a: Uid,
    b: Uid,
    c: Uid,
    d: Uid,
}

fn fixture(revisions: u32) -> Fixture {
    let ca = unit(
        "the door stands half open",
        &[("ge:state", "half"), ("ge:gap", "some")],
        &[],
    );
    let a = canonical_uid(&ca);
    let cb = unit(
        "so the draught is from the door",
        &[("ge:state", "open")],
        &[a],
    );
    let b = canonical_uid(&cb);
    let cc = unit(
        "so the room is cold",
        &[("ge:note", "inferred twice")],
        &[b],
    );
    let c = canonical_uid(&cc);
    let cd = unit("the window is shut", &[("ge:state", "shut")], &[]);
    let d = canonical_uid(&cd);
    let mut records = decls(revisions);
    for (core, ms) in [(ca, 10), (cb, 20), (cc, 30), (cd, 40)] {
        let u = canonical_uid(&core);
        records.push(Record::Unit(core));
        records.push(authored(u, ms));
    }
    records.push(Record::Relation(
        Relation::new(RelKind::Elaborates, a, d).with_attestation(Attestation::new(
            Relation::new(RelKind::Elaborates, a, d).uid(),
            agent(),
            Op::Authored,
            Rung::Document,
            Hlc::new(50, 0, agent()),
        )),
    ));
    records.push(Record::Commit(Commit::new(
        a,
        Commitment::Committed,
        agent(),
        Hlc::new(60, 0, agent()),
    )));
    records.push(Record::LabelBinding(LabelBinding::new(
        Label::new("door/half").unwrap(),
        a,
    )));
    Fixture {
        store: Store::from_records(records),
        a,
        b,
        c,
        d,
    }
}

fn at(rev: u32) -> TargetRevision {
    TargetRevision::parse(&format!("{ID}@{rev}")).unwrap()
}

#[test]
fn a_split_value_lens_exports() {
    let f = fixture(2);
    let sel = Selection::all(&f.store);
    let opts = ExportOptions::new().target_revision(at(1));
    let col = export_store(&f.store, &sel, &opts).unwrap();
    assert_eq!(
        col.index.derived.as_deref(),
        Some("target-revision x.ge16/v1@1")
    );
    assert!(col.index.lens_chain.is_some());
    assert_eq!(col.index.counts["SMY-W491"], 1, "the commitment to `a`");
    assert_eq!(col.report.count(Code::W491), 1);

    let records: Vec<Record> = col
        .files
        .iter()
        .flat_map(|x| smysl_core::from_cbor_seq(&x.bytes).unwrap().0)
        .collect();
    let units: Vec<&UnitCore> = records
        .iter()
        .filter_map(|r| match r {
            Record::Unit(c) => Some(c),
            _ => None,
        })
        .collect();
    // `a` reads as revision 1 wrote it: `ajar`, and no `gap`.
    let a2 = units
        .iter()
        .find(|c| c.gist.starts_with("the door"))
        .expect("a rewritten");
    let v = UnitView::of(a2);
    assert_eq!(v.payload.get("ge:state"), Some(&s("ajar")));
    assert!(!v.payload.contains_key("ge:gap"));
    assert_ne!(canonical_uid(a2), f.a);
    // `d` is unchanged and keeps its uid.
    assert!(units.iter().any(|c| canonical_uid(c) == f.d));
    // No original of a rewritten unit is exported.
    for old in [f.a, f.b, f.c] {
        assert!(!units.iter().any(|c| canonical_uid(c) == old));
    }
    // The edge from `a` is a new relation, attested as a transformation of the old one.
    let old_rid = Relation::new(RelKind::Elaborates, f.a, f.d).uid();
    let rel = records
        .iter()
        .find_map(|r| match r {
            Record::Relation(x) => Some(x),
            _ => None,
        })
        .expect("the edge is exported");
    assert_eq!(rel.from, canonical_uid(a2));
    // A file carries an edge's attestations as records of their own, by rid.
    let att: Vec<&Attestation> = records
        .iter()
        .filter_map(|r| match r {
            Record::Attestation(a) if a.uid == rel.uid() => Some(a),
            _ => None,
        })
        .chain(rel.attestations.iter())
        .collect();
    assert_eq!(att.len(), 1);
    assert_eq!(att[0].op, Op::Transformed);
    assert_eq!(att[0].parents, BTreeSet::from([old_rid]));
    // The label follows the unit.
    assert!(records
        .iter()
        .any(|r| matches!(r, Record::LabelBinding(b) if b.uid == canonical_uid(a2))));

    // Deterministic, and the files are what the index says; P1 and P3 are not claimed.
    assert_eq!(export_store(&f.store, &sel, &opts).unwrap(), col);
    let v = verify_collection(&col);
    assert!(v.holds());
    assert!(matches!(v.p1, Property::NotApplicable(_)));
    assert!(matches!(v.p3, Property::NotApplicable(_)));
}

#[test]
fn a_retire_on_the_backward_path_refuses_with_e422() {
    let f = fixture(3);
    let sel = Selection::all(&f.store);
    match export_store(&f.store, &sel, &ExportOptions::new().target_revision(at(2))) {
        Err(LibError::TargetRevision(code, why)) => {
            assert_eq!(code, Code::E422, "{why}");
            assert!(why.contains("ge:old"), "{why}");
        }
        other => panic!("expected E422, got {other:?}"),
    }
}

#[test]
fn a_two_level_grounds_chain_is_rewritten_in_dependency_order() {
    let f = fixture(2);
    let sel: BTreeSet<Uid> = [f.c].into();
    let rw = rewrite(&f.store, &sel, &at(1)).unwrap();
    let map = rw.map();
    // U is the selection closed along its grounds: `c` brings `b`, `b` brings `a`.
    let order: Vec<Uid> = rw.pairs.iter().map(|(old, _)| *old).collect();
    let pos = |u: &Uid| order.iter().position(|x| x == u).unwrap();
    assert!(pos(&f.a) < pos(&f.b) && pos(&f.b) < pos(&f.c), "{order:?}");
    // Every one has a new uid: `a` for its payload, `b` and `c` because what they ground on
    // moved.
    for old in [f.a, f.b, f.c] {
        assert_ne!(map[&old], old);
    }
    // Every rewritten core references only mapped uids.
    let image: BTreeSet<Uid> = map.values().copied().collect();
    for (_, new) in &rw.pairs {
        let core = &rw.store.get(new).expect("held").core;
        for r in core.deps.iter().chain(&core.grounds) {
            assert!(
                image.contains(r),
                "{} references {}",
                new.short(),
                r.short()
            );
        }
        // And each carries one transformation, of the unit it replaced.
        let t: Vec<&Attestation> = rw
            .store
            .attestations_of(new)
            .iter()
            .filter(|a| a.op == Op::Transformed)
            .collect();
        assert_eq!(t.len(), 1);
        let old = rw.pairs.iter().find(|(_, n)| n == new).unwrap().0;
        assert_eq!(t[0].parents, BTreeSet::from([old]));
        assert_eq!(t[0].agent.as_str(), "tool:smysl-lib");
    }
    let b2 = &rw.store.get(&map[&f.b]).unwrap().core;
    assert_eq!(b2.grounds, BTreeSet::from([map[&f.a]]));
    let c2 = &rw.store.get(&map[&f.c]).unwrap().core;
    assert_eq!(c2.grounds, BTreeSet::from([map[&f.b]]));
}

#[test]
fn the_clock_is_after_everything_rewritten_and_can_be_given() {
    let f = fixture(2);
    let sel: BTreeSet<Uid> = [f.c].into();
    let rw = rewrite(&f.store, &sel, &at(1)).unwrap();
    let ts = |rw: &smysl_lib::export::target_revision::Rewrite| {
        let (_, n) = rw.pairs[0];
        rw.store
            .attestations_of(&n)
            .iter()
            .find(|a| a.op == Op::Transformed)
            .unwrap()
            .ts
            .clone()
    };
    // `a`, `b` and `c` were attested at 10, 20 and 30 ms: the export clock is (30, 1).
    let t = ts(&rw);
    assert_eq!((t.wall_ms, t.counter), (30, 1));
    let given = at(1)
        .clock(1_000, 7)
        .agent(AgentId::new("human:archivist").unwrap());
    let rw = rewrite(&f.store, &sel, &given).unwrap();
    let t = ts(&rw);
    assert_eq!(
        (t.wall_ms, t.counter, t.agent.as_str()),
        (1_000, 7, "human:archivist")
    );
}

#[test]
fn a_unit_older_than_the_target_is_read_forward_through_a_retire() {
    // A unit revision 1 wrote — `ajar`, with an `old` note — exported at revision 3.
    let mut records = decls(3);
    let core = unit(
        "an old door",
        &[("ge:state", "ajar"), ("ge:old", "kept")],
        &[],
    );
    let u = canonical_uid(&core);
    records.push(Record::Unit(core));
    records.push(authored(u, 5));
    let store = Store::from_records(records);
    let rw = rewrite(&store, &[u].into(), &at(3)).unwrap();
    let new = rw.map()[&u];
    let v = UnitView::of(&rw.store.get(&new).unwrap().core);
    assert_eq!(v.payload.get("ge:state"), Some(&s("half")));
    assert_eq!(v.payload.get("ge:gap"), Some(&s("some")));
    assert!(!v.payload.contains_key("ge:old"));
}

#[test]
fn a_unit_already_at_the_target_keeps_its_uid_and_attestations() {
    let f = fixture(2);
    let rw = rewrite(&f.store, &[f.d].into(), &at(1)).unwrap();
    assert_eq!(rw.map()[&f.d], f.d);
    assert_eq!(
        rw.store.attestations_of(&f.d),
        f.store.attestations_of(&f.d)
    );
    assert_eq!(rw.dropped, 0);
}

#[test]
fn a_revision_the_store_does_not_declare_is_e470() {
    let f = fixture(2);
    let sel = Selection::all(&f.store);
    for spec in ["x.ge16/v1@9", "x.other/v1@1"] {
        let t = TargetRevision::parse(spec).unwrap();
        match export_store(&f.store, &sel, &ExportOptions::new().target_revision(t)) {
            Err(LibError::TargetRevision(code, _)) => assert_eq!(code, Code::E470, "{spec}"),
            other => panic!("{spec}: {other:?}"),
        }
    }
    assert!(TargetRevision::parse("x.ge16/v1").is_err());
    assert!(TargetRevision::parse("x.ge16/v1@one").is_err());
}
