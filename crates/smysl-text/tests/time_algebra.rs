//! Rule E as properties: `P-E1`–`P-E8` (SMYSL-2.4 §5.3, TX-P3 step 2).
//!
//! The engine's job is to be *right about what it may conclude*, and that is not a thing a
//! worked example can establish. So the harness builds stores from a ground truth it knows —
//! instants assigned to subjects, as-recorded values derived from them at chosen statuses, and
//! datings derived from them too — and then checks eight claims about what comes out.
//!
//! | id | claim |
//! |---|---|
//! | `P-E1` | order independence: the output does not depend on the order records arrive in, or on how they were split between two stores |
//! | `P-E2` | idempotence: duplicating any record changes nothing |
//! | `P-E3` | no silent override: every bound that differs from as-recorded has a why-chain whose status is at least the bound's |
//! | `P-E4` | soundness: with no planted fault, every interval contains the ground truth and nothing is contested |
//! | `P-E5` | detection: each planted skew or contradiction yields a `W412`/`W413` naming a dating or constraint involved |
//! | `P-E6` | reference: for small networks the solver equals a brute-force Floyd–Warshall |
//! | `P-E7` | withdrawal: withdrawing a dating returns the output to that of the store without it |
//! | `P-E8` | monotone evidence: a live dating no better evidenced than the bounds it touches never changes an applied bound |
//!
//! A seeded xorshift, as `merge_algebra.rs` and `redaction_algebra.rs` have it, so a failure is
//! reproducible from its seed alone.

use std::collections::BTreeMap;

use smysl_core::ids::AgentId;
use smysl_core::types::provenance::Hlc;
use smysl_core::types::{
    Axis, Dating, DatingTarget, DatingValue, Record, SourceKind, SourceRef, Withdrawal,
};
use smysl_core::{Did, KernelType, Status, Uid, UnitCoreBuilder};
use smysl_graph::Store;
use smysl_text::time::engine::{effective, Cause};
use smysl_text::time::{Bound, Instant, Interval, Subject, TimeStatus};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len())]
    }
}

fn agent() -> AgentId {
    AgentId::new("human:vu").expect("an agent")
}

/// A generated scenario: the records, and the truth they were built from.
struct Scenario {
    records: Vec<Record>,
    /// The instant each subject really has.
    truth: BTreeMap<Subject, i64>,
    /// The datings written, so a property can withdraw one.
    datings: Vec<Did>,
}

/// Build a store whose as-recorded values and datings are all consistent with one truth.
///
/// Every value is *derived from* the truth rather than invented beside it, which is what makes
/// P-E4 a real claim: the engine may narrow an interval as much as the records justify, and the
/// truth must still be inside it.
fn consistent(rng: &mut Rng, units: usize) -> Scenario {
    let mut records = Vec::new();
    let mut truth = BTreeMap::new();
    let mut datings = Vec::new();
    // A day in 2024, so that every instant is comfortably inside the millisecond range and the
    // arithmetic never comes near a boundary.
    let base = 1_726_500_000_000i64;
    let mut uids = Vec::new();
    for i in 0..units {
        let at = base + (rng.below(1000) * 60_000) as i64;
        let core = UnitCoreBuilder::new(
            KernelType::Evidence,
            format!("a reading {i} long enough to clear the gist check"),
            Status::Cited,
        )
        .source(SourceRef::new(SourceKind::Metric, format!("m/{i}")).observed_at(at as u64))
        .build()
        .expect("a well-formed unit");
        let uid = smysl_core::canonical_uid(&core);
        truth.insert(Subject::Unit(uid), at);
        uids.push(uid);
        records.push(Record::Unit(core));
    }
    // Orderings that are true of the truth, written as relative datings so that the engine has
    // something to propagate. A dating with no basis is `speculative`, which is the weakest
    // stratum — and that is on purpose here: P-E4 must hold at every status.
    for _ in 0..units {
        if units < 2 {
            break;
        }
        let a = rng.below(units);
        let b = rng.below(units);
        if a == b {
            continue;
        }
        let (early, late) = if truth[&Subject::Unit(uids[a])] <= truth[&Subject::Unit(uids[b])] {
            (a, b)
        } else {
            (b, a)
        };
        let d = Dating::new(
            DatingTarget::Unit(uids[late]),
            Axis::Said,
            DatingValue::Relative {
                allen: smysl_core::Allen::After,
                target: DatingTarget::Unit(uids[early]),
            },
            agent(),
            Hlc::new(1, 0, agent()),
        );
        datings.push(d.did());
        records.push(Record::Dating(d));
    }
    // And absolute datings that are true: a year containing the instant.
    for uid in uids.iter().take(units.min(3)) {
        let d = Dating::new(
            DatingTarget::Unit(*uid),
            Axis::Said,
            DatingValue::Absolute("2024".to_string()),
            agent(),
            Hlc::new(2, 0, agent()),
        );
        datings.push(d.did());
        records.push(Record::Dating(d));
    }
    Scenario {
        records,
        truth,
        datings,
    }
}

fn store_of(records: &[Record]) -> Store {
    let mut s = Store::new();
    s.append(records).expect("records a store accepts");
    s
}

fn dated(store: &Store) -> BTreeMap<Subject, (Interval, bool)> {
    effective(store, Axis::Said)
        .dated
        .into_iter()
        .map(|(s, d)| (s, (d.interval, d.contested)))
        .collect()
}

/// P-E1: the answer is a function of the record set, not of its order or its splits.
#[test]
fn p_e1_order_independence() {
    let mut rng = Rng(0x51A5_1A5E_5EED_0001);
    for case in 0..24 {
        let s = consistent(&mut rng, 2 + case % 5);
        let want = dated(&store_of(&s.records));
        // Every record list is a permutation away from every other.
        for _ in 0..4 {
            let mut shuffled = s.records.clone();
            for i in (1..shuffled.len()).rev() {
                let j = rng.below(i + 1);
                shuffled.swap(i, j);
            }
            assert_eq!(dated(&store_of(&shuffled)), want, "case {case}: permuted");
        }
        // And a split into two stores merged either way is the same store.
        let cut = s.records.len() / 2;
        let (a, b) = (0..cut, cut..s.records.len());
        let mut left = store_of(&s.records[a.clone()]);
        left.append(&s.records[b.clone()]).expect("the rest");
        let mut right = store_of(&s.records[b]);
        right.append(&s.records[a]).expect("the rest");
        assert_eq!(dated(&left), want, "case {case}: split, left first");
        assert_eq!(dated(&right), want, "case {case}: split, right first");
    }
}

/// P-E2: a record twice is a record once.
#[test]
fn p_e2_idempotence() {
    let mut rng = Rng(0x51A5_1A5E_5EED_0002);
    for case in 0..16 {
        let s = consistent(&mut rng, 2 + case % 4);
        let want = dated(&store_of(&s.records));
        let mut doubled = s.records.clone();
        doubled.extend(s.records.iter().cloned());
        assert_eq!(dated(&store_of(&doubled)), want, "case {case}");
        // And re-appending to a store that already holds them.
        let mut store = store_of(&s.records);
        store.append(&s.records).expect("duplicates");
        assert_eq!(dated(&store), want, "case {case}: re-appended");
    }
}

/// P-E3: every bound that moved can say what moved it, at a status that justifies it.
#[test]
fn p_e3_no_silent_override() {
    let mut rng = Rng(0x51A5_1A5E_5EED_0003);
    for case in 0..16 {
        let s = consistent(&mut rng, 2 + case % 5);
        let store = store_of(&s.records);
        let e = effective(&store, Axis::Said);
        for (subject, d) in &e.dated {
            if d.contested {
                continue;
            }
            for (bound, why, status) in [
                (d.interval.lo, d.why_lo, d.lo_status),
                (d.interval.hi, d.why_hi, d.hi_status),
            ] {
                if bound == Bound::Open {
                    continue;
                }
                let why = why.unwrap_or_else(|| panic!("{subject} has a bound and no reason"));
                assert!(
                    why.at >= status,
                    "{subject}: a bound at {status} justified at {}",
                    why.at
                );
                // And a bound that is not the as-recorded one names a record.
                if why.cause != Cause::AsRecorded {
                    assert!(
                        matches!(
                            why.cause,
                            Cause::Dating(_) | Cause::Offset(_) | Cause::Free(_)
                        ),
                        "{subject}: {:?} is not a record anybody wrote",
                        why.cause
                    );
                }
            }
        }
    }
}

/// P-E4: with no planted fault, the truth is inside every interval and nothing is contested.
#[test]
fn p_e4_soundness() {
    let mut rng = Rng(0x51A5_1A5E_5EED_0004);
    for case in 0..32 {
        let s = consistent(&mut rng, 1 + case % 6);
        let store = store_of(&s.records);
        let e = effective(&store, Axis::Said);
        assert_eq!(
            e.contested().count(),
            0,
            "case {case}: consistent records produced a contested subject"
        );
        assert!(
            e.inconsistent.is_empty(),
            "case {case}: {:?}",
            e.inconsistent
        );
        for (subject, at) in &s.truth {
            let i = e.interval(subject);
            assert!(
                i.contains(Instant(*at)),
                "case {case}: {subject} is really at {at} and the engine says {i}"
            );
        }
    }
}

/// P-E5: a planted contradiction is detected and names a record involved.
#[test]
fn p_e5_detection() {
    let mut rng = Rng(0x51A5_1A5E_5EED_0005);
    for case in 0..16 {
        let mut s = consistent(&mut rng, 2 + case % 3);
        let uids: Vec<Uid> = s
            .truth
            .keys()
            .filter_map(|k| match k {
                Subject::Unit(u) => Some(*u),
                _ => None,
            })
            .collect();
        // A date that cannot be: a unit whose instant is in 2024, dated to 1066.
        let planted = Dating::new(
            DatingTarget::Unit(uids[0]),
            Axis::Said,
            DatingValue::Absolute("1066".to_string()),
            agent(),
            Hlc::new(9, 0, agent()),
        );
        let did = planted.did();
        s.records.push(Record::Dating(planted));
        let store = store_of(&s.records);
        let e = effective(&store, Axis::Said);
        let named = e
            .inconsistent
            .iter()
            .chain(&e.not_applied)
            .any(|c| c.datings.contains(&did));
        assert!(
            named,
            "case {case}: a 1066 date on a 2024 unit was neither refused nor reported: \
             inconsistent={:?} not_applied={:?}",
            e.inconsistent, e.not_applied
        );
    }
}

/// P-E5's other half: a weaker dating is reported when it cannot be applied.
#[test]
fn p_e5_a_weaker_date_that_cannot_apply_is_reported() {
    // A unit whose instant is `measured` — an instrument put it there — and a `speculative`
    // absolute dating that would narrow it. The dating is live, it changes nothing, and that is
    // exactly what `SMY-W412` exists to say. The **offset** form of the same fault is
    // `an_unevidenced_offset_cannot_move_a_measured_instant` below; this test's first version
    // described the offset in its comment and tested the absolute, which is how the offset
    // path came to sit outside A-12.2 unnoticed.
    let core = UnitCoreBuilder::new(
        KernelType::Evidence,
        "a measurement from an instrument, long enough to pass",
        Status::Measured,
    )
    .source(SourceRef::new(SourceKind::Metric, "pool.wait_ms").observed_at(1_726_500_000_000))
    .build()
    .expect("a unit");
    let uid = smysl_core::canonical_uid(&core);
    let att = smysl_core::types::Attestation::new(
        uid,
        agent(),
        smysl_core::Op::Imported,
        smysl_core::Rung::Computed,
        // **After** the instant it attests, which is the ordinary case and which this fixture
        // got wrong first time: an attestation clock before the instant is itself a temporal
        // inconsistency — the store cannot have heard of a reading before it was taken — and
        // the first-seen constraint found it before the dating under test was reached.
        Hlc::new(1_726_500_001_000, 0, agent()),
    );
    // A *different* kind of fault from the offset: an absolute dating at a weaker status that
    // would narrow a measured instant.
    let weak = Dating::new(
        DatingTarget::Unit(uid),
        Axis::Said,
        DatingValue::Absolute("1999".to_string()),
        agent(),
        Hlc::new(2, 0, agent()),
    );
    let did = weak.did();
    let store = store_of(&[
        Record::Unit(core),
        Record::Attestation(att),
        Record::Dating(weak),
    ]);
    let e = effective(&store, Axis::Said);
    let d = e.of(&Subject::Unit(uid)).expect("the unit is dated");
    assert_eq!(
        d.lo_status,
        TimeStatus::Measured,
        "the instrument's instant"
    );
    assert!(
        e.inconsistent
            .iter()
            .chain(&e.not_applied)
            .any(|c| c.datings.contains(&did)),
        "a speculative dating against a measured instant must be reported, not applied"
    );
    assert_eq!(
        d.interval,
        Interval::at(Instant(1_726_500_000_000)),
        "and the measured instant must be untouched"
    );
}

/// The offset path, under the same rule as every other value.
///
/// Step 2 applied an offset unconditionally and took the weaker of the two statuses, so an
/// unevidenced `--value offset:` shifted a `measured` instant and relabelled it `speculative`:
/// one command erasing a measurement's standing. A-12.2's no-silent-override and §5.2's own
/// scenario both say the move is not made and is reported, so it is.
#[test]
fn an_unevidenced_offset_cannot_move_a_measured_instant() {
    let at: u64 = 1_726_500_000_000;
    let core = UnitCoreBuilder::new(
        KernelType::Evidence,
        "a measurement from an instrument, long enough to pass admission",
        Status::Measured,
    )
    .source(SourceRef::new(SourceKind::Metric, "pool.wait_ms").observed_at(at))
    .build()
    .expect("a unit");
    let uid = smysl_core::canonical_uid(&core);
    let att = smysl_core::types::Attestation::new(
        uid,
        agent(),
        smysl_core::Op::Imported,
        smysl_core::Rung::Computed,
        Hlc::new(at + 1_000, 0, agent()),
    );
    // No basis, so `speculative` however confident its author (D-4).
    let skew = Dating::new(
        DatingTarget::Unit(uid),
        Axis::Said,
        DatingValue::Offset(-93_000),
        agent(),
        Hlc::new(2, 0, agent()),
    );
    let did = skew.did();
    let store = store_of(&[
        Record::Unit(core),
        Record::Attestation(att),
        Record::Dating(skew),
    ]);
    let e = effective(&store, Axis::Said);
    let d = e.of(&Subject::Unit(uid)).expect("the unit is dated");
    assert_eq!(
        d.interval,
        Interval::at(Instant(at as i64)),
        "the measured instant must not have moved"
    );
    assert_eq!(
        d.lo_status,
        TimeStatus::Measured,
        "nor may its standing have been lowered to the offset's"
    );
    assert!(
        e.not_applied.iter().any(|c| c.datings.contains(&did)),
        "and the offset must be reported: not_applied={:?}",
        e.not_applied
    );
}

/// P-E5's third case: the free constraint that fires on its own.
///
/// A unit whose `observed` is *after* the earliest attestation of it. The store cannot have
/// heard of a reading before it was taken, so the two records cannot both be right — and no
/// dating is involved at all, which is why the contention names a constraint rather than a
/// position.
#[test]
fn p_e5_a_unit_said_after_the_store_heard_of_it_is_inconsistent() {
    let core = UnitCoreBuilder::new(
        KernelType::Evidence,
        "a reading whose clock is ahead of the store's, long enough to pass",
        Status::Measured,
    )
    .source(SourceRef::new(SourceKind::Metric, "pool.wait_ms").observed_at(1_726_500_000_000))
    .build()
    .expect("a unit");
    let uid = smysl_core::canonical_uid(&core);
    let att = smysl_core::types::Attestation::new(
        uid,
        agent(),
        smysl_core::Op::Imported,
        smysl_core::Rung::Computed,
        Hlc::new(1_000, 0, agent()),
    );
    let store = store_of(&[Record::Unit(core), Record::Attestation(att)]);
    let e = effective(&store, Axis::Said);
    assert_eq!(e.contested().count(), 1);
    let c = &e.inconsistent[0];
    assert_eq!(c.over, Subject::Unit(uid));
    assert!(c.datings.is_empty(), "no dating is involved");
    assert_eq!(
        c.free,
        vec![smysl_text::time::constraints::FreeKind::FirstSeen]
    );
}

/// P-E6: the solver and a brute-force Floyd–Warshall agree, on random small networks.
#[test]
fn p_e6_reference() {
    use smysl_text::time::stn::Network;
    let mut rng = Rng(0x51A5_1A5E_5EED_0006);
    for case in 0..400 {
        let vars = 1 + rng.below(8);
        let mut n = Network::new(vars);
        let mut by = 0u32;
        for _ in 0..1 + rng.below(3 * vars) {
            by += 1;
            match rng.below(4) {
                0 => n.lower(rng.below(vars), rng.below(200) as i64, by),
                1 => n.upper(rng.below(vars), rng.below(200) as i64, by),
                2 => n.no_later(
                    rng.below(vars),
                    rng.below(vars),
                    rng.below(11) as i64 - 5,
                    by,
                ),
                _ => n.same(rng.below(vars), rng.below(vars), by),
            }
        }
        let fast = n.solve();
        let (upper, lower, negative) = n.solve_floyd_warshall();
        assert_eq!(
            fast.inconsistent.is_empty(),
            !negative,
            "case {case}: disagreed about consistency over {:?}",
            n.edges()
        );
        if negative {
            continue;
        }
        assert_eq!(fast.upper, upper, "case {case} upper");
        assert_eq!(fast.lower, lower, "case {case} lower");
    }
}

/// P-E7: a withdrawn dating leaves the store as if it had never arrived.
#[test]
fn p_e7_withdrawal() {
    let mut rng = Rng(0x51A5_1A5E_5EED_0007);
    for case in 0..16 {
        let s = consistent(&mut rng, 2 + case % 4);
        if s.datings.is_empty() {
            continue;
        }
        let victim = s.datings[rng.below(s.datings.len())];
        // Without the dating at all.
        let without: Vec<Record> = s
            .records
            .iter()
            .filter(|r| match r {
                Record::Dating(d) => d.did() != victim,
                _ => true,
            })
            .cloned()
            .collect();
        // With the dating and a withdrawal naming its did (A-6).
        let mut withdrawn = s.records.clone();
        withdrawn.push(Record::Withdrawal(Withdrawal::new(
            Uid::from_bytes(*victim.as_bytes()),
            agent(),
            Hlc::new(5, 0, agent()),
        )));
        assert_eq!(
            dated(&store_of(&withdrawn)),
            dated(&store_of(&without)),
            "case {case}: withdrawing {victim} did not undo it"
        );
    }
}

/// P-E8: evidence that is no better than what is there does not move it.
#[test]
fn p_e8_monotone_evidence() {
    let mut rng = Rng(0x51A5_1A5E_5EED_0008);
    for case in 0..16 {
        let s = consistent(&mut rng, 2 + case % 4);
        let before = dated(&store_of(&s.records));
        let uids: Vec<Uid> = s
            .truth
            .keys()
            .filter_map(|k| match k {
                Subject::Unit(u) => Some(*u),
                _ => None,
            })
            .collect();
        // A `speculative` dating — no basis — that is *wider* than what is already known: it
        // can justify nothing, so no applied bound may move.
        let target = rng.pick(&uids);
        let wide = Dating::new(
            DatingTarget::Unit(target),
            Axis::Said,
            DatingValue::Absolute("20XX".to_string()),
            agent(),
            Hlc::new(7, 0, agent()),
        );
        let mut records = s.records.clone();
        records.push(Record::Dating(wide));
        let after = dated(&store_of(&records));
        assert_eq!(after, before, "case {case}: a wider date moved a bound");
    }
}

/// The free constraint that has data: nothing was said after the store first heard of it.
#[test]
fn a_first_seen_ceiling_bounds_a_unit_nobody_dated() {
    let core = UnitCoreBuilder::new(
        KernelType::Claim,
        "a claim with no date on it at all, long enough to pass",
        Status::Speculative,
    )
    .build()
    .expect("a unit");
    let uid = smysl_core::canonical_uid(&core);
    let att = smysl_core::types::Attestation::new(
        uid,
        agent(),
        smysl_core::Op::Authored,
        smysl_core::Rung::Document,
        Hlc::new(1_726_500_000_000, 0, agent()),
    );
    let store = store_of(&[Record::Unit(core), Record::Attestation(att)]);
    let e = effective(&store, Axis::Said);
    let d = e.of(&Subject::Unit(uid)).expect("a subject");
    assert_eq!(
        d.interval.hi,
        Bound::At(Instant(1_726_500_000_001)),
        "the earliest attestation is a ceiling on when it was said"
    );
    assert_eq!(d.interval.lo, Bound::Open, "and nothing bounds it below");
    assert_eq!(d.hi_status, TimeStatus::Derived);
    assert!(matches!(d.why_hi.expect("a reason").cause, Cause::Free(_)));
}

/// The axes that have no as-recorded value start undated rather than borrowing one.
#[test]
fn composed_and_about_start_undated() {
    let mut rng = Rng(0x51A5_1A5E_5EED_0009);
    let s = consistent(&mut rng, 3);
    let store = store_of(&s.records);
    for axis in [Axis::Composed, Axis::About] {
        let e = effective(&store, axis);
        for (subject, d) in &e.dated {
            assert!(
                d.interval.is_undated(),
                "{subject} has a {axis} time nobody wrote: {}",
                d.interval
            );
        }
    }
}
