# Changelog

Notable changes, and what they cost or bought. Versions follow semver on the
**crate**; the *format* version is a separate axis and moves only when the wire
format changes incompatibly. A crate major bump does not imply a format break,
and the facade asserts the two are independent.

---

## Unreleased — 0.4.0

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

### Known limits

- **The surface round trip does not hold for some input.** `parse -> write -> parse` returns
  different records for at least one document, found by the same fuzz run. Open, with the
  artifact in `fuzz/artifacts/surface/`. Not a regression: this is the first time either fuzz
  target has ever been run.

- The fuzz CI job is **reporting rather than blocking** until that is fixed. A gate that fails
  on a backlog it has just discovered teaches people to ignore it.

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
  rather than solving it.
- **`make doc-output` in CI.** It exists and nothing runs it, so the docs can drift between
  releases exactly as they did before it existed.
- **`W305` and `W306`**, the two diagnostic codes with no emission site. `W305`'s information
  already reaches users through the usage totals line; `W306` describes a threshold feature
  that does not exist. Emit or delete.
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
