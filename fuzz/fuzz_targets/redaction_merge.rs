//! Rule Z under merge: the redaction set only grows, and it carries nothing else with it.
//!
//! The record-level half of `P-Z1`–`P-Z4` (SMYSL-2.4 §5.3). The other half is about an object
//! store — a `Store` never holds a record 15 or 18, since `append` refuses every one with
//! `SMY-E452` — and that half lives in `crates/smysl-text/tests/redaction_algebra.rs`, where
//! there is a filesystem to hold the bytes rule Z is about. This target changes the search over
//! what *can* be fuzzed: the algebra of the redaction set itself.
//!
//! What it asserts, for arbitrary stores and arbitrary redactions over them:
//!
//! 1. **`is_redacted` is the union.** A part redacted by either peer is redacted by the merge,
//!    in both orders.
//! 2. **The set only grows.** Merging never un-redacts a part, which is the property a store
//!    that has let bytes go depends on when it meets a peer that still holds them.
//! 3. **A redaction carries nothing else.** The merged store's units, relations and manifests
//!    are the same with the redactions and without them — a redaction is not a retraction.

#![no_main]
use libfuzzer_sys::fuzz_target;
use smysl_core::types::library::Redaction;
use smysl_core::{AgentId, Hlc, Record, Tid};
use smysl_fuzz::{generate, Choices};
use smysl_graph::{merge, MergeOptions, Store};

/// A fixed clock, as the other algebra targets use: a difference between two merges has to be a
/// difference in the algebra rather than in the wall clock.
fn opts() -> MergeOptions {
    MergeOptions::default().with_now(Hlc::new(0, 0, AgentId::new("tool:test").unwrap()))
}

fn merged(a: &Store, b: &Store) -> Store {
    let mut out = a.clone();
    merge(&mut out, b, opts()).expect("merge does not fail without --fail-on-contention");
    out
}

/// Up to four redactions over tids the input chooses, as records.
///
/// The tids are derived from one byte each, so two peers can and do name the same part — which
/// is the case the union is for, and the one a generator of random 32-byte identities would
/// almost never produce.
fn redactions(c: &mut Choices<'_>) -> Vec<Record> {
    let agent = AgentId::new("human:z").expect("an agent");
    let mut out = Vec::new();
    for _ in 0..c.below(5) {
        let tid = Tid::of(&[c.below(6) as u8]);
        let mut r = Redaction::new(tid, agent.clone(), Hlc::new(c.below(4) as u64, 0, agent.clone()));
        if c.chance(3) {
            r.reason = None;
        }
        out.push(Record::Redaction(r));
    }
    out
}

fn with(store: &Store, records: &[Record]) -> Store {
    let mut all: Vec<Record> = store.iter().cloned().collect();
    all.extend(records.iter().cloned());
    Store::from_records(all)
}

fuzz_target!(|data: &[u8]| {
    let mut c = Choices::new(data);
    let plain_a = generate(&mut c, 8);
    let plain_b = generate(&mut c, 8);
    let za = redactions(&mut c);
    let zb = redactions(&mut c);

    let a = with(&plain_a, &za);
    let b = with(&plain_b, &zb);

    let forward = merged(&a, &b);
    let backward = merged(&b, &a);

    // 1 and 2 together: the union, in both orders, and never smaller than either side.
    assert_eq!(
        forward.state_hash(),
        backward.state_hash(),
        "merge(A,B) != merge(B,A) with redactions in play"
    );
    for tid in [0u8, 1, 2, 3, 4, 5].map(|n| Tid::of(&[n])) {
        let either = a.is_redacted(&tid) || b.is_redacted(&tid);
        assert_eq!(
            forward.is_redacted(&tid),
            either,
            "the merged redaction set is not the union"
        );
        assert_eq!(backward.is_redacted(&tid), either);
    }
    assert!(forward.redacted_count() >= a.redacted_count());
    assert!(forward.redacted_count() >= b.redacted_count());

    // Idempotent: merging a store that already holds the redactions changes nothing.
    assert_eq!(
        merged(&forward, &a).state_hash(),
        forward.state_hash(),
        "merging a subset moved the state"
    );

    // 3: a redaction carries nothing else. The same two stores without any redaction merge to
    // the same units, relations and manifests.
    let bare = merged(&plain_a, &plain_b);
    assert_eq!(
        bare.units().count(),
        forward.units().count(),
        "a redaction changed the unit count"
    );
    assert_eq!(
        bare.manifest_count(),
        forward.manifest_count(),
        "a redaction changed the manifest count"
    );
    assert_eq!(bare.relations().count(), forward.relations().count());
});
