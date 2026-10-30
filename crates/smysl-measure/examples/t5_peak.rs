//! T5 over a synthetic text of `<tokens>` words in 25-word segments, drawn from a seeded
//! Zipf-like vocabulary of 12,000 types — a Bible's shape — for its peak memory and time:
//!
//! ```text
//! cargo build --release -p smysl-measure --example t5_peak
//! /usr/bin/time -l target/release/examples/t5_peak 800000
//! ```

use std::collections::BTreeSet;

use smysl_graph::Store;
use smysl_measure::rng::Rng;
use smysl_measure::text::{Corpus, Stream};
use smysl_measure::{run, Level, Outcome, Recipe, Resolved};

fn main() {
    let tokens: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .expect("usage: t5_peak <tokens>");
    // Word r has weight 1/r: draw by inverting the cumulative weights.
    let types = 12_000;
    let mut cum = Vec::with_capacity(types);
    let mut total = 0.0;
    for r in 1..=types {
        total += 1.0 / r as f64;
        cum.push(total);
    }
    let mut rng = Rng::new(&[7; 32], 0);
    let words: Vec<String> = (0..tokens)
        .map(|_| {
            let x = (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64 * total;
            format!("w{}", cum.partition_point(|&c| c < x))
        })
        .collect();
    let segments = words.chunks(25).map(<[String]>::to_vec).collect();
    let c = Corpus::from_terms("smysl/an/1:surface", Stream::Surface, Level::Part, segments);
    let store = Store::new();
    let scope = Resolved::of_uids("synthetic", BTreeSet::new());
    let input = smysl_measure::Input::new(&store, &scope).with_text(&c);
    let r = run(&["T5"], &Recipe::new("", 0, "synthetic"), &input).remove(0);
    match &r.outcome {
        Outcome::Value(v) => println!(
            "tokens {} ngrams {} H {}",
            v.n["tokens"], v.n["ngrams"], v.values["H"]
        ),
        other => println!("{other:?}"),
    }
}
