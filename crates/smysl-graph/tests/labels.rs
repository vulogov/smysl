//! A label the store binds to more than one unit is refused, not resolved to whichever
//! binding came last.
//!
//! A store merged from three extraction runs of one commit bound `d/g90ec2f7-1` to three
//! different uids. `merge` reported the collision as a contention, correctly — and then
//! `smysl trace` and `smysl retract --dry-run` by that label both exited 0 on one of the three,
//! because every caller built its label map with a `BTreeMap` from the binding records and the
//! last insert won. For `retract` that is retracting a unit the caller did not choose.

use smysl_core::{KernelType, Label, LabelBinding, Record, Status, Uid, UnitCoreBuilder};
use smysl_graph::{label_bindings, resolve_label, LabelError, Store};

fn unit(gist: &str) -> (Record, Uid) {
    let core = UnitCoreBuilder::new(KernelType::Decision, gist, Status::Speculative)
        .build()
        .unwrap();
    let uid = smysl_core::canonical_uid(&core);
    (Record::Unit(core), uid)
}

fn bind(label: &Label, uid: Uid) -> Record {
    Record::LabelBinding(LabelBinding::new(label.clone(), uid))
}

#[test]
fn one_binding_resolves() {
    let l = Label::new("d/g90ec2f7-1").unwrap();
    let (u, uid) = unit("run one");
    let store = Store::from_records(vec![u, bind(&l, uid)]);
    assert_eq!(label_bindings(&store, &l), vec![uid]);
    assert_eq!(resolve_label(&store, &l), Ok(uid));
}

#[test]
fn a_label_bound_to_two_units_is_ambiguous_with_both_sorted() {
    let l = Label::new("d/g90ec2f7-1").unwrap();
    let (a, ua) = unit("run one worded it this way");
    let (b, ub) = unit("run two worded it another way");
    let mut expected = vec![ua, ub];
    expected.sort();
    // Both record orders: last-wins was the bug, so the answer must not depend on order.
    for records in [
        vec![a.clone(), b.clone(), bind(&l, ua), bind(&l, ub)],
        vec![a, b, bind(&l, ub), bind(&l, ua)],
    ] {
        let store = Store::from_records(records);
        assert_eq!(label_bindings(&store, &l), expected);
        assert_eq!(
            resolve_label(&store, &l),
            Err(LabelError::Ambiguous(expected.clone()))
        );
    }
}

#[test]
fn the_same_binding_twice_is_one_binding() {
    // A store merged from two copies of one document carries the binding twice. That is one
    // name for one unit, not an ambiguity.
    let l = Label::new("c/x").unwrap();
    let (u, uid) = unit("the same unit");
    let store = Store::from_records(vec![u, bind(&l, uid), bind(&l, uid)]);
    assert_eq!(resolve_label(&store, &l), Ok(uid));
}

#[test]
fn an_unbound_label_is_unbound() {
    let (u, _) = unit("anything");
    let store = Store::from_records(vec![u]);
    assert_eq!(
        resolve_label(&store, &Label::new("c/nope").unwrap()),
        Err(LabelError::Unbound)
    );
}
