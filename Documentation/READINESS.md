# What "production ready" would mean

**Status:** a checklist, not a promise. Updated when something on it moves.

smysl was unpublished for eight releases, and the reason given each time was "not production
ready" — a true answer and a useless one, because nothing said what would change it, so it
could only be deferred and never worked toward. This file was written to say what the gate is.

**1.0.0 is released. Five gates closed, one waived, one partial.**

| | gate | |
|---|---|---|
| 1 | format stability | ✅ policy written, and exercised by the `smysl/1.0` bump |
| 2 | a second implementation | ✅ three, from the specification alone; all three derive uids |
| 3 | public API stability | ✅ mechanism, plus two published quiet cycles |
| 4 | verified providers | ⚠️ **waived** — OpenAI and Anthropic never called live |
| 5 | a test suite that catches what it claims | ✅ every crate measured; survivors triaged; the CLI's three largest clusters closed in 1.1 |
| 6 | performance characterised | ✅ and it found one |
| 7 | documentation matches the binary | ◐ 88 of 194 `smysl` transcripts, plus the `cargo` ones and the feature table |

Gate 7 is honestly partial rather than waived: what it checks, it checks well, and 0.14 proved
the gap is real by finding three stale claims in exactly the blocks the replay cannot reach.
1.1 closed that half with `make doc-cargo` — and it found drift on its first run, including a
version string that had gone stale *again* one release after being fixed by hand.

Gate 4 — a live call to OpenAI and Anthropic — is the waiver, and §4 below says what that
costs rather than burying it. Everything about those two mappers that can be checked without a
credential has been; what has not is whether the endpoint accepts what we send. Because the
concrete mappers are `#[doc(hidden)]`, a mapper found wrong can be fixed without a 2.0, which
is what makes shipping without it defensible rather than optimistic.

Gate 2 was the one that could not be worked around: an interchange format nobody outside the
project has implemented is a file layout, and publishing it would have meant publishing a
claim. It is closed three times over — `python/`, `nodejs/` and `go/`.

**0.11.0 was published and 0.10.0 never will be** — publishing an older version after a newer
one is possible and perverse, and everything in it is in 0.11.0. Not every tag is a
publication. 0.12.0 went the same way; 0.13, 0.14 and 0.15 were all published, and the last two
are the two consecutive quiet cycles that Phase 3 of `ROAD_TO_1.0.md` asked for.

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

## 1. Format stability — *done, and now exercised rather than only written*

`smysl/0.1` did not change across fourteen crate releases. Record type 10 was *added* in 0.2
without a format bump, which rule X is precisely for, and one decoder became stricter in 0.5
about records it should never have accepted. No encoder's output ever moved.

**The policy this gate asked for exists**, as §8 of `SMYSL_FORMAT_SPEC.md`: §8.1 says what may
change within a version and gives a mechanical test for it — a reader written against the older
revision must still round-trip the addition, byte for byte, or it is a break however small it
looks. §8.2 says what requires a new version and that a reader MUST refuse one it does not
know rather than infer compatibility. §8.3 distinguishes tightening an implementation from
changing the format. §8.4 is the deprecation path.

**And it has been used.** `smysl/1.0` arrived in 0.15, and the whole of §8 governed how:
readers learned the new string in 0.14 and were *published* before any writer emitted it,
because §8.2 makes flipping the order a compatibility break. `smysl/0.1` is still accepted and
still round-trips declaring itself. The bump carried no format change — the same fixtures
produce the same uids and the conformance suite did not move — so §8.6 records what the number
means: the format is settled rather than changed.

A policy that has survived being applied once is worth more than one that has only been
written, which is why this gate is closed rather than "close".

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
part of identity — because uids need C-Produce. That was the format's central claim going
untested by anything but the Rust; `python/` closed it in 0.10, `go/` in 1.1 and `nodejs/` in
1.2.

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

**C-Produce reached a second time in 1.1, in `go/`.** §2.1 now has three independent
derivations — the Rust, `python/`, and `go/` — where it had one until 0.10 and two until now.
`go/blake3.go` is hand-rolled for the same reason `python/`'s is, and passes the published
vectors including the lengths straddling the 1024-byte chunk boundary; `go/uid.go` reproduces
all sixteen canonical encodings and all sixteen uids.

It also implements the half of C-Produce that `python/` does not. §7 defines the class as
"structural + epistemic + *shape*", and the shape clause — a gist present, grounds where
`derived` or `inferred` demand them, a source where `measured` or `cited` demands one, no
authored `unfounded` — is enforced by `Validate`, which `Uid` runs first. A malformed unit
cannot get an identity out of that package.

**And it found a fixture that could not fail.** Removing NFC from the Go encoder failed the
property test while the fixture comparison stayed green, which the pair
`unicode-composed` / `unicode-decomposed` exists to prevent. The generator had been recording
each gist *after* `UnitCoreBuilder` normalised it, so both cases carried the identical composed
string — one input under two names, in a fixture documented as the witness for §3 constraint 6.
Every implementation reading it, `python/` included, was agreeing with itself.

`fixtures/wire/uid/cases.json` now records the gist as *authored*, so a reader that skips
constraint 6 cannot reproduce the recorded bytes. Verified in both directions and in both
languages: with the repair, removing NFC fails the fixture comparison in Go and in Python; the
two tests catch different failures, since a *wrong* normalisation still collides both spellings
and only the fixture knows which bytes are right.

**C-Produce reached a third time in 1.2, in `nodejs/`.** §2.1 now has four independent
derivations and §2.3 three witnesses beyond the Rust. `nodejs/src/blake3.js` is hand-rolled for
the reason the other two are, and passes the published vectors including the lengths straddling
the chunk boundary; `nodejs/src/uid.js` reproduces all sixteen canonical encodings and all
sixteen uids, and implements §7's shape clause with `uid()` running it first.

**And it found four things §2.2 and §2.1 did not say.** Not silence in the ordinary sense —
these are four facts a C-Produce implementer cannot proceed without, and every one was
recoverable only by decoding `core_bytes_hex` in the uid fixtures:

- **§2.2 said the opposite of what the encoder does.** `deps` and `grounds` are listed
  "required, MAY be empty"; an empty one is *omitted*. A literal reading emits a five-key map
  where the reference emits three, which is a different uid for every unit that has neither.
- **The status integers appeared nowhere**, though rule M compares them as integers — so a
  reader that guessed a different order would derive wrong uids *and* enforce a different
  monotonicity rule while believing itself conformant.
- **The `source` map had no key layout**, and `kind` was a second undocumented enum.
- **The base32 alphabet was unnamed.** This one does not move a uid, but §2.1 obliges a parser
  to accept 26 to 52 characters, and base32hex was an equally faithful reading of the sentence.

All four are now in the document, at §2.1 and §2.2, and marked `SPEC:` at the point of use.

The finding under the finding is the one worth keeping: **`python/` and `go/` had already
reached C-Produce through all four gaps without recording that they guessed.** They necessarily
arrived at the same answers — they reproduce the same bytes — so nothing disagreed and nothing
was visible. The suite's whole method is that a `SPEC:` mark is the evidence, and two readers
resolved four ambiguities against a fixture and left no mark. Agreement that comes from reading
the same fixture is not the independence this gate is measuring; it took a third reading to
notice that the second and third had not been reading the specification at those four points at
all.

Each of the eight new invariants was verified capable of failing — status dropped from the
hashed core, NFC removed from the encoder, empty sets emitted, the source keys shifted, the
sort dropped, base32hex substituted, `validate` ungated from `uid()`, and the BLAKE3 tree
ignored. Eight breakages, eight distinct failures, each naming the clause.

**Next action:** none for this gate. C-Consume's rule M is deliberately not implemented in any
of the three: it constrains a unit against the statuses of its grounds, which a unit core does
not carry, so it is checkable against a store and not against a unit. A `validate` claiming to
enforce it would be a check that cannot fail.

## 3. Public API stability — *done: the mechanism, and two published cycles of evidence*

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

**Done, as of 0.15.0.** 0.14 and 0.15 both ended with `SEMVER_BREAKING` empty at the cut and
both are published, with `make semver` 12/12 clean against a baseline that is actually on the
registry. Neither was quiet through inactivity: 0.14 carried the format migration's
readers-first half and gate 4's keyless half, 0.15 flipped the writer to `smysl/1.0`. Change
without breakage is the claim, and it is the harder one.

The `smysl/1.0` migration turned out not to be a breaking change at all — `ParseOutcome` and
`WriteContext` had been made `#[non_exhaustive]` in 0.13, so each could gain the field it
needed. It fitted inside the quiet cycles rather than costing one, which was the open question
when Phase 3 was written.

## 4. Verified providers — *waived at 1.0, with the reason and the limit written down*

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

---

### Waived at 1.0.0 — deliberately, and here is the cost

**1.0.0 ships without a live call to OpenAI or Anthropic ever having been made.** No key has
been available for either, and waiting indefinitely for one is not a plan. Three of the five
mappers — ollama, deepseek, gemini — have been exercised against real endpoints. Two have not.

Phase 4 of `ROAD_TO_1.0.md` permits a waived gate and forbids an unmentioned one, so:

**What is verified without a key.** Both mappers were read against their vendor's
documentation, and that reading found two defects: `caps()` declaring `streaming: true` for a
mapper implementing no `stream`, on both Anthropic *and* Gemini — and Gemini is live-tested,
which means the live test never exercised streaming either. The strict-mode schema translation
is checked recursively against OpenAI's documented rules, using the real Appendix C schema
rather than the reduction it was tested against before 0.14. The failure taxonomy — which
statuses mean stop, retry, or fall through — is asserted for all five mappers alike.

**What is not.** Whether the endpoint *accepts* what we send. A schema can satisfy every
documented rule and still be refused for something undocumented; that is exactly what happened
to Gemini, whose response schema was written as a subset of draft 2020-12 and is not one.

**Why this is a smaller risk at 1.0 than it sounds.** The concrete mappers are
`#[doc(hidden)]` as of 0.13 — `Anthropic` and `OpenAi` are not in the public API, and `build`
returns `Box<dyn Provider>`. **A mapper found wrong against a live endpoint can be fixed
without a breaking change.** 1.0 freezes the provider *abstraction*, which is exercised by
three live-tested mappers; it does not freeze the two unverified translations.

**1.1 closed the Anthropic half of what is checkable without a key.** OpenAI's strict mode
restricts the schema, so what was checkable there was the *translation* — done in 0.14.
Anthropic passes `input_schema` through unchanged, so what is checkable is the envelope around
it, and the envelope has rules that break silently: a tool name must match
`^[a-zA-Z0-9_-]{1,64}$`, `input_schema` must be an object schema, and the forced `tool_choice`
must name the tool that was actually declared. That last one is the easy defect — rename the
tool, forget the choice, and the request asks the model to call something absent from its own
list, failing at the one place nobody here can look. All three are asserted now, plus that the
schema reaching the body is the whole kernel schema rather than a reduction, which is the
failure 0.14 found on the OpenAI side.

**How the rest gets closed.** A key, and one afternoon. If you have an OpenAI or Anthropic key
and want to help: run the provider's live tests with the key in the environment and open an
issue with what came back. A report saying "it worked" closes this gate; a report saying it did
not is more useful still, and is fixable without a 2.0 because the concrete mappers are not in
the public API.

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
| `smysl-provider` (1.1, re-measured) | 466 | **29.8%** |

**The table above is a set of readings, not a state.** Every rate below `smysl-check` predates
0.13, and 0.13 changed the code they measure: 482 items left the contract, five crates gained
`#[non_exhaustive]`, and `smysl-provider` alone gained three test files. Re-measured in 1.1: **29.8%
of 466 viable, against a recorded 31.0% of 477.** Barely moved, which is itself the finding —
0.13 added three test files to this crate and took 310 items out of its contract, and the
survivor rate absorbed all of that as roughly one point.

**The first attempt gave a different answer and it was wrong**, which belongs here rather than
in a commit message. The run was killed at 324 of 556 mutants; the loop waiting for it timed
out without its completion sentinel ever appearing, and the partial counters were read as
final. That produced "26.6%, on a population that fell from 477 to 274" — both halves an
artefact of stopping early, written into this file and stated aloud before anyone checked
whether the run had finished. The shards are uneven in difficulty, so a partial run is not a
sample; the last shard alone held 53 of the 139 survivors.

It is the failure this file spends most of its length describing, committed by the person
describing it. The lesson is not "be careful": it is that a measurement harness needs a
completion signal that is *checked*, and that one had a signal nobody read. The re-run is
sharded, each shard writes a `.done` only after `cargo mutants` prints its own
"N mutants tested" line, and 4 × 139 = 556 is asserted against the population before the
arithmetic is believed.

The standing point survives it. A table of measurements taken at different times against
different code reads as a description of the present, and every rate below `smysl-check`
predates 0.13 — which moved 482 items out of the contract, put `#[non_exhaustive]` on five
crates, and gave `smysl-provider` three new test files. The dates in the left column are
load-bearing.
| `smysl-render` (0.12) | 144 | **12.5%** |
| `smysl-retrieve` (0.12) | 77 | **15.6%** |
| `smysl-embed` (0.12) | 59 | **22.0%** |
| `smysl-thread` (0.12) | 84 | **22.6%** |
| `smysl-ingest` (0.12) | 337 | **28.8%** |
| `smysl`, the CLI (0.12, `--all-features`) | 269 | **73.6%** |
| `smysl`, the CLI (0.13, default features, doc-output wired in) | 268 | **64.2%** |
| `smysl`, the CLI (1.1, re-measured, default features) | 275 | **40.0%** |

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

**Confirmed by re-measurement in 1.1: 110 survivors, 109 of them in `src/main.rs` and one in
`src/progress.rs`** — the per-file figures from 2.2, reproduced exactly by a full sharded run.
The distribution is unchanged too: `cmd_providers` 12, `cmd_fmt` 12, `cmd_merge` 11, `main` 9,
`cli` 8.

**The rate is a different story, and it corrects a claim made in this file's own voice.** The
recorded 64.2% was measured in 2.1, *before* 2.2 did the work; the survivor count of 110 was
measured after. Quoting them together read as though both described the present, and the
conclusion drawn from that pairing — that the CLI's rate was probably still near 64% — was
wrong by a whole phase. It is **40.0% of 275 viable**.

That also narrows a generalisation the provider recalibration seemed to support. `smysl-provider`
moved about one point across 0.13's narrowing and three new test files; the CLI moved
twenty-four across 2.2. Rates move when work is aimed at survivors and barely otherwise, so
"rates move slowly" is not a property of the measurement — it is a description of what happens
when nobody is targeting it.

**The `cmd_*` clusters and the dispatch were then closed, across 1.1.** `src/main.rs` went from
**99 survivors of 205 viable (48.3%)** to **59 of 207 (28.5%)** — every run file-scoped to
`src/main.rs`, at default features, and completed.

| pass | survivors | what it reached |
| --- | --- | --- |
| 1.1, before | 99 | — |
| `cmd_fmt`, `cmd_merge`, `cmd_providers` | **73** | three commands' own decisions |
| `tests/dispatch.rs` | **59** | whether each command is reachable at all |

| function | before | after | where |
| --- | --- | --- | --- |
| `cmd_fmt` | 12 | **2** | `tests/cmd_fmt.rs`, 6 tests |
| `cmd_merge` | 11 | **0** | `tests/cmd_merge.rs`, 7 tests |
| `cmd_providers` | 12 | **1** | `tests/cmd_providers.rs`, 5 tests |
| `cli` + `main` | 15 | **1** | `tests/dispatch.rs`, 3 tests |

23 of the first pass's 26 are those three clusters. The other three are `project_file` and one
dispatch arm each in `main` and `cli`, killed incidentally because a test that runs
`smysl merge --staged` reaches them; that is what the arithmetic would have missed in either
direction.

**The dispatch pass is worth stating as a finding rather than a count.** The fifteen were seven
commands with an arm in each of two places — `ingest`, `usage`, `reindex`, `import`, `relink`,
`compact` and `ui` — and what that meant is that **seven of twenty-two commands could stop
working and the suite would stay green**.

Writing the test found that the two arms are not the same mutation, which the first version
assumed:

- deleting a command's arm in **`main`** removes its *routing*; the subcommand still parses and
  the router falls through to "not wired in this build". Invoking the command finds it.
- deleting its arm in **`cli()`** removes its *arguments* and nothing else, because `cli()`
  registers all twenty-two subcommands from the `COMMANDS` table unconditionally. The command is
  still there, still routes, still runs — it has simply lost every flag of its own, and invoking
  it with no arguments notices nothing.

The second was found by deleting an arm and watching the test keep passing, which is the only way
it could have been found. It needs `tests/cli-surface.txt`, a golden file of all 380 arguments
across the 22 commands, compared **inside `cargo test`** — a Makefile gate like `api-check` is
invisible to anything that counts coverage, which is why `doc-output` was wired into the suite in
0.13.

A second hole appeared the same way. Seven commands take a required argument, so clap rejects the
bare name before the router runs and a no-argument invocation cannot see whether they are wired.
Six were covered by other files that exercise them for real; `import` was not, and its mutant
survived a run of this very test. `minimal_args` supplies what each needs, and the clap refusal
is now itself a failure — so adding a required argument tomorrow says so rather than quietly
ceasing to cover the command.

The one survivor is **equivalent**: `COMMANDS.iter().find(|c| c.name == name)` mutated to `!=`
selects the wrong `Cmd`, and the only field read happens in the `_ =>` arm no input reaches.
Documented at the site, the way 0.13 recorded `worse`'s `>=`.

`cmd_fmt`'s two are the round-trip guard, which fires only if `write_surface` produces something
the parser reads back differently; no input a user can supply reaches it. `cmd_providers`'s one
is **not** a gap either: `src/main.rs:3358` is the `#[cfg(not(feature = "providers"))]` stub of
the same name, which neither the default build nor `--all-features` compiles. Mutating code the
build excludes cannot fail a test. It is an artifact of measuring a file that defines one
function twice, and the honest record of it is here rather than a test written to chase it.

**These figures were nearly published as a comparison of two different configurations**, which
is the error corrected two paragraphs above this one. The re-measurement was run
with `--all-features` while the 99 it was being compared against was default features. It read
as 99 → 94, a nearly-useless change from a day's work — and the tell was that the diff showed
**21 survivors appearing in functions nobody had touched**, which a real regression in `cmd_merge`
cannot cause. Re-run at default features it is 99 → 73, with an empty list of new survivors.

The discarded run left one observation worth keeping, and chasing it produced a second correction
to this file rather than a finding. At `--all-features`, 21 mutants across `cmd_pack`, `cmd_diff`,
`cmd_check`, `cmd_trace`, `cmd_salience`, `cmd_thread`, `cmd_retract` and `emit_pack_surface`
survive that the default build catches. This was recorded here as coverage that is
"feature-dependent in a way nothing states".

**Nothing states it is wrong.** `tests/doc_output.rs` is gated to compile in exactly one of the
nine matrix configurations — the plain `cargo test --workspace` — and the header of that file
explains why at length: the manual documents a default-features build, and `exact-pack` compiled
in makes a correct `SMY-W202` claim read as drift. At `--all-features` the test is not skipped,
it does not exist, so the 21 mutants only it reaches are unopposed. Confirmed by counting:
`cargo test --test doc_output` runs 1 test, `cargo test --all-features --test doc_output` runs 0.

So the answer to "why does the CLI measure worse at `--all-features`" is written down in the
file that causes it, and the question was asked by someone who had not read it. The number is
real, the design is deliberate, and the only thing that was missing is this paragraph.

Three things came out of that work that are worth more than the counts.

**A defect the mutants only pointed at.** `merge --format surface` counted a label binding as
having no surface form, so `@claim c/a` warned that one record was "omitted" over output that
read `@claim c/a`. The comment directly above that filter records the *same* mistake being fixed
once already, for the `@doc` header — expressible, and blamed on contentions. The count now asks
`ctx` which label it will write rather than assuming, so a binding that loses the fold still
counts and one that is rendered does not.

**A test may not assume the feature set it was written under.** `cmd_providers` first configured
its hosted provider as `anthropic` and passed under `--all-features` and the default build, then
failed three tests in the matrix entry that has `providers` and ollama but no vendor mappers —
`providers` alone compiles none. `caps().offline` comes from the *endpoint*, not the vendor, so
every provider there is now `kind: ollama` and local-versus-hosted is carried by `127.0.0.1`
versus a public host. That is closer to the rule the code implements, and it does not depend on
which combination is being built. `cmd_merge` had the same shape: `--staged` needs `ingest`, so
those three tests carry their own `cfg` and the other four still run in a `cli`-only build.

**A negative assertion on a padded string is a check that stops working silently.** Asserting
`!stdout.contains("nearby         refused")` ties the test to a column width, and a change to the
padding makes it pass by never matching. The rows are looked up by id now, and a missing row
panics rather than satisfying a `!contains`.

**Next action:** not "fix 357 survivors". The band 12–31% has yielded roughly one real gap per
crate and reading each survivor is the expensive part, so the yield per hour is falling. The CLI
was the exception, and the three clusters that made it one are closed; what remains in
`src/main.rs` is `main` and `cli`, which are subcommand dispatch.

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

### 1.2 measured it, on one module: 22% of its survivors were artefacts

Subject: `crates/smysl-graph/src/store/mod.rs` — 152 mutants, 137 viable — chosen because it
holds `Store::matching_prefix`, the one instance named above, so the run could be checked
against a known answer rather than only producing a plausible one.

| | survivors | of viable |
|---|---:|---:|
| `cargo mutants -p smysl-graph` — the weak claim, how every figure above was produced | 23 | 16.8% |
| the same mutants, `--test-workspace true` — the strong claim | **18** | **13.1%** |

**Five of twenty-three were never gaps.** They are exercised by a downstream crate and reported
as survivors because `-p` runs only `smysl-graph`'s own tests:

- `Store::matching_prefix -> vec![]` — the known case, which is what validates the run
- `delete match arm 1 in Store::resolve_prefix` — the same prefix path
- `Store::contentions -> Vec::leak(Vec::new())`
- `delete match arm Record::LabelBinding in Store::emit`
- `replace == with != in Store::absorb`

So the backlog is **mostly real**: on this module the weak number overstates by about a fifth,
not by the multiple it might have. A prediction made before the run went the other way and is
worth recording — six of the 23 were `delete match arm Record::…` in `Store::emit`, and the
guess was that a crate reading those records back would catch all six. One did.

**One module of one crate.** Whatever this says about the other 357 survivors, it is evidence
and not a figure, and applying 22% to them would be the same error this file records twice
already: a number quoted without its configuration.

**`--test-workspace` needs an explicit `--timeout`, and the default silently produces a run
that is mostly timeouts.** cargo-mutants measured its baseline at *1s of tests*, auto-set the
timeout to 20s, and then ran each mutant against the 109-second workspace suite. The first
attempt came back `4 missed, 6 caught, 15 timeouts` — which, read as a result, says nineteen of
twenty-three survivors were artefacts. It says nothing of the kind; most of those mutants were
never tested. The tell was the one this file keeps naming: a number that moved for no reason
anyone could state, and 19 of 23 is not a plausible artefact rate. Re-run with `--timeout 400`
it is 0 timeouts and the table above.

The costs, since they are the reason this had never been run:

| | package | workspace | ratio |
|---|---:|---:|---:|
| cold, build + test | 229s | 1863s | 8× |
| warm, test only | 7.75s | 109.35s | **14×** |

Run A was 8 minutes for 152 mutants; run B, 12 minutes for 25.

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

**Done in 0.12**, in `crates/smysl-graph/tests/predicates.rs`. Re-measured in 1.1 and all three
are killed. This gate carried "next action: tests for those three" for three releases after the
tests existed, which is its own small lesson: a checklist is only as true as its last reading.

**And the class was not exhausted.** Re-measuring found two more predicates of exactly the same
shape — `Lineage::is_empty` and `RetractionPlan::is_empty`, each replaceable with `true` and
nothing failing, each on a path a user takes. A shortlist is what one pass surfaced, not the
population.

`Lineage::is_empty -> false` is a third case again, and it cannot be killed: `trace` pushes a
node for the root before anything else, and `Lineage` is `#[non_exhaustive]`, so no lineage is
ever empty and no consumer can build one that is. The method can only return `false`. Recorded
as equivalent rather than chased — and worth recording because the first draft of the test
*did* chase it, asserting `is_empty() == (len() == 0)` on a lineage where both are trivially
false. That passes under the mutant. It is the same failure this file keeps describing, written
by the person describing it.

**A real gap found alongside them: the contention-flood cap.** `flooding` decides `SMY-W055`,
and its one test set `max_contentions_per_agent = Some(0)` — which makes `len > cap` trivially
true, so "always fire", "always fire one", and `>` loosened to `>=` all pass it unchanged, and
the count in the message was never read. Three mutants on an anti-abuse diagnostic, alive
because the threshold was only ever tested at the one value where it is not a threshold. Now
tested either side of a real cap, with the reported count asserted.

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

## 7. Documentation that matches the binary — *partial, and the cheap half now gated*

`make doc-output` replays **88 of 194** documented command blocks and gates on them; 105 are
skipped, mostly because they name files a chapter built earlier in its own narrative. Appendix
A, the purity table and the diagnostic registry are cross-checked against the code at every
release cut.

**Both of those numbers moved in 1.1, and the denominator moving is the more serious half.**
It had been quoted as 168 since the script was written. The real figure is 194: the block
regex required the code fence on the line *after* `#screen(...)[`, and chapter 22–24 writes it
on the same line throughout, so **twenty-six blocks matched nothing at all**. They were not
skipped — skipping is counted and reported — they were invisible, and every render transcript
in the book had gone unchecked since the chapter was written. `verify-doc-cargo.py`, written
later, got this right; the two scripts scanning one book disagreed about how many blocks are in
it, which is how it surfaced.

A further twenty-two were skipped as "input file missing" when the token was never a file.
The path test is "contains a slash", and smysl spells labels `kind/name` — so `--thread
t/brief`, `--id v/two-roots` and `--tokenizer tiktoken/cl100k` all read as files that do not
exist. The discriminator is the dot: every file the manual names carries an extension and no
label does.

Bringing those into view found **one real drift** and two conventions the script had never
met, because they live only in the chapter it could not see: `exit N` as a trailing line, now
checked against the actual exit code rather than compared as text; and `(excerpt)` in a
caption, meaning the block is a window onto the output rather than the whole of it. The drift
was a `--target json` transcript that had been hand-wrapped, showing `"notes"` across four
lines where the renderer emits one.

**And the tutorial files are committed, which is gate 7's larger half.** 100 commands name a
file the *prose* asks the reader to create; twelve of those files are now in
`fixtures/tutorial/<chapter>/`, and the commands run with their working directory set there —
so the page still says `smysl check cycle.smy`, the output still says `cycle.smy: error: …`,
and nothing a reader types has changed.

Twelve of forty-six, not all of them, and the arithmetic is the point:

- **22 files, 57 commands, have more than one state.** Chapter 1 has the reader create
  `first.smy` broken, fix it, find it unformatted, then rewrite it in place. Four commands name
  one path and expect four different files; committing any one makes the other three report
  drift that is not there. Splitting them means the chapters naming a different file at each
  step, which changes what the book teaches — ~~still a decision about the book~~. **It was
  not a decision about the book; see 1.2 below.**
- **Seven were extracted and then removed**, because the chapter's own transcript refused to
  reproduce against them: the fenced block before the command is a *fragment* the reader adds,
  not the file. This is the failure the 0.10 attempt hit and recorded, met again and this time
  measured. It is loud rather than subtle — the diagnostics carry byte offsets, so a
  wrongly-assembled file reports `at 0..125` where the manual says `at 296..416`.
- **One is described rather than printed** — `bignote.txt`, "a ~7 KB paragraph, repeated".

The rule that holds it together is that a fixture must be the bytes its chapter prints, and
`verify-doc-output.py` fails if one is not found verbatim in its chapter. Without it, editing
the page leaves the fixture behind and the script starts measuring its own copy of the book.
Verified by appending a line to a fixture and watching it fail.

### 1.2: it was not a decision about the book

The 57 commands were blocked on fixtures being keyed by **filename** when a chapter's state is
keyed by **position**. And two of chapter 1's four states cannot be committed at all, which is
what settles it: state 2 is printed as a *fragment* the reader pastes in, state 3 as a *diff*.
Neither is ever printed as a file, so neither could satisfy the verbatim rule even if somebody
wanted it to. No arrangement of committed fixtures was ever going to reach them.

So the script now replays a chapter **as a chapter**: a scratch copy of its committed files,
walked in document order, with `fmt --write` allowed to actually write. A later state is
*derived rather than recorded*, which is the stronger claim — the file a command runs against
is one the book's own instructions produced. The scratch copy is also what makes writing safe:
commands used to run in the fixture folder itself, so one replayed `fmt --write` would have
rewritten a tracked file on every run.

The reader's hand-edits are the one thing replaying cannot produce, and they are declared in
`fixtures/tutorial/<chapter>/edits.json` **by prose anchor, never by content** — the edit body
is the fenced block following the anchor, read out of the chapter at run time. Nothing is
duplicated, so nothing can go stale: change the page and the anchor stops resolving, which is
an error rather than a silent pass. An anchor must appear exactly once, and appearing twice
fails as loudly as appearing never.

**78 → 88 replayed**, with `first.smy`'s whole four-state narrative now running end to end, and
`step1 → step2 → step3` in chapter 4 — one document under three names, none of the later two
committable. Three breakages verified: an anchor that stops resolving, a stanza match that
finds nothing, and a base fixture edited away from its chapter. Each reports the root cause
*and* the downstream mismatches, in that order.

**And it found real drift, in the chapter with the most at stake.** `beta.smy` in chapter 29 is
printed in full and its transcript claims `14 records, 5 units`. The document the book prints
has **four** units; `check` says `12 records, 4 units`. The transcripts in that chapter were
generated from a `beta.smy` that is not the one the page prints, and it propagates through
seven transcripts — the contention labels in `merge`, the counts in two `check`s, and **a uid
the reader is told to type** into `retract`. `alpha.smy` beside it is in sync, contention uid
included, so this is one document that lost a stanza rather than a chapter that drifted.

It is held back rather than repaired: the missing stanza cannot be reconstructed from the page,
and guessing one into a published chapter is not a repair. Restoring it makes seven more
commands replayable at once, the largest single block left.

**Chapter 4 is now exhausted: 17 of its 20 commands.** Working through it found that only one
of the four remaining files needed a chain at all, and the other three each failed for a
different and more interesting reason:

- **`checkout.smy` needed nothing.** The chapter prints it in full *after* the command — "The
  complete file, for reference" — and the first attempt looked only at the block *before*. The
  assumption was wrong, not the mechanism.
- **`ticket.smy`** is printed only as `fmt`'s *output*, never as input. That output is a fixed
  point, so it is committed as the file, and it exercises exactly what the section is about:
  `ref: 42` unquoted does not re-parse as a string — it is read as a number, leaving the source
  with no `ref` — so a writer that stopped quoting would fail `fmt`'s own reparse assertion and
  refuse to write, which is the guard those pages describe.
- **`batch-a.smy` / `batch-b.smy` would be a check that cannot fail.** They are described rather
  than printed, and `fmt --write` prints nothing on success — so the expected output is empty
  and *any* two valid files satisfy it. Skipped for a stronger reason than missing bytes.
- **`extrel.smy`** is 7 records and 3 units, of which the chapter prints only the one `@rel`
  line that makes it interesting.

**And a second instance of the `beta.smy` pattern.** `draft.smy`'s transcript shows `fmt --write`
warning that two comment lines will not survive. Run against the bytes the page prints, `fmt`
never reaches that warning: the snippet's `@claim c/regression` names `grounds: [e/trace]` and
nothing defines `e/trace`, so it exits 3 with `SMY-E060` and `SMY-E031`. A reader following the
page literally gets two errors where the book shows a warning. That transcript, like chapter
29's, was generated from a fuller document than the page prints — **twice now, in two chapters,
found only once the files became replayable.** Both are held back rather than repaired, for the
same reason: the missing stanza is not on the page and inventing one is not a repair.

**That number is measured under one specific build, and reading it under another gives a
different answer** — which was nearly written into this file as drift. Re-checking it in 1.1 by
running `scripts/verify-doc-output.py` directly produced `ran 45, skipped 123`, and 46 was about
to be "corrected" to 45 across three documents. The script reads `./target/debug/smysl`, and what
was on disk was an `--all-features` binary left by unrelated work; `make doc-output` runs a
default-features `cargo build` first, on purpose, and the script's own docstring says why. Under
the supported invocation it is 46, twice, with `MISMATCHED 0` — and pointed deliberately at an
`--all-features` binary it reports `MISMATCHED 1`, the false drift the docstring warns about.

The lesson is not "the number was fine". It is that **a measurement quoted without its
configuration is not a measurement**, which this file already says about the CLI's survivor rate
one section up, and which cost a wrong correction here before the check was run the supported
way. Both times the tell was the same: a number moving for no reason anyone could name.

**`make doc-cargo` closed the other half in 1.1.** `verify-doc-output.py` replays the documented
`smysl` commands and its skip rules pass straight over the `cargo` ones, which is why every one
of the three stale claims 0.14 found by hand was in a block it cannot reach. The new gate replays
the six single-command `cargo` transcripts, checks every version string against the manifest, and
compares the one feature table that claims to be "transcribed here exactly" against `[features]`.
It found drift on its first run:

- the `cargo build` transcript said `v0.13.0` — stale *again*, one release after being fixed by
  hand in 0.14;
- `cargo xtask check-purity` claimed 25 crates where there are 31, 6 pure crates where there are
  7, 66 source files where there are 71, and omitted the `rule A` line entirely, so the manual
  had documented a purity gate that stopped matching what runs when 0.13 added rule A to it;
- the dependency tree still carried 0.13 versions.

Two design points are worth keeping. Cargo's progress lines are normalised away because they
vary between two correct runs — which leaves `cargo build` with nothing to compare, its whole
body being `Compiling` and `Finished`, so the version inside is checked separately against the
manifest rather than left unchecked. And the feature check is scoped by a marker rather than a
filename: the manual has more than one feature table and the others are prose, so comparing those
produced eleven mismatches of which none was a defect. A check that cannot tell "nothing was
wrong" from "nothing was checked", and a check that cries wolf, fail in opposite directions and
both teach people to ignore it.

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

**1.1 took the cheaper action**, which is `make doc-cargo` and is now part of `make ci`.
`verify-doc-output.py` replays `smysl` commands; these are a different program, so its skip
rules passed over them and nothing checked them at all. The new script replays the six
single-command `cargo` transcripts, checks every version string in them against the manifest,
and compares the one feature table that claims to be "transcribed here exactly" against
`[features]`.

It found drift immediately, which is the argument for it:

- the `cargo build` transcript said `v0.13.0` — **stale again**, one release after being fixed
  by hand in 0.14. A version number in prose goes stale every release; that is a reason to
  check it, not to remove it.
- the `cargo xtask check-purity` transcript claimed 25 crates where there are 31, 6 pure crates
  where there are 7, 66 source files where there are 71 — and omitted the `rule A` line
  entirely, so the manual documented a purity gate that had not matched what runs since 0.13.
- the dependency tree still carried 0.13 versions.

Confirmed by re-introducing 0.14's feature-table defect and watching the check name it.
Twenty-seven seconds warm, which is what makes it affordable on every push rather than at
release time only.

**Next action, still open:** if this is worth more, the fix is in the *manual*, not the script —
commit the tutorial files as fixtures and have the chapters include them, so what the reader
copies and what the script replays are the same bytes. That is a book change, and the book is
the thing the coverage is protecting, so it should be a deliberate decision rather than a side
effect.

---

## What is deliberately *not* on this list

**Feature completeness.** The command surface is twenty-two commands and has not needed a new
one since `find` in 0.5. Readiness is not about having more.

**A single crate.** Abandoned in 0.7.0 — the only benefit was publishing one crate rather than
eleven, and it would have cost the crate boundary as a compiler-enforced constraint. Eleven
crates or none.

**Performance beyond linear.** Nothing in the pure set is worse than linear in store size at
the sizes anyone has. Faster is not readier.
