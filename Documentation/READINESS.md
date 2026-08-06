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

**0.11.0 is published**, and 0.10.0 is not and now never will be — publishing an older version
after a newer one is possible and perverse, and everything in it is in 0.11.0. Not every tag is
a publication.

That gap is worth remembering rather than tidying away. `cargo-semver-checks` fetches its
baseline from crates.io, so `BASELINE` in the Makefile tracks the last *published* version and
not the last tagged one; it sat at 0.9.0 across two cut-but-unpublished releases, which meant
every breaking change was being measured against a version two releases old, and the
`ContextExceeded` repair that wants `parse`'s signature to move was parked behind it. Pointing
it at an unpublished version does not fail loudly — it turns all twelve crates into "version
not found in registry", a red job saying nothing about the API.

Nothing here is a schedule. The point is that each item is either done, or has a next action
that someone could take.

**For 1.0 specifically, see [`ROAD_TO_1.0.md`](ROAD_TO_1.0.md).** This file asks whether the
project is production-ready; that one asks what a version number promising stability would
commit to, which is a narrower and harder question. The two differ in one place worth knowing
about: these gates can be *waived* with a reason, and a 1.0 promise cannot — a name either
moves or it does not.

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

## 3. Public API stability — *the mechanism is done; the evidence is not*

This gate asked for four things and 0.13 delivered all four. What it cannot deliver is the only
thing that counts as evidence, which is time.

**What it asked for, and where it landed:**

- *A pass over the facade's `pub use` list, asking of each name whether it is contract or an
  implementation detail that escaped.* Done — `API_CONTRACT.md`'s three buckets, the three open
  names decided, and then §1.2 of `ROAD_TO_1.0.md` acting on the answer: **482 public items out
  of the contract** across `smysl-provider` and `smysl-ingest`, with the facade's 243 names
  untouched throughout. That last clause is the check that says nothing a consumer had was
  taken away.
- *`#[non_exhaustive]` where the answer is "we will want to add to this".* Done — **152 of 191
  distinct public types carry it**, and each of the other 39 has an answer: 33 are closed by
  encapsulation (no public fields to add to) and six are closed on purpose and say so where
  they are declared. The earlier "110 of 378" counted the same type once per re-export path.
  The argument turned out to be §8's, not taste: the crate and format versions are independent
  axes, so an exhaustive `UnitCore` would make the next format field a crate major.
- *A golden file.* Done, and then found insufficient. `tests/public-api.txt` records the
  facade; `tests/public-api-counts.txt` records each crate's surface *size*, which catches the
  one thing neither other gate does — a public item added by accident, which is nobody's break
  and so was nobody's failure.
- *`cargo-semver-checks` turning an accidental break into a failing job.* Done, and corrected:
  it used to `continue` past every crate on `SEMVER_BREAKING`, so a crate with one deliberate
  break had **nothing** watching it. It now runs them ungated and prints what it finds. That
  change caught a wrong entry on its first run.

**The finding that reorders the three gates.** `cargo-semver-checks` cannot see through a
`pub use` from another crate — the same blind spot `cargo public-api` has. Run against `smysl`
it reports *"no semver update required"* for 0.12's rename of `Error` to `AnyError`, although
`v0.11.0` exported `smysl::Error` and nothing exports it now. **The golden file caught that
rename; the semver gate did not.** So the division is not the obvious one, and
`API_CONTRACT.md` now states which artefact is authoritative for what.

**Why this is still not ready.** The mechanism is in place and the surface is worth freezing;
what is missing is a demonstration that it holds still. 0.13 broke nine of twelve crates —
deliberately, because it is the last cycle in which narrowing is free — so it is cycle zero
rather than one of the two Phase 3 asks for.

**0.13.0 is published**, all twelve crates confirmed on crates.io. `BASELINE` is 0.13.0,
`SEMVER_BREAKING` is empty for the first time since 0.9, and `make semver` now gates all twelve
crates and reports "no semver update required" for every one. Through 0.12 and 0.13 it was
comparing against a version two releases old *and* skipping the crates most likely to move; it
is watching everything now.

**Next action:** two cycles — 0.14 and 0.15 — with `SEMVER_BREAKING` empty at the cut, both
published. `ROAD_TO_1.0.md` Phase 3 lists what could break them and what has already been done
to stop it, including the finding that the `smysl/1.0` format migration is *not* a breaking
change and can therefore land inside a quiet cycle.

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

**0.14 split that question in two, and answered the half that does not need a key.**

"Will it be accepted" is partly a question about their endpoint and partly a question about our
schema. The second half is checkable here: OpenAI documents what strict mode requires, and
whether the schema we send satisfies it is a property of the translation rather than of the
API. That is the same method — read the documentation and *count* — that already found the
`required` mismatch and the two `streaming` defects.

Two gaps closed:

- **The strict-mode invariants are recursive and were only checked at the root.** OpenAI needs
  `additionalProperties: false` and every property in `required` on *every* object, and
  Appendix C nests them — `source`, `payload`, the objects inside `deps` and `grounds`. One
  nested object missing either is a rejected call, not a degraded one. Now walked in full, with
  a control asserting the untranslated schema violates the rules so the test cannot pass on a
  transform that does nothing.
- **The schema being translated was a miniature.** `smysl-provider` cannot depend on
  `smysl-ingest` — ingest depends on the provider, so that is the cycle — and it kept an inline
  copy to test against. The copy had **2 of the 13 kernel types, 2 of the 5 statuses, 1 of the
  3 conditionals** and a different `label` pattern, while `openai.rs` documented these tests as
  running "against the full Appendix C schema rather than a miniature of it". It *was* the
  miniature, and nothing could have noticed: the two definitions had no way to meet.

  `fixtures/schema/unit.json` is where they meet now — generated by `unit_schema()`, asserted
  byte-for-byte against it on the ingest side, and translated and checked on the provider side.
  The translation is correct against the real schema; that had simply never been tested.

**What is left needs a key and nothing else:** whether OpenAI and Anthropic *accept* a schema
that is now known to satisfy their documented rules. That is a much narrower question than the
one this gate started with.

**Mutation testing in 0.12 found the gap this gate describes is not the gap it has.** 477
viable mutants, 31% survivors — the worst of any crate but the packer, and 25 of them on one
cluster: what a mapper makes of an HTTP failure. `delete match arm 401 | 403`, `replace match
guard is_backpressure(s) with false`, `replace >= with <` on the `status >= 400` boundary.

The point is *which* providers. Gemini, DeepSeek and Ollama have all been exercised live, and
the survivors are spread evenly across all five mappers. Live testing verified that a
**successful** call works; nobody provokes a 401 against a real endpoint, so the failure
taxonomy went unexercised on the verified providers too. **A key would not have found this.**

That taxonomy decides behaviour rather than wording: `Unauthorized` stops the run,
`RateLimited` is retried with backoff, `Upstream` may fall through to another provider.
Misclassify a 429 as a fault and a transient overload ends a pipeline; misclassify a 401 as
backpressure and the CLI retries a credential that will never work, three times, with jitter.

`tests/status_taxonomy.rs` covers all five at once, with a control that fails a mapper
returning any single variant for everything. Confirmed against three real survivors in three
different mappers.

**Next action:** still a key, for acceptance — whether the endpoint takes the translated
schema is unreachable without one. But the table above is the more honest reading of what
"verified" has meant here: the happy path, on three of five.

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
| `smysl-provider` (0.12) | 477 | **31.0%** |
| `smysl-render` (0.12) | 144 | **12.5%** |
| `smysl-retrieve` (0.12) | 77 | **15.6%** |
| `smysl-embed` (0.12) | 59 | **22.0%** |
| `smysl-thread` (0.12) | 84 | **22.6%** |
| `smysl-ingest` (0.12) | 337 | **28.8%** |
| `smysl`, the CLI (0.12, `--all-features`) | 269 | **73.6%** |
| `smysl`, the CLI (0.13, default features, doc-output wired in) | 268 | **64.2%** |

**Every crate in the workspace is now measured.** The library sits between 2.6% and 31%, in a
band that has stopped being surprising: most survivors are accessors, display strings and
equivalent mutants, and each run has turned up a handful of real gaps.

**The CLI is not in that band, and the difference is only partly an artefact.** The obvious
explanation was the per-crate one — `cargo mutants -p smysl` runs `cargo test -p smysl`, and
the CLI's principal verification was `make doc-output`, a Python script replaying 46 documented
transcripts against the built binary, which no cargo test invoked. That made mutation testing
structurally blind to it.

**0.13 wired it in and measured both ways** (§1.2's Phase 2.1: two runs, identical default
features, differing only in whether `tests/doc_output.rs` is present). Without it, **72.0%**;
with it, **64.2%** — 21 mutants newly caught across nine `cmd_*` functions, none newly missed.
The 72.0% against the earlier 73.6% also says the original figure was not an artefact of the
feature set.

So the blindness was real and worth about eight points, and it was not the whole story.

**Phase 2.2 took the 172 down to 110.** `src/progress.rs` went from 51 survivors to **1**, and
`src/main.rs` from 121 to **109**.

`progress.rs` was not short of tests so much as unobservable: every decision was welded to the
environment or to `stderr`, and its twelve tests all used `Style::silent()` because nothing
else was available to them. Splitting the decision from the environment (`Style::decide`), the
line from the write (`render`), and adding a sink took it to 43 tests that assert numbers. Two
of the 51 were real defects — a clamp that never clamped, so a bar could print `105/100`, and a
dead assignment whose arithmetic no test could reach. The single remaining survivor is
unreachable in-process and says so where it is declared.

`main.rs` gave up its pure helpers — `looks_like_surface`, the path rule shared by
`project_root` and `project_file`, `read_input`, `finish_over` — plus one *equivalent* mutant
in `worse` that is now documented rather than merely alive.

The remaining 109 are `cmd_*` bodies: `cmd_providers` 12, `cmd_fmt` 12, `cmd_merge` 11. They
read a filesystem, build a store and print, so reaching them means driving the binary the way
`tests/global_flags.rs` does rather than calling a function. That is a body of work in its own
right, and it is not what 1.0 freezes.

**Next action:** not "fix 357 survivors". The band 12–31% has yielded roughly one real gap per
crate and reading each survivor is the expensive part, so the yield per hour is falling. The
CLI is the exception worth acting on, and the useful first move there is to make its real
verification visible to measurement — `doc-output` is the test that covers it, and nothing that
counts coverage can see it.

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

**0.13 put the replay inside `cargo test`.** `tests/doc_output.rs` runs it against
`CARGO_BIN_EXE_smysl`, which is the binary `cargo-mutants` rebuilds with each mutation applied.
That was Phase 2.1's purpose and it moved the CLI's survivor rate from 72.0% to 64.2% — 21
mutants caught across nine `cmd_*` functions that nothing else reached, including an entire
output function being replaced by `String::new()`.

Getting it wired in produced the defect worth recording here. **The first version passed while
the binary's output was changed.** It hands the script an absolute `CARGO_BIN_EXE_smysl`; the
script substitutes that into each command and then scans the tokens for absolute *input* paths,
which are narrative state it cannot replay — so the program itself matched the rule and all 168
blocks were skipped. It printed `ran 0, skipped 168, MISMATCHED 0`, and the test asserted only
that a summary line existed. It now asserts `ran >= 40`, and the script excludes the program
from the scan.

**And 0.13 found real drift, in exactly the place this gate predicts.** Three claims in the
manual had gone stale, all of them in blocks the script *cannot* replay:

- The feature table said `default` turns on `tui`. It does not, and `Cargo.toml`'s own comment
  says `tui` is "deliberately absent" — so a reader was told a plain `cargo build` gives them
  the terminal UI when it does not.
- The `cargo tree --no-default-features` transcript was from 0.1.0 and predated
  `smysl-retrieve` becoming a plain dependency, so it was missing `bm25`, `fxhash` and
  `byteorder` entirely.
- The prose beside it read "the only third-party code in the tree is `blake3` and
  `unicode-normalization`", which those three crates had made false.

All three are `cargo` transcripts rather than `smysl` ones, which is precisely why the replay
never saw them. **The 122 skipped blocks are not merely unchecked; they are where drift
actually accumulates**, and that is now demonstrated rather than assumed.

**Next action:** if this is worth more, the fix is in the *manual*, not the script — commit the
tutorial files as fixtures and have the chapters include them, so what the reader copies and
what the script replays are the same bytes. That is a book change, and the book is the thing
the coverage is protecting, so it should be a deliberate decision rather than a side effect.

A cheaper second action now has evidence behind it: the handful of `cargo` transcripts could be
regenerated at release time the way `make docs` rebuilds the PDFs. Three of the four defects
above would have been caught by that alone.

---

## What is deliberately *not* on this list

**Feature completeness.** The command surface is twenty-two commands and has not needed a new
one since `find` in 0.5. Readiness is not about having more.

**A single crate.** Abandoned in 0.7.0 — the only benefit was publishing one crate rather than
eleven, and it would have cost the crate boundary as a compiler-enforced constraint. Eleven
crates or none.

**Performance beyond linear.** Nothing in the pure set is worse than linear in store size at
the sizes anyone has. Faster is not readier.
