//! Guarantee A2: a purely local command creates no provider thread.
//!
//! This file contains exactly one test, and that is the point rather than an oversight.
//!
//! The unit test in `runtime.rs` says so itself: *"This test can only observe the flag after
//! other tests have run, so it asserts the weaker, always-true half: once started, it stays
//! started."* An honest note about a real limit — every unit test in a crate shares one
//! process, so by the time any of them looks at the flag, some earlier test has almost
//! certainly started the runtime. The interesting half, that it is *not* started, is
//! unobservable there.
//!
//! Mutation testing in 0.12 made the cost visible: `is_started -> true` survives, because
//! nothing anywhere asserts the negative. A2 is a guarantee in the manual and was resting on a
//! test that documented its own inability to check it.
//!
//! An integration test binary is a fresh process. So the negative is observable here, exactly
//! once, provided nothing else in this file touches a provider — which is why nothing else is
//! in it.

/// Do the kind of work `smysl check` does — parse, hash, inspect — and confirm no thread was
/// spun up for it.
///
/// If this ever fails, something on a local path has reached into the provider runtime, and
/// the cost is not the thread: it is that `smysl check` on a laptop with no network would
/// start behaving like a program that has one.
#[test]
fn a_purely_local_workload_starts_no_runtime_thread() {
    assert!(
        !smysl_provider::runtime::is_started(),
        "the runtime was already started before this test did anything"
    );

    // Work representative of the local commands: build a unit, derive its identity, encode it.
    // None of this is a provider concern and none of it should wake the runtime.
    let core = smysl_core::UnitCoreBuilder::new(
        smysl_core::ids::KernelType::Claim,
        "a claim that never leaves the machine",
        smysl_core::Status::Speculative,
    )
    .build()
    .expect("a speculative claim needs nothing else");

    let uid = smysl_core::canonical_uid(&core);
    let bytes = smysl_core::to_cbor(&smysl_core::Record::Unit(core));
    let (back, _) = smysl_core::from_cbor(&bytes).expect("it round-trips");

    assert!(!bytes.is_empty(), "the work actually happened");
    assert!(matches!(back, smysl_core::Record::Unit(_)));
    // The display form abbreviates, so only the prefix is guaranteed. An earlier version of
    // this line asserted a character count, which was both wrong and incidental: what it is
    // for is showing the work happened, not pinning a formatting decision.
    assert!(uid.to_string().starts_with("b3:"), "a uid was derived");

    assert!(
        !smysl_provider::runtime::is_started(),
        "a purely local workload started the provider runtime; guarantee A2 says it must not"
    );
}
