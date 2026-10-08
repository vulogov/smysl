//! The limits suite (SMYSL-2.4 §5.1): one case per cap, and the properties that hold across
//! all of them.
//!
//! §5.1 asks for "one crafted input per cap per reader". The readers arrive in TX-P1 step 3,
//! so what this file can hold is the other half of that sentence: every cap, exercised
//! through the paths that charge it, each refusing with `SMY-E440` naming the cap and its
//! flag. When a reader lands, its crafted input joins this file rather than replacing it —
//! the arithmetic is checked once here, and a reader's test then only has to show that the
//! reader charges.
//!
//! The property worth more than any single case is at the bottom: a refusal happens at the
//! same point every time, on every machine, because nothing in the path reads a clock.

use smysl_core::error::LibError;
use smysl_core::Code;
use smysl_text::limits::{Budget, Caps, Fuel, CAPS};
use smysl_text::reading::{Level, Segment};
use smysl_text::structure::{BuildError, Structure};
use smysl_text::{locator, part, Policy};

fn cap_of(e: &LibError) -> &'static str {
    match e {
        LibError::Limit { cap, .. } => cap,
        other => panic!("not a cap: {other}"),
    }
}

/// Every refusal in this file, as (cap, the error).
///
/// Collected rather than asserted one at a time so the suite can state the two things that
/// must be true of *all* of them — `SMY-E440`, and a message naming the cap — once, and so a
/// cap that stops being reachable shows up as a missing row rather than as a test that
/// quietly stopped testing.
fn every_cap_refusal() -> Vec<(&'static str, LibError)> {
    let mut out = Vec::new();

    // input_bytes: refused before a budget exists at all.
    out.push((
        "input_bytes",
        Budget::new(
            Caps {
                input_bytes: 10,
                ..Caps::DEFAULT
            },
            11,
        )
        .unwrap_err(),
    ));

    // A deep JSON or XML document: depth, not count.
    let mut b = Budget::new(
        Caps {
            nesting: 3,
            ..Caps::DEFAULT
        },
        1_000,
    )
    .unwrap();
    for _ in 0..3 {
        b.enter().unwrap();
    }
    out.push(("nesting", b.enter().unwrap_err()));

    // A zip with one entry at a 1000:1 ratio, over the 1 MiB floor where the ratio is asked
    // about at all.
    let mut b = Budget::new(Caps::DEFAULT, 1_000).unwrap();
    out.push(("ratio", b.entry(2 << 20, 2 << 11).unwrap_err()));

    // One entry larger than the per-entry cap.
    let mut b = Budget::new(Caps::DEFAULT, 1_000).unwrap();
    out.push((
        "entry_bytes",
        b.entry(Caps::DEFAULT.entry_bytes + 1, 1 << 20).unwrap_err(),
    ));

    // 100,001 entries.
    let mut b = Budget::new(
        Caps {
            entries: 2,
            ..Caps::DEFAULT
        },
        1_000,
    )
    .unwrap();
    b.entry(1, 1).unwrap();
    b.entry(1, 1).unwrap();
    out.push(("entries", b.entry(1, 1).unwrap_err()));

    // The archive total, reached by entries each under their own cap.
    let mut b = Budget::new(
        Caps {
            decompressed_bytes: 100,
            ..Caps::DEFAULT
        },
        1_000,
    )
    .unwrap();
    b.entry(60, 60).unwrap();
    out.push(("decompressed_bytes", b.entry(60, 60).unwrap_err()));

    // A structure with more nodes than the cap.
    let mut b = Budget::new(
        Caps {
            nodes: 2,
            ..Caps::DEFAULT
        },
        1_000,
    )
    .unwrap();
    let rows: Vec<Segment> = (0..3)
        .map(|i| {
            Segment::new(
                i * 10,
                i * 10 + 10,
                Level::new("line").unwrap(),
                locator::parse(&format!("L{}", i + 1)).unwrap(),
            )
        })
        .collect();
    match Structure::build(&rows, 30, &mut b).unwrap_err() {
        BuildError::Limit(e) => out.push(("nodes", e)),
        other => panic!("expected a cap: {other}"),
    }

    // A part over the hard ceiling: one node, larger than any policy maximum.
    let mut b = Budget::new(
        Caps {
            part_bytes: 1_000,
            ..Caps::DEFAULT
        },
        1 << 20,
    )
    .unwrap();
    let policy = Policy::new(Level::new("chapter").unwrap(), 1, 10);
    out.push((
        "part_bytes",
        part::group(&[0..1_001, 1_001..1_010], &policy, &mut b).unwrap_err(),
    ));

    // Raw metadata: the per-item cap, which is the one a single pathological segment hits.
    let mut b = Budget::new(
        Caps {
            raw_bytes: (10, 100),
            ..Caps::DEFAULT
        },
        1_000,
    )
    .unwrap();
    out.push(("raw_bytes", b.raw(11).unwrap_err()));

    // Fuel: a pathological line that scans far more than it produces.
    let mut b = Budget::new(
        Caps {
            fuel: Fuel {
                per_byte: 2,
                base: 0,
            },
            ..Caps::DEFAULT
        },
        100,
    )
    .unwrap();
    b.scan(200).unwrap();
    out.push(("fuel", b.scan(1).unwrap_err()));

    out
}

#[test]
fn every_cap_in_the_table_has_a_case_in_this_suite() {
    let refusals = every_cap_refusal();
    for cap in CAPS {
        assert!(
            refusals.iter().any(|(c, _)| c == cap),
            "no case for the `{cap}` cap"
        );
    }
    assert_eq!(
        refusals.len(),
        CAPS.len(),
        "one case per cap, and no case for a cap that is not in the table"
    );
}

#[test]
fn every_cap_refuses_with_e440_naming_the_cap_and_its_flag() {
    for (cap, e) in every_cap_refusal() {
        assert_eq!(e.code(), Some(Code::E440), "{cap}");
        assert_eq!(cap_of(&e), cap, "the error names the cap it was raised for");
        let text = e.to_string();
        assert!(text.starts_with("SMY-E440"), "{cap}: {text}");
        assert!(text.contains(cap), "{cap}: {text}");
        if cap == "nesting" {
            assert!(
                !text.contains("--"),
                "nesting is the format's own cap and has no flag: {text}"
            );
        } else {
            assert!(text.contains("--max-"), "{cap}: {text} names no flag");
        }
    }
}

/// The limit and what the input asked for are both in the message.
///
/// Not one number. "Too many nodes" tells an operator nothing about whether their corpus is
/// slightly over or a thousand times over, which is the difference between raising a flag and
/// looking at the input.
#[test]
fn a_refusal_says_both_the_limit_and_what_was_asked_for() {
    let e = Budget::new(
        Caps {
            input_bytes: 1_000,
            ..Caps::DEFAULT
        },
        5_000,
    )
    .unwrap_err();
    match e {
        LibError::Limit { limit, saw, .. } => {
            assert_eq!(limit, 1_000);
            assert_eq!(saw, 5_000, "the declared size, not the cap plus one");
        }
        other => panic!("{other}"),
    }
    let text = e.to_string();
    assert!(text.contains("1000") && text.contains("5000"), "{text}");
}

/// Nothing is written when a cap is hit: the staging discipline in prose, as a property of
/// this layer rather than of the object store.
///
/// `part::group` is the last thing to run before objects are staged, and it returns a plan
/// rather than writing anything — so a refusal inside it cannot have written a part, whatever
/// the caller does next. The test pins the shape rather than the implementation: the error
/// path returns no plan at all, so there is nothing a caller could have written half of.
#[test]
fn a_refused_grouping_returns_no_plan() {
    let mut b = Budget::new(
        Caps {
            part_bytes: 50,
            ..Caps::DEFAULT
        },
        1_000,
    )
    .unwrap();
    let policy = Policy::new(Level::new("chapter").unwrap(), 10, 20);
    // The first two nodes would group fine; the third is over the ceiling on its own.
    let r = part::group(&[0..10, 10..20, 20..200], &policy, &mut b);
    assert!(r.is_err(), "and the two good parts are not returned either");
    assert_eq!(cap_of(&r.unwrap_err()), "part_bytes");
}

/// The property fuel exists for: the same input refuses at the same point, every time.
#[test]
fn a_refusal_is_reproducible_to_the_unit() {
    let caps = Caps {
        fuel: Fuel {
            per_byte: 1,
            base: 0,
        },
        ..Caps::DEFAULT
    };
    let mut spent_at = Vec::new();
    for _ in 0..5 {
        let mut b = Budget::new(caps, 1_000).unwrap();
        let mut n = 0u64;
        loop {
            if b.scan(7).is_err() {
                break;
            }
            n += 1;
        }
        spent_at.push((n, b.fuel_spent()));
    }
    assert!(
        spent_at.windows(2).all(|w| w[0] == w[1]),
        "fuel ran out at different points: {spent_at:?}"
    );
    assert_eq!(
        spent_at[0],
        (142, 1_000),
        "1000 / 7 = 142 whole charges, and the refused budget is then exhausted rather than \
         left with the six units that were not enough"
    );
}

/// A budget that has refused stays refused.
///
/// Otherwise a reader could retry its way past a cap one unit at a time, which is the failure
/// mode a per-call check has and a running total does not.
#[test]
fn a_budget_does_not_recover() {
    let mut b = Budget::new(
        Caps {
            fuel: Fuel {
                per_byte: 1,
                base: 0,
            },
            ..Caps::DEFAULT
        },
        10,
    )
    .unwrap();
    assert!(b.scan(11).is_err());
    for _ in 0..100 {
        assert!(b.scan(1).is_err(), "it must not refill");
    }
    assert_eq!(b.fuel_remaining(), 0);
}
