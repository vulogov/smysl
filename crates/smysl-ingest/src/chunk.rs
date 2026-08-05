//! Chunking (§22.2).
//!
//! Window to `context_window − prompt_reserve − output_reserve`, split at paragraph
//! boundaries, overlap one paragraph.
//!
//! **Chunk-boundary duplication self-heals.** Two chunks that independently produce the
//! same claim produce the same uid and merge to one unit with two attestations - that is
//! content addressing doing the work. So over-chunking costs tokens, not correctness, and
//! the overlap is deliberately generous rather than minimal.
//!
//! A paragraph longer than a whole window is split on lines, then on characters. The
//! alternative - refusing - would mean one runaway paragraph could fail an ingest, which
//! rule I forbids.

use smysl_core::tokens;

/// How much of the window to keep back for the prompt itself.
pub const PROMPT_RESERVE: usize = 1024;

/// How much to keep back for the model's answer. Ingest output is roughly the size of its
/// input, so this is not a small fraction.
pub const OUTPUT_RESERVE_RATIO: f32 = 0.5;

/// One piece of input, with enough context to say where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Chunk {
    pub text: String,
    /// Zero-based index, for prompts that say "part 3 of 7".
    pub index: usize,
    pub total: usize,
    /// Byte range in the original input, so a diagnostic can point at the source.
    pub start: usize,
    pub end: usize,
    /// Whether this chunk repeats the tail of the one before it.
    pub overlapped: bool,
}

impl Chunk {
    /// `#[cfg(test)]` since 0.13: the only callers are this module's tests, and making the
    /// module `pub(crate)` in §1.2 S4 is what let the compiler say so. Production reads
    /// `Chunk::text` and counts once, rather than re-counting per chunk.
    #[cfg(test)]
    pub fn tokens(&self) -> u32 {
        tokens(&self.text)
    }
}

/// How to chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Window {
    /// Tokens available for input text, after both reserves.
    pub budget: u32,
    /// Whether to repeat the last paragraph of each chunk at the start of the next.
    pub overlap: bool,
}

impl Window {
    /// The window a provider's context leaves for input text (§22.2).
    pub fn for_context(context_window: usize, max_output: usize) -> Window {
        let output_reserve =
            max_output.max(((context_window as f32) * OUTPUT_RESERVE_RATIO) as usize);
        let budget = context_window
            .saturating_sub(PROMPT_RESERVE)
            .saturating_sub(output_reserve);
        Window {
            // A window that reserved itself out of existence would chunk into nothing, so
            // there is a floor: one paragraph has to fit somewhere.
            budget: budget.max(256) as u32,
            overlap: true,
        }
    }

    /// A window at a chosen budget. `#[cfg(test)]` since 0.13: production builds windows
    /// from a provider's context with `for_context`, and only tests pick a raw number.
    #[cfg(test)]
    pub fn of(budget: u32) -> Window {
        Window {
            budget: budget.max(1),
            overlap: true,
        }
    }

    /// `#[cfg(test)]` for the same reason as `of`. Overlap is on for every real ingest —
    /// it is what makes a claim spanning a chunk boundary visible whole to one chunk.
    #[cfg(test)]
    pub fn without_overlap(mut self) -> Window {
        self.overlap = false;
        self
    }
}

/// Split input into windows at paragraph boundaries.
pub fn chunk(input: &str, w: Window) -> Vec<Chunk> {
    let paragraphs = paragraphs(input);
    if paragraphs.is_empty() {
        return Vec::new();
    }

    // A paragraph larger than the whole window cannot be grouped into anything that fits, so
    // it is broken before grouping rather than after.
    //
    // `split_oversized` was written for this and never called: the grouping loop below starts
    // a new group when the *next* paragraph would overflow, which does nothing when a single
    // paragraph is already over budget on its own — it goes into a group regardless. The
    // function was tested, so its own tests passed; nothing checked that anybody used it.
    // Found in 0.13 when §1.2 S4 made the module `pub(crate)` and dead-code analysis could
    // finally see it. One 5 000-token paragraph produced one 5 000-token chunk against a
    // budget of 50.
    let paragraphs: Vec<(usize, usize, String)> = paragraphs
        .into_iter()
        .flat_map(|(start, end, text)| {
            if tokens(&text) > w.budget {
                split_oversized(&text, w.budget, start)
            } else {
                vec![(start, end, text)]
            }
        })
        .collect();

    // Group paragraphs greedily into windows.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut used = 0u32;

    for (i, (_, _, text)) in paragraphs.iter().enumerate() {
        let cost = tokens(text);
        if !current.is_empty() && used + cost > w.budget {
            groups.push(std::mem::take(&mut current));
            used = 0;
        }
        current.push(i);
        used += cost;
    }
    if !current.is_empty() {
        groups.push(current);
    }

    let total = groups.len();
    let mut out = Vec::with_capacity(total);
    for (n, group) in groups.iter().enumerate() {
        let first = group[0];
        // The overlap repeats the previous group's last paragraph, so a claim that spans a
        // boundary is seen whole by at least one chunk. Duplication self-heals.
        let overlapped = w.overlap && n > 0;
        let from = if overlapped {
            groups[n - 1].last().copied().unwrap_or(first)
        } else {
            first
        };
        let last = *group.last().expect("a group is never empty");

        out.push(Chunk {
            text: paragraphs[from..=last]
                .iter()
                .map(|(_, _, t)| t.as_str())
                .collect::<Vec<_>>()
                .join("\n\n"),
            index: n,
            total,
            start: paragraphs[from].0,
            end: paragraphs[last].1,
            overlapped,
        });
    }
    out
}

/// Split into paragraphs, then split any paragraph too large to ever fit.
///
/// Returns `(start, end, text)` triples over the original input.
fn paragraphs(input: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut start = 0usize;

    for raw in input.split("\n\n") {
        let end = start + raw.len();
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let offset = raw.find(trimmed).unwrap_or(0);
            out.push((
                start + offset,
                start + offset + trimmed.len(),
                trimmed.to_string(),
            ));
        }
        // `+ 2` for the separator that `split` removed.
        start = end + 2;
    }
    out
}

/// Break a paragraph that cannot fit any window, on lines and then on characters.
///
/// Refusing would mean one runaway paragraph could fail an ingest, which rule I forbids.
pub fn split_oversized(text: &str, budget: u32, start: usize) -> Vec<(usize, usize, String)> {
    if tokens(text) <= budget {
        return vec![(start, start + text.len(), text.to_string())];
    }

    let mut out = Vec::new();
    let mut buf = String::new();
    let mut buf_start = start;
    let mut cursor = start;

    for line in text.split_inclusive('\n') {
        if !buf.is_empty() && tokens(&(buf.clone() + line)) > budget {
            out.push((buf_start, cursor, buf.trim_end().to_string()));
            buf.clear();
            buf_start = cursor;
        }
        // A single line over budget is split on character boundaries; there is nothing
        // finer to split on, and progress matters more than a tidy boundary.
        if tokens(line) > budget {
            let per = (budget as usize).saturating_mul(4).max(1);
            let mut at = 0usize;
            while at < line.len() {
                let mut to = (at + per).min(line.len());
                while to < line.len() && !line.is_char_boundary(to) {
                    to += 1;
                }
                out.push((cursor + at, cursor + to, line[at..to].to_string()));
                at = to;
            }
            cursor += line.len();
            buf_start = cursor;
            continue;
        }
        buf.push_str(line);
        cursor += line.len();
    }
    if !buf.trim().is_empty() {
        out.push((buf_start, cursor, buf.trim_end().to_string()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rule I, at the one input that used to break it.
    ///
    /// A paragraph bigger than the entire window is the case `split_oversized` was written
    /// for, and until 0.13 `chunk` never called it — so a single runaway paragraph produced a
    /// single chunk many times the budget, which is exactly the "one paragraph fails an
    /// ingest" that rule I forbids. The function's own tests passed throughout; what was
    /// missing was a test that anything used it.
    ///
    /// Asserted on the output of `chunk` rather than on `split_oversized`, deliberately: a
    /// test of the helper is what already existed and what already passed.
    #[test]
    fn one_runaway_paragraph_is_split_rather_than_sent_whole() {
        let para = "word ".repeat(4000); // no blank line anywhere: one paragraph

        // Overlap off, so this measures the split and not the deliberate repetition. With it
        // on a chunk carries its own content plus the previous one's tail, which is by design
        // — "over-chunking costs tokens, not correctness" — and would put every chunk at
        // roughly twice the budget whether or not the split worked.
        let w = Window::of(50).without_overlap();
        let chunks = chunk(&para, w);

        assert!(
            chunks.len() > 1,
            "an oversized paragraph must become several chunks"
        );
        for c in &chunks {
            assert!(
                c.tokens() <= w.budget,
                "chunk {} carries {} tokens against a budget of {}",
                c.index,
                c.tokens(),
                w.budget
            );
        }

        // With overlap on it must still be bounded, just not by one budget. Two windows, plus
        // the `\n\n` the pieces are rejoined with — which costs a token, and is why this is
        // not exactly `2 * budget`.
        let with_overlap = Window::of(50);
        for c in chunk(&para, with_overlap) {
            assert!(
                c.tokens() <= 2 * with_overlap.budget + 1,
                "chunk {} carries {} tokens; overlap doubles the budget, it does not lift it",
                c.index,
                c.tokens()
            );
        }

        // The control: splitting must not lose the text. Token counts are what the budget is
        // measured in, so compare those rather than bytes, which the rejoining can change.
        let total: u32 = chunks.iter().map(|c| c.tokens()).sum();
        assert!(
            total >= tokens(&para) / 2,
            "splitting dropped most of the input: {total} tokens out of {}",
            tokens(&para)
        );
    }

    fn para(n: usize) -> String {
        (0..n)
            .map(|i| format!("Paragraph {i} says something of moderate length about the matter."))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    #[test]
    fn short_input_is_one_chunk() {
        let c = chunk("a single short paragraph", Window::of(1000));
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].index, 0);
        assert_eq!(c[0].total, 1);
        assert!(!c[0].overlapped);
    }

    #[test]
    fn empty_input_yields_no_chunks() {
        assert!(chunk("", Window::of(100)).is_empty());
        assert!(chunk("   \n\n  \n", Window::of(100)).is_empty());
    }

    #[test]
    fn input_is_split_at_paragraph_boundaries() {
        let text = para(12);
        let c = chunk(&text, Window::of(40));
        assert!(c.len() > 1, "expected several chunks");
        for piece in &c {
            // No chunk starts or ends mid-sentence, because splitting only ever happened
            // between paragraphs.
            assert!(piece.text.starts_with("Paragraph"), "{}", piece.text);
            assert!(piece.text.ends_with('.'), "{}", piece.text);
        }
    }

    /// A claim that spans a boundary is seen whole by at least one chunk.
    #[test]
    fn each_chunk_after_the_first_repeats_the_previous_paragraph() {
        let text = para(12);
        let c = chunk(&text, Window::of(40));
        for pair in c.windows(2) {
            let (before, after) = (&pair[0], &pair[1]);
            assert!(after.overlapped);
            let tail = before.text.rsplit("\n\n").next().unwrap();
            assert!(
                after.text.starts_with(tail),
                "overlap missing:\n…{tail}\nvs\n{}",
                after.text
            );
        }
    }

    #[test]
    fn overlap_can_be_turned_off() {
        let c = chunk(&para(12), Window::of(40).without_overlap());
        assert!(c.iter().all(|p| !p.overlapped));
        for pair in c.windows(2) {
            let tail = pair[0].text.rsplit("\n\n").next().unwrap();
            assert!(!pair[1].text.starts_with(tail));
        }
    }

    #[test]
    fn every_chunk_knows_where_it_is() {
        let c = chunk(&para(12), Window::of(40));
        let total = c.len();
        for (i, piece) in c.iter().enumerate() {
            assert_eq!(piece.index, i);
            assert_eq!(piece.total, total);
        }
    }

    /// Byte ranges must point back into the original, so a diagnostic can name a span the
    /// caller can look at.
    #[test]
    fn byte_ranges_point_into_the_original_input() {
        let text = para(12);
        for piece in chunk(&text, Window::of(40)) {
            assert!(piece.end <= text.len());
            assert!(piece.start < piece.end);
            assert!(text.is_char_boundary(piece.start) && text.is_char_boundary(piece.end));
            let slice = &text[piece.start..piece.end];
            assert!(slice.starts_with("Paragraph"), "{slice}");
        }
    }

    /// Over-chunking costs tokens, not correctness - so nothing may be *lost*.
    #[test]
    fn every_paragraph_appears_in_some_chunk() {
        let text = para(30);
        let joined: String = chunk(&text, Window::of(30))
            .iter()
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
            .join("\n\n");
        for i in 0..30 {
            assert!(joined.contains(&format!("Paragraph {i} ")), "lost {i}");
        }
    }

    /// Nothing is dropped, which is what this test was always for.
    ///
    /// Until 0.13 it also asserted `c.len() == 1` — "one paragraph is one group, however
    /// large" — which encoded the defect below it as expected behaviour. The test directly
    /// beneath this one asserts that `split_oversized` breaks such a paragraph up, and quotes
    /// rule I while doing it. The two contradicted each other and both were green, because
    /// nothing connected `split_oversized` to `chunk`.
    ///
    /// The emission half was the part worth keeping: rule I says an ingest makes progress,
    /// and a runaway paragraph vanishing would break it just as surely as a refusal.
    #[test]
    fn a_paragraph_larger_than_the_window_is_still_emitted() {
        let huge = "x ".repeat(5000);
        let c = chunk(&huge, Window::of(10));
        assert!(
            !c.is_empty(),
            "an oversized paragraph must still be emitted"
        );
        assert!(
            c.iter().all(|k| !k.text.is_empty()),
            "no chunk may be empty"
        );
        assert!(
            c.len() > 1,
            "and it is split rather than sent whole: see `chunk`"
        );
    }

    /// Refusing would mean one runaway paragraph could fail an ingest, which rule I
    /// forbids.
    #[test]
    fn an_oversized_paragraph_splits_on_lines_then_characters() {
        let lines = (0..50)
            .map(|i| format!("line {i} of a long block"))
            .collect::<Vec<_>>()
            .join("\n");
        let parts = split_oversized(&lines, 20, 0);
        assert!(parts.len() > 1);
        for (start, end, text) in &parts {
            assert!(start < end);
            assert!(!text.is_empty());
        }

        let unbroken = "z".repeat(4000);
        let parts = split_oversized(&unbroken, 10, 0);
        assert!(parts.len() > 1, "a single line still splits");
        assert_eq!(
            parts.iter().map(|(_, _, t)| t.len()).sum::<usize>(),
            unbroken.len(),
            "nothing is lost"
        );
    }

    #[test]
    fn splitting_something_that_fits_is_the_identity() {
        let parts = split_oversized("small", 100, 7);
        assert_eq!(parts, vec![(7, 12, "small".to_string())]);
    }

    #[test]
    fn a_character_split_lands_on_boundaries() {
        let text = "é".repeat(2000);
        for (_, _, part) in split_oversized(&text, 5, 0) {
            assert!(part.chars().all(|c| c == 'é'), "split mid-character");
        }
    }

    // -- window sizing -------------------------------------------------------

    #[test]
    fn the_window_reserves_room_for_the_prompt_and_the_answer() {
        let w = Window::for_context(8192, 2048);
        assert!(w.budget < 8192 - PROMPT_RESERVE as u32);
        assert!(w.budget > 0);
    }

    /// Ingest output is roughly the size of its input, so the output reserve is not a small
    /// fraction - a window that ignored it would produce chunks whose answers cannot fit.
    #[test]
    fn the_output_reserve_is_at_least_half_the_context() {
        let w = Window::for_context(10_000, 100);
        assert!(w.budget <= 10_000 - 5_000, "budget {}", w.budget);
    }

    /// A window that reserved itself out of existence would chunk into nothing.
    #[test]
    fn a_tiny_context_still_leaves_a_usable_window() {
        let w = Window::for_context(512, 512);
        assert!(w.budget >= 256);
        assert!(!chunk(&para(3), w).is_empty());
    }

    #[test]
    fn a_zero_budget_is_clamped_rather_than_dividing_by_zero() {
        assert!(Window::of(0).budget >= 1);
        assert!(!chunk("text", Window::of(0)).is_empty());
    }
}
