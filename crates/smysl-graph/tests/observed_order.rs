//! `units_in_observed_order` (1.8): what happened, in what order.
//!
//! `captured` is a date, so everything on one Thursday is simultaneous and an incident cannot be
//! reconstructed from it. `observed` carries the instant, and this is the reading of it.

use smysl_core::{
    canonical_uid, KernelType, Record, SourceKind, SourceRef, Status, Uid, UnitCoreBuilder,
};
use smysl_graph::Store;

fn reading(gist: &str, ms: Option<u64>) -> Record {
    let mut src = SourceRef::new(SourceKind::Metric, format!("m.{gist}"));
    if let Some(ms) = ms {
        src = src.observed_at(ms);
    }
    Record::Unit(
        UnitCoreBuilder::new(KernelType::Evidence, gist, Status::Measured)
            .source(src)
            .build()
            .unwrap(),
    )
}

fn uid_of(r: &Record) -> Uid {
    match r {
        Record::Unit(u) => canonical_uid(u),
        _ => unreachable!(),
    }
}

#[test]
fn readings_come_back_oldest_first() {
    let late = reading("billing p99 rose", Some(1_752_069_907_000));
    let early = reading("config push landed", Some(1_752_069_600_000));
    let middle = reading("auth pool wait rose", Some(1_752_069_751_000));

    // Inserted newest-first, so a pass-through order would be wrong.
    let store = Store::from_records(vec![late.clone(), early.clone(), middle.clone()]);
    assert_eq!(
        store.units_in_observed_order(),
        vec![uid_of(&early), uid_of(&middle), uid_of(&late)]
    );
    assert_eq!(store.observed_at(&uid_of(&middle)), Some(1_752_069_751_000));
}

/// A unit with no instant is not early and not late: it is not on the timeline.
#[test]
fn units_without_an_instant_are_left_out() {
    let timed = reading("auth pool wait rose", Some(1_752_069_751_000));
    let untimed = reading("somebody restarted the pod", None);
    let store = Store::from_records(vec![timed.clone(), untimed.clone()]);

    assert_eq!(store.units_in_observed_order(), vec![uid_of(&timed)]);
    assert_eq!(store.observed_at(&uid_of(&untimed)), None);
    assert_eq!(store.units().count(), 2, "both are still in the store");
}

/// Two readings at the same instant still have one order, so two runs agree (rule D).
#[test]
fn a_tie_is_broken_by_uid() {
    let a = reading("auth p99 rose", Some(1_752_069_751_000));
    let b = reading("billing p99 rose", Some(1_752_069_751_000));
    let forward = Store::from_records(vec![a.clone(), b.clone()]);
    let backward = Store::from_records(vec![b, a]);
    assert_eq!(
        forward.units_in_observed_order(),
        backward.units_in_observed_order()
    );
}

/// An empty answer where nothing carries an instant, rather than everything in uid order.
#[test]
fn a_corpus_with_no_instants_has_no_timeline() {
    let store = Store::from_records(vec![
        reading("one thing happened", None),
        reading("another thing happened", None),
    ]);
    assert!(store.units_in_observed_order().is_empty());
}
