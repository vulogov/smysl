# smysl

An AI↔AI↔Human data interchange format, library, and CLI.

Prose is a lossy serialisation of a structure the producing model already had. Every
summarisation hop re-derives that structure from a progressively degraded signal, and
along the way hedges disappear, provenance evaporates, and disagreement gets silently
resolved by whoever summarised last. `smysl` transports the structure instead.

Three properties follow, and they are what distinguish this from a document-passing
pipeline:

1. **Summarisation is precomputed.** Fitting content to a token budget is a pure function
   over the graph, requiring zero model calls.
2. **Epistemic degradation is structurally impossible.** A trust ceiling at ingestion plus
   a monotonicity rule inside the graph mean a speculation cannot become a finding across
   hops.
3. **Disagreement converges without being resolved.** Merge is a coordination-free
   join-semilattice; semantic conflict is materialised, not adjudicated.

Implements **RFC SMYSL-1 (Combined)** — format `smysl/0.1`, kernel `smysl.kernel/0.1`.

## Status

**SM-P9 — packing.** `smysl pack` is what a consuming agent calls instead of asking a
model to summarise: same graph, same budget, same thread yields identical bytes, and no
inference happens anywhere. Seven constraints hold on every pack. The one that matters is
C3 — a selected claim's rebuttals come with it, always — and when the budget cannot hold
both, packing **fails** with the minimum feasible budget rather than emitting the claim
alone.

```bash
$ smysl --format surface pack --budget 8k --focus c/pool-saturation incident.smy
$ smysl pack --budget 5 --focus c/pool-saturation incident.smy
smysl pack: SMY-E200: budget 5 but the mandatory floor needs 46     # exit 4
```

**SM-P10 — exact packing.** `--mode exact`, behind the `exact-pack` feature, replaces
greedy's selection with a provably optimal one by branch and bound. Both modes now report
an optimality gap derived from a fractional relaxation, so the figure is a ceiling that can
be relied on rather than an estimate.

```bash
$ smysl pack --budget 90 --explain incident.smy
… 5 of 8 unit(s), 80 of 90 tokens, greedy mode, gap 0.059
$ smysl pack --budget 90 --mode exact --explain incident.smy
… 5 of 8 unit(s), 90 of 90 tokens, exact mode, gap 0.000 (proven optimal)
```

**SM-P11 — threads.** `smysl thread --derive` turns a graph into an ordered reading of it
under one of five schemas: role assignment from a rule table, salience-ranked selection
inside each role's arity, Kahn ordering over the sequencing edges, then **coherence
repair** — a step whose deps are missing pulls them in, immediately before itself. That
last stage is the phase gate, and it is asserted as a property over generated graphs
rather than by example: the repaired thread satisfies rule L always, or repair is not a
repair.

```bash
$ smysl thread --derive narrative story.smy
@thread t/narrative { schema: narrative, owner: tool:smysl, ts: [0, 0] }
~ The pool wait metric had been visible the whole time, on a dashboard nobody opened.; …
  setup → p/setup
  complication → p/complication
  turn → p/turn
  resolution → p/resolution
  coda → p/coda
$ smysl thread --derive qa --explain incident.smy
incident.smy: question is required by qa and nothing could fill it
```

Derivation is pure — no model is consulted — so `derive_thread` joins `pack`, `salience`
and `merge` as a rule D operation in the determinism matrix. `--refine`, which does consult
a model, arrives with the provider layer.

**SM-P12 — rendering.** `smysl render` turns a thread plus a profile into an artifact.
Two stages with the Render IR between them: everything needing the graph happens before it,
everything needing a file format after it, so no two targets can disagree about what the
document says.

Rule V1 is enforced when the **profile loads**, not when it emits. A profile that renders
`speculative` the way it renders `measured` never becomes a `Profile` value, so there is no
path from a flattening profile to an artifact:

```bash
$ smysl render --profile flat.hjson --thread t/brief incident.smy
smysl render: flat.hjson: SMY-E210: profile flat has no distinct rendering for unfounded   # exit 3
```

Rule V2 is enforced when the IR is built, so a suppressed contention is recorded in every
target — and because merge *reports* detections rather than recording them (§5.4), the
renderer detects live contentions rather than only surfacing written-down ones. Otherwise
rule V2 would be vacuous in exactly the case it exists for.

```bash
$ smysl thread --derive --schema brief incident.smy \
    | smysl render --thread t/derived-brief --profile exec --target markdown -
$ smysl --strict render --profile plain --contentions suppress incident.smy
smysl render: SMY-W211: 1 open contention(s) suppressed by profile plain                   # exit 3
```

Connectives are template selection keyed by relation kind and seeded by `uid[0]`, never
model inference — so inserting a block earlier does not reword every transition after it.
`render` is the fifth and last rule D operation in the determinism matrix.

**SM-P13 — the provider layer.** The first phase that leaves the machine, and most of it
is about not doing so. `--offline` is decided from configuration before any I/O: a hosted
provider fails with exit 7 without a socket being opened, and `providers --tasks` says
exactly which tasks would egress under current routing.

```bash
$ smysl providers --probe
ollama         up    ctx 131072   out 2048   json-schema  local   4 model(s); llama3.2 installed
$ smysl --offline providers --tasks
task                 provider       egress     command
content-ingest       ollama         local      ingest
attest               hosted         LEAVES     attest
--offline: any task marked LEAVES will exit 7 rather than run
```

Fallback fires on `Unreachable` and on nothing else — falling back on `Unauthorized` or
`ContextExceeded` would hide a configuration error behind a different model, and the caller
would get an answer from somewhere they did not choose. Ollama is the conformance reference
because it is the only provider exercisable without keys, cost, or egress; its mapper is
asserted against a running server in CI rather than against a remembered API.

Keys are never stored: only `api_key_env` or `api_key_cmd`, so `.smysl/config.hjson` is
safe to commit. `ProviderConfig` has no field a key could be written into, and a config
naming one is refused at load. The usage ledger records counts, models, task, and recipe —
never prompt or completion text.

Long-running commands report progress on **stderr**, never stdout, and only when stderr is
a terminal: `--noprogress`, `--quiet` and `--json` each turn it off, and a pipeline never
finds a spinner in the middle of a CBOR sequence.

**SM-P14 — hosted providers and ingest.** Four more mappers and the boundary a model's
output has to cross. Three rules meet there, and they are one idea from three directions: a
model's output is a proposal, its failures are recoverable, and its confidence is not
evidence.

**Rule T** caps what a model may claim. A `model`-rung answer claiming `measured` is
downgraded and told so — and if the shape cannot carry the ceiling either (`inferred` needs
grounds), it walks down to what the unit can actually support.

**Rule I** guarantees progress. A span that survives its repair budget becomes an opaque
`prose` unit carrying the raw text verbatim, and the run exits 10 rather than failing: a
corpus with some opaque units is usable, a failed ingest is not.

**Rule S** stages. Model output never enters the store; it lands in `.smysl/staged.smy` as
readable surface text, and `merge --staged` is the confirmation.

```bash
$ smysl ingest --dry-run report.md
provider     ollama
egress       no - local
path         json-ast (default for small enforced ingest)
rung         document (ceiling cited)
$ smysl ingest report.md
smysl ingest: warning: SMY-W304: span degraded to opaque prose after 3 attempt(s)
7 unit(s) staged in ./.smysl/staged.smy; review, then `smysl merge --staged`   # exit 10
```

Recipes (D-8) make a model call's *conditions* auditable even though the call itself is
not reproducible — and `recipe_family`, which drops the provider and the model, is what lets
E9 compare the same logical ingest across vendors.

| Phase | Delivers | State |
|---|---|---|
| SM-P0 | scaffold, diagnostics, gates | **done** |
| SM-P1 | deterministic CBOR codec, kernel types, identity | **done** |
| SM-P2 | surface syntax, `fmt` | **done** |
| SM-P3 | store, index, adjacency, `reindex` | **done** |
| SM-P4 | structural check passes, `check` | **done** |
| SM-P5 | rules M and T, conformance classes, `--as` | **done** |
| SM-P6 | merge, contentions, retraction, `merge` / `retract` | **done** |
| SM-P7 | lineage: `diff`, `trace`, `view`, `bundle` | **done** |
| SM-P8 | salience, `salience --explain` | **done** |
| SM-P9 | packing, `pack --explain` | **done** |
| SM-P10 | exact packing, provable optimality gap | **done** |
| SM-P11 | thread schemas, derivation, `thread --derive` | **done** |
| SM-P12 | Render IR, profiles, six backends, `render` | **done** |
| SM-P13 | provider layer, Ollama, registry, ledger, `providers` / `usage` | **done** |
| SM-P14 | hosted providers, chunking, repair loop, `ingest` / `attest` | **done** |
| SM-P15 | TUI, evaluation | |

## Layout

```
src/                   facade crate: [lib] + [[bin]] smysl
crates/smysl-core      kernel types, deterministic codec, surface syntax, diagnostics
crates/smysl-graph     append-only store, index, adjacency, merge, salience
crates/smysl-check     the ten check passes
crates/smysl-pack      budget-bounded, closure-complete selection
crates/smysl-thread    thread schemas and deterministic derivation
crates/smysl-render    render IR, profiles, backends
crates/smysl-provider  the model boundary - the only crate linking a runtime
crates/smysl-ingest    staging, repair, trust ceiling, recipes
crates/smysl-tui       seven-pane terminal UI
crates/smysl-eval      evaluation harness (not published)
fixtures/              corpus, conformance suite, golden artifacts
xtask/                 purity and determinism gates
```

## Build

```bash
cargo build                       # cli + tui + local (Ollama) + Typst rendering
cargo build --no-default-features # pure library: no runtime, no HTTP, no arg parser
cargo test --workspace --all-features
```

Two gates run in CI and are worth running locally, because both check properties that
lapse silently rather than loudly:

```bash
cargo xtask check-purity   # rules A and B: the library stays synchronous and offline
cargo xtask determinism    # rule D: pure operations are bit-reproducible
```

Guarantee A1 - no panics on untrusted input - is checked two ways: a deterministic
mutation sweep that runs in ordinary CI, and `cargo fuzz` targets under `fuzz/` for the
surface parser and the CBOR reader.

```bash
cargo +nightly fuzz run surface
```

## Embedding

The library is the product; the CLI is its first consumer. No CLI capability is
unreachable from the library, and `default-features = false` yields a fully synchronous
library with no async runtime, no HTTP client, and no argument parser in its dependency
tree — verified in CI, not merely intended.

```toml
[dependencies]
smysl = { version = "0.1", default-features = false }
```

Determinism is part of the API surface: making `pack`, `merge`, `derive_thread`,
`salience`, or `render` non-reproducible is a breaking change regardless of signature.

## Licence

MPL-2.0.
