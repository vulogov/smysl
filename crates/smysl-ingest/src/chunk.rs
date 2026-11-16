//! Chunking (§22.2).
//!
//! Window to `context_window − prompt_reserve − output_reserve`, and never wider than the
//! answer the call can carry (`max_output`); split at paragraph boundaries, overlap one
//! paragraph.
//!
//! **The cap is the output's, not the context's.** A chunk's answer has to fit `max_output`,
//! or the model stops at the limit, the call is a truncation, and the whole chunk degrades to
//! opaque prose under rule I. 1.15 capped the chunk at `max_output` on the premise that an
//! answer is about the size of its passage. 1.16 measured the answer: three to seven times
//! the passage's tokens on the three current Anthropic models, because a staged unit carries
//! its gist, its kind, its span and its provenance, and a short passage pays that per unit.
//! The chunk is therefore sized to `max_output`, less the reasoning headroom, divided by
//! [`ANSWER_PER_INPUT_TOKEN`]. On a million-token window the context alone would still make a
//! chunk of a whole book and lose it.
//!
//! **Chunk-boundary duplication self-heals.** Two chunks that independently produce the
//! same claim produce the same uid and merge to one unit with two attestations - that is
//! content addressing doing the work. So over-chunking costs tokens, not correctness, and
//! the overlap is deliberately generous rather than minimal.
//!
//! A paragraph longer than a whole window is split on lines, then on characters. The
//! alternative - refusing - would mean one runaway paragraph could fail an ingest, which
//! rule I forbids.

/// The estimate a chunk is sized by: bytes per token, one divisor per class of character.
///
/// `smysl_core::tokens` is `ceil(bytes / 4)` and stays so: it is the format's own count, pure
/// by rule D and the same in every implementation. It is also a third out, in the wrong
/// direction for sizing. Measured against `count_tokens` on 48 samples for 1.16 — prose,
/// markup, Rust, TOML, SMYSL surface and passages in fourteen scripts — it under-counted 45 of
/// them, by up to 57%: a chunk sized by it is up to twice its estimate in the tokeniser's
/// terms, and a chunk twice its size is one whose answer the call cannot carry. A
/// least-squares fit of one divisor per class had a mean error of 8% against 34%.
///
/// The divisors here sit on the safe side of that fit, since over-estimating costs a call and
/// under-estimating costs the chunk:
///
/// | class | bytes per token | the fit said |
/// |---|---|---|
/// | ASCII letters and spaces | 3.0 | 3.5 |
/// | other ASCII: digits, punctuation, newlines | 1.0 | 1.1 |
/// | Cyrillic | 4.0 | 5.1 |
/// | other two-byte scripts: Latin accents, Greek, Arabic, Hebrew | 2.0 | 2.5 |
/// | three- and four-byte: CJK, Devanagari, emoji | 2.5 | 2.9 |
///
/// On the 48 samples this under-estimates 8, by at most 25%, and over-estimates the rest by up
/// to 41%; the mean error is 16%.
///
/// **Latin script that is not English is dearer.** The 25% was German, and 1.16 first put it
/// down to compounds. Measured again over GE-T2's corpus — Genesis 4, Exodus 20 and a psalm in
/// each tier-1 language, 45 samples beside the 48 — the tokeniser spends a token per 2.9 letters
/// of English, 2.3 of Spanish or French and 2.1 of German, and the Luther German has almost no
/// long words: the residual is the tokeniser's English, and it reaches every Latin-script
/// language that is not. Those languages carry a mark English does not, accented letters, so
/// where the two-byte letters are at least one in a hundred of the ASCII ones and no Cyrillic is
/// present, letters are counted at 2.25 bytes a token. Under that rule the English samples are
/// as before (8 of 41 under, by at most 11.5%), Spanish and French are within 3%, and nine
/// German samples of ten are within 10%; the tenth is technical German of compounds and two
/// accents, still 28% under. A word rule — letters in words past 8, 10, 12 or 14 characters at
/// a divisor of their own — fitted to nothing on the 93 and is not used.
///
/// It is a sizing estimate and nothing else: the ledger, the gist limits and the pack budgets
/// keep the format's count.
pub fn sizing_tokens(text: &str) -> u32 {
    let mut tally = Sizing::default();
    for ch in text.chars() {
        tally.add(ch);
    }
    tally.tokens()
}

/// The per-class byte tally behind [`sizing_tokens`], so a splitter can add one character at
/// a time and ask after each.
#[derive(Debug, Default, Clone, Copy)]
struct Sizing {
    alpha: u32,
    other: u32,
    cyrillic: u32,
    two: u32,
    wide: u32,
}

impl Sizing {
    fn add(&mut self, ch: char) {
        let code = ch as u32;
        if code < 0x80 {
            if ch.is_ascii_alphabetic() || ch == ' ' {
                self.alpha += 1;
            } else {
                self.other += 1;
            }
        } else if code < 0x800 {
            if (0x400..=0x4ff).contains(&code) {
                self.cyrillic += 2;
            } else {
                self.two += 2;
            }
        } else if code < 0x10000 {
            self.wide += 3;
        } else {
            self.wide += 4;
        }
    }

    /// Each class rounds up on its own, so a text of one class never estimates to zero and
    /// the sum never falls below the fit's figure for any class.
    fn tokens(self) -> u32 {
        // Accented letters mark a Latin-script text as not English, and the tokeniser spends
        // more on it: 2.25 bytes a letter where English gets 3.0 (see [`sizing_tokens`]).
        let alpha = if self.cyrillic == 0 && self.two * 100 >= self.alpha {
            (self.alpha * 4).div_ceil(9)
        } else {
            self.alpha.div_ceil(3)
        };
        alpha
            + self.other
            + self.cyrillic.div_ceil(4)
            + self.two.div_ceil(2)
            + (self.wide * 2).div_ceil(5)
    }
}

/// The estimate chunking works in; see [`sizing_tokens`].
fn tokens(text: &str) -> u32 {
    sizing_tokens(text)
}

/// How much of the window to keep back for the prompt itself.
pub const PROMPT_RESERVE: usize = 1024;

/// How much of the context to keep back for the model's answer. Not a small fraction: the
/// answer is several times the passage (see [`ANSWER_PER_INPUT_TOKEN`]).
pub const OUTPUT_RESERVE_RATIO: f32 = 0.5;

/// How many answer tokens a token of passage costs: the chunk is sized to the answer the call
/// can carry, divided by this.
///
/// Measured for 1.16 on the three current Anthropic models, from the calls that were answered
/// whole (`output_tokens` less the thinking, over the passage's `count_tokens`):
///
/// | passage | passage tokens | model, effort | answer tokens | ratio |
/// |---|---|---|---|---|
/// | the SM-P14 fixture, three sentences | 91 | Haiku `low` | 419 | 4.6 |
/// | | | Sonnet `low` / `medium` / `high` | 516 / 618 / 647 | 5.7 / 6.8 / 7.1 |
/// | | | Opus `low` / `medium` / `high` | 542 / 548 / 607 | 6.0 / 6.0 / 6.7 |
/// | an incident report, five paragraphs | 380 | Sonnet `high` | 1 229 | 3.2 |
/// | a Rust function with a comment | 324 | Sonnet `low` / `medium` / `high` | 993 / 1 015 / 1 508 | 3.1 / 3.1 / 4.7 |
/// | Russian prose, four paragraphs | 259 | Sonnet `high` | 2 101 | 8.1 |
///
/// Four: the long passages, which are what a chunk is, sit at three to five; the fixture's
/// seven is the per-unit overhead on a passage of three sentences, and the 256-token floor is
/// what carries those. The Russian answer, at eight with its repair turns, is the residual —
/// Cyrillic is dear in the output as it is cheap in the input.
pub const ANSWER_PER_INPUT_TOKEN: usize = 4;
/// How much of `max_output` to leave for the model's reasoning, on a provider whose reasoning
/// is spent from it. A quarter. Measured across the matrix for 1.16 — three models, three
/// efforts, four passages, at 800 and at 2,048 — on the calls whose limit held the answer:
/// Sonnet and Opus spent nothing at `low` and `medium` and 11% to 16% at `high`; Haiku spent
/// 59% at `high`. The quarter holds for two of the three models at every effort and is too
/// small only for the smallest at `high`, where no fraction would do — that is what
/// `SMY-W455` is for. Where the limit could not hold the answer at all, Haiku spent all of it
/// thinking at every effort, so an unanswered call on Haiku is first a sign of an undersized
/// limit, and the chunk is sized to the answer ([`ANSWER_PER_INPUT_TOKEN`]) before the
/// headroom is a question.
pub const REASONING_HEADROOM_RATIO: f32 = 0.25;

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
    /// The window a provider's context leaves for input text (§22.2), capped at the passage
    /// whose answer the call can carry.
    ///
    /// Two bounds, and the tighter one wins. The context bound is what is left after the
    /// prompt and the output reserve. The output bound is the answer the call can carry —
    /// `max_output`, less the reasoning headroom where that applies — divided by
    /// [`ANSWER_PER_INPUT_TOKEN`], since a chunk whose answer the limit cannot hold is a
    /// chunk truncated and degraded whole. Before 1.15 only the first bound existed, and a
    /// provider with a million-token context chunked a book into one piece whose 8,192-token
    /// answer could not hold it; 1.15 capped the chunk at `max_output` itself, as if an
    /// answer were the size of its passage, and 1.16 measured that it is four times the size.
    ///
    /// `reasoning_in_output` is the provider's `Capabilities` field of that name: where the
    /// model's reasoning is spent from `max_output` too, the answer gets what is left after
    /// [`REASONING_HEADROOM_RATIO`], and the chunk is sized to that.
    pub fn for_context(
        context_window: usize,
        max_output: usize,
        reasoning_in_output: bool,
    ) -> Window {
        let answer = if reasoning_in_output {
            max_output - ((max_output as f32) * REASONING_HEADROOM_RATIO) as usize
        } else {
            max_output
        };
        let output_reserve =
            max_output.max(((context_window as f32) * OUTPUT_RESERVE_RATIO) as usize);
        let budget = context_window
            .saturating_sub(PROMPT_RESERVE)
            .saturating_sub(output_reserve)
            .min(answer / ANSWER_PER_INPUT_TOKEN);
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

    // Group paragraphs greedily into windows. The separator the group is rejoined with costs
    // tokens too — two, in the sizing estimate, where `ceil(bytes / 4)`'s slack used to hide
    // it — so it is counted with every paragraph after the first, and a chunk measures no
    // more than its budget.
    let separator = tokens("\n\n");
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut used = 0u32;

    for (i, (_, _, text)) in paragraphs.iter().enumerate() {
        let cost = tokens(text);
        if current.is_empty() {
            used = cost;
        } else if used + separator + cost > w.budget {
            groups.push(std::mem::take(&mut current));
            used = cost;
        } else {
            used += separator + cost;
        }
        current.push(i);
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
        // finer to split on, and progress matters more than a tidy boundary. The cut is
        // where the sizing estimate says the budget is, a character at a time — until 1.16
        // it was `budget * 4` bytes, the format count's premise, and a line of letters at
        // three bytes a token came out a third over.
        if tokens(line) > budget {
            let mut at = 0usize;
            let mut tally = Sizing::default();
            for (i, ch) in line.char_indices() {
                let mut next = tally;
                next.add(ch);
                if i > at && next.tokens() > budget {
                    out.push((cursor + at, cursor + i, line[at..i].to_string()));
                    at = i;
                    tally = Sizing::default();
                    tally.add(ch);
                } else {
                    tally = next;
                }
            }
            if at < line.len() {
                out.push((cursor + at, cursor + line.len(), line[at..].to_string()));
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
        // the `\n\n` the pieces are rejoined with — which costs what the sizing estimate says
        // it does, and is why this is not exactly `2 * budget`.
        let with_overlap = Window::of(50);
        for c in chunk(&para, with_overlap) {
            assert!(
                c.tokens() <= 2 * with_overlap.budget + tokens("\n\n"),
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
        let w = Window::for_context(8192, 2048, false);
        assert!(w.budget < 8192 - PROMPT_RESERVE as u32);
        assert!(w.budget > 0);
    }

    /// Ingest output is roughly the size of its input, so the output reserve is not a small
    /// fraction - a window that ignored it would produce chunks whose answers cannot fit.
    #[test]
    fn the_output_reserve_is_at_least_half_the_context() {
        let w = Window::for_context(10_000, 100, false);
        assert!(w.budget <= 10_000 - 5_000, "budget {}", w.budget);
    }

    /// A chunk is never wider than the answer it can get. With a million-token context and
    /// an 8,192-token output, the context bound alone would allow a chunk of ~490k tokens,
    /// whose answer could not fit the call; the output bound holds it to 8,192.
    #[test]
    fn a_chunk_is_never_wider_than_the_answer_the_call_can_carry() {
        let w = Window::for_context(1_000_000, 8192, false);
        assert_eq!(w.budget, 8192 / ANSWER_PER_INPUT_TOKEN as u32);
        // Where the context is the tighter bound, the cap changes nothing: 4096 − 1024 −
        // max(1024, 2048) is 1024, and so is `max_output`.
        // 1,024 of answer carries 256 of passage, which is the floor exactly.
        let w = Window::for_context(4096, 1024, false);
        assert_eq!(w.budget, 256);
        // The floor still applies when `max_output` is below it.
        let w = Window::for_context(100_000, 100, false);
        assert_eq!(w.budget, 256);
    }

    /// Where the reasoning is spent from the same limit, the answer gets what the headroom
    /// leaves, and the chunk is sized to that.
    #[test]
    fn a_provider_that_thinks_from_the_output_gets_a_narrower_chunk() {
        assert_eq!(Window::for_context(1_000_000, 8192, false).budget, 2048);
        assert_eq!(Window::for_context(1_000_000, 8192, true).budget, 1536);
        // The floor still holds.
        assert_eq!(Window::for_context(100_000, 100, true).budget, 256);
    }

    /// A window that reserved itself out of existence would chunk into nothing.
    #[test]
    fn a_tiny_context_still_leaves_a_usable_window() {
        let w = Window::for_context(512, 512, false);
        assert!(w.budget >= 256);
        assert!(!chunk(&para(3), w).is_empty());
    }

    #[test]
    fn a_zero_budget_is_clamped_rather_than_dividing_by_zero() {
        assert!(Window::of(0).budget >= 1);
        assert!(!chunk("text", Window::of(0)).is_empty());
    }

    #[test]
    fn the_sizing_estimate_counts_punctuation_dearer_than_letters() {
        // 30 letters at three per token are 10; 30 marks at one per token are 30.
        assert_eq!(sizing_tokens("abcdefghijklmnopqrstuvwxyzabcd"), 10);
        assert_eq!(sizing_tokens("!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!"), 30);
        assert_eq!(sizing_tokens("1234567890"), 10);
    }

    #[test]
    fn the_sizing_estimate_knows_cyrillic_from_other_two_byte_scripts() {
        // Eight two-byte letters: Cyrillic at four bytes per token is 4; Greek at two is 8.
        assert_eq!(sizing_tokens("абвгдежз"), 4);
        assert_eq!(sizing_tokens("αβγδεζηθ"), 8);
        // Five three-byte characters, 15 bytes at two and a half per token: 6.
        assert_eq!(sizing_tokens("中文分词器"), 6);
    }

    #[test]
    fn the_sizing_estimate_counts_accented_latin_dearer_than_english() {
        // Thirty-six letters: English at three bytes a token is 12. With one umlaut among them
        // the text is not English to the tokeniser, which spends a token per 2.25 bytes of its
        // letters: 16, and the umlaut's own two bytes make 17.
        let latin = "abcdefghijklmnopqrstuvwxyzabcdefghij";
        assert_eq!(sizing_tokens(latin), 12);
        assert_eq!(sizing_tokens(&format!("{latin}\u{fc}")), 17);
        // One accent in three hundred letters is under the one-in-a-hundred mark: 100 and 1.
        assert_eq!(sizing_tokens(&format!("{}\u{fc}", "a".repeat(300))), 101);
        // Cyrillic beside Latin is a mixed text, not an accented one: 12 and 4.
        assert_eq!(sizing_tokens(&format!("{latin}абвгдежз")), 16);
    }

    #[test]
    fn the_sizing_estimate_is_above_the_format_count_on_latin_prose() {
        // The format's count is ceil(bytes / 4); on prose the tokeniser is nearer three
        // bytes a token, and the sizing estimate sits above the count, never below it.
        let prose = "On Thursday the eu-west shard slowed: p95 request latency rose from 180ms \
                     to 410ms. Connection pool wait time rose alongside it.";
        assert!(sizing_tokens(prose) > smysl_core::tokens(prose));
        assert_eq!(sizing_tokens(""), 0);
    }
}
