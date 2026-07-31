//! Constraints C1-C7, and the budget, over generated stores.
//!
//! A wrong pack still packs. There is no parse error to notice and no diagnostic to read:
//! the selection simply omits a ground, or a rebuttal, or costs more than it was allowed,
//! and the consumer receives an argument with a hole in it that looks complete. `verify`
//! decides whether a selection satisfies the constraints independently of the solver that
//! produced it, which is what makes it usable as an oracle here.
//!
//! Budgets are swept per store rather than fuzzed, because the interesting failures live at
//! the boundary between "everything fits" and "nothing does" — and 0.3.0 established those
//! are different code paths, with the whole-scope fast path taken only when the scope fits
//! at its top level.

#![no_main]
use libfuzzer_sys::fuzz_target;
use smysl_fuzz::{generate, Choices};
use smysl_graph::{salience, SalienceRequest};
use smysl_pack::{pack, verify, PackRequest};

fuzz_target!(|data: &[u8]| {
    let mut c = Choices::new(data);
    let store = generate(&mut c, 12);
    let s = salience(&store, &SalienceRequest::default());

    let mut last_value: Option<(u64, f64)> = None;
    for budget in [0u64, 5, 15, 40, 100, 250, 600, 5_000] {
        let req = PackRequest::budget(budget);
        // No focus, so the floor is empty and packing cannot be infeasible.
        let p = pack(&store, &s, &req).expect("no focus, so the floor is empty");

        let violations = verify(&store, &p, &req);
        assert!(
            violations.is_empty(),
            "budget {budget}: {}",
            violations
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        );

        assert!(
            p.used() <= budget,
            "budget {budget}: pack used {} tokens",
            p.used()
        );

        // Value is monotone in budget: more room can never buy less. Note *value*, not unit
        // count — a larger budget may legitimately take one expensive unit in place of two
        // cheap ones, so asserting the count would fail on correct packs. The property is
        // invisible at any single budget; it only exists as a pair.
        let value: f64 = p
            .selection
            .iter()
            .map(|(u, l)| smysl_pack::value(s.get(u), *l))
            .sum();
        if let Some((prev_budget, prev_value)) = last_value {
            assert!(
                value >= prev_value - 1e-9,
                "budget {budget} bought less value ({value}) than budget {prev_budget} \
                 ({prev_value})"
            );
        }
        last_value = Some((budget, value));
    }
});
