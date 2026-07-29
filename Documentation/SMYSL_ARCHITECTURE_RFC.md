# smysl — implemented architecture

**Status:** descriptive, not normative.
**Describes:** the code at `main`, format `smysl/0.1`, kernel `smysl.kernel/0.1`.
**Compiled:** 2026-07-29, from SM-P0 through SM-P15 and the operational-merit work after it.

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

Labels (`c/pool-saturation`) are document-local nicknames for reading and writing. They are
not identity and, at present, have no wire record — see `RFC_PROPOSAL.md` item 16.

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

**There is no comment syntax.** A `#` line between records lexes as body text and is absorbed
into the preceding unit.

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
verbatim, and the run exits 10. A corpus with some opaque units is usable; a failed ingest is
not. `prose` is exempt from single-assertion admission, because requiring the opaque-text type
to be one assertion is requiring it not to be prose.

### 9.3 Rule M — it cannot overstate its grounds

Applied at **staging**, not in the repair loop: grounds may reference units the chunk did not
contain, so a claim resting on something ingested an hour ago is the normal case rather than a
violation.

A unit claiming more than its grounds support is **weakened to what they support, and
reported** (`SMY-W036`) — the same treatment rule T gives an over-claimed rung. Both weakening
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
- **`pack` and `salience` recompute over the whole store on every call.** `compact` bounds
  how large that store gets, but nothing bounds the per-call cost, and PageRank runs over the
  full adjacency each time. No evidence yet that it bites; no measurement either.
- **Labels have no wire record**, so they survive a parse and not a store round trip.
- **`ingest` exit codes do not distinguish** a batch where rule M weakened something from one
  where nothing happened. The CLI prints it; the code does not encode it.
- **`merge` ignores `--format surface`** while `pack` and `render` honour it, and `fmt`
  cannot read a CBOR store although `check` can.
- **Quoting appears to coarsen units.** The same fixture that gave five or six units gives
  three once each must carry a quotable span. Observed, not diagnosed — it may be the prompt
  or it may be inherent to anchoring a unit to text it can quote.
- **`SchemaId` decoding is strict**, so a kernel type added in a later 0.x minor fails to
  decode rather than degrading. Must be revisited before any store exists if kernel types may
  be added within 0.x.
- **~67 divergences from the RFC** remain unreconciled — see [`RFC_PROPOSAL.md`](RFC_PROPOSAL.md).
