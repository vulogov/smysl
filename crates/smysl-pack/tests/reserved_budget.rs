//! R21 (1.6): a pack that fits the caller's budget, not only its own.
//!
//! `budget(n)` counts in smysl's units. A caller sending the pack to a model has a different
//! budget — the context window, less the system prompt, less the question, less the room the
//! answer needs — and was packing to a fixed number and then trimming the result by hand, which
//! drops units from a selection the solver chose as a whole. `reserving(n)` states the other
//! occupants of the window so one decision covers both.

use smysl_core::surface::parse_surface;
use smysl_core::{PackError, Uid};
use smysl_graph::{salience, SalienceRequest, Store};
use smysl_pack::{pack, verify, CostModel, Estimator, ExternalCost, PackRequest};

const SRC: &str = "\
@claim c/pool { status: speculative }
~ The eu-west connection pool is saturated.
- Acquisition wait rose from 2 ms to 310 ms over the same window, on every host.

@evidence e/wait { status: speculative }
~ Pool acquisition wait rose from 2 ms to 310 ms.

@claim c/canary { status: speculative }
~ The 4.2 canary ran the same pool configuration without the regression.

@decision d/raise { status: speculative }
~ Raise the pool ceiling for eu-west.
";

fn corpus() -> Store {
    let out = parse_surface(SRC).unwrap();
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    Store::from_records(out.records)
}

fn packed(store: &Store, req: &PackRequest) -> smysl_pack::Pack {
    let p =
        pack(store, &salience(store, &SalienceRequest::default()), req).expect("the floor fits");
    assert!(
        verify(store, &p, req).is_empty(),
        "{:?}",
        verify(store, &p, req)
    );
    p
}

fn selection(p: &smysl_pack::Pack) -> Vec<(Uid, smysl_core::Lod)> {
    p.selection.iter().map(|(u, l)| (*u, *l)).collect()
}

/// A reservation is a statement about the budget, not a second one: what `budget(b).reserving(r)`
/// selects is exactly what `budget(b - r)` selects, at every b and r that leave room.
#[test]
fn reserving_selects_what_the_smaller_budget_selects() {
    let store = corpus();
    for b in [60u64, 90, 140, 200, 400] {
        for r in [0u64, 5, 25, 50] {
            if r >= b {
                continue;
            }
            let with = packed(&store, &PackRequest::budget(b).reserving(r));
            let without = packed(&store, &PackRequest::budget(b - r));
            assert_eq!(
                selection(&with),
                selection(&without),
                "budget {b} reserving {r} differs from budget {}",
                b - r
            );
            assert_eq!(with.used(), without.used());
        }
    }
}

/// The manifest reports the caller's budget, what was set aside, and what was spent — and the
/// property a caller checks, `used + reserved <= budget`, holds.
#[test]
fn the_manifest_reports_budget_reserved_and_used() {
    let store = corpus();
    let p = packed(&store, &PackRequest::budget(200).reserving(60));
    assert_eq!(p.info.budget, 200);
    assert_eq!(p.info.reserved, 60);
    assert_eq!(p.info.effective_budget(), 140);
    assert!(
        p.info.used + p.info.reserved <= p.info.budget,
        "used {} + reserved {} > budget {}",
        p.info.used,
        p.info.reserved,
        p.info.budget
    );

    // Reserving nothing is the pre-1.6 manifest, exactly.
    let plain = packed(&store, &PackRequest::budget(200));
    assert_eq!(plain.info.reserved, 0);
    assert_eq!(plain.info.effective_budget(), 200);
}

/// Reserving the whole window is a mistake with an answer, not an empty pack.
#[test]
fn reserving_the_whole_budget_is_an_error() {
    let store = corpus();
    let sal = salience(&store, &SalienceRequest::default());
    for (b, r) in [(200u64, 200u64), (200, 400), (1, 1)] {
        match pack(&store, &sal, &PackRequest::budget(b).reserving(r)) {
            Err(PackError::OverReserved { budget, reserved }) => {
                assert_eq!((budget, reserved), (b, r));
            }
            other => panic!("budget {b} reserving {r}: {other:?}"),
        }
    }
    // Zero reserved out of zero budget is the old infeasible case, not an over-reservation.
    assert!(matches!(
        pack(&store, &sal, &PackRequest::budget(0)),
        Ok(_) | Err(PackError::Infeasible { .. })
    ));
}

/// An infeasible floor reports a *total* budget, so the caller can retry with the number it is
/// given and the same reservation.
#[test]
fn an_infeasible_floor_reports_a_budget_the_caller_can_retry_with() {
    let store = corpus();
    let sal = salience(&store, &SalienceRequest::default());
    let focus: Vec<Uid> = store.units().map(|(u, _)| *u).collect();
    let req = PackRequest::budget(40)
        .reserving(20)
        .focusing(focus.clone());
    let Err(PackError::Infeasible { budget, required }) = pack(&store, &sal, &req) else {
        panic!("a 20-token window should not hold four units and their bodies");
    };
    assert_eq!(budget, 40, "the manifest reports what the caller asked for");
    assert!(required > budget);

    let retry = PackRequest::budget(required).reserving(20).focusing(focus);
    let p = pack(&store, &sal, &retry).expect("the reported budget is feasible");
    assert!(p.info.used + p.info.reserved <= p.info.budget);
}

/// A caller counting in its provider's tokens supplies the counter. `PackInfo` names it, and
/// `verify` accepts the pack built under it.
#[test]
fn an_external_estimator_names_itself_and_verifies() {
    fn one_per_word(t: &str) -> u64 {
        t.split_whitespace().count() as u64
    }

    let store = corpus();
    let e = ExternalCost::new("tiktoken/cl100k", one_per_word);
    let model = CostModel::new(Estimator::default(), Some(e));
    assert_eq!(model.id(), "tiktoken/cl100k");
    assert!(model.is_external());
    assert_eq!(model.text("the pool is saturated"), 4);
    // Not nameable back into existence: the id says which tokenizer counted, and only the
    // caller holds it.
    assert_eq!(Estimator::parse("tiktoken/cl100k"), None);

    let req = PackRequest::budget(40).reserving(10).counting_with(e);
    assert_eq!(req.cost_model(), model);
    let p = packed(&store, &req);
    assert_eq!(p.info.estimator, "tiktoken/cl100k");
    assert!(p.info.used + p.info.reserved <= p.info.budget);
    assert!(verify(&store, &p, &req).is_empty());

    // Counting differently really does pack differently: the same budget under smysl's own
    // estimator, which charges per four bytes rather than per word, carries less.
    let plain = PackRequest::budget(40).reserving(10);
    assert!(!plain.cost_model().is_external());
    let q = packed(&store, &plain);
    assert!(
        p.len() >= q.len(),
        "a cheaper ruler should not carry fewer units: {} vs {}",
        p.len(),
        q.len()
    );
}

/// The manifest round-trips, and a pack that reserved nothing encodes to the bytes it always did.
#[test]
fn the_reservation_round_trips_and_costs_nothing_when_unused() {
    use smysl_core::{from_cbor, to_cbor, PackInfo, Record};

    let with = PackInfo::new(4096, 900, "smysl/utf8-div4").reserving(1200);
    let (round, _) = from_cbor(&to_cbor(&Record::PackInfo(with.clone()))).unwrap();
    assert_eq!(round, Record::PackInfo(with));

    let without = PackInfo::new(4096, 900, "smysl/utf8-div4");
    let bytes = to_cbor(&Record::PackInfo(without.clone()));
    assert_eq!(
        from_cbor(&bytes).unwrap().0,
        Record::PackInfo(without.clone()),
        "a zero reservation must decode back to itself"
    );
    assert_eq!(
        bytes,
        to_cbor(&Record::PackInfo(without.reserving(0))),
        "reserving nothing must not add a key"
    );
}
