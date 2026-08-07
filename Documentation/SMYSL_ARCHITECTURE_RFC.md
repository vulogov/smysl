# smysl — implemented architecture

**Status:** descriptive, not normative. The normative document is
[`SMYSL_FORMAT_SPEC.md`](SMYSL_FORMAT_SPEC.md), which is deliberately a fraction of this
one's length: interoperability needs identity, encoding and the rules, not an account of how
this implementation happens to be built.
**Describes:** the code at `main`, crate `0.14.0`, format `smysl/0.1`, kernel `smysl.kernel/0.1`.
**Compiled:** 2026-07-30, from SM-P0 through SM-P15, the operational-merit work after it, the
0.2 cycle (label bindings, comment syntax, forward compatibility), the 0.3 cycle (global
flags, nesting bounds, packer performance) and the 0.4 cycle (the fuzz backlog: seven
round-trip and determinism defects, and a fuzz job that now blocks).

The crate version moved and the format version deliberately did not. Nothing about the wire
format changed incompatibly in 0.2 — a record type was *added*, which an older reader
degrades rather than refuses — so `smysl/0.1` still describes what is on the wire. A crate
major bump must not imply a format break, and the facade asserts the two are independent
axes.

This document says **what the implementation actually does**. It is not RFC SMYSL-1 and does
not restate it: where the two differ, the differences are enumerated in
[`RFC_PROPOSAL.md`](RFC_PROPOSAL.md), and the RFC is the authority on intent while this file
is the authority on behaviour.

Everything below was read off the source rather than remembered. Where a number appears — ten
check passes, seven constraints, fifty-two diagnostics — it is a number a test pins.

---

## 1. The shape of the thing

A `smysl` document is a set of **units** — a claim, a piece of evidence, a question — each
carrying what it asserts, how confident it is, and what it rests on. Units are joined by
typed **relations**. That graph is the whole data model; documents, threads and views are
readings of it rather than containers.

The same bytes serve two audiences. The surface syntax is text a person edits; the wire
format is deterministic CBOR a machine merges. Neither is a rendering of the other — they are
two encodings of one record set, and the round trip is asserted on every corpus fixture.

Ten crates under `crates/`, plus the facade at the repository root — eleven packages:

```
smysl-core      kernel types, deterministic codec, surface syntax, diagnostics
smysl-graph     append-only store, index, adjacency, merge, salience
smysl-check     the ten check passes
smysl-pack      budget-bounded, closure-complete selection
smysl-thread    thread schemas and deterministic derivation
smysl-render    render IR, profiles, six backends
smysl-provider  the model boundary — the only crate linking a runtime or HTTP client
smysl-ingest    staging, repair, trust ceiling, recipes
smysl-tui       seven-pane terminal UI
smysl-eval      evaluation harness E1–E9 (not published)
smysl           facade: library plus the `smysl` binary
```

The library is the product and the CLI is its first consumer: no CLI capability is
unreachable from the library.

---

## 2. Kernel data model

### 2.1 Unit types

Fifteen, closed:

`claim`, `evidence`, `definition`, `question`, `hypothesis`, `finding`, `procedure`,
`decision`, `constraint`, `observation`, `data`, `artifact-ref`, `prose`, `contention`,
`packinfo`.

Two are **derived and unauthorable**: `contention` is produced by merge, `packinfo` by
packing. A model asserting either would be fabricating a machine's conclusion, and the ingest
schema excludes both.

### 2.2 Epistemic status

Six levels, ordered weakest to strongest:

| Status | Meaning | Shape requirement |
|---|---|---|
| `unfounded` | retracted | unauthorable — reachable only by retraction |
| `speculative` | a guess | none |
| `inferred` | reasoned, not directly evidenced | `grounds` |
| `derived` | follows from stated evidence | `grounds` |
| `cited` | attributed to a source | `source` |
| `measured` | an instrument recorded it | `source` |

The ordering is the load-bearing part: it is what rules M and T compare against.

### 2.3 Relations

Fourteen kernel kinds — `elaborates`, `contrasts`, `concedes`, `causes`, `enables`,
`exemplifies`, `conditions`, `sequences`, `answers`, `rebuts`, `warrant`, `backs`,
`supersedes`, `retracts` — plus `Extension(String)` of the form `x.<domain>/<kind>`.

### 2.4 Identity

A unit's uid is BLAKE3 over its canonical CBOR encoding. **Identity is content, including
status.** Two consequences run through the whole system:

- Two agents that independently record the same fact produce the same uid and merge into one
  unit, with no registry and no coordinator. This is what makes merge coordination-free.
- **Changing a unit changes its identity.** Anything referring to the old uid now refers to
  something that does not exist. Section 9.3 covers where this bites.

Labels (`c/pool-saturation`) are nicknames for reading and writing. They are **not identity**
— not hashed, and renaming one cannot move a uid — which is precisely why a binding is its
own record (`Record::LabelBinding`, type code 10) rather than a field on the unit: a label
inside hashed content would make renaming it produce a different unit.

Scope is the store the record lives in. Two stores binding the same label to different uids
are a `label-collision` contention on merge, which was machinery the format already had and
could not use between two CBOR stores, because labels arrived out of band and a CBOR store
had none to offer.

### 2.5 Provenance

An attestation records `op` ∈ {`Authored`, `Transformed`, `Imported`, `Attested`}, an agent,
a hybrid logical clock, and a rung. Agent ids are `model:`, `human:` or `tool:` — `agent:` is
not a kind and is refused.

Rungs order how a unit came to exist, and each caps what may be claimed from it:

| Rung | Origin | Ceiling |
|---|---|---|
| `computed` | deterministic tool or parser | `derived` |
| `document` | user-supplied document | `cited` |
| `web` | fetched content | `cited` |
| `model` | the model's own priors | `inferred` |

**No rung reaches `measured`, and that is deliberate.** The op raises the ceiling, not the
rung: `Op::Imported` at the `computed` rung — a deterministic tool transcribing a reading —
is the only route to it. Keying on the op alone would not do, because `ingest` also records
`Imported` (it transcribes rather than authors), and that would let a model assign `measured`
to whatever it read. Ingest runs at `document`, `web` or `model` and stays capped there.

---

## 3. The rules

Twelve letters, each enforced somewhere specific rather than by convention.

| Rule | Statement | Enforced |
|---|---|---|
| **M** | A unit's status may not exceed its weakest ground | `check` pass 6; applied at ingest |
| **T** | A unit may not exceed its rung's ceiling | `check` pass 7; applied at ingest |
| **L** | Closure: what a unit needs travels with it | `check` pass 4; thread repair |
| **R** | A selected claim's rebuttals are selected with it | pack constraint C3 |
| **U** | Merge is a join-semilattice union: commutative, associative, idempotent | `smysl-graph::merge` |
| **I** | Ingest always makes progress | repair loop degrades to opaque `prose` |
| **S** | Model output never enters the store directly | staging to `.smysl/staged.smy` |
| **V1** | A profile renders each status distinctly | `Profile::load` |
| **V2** | Open contentions are surfaced | `ir::build` |
| **X** | Unknown extensions survive verbatim | `extra` maps, closure treats unknown as `elaborates` |
| **D** | Pure operations are bit-reproducible | `cargo xtask determinism` |
| **P** | stdout defaults to CBOR on a non-TTY | CLI `--format` |

**M and T are the pair that matter.** Rule T stops laundering at the door — a model asserting
from its own priors is capped at `inferred` however confidently it phrases the claim. Rule M
stops it inside the graph — confidence can only fall as it flows through grounds, never rise.
Together they are the mechanical form of "a guess cannot quietly become a fact".

---

## 4. Wire format and surface syntax

### 4.1 Deterministic CBOR

Integer keys, shortest-form integers, definite lengths, sorted map keys, no duplicate keys,
NFC text, quantised floats. The conformance tree carries a fixture for each way to violate
this, and each must be *rejected* rather than normalised — a reader that silently repaired a
non-deterministic encoding would break identity.

Unknown map keys are preserved verbatim in an `extra: BTreeMap<u16, Vec<u8>>` and re-emitted
in key order (rule X). An unknown *record type* decodes to `Record::Unknown` with `SMY-W014` —
a warning, because the record survives.

### 4.2 Surface

```
@doc smysl/0.1 { id: v/f1, intent: incident-brief, roots: [f/root-cause] }

@evidence e/pool-wait { status: measured, source: { kind: metric, ref: "pool.wait_ms" } }
~ Pool acquisition wait rose from 2 ms to 310 ms.

Body text follows the gist, after a blank line.
--
Detail follows the body, after a rule.

@rel c/canary-clean --rebuts--> c/pool-saturation { weight: 0.6 }
```

`~` marks the gist. Headers are HJSON; unknown header keys become the unit's `payload`
(rule X). Source kinds are `url`, `file`, `metric`, `tool`, `doc`.

**Comments** are `#` or `//` at column 0, new in 0.2. Both markers, because an HJSON header
inside a record already took both. A comment is a comment wherever it appears — including
inside a body, which costs a body the ability to open a line with either marker. The reverse
choice was tried and was worse: a body runs from the gist to the next record, so a comment
between records fell inside that range and became the previous unit's body, inventing content
out of a note and firing a granularity warning about the invention.

No record carries a comment, so canonical form cannot reproduce one; `fmt` counts them
(`ParseOutcome::comments`) and warns before dropping any. Format sniffing had to change with
it — the "surface text starts with `@`" test is now one function that looks past leading
blanks and comments, because there were two copies of that test and adding comments broke
both, one of them silently.

### 4.3 Level of detail

`L0` gist only, `L1` adds body, `L2` adds detail. Granularity profiles constrain *production*,
not the store — a merged store legitimately holds units written under different profiles:

| Profile | L0 max | L1 band | Admission |
|---|---|---|---|
| `coarse` | 30 | 120–400 | topical |
| `default` | 30 | 40–120 | single-assertion |
| `fine` | 30 | 20–60 | single-assertion |

A unit with no body is `L0` and exempt from the band.

---

## 5. Store, merge and contention

The store is an **append-only log** with a derived index; `reindex` rebuilds the index from
the log alone.

Merge is a set union over records. Because identity is content, the same fact from two agents
collapses to one unit, and merge is commutative, associative and idempotent — so peers
converge without a coordinator and without locking.

Where agents genuinely disagree, merge **detects** three kinds of contention:

- **supersession fork** — two uids supersede the same target, not ordered among themselves
- **live rebuttal** — a `rebuts` edge whose endpoints a thread presents together
- **label collision** — one label bound to different uids across the merged sources

Detections are **reported, not recorded**. A detection written into an append-only log would
be a stale finding the moment a third store supplied the edge that ordered it — and would
break associativity outright. Fixtures F8a/F8b exercise all three.

---

## 6. The check pipeline

Ten passes, in order:

1. **Codec** — envelope and encoding (runs at read time)
2. **Integrity** — reference integrity
3. **Shape** — status/shape agreement
4. **Closure** — rule L
5. **Granularity** — admission and bands
6. **Epistemics** — rule M
7. **Trust** — rule T
8. **Retraction** — retraction integrity
9. **Extension** — extension and conformance
10. **Hashes** — recomputed uids against stored

Diagnostics are a closed registry of **52 codes** in eight groups (parse, identity, LOD,
epistemics, merge, pack/render, extension, provider). Every code carries a severity, and
fixtures assert exact code sets rather than "some error".

---

## 7. Salience, packing, threads, rendering

### 7.0 Salience and episodes

`raw(u) = w_c·centrality + w_r·corroboration + w_t·role + w_recency·recency`

Centrality is personalised PageRank over the support graph; corroboration counts independent
attesting groups; role comes from the active thread.

**Recency is measured in hops, not wall-clock time.** A hop is one handoff of a pipeline,
recorded on every attestation, and `Store::hop_of` / `hops` / `at_hop` are what let a store
answer *what did the last step add* — a question it could not answer before, although the
field had always been written. Decay halves per hop, and the reference hop is supplied by the
caller rather than read from the store, so a replay can ask what salience looked like *at hop
4*. Measuring in hops rather than seconds is also what keeps rule D intact: no clock is read.

**Weighted zero by default.** Salience feeds `pack`, so a non-zero default would silently
change what every existing store carries forward. `SalienceWeights::recent()` turns it on;
`--weights c,r,t[,recency]` takes an optional fourth number, so command lines written before
recency existed still mean what they meant. A unit with no attestation gets no recency rather
than the worst — it is unplaced, not old.

### 7.1 Packing

`pack` fits a graph to a token budget **without calling a model**. Seven constraints:

| | Constraint |
|---|---|
| C1 | a unit at L1+ has its deps |
| C2 | a unit at L1+ has its grounds |
| C3 | **rule R** — a selected unit has its rebuttals |
| C4 | an open contention has its positions |
| C5 | a pinned unit is at L1+ |
| C6 | a unit at L1+ has its warrant |
| C7 | within budget |

C3 is the one that matters. If a claim's rebuttals cannot fit, packing **fails** with the
minimum feasible budget rather than shipping the claim without the objection to it.

Two modes: greedy, and exact by branch and bound behind the `exact-pack` feature. Both report
an optimality gap from a fractional relaxation, so the figure is a ceiling rather than an
estimate.

### 7.2 Threads

Five schemas — `analysis`, `narrative`, `brief`, `qa`, `plan`. Derivation is pure: role
assignment from a rule table, salience-ranked selection within each role's arity, Kahn
ordering over sequencing edges, then **coherence repair** — a step whose deps are missing
pulls them in immediately before itself. The repaired thread satisfies rule L always.

### 7.3 Rendering

Two stages with a Render IR between them: everything needing the graph happens before it,
everything needing a file format after, so no two targets can disagree about what the
document says. Six backends: markdown, html, typst, slides, json, text.

Connectives are template selection keyed by relation kind and seeded by `uid[0]` — never
model inference, so inserting a block does not reword every transition after it.

---

## 8. The provider layer

The only crate that leaves the machine, and most of it is about not doing so.

`--offline` is decided from configuration **before any I/O**: a hosted provider fails with
exit 7 without a socket being opened. `providers --tasks` reports which tasks would egress
under current routing.

The `Provider` trait is **synchronous**. A runtime lives behind it, which is what guarantee A3
promises callers — and is forced anyway, since `async fn` in a trait is not dyn-compatible and
the registry stores `Box<dyn Provider>`.

Five mappers, and how far each is actually proven:

| Mapper | Structured mode | State |
|---|---|---|
| `ollama` | json-schema | verified against a running server |
| `deepseek` | json-mode | verified against the live endpoint |
| `gemini` | json-schema | verified against the live endpoint |
| `anthropic` | tool-force | **implemented, not tested** |
| `openai` | json-schema | **implemented, not tested** |

That distinction is not pedantry. Gemini's mapper was written from the documentation, which
describes its response schema as a subset of JSON Schema draft 2020-12. It is not one — it is
an OpenAPI 3.0 `Schema` proto with no `additionalProperties` field and no `if`/`then` — so
every structured call was refused until a live key proved it, and no recorded fixture could
have caught it. **Appendix C's schema is therefore translated per mapper, not shared**, because
OpenAI strict *requires* the field Gemini does not have. The schema has since grown twice —
`relations` and `quote` — which widens the surface an untested mapper can reject.

Transport rules learned the same way:

- **A status code is data, not an error.** Only transport failures are `Err`; a 404 with an
  explanatory body is the useful case.
- **Backpressure is a class** — 429, 503 (Gemini's overload) and 529 (Anthropic's) — and the
  *status* decides it ahead of the vendor envelope, or a 503 that explains itself loses its
  retry. 500 is excluded: waiting does not fix a bug on the far side.
- **Thinking tokens are output.** They are billed as output but reported apart from it, and
  spent against `maxOutputTokens`, so a budget sized for the answer alone can never finish.

Keys are never stored: only `api_key_env` or `api_key_cmd`, and a config naming a key inline
is refused at load. The usage ledger records counts, models, task and recipe — never prompt or
completion text.

---

## 9. The ingest boundary

Where model output becomes graph. Three rules meet, and they are one idea from three
directions: **a model's output is a proposal, its failures are recoverable, and its confidence
is not evidence.**

### 9.1 Rule T — it cannot overstate its rung

A `model`-rung answer claiming `measured` is downgraded and told. If the shape cannot carry
the ceiling either, `attainable` walks down to the strongest status the unit actually
supports, floored at `speculative`.

The cap **does not spend the repair budget**. It is already the resolution, and a model
confident enough to claim `measured` claims it again — so retrying only exhausts the budget
and degrades a chunk of correctly capped units.

### 9.2 Rule I — it cannot fail silently

A span that survives its repair budget becomes an opaque `prose` unit carrying the raw text
verbatim, and the run exits 10 — or 11, if rule M also lowered a unit. A corpus with some
opaque units is usable; a failed ingest is
not. `prose` is exempt from single-assertion admission, because requiring the opaque-text type
to be one assertion is requiring it not to be prose.

### 9.3 Rule M — it cannot overstate its grounds

Applied at **staging**, not in the repair loop: grounds may reference units the chunk did not
contain, so a claim resting on something ingested an hour ago is the normal case rather than a
violation.

A unit claiming more than its grounds support is **weakened to what they support, and
reported** (`SMY-W036`, and exit `11` since 0.2) — the same treatment rule T gives an
over-claimed rung. Both weakening
and rejection satisfy rule M identically, and only one keeps the content: rejection cascades
through everything grounded on the unit, loses the text, and is irreversible against the
later merge that would have justified the claim.

Because status is hashed into the uid, **weakening moves an identity**. The pass sweeps the
batch topologically, rewriting references as it goes, which moves those units' uids in turn;
labels follow the remap. Since `attainable` floors at `speculative`, which needs no shape, a
walk-down always lands and the pass never has to reject.

### 9.4 It attributes itself, and the attribution is checked

A `source` names a document; it cannot name a passage. Each unit may therefore carry a
`quote` — the span it was drawn from — and **the quote is checked against the text the chunk
came from**. That is what makes it worth more than another assertion by the thing under
review: a quote that does not occur in the source was invented, and the tool says so.

Three outcomes. *Present* once case, whitespace and smart punctuation are normalised.
*Loose* (`SMY-W308`) when every word appears in order but not contiguously — an elision,
which is honest and must not cost a repair turn. *Absent* (`SMY-E307`) when the words are not
there in that order: a fabrication, an error, and worth the repair turn because it is the one
thing a model can fix. Normalisation stops short of stemming and synonyms, which would make a
*reworded* claim look attributed.

The quote rides in the payload under `ingest:quote` (rule X) rather than as a field of
`SourceRef`, where provenance belongs. That is a wire change, deferred until the shape has
been used in anger.

### 9.5 It produces edges, not only units

Ingest emitted no relations at all until SM-P15, which left most of the format inert on any
real input: rule R had no rebuttals to keep with a claim, merge could not detect a live
rebuttal, threads could not fill a `caveat` role, and rendering had no kind to pick a
connective by. All of it worked only on hand-authored fixtures.

The batch schema now carries `relations`, resolved by the same label-or-uid rule `grounds`
uses. `supersedes` and `retracts` are excluded: a model reading a document cannot know a
graph's history, and either would let it delete evidence by mentioning it.

### 9.6 Rule S — it never writes directly

Output lands in `.smysl/staged.smy` as readable surface text, and `merge --staged` is the
confirmation. The thing a human is asked to approve is the thing they can read.

Recipes make a call's *conditions* auditable even though the call is not reproducible;
`recipe_family` drops the provider and model, which is what lets one logical ingest be
compared across vendors.

---

## 10. Command surface

Twenty-one commands. Only two consult a model.

| Purity | Commands |
|---|---|
| Pure | `fmt` `check` `pack` `merge` `diff` `trace` `view` `bundle` `salience` `retract` `render` `providers` `usage` `reindex` `import` `relink` `compact` `ui` |
| Mixed | `thread` (`--derive` pure; `--refine` consults a model) |
| Model | `ingest` `attest` |

Two are worth naming because they close gaps the rest of the design assumed away.

**`import`** reads a delimiter-separated file and transcribes it into `measured` units, one
per row, with an `op: Imported` attestation at the `computed` rung. It is the only producer
of `measured` and the only producer of units that consults no model. Before it, the top of
the status ladder had no writer and every `measured` unit in the corpus was hand-authored.

**`compact`** drops superseded units that nothing needs. The log is grow-only, and that is
what makes merge a join-semilattice — so compaction is *not* an operation inside that
algebra but a lossy projection to a new store. Two consequences are stated rather than left
to be found: it does **not survive a merge** (a peer still holding the dropped records brings
them back, correctly, because union is union), and a **retraction is never dropped**, since
dropping a retracted unit drops the record that it was retracted and the next merge would
resurrect it without one. It refuses to write to stdout or over its own input, being the one
command whose output cannot reconstruct its input.

**`relink`** re-points references onto the units that replaced their targets. Identity is
content, so a corrected unit is a *different* unit and whatever rested on the original still
rests on the original. Within one document this never shows — references are labels, and uids
recompute on parse — but across stores it does. `supersedes` is the only basis used; a fork
is refused with exit 5 rather than adjudicated, and corrections are appended rather than
applied in place.

`ui` is a seven-pane browser — graph, detail, thread, contentions, lineage, pack simulator,
staging. State and the key map are pure and hold no terminal, so every pane is asserted as
text through a test backend. The pack simulator pins a unit with `f` and moves the budget with
`+`/`-`; below the mandatory floor it reports the floor rather than an empty selection.

---

## 11. Guarantees and how they are held

| | Guarantee | Held by |
|---|---|---|
| **A1** | No panics on untrusted input | mutation sweep in CI, plus `cargo fuzz` targets |
| **A2** | No global state, no implicit I/O | clocks and randomness are passed in |
| **A3** | No hidden async — only `smysl-provider` needs a runtime | `cargo xtask check-purity` |
| **A4** | Typed, `#[non_exhaustive]` errors | the type system |
| **A5** | Determinism is part of the API | `cargo xtask determinism` |

A5 is the sharp one: making `pack`, `merge`, `derive_thread`, `salience` or `render`
non-reproducible is a **breaking change regardless of signature**.

Two gates run in CI because both check properties that lapse silently rather than loudly.
`check-purity` asserts six named crates link no runtime, HTTP client or argument parser;
`determinism` asserts the pure operations are bit-reproducible. A third job builds and tests
the pure crates with **networking denied** at the kernel level.

Every crate carries `#![forbid(unsafe_code)]`; the workspace contains no `unsafe` block.

---

## 12. Evaluation

`smysl-eval` runs a five-hop chain in two arms and reports nine metrics, E1–E9.

The **smysl arm** is a chain of packs — a pure function of store and budget, no model
anywhere. The **prose baseline** is a model summarising at every hop, and is never simulated:
a baseline produced by guessing what a model would drop would measure the guess.

Two design points, both of which produced meaningless numbers when got wrong:

- **The budget must be a fraction of the input.** An absolute budget that does not bind
  reports E1 = 1.0 and E2 = 1.0 on every input — the output of a harness measuring nothing,
  indistinguishable from a triumph.
- **The judge must be controlled.** The same judge reads the *unsummarised* prose first. If it
  already reports certainties there, the post-chain figure is instrument bias, not a finding.

Measured over F1, five hops, four runs — a data point, not a benchmark:

| | Tokens | Claims kept | Hedges lost |
|---|---|---|---|
| control (unsummarised) | 1.00 | 8 / 8 | 0 of 8 |
| prose baseline | 0.46–0.50 | 7–8 / 8 | **3–5 of 8** |
| smysl | 0.56 | 8 / 8 | 0 of 8 |

Compression is a wash. The difference is epistemic: claims the original called `inferred`,
`cited` or `derived` came out of the prose chain reading as measurements.

E8 and E9 report *not run* with a reason rather than defaulting to zero — a metric that
quietly vanishes from a report is the failure this crate exists to detect elsewhere.

---

## 13. Conformance corpus

Eight fixtures, each with an expected diagnostic set that is asserted exactly.

| | Fixture | Exercises |
|---|---|---|
| F1 | incident report, `default` | the baseline path; rules M, R, V1 |
| F2 | research trace, `fine` | deep grounds chains, cascades, corroboration |
| F3 | narrative, `coarse` | the design's most likely falsifier |
| F4 | Q&A session | `answers` edges and the `qa` schema |
| F5 | dataset analysis | `data`, `artifact-ref`, extension payloads |
| F6 | adversarial store | laundering attempts — the only fixture that must *fail* |
| F7 | mixed-granularity merge | D-5: mixed granularity is legal |
| F8 | multi-agent contention | two files; all three detection kinds on merge |

F6 expects `SMY-E030`, and a run that reports nothing means rule M has stopped binding.

---

## 14. Known gaps

- **Anthropic and OpenAI mappers are untested.** OpenAI has a concrete suspect: strict mode
  requires every `properties` key in `required`, and the shared schema lists a fraction of
  them. The risk has grown, not shrunk — Appendix C gained `relations` and `quote` since,
  and the mapper passes it through unchanged. Blocked on a key.
- **`pack` was quadratic in store size**, and is not any more — see "Closed in 0.6" below
  for the fix; the history is kept because what a gap cost is worth keeping. Measured, which
  corrected two guesses this note
  previously made: `salience` is *linear* and fine, and PageRank over the full adjacency is
  not the problem. Reproduce with `python3 scripts/bench-scaling.py`, which prints the ratio
  between successive sizes — 2.0 is linear, 4.0 is quadratic:

  | units | check | salience | merge | pack |
  |---:|---:|---:|---:|---:|
  | 1000 | 8 (1.6x) | 8 (1.5x) | 10 (1.7x) | 161 (3.9x) |
  | 2000 | 13 (1.7x) | 12 (1.6x) | 19 (1.8x) | 658 (4.1x) |
  | 4000 | 25 (2.0x) | 23 (1.9x) | 37 (2.0x) | 2818 (4.3x) |

  Two further facts, both measured, both narrowing where to look:

  - **It is worst when packing is easiest.** With a budget that admits everything, `pack` is
    quadratic (3.8x, 4.3x per doubling); with a budget that binds, it is linear (1.9x, 2.0x).
  - **It is not `improve()`.** Setting `IMPROVEMENT_PASSES` to 0 changed the time by under
    1% at 4000 units, so the local-improvement pass — the obvious nested loop, and the first
    thing reading the code suggests — is not the cost.

  Counting the calls settled it. `closure::delta` ran 7 487 469 times for 4 001 units and
  scaled exactly 4.0x per doubling, with roughly one graph visit per call — so the cost was
  call *volume*, and per-call optimisation would have been wasted effort.

  The volume is structural. The greedy runs one round per unit admitted and re-evaluates every
  remaining candidate each round to choose a global best: O(n²) by construction, not by
  accident. Worth paying when the budget binds; worth nothing when it does not.

  **Fixed for the ample-budget case** by skipping the greedy rather than changing it: if the
  whole scope fits at its top level, take it. Value is monotonic in level and a selection that
  omits nothing satisfies every closure constraint, so if it fits there is nothing to trade.
  4 000 units went from 2 818ms to 26ms, verified byte-identical against the old
  implementation across every fixture at seven budgets. `--explain` now reads `earned on
  density` throughout on that path, since nothing was dragged in under pressure.

  **Closed in 0.6.** The scan is an ordered set keyed on the choice, so a round is a pop
  rather than a walk: 2.07, 2.20, 1.94, 2.23, 2.11x per doubling out to 8 000 units, and
  2.05 ms at 2 000 units against 18.54 ms. Verified byte-identical against a recorded
  baseline of what `pack` selects across nine fixtures at six budgets.

  Two things made it sound where the textbook lazy greedy is not. The tie-break became one
  named type used by the pop and nothing else — the risk was never the algorithm but a heap
  reproducing three of four terms, and no corpus fixture ties on density without also tying
  on salience, so the suite could not have caught it. And an unaffordable candidate is
  *parked* rather than dropped: `used` only grows, so it can never become affordable unless
  its marginal cost falls, which happens exactly when something in its obligation is
  selected and therefore already paid for. That non-monotonicity is what made the 0.3.0
  attempt unsound.

### Closed in 0.10

**§2.3 is verified by a second implementation.** The largest thing 0.9 left open. C-Read does
not reach uid derivation, so three independent readers agreed on every byte of every fixture
without ever computing a uid — and *status is part of identity*, the claim the format rests on,
stayed verified by this implementation alone for nine releases.

`python/` reaches C-Produce as of 0.10: canonical unit-core layout, and BLAKE3 hand-rolled so
that the evidence is two implementations rather than two callers of one C library. It
reproduces all sixteen uids in `fixtures/wire/uid/cases.json`, with the canonical bytes checked
separately from the hash so a disagreement localises itself.

`nodejs/` and `go/` stay at C-Read. That is a scope decision, not a gap, and both say so where
they list what they do not reach — the wording there used to imply the claim went unchecked
anywhere, which is no longer true.

**`topo` was quadratic, and `check` with it.** Kahn's algorithm here sorted its ready set on
every iteration and then popped with `remove(0)` — two quadratic factors in three lines, in the
traversal that thread derivation and rule M's single ordered pass both run over. A min-heap pops
the smallest dense id in log time, which is the same order, so nothing downstream moved: the
determinism gate reports `merge`, `derive_thread` and `render` identical across sixteen runs.
`check` at 16 000 units went from 40.2 ms to 6.6 ms.

Worth recording as an architecture note rather than a bug fix, because of how it survived. The
order was correct throughout, and the cost was invisible to every test — the suite asks whether
the answer is right, and nothing asked what it cost. Three of the four operations in the pure
set were assumed linear; two were not.

**A format-versioning policy**, §8 of the specification. The clause that recurs is §8.3:
tightening an implementation that was *more permissive than the document* is not a format break,
because the documents it stops accepting were never conformant. Writing it uncovered that the
wire carries no version at all — it lives only in surface syntax, the parser validates and
discards it, and a writer reconstructs the header from its own supported list. Correct by
coincidence at one entry, and a document that misdescribes itself at two.

### An error the design makes unconstructible

Found in 0.11 by mutation testing, and worth stating as a property rather than a test note.

`SMY-E061` is a cycle in the support graph. The pass detecting it can be deleted outright with
no test failing — not because the pass is untested, but because **the condition cannot occur**.
Support edges are `deps` and `grounds`, both read from a `UnitCore`'s own fields; a `Unit` does
not store a uid, it derives one from that core. Two units naming each other therefore requires
solving a hash fixpoint.

This is content addressing paying out somewhere the design did not set out to spend it. §2.1
exists so that a uid names exactly one thing; a side effect is that a whole class of structural
corruption becomes unrepresentable rather than merely detectable, and the detector for it is
dead code kept as a backstop.

It stops being dead the moment a relation kind joins `EdgeSet::support()`, because relation
endpoints are arbitrary and cycle freely. A test now fails at that moment rather than after it.

### Closed in 0.9

- **Three independent implementations of the wire format**, in `python/`, `nodejs/` and `go/`,
  each written from `SMYSL_FORMAT_SPEC.md` and each targeting C-Read: decode, re-encode
  byte-identically, preserve what is not understood. All three run in CI against the same
  fixtures in `fixtures/wire/`, produced by this implementation.

  This is the gate `READINESS.md` called the largest, and the reason it mattered is worth
  restating: every other check in this repository would pass just as happily if the
  specification were blank. They test whether the Rust is self-consistent. Only these test
  whether the *document* is sufficient.

  More than one on purpose. Implementations that agree could have made the same guess where
  the document is silent, so agreement is evidence only when the readings are independent. The
  JavaScript was written without consulting the Python, and both arrived at the same two
  ambiguities.

- **Three clarifications to §3**, which is what the exercise produced. Constraint 1 said what
  an encoder may do without saying what a decoder must do; constraint 2 said "no value encoded
  in more bytes than it needs" without scoping that to integers and lengths, so applied
  literally to a float head it rejects 1.0; and tags were not mentioned at all. All three are
  now stated, and the section records that they read as they do because two independent readers
  both had to invent the same answer — their guesses agreeing is fortunate rather than
  reassuring.

- **The Go implementation is the first written against the revised document**, which makes it
  a test of the revision rather than only of the format. It needed no guesses where the earlier
  two did.

- What remains untested by a second implementation is recorded rather than left silent: C-Read
  does not reach uid derivation, so §2.3 — *status is part of identity*, the paragraph the
  whole format rests on — is still verified by this implementation alone. All three suites
  carry a test that fails if that list is ever quietly emptied. Closing it means C-Produce and
  BLAKE3.

### Closed in 0.8

- **The local-improvement pass is gone.** Measured across 28 000 generated packs, it changed
  26 and made 22 of those *worse* by the value function it existed to maximise. It fired on
  0.09% of packs, which is why two earlier measurements read it as harmless: 0.3 found that
  disabling it changed runtime by under 1%, and mutation testing found `improve -> false`
  survives every test. One was measuring time, the other whether anything noticed — neither
  asked whether the packs improved without it. Removing it left the golden file
  byte-identical: nine fixtures, six budgets, not one selection moved.

- **Two oracles were never audited.** `verify` could be replaced with `vec![]` and every test
  passed, including the C1–C7 property test and the `pack_constraints` fuzz target — four
  assertions read `verify(...).is_empty()`, all satisfied by an oracle that never speaks.
  `satisfies_rule_l` had the same defect in the sibling position. Neither is the thing under
  test; both are what other tests *trust*, which makes everything downstream unfalsifiable.

  Found two ways. Mutation testing over `solve.rs` reported 49% of viable mutants surviving on
  the best-tested file in the project, and `verify -> vec![]` was among them. The second came
  from asking directly of each trusted function whether any test makes it say *no* — four
  candidates, two gaps, twenty minutes. For finding oracles the question beats the sweep;
  `conformance` and `Query::admits` came back clean.

- **`Documentation/READINESS.md`** — seven gates on publishing, each done or with a next
  action. Written because "not production ready" had been the answer twice while saying
  nothing about what would change it. The largest gate is that nobody has implemented the
  format from the spec alone.

- **The semantic path runs in CI**, against a 4 KB generated model in `fixtures/embed-tiny`
  rather than a 30 MB download. Before it, that path was exercised only by hand — the shape
  of defect that let two stack overflows survive to 0.3.

### Closed in 0.7

- **Semantic retrieval**, as `smysl-embed`, behind the `Retriever` seam 0.5.0 built and off by
  default under `--features semantic`. Model2Vec static embeddings: a token maps to a vector
  and a sentence is a pooled lookup, so there is no ONNX Runtime, no downloaded binary and no
  `ort` release-candidate pin. `hf-hub` is compiled out, so nothing in it can reach the
  network — a model is three files the operator already has.

  Measured against the same twenty queries as the lexical evaluation, from one shared file so
  the tables can be read side by side:

  | engine | recall@5 | MRR | P@1 |
  |---|---:|---:|---:|
  | lexical | 0.90 | 0.74 | 0.60 |
  | semantic | 0.95 | 0.84 | 0.75 |
  | **hybrid** | 0.95 | **0.87** | **0.80** |

  Precision-at-one on paraphrase went from 0.12 to 0.50, which was the number that justified
  building it.

  **The first hybrid was worse than semantic alone** — 0.78 MRR against 0.84 — and passed its
  test, because the test only asked it to beat lexical. It routed by kernel type when a query
  carried a `kinds` filter and merged both engines when it did not; no real query carries one,
  so the dispatch was never exercised and the merge averaged good ranks with bad. Rewritten to
  route on the *query*: identifier-shaped goes to lexical, everything else to the embedder.
  The assertion is now the property that failed — routing must never lose to either engine it
  routes between.

- **The OpenAI strict-schema defect**, which never needed the API key it was filed behind.
  Strict structured outputs require every key in `properties` to appear in `required`;
  Appendix C declares eleven and requires three, so a strict request was rejected outright
  rather than degrading. `openai_compat::strict_schema` translates at the boundary — every
  property required, optionality expressed as a nullable type, `additionalProperties: false`
  stated, unsupported constructs dropped — because the shared schema is what Gemini and
  DeepSeek receive and both work with it. Verified live afterwards.

- **The hosted gate ran**, and the structured-output modes are visibly different rather than
  nominally so: Gemini's `json-schema` returns four conformant units in one call; DeepSeek's
  `json-mode` guarantees valid JSON but not valid-against-this-schema, so it retried three
  times, produced one unit, degraded it under rule I, and spent two and a half times the
  tokens.

### Closed in 0.6

- **`pack` is linear when the budget binds** — see the performance note above, which is where
  the numbers and the soundness argument live.

- **NFC is enforced by the encoder rather than asserted in debug.** Constructors establish
  the invariant for a unit's gist, body and detail and for nothing else: a thread's gist, a
  step's note, a view's intent, a granularity profile, a source reference and a pack
  estimator all reached the encoder unchecked. Two of those had been found by fuzzing in two
  separate releases and each fixed by normalising in one more constructor — a class treated
  as a list. Enforcing costs a quick-check on text about to be BLAKE3'd anyway, and makes the
  implementation match what the format spec already promised. A debug-only assertion never
  could, because the builds that matter compile it out.

- **`pack --query`**, the composition retrieval was built for. Retrieval names which units
  are relevant; packing pulls in their grounds, deps and live rebuttals, so the result is an
  argument rather than excerpts that scored well. Neither half achieves that alone.

- **`\#`, `\//` and `\\` escape a body or detail line**, which 0.2 documented as a
  limitation and 0.4 half-fixed. A Markdown heading and a line of C++ both open a line the
  way a comment does, and were dropped in silence.

- **A thread's gist kept its leading whitespace** — the 0.4 unit-gist fix in a sibling path
  it never reached. Found within a minute of seeding the fuzzer.

- **The fuzz gate is seeded** from the corpus fixtures and every input that has ever broken
  something; the long run stays cold on purpose, because 0.4 and 0.5 both had findings that
  came from a cold run landing where a warm corpus does not go.

- **`SMY-W305` is emitted and `SMY-W306` deleted**, after two releases of "documented as
  unreachable" — which is a holding pattern rather than a decision.

- **`tui` left the default feature set**, and the CI matrix gained the two combinations
  nothing was building: `cli` alone and `tui` alone. Neither is reachable from any other row,
  and that gap is exactly how a dead-code error under `-D warnings` made the determinism job
  report "rule D failed" for three releases.

- **`ui` was documented as a stub and is not one.** A working TUI, described as unwired
  because a single sentence in Appendix A said so and nobody checked — including me, twice,
  while planning work around removing it.

### Closed in 0.5

- **Retrieval exists**, as `smysl-retrieve` and `smysl find`. It is a *seam* first and an
  engine second: `Retriever` is a trait, and the shipped implementation is BM25 over gists,
  bodies and details. The crate is **pure** and under the purity gate, which is unusual for
  search and follows from taking `bm25` with `default-features = false` — its default
  tokeniser stems and strips stop words, destroying identifiers; its language detection
  would make tokenisation depend on the corpus; its `parallelism` feature would put a rayon
  reduction inside a result that must not vary.

  It indexes the **gist** principally. That is the design's load-bearing idea: a payload may
  be a stack trace, a metric series or a diff, but every unit carries a gist by construction,
  and the gist is a sentence about whatever the payload is. Payload heterogeneity never
  reaches the index.

  Measured rather than asserted — 20 queries over the corpus, in three classes:

  | class | recall@5 | MRR | P@1 |
  |---|---:|---:|---:|
  | shared vocabulary | 1.00 | 0.94 | 0.88 |
  | paraphrase | 0.75 | 0.41 | 0.12 |
  | identifier | 1.00 | 1.00 | 1.00 |

  By kernel type, `evidence` and `data` score 1.00 and `claim` scores 0.67. Concrete things
  are findable by name; an interpretation is phrased in whatever words its author reached
  for. So a semantic backend would pay for itself on `claim`, `finding` and `hypothesis` and
  add nothing on the rest — a narrower case than "add embeddings", and the reason the trait
  exists rather than a second engine.

- **The fuzzers reach the algebra**, not just the parsers. Rule U's join-semilattice laws,
  pack's C1–C7, rule L with guarantee A1 across the pipeline, and exact packing against
  brute-force enumeration. The properties are the ones the seeded tests already assert; what
  changed is that coverage feedback picks the graphs instead of a fixed seed and 200 blind
  rounds.

- **Four vacuity defects**, all in test infrastructure rather than in the product, and all
  found by asserting the shape of what a test is handed rather than trusting a clean run.
  The fuzz generator produced no relations, then no unit above L0. `exact.rs` never generated
  a `detail`, so L2 was never in the search space where branch-and-bound is checked against
  brute force. And the sweep written to generalise 0.4.0's decoder fix never entered
  `dec_schema_decl`, because none of its probe values parsed as a `SchemaId` — so a third
  decoder defaulted a field the encoder always writes, and shipped.

- **A duplicate known-field key leaked into the unknown-key payload.** `HObject::take`
  removed only the first entry, and what a caller leaves behind becomes the payload under
  rule X. Two `deps` parsed as one real field plus one extension, which the writer emitted as
  a plain `deps:` line and the next parse consumed as the field. First wins now, everywhere.

- **CI stopped lying.** It runs on `dev/**` rather than first seeing a cycle at the release
  commit; `make test-matrix` sets `-D warnings` and covers `--features cli`; and failing jobs
  report *why* in an annotation, with fuzz crashes carried as base64. That last one matters
  because job logs and artifacts both need admin rights on this repository — a determinism
  job reported "rule D failed" for three releases when the truth was that a dead-code error
  under `-D warnings` stopped the binary compiling.

### Closed in 0.4

One theme, approached from eight sides: **a uid must name exactly one byte string, and
`parse -> write -> parse` must be a fixed point.** Every item below was found by fuzzing,
none was a regression, and none would have been found by reading the code — several had been
in the tree since before 0.2. Each is pinned by a regression test.

- **`quantise` returned infinity** for a large payload float, which the CBOR writer asserts
  against. Debug builds panicked; release builds wrote the infinity to the store — a value
  the codec's own contract forbids, emitted in silence. Now total, saturating at the largest
  magnitude constraint 4 can express.
- **Two labels can name one unit**, because identity is content: two declarations with the
  same gist, status and grounds *are* one unit, and the surface has room for one name on the
  declaration. The parser kept the first in document order and the writer the first in
  canonical order, so the surviving name changed on every pass. Both now keep the canonically
  first, and the loss is reported as `SMY-W054` rather than happening silently. Invisible
  before `Record::LabelBinding` existed, because nothing carried a label through the wire to
  notice it going missing.
- **A trailing carriage return eroded one per round trip.** The lexer stripped exactly one
  `\r` before a `\n`. All of them now go: line endings are not content, or the same document
  would hash differently under CRLF and LF and identity would depend on a git config.
- **Unknown header keys were written unquoted** while values were quoted, so a key holding a
  `:`, a `}` or a newline tore the header apart and the whole unit vanished on re-parse.
- **A header value starting with `#` or `//` was written unquoted**, so the comment syntax
  added in 0.2 ate the rest of the line and the closing brace with it. Only a *leading*
  marker is a hazard — a quoteless value runs to `,`, `}`, `]` or end of line without
  stopping at either — so `grafana://board/12` still needs no quotes and still gets none.
- **Unknown header text skipped NFC normalisation**, the one text path that did. Debug builds
  tripped the encoder's assertion; release builds encoded the non-NFC text, so two peers
  writing the same content in different Unicode forms produced different uids. Rule D failing
  silently, in the build people ship.
- **A gist assembled from continuation lines kept a leading space** that the reader ate on the
  way back, moving the uid with it.
- **`PackInfo` and `View` decoded with defaults for mandatory fields** the encoder always
  writes, so `[7, {0: 0}]` was accepted and re-encoded as a four-key map — two byte strings
  mapping to one record. Both now reject what they cannot re-emit, and a sweep over every
  record kind and low key guards the invariant generally.

The last one is worth a note on method. The first version of that sweep probed with integer
values alone, so it never entered `dec_view` — whose key 0 is a text id — and reported the
class clean while a second instance of it sat in the tree. The fuzzer found it the next run.
A test that cannot reach the code it claims to cover is worse than no test, because it also
supplies confidence.

### Closed in 0.3

- **Twelve global flags were advertised by every subcommand and implemented by a handful.**
  Measured before the work: `--output` honoured by 3 of 9 commands, `--json` by 1 of 6,
  `--strict` by 1 of 8, `--quiet` by none. A caller who read `--json` in `smysl trace --help`,
  passed it, and got prose had no way to learn the flag was never wired. All four are now
  either honoured or refused out loud, and `tests/global_flags.rs` asserts the matrix so the
  next flag added cannot arrive unwired in nineteen places.
- **`check --json` emitted invalid JSON.** It used Rust's `{:?}`, which renders a control
  character as `\u{1}`; no parser accepts that, and a diagnostic message quotes document
  content. `json_escape` existed for exactly this and the one command emitting JSON did not
  use it.
- **Two stack overflows.** The surface parser aborted at ~5 000 nesting levels and the CBOR
  reader at ~20 000, the latter reached through an unknown key — rule X, the
  forward-compatibility mechanism, was the route to the crash. Both bounded at 128. An abort
  is worse than a panic: it cannot be caught, so an embedder cannot contain it.
- **`--budget Nk` overflowed**, panicking in debug and *wrapping* in release, so a huge budget
  became 384 tokens and was reported as the budget.
- **`pack` was quadratic.** 2 818ms to 26ms at 4 000 units when the budget admits everything,
  and 6-7x faster when it binds. Both fixes are exact and verified byte-identical.
- **`bundle` and `pack` dropped label bindings**, so the two artifacts most likely to be handed
  to someone else arrived spelled in bare uids.
- **Both fuzz targets now run in CI.** They existed from the first commit and nothing ran them,
  which is how the two stack overflows survived to 0.3. Reporting-only at first, because the
  first run found a backlog; blocking as of 0.4, when that backlog was cleared.

### Closed in 0.2

Recorded rather than deleted, because what a gap turned out to cost is worth keeping.

- **Labels had no wire record**, so they survived a parse and not a store round trip. A
  document that had been through `merge` came back with every reference spelled as a bare
  uid: valid, re-checking clean, and unreadable — which broke the format's central claim for
  exactly the multi-agent case it exists to serve. Now `Record::LabelBinding`, type code 10.
- **`SchemaId` decoding was strict**, so a kernel type added in a later 0.x failed the whole
  record with `SMY-E004` while an unknown *record* type and an unknown *extension* type both
  degraded. Now `SchemaId::UnknownKernel`, reported as `SMY-W010`, re-encoding byte for byte.
  Decoding and surface parsing needed opposite behaviour — a typo must stay a typo — so
  `parse_forward` is a second entry point rather than a loosening of `parse`.
- **`SMY-W014` was declared and never emitted.** An unknown record type was preserved in
  silence. `check` now reports it, as it now reports `SMY-W010` without waiting for `--as`.
- **`merge` ignored `--format surface`** and **`fmt` could not read a CBOR store**, so a
  merged store was the one artifact nobody could read back. Both fixed. Fixing the first
  exposed that `write_surface` emitted thread steps naming canonical uids that its own parser
  rejected.
- **`ingest` exit codes did not distinguish** a weakened batch. Now exit `11`; under `--yes`
  it returned plain `0`, so the outcome most worth knowing about was the one that looked like
  nothing having happened.
- **There was no comment syntax**, while HJSON headers accepted `#` and `//` inside a record.
