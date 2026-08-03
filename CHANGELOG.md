# Changelog

Notable changes, and what they cost or bought. Versions follow semver on the
**crate**; the *format* version is a separate axis and moves only when the wire
format changes incompatibly. A crate major bump does not imply a format break,
and the facade asserts the two are independent.

---

## Unreleased — 0.10.0

Nothing yet. What is carried, and what each is actually waiting on:

- **An API stability pass**, which 0.9 promoted from an item to the leading one by publishing.
  Every name in the facade's `pub use` list is now something people can build against, and
  `Hybrid` changed shape twice inside 0.7. The work is going through that list once and asking
  of each name whether it is contract or an implementation detail that escaped, then
  `#[non_exhaustive]` wherever the answer is "we will want to add to this". Cheap now, and
  expensive in proportion to how many releases it waits.

- **C-Produce, in one of the three implementations.** C-Read does not reach uid derivation, so
  §2.3 — *status is part of identity*, the paragraph the whole format rests on — is still
  verified by this implementation alone. All three suites carry a test that fails if that gap
  is ever quietly dropped from their list. Closing it means BLAKE3 and canonical unit-core
  encoding in one language, and it would test the claim the format actually makes rather than
  the claim it is easy to read.

- **A format-versioning policy.** `smysl/0.1` has now been stable across nine releases and is
  published, which raises the cost of getting this wrong. What constitutes a break, what the
  deprecation path is, and whether `0.1` is frozen or merely stable-so-far, belongs in the
  spec — where the three implementations will read it.

- **The quoting coarsening**, still waiting on a design rather than a model. There is no flag
  controlling the quote requirement — `quote` is an optional property in Appendix C and the
  behaviour comes from the prompt — so the two arms have to be built before they can be
  compared. An hour if the difference is only in the prompt.

- **Anthropic's mapper**, read against its documentation the way OpenAI's was. That method
  found a real defect without a key. Anthropic uses `ToolForce` rather than `JsonSchema`, so
  its unknowns are different and none have been looked at.

- **`merge` and `check` scaling.** Both have only ever been measured through the command, where
  parsing dominates. Extending `crates/*/tests/scaling.rs` would close the last "we assume it
  is linear" in the pure set.

- **doc-output coverage**, 46 of 168 documented command blocks. The rest are skipped because
  they name files a chapter built earlier in its own narrative; teaching the verifier to build
  those intermediates would roughly triple it. The manual has been wrong twice in ways that
  mattered, and both were in the uncovered 122.

- **Targeted mutation testing of `smysl-core` codec invariants.** 0.8 established that asking
  what the suite *trusts* finds oracles faster than generating mutants does. The codec is the
  obvious next place to ask it, because everything downstream trusts round-tripping.

---

## 0.9.0 — 2026-08-02

### Added

- **Published to crates.io**, as twelve crates rather than one. The facade `smysl` carries the
  library and the CLI; the eleven behind it are the crate boundaries that enforce rule B, and
  collapsing them into one would have cost the compiler's ability to check that the pure set
  stays pure. That was weighed in 0.7 and settled the same way.

  `smysl-eval` stays unpublished, and its reason is mechanical rather than editorial: its
  tests read `fixtures/corpus` through `../..`, which escapes the package root `cargo package`
  archives. A published copy would ship tests that cannot run.

  Not because the checklist in `READINESS.md` is finished — it is not. Four of its seven gates
  are open, and the reasons are written there rather than summarised here. What changed is that
  gate 2 closed, and gate 2 was the one that could not be worked around: an interchange format
  nobody outside the project has implemented is a file layout, and publishing it would have
  been publishing a claim. Three implementations later it is a fact, and the remaining gates
  are about polish and coverage rather than about whether the thing is real.

  0.x, so semver permits breaking changes, and gate 3 says plainly that the facade's surface
  has not had its stability pass.

- **Three independent implementations of the wire format** — `python/`, `nodejs/` and `go/`,
  each written from `SMYSL_FORMAT_SPEC.md` and each targeting C-Read: decode, re-encode
  byte-identically, preserve what is not understood. All three run in CI against fixtures in
  `fixtures/wire/` that the Rust produced.

  This is the gate `READINESS.md` called the largest. Every other check in this repository
  would pass just as happily if the specification were blank — they test whether the Rust is
  self-consistent, and only these test whether the *document* is sufficient.

  More than one on purpose: implementations that agree could have made the same guess where
  the document is silent, so agreement is evidence only when the readings are independent.

### Changed

- **Three clarifications to §3 of the spec**, which is what writing them produced. Constraint 1
  said what an encoder may do without saying what a decoder must do. Constraint 2 said "no
  value encoded in more bytes than it needs" without scoping that to integers and lengths —
  applied literally to a float head it rejects 1.0, whose payload looks like an over-long
  encoding of 1 065 353 216. Tags were not mentioned at all, and are now constraint 8.

  Two independent readers hit the same two of the three. The section records that, because
  their guesses agreeing is fortunate rather than reassuring: a document that told them would
  have been better than two that happened to concur. The Go implementation, written against
  the revised text, needed no guesses there — which is the check that the revision worked.

### Documentation

- Everything at 0.9.0. The spec gains a §0 pointing at the three implementations as worked
  examples. The rationale gains "Can someone else implement it?", which is the question the
  whole format rests on and could not be answered honestly before. The architecture RFC gains
  "Closed in 0.9".

What is carried, and what each is actually waiting on:

- **C-Produce, in one of the three.** C-Read does not reach uid derivation, so §2.3 — *status
  is part of identity*, the paragraph the whole format rests on — is still verified by this
  implementation alone. All three suites carry a test that fails if that gap is ever quietly
  dropped from their list. Closing it means BLAKE3 and canonical unit-core encoding, in one
  language, and it would test the claim the format actually makes.

- **The quoting coarsening**, which needs a design decision before it needs a model. There is
  no flag controlling the quote requirement — `quote` is an optional property in Appendix C
  and the behaviour comes from the prompt — so the two arms have to be built before they can
  be compared. An hour if the difference is only in the prompt.

- **Anthropic's mapper**, read against its documentation the way OpenAI's was. That method
  found a real defect without a key and narrowed OpenAI's remaining risk to "does the endpoint
  accept the translated schema". Anthropic uses `ToolForce` rather than `JsonSchema`, so it has
  a different set of unknowns and none of them have been looked at.

- **A format-versioning policy.** `smysl/0.1` has been stable across eight releases, which is a
  record rather than a commitment. What constitutes a break, and what the deprecation path is,
  belongs in the spec.

- **An API stability pass.** `Hybrid` changed shape twice inside 0.7. Publishing pins every
  name permanently, so the facade's `pub use` list wants going through once, asking of each
  name whether it is contract or an implementation detail that escaped.

- **`merge` and `check` scaling**, the last two pure operations measured only through the
  command, where parsing dominates. And **`doc-output` covers 46 of 168 blocks** — teaching it
  to build a chapter's intermediate files would roughly triple that.

- **Mutation testing beyond the packer**, deprioritised rather than dropped. 0.8 found that
  asking "what does the suite trust?" locates oracles faster than a sweep does, so the next
  pass should be a targeted audit of `smysl-core`'s codec invariants rather than 1 922 mutants.

- **Publishing**, when the gates above say so. Eleven crates or none; the readiness work is
  done and the dry run is clean.

---

## 0.8.0 — 2026-08-02

### Removed

- **The local-improvement pass.** Step 3 of §18.3 downgraded the least valuable depth and
  spent what that freed on breadth. It was measured and it lost: across 28 000 generated
  packs it changed 26, and **22 of those 26 were worse** by the value function it exists to
  maximise. Four were better.

  It fired on 0.09% of packs, which is why two earlier measurements read it as harmless
  rather than harmful. 0.3.0 found that turning it off changed runtime by under 1% and
  concluded it was not the bottleneck; mutation testing found `improve -> false` survives.
  Neither could see the packs getting *better* without it.

  Removing it changed **no** recorded selection — the golden file across nine fixtures at six
  budgets is byte-identical — because the corpus never reaches a case where it fired. That is
  the same reason it escaped every fixture for eight releases.

  `IMPROVEMENT_PASSES` and `PackRequest::improvement_passes` went with it; a knob for a pass
  that no longer exists is worse than no knob. 123 lines gone.

### Fixed

- **Two oracles were never audited**, found by asking of each thing the suite *trusts*: is
  there a test that it ever says no?

  `satisfies_rule_l` is the second. It is asserted `.is_empty()` in two places and nowhere
  asserted to report anything, so an oracle returning `vec![]` would satisfy both — and the
  repair pass those tests exist to check would be unfalsifiable. A thread could come back with
  holes and every test would agree it had not.

  The hunt also cleared two: `conformance` is tested in both directions with the specific
  blocking code, and `Query::admits` is exercised both ways through the retrieval filter
  tests. Four candidates, two gaps, twenty minutes — a better rate than a 1 922-mutant sweep
  would have given.

- **`verify` was never audited.** It could be replaced with `vec![]` and every test passed —
  four assertions in this repository read `verify(...).is_empty()`, including the C1-C7
  property test and the `pack_constraints` fuzz target, and all four are satisfied by an
  oracle that never says anything. It is not the thing under test; it is what the other tests
  *trust*. Two tests now audit it, both confirmed to fail against the `vec![]` mutant before
  being trusted.

### Measured

- **Mutation testing over the packer core.** 49% of viable mutants survived on the
  best-tested file in the codebase — 11 constraint properties, a golden file, two fuzz
  targets and a brute-force differential oracle. `Pack::is_optimal` could be replaced with a
  constant; so could every comparison operator on `Ordered`, the type introduced in 0.6.0
  with the argument that one implementation of the order "removes the question rather than
  testing around it". Right about consistency, wrong about correctness.

  After the tests above and the removal below, the interesting residue is gone: 22 of the 46
  remaining survivors were inside `improve`.

### Added

- **`Documentation/READINESS.md`** — seven gates on publishing, each done or with a next
  action. "Not production ready" was the answer twice and said nothing about what would
  change it. The largest gate is that nobody has implemented the format from the spec alone.

### Documentation

- Everything at 0.8.0. The manual's packing chapter no longer describes a local-improvement
  pass, and says why it went. The architecture RFC gains "Closed in 0.8".

What is carried, and what it is waiting on:

- **The quoting coarsening**, which turns out to need a design decision before it needs a
  model. There is no flag controlling the quote requirement — `quote` is an optional property
  in Appendix C and the behaviour comes from the prompt — so the two arms have to be
  constructed before they can be compared. An hour if the difference is only in the prompt.
- **Anthropic's mapper is unverified.** OpenAI's is now down to "confirm the translated schema
  is accepted"; Anthropic has had no equivalent narrowing. It uses `ToolForce` rather than
  `JsonSchema`, so it has a different set of unknowns and no counted defect yet — which means
  the first useful step is reading its shape against the documentation the way OpenAI's was,
  not waiting for a key.

- **Publishing, when it is production software.** Eleven crates or none: the single-crate
  restructure was abandoned in 0.7.0 and the reasoning is recorded there. The readiness work
  is done and `cargo publish --dry-run` is clean.

- **`smysl-embed` has no live-gate equivalent.** The semantic evaluation runs from
  `make eval-semantic` and a model directory, and nothing in CI exercises it. That is the
  same shape as a fuzz target nobody runs, and the answer is probably a small committed model
  rather than a download in CI.

---

## 0.7.0 — 2026-08-01

### Added

- **`smysl-embed`: semantic retrieval behind the `Retriever` seam**, off by default under
  `--features semantic`. Model2Vec static embeddings — a token maps to a vector and a
  sentence is a pooled lookup, so there is no ONNX Runtime, no downloaded binary and no `ort`
  release-candidate pin. `model2vec-rs` is taken without `hf-hub`, so nothing here reaches
  the network: a model is three files the operator already has.

- **`make eval-semantic`**, and one query set instead of two. The twenty queries live in
  `fixtures/retrieval/queries.tsv` and both evaluations read it, because two scores measured
  on different questions say nothing about each other.

### Measured

**The hosted provider gate ran against DeepSeek and Gemini**, and the difference between
structured-output modes turns out to be visible rather than theoretical:

| provider | mode | path | units | calls | degraded | tokens |
|---|---|---|---:|---:|---:|---:|
| gemini | `json-schema` | json-ast | 4 | 1 | 0 | 624 |
| deepseek | `json-mode` | surface | 1 | 3 | 1 | 1572 |

Gemini structurally guarantees the shape, so one call returns four conformant units. DeepSeek
guarantees only *valid JSON*, not valid-against-this-schema, so it took three calls, produced
one unit and degraded it under rule I — at two and a half times the tokens. That is the
`StructuredMode` distinction earning its place in the API: a provider that cannot promise
conformance is not a slower version of one that can, it is a different pipeline.

**The OpenAI strict-mode defect is fixed**, and it never needed a key. `strict_schema` in
`openai_compat.rs` translates Appendix C into the subset strict structured outputs accepts:
every property required, optionality moved from omission into a nullable type,
`additionalProperties: false` stated at every level, and the `minLength`/`pattern`/`allOf`
constructs strict mode rejects dropped — unenforced by the provider, still enforced by
`check`, which is where rule M and the shape rules were always going to decide.

Translated at the boundary rather than by changing Appendix C, because the shared schema is
what Gemini and DeepSeek receive and both work with it. A vendor's requirement belongs in
that vendor's mapper. Verified live afterwards: both still run clean.

Tested against the *real* Appendix C rather than a miniature — the defect was counted on that
schema, so a transform satisfying a toy version would have fixed nothing. The test also
asserts the eleven-and-three shape, so if Appendix C changes and the mapper does not, it says
so.

What a key would still add is confirmation that the translated schema is accepted. That is a
smaller and better-defined question than the one that was blocked, and it is the whole of
what remains for OpenAI.

**The suspect, as it was found.** `openai.rs` has warned in its
own header that strict structured outputs require every key in `properties` to appear in
`required`. The shared schema declares eleven properties and three required — `type`, `gist`,
`status` — so eight are missing, and a strict request would be rejected outright rather than
degrading. That is now a fact about our schema rather than a suspicion about their API, and
it can be fixed and asserted statically: the OpenAI mapper should *transform* the schema into
strict form rather than passing it through, since making the shared schema strict would change
what Gemini and DeepSeek receive, and both currently work.

**Semantic retrieval works, and it is worth the model file.** Over the same twenty queries,
`potion-base-8M`:

| class | engine | recall@5 | MRR | P@1 |
|---|---|---:|---:|---:|
| Paraphrase | lexical | 0.75 | 0.41 | **0.12** |
| Paraphrase | semantic | 0.88 | 0.67 | **0.50** |
| Identifier | lexical | 1.00 | 1.00 | **1.00** |
| Identifier | semantic | 1.00 | 0.88 | 0.75 |
| ALL | lexical | 0.90 | 0.74 | 0.60 |
| ALL | semantic | 0.95 | 0.84 | 0.75 |

Precision-at-one on paraphrase goes from 0.12 to 0.50 — four times better on the exact metric
that justified building this. `claim` recall rises 0.67 → 0.83 and its MRR 0.29 → 0.64. The
prediction that lexical would keep identifiers held: 1.00 against 0.75.

**The first hybrid was worse than semantic alone** — 0.78 MRR against 0.84 — which was not
the prediction. It cleared its assertion, because that only asked it to beat lexical, and it
lost to the engine it was built on.

A design error rather than a tuning problem. It routed by kernel type *when the query carried
a `kinds` filter*, and merged both engines on rank when it did not. No query in the
evaluation carries a filter and few real ones will — a caller who knew which kind they wanted
would usually not be searching — so the dispatch it was designed around was never exercised,
and what got measured was the merge, which pulls good ranks down by averaging them with bad
ones.

**Rewritten to route on the query, which is the information available when the decision has
to be made.** An identifier-shaped query — one token carrying a separator, like
`pool.wait_ms` — goes to lexical; everything else goes to the embedder; an explicit `kinds`
filter still refines it. There is no merge, and its absence is pinned by a test.

| | recall@5 | MRR | P@1 |
|---|---:|---:|---:|
| lexical | 0.90 | 0.74 | 0.60 |
| semantic | 0.95 | 0.84 | 0.75 |
| **hybrid** | 0.95 | **0.87** | **0.80** |

It now takes the best of each on every class: perfect on identifiers where lexical is,
perfect on echo and 0.50 on paraphrase where the embedder is, and `Data` back to 1.00 MRR
where routing on kind alone had left it at 0.75.

The assertion is now the property that failed rather than the weaker one that passed: routing
must never lose to *either* engine it routes between. That is the whole promise of dispatch,
and it is what the first version broke while passing its test.

What is queued, in the order I would take it:

- **The semantic retrieval backend**, deferred here from 0.6.0 with a number waiting for it.
  0.5.0 measured where one helps — `claim`, `finding` and `hypothesis`, where a paraphrased
  query ranks the right unit first once in eight — and built the seam it sits behind, so what
  is left is the work rather than the design: a new impure crate, a model-distribution story,
  and the evaluation re-run per kernel type to show it *beat* 0.12 rather than merely
  arrived.

  `model2vec-rs` remains the candidate on unchanged grounds: pure Rust, no ONNX Runtime, no
  `ort` release-candidate pin, and static embeddings that are a table lookup rather than a
  forward pass, so they reproduce across machines. It dispatches by kernel type rather than
  replacing BM25, which is already perfect on identifiers and on `evidence`.

- ~~**One crate instead of eleven.**~~ **Abandoned.** The reason to do it was to publish a
  single crate rather than eleven, and publishing is not happening until this is production
  software — so the restructure would be paying a cost now for a benefit that has no date.

  What it would have cost is clearer than when it was proposed. Not rule B, which survives
  either shape: it is already stated about the facade, so `check-purity` would test the same
  property against one crate. What it costs is the crate boundary as a *compiler-enforced*
  constraint. Today `smysl-core` cannot reach `clap` because it does not depend on it, and
  `smysl-retrieve` is pure because `bm25` is its only dependency — facts the build enforces
  without anyone remembering to. Afterwards those become `#[cfg]` discipline, and the purity
  gate would check one tree instead of seven.

  0.6.0 is also an argument against. The dependency-cycle and reserved-filename defects were
  both found *because* the crates are separate and `cargo publish --dry-run` had something
  per-crate to check.

  So: eleven crates when publishing happens, or none. If the eleven-crate listing is the real
  objection, that is a packaging preference to weigh then, against a restructure whose cost
  is paid in enforcement rather than in lines.

### Documentation

- Everything at 0.7.0, and the semantic backend is taught rather than only shipped. The
  manual gains "When words are not enough" beside the lexical retrieval section, with the
  wrong turn kept on the page: the first routing scored worse than the embedder alone and its
  test passed, because the test only asked it to beat the lexical engine. The rationale says
  the same thing to a reader deciding whether to adopt any of this.

- **Publishing, when it is production software.** Not before. The readiness work is done and
  the dry run is clean; both names are held back on purpose, and the README says so and says
  what to do in the meantime.

- **The quoting coarsening.** A fixture that yields five or six units yields three once each
  must carry a quotable span. Observed once, never explained. One experiment settles it, and
  the experiment needs a model.

- **OpenAI and Anthropic mappers**, still blocked on credentials. The risk has grown rather
  than shrunk since Appendix C gained `relations` and `quote`.

---

## 0.6.0 — 2026-07-31

### Added

- **`pack --query TEXT`** — the composition retrieval was built for, of which 0.5.0 shipped
  only half. Retrieval answers which units are relevant; packing answers what fits without
  holes. The hits become `--focus`, so pack pulls in their grounds, deps and live rebuttals
  and returns an argument rather than excerpts that scored well. Failing loudly when the
  focus does not fit is deliberate; `--query-limit` defaults to 3, because each focused unit
  drags its closure in behind it.

- **`\#`, `\//` and `\\` escape a body or detail line.** A line opening with a comment
  marker is a comment wherever it sits, so a body could never *begin* one — and a Markdown
  heading and a line of C++ both do. 0.2 documented the limitation, 0.4 fixed the
  header-value half, and until now the line was dropped in silence. Only those three
  sequences, only at column 0.

- **`make seed-fuzz`**, and CI seeds the sixty-second gate with the corpus fixtures and every
  input that has ever broken something. `make fuzz-long` stays **cold** on purpose: 0.4.0's
  and 0.5.0's findings both came from a cold run landing where a warm corpus does not go.
  Seeded runs reach 5 339 coverage points against 3 023 cold.

### Fixed

- **A thread's gist kept its leading whitespace** — the 0.4.0 unit-gist fix in the sibling
  path, which it never reached. Found within a minute of seeding the fuzzer.

- **Six free-text fields reached the CBOR encoder without NFC normalisation.** The encoder
  asserted the invariant in debug and trusted it in release; constructors establish it for a
  unit's gist, body and detail and for nothing else — not a thread's gist, a step's note, a
  view's intent, a granularity profile, a source reference or a pack estimator. Two had
  already been found by fuzzing in two separate releases, each fixed by normalising in one
  more constructor, which is a class being treated as a list. **The encoder normalises now**,
  which costs a quick-check on text about to be BLAKE3'd anyway and makes the implementation
  match what `SMYSL_FORMAT_SPEC.md` already promised.

- **`ui` was documented as a stub and is not one.** `smysl-tui` is a working crate with its
  own tests, the `tui` feature is in the default set, and the command runs given a terminal.
  Appendix A said otherwise, so the purity table and the changelog did too — and so did I,
  twice, while planning work around removing it. It has its flag table now.

### Changed

- **`SMY-W305` is emitted; `SMY-W306` is deleted.** Two releases of "documented as
  unreachable" is a holding pattern rather than a decision. The ledger had recorded whether
  each token count came from the provider or from our own estimate since the provider layer
  landed and nothing surfaced it, so `usage` warns. W306 described a usage threshold that
  does not exist, and inventing the feature to justify the code would be the wrong way
  round. The registry is 51.

- **`tui` left the default feature set.** It works and it is tested — that was settled
  earlier this cycle when the "stub" claim turned out to be false — but `ratatui` and
  `crossterm` in every default build is a cost an embedder who only calls the library never
  opted into. `--features tui` for anyone who wants the browser; without it the command says
  so rather than pretending. A default `cargo install smysl` no longer pulls either crate.

- **The CI matrix and `make test-matrix` had drifted**, and now agree at nine rows. Two of
  them are new and neither was reachable from any other: `--no-default-features --features
  cli` and `--features tui`. Default brings `ingest` with it, so a function used only by an
  ingest command is live under default and dead under `cli` alone — which is exactly the
  dead-code error that failed the determinism job for three releases under `-D warnings`.
  `--all-features` cannot substitute for either, being the combination nobody ships.

- Retired-RFC references removed from a user-facing error and from `diag.rs`.

### Performance

- **`pack` is linear when the budget binds.** It was quadratic — 2.81, 3.46, 3.87x per
  doubling, converging on 4.0 — because the greedy ran one round per unit admitted and
  scanned every remaining candidate each round to pick a global best. 0.3.0 removed the
  *pricing* from that scan by caching it behind an exact invalidation index; what remained
  was the scan itself.

  The scan is now an ordered set keyed on the choice, so a round is a pop rather than a walk.
  Measured: 2.07, 2.20, 1.94, 2.23, 2.11x per doubling out to 8 000 units, and 2.05 ms at
  2 000 units against 18.54 ms — **9x**, and the gap widens with size.

  Two things made it sound where the textbook lazy greedy is not:

  - The order is now *one named type*, `Choice`, used by nothing else. The risk in this
    change was a heap reproducing three of the four tie-break terms and producing packs that
    are legal, deterministic, monotone in budget and **different** — and the suite could not
    have caught it, since no corpus fixture ties on density without also tying on salience.
    Having one implementation of the order removes the question instead of testing around it.
  - Affordability is checked at pop, and a candidate that cannot be afforded is *parked*
    rather than dropped. `used` only grows, so an unaffordable candidate can never become
    affordable — **unless its marginal cost falls**, which happens exactly when something in
    its obligation is selected and therefore already paid for. That is a dirty event, so a
    parked candidate is reconsidered when and only when it is dirtied. This is the
    non-monotonicity that makes the naive lazy greedy unsound here.

  Verified byte-identical: `tests/golden-packs.txt` records what `pack` selects across nine
  fixtures at six budgets, and not one line moved.

### Measured

- **`salience` is linear**, isolated from parsing and process startup: 2.07–2.22× per
  doubling, 3.96 ms at 16 000 units. It was the last pure operation whose per-call cost had
  only ever been *assumed* — the same assumption that was twice wrong about `pack`.

- **`pack` with a binding budget** was measured at 3.87x per doubling and 18.5 ms at 2 000
  units, which is what made the fix above worth doing and is how it was shown to have
  worked. Measured in process rather than inferred from command timings dominated by
  parsing.

  Both live in `crates/smysl-graph/tests/scaling.rs`, `#[ignore]`d: a measurement, not a
  gate. Timing assertions on shared runners fail for reasons unrelated to the code, and a
  test that cries wolf gets muted.

### Packaging

- **Not published to crates.io, deliberately.** The readiness work below was done and the
  dry run is clean, but publishing permanently reserves both the name and every version
  number, and 0.6.0 is not something to hand people as production software. The names are
  held back until it is; the README now says so, and says how to build from source or depend
  on a tag in the meantime, which it had never said at all.

- **Publish-readiness, checked with a dry run rather than after the first bug report.** Three
  things it found: `src/types/aux.rs` is a reserved device name on Windows, so `smysl-core`
  would have been unbuildable there from the day it was published (now `annex.rs`);
  `smysl-graph` had a circular dev-dependency on `smysl-pack`, which depends on it normally,
  so neither could be published first (the pack measurement moved to the crate whose
  operation it measures); and the root package would have shipped 8 MB of PDFs and images
  against a 10 MB limit, so most of a consumer's download would have been a book they never
  unpacked.

### Deferred to 0.7.0

- **A semantic retrieval backend.** 0.5.0 produced the measurement that says where one would
  help — `claim`, `finding` and `hypothesis`, where a paraphrased query ranks the right unit
  first once in eight — and 0.5.0 also built the seam it would sit behind. What is left is a
  cycle's worth of work rather than an item: a new impure crate, a model-distribution story,
  and the evaluation re-run per kernel type to show it actually beat 0.12 rather than merely
  arrived.

  `model2vec-rs` remains the candidate, and the reasoning has not changed: pure Rust, no
  ONNX Runtime, no `ort` release-candidate pin, and static embeddings that are a table
  lookup rather than a forward pass — so they are reproducible across machines, which
  matters more here than accuracy at the margin. It would dispatch by kernel type rather
  than replace BM25, because BM25 is already perfect on identifiers and on `evidence`.

  Deferred deliberately, with a number waiting for it. That is a better position to start
  from than most work gets.

### Still carried

- **The quoting coarsening.** A fixture that yields five or six units yields three once each
  must carry a quotable span. Observed once, never explained — it may be the prompt or it may
  be inherent to anchoring a unit to text it can quote. One experiment settles it, and the
  experiment needs a model, so it sits behind the same credentials question as the mappers.
- **OpenAI and Anthropic mappers**, still blocked on credentials. The risk has grown rather
  than shrunk since Appendix C gained `relations` and `quote`.

Everything else carried out of 0.5.0 was closed in this cycle: `pack`'s scan, the fuzz
corpus, the body-line escape syntax, both dead diagnostic codes, the `ui` decision, and
`salience`'s per-call cost — which was the other half of the "two measurement gaps" and is
now measured rather than assumed.

---

## 0.5.0 — 2026-07-31

### Added

- **Retrieval: `smysl-retrieve` and `smysl find`.** A seam first and an engine second.
  `Retriever` is a trait; the shipped implementation is BM25 over gists, bodies and details.

  It indexes the **gist** principally, and that is the load-bearing idea. A unit's payload
  may be a stack trace, a metric series, a diff or a page of prose, and no one way of
  searching covers all four — but every unit carries a gist because the format requires one,
  and a gist is a sentence about whatever the payload is. Payload heterogeneity never
  reaches the index. The cost, stated rather than buried: retrieval quality is bounded by
  gist quality, which is an ingest concern.

  The crate is **pure** and under the purity gate, which is unusual for search. `bm25` is
  taken with `default-features = false`, dropping three things that were each wrong here:
  the default tokeniser stems and strips stop words, destroying identifiers when a payload
  may be source code; language detection would make tokenisation depend on the corpus, so a
  store could tokenise differently as it grew; `parallelism` puts a rayon reduction inside a
  result that must not vary. The tokeniser is ours — split on whitespace and punctuation,
  split camelCase/snake_case/kebab-case while keeping the whole token, lowercase, nothing
  else.

  `--kind` and `--min-status` are filters on the query rather than trimming applied
  afterwards, because trimming a ranked list silently returns fewer than asked for.

- **Measured, not asserted.** 20 queries over the corpus in three classes, none reusing a
  gist verbatim:

  | class | recall@5 | MRR | P@1 |
  |---|---:|---:|---:|
  | shared vocabulary | 1.00 | 0.94 | 0.88 |
  | paraphrase | 0.75 | 0.41 | 0.12 |
  | identifier | 1.00 | 1.00 | 1.00 |

  By kernel type, `evidence` and `data` score 1.00 and `claim` 0.67. Concrete things are
  findable by name; an interpretation is phrased in whatever words its author reached for.
  So a semantic backend would pay on `claim`, `finding` and `hypothesis` and add nothing on
  the rest — narrower and better founded than "add embeddings", and the reason the seam is a
  trait rather than a second engine. Caveats worth keeping: 20 queries is small, the corpus
  is small, and the same hand wrote the queries and the retriever.

- **A fourth fuzz target, `pack_exact`** — exact packing against brute-force enumeration,
  which is obviously correct and obviously too slow, and therefore an oracle rather than a
  second opinion. Both directions are asserted: falling short of the optimum is a weaker
  pack, but *exceeding* it means the search and the verifier disagree about feasibility.

- **Three fuzz targets over the algebra**, not just the parsers. 0.4.0 found eight defects
  in minutes, every one in `surface` or `cbor` — the only two subsystems with a target.
  That is a fact about where anyone was looking, not about where bugs live.

  `merge`, `pack` and `thread` already assert the properties that matter. What drove that
  generation was a fixed-seed xorshift over 100–200 blind rounds: the same cases every run,
  forever, with no coverage feedback. The properties are unchanged; the search is not.

  - `merge_algebra` — commutative, associative, idempotent. If any fails, two peers
    gossiping in different orders reach different stores and the mesh needs coordination,
    which is what rule U exists to avoid. Nothing reports it: each peer believes itself.
  - `pack_constraints` — C1–C7 via `verify`, the budget, and value monotone in budget.
    *Value*, not unit count: a larger budget may take one expensive unit over two cheap
    ones, so asserting the count would fail on correct packs.
  - `pipeline` — guarantee A1 across `check`, `salience` and `derive_thread`, plus rule L
    on every derived thread. A1 had only ever been tested on the two parsers, which is the
    narrow reading: a store from another agent has been through a parser, but the graph it
    describes is still adversarial.

  These are where a defect is quiet. A wrong pack still packs.

### Fixed

- **A duplicate known-field key leaked into the unknown-key payload and broke the round
  trip.** `HObject::take` removed only the first entry under a key, and whatever a caller
  leaves behind becomes the payload under rule X. A header with two `deps` parsed as one
  real `deps` plus a second carried as an extension — which the writer emitted as a plain
  `deps:` line, because that is the key's name. The next parse found one, took it as the
  field, and the payload came back a key short. `take` now removes every entry and returns
  the first, which is already what `object_to_payload` does when it dedups by encoded
  bytes: one rule for duplicates, everywhere.

  Found by the surface fuzzer **in CI, not locally** — a warm corpus and a cold one explore
  differently.

### Changed

- **CI runs on `dev/**` branches, not only `main`.** It used to see a cycle's work for the
  first time at the release commit, which is how the determinism job stayed red across
  0.2.0, 0.3.0 and 0.4.0.
- **`make test-matrix` sets `RUSTFLAGS=-D warnings`** and covers `--features cli`. Neither
  was true before, and between them they hid a dead-code error for three releases.
- **Failing CI jobs report why in an annotation**, and the fuzz jobs emit the crash input as
  base64. Job logs and uploaded artifacts both need admin rights on this repository, so a
  failure otherwise read as "exit code 1" and nothing else.

### Fixed (CI)

- **The determinism job was never a determinism failure.** `read_input` is called only from
  `cmd_ingest`, which is `#[cfg(feature = "ingest")]`, and carried no gate — so it was dead
  code in any build without `ingest`, and `-D warnings` made that a build error. The
  determinism job builds exactly that configuration to run its permutations, so the build
  failed, `pack` exited 101, and the job reported rule D. Three releases running.
- **`make doc-output` could only run on one laptop.** The script hardcoded an absolute path
  and `chdir`'d to it, so CI raised `FileNotFoundError` before comparing a transcript. It
  then found one real skip: `attest` needs `--features local`, which `doc-output` does not
  build.

### Documentation

- **Everything is at 0.5.0**, and `find` is taught rather than merely shipped — in the
  salience chapter, because the pairing is the point: `salience` ranks by structure and
  never reads a word, `find` ranks by words and never looks at the graph. It states where it
  is weak with the measured numbers. The rationale gains "Finding things again"; the format
  guide gains the duplicate-key rule and a callout saying plainly that it is *not* the
  contract; the architecture RFC gains "Closed in 0.5".

- **`make doc-output` runs 45 transcripts**, up from 43 — see the caption-regex defect above.

Carried forward, in the order I would take them:

- **A semantic retrieval backend**, now that there is a measurement saying where one would
  help. `model2vec-rs` is the candidate: pure Rust, no ONNX Runtime, no `ort` release-
  candidate pin, static embeddings that are a lookup rather than a forward pass and so are
  reproducible across machines. It would sit behind `Retriever` in an impure tier, never in
  the pure crates, and dispatch by kernel type rather than replacing BM25.
- **The two measurement gaps.** The quoting coarsening — a fixture that yields five or six
  units yields three once each must carry a quotable span — is observed once and never
  explained; it may be the prompt or it may be inherent to anchoring a unit to text. One
  experiment settles it. And `salience` is now the only pure command whose per-call cost has
  never been characterised, though it measures linear.
- **`pack`'s remaining scan.** Still super-linear when the budget binds (~3x per doubling):
  the pricing is cached but the per-round scan over candidates is not. Removing it needs an
  ordered structure, where the subtlety is that affordability moves with `used` even for
  candidates nothing has touched.
- **Seeding the fuzz corpus.** Each CI run starts cold and reaches less far in its sixty
  seconds than a local run does. The corpus in `fuzz/artifacts/` is the obvious seed.
- **An escape syntax for a body line opening `#` or `//`.** 0.2 documented the limitation and
  0.4 fixed the header-value half of it, which leaves the body case as the only place it
  bites.
- **`W305` and `W306`**, the two diagnostic codes with no emission site. Documented as
  unreachable in 0.4; emit or delete.
- **OpenAI and Anthropic mappers**, when credentials exist. Still blocked, and the risk has
  grown rather than shrunk since Appendix C gained `relations` and `quote`.
- **`ui`**, which this list called a stub through 0.5.0 and which is nothing of the sort —
  a working TUI, in the default feature set. Corrected in 0.6.0; the open question is
  whether it earns its dependencies, not whether it exists.

---

## 0.4.0 — 2026-07-30

### Fixed

Seven round-trip and determinism defects, all found by fuzzing. Each one broke the same
promise from a different side: `parse -> write -> parse` must be a fixed point, and a uid
must name exactly one byte string. Regression tests pin every case.

- **Two labels naming one unit lost a coin toss.** Identity is content, so two declarations
  with the same gist, status and grounds *are* one unit — and surface syntax has room for
  one name on the declaration. The parser kept whichever came first in the file; the writer
  kept the alphabetically first. So the surviving name changed on each pass. Both now keep
  the canonically first, and the loss is reported as `SMY-W054` rather than happening in
  silence. Invisible before `Record::LabelBinding` existed, because nothing carried a label
  through the wire to notice it going missing.

- **A carriage return at the end of a body line eroded one per round trip.** The lexer
  stripped exactly one `\r` before a `\n`, so `x\r\r\n` became `x\r`, which the writer
  emitted as `x\r` + `\n`, which the next parse read as a plain CRLF. All trailing carriage
  returns are now stripped: line endings are not content. Were they, the same document
  checked out with CRLF endings would hash differently from one checked out with LF, and
  identity would depend on a git config. A `\r` *inside* a line still is content, and stays.

- **Unknown header keys were written unquoted.** Values went through the quoter; keys did
  not. Rule X keeps an unknown key verbatim and nothing constrains what a peer puts in one,
  so a key holding a `:`, a `}` or a newline tore the header apart and the **whole unit**
  vanished on re-parse.

- **A header value starting with `#` or `//` was written unquoted**, so the comment syntax
  added in 0.2.0 ate the rest of the line — closing brace included, taking the unit with it.
  Only a *leading* marker is a hazard: the comment skip runs before a value begins, while a
  quoteless value runs to `,`, `}`, `]` or end of line without stopping at either marker.
  So `grafana://board/12` needed no quotes, and still gets none.

- **Unknown header text was not NFC-normalised before hashing.** Every other text field is
  normalised once on construction, so the encoder only asserts the invariant; unknown keys
  and their string values reached it straight from the parser. A debug build tripped the
  assertion. A **release build encoded the non-NFC text**, so two peers writing the same
  content in different Unicode forms produced different uids — rule D failing silently, in
  the build people ship.

- **A gist assembled from continuation lines kept a leading space.** The writer emits `~ ` +
  gist and the reader strips the sigil *and* the whitespace after it, so the space was eaten
  on re-parse and the uid moved with it. The assembled gist is now trimmed.

- **`PackInfo` and `View` decoded with defaults for mandatory fields.** The encoder writes
  them unconditionally, so `[7, {0: 0}]` was accepted and re-encoded as a four-key map: two
  distinct byte strings mapping to one record, which is exactly what stops a uid from being
  an identity. Both now reject a record missing a field the encoder always emits. A sweep
  over every record kind and low key guards the invariant generally — an earlier version of
  that sweep probed with integers alone, never entered `dec_view` (whose key 0 is a text id),
  and let the second instance survive another fuzz run.

- **`quantise` returned infinity for a large payload float**, which is not a multiple of
  1/1024 and not finite, so the CBOR writer's `debug_assert!(is_quantised(q))` fired. In
  release the assertion is compiled out and the infinity was written to the store instead —
  a value the codec's own contract forbids, emitted silently, which is the worse half.
  Reachable from a `.smy` file, so from a document another agent hands you.

  `quantise` is now total, saturating at the largest magnitude constraint 4 can express. A
  value that large has no faithful representation under the constraint, so there is nothing
  to preserve.

- **A third decoder defaulted a field the encoder always writes.** `dec_schema_decl`
  defaulted `version`, so `[8, {0: "smysl.kernel/x"}]` decoded and re-encoded as a two-key
  map. Same defect as `dec_packinfo` and `dec_view` in 0.4.0 — and it survived the sweep
  written to generalise that fix, because none of the sweep's probe values parsed as a
  `SchemaId`, so the record type was never entered at all.

- **Five vacuity defects in test infrastructure**, none in the product, all found by
  asserting the shape of what a test is handed rather than trusting a clean run:

  - the fuzz store generator produced **no relations**, so the join-semilattice laws ran
    against stores with no rebuttals, supersessions or contentions — the entire class the
    laws are about;
  - then **no unit above L0**, so `pack` had no level to choose and its search collapsed to
    in-or-out;
  - `exact.rs` never generated a `detail`, so **L2 was never in the search space** where
    branch-and-bound is checked against brute force;
  - the decoder sweep never entered `dec_schema_decl` (above), and the first repair of that
    sweep was itself vacuous — a `0x73` header for a sixteen-byte string, so the value
    failed to decode and the record type stayed skipped;
  - `make doc-output`'s caption regex stopped at the first `"`, so two newly written
    transcripts containing a quoted argument were skipped the moment they were written.

  Every one is now pinned by a check that was verified to fail before it was trusted.

### Changed

- **RFC SMYSL-1 is retired**, and `SMYSL_FORMAT_SPEC.md` is normative in its place — under
  250 lines covering identity, deterministic CBOR, record framing, the surface round-trip
  fixed point, rule X, the twelve rules, the conformance classes and the version axes. The
  RFC was the product idea rather than doctrine; reconciling the code back to it would have
  been fidelity to a plan nobody holds. `RFC_PROPOSAL.md` becomes a design log rather than a
  work list — nothing in it was ever outstanding.

  Two claims written from memory were wrong and corrected against the code: the canonical
  uid text form is 52 base32 characters with a 26-character display form, and the
  conformance classes are **not a ladder** — C-Merge adds lifecycle obligations to C-Consume
  and does not subsume C-Produce.

- **The fuzz CI job blocks.** It ran with `continue-on-error` through the 0.4 cycle while it
  worked off the backlog it discovered on its first run. That backlog is clear, both targets
  run for minutes without a finding, and every case is pinned by a regression test.

### Documentation

- **Three wired subcommands were missing from Appendix A entirely.** `import`, `relink` and
  `compact` were wired in SM-P15 and the appendix was never extended, while its opening
  paragraph claimed the table could not drift from the binary. It could, and it had. All
  twenty are now covered, and the purity table in Chapter 3 lists all twenty-one commands
  rather than seventeen.

- **`make doc-output` reports zero drift, exits non-zero on any, and runs in CI.** It had
  reported fifteen mismatches since it was written, which is why it was never made a gate —
  and every one of them was an artifact of the script rather than a stale manual:

  - it concatenated two separately-captured streams, which does not reproduce the order a
    terminal shows (`check --granularity` writes to stdout, then stderr, then stdout again);
  - a block quoting *one* stream — the usual shape when stdout is a store and the report
    goes to stderr — could never equal both;
  - a block eliding with `...`, or a caption abbreviated with `…`, or one annotated as a
    different build, was compared as though it were complete and literal.

  Fixed at the source rather than by loosening the comparison: it still catches a
  one-character change to a documented count, which was tested before the gate was turned
  on. A check with a permanent backlog of false positives teaches people to ignore it, and
  then it catches nothing when something real breaks.

- **The manual's round-trip section claimed too much.** It said no string it could find
  survived being written unquoted and came back changed, and offered that as evidence the
  guarantee was working. Four of the seven defects above are exactly that string. The
  section now carries the correction and what it costs: "I looked and could not find one" is
  a statement about the search, not about the code.

- **`SMY-W036`, `SMY-E307` and `SMY-W308` were emitted but undocumented**, and `SMY-W054`'s
  entry described behaviour it no longer has. Appendix D now matches the registry exactly,
  and says outright that `SMY-W305` and `SMY-W306` have no emission site rather than leaving
  a reader waiting for a diagnostic that cannot arrive.

- **The presentation was not in `make docs`**, so it was the one document that could drift
  without anyone noticing. It is now built with the other three.

### Known limits

- **The fuzz corpus is not seeded**, so each CI run starts cold and reaches less far in its
  sixty seconds than a local run does. Noted rather than quietly skipped: cold still catches
  the regressions the job exists to catch.

Carried forward, in the order I would take them:

- **The two measurement gaps.** The quoting coarsening — a fixture that yields five or six
  units yields three once each must carry a quotable span — is observed once and never
  explained; it may be the prompt or it may be inherent to anchoring a unit to text. One
  experiment settles it. And `salience` is now the only pure command whose per-call cost has
  never been characterised, though it measures linear.
- **`pack`'s remaining scan.** Still super-linear when the budget binds (~3x per doubling):
  the pricing is cached but the per-round scan over candidates is not. Removing it needs an
  ordered structure, where the subtlety is that affordability moves with `used` even for
  candidates nothing has touched.
- **An escape syntax for a body line opening `#` or `//`.** 0.2 documented the limitation
  rather than solving it. 0.4 fixed the *header value* half of this — a value starting with a
  marker is now quoted — which leaves the body case as the only place the limitation bites.
- **`W305` and `W306`**, the two diagnostic codes with no emission site. `W305`'s information
  already reaches users through the usage totals line; `W306` describes a threshold feature
  that does not exist. Emit or delete. Documented as unreachable in the meantime, so at least
  nobody waits for one.
- **OpenAI and Anthropic mappers**, when credentials exist. Still blocked, and the risk has
  grown rather than shrunk since Appendix C gained `relations` and `quote`.
- **The ~69 RFC divergences**, which are real debt and not a release feature.

---

## 0.3.0 — 2026-07-30

Format stays at `smysl/0.1`, kernel at `smysl.kernel/0.1`. Nothing on the wire changed.

The theme: **a flag the tool advertises is a flag the tool honours, or says it cannot.**
Twelve global flags are declared once and therefore appear in every subcommand's `--help`;
measured at the start of the cycle, `--output` was honoured by 3 of 9 commands, `--json` by 1
of 6, `--strict` by 1 of 8, and `--quiet` by none. The stability and performance work came out
of a scan run against that same instinct — check what is claimed, then measure it.

### Fixed — stability

Three defects found by a performance and stability scan, all reachable from input another
agent hands you. Every threshold below was measured against the built binary.

- **Stack overflow in the surface parser.** `object`/`array`/`value` recursed with no depth
  bound, so a deeply nested header **aborted the process** — `fatal runtime error: stack
  overflow` at roughly 5 000 levels. An abort is worse than a panic: it cannot be caught, so
  an embedder cannot contain it, and rule A1 promises no panics on untrusted input.

- **Stack overflow in the CBOR reader**, at roughly 20 000 levels. More serious than the
  above, for two reasons: CBOR is the wire format, so this is a store arriving from another
  agent; and the way in is `skip_item`, which preserves unknown keys — meaning rule X, the
  forward-compatibility mechanism, was the route to the crash.

  Both now refuse at `cbor::MAX_NESTING` (128), far above anything a real document produces
  — the deepest shape the kernel defines is three levels — and far below what threatens the
  stack. `CodecError::NestingTooDeep` is a distinct variant so a caller can tell "too deep"
  from "corrupt", reported as the existing `SMY-E004` rather than adding to the diagnostic
  registry mid-cycle.

- **Integer overflow in `--budget Nk`.** The multiply was unchecked: debug builds panicked,
  and release builds — the ones people ship — **wrapped**. `--budget 18446744073709552k`
  silently became 384 tokens, and `--explain` then reported 384 *as the budget*. A budget
  that quietly becomes a different budget is the exact silent-degradation failure this
  project argues against. Now refused as a usage error.

### Added

- **Both fuzz targets run in CI**, time-boxed to sixty seconds each. They existed from the
  start and nothing ever ran them, which is how the two stack overflows survived to 0.3 — a
  fuzzer finds that shape in seconds. `make fuzz` runs the same pair locally; `make
  fuzz-long` is the old unbounded behaviour.

- **`--strict` is honoured wherever a command has a warning to promote** — `merge`, `pack`,
  `thread` and `fmt`, where before only `check` and one branch of `render` acted on it. This
  book recommends `--strict` for CI gates, so a pipeline running `merge --strict` believed it
  would fail on a warning and would not.

  `thread` has no diagnostic report to threshold, so it keys on the condition it already
  prints: a role the schema requires that nothing could fill. The thread is still emitted —
  the caller asked for one — but the gate is told.

  `bundle` is untouched deliberately: it produces no diagnostics, so there is nothing for
  `--strict` to promote and honouring it is a no-op rather than a gap.

- **`--quiet` suppresses the summary line**, which is what its help always promised; it had
  only ever dimmed the progress bar. Diagnostics and exit codes are untouched on purpose — a
  quiet run that also swallowed its warnings would be a worse flag than one that did nothing.

- **`--json` is honoured by every command that reports something** — `diff`, `trace`,
  `salience`, `view` and `retract`, where before it was accepted and ignored. Only `check`
  implemented it, while all twelve global flags are declared once and therefore advertised
  in every subcommand's `--help`. A caller who read `--json` in `smysl trace --help`,
  passed it, and got prose had no way to learn the flag was never wired.

  `retract --json` carries `authorised` and `refusal`, which the text form reports on
  *stderr* where a machine reading stdout would never see them.

- **`tests/global_flags.rs`** asserts the matrix: every (command, global flag) pair is
  either honoured or explicitly refused, and silence is a failure. Fixing instances does not
  stop the class — the next flag added reaches every subcommand's help the moment it is
  declared — so the shape is pinned rather than the instances.

  `--json` is checked with a real parser, because the bug being guarded against is
  machine-readable output a machine cannot read.

### Fixed

- **`check --json` emitted invalid JSON.** It used Rust's `{:?}`, which renders a control
  character as `\u{1}` — no parser accepts that. A diagnostic message quotes document
  content, so an authored gist or a model's output through `ingest` could break whatever was
  consuming the stream. `json_escape` existed for exactly this, documented as shared by
  "every caller that emits JSON", and the one command emitting JSON did not use it. It was
  also not re-exported from the facade, so a library caller could not have used it either
  (rule A).

- **Six of nine commands advertised `--output` and ignored it.** The flag is global, so
  every subcommand's `--help` lists it; `fmt`, `pack`, `thread`, `view`, `salience` and
  `retract` wrote to stdout regardless. Silently: a caller who passed `-o` got an empty
  file, no diagnostic, and a terminal full of CBOR.

  `fmt`, `pack` and `thread` now write the file (`fmt` refuses more than one input, since
  one path cannot receive several documents). `view`, `salience` and `retract` print a
  report assembled line by line rather than one artifact, so they say `--output` is not
  honoured and point at shell redirection instead of pretending.

- **`bundle` and `pack` dropped label bindings**, so both came back with every reference
  spelled as a bare uid — the gap 0.2 closed for `merge`, still open in the two artifacts
  most likely to be handed to somebody else. `bundle` is the worse case: closure exists
  precisely so it can be given to a recipient with nothing else to read it against.

  Fixed in `Store::emit` rather than in the CLI, because a library caller building a bundle
  needs a readable one too (rule A). The record type was added in 0.2 and `emit`'s
  catch-all excluded it without comment.

- **`thread --derive` ignored `--format` and dropped the `@doc` header** — the identical
  `write_surface(None, …)` mistake `merge --format surface` shipped with.

### Fixed — performance

- **`pack` is no longer quadratic when the budget admits the whole scope**: 2 818ms to 26ms
  at 4 000 units, and linear thereafter. Reproduce with `scripts/bench-scaling.py`.

  Counting the calls found it. `closure::delta` ran 7.5 million times for 4 000 units,
  scaling exactly 4.0x per doubling, because the greedy is O(n²) *by construction* — one
  round per unit admitted, every remaining candidate re-evaluated each round to pick a global
  best. That is worth paying when the budget binds and worth nothing when it does not, which
  is why the pathology appeared in the *easy* case.

  So the greedy is untouched and skipped: if the whole scope fits at its top level, that
  selection is taken directly. Not a heuristic — value is monotonic in level and every
  closure constraint is trivially met by a selection that omits nothing, so if it fits there
  is nothing left to trade. Verified byte-identical against the previous implementation across
  every corpus fixture at seven budgets.

  **One user-visible change.** Under this path `--explain` reads `earned on density` for every
  unit, where the greedy would have credited some to `C3 rebuts …` or `C1 dep of …`. The
  C-reasons mean "dragged in by another unit's obligation under budget pressure", and on this
  path there was no pressure and nothing was dragged. The greedy's attribution also cannot be
  reproduced without the greedy: it depends on admission order.

- **Obligations are memoised.** `closure::required` is a pure function of `(uid, level)` — an
  obligation does not change as a selection grows, only the shortfall against it does — and
  the greedy re-walked the graph for every candidate in every round anyway. `closure::Needs`
  caches it, which is exactly output-preserving and takes the binding-budget case from 2 807ms
  to 2 041ms at 4 000 units.

- **The binding-budget case is 6-7x faster**, by caching each candidate's cost and value and
  recomputing only those a change can have touched. At 4 000 units: 1 924ms to 273ms at half
  the store's cost, 2 698 to 421 at 90%, 2 820 to 448 at 99%. Per-doubling growth falls from
  ~4.3x to ~3x.

  The invalidation is exact, not approximate. A candidate's figures depend on the selection
  *only* through its own obligation — `delta` filters the obligation by what is held and
  `weigh` prices each member against the level held for it — so raising a unit can disturb
  only candidates whose obligation mentions that unit, and every other cached figure stays as
  valid as it was. Verified byte-identical against the previous implementation across ten
  fixtures at eight budgets and two synthetic stores at five more, `--explain` included.

  A lazy greedy over stale densities was considered and rejected as **unsound**: density is
  not monotone under selection growth. When a member leaves a delta because the selection
  already covers it, density becomes `(dv - v_m)/(dc - c_m)`, which *exceeds* `dv/dc` whenever
  the departing member's own density was the lower. A probe found no violations on the corpus,
  which is evidence and not a guarantee — and a packer that silently chose differently would be
  a far worse defect than a slow one.

### Known limits

- `pack` is still super-linear when the budget binds (~3x per doubling): the greedy still
  scans every candidate each round, and only the *pricing* is now cached. Removing the scan
  needs an ordered structure over the cached figures, where the subtlety is that affordability
  moves with `used` even for candidates nothing has touched.
- `thread` still defaults to surface output where `merge` and `pack` default to CBOR, so it
  sits awkwardly against rule P. Changing the default would be right by the rule and would
  also change what every documented `thread --derive` example prints, so it is left for a
  decision rather than taken quietly.

### Carried forward from 0.2, in the order I would take them:

- **Measure `pack` and `salience` per-call cost.** They recompute over the whole
  store every call, with PageRank over the full adjacency. There is no evidence
  it bites and no measurement either, and the missing measurement is the actual
  gap — a benchmark that finds the knee, not an optimisation.
- **Diagnose the quoting coarsening.** A fixture that yields five or six units
  yields three once each must carry a quotable span. Observed once, never
  explained; it may be the prompt or it may be inherent to anchoring a unit to
  text it can quote.
- **An escape syntax for a body line opening `#` or `//`.** 0.2 documented the
  limitation rather than solving it.
- **OpenAI and Anthropic mappers**, when credentials exist. The risk has grown
  rather than shrunk: Appendix C gained `relations` and `quote`, and the mappers
  pass it through unchanged.
- **The ~69 RFC divergences**, which are real debt and not a release feature.

---

## 0.2.0 — 2026-07-29

Format stays at `smysl/0.1`, kernel at `smysl.kernel/0.1`. A record type was
*added*, which an older reader degrades rather than refuses, so nothing on the
wire changed incompatibly.

The theme, if it has one: **a document should survive contact with machines and
still be legible to the person who has to answer for it.** Every item below is
some version of that, and most of them started as a gap the previous release
knew about and had written down.

### Breaking

Small, but real. Each one changes something a script or a reader could depend on.

- **A labelled unit now yields two records**, so `check` reports more records
  than before — `F1-incident.smy` went from 13 to 21. `units` is the figure to
  read when you want to know how much document you have; `records` is the figure
  to read when you want to know what a merge or a round trip has to carry. All 34
  documented counts in the manual were re-measured.
- **Exit code `11`** joins the contract. A script testing `= 10` for "staged"
  should test `>= 10`.
- **A typo in a unit type is now a warning, not an error.** `@clai c/a { … }` was
  `SMY-E001` and is now `SMY-W010` naming `clai`. This is irreducible rather than
  a preference: a tool cannot distinguish a typo from a kernel type added next
  year, because the two are structurally identical. `--strict` restores the
  failure, and the message is more precise than it was.
- **`fmt` refuses `--check` and `--write` on a CBOR store** with exit `2` rather
  than reinterpreting them. `--check` asks whether *text* is spelled canonically
  and a log is canonical by construction; `--write` would convert a binary store
  to text in place.
- **Live tests no longer run on the strength of a key.** `SMYSL_EVAL_LIVE`,
  `SMYSL_INGEST_LIVE` and `SMYSL_DEEPSEEK` must be set. A credential in the
  environment is not consent to spend it.

### Added

- **`Record::LabelBinding`** (envelope type code 10). Labels now survive a store
  round trip. Before this they survived a parse and not a store, so a document
  that had been through `merge` came back with every reference spelled as a bare
  `b3:…` uid — valid, re-checking clean, and unreadable. That broke the format's
  central claim for exactly the multi-agent case it exists to serve, and the
  evaluation harness never saw it because it measures claims and hedges, not
  legibility.

  The binding is a separate record rather than a field because a label is not
  identity: inside hashed content, renaming one would produce a different unit.

- **Comment syntax**: `#` or `//` at column 0. Both markers, because an HJSON
  header inside a record already accepted both, so the surface had been
  rejecting between records what it accepted within one.

  A comment is a comment *wherever* it sits, including inside a body — which
  costs a body the ability to open a line with either marker. The reverse was
  implemented first and was worse: a body runs from the gist to the next record,
  so a comment between two records fell inside that range and became the previous
  unit's body, inventing content out of a note.

  No record carries a comment, so canonical form cannot reproduce one and `fmt`
  warns before dropping any — this project recommends `fmt --write` as a
  pre-commit habit, which makes silent deletion of a reviewer's notes the
  difference between a formatter and a hazard.

- **`SchemaId::UnknownKernel`**, so a kernel type added by a later version
  decodes, reports `SMY-W010`, and re-encodes byte for byte. It used to fail the
  whole record with `SMY-E004` — corruption, not degradation — while an unknown
  *record* type and an unknown *extension* type both degraded correctly.

  Decoding and surface parsing need opposite behaviour here and cannot share one
  function, so `parse_forward` is a second entry point; `parse` still refuses.

- **Exit code `11`, `StagedWithCorrections`.** `ingest` knew rule M had corrected
  the model and had no way to say so; under `--yes` it returned plain `0`, making
  the outcome most worth knowing about indistinguishable from nothing having
  happened. A refinement of `Staged` rather than a failure — the batch is intact
  and every corrected unit is in it.

- **`fixtures/corpus/F9-forward-compat.smy`**, so the degradation paths are in
  the conformance corpus rather than only in unit tests.

- **`scripts/verify-doc-output.py`** and `make doc-output`: replays the manual's
  documented commands against the real binary. The manual quotes ~190 command
  outputs and nothing checked them, which is how 34 of them went wrong at once.

### Fixed

- **`merge` ignored `--format surface`**, so a merged store was the one artifact
  nobody could read back: `fmt` takes surface text, and piping the log into it
  fails on invalid UTF-8. Fixing it exposed that `write_surface` emitted `@doc`
  headers nowhere and thread steps naming canonical uids that its own parser
  rejected — the writer was producing documents the reader refused.
- **`fmt` could not read a CBOR store** although `check` read both forms, which
  is odd for the command whose job is making a store readable.
- **`SMY-W014` was declared and never emitted.** An unknown record type was
  preserved in perfect silence. Preservation is rule X working; saying nothing
  about it is how a reader comes to believe they have seen the whole document.
  `SMY-W010` had a milder version of the same problem — it fired only when `--as`
  named a consumer profile, though a type this *build* cannot interpret is the
  stronger fact and does not depend on being asked.
- **Format sniffing** was duplicated in two places and adding comments broke
  both, one of them silently. Now one function.

### Not fixed, and why

- **`merge` does not persist the contentions it detects.** Reported as a bug
  during this cycle and it is not one: detection is not monotone, so writing a
  finding into an append-only log would make a stale detection permanent and
  break the associativity rule U promises. Detection stays a derived view of the
  union.
- **OpenAI and Anthropic mappers remain untested.** Blocked on credentials, and
  shipping untested network code is worse than shipping none.
- **`pack` and `salience` recompute over the whole store per call.** No evidence
  yet that it bites, and no measurement either — which is the actual gap. A
  benchmark, not an optimisation, is the next step.

### Known limits

- A body cannot open a line with `#` or `//`; there is no escape syntax yet.
- Sixteen counts in the manual were re-measured by rebuilding each example from
  the listing its chapter prints. Where a chapter does not print the file, a file
  of the shape the prose states was used instead.
- Exit code `11` is not in RFC Appendix E, and is recorded as a divergence.

---

## 0.1.0

Initial implementation: SM-P0 through SM-P15. Kernel data model, deterministic
CBOR codec, surface syntax, the check pipeline, exact packing, threads, six
render backends, the provider layer, the ingest boundary, and the evaluation
harness.
