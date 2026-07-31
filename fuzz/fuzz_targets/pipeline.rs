//! Guarantee A1 across the pipeline, and rule L on every derived thread.
//!
//! A1 says no panics on untrusted input. Until now that was tested on the two *parsers*,
//! which is the narrow reading: a store arriving from another agent has been through a
//! parser, but the graph it describes is still adversarial. A cycle in `supersedes`, a
//! rebuttal pointing at its own rebutter, a thread whose steps name units the store no
//! longer holds — none of that is a parse error, and all of it reaches `check`, `salience`,
//! `derive_thread` and `render` unmediated.
//!
//! Rule L is asserted here rather than left to `check`, because the repair pass is supposed
//! to *establish* it. A thread that still violates rule L after repair is a thread whose
//! steps reference units whose deps are absent — an argument with a hole in it, rendered
//! without complaint.

#![no_main]
use libfuzzer_sys::fuzz_target;
use smysl_check::{check, CheckOptions};
use smysl_core::{AgentId, ThreadId, ThreadSchema};
use smysl_fuzz::{generate, Choices};
use smysl_graph::{salience, SalienceRequest};
use smysl_thread::{derive_thread, satisfies_rule_l, DeriveOptions};

fuzz_target!(|data: &[u8]| {
    let mut c = Choices::new(data);
    let store = generate(&mut c, 14);

    // Each of these must return rather than abort, whatever the graph looks like.
    let _ = check(&store, CheckOptions::default());
    let _ = salience(&store, &SalienceRequest::default());

    for &schema in ThreadSchema::ALL {
        let opts = DeriveOptions::new(
            ThreadId::new("t/derived").unwrap(),
            AgentId::new("tool:test").unwrap(),
        );
        let (thread, report) = derive_thread(&store, schema, &opts);

        let broken = satisfies_rule_l(&store, &thread);
        assert!(
            broken.is_empty(),
            "{schema}: {} step(s) reference a unit whose deps are absent after repair \
             (repaired {})",
            broken.len(),
            report.repaired.len()
        );

        // Derivation is a pure function of the store, so a second call must agree. A
        // difference here is nondeterminism reaching a user-visible artifact.
        let (again, _) = derive_thread(&store, schema, &opts);
        assert_eq!(thread, again, "{schema}: derivation is not reproducible");
    }
});
