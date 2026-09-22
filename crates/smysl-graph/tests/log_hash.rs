//! The log fingerprint is incremental (1.8), and identical to what it was.
//!
//! `log_hash` validates a cached index: `Index::matches(log_len, log_hash)`. It was recomputed
//! from scratch on every `append` — re-encoding every record in the store to hash them again —
//! so appending one record to a thirty-thousand-record store took 130 ms and the cost grew with
//! the store. The digest must not change, or every cached index in existence is invalidated.

use smysl_core::{hash_bytes, KernelType, Record, Status, UnitCoreBuilder};
use smysl_graph::Store;

fn claim(gist: &str) -> Record {
    Record::Unit(
        UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
            .build()
            .unwrap(),
    )
}

/// The incremental digest equals the one-shot digest over the same bytes, at every step.
#[test]
fn the_rolling_hash_is_the_one_shot_hash() {
    let mut store = Store::new();
    for i in 0..40 {
        store
            .append(&[claim(&format!("claim number {i}"))])
            .unwrap();
        assert_eq!(
            store.log_hash(),
            &hash_bytes(&store.log_bytes()),
            "after {} append(s) the fingerprint diverged from the log",
            i + 1
        );
        assert_eq!(store.log_len(), store.log_bytes().len() as u64);
    }
}

/// A store built in one go and one built by appending agree — the digest is a function of the
/// log, not of how it was assembled.
#[test]
fn assembling_it_differently_gives_the_same_fingerprint() {
    let records: Vec<Record> = (0..12).map(|i| claim(&format!("claim {i}"))).collect();

    let at_once = Store::from_records(records.clone());
    let mut one_at_a_time = Store::new();
    for r in &records {
        one_at_a_time.append(std::slice::from_ref(r)).unwrap();
    }
    let mut in_batches = Store::new();
    for chunk in records.chunks(5) {
        in_batches.append(chunk).unwrap();
    }

    assert_eq!(at_once.log_hash(), one_at_a_time.log_hash());
    assert_eq!(at_once.log_hash(), in_batches.log_hash());
    assert_eq!(at_once.log_len(), in_batches.log_len());
}

/// A duplicate appends nothing, so it must not disturb the fingerprint either (R10).
#[test]
fn a_duplicate_leaves_the_fingerprint_alone() {
    let mut store = Store::from_records(vec![claim("the pool saturated")]);
    let before = *store.log_hash();
    let report = store.append(&[claim("the pool saturated")]).unwrap();
    assert_eq!(report.added, 0);
    assert_eq!(*store.log_hash(), before);
    assert_eq!(store.log_hash(), &hash_bytes(&store.log_bytes()));
}

/// And a reopened store agrees with the one that wrote it.
#[test]
fn a_reopened_store_has_the_same_fingerprint() {
    let dir = std::env::temp_dir().join(format!("smysl-loghash-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("store.cbor");

    let mut written = Store::open(&path).unwrap();
    written
        .append(
            &(0..6)
                .map(|i| claim(&format!("claim {i}")))
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let reopened = Store::open(&path).unwrap();
    assert_eq!(written.log_hash(), reopened.log_hash());
    assert_eq!(written.log_len(), reopened.log_len());

    // And appending to the reopened one keeps agreeing.
    let mut more = reopened;
    more.append(&[claim("one more")]).unwrap();
    assert_eq!(more.log_hash(), &hash_bytes(&more.log_bytes()));
    let _ = std::fs::remove_dir_all(&dir);
}
