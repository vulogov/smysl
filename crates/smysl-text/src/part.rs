//! Part policy: how a reader's nodes become parts.
//!
//! A reader emits structure; a part is a unit of *storage*. The two are different questions,
//! and keeping them apart is what lets one Bible be one manifest of sixty-six parts while a
//! three-year chat export is one manifest of a thousand, without either reader knowing
//! anything about object stores.
//!
//! # Whole nodes, always
//!
//! A part is a run of consecutive whole nodes at one level. Never half a node: a part is what
//! a tid names, spans are byte offsets into a part, and a node split across two parts would
//! have no span at all. So the size targets are targets — [`Policy::target_max`] is the size
//! at which grouping stops adding nodes, and a single node larger than it becomes one part on
//! its own, bounded only by the hard ceiling in [`crate::limits::Caps::part_bytes`].
//!
//! # The policy is recorded, not assumed
//!
//! Manifest key 17 holds [`Policy::id`], and it is a required key. The defaults were
//! provisional until GE-T14 measured them at the end of TX-P2 — and the measurement moved
//! `target_min` from 64 KiB to 1 KiB, which is exactly the event the key exists for: every
//! corpus cut under the old default stays valid under the policy it recorded, and `text append`
//! goes on cutting it that way. See [`Policy::DEFAULT_MIN`] for the curve and the argument.

use smysl_core::error::LibError;

use crate::limits::Budget;
use crate::norm::Normalised;
use crate::reading::Level;
use crate::reading::Segment;
use smysl_core::types::library::PartText;

/// The boundary rule and the size targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// The level whose nodes are grouped. Nodes at other levels ride along inside them.
    pub boundary_level: Level,
    /// Keep adding nodes until the part is at least this big.
    pub target_min: u64,
    /// Stop adding nodes once the part would exceed this.
    pub target_max: u64,
}

impl Policy {
    /// The version of the policy string this build writes and reads.
    ///
    /// Versioned like every other derived-artifact identifier in this format
    /// (`smysl/seg-uax29+abbr/1`, `smysl/utf8-div4`): the string is a claim about how the
    /// bytes were cut, and a claim that cannot say which rule made it is a claim nobody can
    /// check later.
    pub const ID: &'static str = "smysl/parts/1";

    /// 1 KiB to 4 MiB at the reader's top level — **fixed by GE-T14** (TX-P2 step 6).
    ///
    /// `Level::new` cannot be `const`, so the default level is a `&str` here and a `Level`
    /// in the [`Default`] impl. The level is `chapter` rather than `book`: a
    /// 4 MiB ceiling over book-sized nodes would put most Bibles in one part and make every
    /// re-read of a corrected verse rewrite the whole thing.
    ///
    /// # Why 1 KiB, and why it was 64
    ///
    /// `target_min` was 64 KiB and provisional, which is why manifest key 17 records the policy:
    /// a corpus cut under the old default stays valid under the policy it recorded.
    /// `crates/smysl-text/tests/part_size.rs` is the measurement that settled it, over a year of
    /// chat, a revised article and a 1,189-chapter Bible. What it found:
    ///
    /// - **Growth costs at most two parts at every size** — the one that was last, and a new one
    ///   where the last group did not absorb the new nodes. So the minimum buys nothing for the
    ///   case a chat is in every day.
    /// - **An in-place correction costs one part if its length is unchanged, and every part after
    ///   it if it is not** — also at every size. Smaller parts make the second case *worse* in
    ///   object count, not better.
    /// - So the only axis the size genuinely trades is **object count and catalog size against
    ///   redaction granularity**: 1 KiB gives a year of chat 365 parts, a 40 KB manifest and 7 MB
    ///   of catalog over 365 daily appends; 64 KiB gives 22 parts, 2.5 KB and 0.4 MB — and makes
    ///   one redaction remove **sixteen days** of conversation instead of one.
    ///
    /// The decision follows from what the two numbers are *about*. The boundary level is the
    /// structural judgement the reader's author already made — a day, a chapter, a paragraph —
    /// and a minimum above the typical node size silently overrides it: at 64 KiB a part is
    /// sixteen days whatever the reader said. So the minimum's job is only to stop
    /// pathologically small parts, and its value belongs **below** the node sizes that matter
    /// (a chat day and a Bible chapter both measure about 4 KB). 1 KiB is below them, so the
    /// level governs and a redaction removes about what it names; 4 KiB sits *at* them, where
    /// the grouping flips on fifty-seven bytes, which is the one value to avoid.
    ///
    /// The cost is 7 MB of catalog per chat-year and 2,378 objects for a Bible, against a
    /// redaction that takes one day rather than a fortnight. That exchange is settled in favour
    /// of the obligation that comes from outside the system.
    ///
    /// `target_max` is unchanged and, at this minimum, inert: coalescing stops at 1 KiB, so the
    /// ceiling binds only on a single node larger than it — and `Caps::part_bytes` is the hard
    /// limit behind that.
    pub const DEFAULT_LEVEL: &'static str = "chapter";
    pub const DEFAULT_MIN: u64 = 1 << 10;
    pub const DEFAULT_MAX: u64 = 4 << 20;

    pub fn new(boundary_level: Level, target_min: u64, target_max: u64) -> Policy {
        Policy {
            boundary_level,
            target_min,
            target_max,
        }
    }

    /// The string manifest key 17 records.
    pub fn id(&self) -> String {
        format!(
            "{} level={} min={} max={}",
            Policy::ID,
            self.boundary_level,
            self.target_min,
            self.target_max
        )
    }

    /// Read back a policy string.
    ///
    /// `None` for anything this build does not recognise, including a later version of the
    /// id — which is the honest answer: a manifest built under `smysl/parts/2` was cut by a
    /// rule this build does not have, and guessing at it would be worse than saying so. The
    /// manifest still opens; it is the *policy* that is unavailable, and only `text append`
    /// needs it.
    pub fn parse(s: &str) -> Option<Policy> {
        let rest = s.strip_prefix(Policy::ID)?.trim_start();
        let mut level = None;
        let mut min = None;
        let mut max = None;
        for field in rest.split_whitespace() {
            let (k, v) = field.split_once('=')?;
            match k {
                "level" => level = Level::new(v),
                "min" => min = v.parse().ok(),
                "max" => max = v.parse().ok(),
                _ => return None,
            }
        }
        Some(Policy {
            boundary_level: level?,
            target_min: min?,
            target_max: max?,
        })
    }
}

/// The provisional defaults, as [`Policy::DEFAULT_LEVEL`] and the two sizes.
///
/// A `Default` impl rather than an associated constant because [`Level`] holds a `String` and
/// a `String` cannot be built in a `const`. The three numbers are constants, so a caller who
/// wants one of them without the whole policy can still have it.
impl Default for Policy {
    fn default() -> Policy {
        Policy {
            boundary_level: Level::new(Policy::DEFAULT_LEVEL).expect("a valid level"),
            target_min: Policy::DEFAULT_MIN,
            target_max: Policy::DEFAULT_MAX,
        }
    }
}

impl std::fmt::Display for Policy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.id())
    }
}

/// One part, as grouping decided it: a byte range of the whole text, and the nodes in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartPlan {
    pub range: std::ops::Range<u64>,
    /// The indices, into the slice given to [`group`], of the nodes this part holds.
    pub nodes: std::ops::Range<usize>,
}

impl PartPlan {
    pub fn len(&self) -> u64 {
        self.range.end.saturating_sub(self.range.start)
    }

    pub fn is_empty(&self) -> bool {
        self.range.end <= self.range.start
    }
}

/// Group consecutive boundary-level nodes into parts, partitioning the text.
///
/// `nodes` are the boundary-level nodes' byte ranges, in document order and ascending — what
/// [`crate::structure::Structure::at_level`] over a reader's top level gives. Each part is
/// charged against [`crate::limits::Caps::part_bytes`], so an input whose single node exceeds
/// the hard ceiling is `SMY-E440` here rather than at the object store.
///
/// The grouping rule, in order: add nodes while the part is below `target_min`; stop once
/// adding the next node would take it past `target_max`; never emit an empty part; a single
/// node over `target_max` is its own part.
///
/// # The parts partition the text, and the nodes do not
///
/// This function used to take each part's range straight from its nodes' extent, on a stated
/// premise that the nodes cover the text “without gaps or overlaps”. **They do not.** A row
/// starts at its first text byte and ends at its last (TX-P1 step 3 made it so, to stop a row
/// carrying its predecessor's separator), so every byte *between* two nodes — a newline, a
/// blank line, the whitespace between two chapters — is in no node at all. Node-bounded parts
/// therefore dropped those bytes at each cut, and dropped the text's head and tail outright:
/// the committed `notes.txt` fixture is 471 bytes and its one part was `0..470`.
///
/// Dropped bytes are not a cosmetic loss. A part is addressed by the hash of its bytes, so the
/// concatenation of a text's parts has to *be* the text or no span, alignment or locator range
/// that crosses a cut means what it says, and `text show` of such a range would quietly be
/// missing a byte. So the boundaries are now: the first part starts at 0, each later part
/// starts where the previous one ended, and the last ends at `length`. The cut lands in the
/// gap between two nodes, which is exactly where there is nothing to cut through.
///
/// `length` is the normalised length of the whole text. Nodes past it are kept inside the last
/// part rather than refused: this is a total function over input that may break its contract,
/// for the reason the loop below gives.
pub fn group(
    nodes: &[std::ops::Range<u64>],
    policy: &Policy,
    length: u64,
    budget: &mut Budget,
) -> Result<Vec<PartPlan>, LibError> {
    let mut parts: Vec<PartPlan> = Vec::new();
    let mut i = 0usize;
    // Where the next part begins: the text's start, then each part's end.
    let mut start = 0u64;
    while i < nodes.len() {
        let extent = nodes[i].start;
        let mut end = nodes[i].end;
        let first = i;
        i += 1;
        // The first node is always in, whatever its size: the alternative is an empty part.
        //
        // The size tests are on the *nodes'* extent rather than on the part's stretched range,
        // because what the targets are about is how much text a part holds, and the separators
        // between nodes are not text anybody asked for. Every size here is a saturating
        // subtraction and `end` only ever grows — input that breaks the ascending contract gets
        // whole-node parts and an odd grouping, rather than a panic in a release build and a
        // wrong answer in a debug one.
        while i < nodes.len() {
            let would_be = nodes[i].end.saturating_sub(extent);
            if end.saturating_sub(extent) >= policy.target_min || would_be > policy.target_max {
                break;
            }
            end = end.max(nodes[i].end);
            i += 1;
        }
        // The part runs to the next group's first node, or to the end of the text. Clamped to
        // at least `start`, which only ever binds on input that breaks the ascending contract:
        // a later node that begins before an earlier one would otherwise make a part end before
        // it began, and `Range` does not forbid that — it just makes `len()` zero and every
        // slice of it `None`, which is a wrong answer where a strange one will do.
        let stop = match nodes.get(i) {
            Some(next) => next.start.max(end),
            None => length.max(end),
        }
        .max(start);
        budget.part(stop.saturating_sub(start))?;
        parts.push(PartPlan {
            range: start..stop,
            nodes: first..i,
        });
        start = stop;
    }
    Ok(parts)
}

/// A reader's rows, restricted to one part and shifted to the part's own offsets.
///
/// The shift is the whole point and the reason this is a named function with a test. A row's
/// `start`/`end` are offsets into **the part**, and a reader produces one table over the whole
/// text; a part that begins at byte 70,000 would otherwise carry rows pointing past its own
/// end, `Structure::build` would refuse the table, and the structure hash — which is over the
/// table — would be a hash of the wrong thing in the manifest entry.
///
/// Nothing caught this until now because every reader fixture in the tree produces exactly one
/// part, and for one part the shift is zero.
pub fn local_rows(rows: &[Segment], range: &std::ops::Range<u64>) -> Vec<Segment> {
    rows.iter()
        .filter(|r| r.start >= range.start && r.end <= range.end)
        .map(|r| {
            let mut row = r.clone();
            row.start -= range.start;
            row.end -= range.start;
            row
        })
        .collect()
}

/// Carve a part's text out of the whole, as a part text with its tid.
///
/// `None` when the range is not a normalised slice of the text — out of bounds, inside a
/// character, or cutting a combining mark off its base. See [`Normalised::slice`]: parts are
/// cut at structure boundaries, where that does not happen, so the refusal is the one case
/// where two libraries would otherwise name the same part differently.
pub fn text_of(text: &Normalised, range: std::ops::Range<u64>) -> Option<PartText> {
    let start = usize::try_from(range.start).ok()?;
    let end = usize::try_from(range.end).ok()?;
    let slice = text.slice(start..end)?;
    Some(PartText::new(slice.as_bytes().to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::Caps;

    fn budget() -> Budget {
        Budget::new(Caps::DEFAULT, 1 << 20).unwrap()
    }

    fn even(n: usize, size: u64) -> Vec<std::ops::Range<u64>> {
        (0..n as u64).map(|i| i * size..(i + 1) * size).collect()
    }

    fn row(start: u64, end: u64, loc: &str) -> Segment {
        Segment::new(
            start,
            end,
            Level::new("line").expect("a level"),
            crate::locator::parse(loc).expect("a locator"),
        )
    }

    #[test]
    fn the_policy_string_round_trips() {
        let p = Policy::default();
        assert_eq!(p.id(), "smysl/parts/1 level=chapter min=1024 max=4194304");
        assert_eq!(Policy::parse(&p.id()), Some(p.clone()));

        let other = Policy::new(Level::new("day").unwrap(), 1, 2);
        assert_eq!(Policy::parse(&other.id()), Some(other));
    }

    #[test]
    fn a_policy_string_this_build_does_not_know_is_none_rather_than_a_guess() {
        assert_eq!(Policy::parse("smysl/parts/2 level=day min=1 max=2"), None);
        assert_eq!(Policy::parse("level=day min=1 max=2"), None);
        assert_eq!(
            Policy::parse("smysl/parts/1 level=day min=1"),
            None,
            "a field missing is not a default"
        );
        assert_eq!(
            Policy::parse("smysl/parts/1 level=day min=1 max=2 hue=3"),
            None
        );
        assert_eq!(Policy::parse("smysl/parts/1 level=Day min=1 max=2"), None);
    }

    #[test]
    fn grouping_fills_to_the_minimum_and_stops_before_the_maximum() {
        let policy = Policy::new(Level::new("chapter").unwrap(), 100, 250);
        let nodes = even(10, 40);
        let parts = group(&nodes, &policy, 400, &mut budget()).unwrap();
        // 40-byte nodes, minimum 100: three nodes reach 120, which is over the minimum, so
        // each part is three nodes until the tail.
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            [120, 120, 120, 40]
        );
        assert_eq!(parts[0].nodes, 0..3);
        assert_eq!(parts[3].nodes, 9..10);
        // The parts partition the text: no gaps, no overlaps, nothing lost.
        assert_eq!(parts[0].range.start, 0);
        assert_eq!(parts.last().unwrap().range.end, 400);
        for w in parts.windows(2) {
            assert_eq!(w[0].range.end, w[1].range.start);
        }
    }

    #[test]
    fn a_node_larger_than_the_maximum_is_its_own_part() {
        let policy = Policy::new(Level::new("chapter").unwrap(), 100, 250);
        let nodes = vec![0..10, 10..1_000, 1_000..1_010];
        let parts = group(&nodes, &policy, 1_010, &mut budget()).unwrap();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[1].range, 10..1_000);
        assert_eq!(parts[1].nodes, 1..2, "alone, rather than split");
    }

    #[test]
    fn no_part_is_ever_empty() {
        let policy = Policy::new(Level::new("verse").unwrap(), 100, 250);
        // Three empty nodes and one with content: an empty verse is legal.
        let nodes = vec![0..0, 0..0, 0..0, 0..10];
        let parts = group(&nodes, &policy, 10, &mut budget()).unwrap();
        assert!(!parts.is_empty());
        assert_eq!(parts.iter().map(|p| p.nodes.len()).sum::<usize>(), 4);
        assert_eq!(parts.last().unwrap().range.end, 10);
    }

    #[test]
    fn nothing_to_group_is_no_parts_and_not_one_empty_part() {
        let parts = group(&[], &Policy::default(), 0, &mut budget()).unwrap();
        assert!(parts.is_empty(), "an import manifest has no parts");
    }

    #[test]
    fn a_part_over_the_hard_ceiling_is_e440_and_nothing_is_planned() {
        let policy = Policy::new(Level::new("chapter").unwrap(), 1, 10);
        let mut b = Budget::new(
            Caps {
                part_bytes: 100,
                ..Caps::DEFAULT
            },
            1 << 20,
        )
        .unwrap();
        let e = group(&[0..101, 101..110], &policy, 110, &mut b).unwrap_err();
        assert_eq!(e.code(), Some(smysl_core::Code::E440));
        assert!(e.to_string().contains("part_bytes"), "{e}");
        assert!(e.to_string().contains("--max-part"), "{e}");
    }

    /// The bytes between two nodes belong to a part, and the text's head and tail do too.
    ///
    /// The case that was wrong until TX-P1 step 6: a row ends at its last text byte, so every
    /// separator is outside every node, and node-bounded parts dropped one byte per cut plus
    /// whatever sat before the first node and after the last. A part is addressed by the hash
    /// of its bytes, so a text whose parts do not concatenate back to it has spans that mean
    /// nothing across a cut.
    /// The shift, stated as the property the manifest entry depends on: a part's rows start at
    /// zero and end at the part's length, whatever the part's place in the text.
    #[test]
    fn a_parts_rows_are_its_own_offsets() {
        let rows = vec![row(0, 10, "L1"), row(10, 25, "L2"), row(25, 40, "L3")];
        let second = local_rows(&rows, &(10..40));
        assert_eq!(second.len(), 2);
        assert_eq!((second[0].start, second[0].end), (0, 15));
        assert_eq!((second[1].start, second[1].end), (15, 30));

        // The first part is the case that hid this: its shift is zero.
        let first = local_rows(&rows, &(0..10));
        assert_eq!((first[0].start, first[0].end), (0, 10));
    }

    /// A row straddling a boundary belongs to neither part.
    ///
    /// It cannot happen from `group`, which cuts on whole nodes, and the filter is written to
    /// be total anyway: a row half in a part would otherwise be shifted to a negative offset
    /// and panic in a debug build.
    #[test]
    fn a_straddling_row_is_in_no_part() {
        let rows = vec![row(0, 20, "L1")];
        assert!(local_rows(&rows, &(10..20)).is_empty());
        assert!(local_rows(&rows, &(0..10)).is_empty());
    }

    #[test]
    fn the_parts_cover_every_byte_including_the_gaps_between_nodes() {
        let policy = Policy::new(Level::new("line").unwrap(), 10, 30);
        // Three nodes with a one-byte separator after each, inside a 34-byte text: two bytes
        // of preamble, and a trailing newline nothing claims.
        let nodes = vec![2..12, 13..23, 24..33];
        let parts = group(&nodes, &policy, 34, &mut budget()).unwrap();

        assert!(parts.len() >= 2, "the minimum should force a cut");
        assert_eq!(parts[0].range.start, 0, "the preamble is in the first part");
        assert_eq!(
            parts.last().unwrap().range.end,
            34,
            "the trailing byte is in the last part"
        );
        for w in parts.windows(2) {
            assert_eq!(w[0].range.end, w[1].range.start, "no byte is in no part");
        }
        assert_eq!(
            parts.iter().map(|p| p.len()).sum::<u64>(),
            34,
            "the parts partition the text"
        );
        // Each node still sits inside the part that claims it.
        for part in &parts {
            for node in &nodes[part.nodes.clone()] {
                assert!(
                    node.start >= part.range.start && node.end <= part.range.end,
                    "{node:?} is not inside {:?}",
                    part.range
                );
            }
        }
    }

    /// Input that breaks the ascending-and-gapless contract still produces whole-node parts
    /// and no panic. Not a supported case — a documented one.
    #[test]
    fn nodes_out_of_order_do_not_panic() {
        let policy = Policy::new(Level::new("line").unwrap(), 10, 20);
        let parts = group(&[100..110, 0..10, 50..60], &policy, 110, &mut budget()).unwrap();
        assert_eq!(
            parts.iter().map(|p| p.nodes.len()).sum::<usize>(),
            3,
            "every node is in exactly one part"
        );
        for p in &parts {
            assert!(
                p.range.end >= p.range.start,
                "no part has a negative length"
            );
        }
    }

    #[test]
    fn a_part_text_is_carved_out_with_the_tid_of_its_own_bytes() {
        let whole = Normalised::of("first line\nsecond line\n");
        let part = text_of(&whole, 11..23).unwrap();
        assert_eq!(part.text, b"second line\n".to_vec());
        assert!(part.verify());
        assert_eq!(
            part.tid,
            Normalised::of("second line\n").tid(),
            "a part's tid is its own bytes, not the document's"
        );
    }

    #[test]
    fn a_range_that_is_not_a_normalised_slice_is_refused() {
        let whole = Normalised::of("x\u{1100}\u{1161}y");
        assert!(text_of(&whole, 0..2).is_none(), "cuts into a composed jamo");
        assert!(text_of(&whole, 0..99).is_none(), "out of bounds");
        assert!(text_of(&whole, 0..1).is_some());
    }
}
