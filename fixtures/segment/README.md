# Sentence-boundary gold sets

`gold/<lang>.txt` is a passage of prose with a **`‖` (U+2016) immediately after the last
character of every sentence**. Nothing else is markup: the whitespace, the line wraps and the
blank lines are part of the text and are measured as such. `crates/smysl-text/tests/segment_f1.rs`
strips the markers, remembers where they were, and scores
`smysl/seg-uax29+abbr/1` against them.

    Mr. Alder read it twice.‖ "There is no
    money," he said.‖

    She had expected it.‖

Two sentences, then a paragraph break, then one more. The second sentence spans a line wrap,
which is one of the four things the segmenter has a rule for — abbreviations, soft wraps,
lowercase continuations and closing marks — and therefore one of the things a gold set has to
contain.

## Running it against another corpus

    SMYSL_SEG_GOLD=/path/to/dir cargo test -p smysl-text --test segment_f1 -- --nocapture

The directory needs one `<lang>.txt` per language in `segment::LANGUAGES`, in this format. No
code changes: this is the exit test, pointed at a different corpus.

## What is here, and what is missing

TX-P2 step 1's exit asks for **F1 ≥ 0.97** (en, es, fr, de) and **≥ 0.95** (ru) over **500
sentences per language**. What is in `gold/` is ~30 sentences per language, and it is a
**development set**, not that corpus:

* it was written here, alongside the segmenter, so it contains the cases its author thought
  of — and two list rows and one algorithm rule were added *because* it failed on them, which
  is fitting to the set, not evidence against an unseen one;
* 30 sentences cannot distinguish 0.97 from 1.00. Four of the five languages score 1.000 on
  it, which says the obvious errors are gone and nothing more than that.

The 500-sentence gold set is an outstanding corpus item, recorded on GE-T1's row in
SMYSL-2.4 §6 beside the Bibles and the JSON series. It needs prose nobody here wrote and
boundaries annotated by someone who is not the author of the segmenter.

The passages in `gold/` are original, written for this repository, and carry its licence.
