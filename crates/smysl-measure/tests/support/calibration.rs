//! T3's calibration source and row (SMYSL-2.6 §3.7), shared by `tests/lz_match.rs` and
//! `examples/t3_calibration.rs` so that the fixture and its check cannot drift apart.

use std::collections::BTreeSet;

use smysl_graph::Store;
use smysl_measure::rng::Rng;
use smysl_measure::text::{Corpus, Doc};
use smysl_measure::{run, to_payload, MeasureResult, Outcome, Recipe, Resolved};

/// The source entropies the table covers, in bits.
pub const BITS: [u32; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

/// The lengths it covers: §3.7's 10³ to 10⁷, and §2.9's 2k, 8k and 32k.
pub const LENGTHS: [usize; 8] = [
    1_000, 2_000, 8_000, 10_000, 32_000, 100_000, 1_000_000, 10_000_000,
];

/// `n` symbols uniform over `2^bits`, as characters from `a` for up to 16 symbols and from
/// U+0100 past it, under the crate's generator keyed by the row.
pub fn iid(bits: u32, n: usize, seed: u64) -> String {
    let k = 1usize << bits;
    let first = if k <= 16 { 'a' as u32 } else { 0x100 };
    let mut rng = Rng::new(&[4; 32], (u64::from(bits) << 32) | seed);
    (0..n)
        .map(|_| char::from_u32(first + rng.index(k) as u32).unwrap())
        .collect()
}

/// T3 over one text, under the default recipe with `min:n` at `min_n`.
pub fn t3(text: String, min_n: u64) -> MeasureResult {
    let c = Corpus::from_docs(vec![Doc::new("iid", vec![text])]);
    let mut recipe = Recipe::new("", 0, "synthetic");
    recipe.min_n = Some(min_n);
    let store = Store::new();
    let scope = Resolved::of_uids("synthetic", BTreeSet::new());
    let input = smysl_measure::Input::new(&store, &scope).with_text(&c);
    run(&["T3"], &recipe, &input).remove(0)
}

/// One row of the table: `bits n seed q20(H) q20(H:mr) H H:mr payload`, the payload as the
/// BLAKE3 of its bytes, so the row pins the whole result and not only its two numbers.
pub fn row(bits: u32, n: usize, seed: u64) -> String {
    let r = t3(iid(bits, n, seed), 2);
    let Outcome::Value(v) = &r.outcome else {
        panic!("T3 did not run on H = {bits}, n = {n}: {:?}", r.outcome);
    };
    let fixed = |k: &str| match v.values.get(k) {
        Some(smysl_measure::Num::Fixed(i)) => *i,
        other => panic!("`{k}` is {other:?}"),
    };
    let (h, mr) = (fixed("H"), fixed("H:mr"));
    let payload = to_payload(&r.recipe, v).unwrap();
    let hash: String = smysl_core::hash_bytes(&payload)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!(
        "{bits}\t{n}\t{seed}\t{h}\t{mr}\t{}\t{}\t{hash}",
        smysl_measure::num::render_q20(h, 6),
        smysl_measure::num::render_q20(mr, 6)
    )
}
