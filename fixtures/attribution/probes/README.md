# TX-P5 step 3b's probe set

What the clause asks (SMYSL-2.4 §6 TX-P5 step 3b): attribution-bearing passages with known
reported speech, showing the **asserted-as-reported rate below 10%** — the threshold the S0
spike's T8 fired on at 11.1–24.5%. `crates/smysl-ingest/tests/probe_set.rs` measures it.

## Where the files came from

Mark 14–15 in two editions, from eBible.org's USFM distributions, taken 2026-10-15:

| directory | source | licence |
|---|---|---|
| `eng-kjv/` | `https://ebible.org/Scriptures/eng-kjv_usfm.zip`, `71-MRKeng-kjv.usfm` | public domain; the 1769 standardised text, via the Crosswire Bible Society and eBible.org. Letters patent restrict *printing* in the United Kingdom and have no effect elsewhere |
| `engwebp/` | `https://ebible.org/Scriptures/engwebp_usfm.zip`, `71-MRKengwebp.usfm` | public domain; "World English Bible" is a trademark of eBible.org, and this text is unaltered |

Note that this is **not** where `library/bibles/slice/` came from. That slice was built by
`vpl2usfm.py` out of eBible's verse-per-line distribution, which carries no markup at all — which
is why its `eng-kjv/` has no `\wj` in it, and why a probe set could not be drawn from it.

Each file is whole chapters, cut out of the book with every byte inside a chapter left as the
source wrote it: `\w ...|strong="G0000"\w*` word markup, `\f ... \f*` footnotes, `\q1` poetry and
all. The reader strips what it does not keep and reports `lossy`, which is the behaviour a probe
set should be measuring rather than a thing to tidy away first.

## What was chosen, and what was not

The passages are **Mark 15, every utterance in the chapter**, in both editions — one probe per
utterance, at the verse it begins in, so an utterance that runs on into the next verse is probed
on the part inside its own. Picking which
clauses to probe is the one authored judgement in a probe set, and the way to keep it honest is
not to make it: a chapter is taken whole, so a detector cannot have been flattered by a choice of
verses. Mark 15 was taken because the Passion narrative is where several named holders speak in
turn — Pilate, the chief priests, the soldiers, the crowd, the passers-by, a bystander, the
centurion, and a verse of scripture quoted as if it spoke — so one chapter covers ten speakers,
and one edition's red-letter markup covers exactly one of them.

**Chapter 14 is carried and not probed.** It is here so that chapter 15's probes are read out of a
*second* part, whose offsets are not the text's offsets. That is not hypothetical tidiness: it is
the defect this material found on its first run (`smysl_text::part::local_rows` shifted each row
to the part's own offsets and left the marks inside the row at the whole text's, so chapter 15's
marks pointed past the end of chapter 15). Every reader fixture in the tree before this one
produced exactly one part, and for one part the shift is zero.

The labels are two facts and no opinions:

- `expected` is `reported` where the verse frames the clause with a **verb of saying** attributing
  it to somebody, and `none` where it does not. So `And the scripture was fulfilled, which saith,
  And he was numbered with the transgressors` is reported speech, and `which is, being
  interpreted, The place of a skull` is not — a gloss on a name is not a saying, and neither is a
  superscription that `was written over`. Three probes are labelled `none` for that reason, and
  they are the ones that measure the opposite error from T8's.
- `marking` is how the source file marks the clause: `wj`, `quoted`, `both`, or `bare`. Anyone can
  check it by opening the file.

## What it measures, and what it reports instead of measuring

Only `wj`, `quoted` and `both` probes are in the **gated** denominator. A clause its own edition
leaves to a comma and a capital letter is counted and reported, not passed and not failed — the
rule GE-T13 settled on for a date pair open on the side being compared, and for the same reason: a
run of clauses nothing can see reads exactly like a run of clauses the detector missed, and
folding them in would make the figure a figure about typography.

That split is the whole finding of this material, and the two arms state it:

- `engwebp` punctuates every quotation and marks the words of Jesus, so its 18 probes are nearly
  all gated.
- `eng-kjv` is the 1769 text, which marks the words of Jesus and punctuates **nothing**: 2 of its
  18 probes are gated and 16 are the typography arm, reported at whatever rate they come out at.

The two editions carry the same 47 verses, so the arms differ in nothing but how the speech is
marked.

## Running it against other material

`SMYSL_PROBE_SET=<dir>` replaces this directory with somebody else's and needs no code change. The
format is `probes.tsv` plus the files it names; see the header of `probe_set.rs` for the columns
and for how a plain-text passage (one with no reader, and no markup to read) is handled.
