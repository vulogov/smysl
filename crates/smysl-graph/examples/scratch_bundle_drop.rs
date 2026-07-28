//! Scratch generator for the manual (Ch16 / Ch14): a view with two roots, one of which is
//! retracted and referenced by nothing else, to show `bundle` actually drop a retracted
//! unit versus `bundle --include-retracted` keep it. Not part of the crate; run with
//! `cargo run -p smysl-graph --example scratch_bundle_drop`.

use smysl_core::{
    canonical_uid, to_cbor_seq, KernelType, Record, RelKind, Relation, Status, UnitCoreBuilder,
    View, ViewId,
};

fn main() {
    let kept = UnitCoreBuilder::new(
        KernelType::Claim,
        "the finding that stands",
        Status::Speculative,
    )
    .build()
    .unwrap();
    let uk = canonical_uid(&kept);

    // A second root, unreferenced by anything else in the store - the case bundle_with's
    // "still needed" check is built for.
    let stray = UnitCoreBuilder::new(
        KernelType::Claim,
        "a side note nobody built on",
        Status::Speculative,
    )
    .build()
    .unwrap();
    let us = canonical_uid(&stray);

    let view = View::new(ViewId::new("v/demo").unwrap(), "bundle-drop-demo").with_roots([uk, us]);

    let mut records = vec![Record::Unit(kept), Record::Unit(stray), Record::View(view)];
    records.push(Record::Relation(Relation::new(RelKind::Retracts, us, us)));

    std::fs::write("bundle-drop-demo.cbor", to_cbor_seq(&records)).unwrap();
    eprintln!("kept uid:  {uk}");
    eprintln!("stray uid: {us}");
}
