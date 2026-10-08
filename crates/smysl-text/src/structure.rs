//! The structure of a part: the segment table read as a tree.
//!
//! The rows of a reading are nodes in document order, each with a half-open byte range. The
//! tree is derived from those ranges by containment rather than stored beside them, so there
//! is exactly one statement of the structure and it is the one the structure hash covers
//! (A-5). Reading the tree out of the rows is cheap — one pass with a stack — and it means a
//! reading that round-trips through any implementation produces the same tree, including the
//! implementations that keep the table opaque.
//!
//! # What a defect in the table is
//!
//! Siblings are disjoint and ordered; a child is inside its parent. Those are the only rules,
//! and they are checked rather than assumed, because the thing they rule out is the one that
//! would be found much later: two nodes that *partially* overlap. A reader with an off-by-one
//! in a verse boundary produces exactly that, every span attached to either node is then
//! quietly wrong, and nothing downstream has the information to notice. [`Structure::build`]
//! is where that is caught, once, at the reader's own output.

use std::collections::BTreeMap;
use std::ops::Range;

use crate::limits::Budget;
use crate::locator::Locator;
use crate::reading::{Level, Segment};
use smysl_core::error::LibError;

/// A node: one row of the table, placed in the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Node {
    /// The row this node came from, which is also its index in the arena.
    pub row: u32,
    pub level: Level,
    pub range: Range<u64>,
    pub locator: Locator,
    /// This node's children, as a range into the child index (see [`Structure::children`]).
    pub children: Range<u32>,
    pub parent: Option<u32>,
}

impl Node {
    pub fn len(&self) -> u64 {
        self.range.end.saturating_sub(self.range.start)
    }

    pub fn is_empty(&self) -> bool {
        self.range.end <= self.range.start
    }
}

/// Why a segment table is not a structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureError {
    /// The row the defect was found at.
    pub row: usize,
    pub defect: Defect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Defect {
    /// `end` before `start`.
    Backwards,
    /// A row that starts before the previous row did: the table is not in document order.
    OutOfOrder,
    /// A row that overlaps an earlier one without being inside it. The defect this type
    /// exists for.
    PartialOverlap,
    /// A row reaching past the end of the part.
    PastTheEnd,
    /// Two rows with the same locator: one name for two places.
    DuplicateLocator,
}

impl Defect {
    pub const fn as_str(self) -> &'static str {
        match self {
            Defect::Backwards => "end before start",
            Defect::OutOfOrder => "row starts before the previous row",
            Defect::PartialOverlap => "row overlaps an earlier row without nesting in it",
            Defect::PastTheEnd => "row reaches past the end of the part",
            Defect::DuplicateLocator => "two rows with the same locator",
        }
    }
}

impl std::fmt::Display for StructureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "row {}: {}", self.row, self.defect.as_str())
    }
}

impl std::error::Error for StructureError {}

/// A part's structure: the arena, the child index, and a locator index over both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Structure {
    nodes: Vec<Node>,
    child_index: Vec<u32>,
    roots: Vec<u32>,
    by_locator: BTreeMap<Locator, u32>,
    length: u64,
}

impl Structure {
    /// Build the tree from a reading's rows, charging a budget as it goes.
    ///
    /// `length` is the part's length in bytes, which is what makes `PastTheEnd` checkable;
    /// pass `u64::MAX` when the part is not to hand (a reading decoded on its own, for
    /// instance) and the other four checks still apply.
    pub fn build(
        rows: &[Segment],
        length: u64,
        budget: &mut Budget,
    ) -> Result<Structure, BuildError> {
        let mut nodes: Vec<Node> = Vec::with_capacity(rows.len());
        let mut kids: Vec<Vec<u32>> = Vec::with_capacity(rows.len());
        let mut roots = Vec::new();
        let mut by_locator = BTreeMap::new();
        // The open ancestors, innermost last.
        let mut stack: Vec<u32> = Vec::new();
        let mut prev_start = 0u64;

        for (i, row) in rows.iter().enumerate() {
            budget.node().map_err(BuildError::Limit)?;
            let fail = |defect| BuildError::Defect(StructureError { row: i, defect });
            if row.end < row.start {
                return Err(fail(Defect::Backwards));
            }
            if row.end > length {
                return Err(fail(Defect::PastTheEnd));
            }
            if row.start < prev_start {
                return Err(fail(Defect::OutOfOrder));
            }
            prev_start = row.start;

            // Close every ancestor this row is not inside of. A row that starts inside an
            // ancestor but ends outside it is the partial overlap.
            while let Some(&top) = stack.last() {
                let t = &nodes[top as usize].range;
                if row.start >= t.end {
                    stack.pop();
                    continue;
                }
                if row.end > t.end {
                    return Err(fail(Defect::PartialOverlap));
                }
                break;
            }

            let idx = nodes.len() as u32;
            let parent = stack.last().copied();
            if by_locator.insert(row.locator.clone(), idx).is_some() {
                return Err(fail(Defect::DuplicateLocator));
            }
            nodes.push(Node {
                row: idx,
                level: row.level.clone(),
                range: row.range(),
                locator: row.locator.clone(),
                // Filled once every child is known: a range into the child index cannot be
                // written before the children exist.
                children: 0..0,
                parent,
            });
            kids.push(Vec::new());
            match parent {
                Some(p) => kids[p as usize].push(idx),
                None => roots.push(idx),
            }
            // Every node is open until a later row starts at or after its end. A leaf is
            // popped by the next sibling rather than closed here, because nothing in a row
            // says whether it has children — the ranges do, and they are not known yet.
            stack.push(idx);
        }

        // Flatten the per-node child lists into one index, in node order, so a node's
        // children are a contiguous range and the whole structure is three vectors.
        let mut child_index = Vec::new();
        for (i, list) in kids.iter().enumerate() {
            let start = child_index.len() as u32;
            child_index.extend_from_slice(list);
            nodes[i].children = start..child_index.len() as u32;
        }

        Ok(Structure {
            nodes,
            child_index,
            roots,
            by_locator,
            length,
        })
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The part's length in bytes, as `build` was told it.
    pub fn length(&self) -> u64 {
        self.length
    }

    pub fn node(&self, index: u32) -> Option<&Node> {
        self.nodes.get(index as usize)
    }

    /// The top-level nodes, in document order. More than one is normal: a text file read as
    /// lines has no root.
    pub fn roots(&self) -> &[u32] {
        &self.roots
    }

    pub fn children(&self, node: &Node) -> &[u32] {
        &self.child_index[node.children.start as usize..node.children.end as usize]
    }

    /// The nodes at one level, in document order.
    pub fn at_level<'a>(&'a self, level: &'a Level) -> impl Iterator<Item = &'a Node> + 'a {
        self.nodes.iter().filter(move |n| &n.level == level)
    }

    /// The byte range a locator names, exactly.
    ///
    /// A range locator resolves to the hull of its two ends — both must be present, because a
    /// hull with one end missing is a guess, and a guess here is a span attached to the wrong
    /// text. Nothing is inferred: a locator the reader did not emit resolves to `None`, and
    /// [`Structure::resolve_containing`] is where a coarser answer can be asked for
    /// deliberately.
    pub fn resolve(&self, loc: &Locator) -> Option<Range<u64>> {
        // The exact table first, whatever shape the locator has. A range is not only a way of
        // asking for the hull of two nodes — it is also the *address of a node*, because a
        // verse bridge (`\v 1-2`, or an `osisID` naming two verses) is one span of text whose
        // own locator is a range. Decomposing first meant a bridge could not be resolved by
        // the address it was emitted under unless both of its ends happened to be separate
        // nodes, which in a bridge they never are. Found by fuzzing `osis/1`, which emitted
        // `Gen.0.3-s` and could not then resolve it.
        if let Some(i) = self.by_locator.get(loc) {
            return Some(self.nodes[*i as usize].range.clone());
        }
        match loc {
            Locator::Range(a, b) => {
                let lo = self.resolve(a)?;
                let hi = self.resolve(b)?;
                Some(lo.start..hi.end)
            }
            _ => None,
        }
    }

    /// The narrowest node whose locator contains this one.
    ///
    /// For a citation finer than the segmentation: `Gen.1.1` in a reading segmented by
    /// chapter resolves to the chapter. Separate from [`Structure::resolve`] so that a caller
    /// has to decide it wants a coarser answer; a single function that silently widened would
    /// make every unresolvable citation look resolvable.
    pub fn resolve_containing(&self, loc: &Locator) -> Option<(u32, Range<u64>)> {
        let mut best: Option<(u32, Range<u64>)> = None;
        for (i, n) in self.nodes.iter().enumerate() {
            if n.locator.contains(loc) {
                let better = match &best {
                    Some((_, r)) => n.len() < r.end.saturating_sub(r.start),
                    None => true,
                };
                if better {
                    best = Some((i as u32, n.range.clone()));
                }
            }
        }
        best
    }

    /// The deepest node containing a byte offset.
    pub fn node_at(&self, offset: u64) -> Option<&Node> {
        let mut best: Option<&Node> = None;
        for n in &self.nodes {
            if n.range.start <= offset && offset < n.range.end {
                let better = match best {
                    Some(b) => n.len() <= b.len(),
                    None => true,
                };
                if better {
                    best = Some(n);
                }
            }
        }
        best
    }
}

/// What `build` can refuse with: a defect in the table, or a resource cap.
///
/// Two kinds in one type because they are two kinds: a defect is a statement about the
/// reader's output, a cap is a statement about this run. `SMY-E440` carries the second and
/// nothing carries the first, because a malformed table never becomes a corpus — it is
/// refused at the reader, and `check` is never asked about it later.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BuildError {
    Defect(StructureError),
    Limit(LibError),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::Defect(e) => write!(f, "{e}"),
            BuildError::Limit(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for BuildError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::Caps;
    use crate::locator;
    use crate::reading::encode_table;

    fn level(s: &str) -> Level {
        Level::new(s).unwrap()
    }

    fn loc(s: &str) -> Locator {
        locator::parse(s).unwrap()
    }

    fn seg(start: u64, end: u64, lvl: &str, l: &str) -> Segment {
        Segment::new(start, end, level(lvl), loc(l))
    }

    fn budget() -> Budget {
        Budget::new(Caps::DEFAULT, 1_000_000).unwrap()
    }

    /// A chapter with three verses, as `osis/1` would emit it.
    fn bible() -> Vec<Segment> {
        vec![
            seg(0, 90, "chapter", "Gen.1"),
            seg(0, 30, "verse", "Gen.1.1"),
            seg(30, 60, "verse", "Gen.1.2"),
            seg(60, 90, "verse", "Gen.1.3"),
        ]
    }

    /// A node whose own address is a range resolves to itself, not to its ends.
    ///
    /// A verse bridge is one span of text addressed `Gen.1.1-Gen.1.2`, and neither end is a
    /// node of its own — so a resolver that always decomposed a range could never find it.
    /// Found by fuzzing a reader, fixed here, because the defect is the resolver's.
    #[test]
    fn a_range_that_is_itself_a_node_resolves_to_that_node() {
        let rows = vec![
            Segment::new(
                0,
                20,
                Level::new("chapter").expect("level"),
                "Exod.20".parse().expect("locator"),
            ),
            Segment::new(
                0,
                20,
                Level::new("verse").expect("level"),
                "Exod.20.1-Exod.20.2".parse().expect("locator"),
            ),
        ];
        let mut budget = Budget::new(Caps::DEFAULT, 1 << 10).expect("budget");
        let structure = Structure::build(&rows, 20, &mut budget).expect("a valid table");
        let bridge: Locator = "Exod.20.1-Exod.20.2".parse().expect("locator");
        assert_eq!(structure.resolve(&bridge), Some(0..20));
        // Neither end is a node, so asking for one of them alone still finds nothing.
        let one: Locator = "Exod.20.1".parse().expect("locator");
        assert_eq!(structure.resolve(&one), None);
    }

    /// A range nobody emitted is still the hull of its two ends.
    #[test]
    fn a_range_of_two_nodes_is_still_their_hull() {
        let rows = vec![
            Segment::new(
                0,
                10,
                Level::new("verse").expect("level"),
                "Gen.1.1".parse().expect("locator"),
            ),
            Segment::new(
                11,
                20,
                Level::new("verse").expect("level"),
                "Gen.1.2".parse().expect("locator"),
            ),
        ];
        let mut budget = Budget::new(Caps::DEFAULT, 1 << 10).expect("budget");
        let structure = Structure::build(&rows, 20, &mut budget).expect("a valid table");
        let span: Locator = "Gen.1.1-Gen.1.2".parse().expect("locator");
        assert_eq!(structure.resolve(&span), Some(0..20));
    }

    #[test]
    fn nesting_comes_from_the_ranges() {
        let s = Structure::build(&bible(), 90, &mut budget()).unwrap();
        assert_eq!(s.len(), 4);
        assert_eq!(s.roots(), &[0]);
        let chapter = s.node(0).unwrap();
        assert_eq!(s.children(chapter), &[1, 2, 3]);
        for i in 1..4 {
            let v = s.node(i).unwrap();
            assert_eq!(v.parent, Some(0));
            assert!(s.children(v).is_empty());
        }
    }

    #[test]
    fn a_flat_reading_has_many_roots_and_no_children() {
        let rows: Vec<Segment> = (0..5)
            .map(|i| seg(i * 10, i * 10 + 10, "line", &format!("L{}", i + 1)))
            .collect();
        let s = Structure::build(&rows, 50, &mut budget()).unwrap();
        assert_eq!(s.roots().len(), 5);
        assert!(s.nodes().iter().all(|n| n.parent.is_none()));
    }

    #[test]
    fn resolve_returns_the_node_the_reader_emitted() {
        let rows = bible();
        let s = Structure::build(&rows, 90, &mut budget()).unwrap();
        for r in &rows {
            assert_eq!(
                s.resolve(&r.locator),
                Some(r.range()),
                "{} should resolve to its own range",
                r.locator
            );
        }
        assert_eq!(s.resolve(&loc("Gen.1.4")), None, "nothing is invented");
    }

    #[test]
    fn a_range_locator_resolves_to_the_hull_and_only_with_both_ends() {
        let s = Structure::build(&bible(), 90, &mut budget()).unwrap();
        assert_eq!(s.resolve(&loc("Gen.1.1-Gen.1.3")), Some(0..90));
        assert_eq!(s.resolve(&loc("Gen.1.2-Gen.1.3")), Some(30..90));
        assert_eq!(s.resolve(&loc("Gen.1.2-Gen.1.9")), None);
    }

    #[test]
    fn a_citation_finer_than_the_segmentation_resolves_only_when_asked_to_widen() {
        let chapter_only = vec![seg(0, 90, "chapter", "Gen.1")];
        let s = Structure::build(&chapter_only, 90, &mut budget()).unwrap();
        assert_eq!(s.resolve(&loc("Gen.1.1")), None);
        assert_eq!(s.resolve_containing(&loc("Gen.1.1")), Some((0, 0..90)));
    }

    #[test]
    fn resolve_containing_takes_the_narrowest_node() {
        let s = Structure::build(&bible(), 90, &mut budget()).unwrap();
        assert_eq!(s.resolve_containing(&loc("Gen.1.2")), Some((2, 30..60)));
    }

    #[test]
    fn node_at_takes_the_deepest_node_over_a_byte() {
        let s = Structure::build(&bible(), 90, &mut budget()).unwrap();
        assert_eq!(s.node_at(0).unwrap().locator, loc("Gen.1.1"));
        assert_eq!(s.node_at(89).unwrap().locator, loc("Gen.1.3"));
        assert!(s.node_at(90).is_none(), "the range is half-open");
    }

    /// The defect this module exists for: a verse boundary off by one, which makes every span
    /// on either node quietly wrong and which nothing downstream could notice.
    #[test]
    fn a_partial_overlap_is_refused_at_the_row_that_causes_it() {
        let rows = vec![
            seg(0, 50, "verse", "Gen.1.1"),
            seg(40, 90, "verse", "Gen.1.2"),
        ];
        let e = Structure::build(&rows, 90, &mut budget()).unwrap_err();
        assert_eq!(
            e,
            BuildError::Defect(StructureError {
                row: 1,
                defect: Defect::PartialOverlap
            })
        );
    }

    #[test]
    fn the_other_four_defects_are_refused_too() {
        let cases: Vec<(Vec<Segment>, u64, usize, Defect)> = vec![
            (vec![seg(10, 5, "line", "L1")], 90, 0, Defect::Backwards),
            (
                vec![seg(10, 20, "line", "L1"), seg(5, 8, "line", "L2")],
                90,
                1,
                Defect::OutOfOrder,
            ),
            (vec![seg(0, 91, "line", "L1")], 90, 0, Defect::PastTheEnd),
            (
                vec![seg(0, 10, "line", "L1"), seg(10, 20, "line", "L1")],
                90,
                1,
                Defect::DuplicateLocator,
            ),
        ];
        for (rows, length, row, defect) in cases {
            let e = Structure::build(&rows, length, &mut budget()).unwrap_err();
            assert_eq!(
                e,
                BuildError::Defect(StructureError { row, defect }),
                "{defect:?}"
            );
        }
    }

    #[test]
    fn an_empty_node_is_allowed_because_an_empty_verse_is() {
        let rows = vec![
            seg(0, 10, "verse", "Gen.1.1"),
            seg(10, 10, "verse", "Gen.1.2"),
            seg(10, 20, "verse", "Gen.1.3"),
        ];
        let s = Structure::build(&rows, 20, &mut budget()).unwrap();
        assert!(s.node(1).unwrap().is_empty());
        assert_eq!(s.resolve(&loc("Gen.1.2")), Some(10..10));
    }

    #[test]
    fn building_charges_one_node_each_and_refuses_over_the_cap() {
        let mut b = budget();
        Structure::build(&bible(), 90, &mut b).unwrap();
        assert_eq!(b.nodes(), 4);

        let mut tight = Budget::new(
            Caps {
                nodes: 3,
                ..Caps::DEFAULT
            },
            1_000,
        )
        .unwrap();
        let e = Structure::build(&bible(), 90, &mut tight).unwrap_err();
        assert!(matches!(e, BuildError::Limit(_)), "{e}");
        if let BuildError::Limit(e) = e {
            assert_eq!(e.code(), Some(smysl_core::Code::E440));
        }
    }

    /// The tree is derived, so re-reading the same table gives the same tree and the same
    /// hash — which is what `SMY-E401` is comparing when a reader is upgraded under a corpus.
    #[test]
    fn the_structure_is_stable_under_re_reading() {
        let rows = bible();
        let first = Structure::build(&rows, 90, &mut budget()).unwrap();
        let bytes = encode_table(&rows);
        let decoded = crate::reading::decode_table(&bytes).unwrap();
        let second = Structure::build(&decoded, 90, &mut budget()).unwrap();
        assert_eq!(first, second);
        assert_eq!(encode_table(&decoded), bytes);
    }

    #[test]
    fn at_level_walks_one_level_in_document_order() {
        let s = Structure::build(&bible(), 90, &mut budget()).unwrap();
        let verses: Vec<String> = s
            .at_level(&level("verse"))
            .map(|n| n.locator.to_string())
            .collect();
        assert_eq!(verses, ["Gen.1.1", "Gen.1.2", "Gen.1.3"]);
        assert_eq!(s.at_level(&level("book")).count(), 0);
    }

    #[test]
    fn an_empty_table_is_an_empty_structure() {
        let s = Structure::build(&[], 0, &mut budget()).unwrap();
        assert!(s.is_empty());
        assert!(s.roots().is_empty());
        assert_eq!(s.resolve(&loc("L1")), None);
    }
}
