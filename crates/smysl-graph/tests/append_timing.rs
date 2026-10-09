//! The append cost model, measured rather than asserted.
//!
//! `Store::append` documents a table — 1336 µs a record at one record per call, 5 µs at a
//! thousand — because the derived state is rebuilt once per *call*, so a caller's batching is
//! the whole cost model. TX-P1 step 4 added a condition to that rebuild: text — a manifest, a
//! part text, a part reading — names no unit as an endpoint, so a text-only append does none of
//! it. Everything else rebuilds, which is the direction that fails safe.
//!
//! These are `#[ignore]`d. A timing assertion in CI is a flake generator — the figures depend
//! on the machine, the profile and what else is running — so what is pinned in the suite is
//! the *behaviour* (`everything_but_text_rebuilds_the_adjacency`, in the crate), and
//! what is here is the harness that produced the numbers in `append`'s doc comment. Rerun it
//! with:
//!
//! ```text
//! cargo test -p smysl-graph --release --test append_timing -- --ignored --nocapture
//! ```

use std::time::Instant;

use smysl_core::ids::{LangTag, Rdid, Tid};
use smysl_core::types::{Carry, Manifest, PartEntry};
use smysl_core::{KernelType, Record, Status, UnitCoreBuilder};
use smysl_graph::Store;

fn claim(i: usize) -> Record {
    Record::Unit(
        UnitCoreBuilder::new(
            KernelType::Claim,
            format!("synthetic claim number {i} for the timing harness"),
            Status::Speculative,
        )
        .build()
        .expect("a unit"),
    )
}

fn manifest(i: usize) -> Record {
    let mut m = Manifest::new(
        format!("alias{i}"),
        LangTag::new("en").expect("a tag"),
        "txt/1",
        "public-domain",
        "smysl/parts/1 level=line min=65536 max=4194304",
    );
    m.carry = Carry::Text;
    m.parts.push(PartEntry::new(
        Tid::from_normalised_bytes(&i.to_le_bytes()),
        128,
        [0u8; 32],
        Rdid::from_bytes([0u8; 32]),
    ));
    Record::Manifest(m)
}

/// Microseconds per record for a given batch size, over `total` records.
fn per_record(total: usize, batch: usize, make: impl Fn(usize) -> Record) -> f64 {
    let mut store = Store::new();
    let start = Instant::now();
    let mut i = 0;
    while i < total {
        let n = batch.min(total - i);
        let records: Vec<Record> = (i..i + n).map(&make).collect();
        store.append(&records).expect("appended");
        i += n;
    }
    start.elapsed().as_secs_f64() * 1e6 / total as f64
}

/// The 1.8 table, for unit batches. Step 4 must not have changed it.
#[test]
#[ignore = "timing; run with --release --ignored"]
fn unit_batches_cost_what_they_did() {
    println!("units, 20000 records:");
    for batch in [1usize, 10, 50, 200, 1000] {
        println!(
            "  {batch:>5} per call: {:>8.1} µs/record",
            per_record(20_000, batch, claim)
        );
    }
}

/// A manifest-only append does not rebuild the adjacency, so its cost does not grow.
///
/// The figure to compare against is the first row above: before step 4 a manifest paid exactly
/// what a unit paid, because `absorb` rebuilt whatever arrived.
#[test]
#[ignore = "timing; run with --release --ignored"]
fn manifest_only_appends_do_not_pay_for_the_store() {
    println!("manifests, one per call:");
    for total in [1_000usize, 5_000, 20_000] {
        println!(
            "  {total:>6} records: {:>8.1} µs/record",
            per_record(total, 1, manifest)
        );
    }
    println!("units, one per call, for comparison:");
    for total in [1_000usize, 5_000] {
        println!(
            "  {total:>6} records: {:>8.1} µs/record",
            per_record(total, 1, claim)
        );
    }
}
