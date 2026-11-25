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

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::types::library::Axis;
use smysl_core::types::Record;

use smysl_core::Uid;
use smysl_graph::{Store, StoreRead};

use crate::time::engine::{findings, findings_among, seed_of, Effective};
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

/// Rows per block, at least: a block holds `B / 2` to `2 * B` rows (RFC SMYSL-9 §5.2). An append
/// moves O(B) rows within a block and O(rows / B) blocks, instead of copying every row.
const B: usize = 512;

/// A run of rows in `lo` order, with the max-`hi` tree over them.
#[derive(Debug, Clone, Default)]
struct Block {
    rows: Vec<Row>,
    /// `max(hi)` per node of a segment tree over `rows`; node 1 is the block's own `max_hi`.
    tree: Vec<Bound>,
}

impl Block {
    fn new(rows: Vec<Row>) -> Block {
        let tree = build_tree(&rows);
        Block { rows, tree }
    }

    fn retree(&mut self) {
        self.tree = build_tree(&self.rows);
    }

    fn max_hi(&self) -> Bound {
        self.tree[1]
    }
}

/// A range table: rows in `lo` order, in blocks, each with a max-`hi` tree over its rows.
///
/// The blocks are a layout, not content: iterated in order they are the one sorted row vector
/// the table always was, which is what [`Ranges::rows`] gives, what the dump holds and what §3.8's
/// byte comparison compares. The trees are derived from the rows and are not persisted.
#[derive(Debug, Clone, Default)]
pub struct Ranges {
    /// Non-empty blocks, ordered by their first row, holding every row once.
    blocks: Vec<Block>,
    len: usize,
    /// The rows flattened, built when [`Ranges::rows`] asks and dropped by the next change.
    flat: std::sync::OnceLock<Vec<Row>>,
    /// Each subject's row, for removing it without a scan. Built by the first splice and kept
    /// with the table; never written down. `None` until then, and for good when a subject has
    /// more than one row, which no builder in this crate makes: such a table splices whole.
    rows_of: Option<BTreeMap<Subject, Row>>,
    /// Set when a subject was found with two rows; splices then rebuild the table whole.
    many: bool,
    /// Whether the corpus held an unresolved dating or an unbuilt constraint when this was built.
    ///
    /// While it does, an arriving unit can resolve one, so the incremental builder has no sound
    /// local path and recomputes. See the module note.
    danglers: bool,
    /// The subjects the last solve found anything bearing on, when this process solved for the
    /// table: every other row is its subject's seed. What lets an append that bears on time
    /// re-solve and rewrite only those rows (see `axis_on_append`). Never written down; a table
    /// read from the sidecar has none, and its first such append solves the whole axis.
    involved: Option<std::sync::Arc<BTreeSet<Subject>>>,
    /// The `replies` and `quotes` relations under each end, withdrawn ones included: what ties
    /// two units into one of rule E's components in a library with no datings or manifests, so
    /// that an append re-solves only the components it touches (AC-8). Built from the store the
    /// first time it is needed, and carried; never written down.
    links: Option<std::sync::Arc<Links>>,
}

/// Rule E's ties, under each end.
type Links = BTreeMap<Uid, Vec<smysl_core::Relation>>;

fn is_link(kind: &smysl_core::RelKind) -> bool {
    use smysl_core::schemas::text::{QUOTES, REPLIES};
    matches!(kind, smysl_core::RelKind::Extension(k) if k == REPLIES || k == QUOTES)
}

fn link(links: &mut Links, r: &smysl_core::Relation) {
    if is_link(&r.kind) {
        links.entry(r.from).or_default().push(r.clone());
        if r.to != r.from {
            links.entry(r.to).or_default().push(r.clone());
        }
    }
}

/// Equal by what a table answers: its rows and whether anything dangled. The blocks and trees
/// are a layout of the rows, and what a solve remembers is how the table was built, not what it
/// holds.
impl PartialEq for Ranges {
    fn eq(&self, other: &Ranges) -> bool {
        self.len == other.len && self.danglers == other.danglers && self.iter().eq(other.iter())
    }
}

impl Eq for Ranges {}

impl Ranges {
    /// The rows, in `lo` order. What the dump writes.
    ///
    /// Flattened from the blocks on the first call after a change, so it is O(rows) then and
    /// free after; the commit path never calls it.
    pub fn rows(&self) -> &[Row] {
        self.flat.get_or_init(|| self.iter().copied().collect())
    }

    /// The rows in `lo` order, without flattening them.
    fn iter(&self) -> impl Iterator<Item = &Row> {
        self.blocks.iter().flat_map(|b| b.rows.iter())
    }

    /// Whether this table was built over a corpus with something dangling in it.
    pub fn danglers(&self) -> bool {
        self.danglers
    }

    /// How many subjects the table holds.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
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
        Ranges::sorted(rows, danglers)
    }

    /// A table over rows already sorted and deduped, cut into blocks of `B`.
    fn sorted(rows: Vec<Row>, danglers: bool) -> Ranges {
        let len = rows.len();
        let blocks = rows.chunks(B).map(|c| Block::new(c.to_vec())).collect();
        Ranges {
            blocks,
            len,
            flat: std::sync::OnceLock::new(),
            rows_of: None,
            many: false,
            danglers,
            involved: None,
            links: None,
        }
    }

    /// Replace the rows of `drop`'s subjects with `add`, in place.
    ///
    /// Each subject's row is found through `rows_of`, removed from its block, and each new row
    /// inserted where it sorts: O(B + blocks) a row, not O(rows) (RFC SMYSL-9 §5.2). The rows that
    /// result are the ones a merge of the kept rows and the new would give, which is what §3.8's
    /// comparison between this and a rebuild checks.
    fn splice(&mut self, drop: &BTreeSet<Subject>, add: Vec<Row>, danglers: bool) {
        let mut add: Vec<Row> = add
            .into_iter()
            .filter(|r| !(r.lo == Bound::Open && r.hi == Bound::Open))
            .collect();
        add.sort_unstable();
        add.dedup();
        self.danglers = danglers;
        self.flat = std::sync::OnceLock::new();
        if !self.many && self.rows_of.is_none() {
            let mut index = BTreeMap::new();
            let many = !self
                .blocks
                .iter()
                .flat_map(|b| b.rows.iter())
                .all(|r| index.insert(r.subject, *r).is_none());
            self.many = many;
            if !many {
                self.rows_of = Some(index);
            }
        }
        // A second row for one subject — two in `add`, or one beside a row `drop` keeps — is a
        // table the subject index cannot hold: spliced whole instead, as every splice was.
        let mut seen = BTreeSet::new();
        let second = |r: &Row, index: &BTreeMap<Subject, Row>| {
            !drop.contains(&r.subject) && index.contains_key(&r.subject)
        };
        let one_each = match &self.rows_of {
            Some(index) => add
                .iter()
                .all(|r| seen.insert(r.subject) && !second(r, index)),
            None => false,
        };
        if !one_each {
            self.splice_whole(drop, add);
            return;
        }
        let mut index = self.rows_of.take().expect("built above");
        for subject in drop {
            if let Some(row) = index.remove(subject) {
                self.remove(&row);
            }
        }
        for row in add {
            index.insert(row.subject, row);
            self.insert(row);
        }
        self.rows_of = Some(index);
    }

    /// [`Ranges::splice`] by one pass over every row: for a table where a subject has two rows,
    /// which the subject index cannot hold.
    fn splice_whole(&mut self, drop: &BTreeSet<Subject>, add: Vec<Row>) {
        let kept = self
            .blocks
            .iter()
            .flat_map(|b| b.rows.iter())
            .filter(|r| !drop.contains(&r.subject));
        let mut rows = Vec::with_capacity(self.len + add.len());
        let (mut a, mut b) = (kept.peekable(), add.into_iter().peekable());
        loop {
            let from_kept = match (a.peek(), b.peek()) {
                (Some(x), Some(y)) => *x <= y,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            let next = if from_kept {
                *a.next().expect("peeked")
            } else {
                b.next().expect("peeked")
            };
            if rows.last() != Some(&next) {
                rows.push(next);
            }
        }
        let mut subjects = BTreeSet::new();
        self.many = !rows.iter().all(|r| subjects.insert(r.subject));
        let rebuilt = Ranges::sorted(rows, self.danglers);
        self.blocks = rebuilt.blocks;
        self.len = rebuilt.len;
        self.rows_of = None;
    }

    /// The block `row` belongs in: the last whose first row is not after it, or the first.
    fn block_for(&self, row: &Row) -> usize {
        self.blocks
            .partition_point(|b| b.rows[0] <= *row)
            .saturating_sub(1)
    }

    fn insert(&mut self, row: Row) {
        if self.blocks.is_empty() {
            self.blocks.push(Block::new(vec![row]));
            self.len = 1;
            return;
        }
        let i = self.block_for(&row);
        let block = &mut self.blocks[i];
        let Err(at) = block.rows.binary_search(&row) else {
            return;
        };
        block.rows.insert(at, row);
        self.len += 1;
        if block.rows.len() > 2 * B {
            let upper = block.rows.split_off(block.rows.len() / 2);
            block.retree();
            self.blocks.insert(i + 1, Block::new(upper));
        } else {
            block.retree();
        }
    }

    fn remove(&mut self, row: &Row) {
        if self.blocks.is_empty() {
            return;
        }
        let i = self.block_for(row);
        let count = self.blocks.len();
        let block = &mut self.blocks[i];
        let Ok(at) = block.rows.binary_search(row) else {
            return;
        };
        block.rows.remove(at);
        self.len -= 1;
        if block.rows.is_empty() {
            self.blocks.remove(i);
            return;
        }
        if block.rows.len() < B / 2 && count > 1 {
            // Merge with a neighbour, the next one if there is one, and split again if that made
            // a block over `2 * B`.
            let (left, right) = if i + 1 < self.blocks.len() {
                (i, i + 1)
            } else {
                (i - 1, i)
            };
            let moved = self.blocks.remove(right).rows;
            let merged = &mut self.blocks[left];
            merged.rows.extend(moved);
            if merged.rows.len() > 2 * B {
                let upper = merged.rows.split_off(merged.rows.len() / 2);
                merged.retree();
                self.blocks.insert(left + 1, Block::new(upper));
            } else {
                merged.retree();
            }
            return;
        }
        block.retree();
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
        // The blocks holding a row with `lo < window.hi`: a prefix, since each block's first row
        // is its least. Blocks are skipped on their first row and their `max_hi`, which reads no
        // row as this counts them.
        let p = self
            .blocks
            .partition_point(|b| before(b.rows[0].lo, window.hi));
        for block in &self.blocks[..p] {
            if !before(window.lo, block.max_hi()) {
                continue;
            }
            // Inside the block, every row with `lo < window.hi` is a prefix, and the tree lets a
            // subtree whose every `hi` is at or below `window.lo` be skipped whole, which is what
            // keeps a narrow window from reading a wide prefix.
            let limit = block.rows.partition_point(|r| before(r.lo, window.hi));
            descend(
                block,
                1,
                0,
                block.rows.len(),
                limit,
                window,
                &mut out,
                &mut visited,
            );
        }
        (out, visited)
    }
}

/// One node of the descent within a block: `[lo, hi)` is the node's span, `limit` the prefix end.
#[allow(clippy::too_many_arguments)]
fn descend(
    block: &Block,
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
    if !before(window.lo, block.tree[node]) {
        return;
    }
    if hi - lo == 1 {
        *visited += 1;
        let row = block.rows[lo];
        if row.overlaps(window) {
            out.push(row.subject);
        }
        return;
    }
    let mid = lo + (hi - lo) / 2;
    descend(block, node * 2, lo, mid, limit, window, out, visited);
    descend(block, node * 2 + 1, mid, hi, limit, window, out, visited);
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
///
/// [`findings`] for the subjects anything bears on, and every other subject's seed: what
/// [`crate::time::engine::effective`] answers, without a map of every subject beside the rows.
pub fn axis_rebuild(lib: &crate::library::Library, axis: Axis) -> Ranges {
    let store = lib.store();
    let found = findings(store, axis);
    let everything = store.units_in_range(&Uid::from_bytes([0; 32]), &Uid::from_bytes([0xff; 32]));
    let subjects = everything.iter().map(|u| Subject::Unit(*u)).chain(
        store
            .library()
            .manifests()
            .map(|(m, _)| Subject::Manifest(*m)),
    );
    let rows: Vec<Row> = subjects
        .map(|subject| row_of(store, &found, subject, axis))
        .collect();
    let mut table = Ranges::of(rows, !found.unresolved.is_empty());
    table.involved = Some(std::sync::Arc::new(found.dated.keys().copied().collect()));
    table
}

/// A subject's row: what the solve found for it, or its seed.
fn row_of(store: &Store, found: &Effective, subject: Subject, axis: Axis) -> Row {
    let interval = found
        .dated
        .get(&subject)
        .map(|d| d.interval)
        .unwrap_or_else(|| seed_of(store, &subject, axis));
    Row {
        lo: interval.lo,
        hi: interval.hi,
        subject,
    }
}

/// Bring an axis table forward over an appended batch.
///
/// Local when the batch can only seed new subjects and the corpus has nothing dangling.
/// Otherwise the axis is solved again — one dating can move any subject's interval, and only the
/// solve knows which — but only the rows that can have changed are rewritten: those of the
/// subjects this solve or the last one found anything bearing on, and the batch's own. Every
/// other row was its subject's seed before and is after. Rewriting every row instead was most
/// of a 10⁶-unit library's build, a solve and a row per unit for each of a thousand appends
/// (GE-T7's stand-in, TX-P13c's time-engine step). See the module note.
pub fn axis_on_append(
    table: &mut Ranges,
    lib: &crate::library::Library,
    axis: Axis,
    records: &[Record],
) {
    let batch: BTreeSet<Subject> = records
        .iter()
        .filter_map(|record| match record {
            Record::Unit(core) => Some(Subject::Unit(smysl_core::canonical_uid(core))),
            Record::Manifest(m) => Some(Subject::Manifest(m.mid())),
            _ => None,
        })
        .collect();
    let store = lib.store();
    if !table.danglers() && region_on_append(table, lib, axis, records) {
        return;
    }
    if table.danglers() || records.iter().any(bears_on_time) {
        let Some(before) = table.involved.clone() else {
            *table = axis_rebuild(lib, axis);
            return;
        };
        // Only the subjects the last solve involved, and the batch's, can have an empty seed.
        let maybe_empty: BTreeSet<Subject> = before.iter().chain(&batch).copied().collect();
        let found = findings_among(store, axis, &maybe_empty);
        let now: BTreeSet<Subject> = found.dated.keys().copied().collect();
        let touched: BTreeSet<Subject> = before.iter().chain(&now).chain(&batch).copied().collect();
        let add: Vec<Row> = touched
            .iter()
            .filter(|s| match s {
                Subject::Unit(u) => store.contains_uid(u),
                Subject::Manifest(m) => store.library().manifest(m).is_some(),
            })
            .map(|s| row_of(store, &found, *s, axis))
            .collect();
        table.splice(&touched, add, !found.unresolved.is_empty());
        table.involved = Some(std::sync::Arc::new(now));
        return;
    }
    if batch.is_empty() {
        return;
    }
    // Seeds alone: nothing bears on a subject that only arrived.
    let add: Vec<Row> = batch
        .iter()
        .map(|subject| {
            let seeded = crate::time::engine::seed_of(store, subject, axis);
            Row {
                lo: seeded.lo,
                hi: seeded.hi,
                subject: *subject,
            }
        })
        .collect();
    table.splice(&batch, add, false);
}

/// [`axis_on_append`] for a library with no datings and no manifests (AC-8, RFC SMYSL-3
/// §17.1): rule E solved over the components the batch touches, and only their rows rewritten.
///
/// In such a library nothing but a `replies` or `quotes` relation ties two units, and since
/// each component is solved on its own (`engine::solve_components`), a row outside the
/// components the batch touches is what it was. Those components are the closure, over the
/// ties, of the batch's units, the subjects of its attestations and the ends of its new ties.
/// `false`, having changed nothing, when the library has datings or manifests, or the batch
/// carries a record rule E reads other than a unit, a relation or an attestation (a schema
/// declaration, a View and a label binding it does not read): the general path then decides. The table forgets what the last full solve involved, which only
/// that path reads, so the next time it is taken it solves the whole axis.
fn region_on_append(
    table: &mut Ranges,
    lib: &crate::library::Library,
    axis: Axis,
    records: &[Record],
) -> bool {
    let store = lib.store();
    let catalog = store.library();
    if catalog.datings().next().is_some() || catalog.manifests().next().is_some() {
        return false;
    }
    if !records.iter().all(|r| {
        matches!(
            r,
            Record::Unit(_)
                | Record::Relation(_)
                | Record::Attestation(_)
                | Record::LabelBinding(_)
                | Record::SchemaDecl(_)
                | Record::View(_)
        )
    }) {
        return false;
    }
    // The store already holds the batch: the ties are read from it the first time, and only the
    // batch's are added after.
    let links = match table.links.take() {
        Some(l) => {
            let mut l = l;
            for r in records {
                if let Record::Relation(rel) = r {
                    link(std::sync::Arc::make_mut(&mut l), rel);
                }
            }
            l
        }
        None => {
            let mut l = Links::new();
            for r in store.relations() {
                link(&mut l, r);
            }
            std::sync::Arc::new(l)
        }
    };
    let mut todo: Vec<Uid> = Vec::new();
    for r in records {
        match r {
            Record::Unit(core) => todo.push(smysl_core::canonical_uid(core)),
            Record::Attestation(a) => todo.push(a.uid),
            Record::Relation(rel) if is_link(&rel.kind) => {
                todo.push(rel.from);
                todo.push(rel.to);
            }
            _ => {}
        }
    }
    let mut units: BTreeSet<Uid> = BTreeSet::new();
    let mut ties: BTreeMap<Uid, smysl_core::Relation> = BTreeMap::new();
    while let Some(u) = todo.pop() {
        if !units.insert(u) {
            continue;
        }
        for r in links.get(&u).into_iter().flatten() {
            ties.entry(r.uid()).or_insert_with(|| r.clone());
            for end in [r.from, r.to] {
                if !units.contains(&end) {
                    todo.push(end);
                }
            }
        }
    }
    let ties: Vec<smysl_core::Relation> = ties.into_values().collect();
    let Some(found) = crate::time::engine::findings_within(store, axis, &units, &ties) else {
        table.links = Some(links);
        return false;
    };
    let touched: BTreeSet<Subject> = units.iter().map(|u| Subject::Unit(*u)).collect();
    let add: Vec<Row> = touched
        .iter()
        .filter(|s| matches!(s, Subject::Unit(u) if store.contains_uid(u)))
        .map(|s| row_of(store, &found, *s, axis))
        .collect();
    table.splice(&touched, add, false);
    table.involved = None;
    table.links = Some(links);
    true
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
    // A unit appended twice in one batch keeps its last row, as it always did.
    let mut add: BTreeMap<Subject, Row> = BTreeMap::new();
    for record in records {
        let Record::Unit(core) = record else { continue };
        let subject = Subject::Unit(smysl_core::canonical_uid(core));
        if let Some(row) = observed_row(subject, core.source.as_ref()) {
            add.insert(subject, row);
        }
    }
    if !add.is_empty() {
        let drop: BTreeSet<Subject> = add.keys().copied().collect();
        table.splice(&drop, add.into_values().collect(), false);
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

#[cfg(test)]
mod tests {
    //! RFC SMYSL-9 RT-1: after any sequence of splices, the blocked table is the table built from
    //! scratch over the same rows, and answers every window as a scan does.

    use super::*;

    /// SplitMix64, so a failure reproduces from its case number.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn subject(i: u64) -> Subject {
        let mut bytes = [0u8; 32];
        bytes[..8].copy_from_slice(&i.to_be_bytes());
        Subject::Unit(Uid::from_bytes(bytes))
    }

    /// A row for subject `i`: mostly instants on a narrow clock, so `lo`s collide; some long,
    /// some open at one end, a few undated (which no table holds).
    fn row(rng: &mut Rng, i: u64) -> Row {
        let at = |rng: &mut Rng| Bound::At(Instant(rng.below(5_000) as i64));
        let (lo, hi) = match rng.below(20) {
            0 => (Bound::Open, at(rng)),
            1 => (at(rng), Bound::Open),
            2 => (Bound::Open, Bound::Open),
            _ => {
                let lo = rng.below(5_000) as i64;
                (
                    Bound::At(Instant(lo)),
                    Bound::At(Instant(lo + 1 + rng.below(300) as i64)),
                )
            }
        };
        Row::new(subject(i), lo, hi)
    }

    /// The reference: what `of` builds over the rows `drop` keeps and `add`.
    fn reference(before: &[Row], drop: &BTreeSet<Subject>, add: &[Row]) -> Ranges {
        Ranges::of(
            before
                .iter()
                .filter(|r| !drop.contains(&r.subject))
                .chain(add)
                .copied(),
            false,
        )
    }

    fn check_layout(t: &Ranges, case: u64) {
        assert_eq!(t.len, t.blocks.iter().map(|b| b.rows.len()).sum::<usize>());
        for (i, b) in t.blocks.iter().enumerate() {
            assert!(!b.rows.is_empty(), "case {case}: an empty block");
            assert!(
                b.rows.len() <= 2 * B,
                "case {case}: a block of {}",
                b.rows.len()
            );
            let max = b.rows.iter().map(|r| r.hi).fold(b.rows[0].hi, later);
            assert_eq!(b.max_hi(), max, "case {case}: block {i}'s max_hi");
        }
        assert!(
            t.iter().zip(t.iter().skip(1)).all(|(a, b)| a < b),
            "case {case}: order"
        );
    }

    #[test]
    fn splices_give_the_table_a_rebuild_gives() {
        for case in 0..25u64 {
            let mut rng = Rng(case);
            let start = 1 + rng.below(3_000);
            let mut table = Ranges::of((0..start).map(|i| row(&mut rng, i)), false);
            let mut next = start;
            for step in 0..60 {
                let before: Vec<Row> = table.rows().to_vec();
                // Drop some present subjects, some absent; re-add some of them with new rows;
                // add new subjects. Sometimes a large batch, to split and merge blocks.
                let big = rng.below(10) == 0;
                let n = if big { 1_500 } else { 1 + rng.below(12) };
                let mut drop = BTreeSet::new();
                let mut add = Vec::new();
                for _ in 0..n {
                    match rng.below(4) {
                        0 => {
                            drop.insert(subject(rng.below(next + 10)));
                        }
                        1 => {
                            let s = rng.below(next);
                            drop.insert(subject(s));
                            add.push(row(&mut rng, s));
                        }
                        _ => {
                            drop.insert(subject(next));
                            add.push(row(&mut rng, next));
                            next += 1;
                        }
                    }
                }
                // A batch may name a subject twice; the last row is the one it means.
                let mut last: BTreeMap<Subject, Row> = BTreeMap::new();
                for r in add {
                    last.insert(r.subject, r);
                }
                let add: Vec<Row> = last.into_values().collect();
                let want = reference(&before, &drop, &add);
                table.splice(&drop, add, false);
                assert_eq!(table, want, "case {case}, step {step}");
                assert_eq!(table.rows(), want.rows(), "case {case}, step {step}");
                check_layout(&table, case);
                // And it answers as a scan does.
                for _ in 0..4 {
                    let lo = rng.below(5_200) as i64 - 100;
                    let window = Interval::of(lo, lo + 1 + rng.below(400) as i64);
                    let mut got = table.during(window);
                    got.sort_unstable();
                    assert_eq!(got, scan(table.rows(), window), "case {case}, step {step}");
                }
            }
            assert!(
                !table.many,
                "case {case}: one row a subject, so the index held"
            );
        }
    }

    /// A subject with two rows, which `of` allows, splices whole and stays correct.
    #[test]
    fn a_subject_with_two_rows_splices_whole() {
        let a = Row::new(subject(1), Bound::At(Instant(1)), Bound::At(Instant(2)));
        let b = Row::new(subject(1), Bound::At(Instant(5)), Bound::At(Instant(9)));
        let c = Row::new(subject(2), Bound::At(Instant(3)), Bound::At(Instant(4)));
        let mut t = Ranges::of([a, b, c], false);
        let d = Row::new(subject(3), Bound::At(Instant(0)), Bound::At(Instant(7)));
        t.splice(&BTreeSet::from([subject(3)]), vec![d], false);
        assert!(t.many);
        assert_eq!(t.rows(), &[d, a, c, b]);
        t.splice(&BTreeSet::from([subject(1)]), Vec::new(), false);
        assert_eq!(t.rows(), &[d, c]);
        assert!(!t.many, "one row a subject again");
    }
}
