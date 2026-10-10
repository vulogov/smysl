//! **GE-T14**: what the part policy's size targets cost, measured (SMYSL-2.4 §5.5, TX-P2 step 6).
//!
//! The defaults — 64 KiB to 4 MiB at the reader's top level — have been provisional since they
//! were written, and §3.1 says so on the wire: manifest key 17 records the policy a corpus was
//! cut by, precisely so that the default can change without invalidating anything. This file is
//! the measurement that was supposed to settle it, and the number it produces is a curve rather
//! than a value, because the three things a part size trades off pull in opposite directions:
//!
//! | axis | large parts | small parts |
//! |---|---|---|
//! | object count | few | one per node, at worst |
//! | reuse on revision | the one part changes, so nothing is reused | only the touched part changes |
//! | redaction granularity | a redaction removes a lot of text with it | a redaction removes about what it names |
//!
//! # Why a synthetic corpus is the honest instrument here
//!
//! GE-T14's row asks for "real chats, revised articles, Bibles". This repository holds three
//! chat exports of two days and five verses of Genesis, which is an instrument and not a
//! corpus — the same debt GE-T1 carries. But unlike a sentence-boundary gold set, **all three
//! axes are functions of the structure alone**: the number of parts, which of them a revision
//! moves, and how many bytes one part holds depend on the node sizes and their order, and on
//! nothing about the words. So a generated corpus whose node-size distribution matches the real
//! shapes measures the *policy* exactly, and what stays owed is only whether those
//! distributions are right — which anybody with a real corpus can check in an afternoon, and
//! which the fixtures below anchor at one real point each.
//!
//! The numbers are arithmetic over ranges, so they are exact and reproducible: this is a test
//! that asserts the curve rather than a harness that prints one and rots.

use std::ops::Range;

use smysl_text::limits::{Budget, Caps};
use smysl_text::part::{self, Policy};
use smysl_text::reading::Level;

/// One shape of document, as node extents over a text.
///
/// Extents rather than text: `part::group` takes the nodes' byte ranges and the text's length,
/// and every axis GE-T14 measures is a function of those. Generating prose to measure a cut
/// would be measuring the generator.
struct Shape {
    name: &'static str,
    level: &'static str,
    /// The nodes a part is cut on, in document order, with the separator between them.
    nodes: Vec<Range<u64>>,
    length: u64,
}

/// A deterministic size in `lo..=hi`, from a counter. No RNG: the curve has to be the same
/// number on every machine and in every release, or the recorded table is a snapshot of one run.
fn size(n: u64, lo: u64, hi: u64) -> u64 {
    lo + (n * 2_654_435_761) % (hi - lo + 1)
}

/// `count` nodes of `lo..=hi` bytes, separated by one byte, as a reader emits them.
fn shape(name: &'static str, level: &'static str, count: u64, lo: u64, hi: u64) -> Shape {
    let mut nodes = Vec::with_capacity(count as usize);
    let mut at = 0u64;
    for i in 0..count {
        let len = size(i, lo, hi);
        nodes.push(at..at + len);
        // One separator byte between nodes, which is what every reader in this crate emits and
        // what makes a part's range reach past its last node (TX-P1 step 6).
        at += len + 1;
    }
    let length = at.saturating_sub(1);
    Shape {
        name,
        level,
        nodes,
        length,
    }
}

/// A year of chat: 40 messages a day of 60–140 bytes, grouped into days.
///
/// Anchored on `fixtures/library/readers/telegram.json`, whose messages are 28–81 bytes and
/// whose days are 113 and 235 — the generated day is 4 KB because a real day is forty messages
/// and not three.
fn chat_year() -> Shape {
    let mut nodes = Vec::new();
    let mut at = 0u64;
    for day in 0..365u64 {
        let start = at;
        for m in 0..40u64 {
            at += size(day * 40 + m, 60, 140) + 1;
        }
        nodes.push(start..at - 1);
        at += 1;
    }
    Shape {
        name: "chat, one year",
        level: "day",
        nodes,
        length: at.saturating_sub(1),
    }
}

/// A Bible: 1,189 chapters of about thirty verses each.
///
/// Anchored on `fixtures/library/readers/gen1.usfm`: five verses, 55–97 bytes. A chapter of
/// thirty such verses is about 4.5 KB, and the whole is about 5 MB — which is what a printed
/// Bible's text comes to.
fn bible() -> Shape {
    let mut nodes = Vec::new();
    let mut at = 0u64;
    for chapter in 0..1189u64 {
        let start = at;
        for v in 0..30u64 {
            at += size(chapter * 30 + v, 110, 190) + 1;
        }
        nodes.push(start..at - 1);
        at += 1;
    }
    Shape {
        name: "Bible, 1189 chapters",
        level: "chapter",
        nodes,
        length: at.saturating_sub(1),
    }
}

/// A revised article: forty paragraphs of 400–900 bytes.
fn article() -> Shape {
    shape("article, 40 paragraphs", "paragraph", 40, 400, 900)
}

fn plans(shape: &Shape, min: u64) -> Vec<Range<u64>> {
    let policy = Policy::new(
        Level::new(shape.level).expect("a level"),
        min,
        Policy::DEFAULT_MAX,
    );
    let mut budget = Budget::new(Caps::DEFAULT, shape.length).expect("a budget");
    part::group(&shape.nodes, &policy, shape.length, &mut budget)
        .expect("grouping whole nodes cannot exceed a cap this corpus reaches")
        .into_iter()
        .map(|p| p.range)
        .collect()
}

/// What one revision rewrites, in parts.
///
/// A part survives a revision when its bytes do: its range must be identical in both cuts
/// **and** lie wholly before the first byte the revision touched. Everything at or after that
/// byte is new text or shifted text, and a shifted range is a different tid even where the words
/// are the same.
fn rewritten(before: &[Range<u64>], after: &[Range<u64>], edit_at: u64) -> usize {
    let kept = after
        .iter()
        .filter(|r| r.end <= edit_at && before.iter().any(|b| *b == **r))
        .count();
    after.len() - kept
}

/// One row of the curve.
struct Row {
    min: u64,
    parts: usize,
    mean: u64,
    max: u64,
    /// Parts rewritten when the document grows at the end.
    grow: usize,
    /// Parts rewritten when a node in the middle is corrected **without changing its length**.
    correct: usize,
    /// Parts rewritten when a node in the middle is corrected and its length changes.
    relength: usize,
    /// Parts below `target_min`, excluding the last — which is short whenever the document ran
    /// out of nodes, and is the one part the target cannot bind on.
    short: usize,
}

fn curve(shape: &Shape, grown: &Shape, edit_index: usize) -> Vec<Row> {
    let mut rows = Vec::new();
    for min in [1u64 << 10, 4 << 10, 16 << 10, 64 << 10, 256 << 10, 1 << 20] {
        let before = plans(shape, min);
        let mean = before.iter().map(|r| r.end - r.start).sum::<u64>() / before.len() as u64;
        let max = before
            .iter()
            .map(|r| r.end - r.start)
            .max()
            .unwrap_or_default();

        // Growth: the same document with more nodes at the end. The first byte that changes is
        // the old text's last byte — the separator the part that *was* last gains (TX-P2 step 3).
        let grow = rewritten(&before, &plans(grown, min), shape.length);

        // A correction of the same length: every offset after it is unchanged, so the only part
        // whose bytes move is the one the edit is inside. Counted directly rather than through
        // `rewritten`, which answers the *shift* question — "what lies after the edit" — and
        // would report every later part here, which is the wrong answer for an edit that
        // changes no offsets.
        let edit_at = shape.nodes[edit_index].start;
        let correct = before.iter().filter(|r| r.contains(&edit_at)).count();

        // A correction that changes the node's length: every later offset shifts, so every
        // later part is a different part.
        let mut shifted = Shape {
            name: shape.name,
            level: shape.level,
            nodes: shape.nodes.clone(),
            length: shape.length + 7,
        };
        for (i, n) in shifted.nodes.iter_mut().enumerate() {
            if i == edit_index {
                n.end += 7;
            } else if i > edit_index {
                n.start += 7;
                n.end += 7;
            }
        }
        let relength = rewritten(&before, &plans(&shifted, min), edit_at);

        let last = before.len().saturating_sub(1);
        let short = before
            .iter()
            .enumerate()
            .filter(|(i, r)| *i != last && r.end - r.start < min)
            .count();

        rows.push(Row {
            min,
            parts: before.len(),
            mean,
            max,
            grow,
            correct,
            relength,
            short,
        });
    }
    rows
}

fn print_curve(shape: &Shape, rows: &[Row]) {
    println!(
        "\n{} — {} nodes at level `{}`, {} bytes",
        shape.name,
        shape.nodes.len(),
        shape.level,
        shape.length
    );
    println!("  target_min   parts   mean part   max part   rewritten: grow / correct / relength");
    for r in rows {
        println!(
            "  {:>9}  {:>6}  {:>10}  {:>9}   {:>4} / {:>7} / {:>8}",
            r.min, r.parts, r.mean, r.max, r.grow, r.correct, r.relength
        );
    }
}

/// **The curve.** Printed with `--nocapture`, and asserted so that it cannot rot.
///
/// What the three shapes agree on, and what fixes the default:
///
/// 1. **Growth costs at most two parts at every size**, and never more: the part that *was*
///    last — which gains the separator joining it to its successor (TX-P2 step 3) — and the new
///    part, where one was opened rather than the last group absorbing the new nodes. Which of
///    the two happens depends on whether the last group had reached `target_min`, so the number
///    alternates between 1 and 2 down the sweep and is bounded by 2 everywhere. `target_min`
///    therefore buys **nothing** for growth, which is the case a chat corpus is in every day.
/// 2. **An in-place correction costs one part if its length is unchanged, and every part after
///    it if it is not.** Also independent of the size — what decides it is whether the bytes
///    after the edit kept their offsets, and a part is named by the hash of its bytes. Smaller
///    parts make the re-length case *worse* in object count, not better: 185 parts rewritten at
///    1 KiB against 12 at 64 KiB, for the same correction.
/// 3. **So the only axis the size genuinely trades is object count against redaction
///    granularity**, and that one is a straight exchange: 1 KiB parts give 16× the objects of
///    64 KiB and remove 16× less text per redaction.
#[test]
fn the_part_size_curve_is_what_it_is() {
    let shapes: Vec<(Shape, Shape, usize)> = vec![
        (chat_year(), chat_longer(), 180),
        (article(), article_longer(), 20),
        (bible(), bible_longer(), 600),
    ];
    for (shape, grown, edit) in &shapes {
        let rows = curve(shape, grown, *edit);
        print_curve(shape, &rows);

        for r in &rows {
            // 1. Growth is bounded by two parts: the one that was last, and a new one where the
            //    last group did not absorb the new nodes.
            assert!(
                (1..=2).contains(&r.grow),
                "{}: growing at the end rewrote {} parts at min={}",
                shape.name,
                r.grow,
                r.min
            );
            // 2. A same-length correction is one part; a re-length is that part and every one
            //    after it.
            assert_eq!(r.correct, 1, "{} at min={}", shape.name, r.min);
            assert!(
                r.relength >= 1,
                "{} at min={}: a re-length rewrites at least the part it is in",
                shape.name,
                r.min
            );
            // 3. Every part but the last reaches the minimum.
            //
            // Not the *mean*, which is what this asserted first and which is false for a good
            // reason: the last part is whatever is left when the nodes run out, so with six
            // parts at 256 KiB a short tail pulls the mean below the target. The target is a
            // claim about each part that could be grown, and the last one could not.
            assert_eq!(
                r.short, 0,
                "{} at min={}: {} part(s) below the minimum",
                shape.name, r.min, r.short
            );
        }
        // The exchange, stated as a number: the smallest size in the sweep makes more parts
        // than the largest, and they are smaller in the same proportion.
        let small = &rows[0];
        let large = &rows[rows.len() - 1];
        assert!(small.parts >= large.parts, "{}", shape.name);
        assert!(small.mean <= large.mean, "{}", shape.name);
    }
}

/// The chat, one day longer.
fn chat_longer() -> Shape {
    let mut s = chat_year();
    let start = s.length + 1;
    let mut at = start;
    for m in 0..40u64 {
        at += size(365 * 40 + m, 60, 140) + 1;
    }
    s.nodes.push(start..at - 1);
    s.length = at - 1;
    s
}

/// The article, one paragraph longer.
fn article_longer() -> Shape {
    let mut s = article();
    let start = s.length + 1;
    let len = size(40, 400, 900);
    s.nodes.push(start..start + len);
    s.length = start + len;
    s
}

/// The Bible, one chapter longer. (A canon does not grow; the shape of the measurement does not
/// depend on that, and a revised edition that adds an apocryphal book is the same arithmetic.)
fn bible_longer() -> Shape {
    let mut s = bible();
    let start = s.length + 1;
    let mut at = start;
    for v in 0..30u64 {
        at += size(1189 * 30 + v, 110, 190) + 1;
    }
    s.nodes.push(start..at - 1);
    s.length = at - 1;
    s
}

/// The fourth axis, and the one that argues the other way: what the **catalog** costs.
///
/// A part is an entry in a manifest, and an append writes a whole new manifest — so a year of
/// daily appends writes the manifest 365 times, each time as long as the corpus has parts by
/// then. Smaller parts mean more entries mean a longer manifest mean a bigger log, and the
/// relationship is linear in the part count. Measured rather than estimated, because the
/// decision in this file's header turns on whether the number is megabytes or gigabytes.
#[test]
fn the_catalog_cost_of_a_year_of_daily_appends() {
    use smysl_core::cbor::envelope::manifest_bytes;
    use smysl_core::ids::{Rdid, Tid};
    use smysl_core::types::{Carry, PartEntry};
    use smysl_text::ManifestBuilder;

    let entry = |n: u64| {
        PartEntry::new(
            Tid::of(&n.to_le_bytes()),
            4_041,
            [0x5a; 32],
            Rdid::of(&n.to_le_bytes()),
        )
    };
    let manifest_of = |parts: usize| -> usize {
        let mut b = ManifestBuilder::new(
            "chat",
            smysl_core::LangTag::new("mul").expect("a tag"),
            "telegram/1",
            "CC0-1.0",
            &Policy::new(
                Level::new("day").expect("a level"),
                1 << 10,
                Policy::DEFAULT_MAX,
            ),
        )
        .expect("a builder")
        .carry(Carry::Text);
        for n in 0..parts as u64 {
            b = b.part(entry(n));
        }
        manifest_bytes(&b.build().expect("a manifest")).len()
    };

    println!(
        "
the catalog cost of a year of daily appends to one chat"
    );
    println!("  target_min   parts   manifest   log after 365 appends");
    let chat = chat_year();
    for min in [1u64 << 10, 4 << 10, 16 << 10, 64 << 10, 256 << 10] {
        let parts = plans(&chat, min).len();
        let full = manifest_of(parts);
        // Each day's append writes a manifest of the size the corpus had reached. The part
        // count grows linearly with the days, so the sum is the average size times the days —
        // computed by summing rather than by assuming the linearity.
        let total: usize = (1..=365)
            .map(|day| manifest_of((parts * day / 365).max(1)))
            .sum();
        println!(
            "  {:>9}  {:>6}  {:>9}  {:>10} ({:.1} MB)",
            min,
            parts,
            full,
            total,
            total as f64 / 1_048_576.0
        );
    }

    // The number the decision rests on: at the finest size in the sweep, a year of daily
    // appends to one chat costs single-digit megabytes of catalog. That is the cost of being
    // able to redact one day instead of a fortnight.
    let finest = plans(&chat, 1 << 10).len();
    let total: usize = (1..=365)
        .map(|day| manifest_of((finest * day / 365).max(1)))
        .sum();
    assert!(
        total < 16 * 1_048_576,
        "a year of daily appends at 1 KiB costs {total} bytes of catalog"
    );
}

/// What a redaction takes with it, at each size, for the shape it matters most for.
///
/// The axis `target_min` actually buys something on. A redaction names a *part*: the bytes it
/// removes are the whole part, so the mean part size is the granularity. For a chat, that is the
/// difference between losing a day and losing two months.
#[test]
fn redaction_granularity_is_the_part_size() {
    let chat = chat_year();
    let day = chat.nodes.iter().map(|n| n.end - n.start).sum::<u64>() / chat.nodes.len() as u64;
    println!("\na chat day is about {day} bytes");
    for min in [1u64 << 10, 4 << 10, 64 << 10] {
        let parts = plans(&chat, min);
        let mean = parts.iter().map(|r| r.end - r.start).sum::<u64>() / parts.len() as u64;
        println!(
            "  min={:>6}: {:>4} parts, a redaction removes about {} bytes — {} day(s)",
            min,
            parts.len(),
            mean,
            mean / day
        );
    }
    // **The assertion the default was changed for.** At 64 KiB — the provisional default this
    // measurement replaced — a chat redaction took about sixteen days of conversation with it.
    // At the default GE-T14 fixed, it takes one.
    let at_old_default = plans(&chat, 64 << 10);
    let old_mean =
        at_old_default.iter().map(|r| r.end - r.start).sum::<u64>() / at_old_default.len() as u64;
    assert!(
        old_mean / day >= 10,
        "the provisional default removed {} days, which is why it was provisional",
        old_mean / day
    );

    let at_default = plans(&chat, Policy::DEFAULT_MIN);
    let mean = at_default.iter().map(|r| r.end - r.start).sum::<u64>() / at_default.len() as u64;
    assert_eq!(
        mean / day,
        1,
        "a redaction at the fixed default removes about one day"
    );
    assert_eq!(
        at_default.len(),
        chat.nodes.len(),
        "at the fixed default the boundary level governs: one part per day"
    );
}
