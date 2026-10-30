//! A union of shards answers as the store of their records (SMYSL-2.8 §3.7, §5.1).
//!
//! The records are a generated store's, dealt into up to five shards by the input: some records
//! land in two shards, attestations land away from their subjects, and the shards are handed to
//! the union in an input-chosen order. The union must answer as `Store::from_records` of the
//! shards' records concatenated in shard-id order — and, for what does not depend on order, as
//! the generated store itself.
//!
//! The seeded counterpart is implementation 6 of `smysl-graph/tests/store_conformance.rs`; this
//! target changes the search, not the property.

#![no_main]
use libfuzzer_sys::fuzz_target;
use smysl_core::diag::Report;
use smysl_core::{to_cbor, Record};
use smysl_fuzz::{generate, Choices};
use smysl_graph::{Store, StoreRead, Union};

/// What a store answers, in a form two stores can be compared by.
fn answers<S: StoreRead + ?Sized>(s: &S) -> Vec<String> {
    let mut dangling = Report::new();
    s.report_dangling(&mut dangling);
    vec![
        format!("{:?}", s.units().map(|(u, x)| (*u, x.attestations.clone())).collect::<Vec<_>>()),
        format!("{:?}", s.adjacency()),
        format!("{:?}", s.relations().collect::<Vec<_>>()),
        format!("{:?}", s.threads().collect::<Vec<_>>()),
        format!("{:?}", s.views().collect::<Vec<_>>()),
        format!("{:?}", s.contentions()),
        format!("{:?}", s.withdrawals().collect::<Vec<_>>()),
        format!("{:?}", s.commits().collect::<Vec<_>>()),
        format!("{:?}", s.library()),
        format!("{:?}", s.records().map(|r| to_cbor(&r)).collect::<Vec<_>>()),
        format!("{:?}", s.record_set_digest()),
        format!("{:?}", s.state_hash()),
        format!("{:?}", s.units_in_observed_order()),
        format!("{:?}", dangling.diagnostics),
        format!("{:?}", s.adjacency().nodes().map(|n| s.adjacency().uid(n).map(|u| s.is_unfounded(u))).collect::<Vec<_>>()),
    ]
}

fuzz_target!(|data: &[u8]| {
    let mut c = Choices::new(data);
    let whole = generate(&mut c, 10);
    let log: Vec<Record> = whole.iter().cloned().collect();

    let k = 1 + c.below(5);
    let mut parts = vec![Vec::new(); k];
    for r in &log {
        let a = c.below(k);
        parts[a].push(r.clone());
        if k > 1 && c.chance(6) {
            parts[(a + 1 + c.below(k - 1)) % k].push(r.clone());
        }
    }
    let stores: Vec<Store> = parts.iter().cloned().map(Store::from_records).collect();
    let mut order: Vec<usize> = (0..k).collect();
    for i in (1..k).rev() {
        order.swap(i, c.below(i + 1));
    }
    let union = Union::new(order.iter().map(|&i| (format!("s{i}"), &stores[i])));

    let oracle = Store::from_records(parts.iter().flatten().cloned().collect());
    assert_eq!(answers(&union), answers(&oracle), "a union disagrees with its records' store");
    assert_eq!(
        union.record_set_digest(),
        whole.record_set_digest(),
        "a union of a partition is not the whole"
    );
    assert_eq!(
        format!("{:?}", union.units().map(|(u, x)| (*u, x.attestations.clone())).collect::<Vec<_>>()),
        format!("{:?}", whole.units().map(|(u, x)| (*u, x.attestations.clone())).collect::<Vec<_>>()),
        "a union's units are not the whole's"
    );
});
