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

**SM-P3 — store and index.** Documents now live in an append-only log with a derived
index beside it. The log is the only authority: a stale or corrupt sidecar is rebuilt
rather than trusted, a truncated tail yields everything up to the last complete record,
and an index rebuilt from the log alone is byte-identical to the one maintained while
appending. Traversal runs over a purpose-built adjacency store whose dense ids follow
ascending uid order, so iteration is canonical by construction rather than by a sort at
each step.

| Phase | Delivers | State |
|---|---|---|
| SM-P0 | scaffold, diagnostics, gates | **done** |
| SM-P1 | deterministic CBOR codec, kernel types, identity | **done** |
| SM-P2 | surface syntax, `fmt` | **done** |
| SM-P3 | store, index, adjacency, `reindex` | **done** |
| SM-P4–P5 | check pipeline, rules M and T | next |
| SM-P6–P8 | merge, lineage, salience | |
| SM-P9–P10 | packing | |
| SM-P11–P12 | threads, rendering | |
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
