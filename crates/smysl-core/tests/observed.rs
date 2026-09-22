//! `SourceRef.observed` (1.8): when the observation was taken, to the millisecond.
//!
//! `captured` is a `Date` and closed by design — year, month, day. That is right for a document
//! and useless for telemetry: two readings a minute apart carry the same date, so nothing can
//! order them, and causal analysis over an estate of incidents is guesswork without "before".
//!
//! The field is supplied by the instrument and never read from a clock, which is the same
//! arrangement `Hlc::wall_ms` has had since 0.1 — the objection that closed `Date` was to
//! *reading* a clock inside a pure path, not to carrying a number somebody else read.

use smysl_core::{
    canonical_uid, from_cbor, to_cbor, KernelType, Record, SourceKind, SourceRef, Status,
    UnitCoreBuilder,
};

fn reading(ms: Option<u64>) -> smysl_core::UnitCore {
    let mut src = SourceRef::new(SourceKind::Metric, "auth.p99");
    if let Some(ms) = ms {
        src = src.observed_at(ms);
    }
    UnitCoreBuilder::new(
        KernelType::Evidence,
        "auth p99 reached 540 ms",
        Status::Measured,
    )
    .source(src)
    .build()
    .unwrap()
}

/// The point of the field: two readings of one metric, same sentence, different instants, are
/// two units. A day-precision `captured` cannot tell them apart at all.
#[test]
fn two_instants_are_two_units() {
    let a = reading(Some(1_752_069_751_000));
    let b = reading(Some(1_752_069_907_000));
    assert_ne!(
        canonical_uid(&a),
        canonical_uid(&b),
        "two readings a minute apart collapsed into one unit"
    );
    // And the same instant is the same unit, so a replayed ingest is idempotent.
    assert_eq!(
        canonical_uid(&a),
        canonical_uid(&reading(Some(1_752_069_751_000)))
    );
}

/// It round-trips, and a source without one encodes to the bytes it always did.
#[test]
fn it_round_trips_and_costs_nothing_when_absent() {
    let with = reading(Some(1_752_069_751_000));
    let bytes = to_cbor(&Record::Unit(with.clone()));
    let (back, _) = from_cbor(&bytes).unwrap();
    assert_eq!(back, Record::Unit(with));
    assert_eq!(to_cbor(&back), bytes, "re-encodes to what it was read from");

    let without = reading(None);
    let plain = to_cbor(&Record::Unit(without.clone()));
    assert_eq!(from_cbor(&plain).unwrap().0, Record::Unit(without.clone()));
    assert!(
        plain.len() < bytes.len(),
        "an absent instant must not add a key"
    );

    // The uid of a source with no instant is what it was before the field existed.
    let legacy = UnitCoreBuilder::new(
        KernelType::Evidence,
        "auth p99 reached 540 ms",
        Status::Measured,
    )
    .source(SourceRef::new(SourceKind::Metric, "auth.p99"))
    .build()
    .unwrap();
    assert_eq!(canonical_uid(&without), canonical_uid(&legacy));
}

/// Surface text carries it as milliseconds, the idiom `ts: [wall_ms, counter]` already uses.
#[test]
fn surface_carries_the_instant() {
    use smysl_core::surface::{parse_surface, write_surface, WriteContext};

    let src = "\
@evidence e/p99 { status: measured, source: { kind: metric, ref: \"auth.p99\", observed: 1752069751000, captured: 2026-07-09 } }
~ auth p99 reached 540 ms.
";
    let out = parse_surface(src).unwrap();
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let Record::Unit(u) = &out.records[0] else {
        panic!("a unit came first")
    };
    assert_eq!(u.source.as_ref().unwrap().observed, Some(1_752_069_751_000));

    // With the labels, so the round trip keeps `e/p99` rather than writing the uid and losing
    // the binding that named it.
    let written = write_surface(None, &out.records, &WriteContext::from_labels(&out.labels));
    assert!(written.contains("observed: 1752069751000"), "{written}");
    let again = parse_surface(&written).unwrap();
    assert_eq!(
        again.records, out.records,
        "parse → write → parse is a fixed point"
    );
}

/// Ordering, which is the whole reason the field exists.
#[test]
fn readings_sort_by_when_they_were_taken() {
    let mut units = [
        reading(Some(1_752_069_907_000)),
        reading(Some(1_752_069_751_000)),
        reading(Some(1_752_069_830_000)),
    ];
    units.sort_by_key(|u| u.source.as_ref().and_then(|s| s.observed));
    let order: Vec<u64> = units
        .iter()
        .map(|u| u.source.as_ref().unwrap().observed.unwrap())
        .collect();
    assert_eq!(
        order,
        [1_752_069_751_000, 1_752_069_830_000, 1_752_069_907_000]
    );
}
