//! In-process cost of `derive_thread`, and how it scales with the store's relations.
//!
//! ```text
//! cargo test -p smysl-thread --release --test scaling -- --ignored --nocapture
//! ```
//!
//! H-18's reason for existing. `Matcher::SourceOf` and `TargetOf` answered by filtering every
//! relation in the store, allocating a `Vec`, once per unit in scope per rule — O(U · R) work
//! for a question with one answer per kind. That shape does not show up at corpus size, where
//! R is a dozen; it shows up on a store built by an extraction run, where it is the derivation
//! rather than the parse that dominates.
//!
//! The second measurement is the one that matters: cost against R with U held fixed. Under the
//! old code it rose linearly with R for each unit; it is now close to flat, because the index
//! is built once.
//!
//! Measured on one machine, release build, median of five, before and after H-18. The "before"
//! column is the shape the change was made to remove, not a figure quoted from a document:
//!
//! ```text
//! units (2 rel/unit)      before      after        x per 2x before / after
//!   1000                  59.19       1.96         -      / -
//!   2000                 235.22       4.47         3.97   / 2.28
//!   4000                 940.83       9.87         4.00   / 2.21
//!   8000                3768.72      21.79         4.01   / 2.21
//!
//! relations (2000 units)  before      after
//!   2000                 120.17       4.07
//!  32000                2030.53       3.90
//! ```
//!
//! So it really was quadratic in units, and really was linear in relations per unit, and is now
//! linear and flat respectively — 173x at 8000 units. `derive_cost_per_schema` carries its own
//! control: `narrative` names no relation kind in its rules, and it was the one schema the old
//! code did not punish (6.06 ms against 2294 ms for `analysis` on the same store). That is what
//! makes the cause the relation lookups rather than something else that happened to be slow.
//!
//! SMYSL-2.8's 171k-unit probe is a different generator and is not reproduced here; these
//! numbers are this file's own.
//!
//! `#[ignore]` because it is a measurement, not a gate, for the reason the other three
//! `scaling.rs` files give: timing assertions on shared CI runners fail for reasons that have
//! nothing to do with the code, and a test that cries wolf gets muted.

use std::time::Instant;

use smysl_core::{
    canonical_uid, AgentId, KernelType, Record, RelKind, Relation, SourceKind, SourceRef, Status,
    ThreadId, ThreadSchema, Uid, UnitCoreBuilder,
};
use smysl_graph::Store;
use smysl_thread::{derive_thread, DeriveOptions};

/// A store with real depth and fan-out, and `rels_per_unit` relations on top.
///
/// The unit half is deliberately the generator `smysl-check`, `smysl-pack` and `smysl-graph`
/// use, so the four measurements are comparable. The relation half is this file's own, because
/// relations are the axis under test.
fn store(n: usize, rels_per_unit: usize) -> Store {
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

    // Kinds the schemas' rules actually name, so the index is exercised rather than skipped.
    const KINDS: &[RelKind] = &[
        RelKind::Causes,
        RelKind::Rebuts,
        RelKind::Elaborates,
        RelKind::Answers,
    ];
    for i in 0..n {
        for j in 0..rels_per_unit {
            let to = (i + 1 + j * 3) % uids.len();
            if to != i {
                records.push(Record::Relation(Relation::new(
                    KINDS[(i + j) % KINDS.len()].clone(),
                    uids[i],
                    uids[to],
                )));
            }
        }
    }
    Store::from_records(records)
}

fn opts() -> DeriveOptions {
    DeriveOptions::new(
        ThreadId::new("t/scaling").unwrap(),
        AgentId::new("tool:scaling").unwrap(),
    )
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
fn derive_cost_against_units() {
    println!("\nderive_thread(analysis), 2 relations per unit, median of 5\n");
    println!("{:>7}  {:>10}  {:>8}", "units", "ms", "x per 2x");
    let mut prev: Option<f64> = None;
    for n in [1_000usize, 2_000, 4_000, 8_000] {
        let s = store(n, 2);
        let ms = timed(|| {
            let _ = derive_thread(&s, ThreadSchema::Analysis, &opts());
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

/// The axis H-18 changed: relations, with units held fixed.
///
/// Before the change each extra relation cost every unit in scope another comparison, so this
/// column rose with R. The index is built once per derivation, so it should be close to flat —
/// what remains is building it, which is linear in R and paid a single time.
#[test]
#[ignore = "a measurement, not a gate"]
fn derive_cost_against_relations() {
    println!("\nderive_thread(analysis), 2000 units, median of 5\n");
    println!(
        "{:>5}  {:>9}  {:>10}  {:>8}",
        "r/u", "relations", "ms", "x per 2x"
    );
    let mut prev: Option<f64> = None;
    for rpu in [1usize, 2, 4, 8, 16] {
        let s = store(2_000, rpu);
        let rels = s.relations().count();
        let ms = timed(|| {
            let _ = derive_thread(&s, ThreadSchema::Analysis, &opts());
        });
        let ratio = match prev {
            Some(p) if p > 0.0 => format!("{:.2}", ms / p),
            _ => "-".to_string(),
        };
        println!("{rpu:>5}  {rels:>9}  {ms:>10.2}  {ratio:>8}");
        prev = Some(ms);
    }
    println!("\n1.0 = flat in R, 2.0 = linear in R per unit\n");
}

/// Every schema, at one size, so a rule set with more relation rules than `analysis` is not
/// left unmeasured.
#[test]
#[ignore = "a measurement, not a gate"]
fn derive_cost_per_schema() {
    println!("\nderive_thread, 4000 units, 4 relations per unit, median of 5\n");
    let s = store(4_000, 4);
    println!("{:>12}  {:>10}", "schema", "ms");
    for &schema in ThreadSchema::ALL {
        let ms = timed(|| {
            let _ = derive_thread(&s, schema, &opts());
        });
        println!("{:>12}  {ms:>10.2}", schema.to_string());
    }
    println!();
}
