//! `SMY-W111` (1.7): a log that holds records more than once says so.
//!
//! 1.4's R10 stopped `append` from creating repeats and `compact` removes the ones an older log
//! already has — but nothing told a reader they were there. A store written before that fix could
//! carry them indefinitely, and the only way to find out was to run `compact` and read the number
//! it printed. `open` keeps a log exactly as it is on disk, which is the right behaviour and the
//! reason this had to be reported rather than repaired on read.

use smysl_check::{check, CheckOptions};
use smysl_core::diag::Code;
use smysl_core::{
    canonical_uid, to_cbor_seq, KernelType, Label, LabelBinding, Record, Status, UnitCoreBuilder,
};
use smysl_graph::Store;

fn records() -> Vec<Record> {
    let u = UnitCoreBuilder::new(
        KernelType::Claim,
        "the eu-west connection pool saturated",
        Status::Speculative,
    )
    .build()
    .unwrap();
    let uid = canonical_uid(&u);
    vec![
        Record::Unit(u),
        Record::LabelBinding(LabelBinding::new(Label::new("c/pool").unwrap(), uid)),
    ]
}

#[test]
fn a_log_with_repeats_is_reported_once() {
    let rs = records();
    // A label binding twice: exactly what a pre-R10 self-merge produced.
    let mut bytes = to_cbor_seq(&rs);
    bytes.extend(to_cbor_seq(&rs[1..2]));
    bytes.extend(to_cbor_seq(&rs[1..2]));

    let dir = std::env::temp_dir().join(format!("smysl-w111-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("old.cbor");
    std::fs::write(&path, &bytes).unwrap();

    let old = Store::open(&path).unwrap();
    assert_eq!(
        old.duplicate_records(),
        2,
        "`open` keeps the log as written"
    );

    let report = check(&old, CheckOptions::default());
    assert_eq!(
        report.count(Code::W111),
        1,
        "one diagnostic, not one per repeat: the store is larger than it needs to be, not wrong"
    );
    assert!(
        report.iter().any(|d| d.code == Code::W111
            && d.message.contains("2 record(s)")
            && d.message.contains("compact")),
        "the message should say how many and what to do: {:?}",
        report.iter().map(|d| &d.message).collect::<Vec<_>>()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_store_this_build_wrote_has_nothing_to_report() {
    let store = Store::from_records(records());
    assert_eq!(store.duplicate_records(), 0);
    assert_eq!(check(&store, CheckOptions::default()).count(Code::W111), 0);
}
