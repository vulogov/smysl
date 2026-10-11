//! The range tables, and the claim that a range answers without reading the corpus (TX-P4 step 3).
//!
//! # Why the instrument counts rows rather than milliseconds
//!
//! The exit clause is "a `said during` range over 155k units answers **without sorting the
//! store**, asserted by an instrumented test". A timing would not assert that: a fast machine
//! passes a scan and a slow one fails an index. What the clause is about is *work*, so the
//! structure counts the rows a query looked at — misses included, so it cannot flatter itself by
//! not counting the rows it rejected — and the test asserts that the count is proportional to the
//! answer and not to the corpus.
//!
//! The rows here are synthesised directly rather than grown out of a library, which is the right
//! instrument for this claim: the query's cost is a function of the table, and building 155,510
//! units to produce 155,510 rows would measure the append path. The library path is exercised at
//! this size in `tests/postings_timing.rs`, and at fixture size by `tests/index.rs`, where the
//! two builders' dumps are compared byte for byte with these three tables in them.

use smysl_core::Uid;
use smysl_text::index::ranges::{self, Bound, Instant, Ranges, Row};
use smysl_text::time::{Interval, Subject};

/// The size the clause names.
const UNITS: usize = 155_510;

fn uid(i: usize) -> Uid {
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(&(i as u64).to_be_bytes());
    Uid::from_bytes(bytes)
}

fn row(i: usize, lo: i64, hi: i64) -> Row {
    Row::new(
        Subject::Unit(uid(i)),
        Bound::At(Instant(lo)),
        Bound::At(Instant(hi)),
    )
}

/// A deterministic LCG, because a fuzzed window set should be the same window set on every
/// machine — the rule every other generated fixture here is under.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        self.0 >> 11
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// An hour apart each, which is a chat's shape and the one `observed` produces.
fn dense(n: usize) -> Vec<Row> {
    (0..n)
        .map(|i| {
            let at = 1_700_000_000_000 + (i as i64) * 3_600_000;
            row(i, at, at + 1)
        })
        .collect()
}

#[test]
fn the_answer_is_the_one_a_scan_gives() {
    // Varied deliberately: instants an hour apart, long overlapping intervals, and rows open at
    // one end. A table of equal-width non-overlapping rows agrees with a scan under almost any
    // implementation of the descent.
    let mut rows = dense(2_000);
    for i in 0..200 {
        let at = 1_700_000_000_000 + (i as i64) * 86_400_000;
        rows.push(row(10_000 + i as usize, at, at + 30 * 86_400_000));
    }
    for i in 0..50 {
        rows.push(Row::new(
            Subject::Unit(uid(20_000 + i as usize)),
            Bound::At(Instant(1_700_000_000_000 + (i as i64) * 1_000_000)),
            Bound::Open,
        ));
        rows.push(Row::new(
            Subject::Unit(uid(30_000 + i as usize)),
            Bound::Open,
            Bound::At(Instant(1_700_000_000_000 + (i as i64) * 1_000_000)),
        ));
    }
    let table = Ranges::of(rows.clone(), false);

    let mut rng = Rng(7);
    let mut answered = 0usize;
    for _ in 0..500 {
        let lo = 1_700_000_000_000 + rng.below(4_000) as i64 * 3_600_000;
        let width = rng.below(20) as i64 * 3_600_000;
        let window = Interval::of(lo, lo + width + 1);
        let mut fast = table.during(window);
        fast.sort_unstable();
        fast.dedup();
        assert_eq!(
            fast,
            ranges::scan(table.rows(), window),
            "the descent and the scan disagree about {window:?}"
        );
        answered += fast.len();
    }
    assert!(
        answered > 1_000,
        "only {answered} subjects across 500 windows — the windows missed the corpus"
    );

    // And the two unbounded windows, which are the edges a descent gets wrong: one admits
    // everything the table holds, the other is the same question asked with no bounds at all.
    let all = table.during(Interval::UNDATED);
    assert_eq!(
        all.len(),
        table.rows().len(),
        "an unbounded window is every row"
    );
}

/// The exit clause.
#[test]
fn a_range_over_155k_units_answers_without_reading_them() {
    let table = Ranges::of(dense(UNITS), false);
    assert_eq!(table.len(), UNITS);

    // A day's window, in the middle of a corpus spanning eighteen years.
    let lo = 1_700_000_000_000 + (UNITS as i64 / 2) * 3_600_000;
    let window = Interval::of(lo, lo + 86_400_000);
    let (answer, visited) = table.instrumented(window);

    assert_eq!(
        answer.len(),
        24,
        "a day of a corpus recorded hourly is twenty-four units"
    );
    assert_eq!(
        answer,
        ranges::scan(table.rows(), window),
        "the fast answer is not the answer"
    );
    // The claim. Twenty-four answers out of 155,510 rows: the descent prunes by the subtree
    // maximum, so what it reads is the answer plus the boundary of the descent — `O(k log n)` —
    // and nothing proportional to the corpus. A scan would read 155,510; sorting the store would
    // read them all and move them too.
    assert!(
        visited < 200,
        "the query read {visited} of {UNITS} rows, which is a scan wearing an index's clothes"
    );
    println!(
        "155k rows, a day's window: {} answers, {visited} rows read",
        answer.len()
    );

    // A window before the corpus and one after it: both answer nothing, and the one before reads
    // nothing at all, because the prefix search ends before the first row.
    let (before, read_before) = table.instrumented(Interval::of(0, 1));
    assert!(before.is_empty());
    assert_eq!(
        read_before, 0,
        "a window before everything read {read_before} rows"
    );
    let (after, read_after) = table.instrumented(Interval::of(i64::MAX - 2, i64::MAX - 1));
    assert!(after.is_empty());
    assert!(
        read_after < 200,
        "a window after everything read {read_after} rows"
    );
}

/// A-2.4 rule 5: undated is not early and not late, so it is not in a range table.
#[test]
fn an_undated_subject_is_not_a_row() {
    let rows = vec![
        row(1, 10, 20),
        Row::new(Subject::Unit(uid(2)), Bound::Open, Bound::Open),
    ];
    let table = Ranges::of(rows, false);
    assert_eq!(table.len(), 1, "the undated row was kept");
    // The reason it must not be kept: `Interval::UNDATED` intersects every window, so an undated
    // row would answer every question about every period.
    assert_eq!(table.during(Interval::of(0, 5)).len(), 0);
    assert_eq!(table.during(Interval::of(15, 16)).len(), 1);
}

/// A bound that is open is unbounded on the side it is on, and not a sentinel instant.
#[test]
fn an_open_bound_is_unbounded_on_its_own_side() {
    let from_1984 = Row::new(
        Subject::Unit(uid(1)),
        Bound::At(Instant(441_763_200_000)),
        Bound::Open,
    );
    let until_1984 = Row::new(
        Subject::Unit(uid(2)),
        Bound::Open,
        Bound::At(Instant(441_763_200_000)),
    );
    let table = Ranges::of(vec![from_1984, until_1984], false);

    // Far future: only the row that never ends.
    let future = table.during(Interval::of(4_000_000_000_000, 4_000_000_000_001));
    assert_eq!(future, vec![Subject::Unit(uid(1))]);
    // Far past: only the row that never started.
    let past = table.during(Interval::of(0, 1));
    assert_eq!(past, vec![Subject::Unit(uid(2))]);
    // The instant itself: half-open, so `[lo, hi)` includes the start and excludes the end.
    let at = table.during(Interval::of(441_763_200_000, 441_763_200_001));
    assert_eq!(at, vec![Subject::Unit(uid(1))]);
}

/// The rows are in `lo` order and hold each subject once, which is what the dump's determinism
/// rests on.
#[test]
fn the_rows_are_sorted_and_deduped() {
    let mut rows = dense(50);
    rows.reverse();
    rows.extend(dense(50));
    let table = Ranges::of(rows, false);
    assert_eq!(table.len(), 50, "the duplicates survived");
    assert!(
        table.rows().windows(2).all(|w| w[0] <= w[1]),
        "the rows are not in order"
    );

    // Two orders of arrival, one table: the same property the eleven tables are gated on.
    let a = Ranges::of(dense(50), false);
    let mut shuffled = dense(50);
    let mut rng = Rng(3);
    for i in (1..shuffled.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        shuffled.swap(i, j);
    }
    assert_eq!(
        a,
        Ranges::of(shuffled, false),
        "arrival order built a different table"
    );
}

/// An empty table answers nothing and reads nothing, rather than panicking in the descent.
#[test]
fn an_empty_table_answers_nothing() {
    let table = Ranges::of(Vec::new(), false);
    assert!(table.is_empty());
    let (answer, visited) = table.instrumented(Interval::of(0, 1));
    assert!(answer.is_empty());
    assert_eq!(visited, 0);
}
