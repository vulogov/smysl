# What "production ready" would mean

**Status:** a checklist, not a promise. Updated when something on it moves.

smysl was unpublished for eight releases, and the reason given each time was "not production
ready" — a true answer and a useless one, because nothing said what would change it, so it
could only be deferred and never worked toward. This file was written to say what the gate is.

**Published as of 0.9.0**, with four of these seven items still open. That is not the list
being abandoned. Gate 2 was the one that could not be worked around: an interchange format
nobody outside the project has implemented is a file layout, and publishing it would have
meant publishing a claim. It is closed three times over. What remains is coverage and polish,
and 0.x is the honest signal for that — the version says the surface can still move, and gate
3 says exactly where.

**0.10.0 is released and not published**, which is a state worth naming because it is easy to
read a tag as a publication. `cargo-semver-checks` fetches its baseline from crates.io, so
`BASELINE` in the Makefile tracks the last *published* version rather than the last tagged
one; pointing it at 0.10.0 turns all twelve crates into "version not found in registry". The
consequence that bites is that a breaking change is still measured against 0.9.0, so a repair
wanting the public surface to move stays parked until 0.10.0 goes up.

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

The whole proposition is that two implementations agree on what a document says, and for eight
releases that had been tested by nothing except the implementation that wrote the document.
"Another team can implement this" was a claim rather than a fact, and for an interchange format
that is the difference between a product and a file layout.

`SMYSL_FORMAT_SPEC.md` is short so that it could be read in an afternoon — under four hundred
lines, having grown from 250 as implementers found it silent in places. That growth is the
point rather than a regression: every added clause is somewhere a reader had to guess.

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

Anthropic's mapper was read against the documentation in 0.10, the way OpenAI's was, and it
found one: `caps()` declared `streaming: true` while the mapper implements no `stream`, so it
inherited the trait default, which refuses. Gemini had the same defect — and Gemini is
live-tested, which means the live test never exercised streaming either.

The rest of the mapper reads correctly against Anthropic's documentation: `x-api-key` rather
than a bearer token, the `anthropic-version` header, `system` as a top-level field, a forced
`tool_choice`, the block-list response with `tool_use.input`, and Anthropic's own
`usage.input_tokens` names rather than OpenAI's.

That is now twice this method has found a defect without a key. What it still cannot answer for
either provider is whether the endpoint *accepts* the translated schema.

**Next action:** unchanged — a key, for OpenAI and Anthropic both. Everything reachable by
reading has been read.

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

Both next actions ran in 0.10, and both produced findings.

The **comment sweep** — 471 comments make a modal claim; narrowing to claims of
*comprehensiveness* left 178; the three most load-bearing gave two real corrections and one
clean result. `Secret`'s "a key never reaches a `Debug` output" was genuinely covered, which is
recorded here because a sweep that reports only hits is the failure it is looking for.

**Mutation testing of the codec** — 143 viable mutants, **33 survivors, 23%**, against the
packer's 49% in 0.8. Most survivors are equivalent mutants; three were real gaps and are
closed. The most instructive is that the *map* arm of `skip_one` could stop bounding its depth
with nothing failing, because the nesting fixture nested arrays — a bound tested on one shape
and decorative on the other.

Both ran in 0.10 as well, and the numbers are now three points on one curve rather than one:

| target | viable mutants | survivors |
|---|---|---|
| `smysl-pack` core (0.8) | — | **49%** |
| `cbor/reader.rs` + `writer.rs` | 143 | **23%** |
| `cbor/envelope.rs` | 115 | **2.6%** |
| `smysl-check` (0.11) | 143 | **9.1%** |
| `smysl-graph` (0.11) | 625 | **15.0%** |

**Read these as "does this crate's own suite cover it", not "is this covered".** `cargo mutants
-p X` runs `cargo test --package=X`, so a function only exercised by a downstream crate is
reported as a survivor while being perfectly well tested. `Store::matching_prefix` is exactly
that — replacing it with `vec![]` survives `smysl-graph`'s suite and fails two tests in
`smysl-check`'s.

The distinction was not noticed for four measurements, and every figure above was quoted as if
it meant the stronger thing. It is still a number worth having: a crate ought to test its own
API, and one that leans on a consumer's tests has a coverage hole that moves the moment the
consumer changes. But it is the weaker claim, and the difference has to be measured rather than
assumed — `--test-workspace` gives the stronger one at the cost of running the whole suite per
mutant, which is a day's work at these counts and so is worth spending only on the survivors
that would otherwise be acted on.

`envelope.rs` is the best-covered code in the project, and the two survivors that mattered are
worth more than the rate. An attestation's `sig` could stop decoding and be preserved as an
unknown key — invisible in the bytes and in the uid, and read as *unsigned* by anything that
later verifies. And `l0_max` could stop decoding, which
`every_granularity_preset_round_trips` looks like it covers: **all three presets carry
`l0_max: 30`**, so the loop varies everything except the field under test.

The **sweep of `smysl-graph` and `smysl-check`** found the traversal module claiming an order
`topo` does not have — "every result is a `Vec` in dense-id order", when a topological order
uses dense id only to break ties. Four traversals were claimed and two were tested; the two
that were not had tests that pass on the shape of their fixture rather than on the property.

`smysl-check` produced the most instructive result so far, and it is a **negative** one.
`support_cycles` — the pass detecting `SMY-E061`, a cycle in the support graph — can be
replaced with `()` and every test still passes. That reads exactly like the `verify -> vec![]`
oracle of 0.8, and is a different thing entirely: `EdgeSet::support()` is `{Deps, Grounds}`,
both derived
from a `UnitCore`'s own fields, and `Unit` derives its uid rather than storing one, so two units
naming each other requires solving a hash fixpoint. **No input can reach the loop.** The code is
unreachable, not untested, and no test could have been written for it.

That distinction is the reason a survivor count is not a finding. Three of the five
`smysl-check` survivors triaged this way turned out to need work; one was equivalent — a `<`
that only picks a word *inside* a branch where the two operands cannot be equal — and one was
unreachable. Reading each one is the whole job; the number is only what points at them.

Also closed there: §7's conformance table had no test at all, so all four `||` in
`ConformanceClass::forbids` could be flipped to `&&` with nothing failing. That direction of
error makes every class forbid almost nothing, which reads as "your store is fine at every
class".

`smysl-graph` was measured in 0.11: **94 survivors of 625 viable, 15.0%**, in four shards
because two unsharded attempts died around 470 mutants while sharing the machine with a build.

Its four whole-function survivors were then re-checked against the *whole workspace*, which is
the only way to tell "untested" from "untested here". Three survive that too and are real gaps:
`MergeReport::has_contentions`, which `merge --fail-on-contention` reads; `is_retracted`, which
decides whether a retraction took; and `TraceKind::follows_parents`, which picks a direction for
`trace`. The fourth, `Store::matching_prefix`, is caught by `smysl-check` and was never a gap at
all — which is the whole reason the re-check exists.

**Next action:** tests for those three. They are cheap, and each backs a command a user runs.

The comment sweep is retired as a primary instrument. Its yield fell across three crates — two
findings in `smysl-core`, one in `smysl-graph`, all three documentation rather than behaviour —
while mutation testing in the same cycles produced defects that reached identity and the wire.
Sweeping is now what one does to a claim mutation testing has already pointed at.

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

**The stated next action here was wrong, and 0.10 measured it.** It read "build a chapter's
intermediate files, which would roughly triple coverage". Only **8** of the 168 blocks name a
file an earlier *command* produced. **97** name a file the *prose* asks you to write —
`first.smy`, `missing-gist.smy` — which is a different problem.

Reconstructing those was attempted and abandoned, which is worth recording rather than
retrying. The contents are in the manual, so a chapter's directory looks rebuildable; but a
chapter shows the fix as a *fragment* ("Add the missing grounds:" and the one changed stanza),
not as the file restated. Splicing a fragment back takes guessing where it goes, and guessing
wrong makes this script report drift that is not there. A check with false positives is worse
than no check — this script has a comment saying exactly that, about fifteen phantom mismatches
that once sat in it unread. Implemented conservatively, the reconstruction ran zero commands:
every tutorial file is retired by a fragment before the commands that use it.

What it did find is that the coverage number was not deterministic. `merge … -o /tmp/x.cbor`
was required to exist before running, so a command replayed only if an earlier replay had left
its output behind: 44 blocks on a clean machine, 46 on a dirty one, with nothing changed.
Outputs are excluded from that check now, and it reports 45 either way.

**Next action:** if this is worth more, the fix is in the *manual*, not the script — commit the
tutorial files as fixtures and have the chapters include them, so what the reader copies and
what the script replays are the same bytes. That is a book change, and the book is the thing
the coverage is protecting, so it should be a deliberate decision rather than a side effect.

---

## What is deliberately *not* on this list

**Feature completeness.** The command surface is twenty-two commands and has not needed a new
one since `find` in 0.5. Readiness is not about having more.

**A single crate.** Abandoned in 0.7.0 — the only benefit was publishing one crate rather than
eleven, and it would have cost the crate boundary as a compiler-enforced constraint. Eleven
crates or none.

**Performance beyond linear.** Nothing in the pure set is worse than linear in store size at
the sizes anyone has. Faster is not readier.
