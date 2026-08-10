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
the command runs with its working directory set to the folder holding `cycle.smy`, so the
diagnostics still print `cycle.smy: error: …` and the transcript compares exactly as written.
Substituting a path into the command would have changed its output and made every one of these
transcripts drift on its own filename.

## What is not here, and why

Twelve files of the forty-six the manual names. The rest are held back for two reasons, both of
which produce *false* drift if ignored — and this script's history says a false positive is
worse than no check.

**A filename with more than one state.** Chapter 1 has the reader create `first.smy` broken,
fix it, discover it is not canonically formatted, and then rewrite it in place with
`fmt --write`. Four commands name one path and expect four different files. Committing any one
of them makes the other three report drift that is not there. 22 files and 57 commands are in
this position; splitting them would mean the chapters naming a different file at each step,
which changes what the book teaches — a decision about the book, still open.

**A file the prose describes rather than prints.** `bignote.txt` is "a ~7 KB paragraph,
repeated". There is nothing to copy, so there is nothing to commit that is provably the same
bytes.

Seven more were extracted and then removed because the chapter's own transcript refused to
reproduce against them — the fenced block before the command turned out to be a *fragment* the
reader adds, not the file. That failure is loud rather than subtle, which is the useful part:
the diagnostics carry byte offsets, so a file assembled wrongly reports `at 0..125` where the
manual says `at 296..416` and the mismatch says exactly what happened.

## Regenerating

There is no generator. Each file was taken from the fenced block its chapter prints and kept
only when the chapter's transcript reproduced against it. Adding one is the same procedure:
copy the block, run `make doc-output`, and keep it only if the count rises and nothing
mismatches.
