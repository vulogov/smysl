//! Rule E over arbitrary stores: order independence, idempotence, and no silent override.
//!
//! The three of `P-E1`–`P-E8` that can be checked without a ground truth, driven by the shared
//! generator rather than by a seeded loop. `crates/smysl-text/tests/time_algebra.rs` has the
//! other five, because soundness and detection need a truth the harness assigned — and this
//! target has what that harness cannot: inputs nobody designed.
//!
//! 1. **`P-E1`.** The effective time of a store does not depend on the order its records
//!    arrived in. Asserted over the same records appended forwards and backwards, which is the
//!    cheapest permutation that exercises the whole list.
//! 2. **`P-E2`.** Appending every record a second time changes nothing.
//! 3. **`P-E3`.** Every bound that is not open names what set it, at a status that justifies
//!    it. This is the property stratification exists to deliver, and the one an off-by-one in
//!    the status comparison would quietly break.
//! 4. **Totality.** A store the generator can build never panics the engine and never leaves a
//!    bound crossed: a subject that is not contested has `lo < hi`.

#![no_main]
use libfuzzer_sys::fuzz_target;
use smysl_core::types::{Axis, Dating, DatingTarget, DatingValue};
use smysl_core::{AgentId, Allen, Hlc, Record, Uid};
use smysl_fuzz::{generate, Choices};
use smysl_graph::Store;
use smysl_text::time::engine::effective;
use smysl_text::time::Bound;

fn store_of(records: &[Record]) -> Store {
    let mut s = Store::new();
    // The generator can produce a record a log refuses — a part text, say — and a refusal is
    // not a finding here: this target is about the engine, and a batch that cannot be appended
    // never reaches it.
    if s.append(records).is_err() {
        return Store::new();
    }
    s
}

/// Up to four datings over units the input chooses.
///
/// The generator builds units, relations and attestations and no datings, so without this the
/// target would exercise the seeds and the free constraints and never a correction. The uids
/// come from the store, so every dating names something — an unresolvable target is its own
/// case and `tests/time_algebra.rs` covers it.
fn datings(c: &mut Choices<'_>, uids: &[Uid]) -> Vec<Record> {
    let agent = AgentId::new("tool:test").expect("an agent");
    let mut out = Vec::new();
    if uids.is_empty() {
        return out;
    }
    for i in 0..c.below(5) {
        let target = DatingTarget::Unit(uids[c.below(uids.len())]);
        let value = match c.below(3) {
            0 => DatingValue::Absolute(
                ["1066", "1984", "2024", "201X", "1984/1985", "1984?"][c.below(6)].to_string(),
            ),
            1 => DatingValue::Offset(c.below(7_200_001) as i64 - 3_600_000),
            _ => DatingValue::Relative {
                allen: [
                    Allen::Before,
                    Allen::After,
                    Allen::Meets,
                    Allen::Overlaps,
                    Allen::During,
                    Allen::Contains,
                    Allen::Equals,
                ][c.below(7)],
                target: DatingTarget::Unit(uids[c.below(uids.len())]),
            },
        };
        let mut d = Dating::new(
            target,
            [Axis::Said, Axis::Composed, Axis::About][c.below(3)],
            value,
            agent.clone(),
            Hlc::new(i as u64, 0, agent.clone()),
        );
        // Half of them with a basis, so both halves of D-4 are exercised: a dating with one
        // takes its status and a dating without one is `speculative`.
        if c.below(2) == 0 {
            d = d.with_basis(uids[c.below(uids.len())]);
        }
        out.push(Record::Dating(d));
    }
    out
}

fuzz_target!(|data: &[u8]| {
    let mut choices = Choices::new(data);
    let seeded = generate(&mut choices, 8);
    let mut records: Vec<Record> = seeded.iter().cloned().collect();
    if records.is_empty() {
        return;
    }
    let uids: Vec<Uid> = seeded.units().map(|(u, _)| *u).collect();
    records.extend(datings(&mut choices, &uids));
    let forward = store_of(&records);
    let mut reversed: Vec<Record> = records.clone();
    reversed.reverse();
    let backward = store_of(&reversed);

    for axis in [Axis::Said, Axis::Composed, Axis::About] {
        let a = effective(&forward, axis);
        let b = effective(&backward, axis);
        assert_eq!(
            a.dated.len(),
            b.dated.len(),
            "order changed how many subjects there are"
        );
        for (subject, dated) in &a.dated {
            let other = b.dated.get(subject).expect("the same subjects");
            assert_eq!(dated.interval, other.interval, "{subject}: {axis} moved");
            assert_eq!(dated.contested, other.contested, "{subject}: {axis}");
            assert_eq!(dated.lo_status, other.lo_status, "{subject}: {axis}");
            assert_eq!(dated.hi_status, other.hi_status, "{subject}: {axis}");

            // P-E3, and totality.
            if !dated.contested {
                if let (Bound::At(lo), Bound::At(hi)) = (dated.interval.lo, dated.interval.hi) {
                    assert!(lo.ms() < hi.ms(), "{subject}: {axis} is an empty interval");
                }
            }
            for (bound, why, status) in [
                (dated.interval.lo, dated.why_lo, dated.lo_status),
                (dated.interval.hi, dated.why_hi, dated.hi_status),
            ] {
                if bound == Bound::Open {
                    continue;
                }
                let why = why.expect("a bound names what set it");
                assert!(why.at >= status, "{subject}: {axis} justified below its status");
            }
        }

        // P-E2.
        let mut doubled = forward.clone();
        if doubled.append(&records).is_ok() {
            let c = effective(&doubled, axis);
            for (subject, dated) in &a.dated {
                assert_eq!(
                    c.dated.get(subject).map(|d| d.interval),
                    Some(dated.interval),
                    "{subject}: {axis} changed when a record arrived twice"
                );
            }
        }
    }
});
