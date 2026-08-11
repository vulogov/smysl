# Tutorial files, committed so the manual's commands can be replayed

Gate 7's larger half. `make doc-output` replays the manual's transcripts against the real
binary, and 100 of its commands name a file the *prose* asks the reader to create —
`smysl check cycle.smy` — so there was nothing to run them against. These are those files.

**One rule holds the arrangement together: a fixture here must be the same bytes the chapter
prints.** `verify-doc-output.py` checks it, and fails if a fixture is not found verbatim in its
chapter. Without that, editing the page leaves the fixture behind and the script starts
measuring its own copy rather than the book — a check that passes against bytes no reader ever
sees.

Nothing in the manual changed to make this work. The page still says `smysl check cycle.smy`;
the command runs with its working directory set to a scratch copy of the folder holding
`cycle.smy`, so the diagnostics still print `cycle.smy: error: …` and the transcript compares
exactly as written. Substituting a path into the command would have changed its output and
made every one of these transcripts drift on its own filename.

## A chapter is a sequence, not a set

1.1 committed twelve files and left 57 commands unreachable, recorded as a decision about the
book: 22 filenames have **more than one state** — chapter 1 has the reader create `first.smy`
broken, fix it, find it unformatted, then rewrite it in place — so four commands name one path
and expect four different files, and committing any one makes the other three report drift
that is not there. Splitting them would mean the chapters naming a different file at each step,
which changes what they teach.

**It was not a decision about the book.** It was these fixtures being keyed by filename when a
chapter's state is keyed by *position*. And two of chapter 1's four states cannot be committed
at all, which is what makes the point: state 2 is printed as a **fragment** the reader pastes
in, and state 3 as a **diff**. Neither is ever printed as a file, so neither could satisfy the
rule above even if somebody wanted it to.

So they are not committed. Since 1.2.0 the script replays a chapter *as a chapter*: a scratch
copy of the committed files, walked in document order, with `fmt --write` allowed to actually
write. A later state is **derived rather than recorded**, which is a stronger claim than a
fixture makes — the file a command runs against is one the book's own instructions produced.

The scratch copy is what makes writing safe. Commands used to run in this folder directly; a
single replayed `fmt --write` would have rewritten a tracked fixture on every run.

## `edits.json` — the reader's own edits

A hand edit is the one thing replaying a chapter cannot produce, so each chapter that needs one
carries an `edits.json`. It is **anchored by prose, never by content**: the edit body is the
fenced block that follows the anchor, read out of the chapter at run time.

```json
"step2.smy": [
  { "anchor": "Add the claim the evidence is for:", "op": "append", "from": "step1.smy" }
]
```

Nothing is duplicated, so nothing can go stale — change the page and the anchor stops
resolving, which is an error rather than a silent pass. An anchor must appear **exactly once**;
appearing twice fails as loudly as appearing never, because an ambiguous anchor would quietly
pick whichever came first.

Three operations, and `from` for the chapters that build one document up under several names:

| op | what it does |
|---|---|
| `append` | add the anchored block to the end of the file |
| `replace-stanza` | replace the `@…` stanza named by `match` with the anchored block |
| `from` | seed the file from an earlier one before applying — `step1` → `step2` → `step3` |

Keys beginning `_` are prose. JSON has no comments and these files have to explain themselves.

## Chapter 4 is exhausted: 17 of its 20 commands

Working through it turned up three shapes, only one of which needed an `edits.json` chain:

- **`step2.smy` / `step3.smy`** — the chain. One document under three names, each later state
  printed only as the stanza that was added.
- **`checkout.smy`** — needed nothing at all. The chapter prints it in full *after* the command
  ("The complete file, for reference"), and the first attempt at extracting it looked only at
  the block *before*. Worth stating because it was the assumption, not the mechanism, that was
  wrong.
- **`ticket.smy`** — the chapter prints only `fmt`'s *output*, never the input. That output is a
  fixed point, so it is committed as the file, and it exercises precisely what the section is
  about. §4 requires a value that would re-parse as something else to be quoted, and `ref: 42`
  unquoted does not re-parse as a string at all — it is read as a number, leaving the source
  with no `ref`, which is `SMY-E001`. So a writer that stopped quoting would not merely print
  something different; it would fail `fmt`'s own reparse assertion and refuse to write, which
  is the guard the surrounding pages describe.

## What is not here, and why

**`batch-a.smy` / `batch-b.smy` — and this one would be a check that cannot fail.** They are
described rather than printed ("one with a multi-line header and a quoted date, the other with
`grounds` and `status` swapped"), and the expected output is **empty**, because `fmt --write`
prints nothing on success. Any two valid files would satisfy it. Skipping is right here for a
stronger reason than "the bytes are not available".

**`extrel.smy`** — 7 records, 3 units, and the chapter prints only the one `@rel` line that
makes it interesting. The three units are never printed as a set.

**A file the prose describes rather than prints.** `bignote.txt` is "a ~7 KB paragraph,
repeated". There is nothing to copy, so there is nothing to commit that is provably the same
bytes.

**`draft.smy`, and this is the second instance of a pattern.** The chapter prints the snippet
that demonstrates comments, then shows `fmt --write draft.smy` warning that two comment lines
will not survive. Run against the printed bytes, `fmt` never reaches that warning: the snippet's
`@claim c/regression` has `grounds: [e/trace]` and nothing defines `e/trace`, so it exits 3 with
`SMY-E060` and `SMY-E031`. That transcript was generated from a fuller `draft.smy` than the page
prints — the same shape as `beta.smy` in chapter 29, now seen twice.

A reader following the page literally gets two errors where the book shows a warning. Held back
rather than repaired for the same reason: the missing stanza is not on the page, and inventing
one is not a repair.

**`beta.smy` in chapter 29, and this one is a finding rather than a limitation.** The chapter
prints it in full and its transcript claims `14 records, 5 units`. The document it prints has
**four** units, and `check` on it says `12 records, 4 units` — exactly one labelled unit fewer,
which is two records. The transcripts in that chapter were generated from a `beta.smy` that is
not the one the book prints, and the difference propagates through seven transcripts, including
the contention labels in `merge` and **a uid the reader is told to type** into `retract`.

`alpha.smy` beside it is in sync: its transcript matches, contention uid included. So this is
one document that lost a stanza, not a chapter that drifted.

It is held back rather than fixed because the missing stanza cannot be reconstructed from the
page, and guessing one into a published chapter is not a repair. Restoring it makes seven more
commands replayable at once, which is the largest single block left.

## Adding one

Copy the block its chapter prints, run `make doc-output`, and keep it only if `ran` rises and
nothing mismatches. If the command fails on unresolved references, the block was a fragment —
that is the common case, and `edits.json` is the answer rather than a bigger copy. The
diagnostics carry byte offsets, so a file assembled wrongly reports `at 0..125` where the manual
says `at 296..416`, and the mismatch says exactly what happened.
