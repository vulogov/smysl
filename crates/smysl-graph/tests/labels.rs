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

/// R17 (1.5): uid → label, which anything that prints a store needs. A consumer kept its own map
/// because the store offered only label → uid, and a store read back from disk had none.
#[test]
fn labels_of_and_label_index_answer_the_other_direction() {
    use smysl_graph::{label_index, labels_of, resolve_label};

    let src = "\
@claim c/pool { status: speculative }
~ The pool saturated.

@claim c/canary { status: speculative }
~ The canary stayed clean.
";
    let out = smysl_core::surface::parse_surface(src).unwrap();
    let store = Store::from_records(out.records.clone());
    let pool = out.labels[&Label::new("c/pool").unwrap()];

    assert_eq!(
        labels_of(&store, &pool),
        vec![Label::new("c/pool").unwrap()]
    );
    assert!(labels_of(&store, &Uid::from_bytes([7; 32])).is_empty());

    // Every label resolves to a unit that names it back.
    for label in out.labels.keys() {
        let uid = resolve_label(&store, label).unwrap();
        assert!(labels_of(&store, &uid).contains(label), "{label}");
    }

    // The index agrees with the per-unit answer, and survives a round trip through bytes.
    let index = label_index(&store);
    for (uid, labels) in &index {
        assert_eq!(&labels_of(&store, uid), labels);
    }
    let reopened = Store::from_records(smysl_core::from_cbor_seq(&store.log_bytes()).unwrap().0);
    assert_eq!(label_index(&reopened), index, "a store read back disagrees");

    // Two names for one unit: identity is content, so both bind the same uid.
    let twice = "\
@claim c/one { status: speculative }
~ One claim.

@claim c/two { status: speculative }
~ One claim.
";
    let out = smysl_core::surface::parse_surface(twice).unwrap();
    let store = Store::from_records(out.records);
    let (uid, labels) = label_index(&store).into_iter().next().expect("one unit");
    assert_eq!(
        labels.len(),
        1,
        "surface keeps one name per unit: {labels:?}"
    );
    assert_eq!(labels_of(&store, &uid), labels);
}
