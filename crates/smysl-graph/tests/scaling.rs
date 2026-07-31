//! In-process cost of the graph operations, isolated from parsing and process startup.
//!
//! ```text
//! cargo test -p smysl-graph --release --test scaling -- --ignored --nocapture
//! ```
//!
//! `scripts/bench-scaling.py` measures the *commands*, which is what a user experiences and
//! the right thing to quote. It is the wrong thing for answering "what does `salience` cost",
//! because at these store sizes the process spends most of its time parsing and starting up
//! — at 500 units the whole command is 8 ms, and the operation under test is a fraction of
//! that. A ratio computed from mostly-noise is a ratio about noise.
//!
//! `salience` was the last pure operation whose per-call cost had never been characterised.
//! It was *assumed* linear. That assumption is exactly the shape of two earlier ones about
//! `pack` that turned out to be wrong when someone finally counted, so it is measured here
//! rather than asserted anywhere.
//!
//! `#[ignore]` because it is a measurement, not a gate. Timing assertions on shared CI
//! runners fail for reasons that have nothing to do with the code, and a test that cries
//! wolf gets muted — which costs more than the coverage it was supposed to buy.

use std::time::Instant;

use smysl_core::{
    canonical_uid, KernelType, Record, RelKind, Relation, SourceKind, SourceRef, Status, Uid,
    UnitCoreBuilder,
};
use smysl_graph::{salience, SalienceRequest, Store};

/// A store with real depth and fan-out.
///
/// Each claim grounds on its predecessor and on one seven back, so the graph is neither a
/// chain (no fan-out) nor a star (no depth). Both shapes would understate the cost of a walk.
fn store(n: usize) -> Store {
    let mut records: Vec<Record> = Vec::new();
    let mut uids: Vec<Uid> = Vec::new();

    let base = UnitCoreBuilder::new(
        KernelType::Evidence,
        "a baseline reading for the scaling measurement",
        Status::Measured,
    )
    .source(SourceRef::new(SourceKind::Metric, "m"))
    .build()
    .unwrap();
    uids.push(canonical_uid(&base));
    records.push(Record::Unit(base));

    for i in 0..n {
        let grounds: Vec<Uid> = if i < 7 {
            vec![uids[0]]
        } else {
            vec![uids[i], uids[i - 6]]
        };
        let core = UnitCoreBuilder::new(
            KernelType::Claim,
            format!("generated claim number {i} in the scaling store"),
            Status::Inferred,
        )
        .grounds(grounds)
        .build()
        .unwrap();
        uids.push(canonical_uid(&core));
        records.push(Record::Unit(core));
    }

    // A rebuttal every ten units, so the rebuttal term has something to weigh.
    for i in (0..n).step_by(10) {
        if i + 5 < uids.len() {
            records.push(Record::Relation(Relation::new(
                RelKind::Rebuts,
                uids[i + 5],
                uids[i],
            )));
        }
    }
    Store::from_records(records)
}

/// Median of five, so one scheduling hiccup does not become the number.
fn timed(f: impl Fn()) -> f64 {
    let mut runs: Vec<f64> = (0..5)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    runs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    runs[2]
}

#[test]
#[ignore = "a measurement, not a gate"]
fn salience_per_call_cost() {
    println!("\nsalience, in process, median of 5\n");
    println!("{:>7}  {:>10}  {:>8}", "units", "ms", "x per 2x");
    let mut prev: Option<(usize, f64)> = None;
    for n in [1_000usize, 2_000, 4_000, 8_000, 16_000] {
        let s = store(n);
        let ms = timed(|| {
            let _ = salience(&s, &SalienceRequest::default());
        });
        let ratio = match prev {
            Some((_, p)) if p > 0.0 => format!("{:.2}", ms / p),
            _ => "-".to_string(),
        };
        println!("{n:>7}  {ms:>10.2}  {ratio:>8}");
        prev = Some((n, ms));
    }
    println!("\n2.0 = linear, 4.0 = quadratic\n");
}
