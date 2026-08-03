# What "production ready" would mean

**Status:** a checklist, not a promise. Updated when something on it moves.

smysl was unpublished for eight releases, and the reason given each time was "not production
ready" — a true answer and a useless one, because nothing said what would change it, so it
could only be deferred and never worked toward. This file was written to say what the gate is.

**Published as of 0.9.0**, with four of these seven items still open. That is not the list being
abandoned. Gate 2 was the one that could not be worked around: an interchange format nobody
outside the project has implemented is a file layout, and publishing it would have meant
publishing a claim. It is closed three times over. What remains is coverage and polish, and
0.x is the honest signal for that — the version says the surface can still move, and gate 3
says exactly where.

Nothing here is a schedule. The point is that each item is either done, or has a next action
that someone could take.

---

## 1. Format stability — *close*

`smysl/0.1` has not changed across seven crate releases. Record type 10 was *added* in 0.2
without a format bump, which rule X is precisely for, and one decoder became stricter in 0.5
about records it should never have accepted. No encoder's output has moved.

That is a good record, and it is not yet a commitment. What is missing is a written policy:
what constitutes a format break, what the deprecation path is, and whether `smysl/0.1` is
frozen or merely stable-so-far.

**Next action:** a versioning section in `SMYSL_FORMAT_SPEC.md` saying which changes are
allowed within a format version and which require a new one.

## 2. A second implementation — *done, including the claim that mattered*

The whole proposition is that two implementations agree on what a document says. That has
never been tested by anything except this one. `SMYSL_FORMAT_SPEC.md` exists and is under 250
lines specifically so that it could be, but nobody has written against it.

Until someone does, "another team can implement this" is a claim, not a fact. For an
interchange format that is the difference between a product and a file layout.

**Done for C-Read, in 0.9.0 — twice.** `python/` and `nodejs/` each hold an independent
implementation written from the spec alone, with no dependencies, and each decodes and
re-encodes every fixture in `fixtures/wire/` byte for byte. Both run in CI.

Three as of the same cycle: `go/` is a fourth reading, and the first written against the
*revised* spec — so it tests the clarifications as well as the format. It needed no guesses
where the earlier two did, which is the outcome those clauses were written for.

More than one on purpose: implementations that agree could have made the same guess
where the document is silent, so agreement is only evidence when the readings were
independent. The JavaScript was written without consulting the Python, and both arrived at the
same two ambiguities — which is the result worth having.

It found three places where the spec is insufficient, all marked in the Python source:
constraint 2 (shortest form) does not say it applies to integers and lengths rather than float
payloads; major type 6 (tags) is not mentioned at all; and constraint 1 says text keys are
"permitted only inside a payload" without saying what a decoder must do on meeting one at
kernel level. None is a defect in the Rust; all three are places a second implementer must
guess.

The suite also records what C-Read *cannot* reach, and the largest entry is §2.3 — status is
part of identity — because uids need C-Produce. The format's central claim is still untested
by a second implementation.

All three clarifications are folded into §3 of the spec as of 0.9.0: constraint 1 gained the
decoder's obligation, constraint 2 is scoped to integers and lengths, and tags are constraint
8. The Go implementation is the check — written against the revised text, it needed no guesses
in any of the three.

**C-Produce reached in 0.10.0, in `python/`.** This is the item that mattered. C-Read never
touches uid derivation — reading a document does not require computing one — so three
independent readers round-tripped every fixture byte for byte while remaining ignorant of what
a uid *is*, and §2.3, *status is part of identity*, stayed verified by the Rust alone across
nine releases.

`python/smysl/uid.py` lays out a unit core in canonical form and hashes it with a BLAKE3
hand-rolled in `python/smysl/blake3.py`. Hand-rolled deliberately: a binding to the same C
library the Rust uses would have tested two callers of one implementation. It passes the
published BLAKE3 vectors, including the multi-chunk lengths that a single-chunk shortcut would
get wrong, and reproduces all sixteen uids in `fixtures/wire/uid/cases.json` — canonical bytes
checked separately from the hash, so a disagreement says which half broke.

The §2.3 witness is a pair whose every field is identical and whose status differs: one byte
apart in the canonical encoding, two unrelated uids. Verified capable of failing — dropping
`status` from the encoder fails 35 tests, by name.

**Next action:** none for this gate. `nodejs/` and `go/` remain C-Read, which is a scope
decision rather than a gap; both say so where they list what they do not reach.

## 3. Public API stability — *not ready*

The facade's surface still churns. `Hybrid` changed shape twice inside 0.7.0, and
`smysl-retrieve`'s trait is one cycle old. Publishing pins every name permanently, and
semver on a 0.x crate lets you break things, but the point of publishing is that people build
on it.

Published as of 0.9.0, which converts this from a precaution into a debt: every name is now
something people can build against, and the cost of the pass grows with each release it waits.

**Next action:** a pass over the facade's `pub use` list asking, of each name, whether it is
part of the contract or an implementation detail that escaped. Then `#[non_exhaustive]` where
that answer is "we will want to add to this" — 110 types already carry it, out of 378 public
ones across the crates, and nobody has checked that the split is deliberate.

Mechanise it rather than trusting a reading: `cargo public-api` emits the reachable surface,
which belongs in a golden file the way `golden-packs.txt` records what the packer selects, and
`cargo-semver-checks` turns an accidental break into a failing job. This project has learned
that a rule nothing enforces is a rule that drifts.

## 4. Verified providers — *partial*

| provider | live-tested | note |
|---|---|---|
| ollama | yes | no key needed |
| deepseek | yes | `json-mode`; degrades under rule I as designed |
| gemini | yes | `json-schema`; clean |
| openai | **no** | its one identified defect is fixed and tested; acceptance unconfirmed |
| anthropic | **no** | `ToolForce`; no counted defect yet, and none looked for |

**Next action:** read Anthropic's mapper against the documentation the way OpenAI's was. That
found a real defect without a key, and it is the cheapest remaining move.

## 5. A test suite that catches what it claims — *measuring, and now cross-checked*

Seven defects across 0.4–0.7 were the same shape: a check that passed without covering what it
claimed. Two fuzz-generator vacuities, an exact-pack gate that never reached L2, a decoder
sweep that never entered the decoder it was written for, a repair of that sweep that was
itself vacuous, a doc-output regex that skipped the transcripts it was handed, and a routing
test that passed on routing measurably worse than not routing at all.

0.8 measured it: mutation testing over the packer core — the best-tested file in the project —
left 49% of viable mutants alive, and two oracles turned out to be replaceable by a stub
without a single test noticing.

0.10 added a defect of the same shape but a different *kind*, and it is the most instructive
one yet. `skip_item` had a comment asserting that unknown payloads are "parsed strictly, so an
unknown record cannot smuggle in a non-deterministic encoding". It was false, and had been
since before the comment was written. No test was vacuous; no oracle was stubbed. The check
that would have caught it **did not exist**, because the property was asserted in a comment
instead. A shared rejection corpus found it in an hour.

The lesson generalises past this project: a claim written in prose next to the code is not
weaker evidence than a test, it is *no* evidence, and it reads exactly like evidence.

**Next action:** targeted mutation testing of `smysl-core` codec invariants — everything
downstream trusts round-tripping. And, cheaper, a sweep for load-bearing claims made only in
comments.

## 6. Performance characterised — *done, and it found one*

`pack` is linear when the budget binds as of 0.6 (was quadratic), `salience` is linear and
measured in isolation. `merge` and `check` were the last two measured only *through the
command*, where parsing dominates and the ratio is a ratio about parsing.

Measured in 0.10. `merge` was linear as assumed. **`check` was not** — 3.47x per doubling, and
the per-pass breakdown put it in `integrity` at 3.84x while every other pass sat at 2.0.

The cause was three lines in `topo`: the ready set was sorted on every iteration and then
popped with `remove(0)`, which is two quadratic factors stacked. A min-heap pops in the same
ascending dense-id order — the order rule D requires — in log time. `check` at 16 000 units
went from 40.2 ms to 6.6 ms and the curve straightened to 2.16x. The `integrity` pass alone
went from 8.62 ms to 0.45 ms at 8 000.

Three of the four operations in the pure set were assumed linear and two of them were not.
That is the argument for measuring rather than reasoning, and it is now the third time this
project has made it.

**Next action:** none outstanding. The scaling tests are `#[ignore]`d measurements rather than
gates, deliberately — timing assertions on shared runners cry wolf — so the standing cost is
that somebody has to run them. Worth running at a release cut.

## 7. Documentation that matches the binary — *partial*

`make doc-output` replays 46 of 168 documented command blocks and gates on them. The other 122
are skipped, mostly because they name files a chapter built earlier in its own narrative.
Appendix A, the purity table and the diagnostic registry are cross-checked against the code at
every release cut.

The manual has been wrong twice in ways that mattered: it described `ui` as a stub for
several releases, and it stated a body-line limitation that had been fixed.

The API documentation was never checked at all until 0.10, and publishing made that visible:
docs.rs was rendering a partial crate, because no manifest set `all-features` and the
feature-gated half simply was not there. Six rustdoc warnings had accumulated, one of which
told an implementor outside the crate to build a `Usage` through a constructor that does not
exist. `make doc-gate` and a CI job now run rustdoc with `-D warnings`, confirmed to fail
against a deliberate broken link before being trusted.

**Next action:** teach `verify-doc-output.py` to build a chapter's intermediate files, which
would roughly triple coverage of the manual.

---

## What is deliberately *not* on this list

**Feature completeness.** The command surface is twenty-two commands and has not needed a new
one since `find` in 0.5. Readiness is not about having more.

**A single crate.** Abandoned in 0.7.0 — the only benefit was publishing one crate rather than
eleven, and it would have cost the crate boundary as a compiler-enforced constraint. Eleven
crates or none.

**Performance beyond linear.** Nothing in the pure set is worse than linear in store size at
the sizes anyone has. Faster is not readier.
