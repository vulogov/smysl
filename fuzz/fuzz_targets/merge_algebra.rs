//! Rule U: merge is a join-semilattice.
//!
//! Commutative, associative, idempotent. If any of the three fails, two peers gossiping in
//! different orders reach different stores, and the mesh needs coordination — which is the
//! thing rule U exists to avoid. There is no diagnostic for it and no way to notice from
//! inside one peer: each simply believes its own store.
//!
//! The same three laws are asserted in `smysl-graph/tests/merge_algebra.rs` over a
//! fixed-seed generator. This target changes the search, not the properties.

#![no_main]
use libfuzzer_sys::fuzz_target;
use smysl_fuzz::{generate, Choices};
use smysl_graph::{merge, MergeOptions, Store};
use smysl_core::{AgentId, Hlc};

/// A fixed clock, so merge stays a pure function of its inputs and a difference between two
/// merges is a difference in the algebra rather than in the wall clock.
fn opts() -> MergeOptions {
    MergeOptions::default().with_now(Hlc::new(0, 0, AgentId::new("tool:test").unwrap()))
}

fn merged(a: &Store, b: &Store) -> Store {
    let mut out = a.clone();
    merge(&mut out, b, opts()).expect("merge does not fail without --fail-on-contention");
    out
}

fuzz_target!(|data: &[u8]| {
    let mut c = Choices::new(data);
    // Three stores from one input, so the fuzzer controls how they relate to each other —
    // overlapping uids included, which is the case that actually exercises the join.
    let a = generate(&mut c, 8);
    let b = generate(&mut c, 8);
    let d = generate(&mut c, 6);

    assert_eq!(
        merged(&a, &b).state_hash(),
        merged(&b, &a).state_hash(),
        "merge(A,B) != merge(B,A)"
    );

    assert_eq!(
        merged(&merged(&a, &b), &d).state_hash(),
        merged(&a, &merged(&b, &d)).state_hash(),
        "association changed the result"
    );

    let ab = merged(&a, &b);
    assert_eq!(
        merged(&ab, &b).state_hash(),
        ab.state_hash(),
        "merging B a second time changed the store"
    );
});
