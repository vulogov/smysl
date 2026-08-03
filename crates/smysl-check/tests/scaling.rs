//! In-process cost of `check`, isolated from parsing and process startup.
//!
//! ```text
//! cargo test -p smysl-check --release --test scaling -- --ignored --nocapture
//! ```
//!
//! `check` and `merge` were the last two operations in the pure set whose per-call cost had
//! only ever been measured *through the command*, where parsing dominates: at 500 units the
//! whole command is 8 ms and the operation under test is a fraction of it, so the ratio
//! computed from it is a ratio about parsing. Both were assumed linear.
//!
//! That assumption has been wrong before. `pack` was assumed linear for four releases and was
//! quadratic; the local-improvement pass was assumed harmless for eight and was making packs
//! worse. Assumed-linear is exactly the claim this project has learned to count rather than
//! repeat.
//!
//! `#[ignore]` because it is a measurement, not a gate. Timing assertions on shared CI runners
//! fail for reasons that have nothing to do with the code, and a test that cries wolf gets
//! muted — which costs more than the coverage it was supposed to buy.

use std::time::Instant;

use smysl_check::{check, CheckOptions};
use smysl_core::{
    canonical_uid, KernelType, Record, RelKind, Relation, SourceKind, SourceRef, Status, Uid,
    UnitCoreBuilder,
};
use smysl_graph::Store;

/// A store with real depth and fan-out.
///
/// Each claim grounds on its predecessor and on one seven back, so the graph is neither a
/// chain (no fan-out) nor a star (no depth). Both shapes would understate the cost of a walk.
/// Deliberately the same generator as `smysl-pack` and `smysl-graph` use, so the three
/// measurements are comparable.
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

    // A rebuttal every ten units, so rule R has something to travel along.
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
fn check_per_call_cost() {
    println!("\ncheck, every pass, in process, median of 5\n");
    println!("{:>7}  {:>10}  {:>8}", "units", "ms", "x per 2x");
    let mut prev: Option<f64> = None;
    for n in [1_000usize, 2_000, 4_000, 8_000, 16_000] {
        let s = store(n);
        let ms = timed(|| {
            let _ = check(&s, CheckOptions::default());
        });
        let ratio = match prev {
            Some(p) if p > 0.0 => format!("{:.2}", ms / p),
            _ => "-".to_string(),
        };
        println!("{n:>7}  {ms:>10.2}  {ratio:>8}");
        prev = Some(ms);
    }
    println!("\n2.0 = linear, 4.0 = quadratic\n");
}

/// Per-pass, because the total hides a single quadratic pass behind a dozen linear ones.
/// If the aggregate above ever turns super-linear, this says which pass to look at.
#[test]
#[ignore = "a measurement, not a gate"]
fn check_per_pass_cost() {
    use smysl_check::Pass;

    // Every pass the build implements, so a new one cannot be added without appearing here.
    let passes = Pass::ALL;
    println!("\ncheck, one pass at a time, ms — and the 2x ratio at each step\n");
    print!("{:>14}", "pass");
    for n in [2_000usize, 4_000, 8_000] {
        print!("  {n:>8}");
    }
    println!("  {:>8}", "x per 2x");

    for &p in passes {
        print!("{:>14}", format!("{p:?}"));
        let mut times = Vec::new();
        for n in [2_000usize, 4_000, 8_000] {
            let s = store(n);
            // `CheckOptions` is `#[non_exhaustive]`, so it is built and then adjusted rather
            // than written as a literal — which is the point of the marker.
            let mut opts = CheckOptions::default();
            opts.only = vec![p];
            let ms = timed(|| {
                let _ = check(&s, opts.clone());
            });
            times.push(ms);
            print!("  {ms:>8.2}");
        }
        let ratio = if times[1] > 0.0 {
            times[2] / times[1]
        } else {
            0.0
        };
        println!("  {ratio:>8.2}");
    }
    println!("\n2.0 = linear, 4.0 = quadratic\n");
}
