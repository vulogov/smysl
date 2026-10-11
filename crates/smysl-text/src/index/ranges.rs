//! The `said`, `composed` and `observed` range tables of RFC SMYSL-2.4 §3.8 (TX-P4 step 3).
//!
//! # What a range table is for
//!
//! "Which units were said during the 1990s" is a question about an **interval** per subject, not
//! a key per subject, and the answer is every subject whose interval overlaps the window. Without
//! a table that means computing effective time for the whole store and then sorting it, per
//! query. With one it is a binary search and a descent.
//!
//! # Three tables, two of which are the rule E engine's answer
//!
//! `said` and `composed` hold [`crate::time::engine::effective`] for their axis: the as-recorded
//! value, tightened by every live dating and constraint, stratified by status. `observed` holds
//! something much simpler — the instant a unit's source recorded, one millisecond wide — because
//! `observed` is a field and not an axis. Nothing propagates into it, so it has a genuinely local
//! incremental builder where the other two mostly do not.
//!
//! `about` has no table, and that is §3.8's choice rather than an omission: an `about` time is
//! reached through datings alone, so a corpus with no datings has an empty table, and one with
//! datings gets the same answer from the `said` machinery with the axis changed. A table nobody
//! asked for is a table to keep in agreement for nothing.
//!
//! # Why an undated subject is not a row
//!
//! A-2.4 rule 5: undated is *not early and not late*. [`Interval::UNDATED`] intersects every
//! window — it is the interval with no bounds — so a table holding undated rows would answer
//! "said during 1990" with every unit nobody has dated. The table holds the subjects that are
//! **known** to overlap something, and a half-bounded interval ("from 1984 onwards") is such a
//! subject while an unbounded one is not. A caller that wants the undated ones is asking the
//! complement of a different question, and `date show` answers it per subject.
//!
//! # Why the incremental builder recomputes, mostly
//!
//! Effective time is produced by constraint propagation: one dating can move any subject's
//! interval, and the only way to know which is to solve the network. So an append that can bear
//! on time recomputes the axis, and an append that cannot — units and manifests arriving into a
//! corpus with nothing dangling, whose seeds are their own fields — extends the table locally.
//! The condition is not a guess: a dating that named a subject nobody has appended yet is
//! recorded by the engine as *unresolved*, and a constraint whose endpoints are not both present
//! is recorded as *unbuilt*. While either is true, an arriving unit can resolve something, and
//! the local path is unsound. [`Ranges::danglers`] is that bit, and it is in the dump.
//!
//! Recomputing is affordable because of what this step measured: one axis is **51 ms** at 155k
//! subjects. It was 64 s before the step started, and the one-line reason is recorded on
//! `crate::time::engine`'s `seed`.

use std::collections::BTreeMap;

use smysl_core::types::library::Axis;
use smysl_core::types::Record;

use crate::time::engine::effective;
use crate::time::Interval;
pub use crate::time::{Bound, Instant, Subject};

/// One subject's interval on one axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct Row {
    /// Sorted on first, so the rows are in `lo` order and a window's prefix is a binary search.
    pub lo: Bound,
    pub hi: Bound,
    pub subject: Subject,
}

impl Row {
    /// One subject's interval. `#[non_exhaustive]` with a constructor, on the pattern
    /// `smysl_retrieve::Hit` set: a row plausibly gains a field — the status of a bound is the
    /// obvious candidate, since the engine knows it — and a caller outside the crate has to be
    /// able to build one, because the tests that check the fast answer against the slow one are
    /// in `tests/ranges.rs`.
    pub fn new(subject: Subject, lo: Bound, hi: Bound) -> Row {
        Row { lo, hi, subject }
    }

    /// Whether this row's interval overlaps `window`, both half-open, `Open` meaning unbounded.
    ///
    /// Not `Interval::intersect(..).is_empty()`: that treats an unbounded side as *no
    /// constraint*, which is right for intersecting two intervals and would make an undated row
    /// overlap everything. Undated rows are not in the table, and this says so in the one place
    /// a reader will look for it.
    pub fn overlaps(&self, window: Interval) -> bool {
        before(self.lo, window.hi) && before(window.lo, self.hi)
    }
}

/// `lo < hi` for two bounds of a half-open interval, with `Open` unbounded on the side it is on.
///
/// `before(Open, _)` and `before(_, Open)` are both true: a start with no bound is before every
/// end, and an end with no bound is after every start. That is the only reading under which a
/// half-bounded row behaves like the interval it denotes.
fn before(lo: Bound, hi: Bound) -> bool {
    match (lo, hi) {
        (Bound::At(a), Bound::At(b)) => a.0 < b.0,
        _ => true,
    }
}

/// A range table: rows in `lo` order, with a max-`hi` tree over them.
///
/// The tree is **derived from the rows** and is not persisted: it is `O(rows)` to build and would
/// be a second thing to keep in agreement for no gain. [`Ranges::rows`] is what the dump holds
/// and what §3.8's byte comparison compares.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ranges {
    rows: Vec<Row>,
    /// Whether the corpus held an unresolved dating or an unbuilt constraint when this was built.
    ///
    /// While it does, an arriving unit can resolve one, so the incremental builder has no sound
    /// local path and recomputes. See the module note.
    danglers: bool,
    /// `max(hi)` per node of a segment tree over `rows`, `None` meaning unbounded and therefore
    /// maximal. Derived; never written down.
    maxhi: Vec<Bound>,
}

impl Ranges {
    /// The rows, in `lo` order. What the dump writes.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Whether this table was built over a corpus with something dangling in it.
    pub fn danglers(&self) -> bool {
        self.danglers
    }

    /// How many subjects the table holds.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Build from rows that may be in any order, and in any state of repetition.
    ///
    /// Sorted and deduped here rather than by the callers, so that the two builders cannot
    /// disagree about the order the dump is written in — which is the whole content of §3.8's
    /// gate. Undated rows are dropped here too, for the reason the module gives, so that no
    /// caller has to remember to.
    pub fn of(rows: impl IntoIterator<Item = Row>, danglers: bool) -> Ranges {
        let mut rows: Vec<Row> = rows
            .into_iter()
            .filter(|r| !(r.lo == Bound::Open && r.hi == Bound::Open))
            .collect();
        rows.sort_unstable();
        rows.dedup();
        let maxhi = build_tree(&rows);
        Ranges {
            rows,
            danglers,
            maxhi,
        }
    }

    /// The subjects whose interval overlaps `window`, in row order.
    pub fn during(&self, window: Interval) -> Vec<Subject> {
        self.instrumented(window).0
    }

    /// [`Ranges::during`], and how many rows of the table were looked at.
    ///
    /// The second value is what TX-P4 step 3's exit clause is asserted with. "Answers without
    /// sorting the store" is a claim about work, and a claim about work needs a count rather than
    /// a timing: a timing says this machine was fast, a count says the answer did not touch the
    /// corpus. It counts **rows**, including the ones the descent examined and rejected, so it
    /// cannot flatter itself by not counting the misses.
    pub fn instrumented(&self, window: Interval) -> (Vec<Subject>, usize) {
        let mut out = Vec::new();
        let mut visited = 0usize;
        if self.rows.is_empty() {
            return (out, visited);
        }
        // Every row with `lo < window.hi`, which is a prefix because the rows are in `lo` order.
        // `partition_point` is a binary search and visits no rows as this counts them: what it
        // reads is the sort key, not the row's membership in the answer.
        let p = self.rows.partition_point(|r| before(r.lo, window.hi));
        if p == 0 {
            return (out, visited);
        }
        // Inside that prefix, every row with `hi > window.lo`. The tree lets a subtree whose
        // every `hi` is at or below `window.lo` be skipped whole, which is what keeps a narrow
        // window from reading a wide prefix.
        self.descend(1, 0, self.rows.len(), p, window, &mut out, &mut visited);
        (out, visited)
    }

    /// One node of the descent: `[lo, hi)` is the node's span, `limit` the prefix end.
    #[allow(clippy::too_many_arguments)]
    fn descend(
        &self,
        node: usize,
        lo: usize,
        hi: usize,
        limit: usize,
        window: Interval,
        out: &mut Vec<Subject>,
        visited: &mut usize,
    ) {
        if lo >= limit || lo >= hi {
            return;
        }
        // The prune: nothing in this subtree ends after the window starts.
        if !before(window.lo, self.maxhi[node]) {
            return;
        }
        if hi - lo == 1 {
            *visited += 1;
            let row = self.rows[lo];
            if row.overlaps(window) {
                out.push(row.subject);
            }
            return;
        }
        let mid = lo + (hi - lo) / 2;
        self.descend(node * 2, lo, mid, limit, window, out, visited);
        self.descend(node * 2 + 1, mid, hi, limit, window, out, visited);
    }
}

/// `max(hi)` per node, over a recursive halving of the rows, `Open` winning as unbounded.
fn build_tree(rows: &[Row]) -> Vec<Bound> {
    if rows.is_empty() {
        return Vec::new();
    }
    // 4n is the usual safe size for a recursive segment tree over a non-power-of-two span, and
    // it is two bytes a row at worst. A tighter bound would be a calculation to get wrong.
    let mut tree = vec![Bound::At(Instant(i64::MIN)); 4 * rows.len()];
    fill(&mut tree, rows, 1, 0, rows.len());
    tree
}

fn fill(tree: &mut [Bound], rows: &[Row], node: usize, lo: usize, hi: usize) -> Bound {
    if hi - lo == 1 {
        tree[node] = rows[lo].hi;
        return tree[node];
    }
    let mid = lo + (hi - lo) / 2;
    let left = fill(tree, rows, node * 2, lo, mid);
    let right = fill(tree, rows, node * 2 + 1, mid, hi);
    tree[node] = later(left, right);
    tree[node]
}

/// The later of two upper bounds, `Open` being later than every instant.
fn later(a: Bound, b: Bound) -> Bound {
    match (a, b) {
        (Bound::Open, _) | (_, Bound::Open) => Bound::Open,
        (Bound::At(x), Bound::At(y)) => Bound::At(Instant(x.0.max(y.0))),
    }
}

// ---------------------------------------------------------------------------
// said / composed: the engine's answer
// ---------------------------------------------------------------------------

/// Build an axis table by solving for effective time.
pub fn axis_rebuild(lib: &crate::library::Library, axis: Axis) -> Ranges {
    let eff = effective(lib.store(), axis);
    let danglers = !eff.unresolved.is_empty();
    Ranges::of(
        eff.dated.iter().map(|(subject, dated)| Row {
            lo: dated.interval.lo,
            hi: dated.interval.hi,
            subject: *subject,
        }),
        danglers,
    )
}

/// Bring an axis table forward over an appended batch.
///
/// Local when the batch can only seed new subjects and the corpus has nothing dangling;
/// otherwise a recompute, which is the only correct answer for a quantity a single dating can
/// move anywhere. See the module note.
pub fn axis_on_append(
    table: &mut Ranges,
    lib: &crate::library::Library,
    axis: Axis,
    records: &[Record],
) {
    if table.danglers() || records.iter().any(bears_on_time) {
        *table = axis_rebuild(lib, axis);
        return;
    }
    let mut rows: Vec<Row> = table.rows().to_vec();
    let mut touched = false;
    for record in records {
        let subject = match record {
            Record::Unit(core) => Subject::Unit(smysl_core::canonical_uid(core)),
            Record::Manifest(m) => Subject::Manifest(m.mid()),
            _ => continue,
        };
        touched = true;
        let seeded = crate::time::engine::seed_of(lib.store(), &subject, axis);
        rows.retain(|r| r.subject != subject);
        rows.push(Row {
            lo: seeded.lo,
            hi: seeded.hi,
            subject,
        });
    }
    if touched {
        *table = Ranges::of(rows, false);
    }
}

/// Whether a record can move a subject's effective time other than by seeding its own.
///
/// Everything that is not a unit, a manifest or a text object: a dating sets and tightens, a
/// withdrawal can make a dating not live, a relation can carry an Allen constraint, an
/// attestation decides a dating's stratum, and a retraction removes a basis. Written as a
/// deny-list of the five that are safe rather than an allow-list of the rest, because a record
/// type added later is then treated as bearing on time until somebody shows it does not — which
/// is the direction that fails safe.
fn bears_on_time(record: &Record) -> bool {
    !matches!(
        record,
        Record::Unit(_)
            | Record::Manifest(_)
            | Record::PartText(_)
            | Record::PartReading(_)
            | Record::Redaction(_)
    )
}

// ---------------------------------------------------------------------------
// observed: a field, not an axis
// ---------------------------------------------------------------------------

/// Every unit whose source recorded an instant, one millisecond wide.
pub fn observed_rebuild(lib: &crate::library::Library) -> Ranges {
    Ranges::of(
        lib.store()
            .units()
            .filter_map(|(uid, unit)| observed_row(Subject::Unit(*uid), unit.core.source.as_ref())),
        false,
    )
}

/// The same, over an appended batch. Genuinely local: nothing propagates into `observed`.
pub fn observed_on_append(table: &mut Ranges, records: &[Record]) {
    let mut rows: Vec<Row> = table.rows().to_vec();
    let mut touched = false;
    for record in records {
        let Record::Unit(core) = record else { continue };
        let subject = Subject::Unit(smysl_core::canonical_uid(core));
        if let Some(row) = observed_row(subject, core.source.as_ref()) {
            rows.retain(|r| r.subject != subject);
            rows.push(row);
            touched = true;
        }
    }
    if touched {
        *table = Ranges::of(rows, false);
    }
}

fn observed_row(subject: Subject, source: Option<&smysl_core::types::SourceRef>) -> Option<Row> {
    // `i64`, because the table is in milliseconds since the epoch and `observed` is a `u64`
    // clock. A value that does not fit is no row rather than a clamped one: a clamp would put a
    // unit in a window it is not in.
    let ms = i64::try_from(source?.observed?).ok()?;
    let at = Interval::at(Instant(ms));
    Some(Row {
        lo: at.lo,
        hi: at.hi,
        subject,
    })
}

/// A brute-force answer, for the tests that check the fast one.
///
/// Public because the assertion that matters is *agreement*: a structure that answers a range
/// query quickly and wrongly is worse than one that answers it slowly. Kept here rather than in
/// the test file so that both the `smysl-text` suite and any later caller compare against the
/// same definition of the answer.
pub fn scan(rows: &[Row], window: Interval) -> Vec<Subject> {
    let mut out: Vec<Subject> = rows
        .iter()
        .filter(|r| r.overlaps(window))
        .map(|r| r.subject)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The three tables an index holds, by their name in §3.8.
pub fn named(index: &super::Index) -> BTreeMap<&'static str, &Ranges> {
    [
        ("said", &index.said),
        ("composed", &index.composed),
        ("observed", &index.observed),
    ]
    .into_iter()
    .collect()
}
