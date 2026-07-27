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
| SM-P12 | rendering | |
| SM-P13–P14 | providers, ingest | |
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
