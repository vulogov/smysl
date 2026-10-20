# RFC SMYSL-2.6 — Measures and assessment

**Status:** draft 1, for discussion. Implementation RFC (non-normative); normative rules are in SMYSL-2.3.
**Author:** Vladimir Ulogov
**Date:** 2026-10-02
**Part of:** RFC set SMYSL-2 — see SMYSL-2.0 (index and roadmap).
**For:** crate `1.9.0-dev` (`d25ec9e`), format `smysl/1.0`, kernel `smysl.kernel/0.1`.
**Derived from:** RFC SMYSL-2 draft 3, §1 (research questions), §13 (measures and assessment),
§17–§18 (command surface, crates), §19 (worked examples), §22 (TX-P8, TX-P9), §23 (GE-T3, GE-T6,
GE-T8, GE-T9).
**Phases:** TX-P8 (text-level measures `T1`–`T7` only), TX-P9.
**Depends on:** SMYSL-2.1 (FC-6 script-aware estimator, normaliser v2), SMYSL-2.3 (FC-1 `lang`,
FC-3 spans, FC-5 `x.text/v1` holder/mode/speaker, records 14/15/17, class derivation `strict`),
SMYSL-2.4 (parts, manifests, analyzers, effective time, proposition layer and classes),
SMYSL-2.5 (`sq` scopes, dossiers, lens chains).

---

## 0. Summary

This RFC turns §13 of draft 3 into a crate, `smysl-measure`, and two command families, `measure`
and `assess`.

- **Every measure is a pure function of `(store, substrate, recipe)`** unless it is labelled
  model-dependent. Its result is written as a `@data` unit with status `derived`, whose payload
  holds the value and the recipe. Same recipe, same scope, same data: same uid. A rerun adds
  only an attestation; a changed input is a new unit beside the old one.
- **Determinism needs three things the tree does not have yet.** Transcendental functions
  (`ln`, `log2`, `exp`) from a pure-Rust `libm` rather than the platform's: on this machine glibc's
  `ln` and the `libm` crate disagree in the last bit for 2.0% of a million random inputs
  (§2.9). Exact payload numbers: payload floats are binary32 quantised to 1/1024, which is too
  coarse for a JSD of 0.01 and loses integers above 2¹⁴; ratios are stored as integer pairs and
  real values as 2⁻²⁰ fixed point. A seeded generator for bootstrap intervals.
- **`smysl-eval` stays the harness.** Its `Outcome::NotRun` rule becomes the crate-wide rule. Its
  token accounting (`full_tokens`, `floor_tokens`) and warrant density (E6) move into
  `smysl-measure` and are called from both.
- **Three deviations from draft 3, each argued in §3.1:** a measure is grounded on a small
  **basis** (the catalog units of the texts in scope), not given `deps` on everything it read;
  not-run outcomes are reported but not persisted; `T3` is computed per manifest and aggregated,
  because the match-length estimator is biased low at Bible-book lengths (measured, §2.9).
- New diagnostics `SMY-E480`–`SMY-E489`. New open questions OQ-55–OQ-59.

---

## 1. Scope

**In:**
- Crate `smysl-measure`: recipe, scope binding, outcome, payload encoding, refusal rules.
- Text-level measures `T1`–`T7` (TX-P8). `T8` is declared and always not run (model-dependent,
  deferred).
- Assertion-level `A1`–`A10`, channel `C1`–`C5`, dialogue `D1`–`D6` (TX-P9). `D5` ships its lexical
  form; the embedding form waits for SMYSL-2.5's embeddings (TX-P10).
- Assessment `K1`–`K4`: keys, response ingestion, scoring, knowledge states and their diff.
- CLI: `measure`, `assess key|score|state|diff`.
- Tests and the measurement side of GE-T3, GE-T6, GE-T8 and GE-T9.

**Out:**
- Class derivation (`strict`, `component`, `attested:n`) and same-as proposal: SMYSL-2.4. This crate
  consumes classes. Draft 3 §18 listed "proposition classes" under `smysl-measure`; the set moves
  them to the proposition layer's owner so `sq`'s `align by class` and measures share one function.
- `sq` parsing and planning, dossiers: SMYSL-2.5. This crate receives a resolved scope.
- Effective time: SMYSL-2.4. This crate reads intervals.
- Truth. Assessment is relative to texts (draft 3 §24). No measure, score or report says a
  proposition is true.

---

## 2. What exists today (verified against `d25ec9e`)

### 2.1 `smysl-eval`

`crates/smysl-eval` (`publish = false`, 1,120 lines in `src/`). Normal dependencies: `smysl-core`,
`smysl-graph`, `smysl-check`, `smysl-pack`. Dev-dependencies: `smysl-provider` (gemini, deepseek)
and `smysl-ingest` (model). Its 11 lib tests and the 9 `corpus_chain` tests pass at `d25ec9e`
(run for this RFC).

| file | what is there | fate |
|---|---|---|
| `src/lib.rs` | `Metric` enum E1–E9 with `id()` and `is_structural()` (E3, E4) | stays |
| `src/measure.rs` | `Outcome { Observed(f64), NotRun(&'static str) }`, `Measurement { metric, outcome, unit }`, `measure(store, run) -> Vec<Measurement>`; E1 token cost, E2 claim survival, E3 rule M/T violations via `smysl_check::check(.. only([Epistemics, Trust]))`, E4 worst-hop rebuttal ratio, E5 gist coverage via `smysl_core::tokens`, E6 warrant density, E7 CBOR round trip; E8, E9 `NotRun` with a reason | E6 body moves to `graph::grounding` (A5); the rest stays |
| `src/chain.rs` | `Arm { Prose, Smysl }`, `Budget { Tokens, Fraction }`, `ChainOptions` (5 hops, `Fraction(0.6)`), `Hop`, `Run`; `unit_tokens`, `floor_tokens` (all units at `L0`), `full_tokens` (each at its richest level); `run_smysl_arm` packs `hops` times with `Estimator::default()` and `SalienceRequest::default()` | `floor_tokens`, `full_tokens` move to `graph::rd` (A8) |
| `src/prose.rs` | ports `Summariser` and `Judge`; `Claim`, `Verdict { present, as_stated, attributed_to }`, `Judged { surviving, inflated, abstained, total, attributed, attributable }`, `judge()`; `Judged::is_usable()` requires abstentions below half | stays; `Judge` is the model port K3 reuses for model-judged same-as (§3.9) |

Two behaviours `smysl-measure` must **not** copy:
- `ratio(part, whole)` in `measure.rs` returns `1.0` when `whole == 0`. E2 on an empty input is
  "everything survived". Under draft 3 §13.1 a ratio with an empty denominator is **not run**.
  `smysl-eval` keeps its behaviour (its tests and reports depend on it); the new crate refuses.
- E4's `fold(f64::INFINITY, f64::min)` with a `1.0` fallback has the same shape.

### 2.2 Salience and quantisation

- `smysl_graph::salience` (`crates/smysl-graph/src/salience.rs`): personalised PageRank, `DAMPING =
  0.85`, accumulated in `f64`, quantised once at the end. Its module comment states the
  determinism argument this RFC reuses.
- `smysl_core::quantise(v: f32) -> f32` (`crates/smysl-core/src/types/mod.rs`): rounds to 1/1024,
  saturates non-finite input to `±MAX_QUANTISED`, maps NaN to 0.
- **No transcendental function appears anywhere in `crates/*/src` at `d25ec9e`** (searched for
  `ln()`, `log2`, `.exp(`, `powf`, `powi`). `recency_at` uses powers of two
  (`1.0 / (1u32 << d) as f32`). Measures will be the first hash-path code to call `ln`/`log2`.
- Store iteration is deterministic: `Store` keeps `units: BTreeMap<Uid, Unit>` and `relations:
  BTreeMap<(String, Uid, Uid), Relation>` (`crates/smysl-graph/src/store/mod.rs`).

### 2.3 Payload encoding

`crates/smysl-core/src/surface/payload.rs`:
- `object_to_payload(&HObject) -> Option<Vec<u8>>` writes a deterministic CBOR map with text keys
  sorted by encoded bytes; values NFC-normalised; an empty object gives no payload.
- **Floats:** `HValue::Float(f) => e.f32q(*f as f32)`: binary32, quantised to 1/1024. `is_lossless`
  documents that floats do not round-trip. Integers (`HValue::Int(i64)`) do.
- Consequence for measures: a value written as a payload float has an absolute resolution of
  ~0.001 and, being `f32`, loses the 1/1024 grid above 2¹⁴. A JSD of 0.0123 or a C2 of 0.9973 would
  be stored as 0.0127 and 0.9971. §3.6 avoids payload floats entirely.

### 2.4 Units, statuses, rungs

- `UnitCoreBuilder::new(schema, gist, status)` with `.deps`, `.grounds`, `.source`, `.payload`
  (`crates/smysl-core/src/types/unit.rs`). `UnitCore::new` refuses `derived`/`inferred` without
  grounds (`ShapeError::GroundsRequired`) and `measured`/`cited` without a source.
- `KernelType::Data` exists (`crates/smysl-core/src/ids.rs`). The only producer today is CSV import
  (`crates/smysl-ingest/src/import.rs`): `@data`, `measured`, `source` = `<file>#row=N`,
  attestation `Op::Imported` at `Rung::Computed`, every cell in the payload, gist cut to `l0_max`.
- Rule T (`crates/smysl-ingest/src/ceiling.rs`; check pass `crates/smysl-check/src/passes/trust.rs`):
  `Rung::Computed` caps at `derived`; `Attestation::ceiling()` lifts it to `measured` only for
  `Op::Imported` at `Rung::Computed` (`crates/smysl-core/src/types/provenance.rs`).
- Rule M (`crates/smysl-check/src/passes/epistemics.rs`): a `derived` unit's status ≤ the minimum
  status of its present grounds, else `SMY-E030`.
- `Attestation` has `recipe: Option<[u8; 32]>` and `family: Option<[u8; 32]>`, documented as
  hashes of a model call's conditions. A measure's recipe hash fits the same slot.

### 2.5 Pack and estimators

`crates/smysl-pack`:
- `Estimator` is `#[non_exhaustive]` with one variant `Utf8Div4`, id `smysl/utf8-div4`, cost
  `smysl_core::tokens(t) + 2` (`src/cost.rs`); `Estimator::ALL`, `parse(id)`.
- `PackRequest` (`src/solve.rs`): `budget`, `reserved`, `focus`, `mode`, `estimator`, `external`,
  `max_lod`, `scope`, `support`, `exact_threshold`; builders `budget()`, `scoped()`,
  `resting_on()`, `exact()`. `pack(store, &SalienceReport, &PackRequest) -> Result<Pack,
  PackError>`; `Pack { selection, info, why, report }`, `PackInfo` records the estimator id.
- FC-6 (`smysl/script-aware/1`) is SMYSL-2.1's work. A8 refuses a byte estimator on a `mul` scope
  (`SMY-E407`, draft 3 Appendix D).

### 2.6 Graph functions the measures call

| need | function (verified) |
|---|---|
| A5 grounds-chain depth | `smysl_graph::lineage::trace(store, root, TraceKind::Grounds, depth) -> Lineage` with `max_depth()`; `rests_on(store, uid, &EdgeSet)` transitive; `EdgeSet::{premises, dependency, support, one}` |
| A5 unfounded | `Store::is_unfounded(&uid)` |
| A5 warrants | `Store::relations_of_kind(&RelKind::Warrant)` (E6 uses it) |
| A7 contentions | `Store::open_contentions()`, `contention_status()`; `Contention { id, over, positions, detected, status }`; `DetectionKind` has four kinds, none for cross-text contradiction yet |
| D1, K1 | `RelKind::Answers` is a kernel relation (`crates/smysl-core/src/types/relation.rs`) |
| D2 | `RelKind::{Elaborates, Rebuts, Answers}`; restatement is same-as (SMYSL-2.4) |
| agreement (GE-T9, `attested:n`) | `Store::attestations_of`, `attested_by(&uid) -> BTreeSet<&AgentId>`, `agreement(&uid, n)` |
| rebuttals | `Store::rebuttals_of`, `is_live_rebuttal` |
| scoping before `sq` | `Store::units_with_source_prefix(prefix)` |
| hashing | `smysl_core::hash_bytes(&[u8]) -> [u8; 32]` (BLAKE3), `canonical_uid(&UnitCore)` |

### 2.7 Tokenisation

`smysl_retrieve::tokenize` (`crates/smysl-retrieve/src/tokenize.rs`) splits on
`!is_alphanumeric() && c != '_' && c != '-'`, lowercases, and **emits the whole token and then its
identifier parts** (`poolSize` → `poolsize`, `pool`, `size`). `Tokenizer::folding()` adds English
suffix folds beside the term. Right for retrieval; wrong for statistics, where one word would count
as three types and three tokens. Text-level measures use SMYSL-2.4's analyzer chains, which emit
one surface stream and a separate stem stream; the recipe names the stream.

### 2.8 CLI and gates

- `src/main.rs`: `enum Purity { Pure, Mixed, Model }`, `struct Cmd { name, about, purity, phase }`,
  `const COMMANDS` with 26 entries; dispatch by `match name`.
- Three gates fail when a command is added: `command_table_matches_section_23` in `src/main.rs`
  (`assert_eq!(COMMANDS.len(), 26)` and the name list), `tests/dispatch.rs` (`const COMMANDS:
  [&str; 26]`), `tests/cli-surface.txt` (every argument of every subcommand).
- **`--seed-check` is declared as a global flag** ("Assert this invocation is bit-reproducible") **and
  read nowhere** in `src/` or `crates/` at `d25ec9e`. It appears only in `cli()` and
  `tests/cli-surface.txt`.
- Purity gate `xtask/src/purity.rs`: `PURE_CRATES` (core, graph, check, pack, thread, render,
  retrieve); `FORBIDDEN_DEPS` (tokio, ureq, clap, ratatui, crossterm, reqwest, hyper, async-std,
  smol, rustls, serde_json); a source grep for `std::net`, `TcpStream`, `async fn` and others.
- Diagnostics registry: `crates/smysl-core/src/diag.rs`, a `registry!` macro with 8 groups and a
  test `assert_eq!(Code::ALL.len(), 57)` and `assert_eq!(Group::ALL.len(), 8)`. New codes are
  appended in blocks at the end so nothing is renumbered.

### 2.9 Probes run for this RFC

A scratch crate outside the tree (`brotli 8.0.4` with `default-features = false, features =
["std"]`, `libm 0.2.16`, `rand_chacha 0.9.0` with `default-features = false`), release build, one
core of this machine:

| probe | result |
|---|---|
| dependency tree | `brotli` → `brotli-decompressor`, `alloc-no-stdlib`, `alloc-stdlib`; `rand_chacha` → `rand_core`, `ppv-lite86`, `zerocopy`; `libm` alone. No `cc`, no `-sys`, no `getrandom`. None in `FORBIDDEN_DEPS`. |
| licences | brotli `BSD-3-Clause AND MIT`; libm `MIT`; rand_chacha `MIT OR Apache-2.0`. All compatible with MPL-2.0. |
| brotli q11, lgwin 24, i.i.d. uniform, n = 10⁶ | 2 symbols: 1.0002 bits/symbol; 16: 4.0003; 256: 8.0000 |
| brotli q11 throughput | 402,311 bytes of the repository's own `Documentation/*.md` + `*.typ`: 0.80 s, 2.283 bits/byte. About 0.5 MB/s. The i.i.d. inputs took 0.8–4.3 s per MB. Repeated compression gave identical lengths. |
| Kontoyiannis match-length estimator (increasing window, `Λᵢ / log₂(i+1)` form), i.i.d. | true H = 1: 0.869 / 0.896 / 0.910 at n = 2k / 8k / 32k. H = 4: 3.08 / 3.21 / 3.30. H = 6: 4.32 / 4.48 / 4.60. The mean-ratio form `mean(log₂(i+1)/Λᵢ)` is less biased (0.925, 3.36, 4.68 at 32k) and still low. |
| `f64::ln` (glibc) vs `libm::log` | 20,098 of 10⁶ random inputs in (0, 1000] differ in the last bit |

The probes are evidence, not tests; §5 turns each into a test.

---

## 3. Design

### 3.1 Deviations from draft 3, and why

1. **Grounds, not `deps` on everything.** Draft 3 §13.1 gives a measure `deps` on what it was
   computed over. A `derived` unit must have grounds (§2.4), and rule M caps it at its weakest
   ground. Grounding a measure on the units it counts breaks twice: a measure over 10⁶ units would
   carry 32 MB of uids in its core, and a measure over a scope holding one `speculative` unit
   could be at most `speculative`. That second effect is also wrong in meaning: "41% of these units
   are speculative" does not become less certain because the units are.
   **Rule here:** a measure's grounds are its **basis**, the catalog units (`x.text/meta`,
   SMYSL-2.4) of the manifests whose text or units are in scope, deduplicated and sorted; `deps` is
   empty. The scope itself is pinned by a digest in the payload (§3.6). Catalog units are `cited`,
   so rule M holds. A scope with no basis (a store with no manifests, such as the eval fixtures)
   gets its value printed and **not persisted** (`SMY-W483`). OQ-55 asks whether this is right.
   *As built (TX-P8 M1):* the basis is **the scope saved as an `x.query/saved` unit**, OQ-55's
   own alternative. The catalog units this rule names do not exist: SMYSL-2.4 keeps a manifest
   as a record identified by a `Mid`, which is not a unit and cannot be a ground, and nothing
   writes `x.text/meta`, so the rule as written would persist no measure at all. A scope is an
   sq query, and TX-P8 step 1 made a query that ran into a `cited` unit with an `answers` edge
   to each unit it answered with; `smysl_measure::resolve` runs the scope and builds it, and its
   uid is the measure's one ground. `W483` keeps its meaning for a scope given as a set rather
   than a query (`Resolved::of_uids`), which has a digest and nothing to ground on.
2. **Not-run is reported, not persisted.** A unit asserts a value. A refusal is a diagnostic and a
   line in the report, never a unit with an empty value.
3. **`T3` per manifest.** §2.9 shows the estimator is 9–23% low at n ≤ 32k symbols and converges
   slowly. Concatenating manifests in a scope would let matches cross text boundaries and mix
   lengths. `T3` is computed per manifest and aggregated (§3.7). Its report states n.

### 3.2 Module tree

```
crates/smysl-measure/
  Cargo.toml
  src/
    lib.rs            Measure trait, registry (ALL), run(), re-exports
    recipe.rs         Recipe, Params, ClassPolicyRef, Axis, Level, recipe hash
    scope.rs          binding to smysl_query::Scope; Resolved; scope digest
    outcome.rs        Outcome, Value, Num, NotRun reasons; to_payload(); gist text
    num.rs            libm wrappers, Kahan-free fixed-order sums, q20 fixed point
    rng.rs            seeded generator, index derivation (§3.5)
    boot.rs           segment bootstrap, percentile intervals
    text/
      mod.rs          term streams from analyzers, segment views
      entropy.rs      T1 plug-in + Miller–Madow
      compress.rs     T2 brotli q11 w24, encoder id
      lz_match.rs     T3 SA-IS, LPF array, estimator
      zipf_heaps.rs   T4 discrete power-law MLE, Hurwitz zeta, Heaps fit, bootstrap
      kneser_ney.rs   T5 interpolated KN, leave-one-segment-out
      ncd.rs          T6 matrix, window guard, average-linkage clustering
      divergence.rs   T7 JSD, PMI, Dunning G²
    graph/
      density.rs      A1
      redundancy.rs   A2
      novelty.rs      A3
      epistemic.rs    A4
      grounding.rs    A5 (+ E6 warrant density, moved from smysl-eval)
      concentration.rs A6 Gini of salience
      contention.rs   A7
      rd.rs           A8 (+ full_tokens, floor_tokens, moved from smysl-eval)
      entity.rs       A9
      dating.rs       A10
    channel/
      mod.rs          Channel binding, alignment by class
      c1.rs c2.rs c3.rs c4.rs c5.rs
    dialogue/
      d1.rs … d6.rs
    assess/
      key.rs          K1
      score.rs        K3 (K2 is ingestion, outside this crate)
      state.rs        K4 states and diff
      agreement.rs    Krippendorff's α (GE-T9)
```

### 3.3 The `Measure` trait and the registry

```rust
pub trait Measure: Sync {
    /// "T2", "A8", "K3".
    fn id(&self) -> &'static str;
    /// Bumped whenever the formula or a default parameter changes. Part of the recipe.
    fn version(&self) -> u16;
    fn purity(&self) -> Purity;               // Pure | ModelDependent
    fn needs(&self) -> Needs;                 // TEXT | UNITS | CLASSES | TIME | SALIENCE | PACK
    /// Refusals that do not need the computation: lossy text, mul+byte estimator, no policy,
    /// undated scope, compressor window. Pure and cheap; runs before `run`.
    fn admit(&self, r: &Recipe, input: &Input<'_>) -> Result<(), Refusal>;
    fn run(&self, r: &Recipe, input: &Input<'_>) -> Outcome;
}

pub const ALL: &[&dyn Measure] = &[&T1, &T2, /* … */ &K3];   // registry order = report order
```

`Input` carries `&Store`, the resolved scope, the substrate handle (SMYSL-2.4's object store and
readings), classes when `needs` includes `CLASSES` (computed once per run and shared), and the
effective-time index when it includes `TIME`. A run of several ids computes each shared input
once.

### 3.4 Recipe

The recipe is the payload's `measure:recipe` map. Every field from draft 3 §13.1, plus what this
RFC found the measures depend on:

| field | type | meaning | default |
|---|---|---|---|
| `id`, `v` | text, int | measure id and version | — |
| `scope` | text | the `sq` scope as written, `using` pins included (SMYSL-2.5) | required |
| `scope:digest` | text `b3:` | BLAKE3 of `"smysl/measure-scope/1\0"` ‖ sorted tids (text measures) or sorted uids (graph measures) | computed |
| `lenses` | text `b3:` | lens-chain hash the scope resolved under (draft 3 §15.3) | computed |
| `analyzer` | text | analyzer chain id and stream: `en/1:surface`, `ru/1:stem`; *as built (M2)*, the full SMYSL-2.4 chain id, `smysl/an-en/1:surface` | per `lang`; refused when the scope's texts choose two chains (`SMY-E489`) |
| `estimator` | text | pack estimator id (A1 profile, A8) | `smysl/utf8-div4` |
| `profile` | text | granularity profile (A1) | required for A1 |
| `policy` | text | class policy `strict`, `component`, `attested:n` | required for class measures (`SMY-E481`) |
| `level` | text | segment level: `part`, `chapter`, `verse`, `message`, `window:<n>` | per measure |
| `axis` | text | `said`, `composed`, `about`, `known` | `said` |
| `filter` | text | unit filter for "what the text asserts" | `holder = text and mode = asserted` |
| `undated:max` | ratio | largest undated share a time-ordered measure accepts (`SMY-E482`) | 1/10 |
| `lossy` | bool | admit lossy texts (`SMY-W408`) | false (`SMY-E480`) |
| `min:n` | int | smallest sample the estimator accepts (`SMY-W485`) | per measure, §3.7 |
| `seed` | int | bootstrap seed | 0 |
| `boot:b` | int | bootstrap replicates | 1000 |
| `codec` | text | encoder and version, `brotli-rs/8.0.4 q11 w24`; *as built (M3)*, with the encoder's build, `brotli-rs/8.0.4 no-std q11 w24` (§3.7 T2) | pinned |
| `params` | map | measure-specific (n-gram order, budgets, λ) | per measure |

The **recipe hash** is `hash_bytes` of the canonical CBOR of this map. It goes into the
attestation's `recipe` slot (§2.4), so "which runs used the same recipe" is answerable without
decoding payloads.

### 3.5 Determinism

The target is rule D: the same `(store, substrate, recipe)` gives the same payload bytes on
x86_64 and aarch64, Linux, macOS and Windows.

- **Order.** Every iteration that feeds an accumulation runs in a fixed order: `BTreeMap` order for
  units, span order for text, term-id order for count tables. `HashMap` (randomly seeded per
  process) is allowed for lookup only; a lint test greps `smysl-measure` for `.iter()` on a
  `HashMap` binding and fails (crude, like the purity grep, and on purpose). No parallel
  reduction of floats. **Parallelism is allowed where results are integers** (compressed lengths
  in T2/T6, counts per segment) and joined in index order.
- **Arithmetic.** Accumulate in `f64`. Rust does not contract `a*b+c` into FMA without explicit
  `mul_add`, and basic operations are correctly rounded IEEE 754. Transcendentals are not: every
  `ln`, `log2`, `exp`, `powf`, `lgamma` goes through `libm` with `default-features = false` (no
  `arch` intrinsics), wrapped in `num.rs`. A `clippy.toml` with `disallowed-methods` for
  `f64::{ln, log2, log10, exp, powf, powi}` scoped to this crate enforces it (the repository has no
  `clippy.toml` today).
- **Quantisation.** Once, at the end, into the payload encoding of §3.6. Intermediate values are
  never rounded.
- **Random numbers.** Bootstrap draws come from `rand_chacha::ChaCha20Rng` seeded with the first
  32 bytes of `hash_bytes("smysl/measure-rng/1\0" ‖ recipe hash ‖ seed)`, read with
  `RngCore::next_u64` only. An index in `[0, n)` is `((x as u128 * n as u128) >> 64) as usize`,
  specified here rather than taken from `rand`'s `gen_range`, whose algorithm is not a format
  promise. Bias is below 2⁻⁴⁰ for n < 2²⁴. A second implementation must reproduce the ChaCha20
  block layout `rand_chacha` uses (OQ-57).
- **Pinned versions.** `brotli` and `libm` are pinned exactly (`=8.0.4`, `=0.2.16`) and recorded in
  `codec`. A brotli upgrade can change compressed lengths. It is a recipe change and a new
  measure version, never a silent difference.

### 3.6 Results as units

**Unit shape.**
- Schema `data` (kernel), status `derived`, grounds = basis (§3.1), no `deps`, no `source`.
- Gist: the measure, its scope alias and the headline value rendered **from the stored integer**
  (`T2 brotli rate of kjv-1769: 2.2830 bits/char`), cut to the gist bound as `import.rs`'s
  `fit_gist` does. Rendering from the integer keeps the gist deterministic.
- Attestation: the running agent, `Op::Transformed`, `Rung::Computed`, `recipe` = recipe hash,
  `ts` from the CLI's clock. The clock is outside the core, so it does not touch identity.
- Payload (deterministic CBOR via `object_to_payload`, **integers only**):

| key | content |
|---|---|
| `measure:id`, `measure:v` | `"T2"`, `1` |
| `measure:recipe` | the map of §3.4 |
| `measure:n` | sample sizes: `{chars, words, segments, units, classes, …}` as integers |
| `measure:value` | named results; each a **num** |
| `measure:ci` | optional; `{lo, hi, level, b}` per bootstrapped result |
| `measure:series` | optional arrays (A3 curve, A8 points, T5 per-segment surprisal top-k) |
| `measure:flags` | optional text list: `lossy-admitted`, `low-n`, `stem-level` |

A **num** is one of three CBOR shapes, chosen by the result's nature:
- **count:** an integer.
- **ratio:** `[num, den]`, both integers, never reduced. A2, C1, C2, D1 are ratios of counts and are
  stored exactly.
- **fixed:** `{q20: i}` with value `i · 2⁻²⁰`, `i = round_half_even(v · 2²⁰)` computed in `f64`. Used
  for entropies, exponents, divergences, Gini. Resolution 9.5·10⁻⁷, range ±8.8·10¹².

Surface form, Psalms of the KJV:

```
@data { status: derived, grounds: [b3:…kjv-catalog],
        "measure:id": "T2", "measure:v": 1,
        "measure:recipe": { scope: "from texts where alias = kjv-1769 and book = Ps",
                            "scope:digest": "b3:…", lenses: "b3:…", level: "part",
                            codec: "brotli-rs/8.0.4 q11 w24", seed: 0 },
        "measure:n": { chars: 243117, bytes: 243117 },
        "measure:value": { "bits/char": { q20: 2393899 }, "bits/word": { q20: 10727114 } } }
~ T2 brotli rate of kjv-1769 Ps: 2.2830 bits/char
```

(numbers illustrative). Same recipe and data give the same core and so the same uid; the store
deduplicates the append. Old results are never overwritten (draft 3 §13.1).

**Measures aggregate upward** (draft 3 §13.1 principle 4) by a rule fixed per measure, never by
averaging values: T1 and T7 pool counts, T2 sums bits and characters, T3 aggregates harmonically
(§3.7), A* ratios add numerators and denominators.

### 3.7 Text-level measures (TX-P8)

Notation: a scope's text is a sequence of segments at the recipe's `level`; `N` tokens (analyzer
terms of the recipe's stream) or characters (Unicode scalar values after NFC), `V` types, `cᵥ`
counts, `p̂ᵥ = cᵥ/N`. Logs are base 2 unless written `ln`.

**T1 — unigram entropy.**
- Plug-in: `Ĥ = −Σ p̂ᵥ log₂ p̂ᵥ`, summed in term-id order.
- Miller–Madow: `Ĥ_MM = Ĥ + (V̂ − 1) / (2N ln 2)`, `V̂` = observed types (Miller 1955).
- Both stored; the report states the stream (`surface`/`stem`) because Russian and German statistics
  are stem-level until a lemmatiser exists (draft 3 OQ-2). Bootstrap CI over segments.
- `min:n` default 10,000 tokens. Cost O(N) time, O(V) memory.
- *As built (M2).* **A text scope is the `from` of an sq query.** An sq scope answers with units,
  and a text has none until something is extracted from it, so a text measure reads the query's
  `from` items, resolved to manifest heads by SMYSL-2.5 §3.3's own definitions (`texts`,
  `library`, `store:`, `collection:`, `import:`), and refuses every stage after it (`SMY-E461`):
  `where book = Ps` is a filter on units, and reading it as one on text would give sq/1 a meaning it
  does not have. A narrower text is named by its alias, `from collection:"kjv/19-ps"`. A forked
  alias is refused (`SMY-E489`) rather than counted twice; `channel:` waits for the measures over
  messages. The basis is the whole query, saved, as §3.1 has it. **The streams are
  `smysl_text::Chain::surface` and `::stems`**, one term per word each, added in M2: §2.7's
  separate streams did not exist, and `Chain::terms` interleaves a stem with its surface form only
  when they differ. A type's id is its rank in byte order of the term, so term-id order is a
  property of the vocabulary. Segments with no terms are dropped; `window:<n>` does not run
  across two expressions.

**T2 — compression rate.**
- `brotli` quality 11, `lgwin` 24, NFC UTF-8 of each manifest's in-scope text, segments joined by
  `\n`. `bits/char = 8·C / chars`, `bits/word = 8·C / words`.
- The window is 2²⁴ − 16 bytes. A manifest longer than that is compressed in window-sized pieces
  at segment boundaries and the bits summed; the report flags it.
- Scope aggregate: `Σ 8·Cᵢ / Σ charsᵢ` over manifests.
- `miniz_oxide` and `lz4_flex` are excluded (draft 3 §13.2: windows of 32 KiB and 64 KiB).
- Cost: measured ~0.5 MB/s on English text (§2.9), O(window) memory, ~ 200 MB peak for q11 lgwin 24
  (unverified; measured in M3).
- *As built (M3).* **The codec is brotli without its `std` feature**, and the build is in the
  codec's name, `brotli-rs/8.0.4 no-std q11 w24`, because it changes the output. With `std` the
  encoder's cost model takes `f32::log2` from the platform for every count past 255; without
  it the same function is a table and shifts, and the encoder's remaining floating point is
  IEEE `f32` arithmetic, which every target rounds alike. Over 29 of the repository's own files
  the two builds disagree on 24, by −9 to +176 bytes (0.17% at most), so a platform `log2` one
  bit off would be enough to move a T2 value — rule D (§3.5) is what rules `std` out. The
  allocator `std` would have brought is twenty lines of safe code (`text::compress::Heap`), and
  the input goes to the encoder whole, in one `FINISH`, so a length cannot depend on how a
  reader chunked it (with `std`, chunked and one-shot agreed on all 29). A `brotli` with `std`
  anywhere in a build would switch the encoder by feature unification; the codec's lengths for
  the four slice files are pinned in `fixtures/measure/compress.tsv`, so that would fail a
  test rather than move a value. **T2 is the codec's rate, not the source's.** Brotli codes a
  literal with a prefix code under a context, and for UTF-8 the context is the *class* of the
  previous two bytes — lowercase vowel, lowercase consonant, uppercase, digit, space, a few
  punctuation classes — not the bytes. An i.i.d. source over 2, 16 and 256 symbols reads
  1.00026, 4.00030 and 8.00048 bits/char (§5.1's row, 0.5%); a first-order chain over sixteen
  letters, each moving to one of eight with dyadic probabilities, rate 2.75, reads 3.75, because
  the letters are one class; and a chain of the same rate over eight characters in eight
  classes reads 2.7517. A prefix code reaches a rate only when the probabilities are powers of
  ½, and very skewed rows cost more even under distinct contexts, LZ copies winning the parse:
  ½, ¼, ⅛ … at 1.98 bits reads 2.31. §5.1's Markov row
  is therefore the chain brotli can model, which shows T2 reads a rate when the codec has the
  model and says nothing about the gap when it has not; GE-T6 compares T2 across texts, where
  the codec is the same for both. **`min:n` is in characters, 1**: §3.7 gives T2 none, and a
  rate over no characters is 0/0. A corpus also keeps each expression's segments as text
  (`text::Doc`), NFC again since NFC is not closed under slicing; at `window:<n>` the
  characters are the parts'. Reported: `bits/char`, `bits/word`; `n` holds `chars`, `bytes`,
  `words`, `compressed`, `texts`, `segments`, `pieces`; a text in more than one piece is
  flagged `windowed`. **Peak memory** of one compression, measured with `examples/codec_peak`
  under `/usr/bin/time -l`, release, this machine: **597 MB** for a full window (16 MiB − 16) of
  words drawn from a seeded vocabulary, 110 MB at 4 MiB and 42 MB at 1 MiB, and 75 MB for 16
  MiB of this directory's RFCs repeated — so the peak follows what the text is as well as its
  length, and the RFC's 200 MB was low by three for the worst case. Throughput was 1.3 MB/s on
  the words and 15 MB/s on the repeated text, both above §2.9's 0.5 MB/s.

**T3 — match-length entropy rate** (Kontoyiannis, Algoet, Suhov, Wyner 1998, *IEEE Trans. IT* 44(3)).
- `Λᵢ` = 1 + the length of the longest prefix of `x[i..]` that also starts at some `j < i`
  (overlap allowed). Estimator `Ĥ = (n−1) / Σ_{i=1}^{n−1} Λᵢ / log₂(i+1)`; the mean-ratio form
  `(1/(n−1)) Σ log₂(i+1)/Λᵢ` (Gao, Kontoyiannis, Bienenstock 2008) is stored beside it.
- `Λᵢ` for all i in O(n) from the longest-previous-factor array (Crochemore, Ilie 2008) over a suffix
  array (SA-IS, Nong, Zhang, Chan 2009) and its LCP (Kasai et al. 2001). Hand-rolled SA-IS
  generic over `u32` symbols, so the same code runs on characters and on term ids. (The `suffix`
  crate indexes `&str` only — unverified — and is not used.)
- Per manifest; aggregate `Ĥ_scope = Σ nᵢ / Σ (nᵢ / Ĥᵢ)`. `min:n` default 50,000 symbols.
- Bias: low by 9–23% below 32k symbols on i.i.d. input (§2.9). Every T3 value carries n. A
  calibration table (i.i.d. sources at H = 1…8 bits, n = 10³…10⁷) ships as a fixture so a reader
  can see the bias at their n; whether to publish a corrected value is OQ-59.
- Memory: text (4n for `u32` symbols) + SA (4n) + LCP (4n) + inverse SA (4n) = 16n bytes. Peak per
  manifest; a Bible at 4.3 M chars (unverified size) is about 70 MB.
- *As built (M4).* `text::lz_match`: SA-IS over `u32` symbols, Kasai's LCP, and LPF in one pass
  with a stack (a suffix's longest previous factor is shared with the nearest suffix on either
  side of it in suffix order that starts earlier, and the stack holds exactly those). **The
  symbols are `params.symbols`**, `chars` by default — scalar values after NFC, a text's
  segments joined by `\n` as T2 reads them — or `terms`, the analyzer stream; a symbol's id is
  its rank in the text's alphabet, which Λ cannot see. **Per text, not per manifest**: a text
  is a scope's expression, `Doc` in M3's corpus, which has one head manifest (two are refused
  already), so the two are the same. `H` is the increasing-window form and `H:mr` the
  mean-ratio form; `H` aggregates harmonically as above, and `H:mr`, a mean of per-symbol
  rates, as the mean over every symbol, `Σ nᵢ H:mrᵢ / Σ nᵢ`. A scope of more than one text stores
  each text's values beside them (`H:<alias>`, `H:mr:<alias>`, `symbols:<alias>`). **`min:n`
  holds for every text**, not for the scope's sum: the bias is a function of a text's own n, so
  a short text is a biased term in the aggregate whatever the total; the refusal names the
  shortest. **The calibration table** is `fixtures/measure/t3-calibration.tsv`: H = 1…8 bits at
  n = 10³, 2k, 8k, 10⁴, 32k, 10⁵, 10⁶, 10⁷, seed 0, both forms and the payload's hash, written by
  `examples/t3_calibration` (9 s in release). At 10⁷ symbols the estimator is still 6% low at
  H = 1 (0.942) and 16% low at H = 8 (6.718). **It is not monotone in n on large alphabets**:
  Λᵢ is an integer near `log₂ i / H`, so the sum moves in steps as `log₂ i` crosses multiples of
  H — H = 6 reads 4.601 at 32k and 4.827 at 10⁵; H = 8 reads 6.107 at 32k and 6.091 at 10⁵; H = 7
  reads 5.780 at 10⁶ and 5.727 at 10⁷ — and the spread over seeds is a few thousandths, so the
  steps are the estimator's and not noise. A correction interpolated in n (OQ-59) would be
  wrong between steps. **Peak memory**, measured with `examples/t3_calibration <bits> <n>`
  under `/usr/bin/time -l`, release, this machine, less the harness's corpus measured the same
  way: about **100 MB at 4.3 M characters** (H = 1; 122 MB at H = 8) and 238–289 MB at 10⁷,
  23–29 bytes a symbol rather than 16, because SA-IS's top level holds the text, its type bits,
  the LMS map, the suffix array and the LMS names at once. Time is 0.3–0.4 s at 4.3 M.

**T4 — Zipf and Heaps.**
- Zipf: fit a discrete power law `P(f) = f^{−α} / ζ(α, f_min)` to the term **frequencies**
  `cᵥ ≥ f_min` by maximum likelihood (Clauset, Shalizi, Newman 2009, *SIAM Review* 51(4), §3).
  `f_min` minimises the Kolmogorov–Smirnov distance over candidates in increasing order. α by
  golden-section search on [1.01, 6] for 64 fixed iterations; Hurwitz ζ by Euler–Maclaurin with 10
  terms and 4 Bernoulli corrections, all through `libm`. Report α, `f_min`, KS distance, and the
  rank–frequency exponent `s = 1/(α − 1)`.
- Heaps: `V(n) = K nᵝ`, ordinary least squares of `ln V` on `ln n` at 64 geometric checkpoints in
  reading order.
- CI: segment bootstrap (resample segments with replacement, concatenate in drawn order, refit),
  `boot:b` replicates, 2.5/97.5 percentiles. Cost O(B·N) with B = 1000: about 10⁹ term visits for a
  Bible, minutes on one core (estimate); B is a recipe field.
- *As built (M2).* **α's interval resamples types, not segments** — Clauset et al.'s own
  bootstrap, over the measurements the fit is of. The segment bootstrap is biased for it: a
  segment drawn twice adds its tokens twice and no new type, so a replicate's rare types are fewer
  and more frequent than the text's, and α\* runs high. On this section's own synthetic text at
  α = 2.5 the segment interval was [2.502, 2.831] around α̂ = 2.484; resampling types, §5.1's
  coverage row holds at 96 and 95 seeds of 100 for α = 2.0 and 2.5. **β keeps the segment
  bootstrap**, since a vocabulary curve is about reading order, and has the same bias, which no
  resampling of a text avoids; a result whose stored interval excludes its estimate carries the
  flag `ci-off-estimate`, and OQ-89 asks what the interval should be. **Heaps' checkpoints run
  from ⌈√N⌉, not from 1**: `V(n) ≤ n` binds before Heaps' law does, below n = 10^2.5 for §5.1's
  `10·n^0.6`, and checkpoints from 1 put 42% of the fit at slope 1 (β̂ ≈ 0.8 for a text built to
  have 0.6). **A candidate `f_min` must leave a tail of `params.tail:min` types**, default 50,
  recorded in the recipe; without a floor the search fits the three most frequent words. The KS
  distance is evaluated at each observed frequency `uⱼ` and at `uⱼ₊₁ − 1`, which is the supremum
  over every integer. Hurwitz ζ's first neglected term, B₁₀'s, is at most 3·10⁻¹³ relative over
  [1.01, 6]. `min:n` is T1's, 10,000 tokens, since §3.7 gives T4 none and the two read one count
  table. Reported: `alpha`, `s`, `ks`, `f:min`, `heaps:beta`, `heaps:K`; `n` holds `tokens`,
  `types`, `tail`, `segments`.

**T5 — n-gram surprisal.**
- Interpolated Kneser–Ney (Chen, Goodman 1998, TR-10-98, §3), order 3 default, one absolute
  discount per order `D_k = n₁/(n₁ + 2n₂)` from count-of-counts. Unknown words: the order-1
  continuation distribution interpolates with uniform over `V + 1`.
- Leave-one-segment-out: global counts minus the segment's own n-grams, with continuation counts
  `N₁₊(•w)` maintained by multiplicity so subtraction is exact, then restored. Equal to retraining
  without the segment (property-tested, §5.3).
- Per segment: mean surprisal `−(1/|s|) Σ log₂ P(wᵢ | wᵢ₋₂ wᵢ₋₁)` in bits/token. The top-k segments
  by surprisal go into `measure:series` with their locators.
- Memory: about 1.5 M distinct n-grams for a Bible-sized text at order 3 (estimate), ~100 MB.
- *As built (M5).* `text::kneser_ney`. A segment is padded on the left with `order − 1`
  beginning marks, which are context and never predicted; the top order counts n-grams and each
  lower order counts **continuations, `N₁₊(• g)`, as the number of distinct left extensions
  one order up**, so every order follows from the top order's counts and a count that appears or
  vanishes moves exactly one continuation below it. `Model` holds each order's counts, each
  context's total and type count, and each order's n₁ and n₂ as integers; removing a segment
  subtracts its top-order n-grams through that cascade, so the model without it is the same
  integers as the model never given it — **the discounts and V included, recomputed and not
  frozen** — and every probability is the same to the bit. An order with neither ones nor twos
  has D = 0, and a word it then gives probability 0 makes the result not run rather than
  infinite. `params.order` (1–5, default 3) and `params.top:k` (default 10). `H` is `Σ bits /
  Σ tokens` over every segment scored by the others; `min:n` is 10,000 tokens and a scope needs
  two segments (`SMY-W485`). **Locators**: a payload series holds numbers only, so the top-k are
  stored by locator as `top:<locator>` (mean surprisal in `measure:value`, tokens in
  `measure:n`), and the series `top` is the rank order, `[ordinal, tokens, mean]` each, ties to
  the earlier segment; a corpus now keeps each segment's locator — `<alias>#<locator>` at a
  named level (`kjv/65-jud#Jude.1.20`), `<alias>#part:<k>` and `<alias>#window:<k>` otherwise.
  The full model's discounts are stored as `D:<k>`. **Peak memory**, `examples/t5_peak` under
  `/usr/bin/time -l`, release, this machine: **155 MB and 2.0 s** for 800,000 tokens of a seeded
  Zipf vocabulary of 12,000 types in 25-word segments (700k distinct trigrams), the corpus
  included; 27 MB at 100,000.

**T6 — normalised compression distance** (Cilibrasi, Vitányi 2005, *IEEE Trans. IT* 51(4)).
- `NCD(x,y) = (min(C(xy), C(yx)) − min(C(x), C(y))) / max(C(x), C(y))`, C = T2's codec. The
  min over both orders removes the compressor's asymmetry.
- Refused when `|x| + |y|` exceeds the window (`SMY-E484`): the compressor would not see x while
  coding y, and the distance would mean nothing.
- Matrix over the recipe's grouping (manifests, books, speakers); average-linkage clustering with
  ties broken by label order. Lengths are integers, so pairs run in parallel.
- Cost: 66 books → 2,145 pairs × ~130 KB ≈ 280 MB compressed ≈ 9 min on one core at the measured
  rate; linear speed-up across cores.
- *As built (M3).* **The grouping is the scope's expressions**, in alias order: a library names
  a book as an alias (`kjv/08-rut`), so a scope of one Bible's books is a matrix over books,
  and speakers wait for the measures over messages. `xy` is the two texts joined by `\n`, as a
  text's own segments are. The window guard runs in `admit`, on the two longest texts, so a
  refused pair costs no compression. NCD is a ratio of integers that can fall a little below 0
  for a compressor that is not normal and is stored as `q20` rather than as a ratio, which is
  unsigned. **The clustering is a series, `linkage`**: four numbers a merge, as SciPy writes
  one — two nodes, smaller id first; the height; the size — with leaves `0..n` in alias order,
  the order of the `ncd:<a>|<b>` value keys, and the k-th merge node `n + k`. Ties go to the
  pair whose labels (smallest leaves) come first, by scanning in label order for the first
  strict minimum; distances update by Lance–Williams. `min:n` is in texts, 2. Runs are
  sequential for now; the parallelism §3.5 allows is M7's, with a worst-case 600 MB a worker
  (T2's peak) to plan for. Over the four slice books (KJV and Synodal, Ruth and Jude) the books
  of each Bible merge first, at 0.966 and 0.971, and every cross-language pair is above 1.

**T7 — divergences and collocations.**
- Jensen–Shannon: `JSD(P,Q) = H(M) − ½H(P) − ½H(Q)`, `M = ½(P+Q)`, bits, in [0, 1]; `√JSD` (a
  metric) beside it. Both scopes must use one analyzer and stream (`SMY-E489` otherwise).
- PMI: `log₂ (c(xy)·N / (c(x)·c(y)))` over adjacent pairs with `c(xy) ≥ params.min_pair` (default 5).
- Log-likelihood G² (Dunning 1993, *Computational Linguistics* 19(1)): `2 Σᵢⱼ Oᵢⱼ ln(Oᵢⱼ/Eᵢⱼ)` on
  the 2×2 contingency table, `0·ln 0 = 0`. Top-k by G², ties by term order.
- *As built (M6).* `text::divergence`. **JSD is between every two texts in scope**, keyed
  `jsd:<a>|<b>` and `jsd:sqrt:<a>|<b>` as T6 keys its pairs: a measure has one scope, and a
  scope's texts are read under one analyzer and stream by construction, so the `SMY-E489` above
  is the corpus loader's refusal of texts that choose two chains. It is computed as `½ (1/N_P) Σ
  c_P log₂(2 c_P N_Q / (c_P N_Q + c_Q N_P)) + ½ (1/N_Q) Σ c_Q log₂(…)` over integer products,
  which makes **both of §5.1's ends exact** rather than three rounded entropies subtracted:
  equal distributions give `log₂ 1` = 0 in every term, and disjoint supports give `log₂ 2` = 1
  in every term and `N/N` after the sum. Over the four slice books, every pair across the two
  languages is 1 exactly. **Collocations** are adjacent pairs within a segment, never across
  two, with the 2×2 table taken from the pair table's own margins (`N` pairs, `R` pairs
  starting with x, `C` ending with y), so G² and PMI read one table. **G² ranks attraction
  only** — `O₁₁ > E₁₁`, compared in integers — since G² is as large for a pair seen less often
  than chance and that is not a collocation. PMI's threshold is `params.min:pair` (default 5,
  spelt as the crate's other parameters are); `params.top:k` (default 20) for each list.
  Stored by terms, `g2:<x> <y>` and `pmi:<x> <y>`, with the pair's count as `pair:<x> <y>` in
  `measure:n`. `min:n` is 10,000 tokens, T1's. No headline: like T6, a result of many values.

**T8 — neural surprisal.** Declared, `ModelDependent`, always `NotRun("deferred: needs a candle LM")`.

### 3.8 Assertion-level measures (TX-P9)

All read units through SMYSL-2.5's lens application. "Asserted by the text" is the recipe `filter`,
default `holder = text ∧ mode = asserted` (draft 3 §7.4).

| id | computation | cost at 10⁶ units |
|---|---|---|
| A1 | `units / words × 1000`, by type, holder, mode; words from the analyzer over the spans in scope. Refuses without `profile`. | O(U + words) |
| A2 | `1 − classes/units` as the ratio `[units − classes, units]`, within each manifest and across the scope | O(U + same-as edges) given classes |
| A3 | units ordered by span start (reading order) or effective time on `axis`, ties by uid; `new(i) = 1` if i's class is first seen at i. Curve in 64 equal-count bins; integral = classes ÷ units. Undated units excluded and counted (`SMY-W415`); above `undated:max`, refused (`SMY-E482`). | O(U log U) |
| A4 | `H(status)` over the six statuses, unfounded read through `Store::is_unfounded`; hedging rate `[asserted units below cited, asserted units]` | O(U) |
| A5 | grounded share `[claims with ≥ 1 ground, claims]`; warrant density (E6's code, moved); `[unfounded, units]`; grounds-chain depth max and median from `trace(.., TraceKind::Grounds, None)` per claim, memoised in topological order (`smysl_graph::topo`) so the total is O(U + E) | O(U + E) |
| A6 | Gini of salience: `G = 2 Σᵢ i·s₍ᵢ₎ / (n Σ s) − (n + 1)/n` over sorted quantised scores from `salience()` with the recipe's request | salience cost (PageRank) + O(U log U) |
| A7 | open contentions touching the scope plus live `x.text/contradicts` relations, per 100 units, within each text and across | O(C + R) |
| A8 | see below | k packs |
| A9 | mention graph over `x.text/mentions`: degree, eigenvector centrality (power iteration, 100 fixed steps, `f64`), communities by synchronous label propagation in uid order with smallest-label ties (deterministic), event chains = longest paths in the DAG of `sequences` and `x.text/precedes`. Betweenness only on request, on the entity graph, Brandes O(VE). | O(E·100) |
| A10 | shares of units undated / dated by recording / by documentary dating / contested, per scope; median and p90 width of effective intervals, in days | O(U) given the time index |

**A8 — rate–distortion.**
- Budgets `bₖ = fₖ · full_tokens(scope)` for `fₖ ∈ params.fractions` (default 0.05, 0.10, …, 1.00).
  *As built (N2):* `fₖ = k / params.steps`, `steps` 20, each budget rounded half to even.
  `full_tokens` and `floor_tokens` are the functions moved from `smysl-eval` (§4.3).
- For each, `pack()` with the recipe's estimator, salience request and `scope`. A budget below the
  mandatory floor gives `PackError`; the point is recorded as **refused**, not as distortion 1,
  the way `run_smysl_arm` records `refused`.
- Distortion `D(b) = D_sal + λ·D_stat`, with `D_sal = Σ_{u∉sel} s(u) / Σ s(u)` (salience mass
  excluded) and `D_stat = Σ_{u∉sel} w(status u) / Σ w(status u)`, `w = status.as_u8()` (speculative
  1 … measured 5); `λ = params.lambda`, default 1/2.
- Signature: the curve and its area by the trapezoid rule over admitted points. *As built (N2):*
  the curve is the running minimum of the packs' distortions, since the greedy pack's own is
  not monotone; each pack's is `series.D:packed`.
- Refused on a `mul` scope with a byte estimator (`SMY-E407`). Cross-lingual A8 therefore waits for
  FC-6 (SMYSL-2.1).
- Cost: 20 packs. Greedy pack is about O(U log U) each (unverified at 10⁶; measured in N2).
  *Measured (N2):* 0.22 s at 10⁴ units, 3.06 s at 10⁵, 35.4 s at 10⁶ — 11.6× per decade.

### 3.9 Channel measures

A **channel** binds a source scope S and a target scope T (`from channel kjv-1769 -> syn-1876`,
SMYSL-2.5). S and T must not overlap (`SMY-E489`). Classes come from the recipe's `policy`.

- **Alignment.** A class is aligned when it has members in both. For a target unit t in an aligned
  class, its partner s* is the source member with the **highest** status, ties by smallest uid.
  Choosing the strongest source makes inflation conservative: t is inflated only if no source
  member supports its status.
- **C1** `[source classes with a target member, source classes]`.
- **C2** `[target units in an aligned class, target units]`; the complement is addition. When T was
  produced by a model (attestation rung `model`), the report calls it fabrication.
- **C3** distribution of `Δ = status(t) − status(s*)` over −5…+5 as counts; inflation
  `[Δ > 0, pairs]`; mode-change matrix (7×7 counts over `text:mode`).
- **C4** `[pairs whose holder differs, pairs]`; holders compared by entity class under the same
  policy, `text` equal only to `text`.
- **C5** `T2(T)/T2(S)` and `T3(T)/T3(S)` as fixed nums, beside C1. Requires both texts' T2/T3 under
  identical `codec` and `level`.

`smysl-eval`'s E2 (claim survival) is C1 under an **identity** policy (every unit its own class),
and E3 is the structural check C3 cannot replace (rules M and T over survivors). They stay in the
harness; the documentation states the correspondence.

### 3.10 Dialogue and temporal measures

Over chat manifests (SMYSL-2.4 readers), speakers from `text:speaker`, time on `axis` (default
`said`, effective).

| id | computation |
|---|---|
| D1 | `[question units with a live answers edge, question units]`; time to answer = earliest answering unit's effective `said` minus the question's, median and p90 in seconds; unanswered excluded from times and counted |
| D2 | per speaker: `[asserted units that another speaker's unit elaborates, rebuts, answers or joins by same-as, asserted units]` |
| D3 | per class: last reference minus first utterance; a reference is a member or a relation into a member. Median over classes |
| D4 | per speaker: classes first introduced ÷ classes, minus messages ÷ messages; both stored as ratios, the gap as fixed |
| D5 | lexical: `JSD` between consecutive windows' term distributions (`level = window:<n>`), pure. Embedding centroids: `ModelDependent`, not run until TX-P10 |
| D6 | per speaker and class: reversals between `asserted` and `negated` over effective time; ties in time broken by uid |

D1, D3 and D6 are time-ordered and obey `undated:max` (`SMY-E482`).

### 3.11 Assessment K1–K4

**K1 — keys.** `assess key <view|dossier> [--scope S] [--policy P]`.
- From a view or dossier: items are the `question` units in it with live `answers` edges; a dossier
  (SMYSL-2.5) yields one item per slot holding units.
- From generation: `assess key --generate --recipe assess-key/1 --scope S`. Answer candidates are
  selected purely (claims matching `filter`, top-k by salience, ties by uid); the model writes a
  question per candidate through `ingest` with the item's answer unit in the prompt as data
  (D-10 delimiting). Questions are staged (rule S) and grounded on their answers. Mixed purity.
- Each item records its **difficulty prior**: `d_path`, the shortest path along `grounds`
  (`EdgeSet::one(EdgeKind::Grounds)`) from the answer to a unit carrying `source.span`; `d_spans`,
  distinct `(tid, span)` among the evidence reached.
- The key is a view (roots = item questions) plus a `@data` key unit, status `derived`, grounded on
  the questions, payload `key:items` = `[{q, answers: [uid…], mode: any|all, d_path, d_spans}]`,
  `key:scope`, `key:policy`. **Answer uids are stored, not class ids**: a class id is the smallest
  member uid and moves when a smaller member joins (draft 3 §9.1), so classes are resolved at
  scoring time under the recorded policy.

**K2 — responses.** Not new code in this crate. A reader's answers are ingested through SMYSL-2.4's
text ingest with `--agent reader:<id> --recipe assess-response/1 --key <key uid>`:
- each answer segment yields units with an `answers` edge to its item's question;
- the attestation agent is the reader; a model reader is rung `model`, a human reader's typed
  answers rung `document`;
- the extraction keeps the reader's own hedges as statuses and modes. That is what makes status
  fidelity measurable.

**K3 — scoring.** `assess score <key> --reader <id> [--policy P] [--judge lexical|anchored|model]`.
1. Propose same-as between response units and keyed answers (SMYSL-2.4 engines). The `model` judge
   uses `smysl-eval`'s `Judge` port shape and makes the run `Mixed`.
2. Per item: matched if a response unit shares a class with a keyed answer (`any`), or with each
   (`all`, scored as a ratio).
3. Status fidelity: C3 over matched pairs. Holder fidelity: C4.
4. Fabrication: `[response units whose class has no member in the key's scope, response units]`.
   Relative to the texts, never the world (draft 3 §24).
5. Unanswered items are counted apart from wrong ones.
6. Response units with no `answers` edge to an item are counted and skipped (`SMY-W487`). Items
   whose question or answer has since been withdrawn, retracted, superseded or become unfounded
   are excluded and counted (`SMY-W488`).

The score is a `@data` `derived` unit grounded on the key unit and the reader's response units for
the items (bounded by the key size), payload `score:items` and `score:summary` (accuracy ratio,
inflation ratio, holder-change ratio, fabrication ratio, unanswered count) with the judging
recipe.

**K4 — knowledge states.** `assess state <key> --reader <id>` writes a view whose roots are one
representative per reproduced class (smallest uid) and a `@data` state unit listing, per class,
**all** reproduced member uids. `assess diff <state-a> <state-b>` compares by membership: two
classes correspond when they share a member under the current policy, so a class id that moved
does not read as a lost class (OQ-58). Output: gained, lost, kept, and status changes on kept
classes.

**Agreement (GE-T9).** `assess::agreement::alpha(store, items, agents)` computes Krippendorff's α
(nominal) from attestations: annotators are `AgentId`s with rung `document`, values are the
presence of a unit or a same-as edge per item. Pure.

---

## 4. Implementation plan

### 4.1 Crates and modules

- **New:** `crates/smysl-measure` (tree in §3.2). Workspace member by the existing `crates/*` glob;
  add `smysl-measure = { path = "crates/smysl-measure", version = "1.9.0" }` to
  `[workspace.dependencies]`.
- **Changed:** `crates/smysl-eval` (gains a dependency on `smysl-measure`; two function bodies
  move), `crates/smysl-core/src/diag.rs` (codes), `src/lib.rs` (facade), `src/main.rs` (commands),
  `xtask/src/purity.rs`, test gates.

### 4.2 Public API (sketch)

```rust
// smysl_measure
pub enum Purity { Pure, ModelDependent }
pub struct Needs(u8);  // no bitflags crate
impl Needs { pub const TEXT: Needs = Needs(1); pub const UNITS: Needs = Needs(2); /* … */ }

pub struct Recipe {
    pub id: &'static str, pub v: u16,
    pub scope: String, pub scope_digest: [u8; 32], pub lenses: Option<[u8; 32]>,
    pub analyzer: Option<AnalyzerId>, pub estimator: Option<String>, pub profile: Option<String>,
    pub policy: Option<ClassPolicy>, pub level: Level, pub axis: Axis, pub filter: Filter,
    pub undated_max: (u32, u32), pub lossy: bool, pub min_n: Option<u64>,
    pub seed: u64, pub boot_b: u32, pub codec: &'static str,
    pub params: BTreeMap<String, Param>,
}
impl Recipe { pub fn hash(&self) -> [u8; 32]; pub fn to_cbor(&self) -> Vec<u8>; }

pub enum Num { Count(u64), Ratio(u64, u64), Fixed(i64) }
pub struct Value { pub n: BTreeMap<&'static str, u64>, pub values: BTreeMap<&'static str, Num>,
                   pub ci: BTreeMap<&'static str, Ci>, pub series: BTreeMap<&'static str, Series>,
                   pub flags: Vec<&'static str> }
#[non_exhaustive]
pub enum Outcome { Value(Value), NotRun { why: &'static str, code: Option<Code> } }
pub struct Refusal { pub code: Code, pub message: String }

pub struct Input<'a> { pub store: &'a Store, pub scope: &'a Resolved,
                       pub text: Option<&'a dyn Substrate>, pub classes: Option<&'a Classes>,
                       pub time: Option<&'a TimeIndex> }

pub struct MeasureResult { pub recipe: Recipe, pub outcome: Outcome, pub report: Report }
pub fn run(ids: &[&str], base: &Recipe, input: &Input<'_>) -> Vec<MeasureResult>;
pub fn to_unit(r: &MeasureResult, basis: &BTreeSet<Uid>) -> Option<UnitCore>; // None: not run, or no basis
pub fn basis(store: &Store, scope: &Resolved) -> BTreeSet<Uid>;

// moved from smysl-eval
pub mod graph::rd { pub fn full_tokens(..) -> u64; pub fn floor_tokens(..) -> u64; }
pub mod graph::grounding { pub fn warrant_density(store: &Store, uids: &BTreeSet<Uid>) -> Num; }

// assessment
pub mod assess {
    pub fn key_from_view(store: &Store, view: &View, policy: ClassPolicy) -> Result<Key, Refusal>;
    pub fn score(store: &Store, key: &Key, reader: &AgentId, classes: &Classes) -> Score;
    pub fn state(store: &Store, key: &Key, reader: &AgentId, classes: &Classes) -> State;
    pub fn diff(a: &State, b: &State, classes: &Classes) -> StateDiff;
    pub mod agreement { pub fn alpha(store: &Store, items: &[Uid], agents: &[AgentId]) -> Num; }
}
```

`Resolved`, `Classes`, `ClassPolicy`, `AnalyzerId`, `Substrate` and `TimeIndex` are SMYSL-2.4 and
SMYSL-2.5 types; the signatures follow whatever those RFCs settle.

### 4.3 Changes to existing crates

| file | change |
|---|---|
| `crates/smysl-eval/Cargo.toml` | add `smysl-measure` to `[dependencies]` |
| `crates/smysl-eval/src/chain.rs` | `floor_tokens`, `full_tokens` become re-exports of `smysl_measure::graph::rd`; signatures unchanged, so `corpus_chain` tests pass unedited |
| `crates/smysl-eval/src/measure.rs` | `warrant_density` calls `smysl_measure::graph::grounding::warrant_density` and converts `Ratio` to `f64`; `ratio()` keeps its 0/0 = 1.0 behaviour, documented as differing from §13.1 |
| `crates/smysl-core/src/diag.rs` | codes `E480`–`E489` in a block appended at the end (group per SMYSL-2.3's 4xx convention); bump `Code::ALL.len()` and, if a group is added, `Group::ALL.len()` |
| `src/lib.rs` | `#[cfg(feature = "measure")] pub mod measure { pub use smysl_measure::{Measure, Recipe, Outcome, Num, run, to_unit, assess}; }` per draft 3 §17's facade line |
| `src/main.rs` | two `Cmd` entries, dispatch arms, `cmd_measure`, `cmd_assess`; first reader of `--seed-check` (§4.4) |
| `tests/dispatch.rs`, `tests/cli-surface.txt`, `tests/public-api*.txt` | 28 commands, new arguments, new facade items |
| `Cargo.toml` (facade) | feature `measure = ["dep:smysl-measure", "text", "query"]`; `cli` enables it |

### 4.4 CLI

| `Cmd` | about | purity | phase |
|---|---|---|---|
| `measure` | "Text, graph, channel and dialogue measures as derived data units" | `Mixed` | `TX-P8` |
| `assess` | "Assessment keys, reader scores and knowledge states" | `Mixed` | `TX-P9` |

```
smysl measure <ids> --scope <sq scope> [--channel <sq channel>] [--policy P] [--analyzer A]
              [--estimator E] [--profile G] [--level L] [--axis A] [--param k=v]… [--seed N]
              [--allow-lossy] [--dry-run] [--json]
smysl assess key   <view|dossier> [--generate --recipe R --scope S] [--policy P]
smysl assess score <key> --reader <agent> [--policy P] [--judge lexical|anchored|model]
smysl assess state <key> --reader <agent> [--policy P]
smysl assess diff  <state-a> <state-b>
```

- `measure` is pure when every id is pure; `T8`, embedding `D5` and `--judge model` make an
  invocation model-dependent, and the report says so. `Cmd.purity` is `Mixed`, as for `thread`.
- **`--seed-check`** (global, unread today): for `measure` and `assess score|state|diff`, compute
  twice in-process, compare payload bytes, exit non-zero on a difference; refuse up front on any
  model-dependent id (`SMY-E416`, the code SMYSL-2.5 uses for `q`; one meaning, one code).
- `--dry-run` prints values and refusals and writes nothing. Otherwise units are appended and the
  report lists uids, refusals and not-run reasons in id order. `--json` gives the same report.
- Rule A: `cmd_measure` and `cmd_assess` call only `smysl::measure::*`.
- *As built (TX-P8 M7).* `smysl measure <IDS> --scope <SQ> [--level L] [--analyzer A]
  [--param [ID.]K=V]… [--min-n [ID=]N]… [--seed N] [--allow-lossy] [--dry-run] [--at MS]
  <library>`, the thirty-fourth command; `smysl::measure` is the facade's re-export of the crate,
  behind `text`. The flags for graph, channel and dialogue measures (`--channel`, `--policy`,
  `--estimator`, `--profile`, `--axis`) arrive with the measures that read them (TX-P9), since a
  flag no measure reads would be a recipe field written for nothing. **One recipe per id**:
  `--param` without a prefix goes to every id asked for that takes the key, `T5.order=2` to one,
  and a key no requested id takes is refused before anything runs, so a parameter cannot land in
  a recipe it does not belong to and move that result's hash; `--min-n` is per id the same way,
  which is what M3 owed (`min:n` is in each measure's own unit). The scope is the M2 form, `from
  collection:bible` rather than `from texts where collection = bible`, since a text scope is a
  `from` (§3.7 T1). `Mixed` in the command table, and `reproducible` says pure: every id this
  build registers is `Purity::Pure`, and `cmd_measure` refuses a model-dependent one under
  `--seed-check` per id. `SMY-E416` is not registered: no id can raise it until T8 or D5 exists,
  on the registry's standing rule. Writing is the CLI's: the `x.query/v1` declaration when the
  store has none, the basis and its answer edges, then each unit with an attestation of the run
  (`Op::Transformed`, `Rung::Computed`, the recipe hash, `--at` or the clock), through
  `Library::append_records`; the report counts the units that were new and those the store held.

### 4.5 Features, dependencies, purity, C toolchain

```toml
[dependencies]
smysl-core = { workspace = true }   smysl-graph = { workspace = true }
smysl-pack = { workspace = true }   smysl-text  = { workspace = true }   # SMYSL-2.4
smysl-query = { workspace = true }                                      # SMYSL-2.5
brotli      = { version = "=8.0.4", default-features = false }   # as built (M3): no `std`, §3.7 T2
libm        = { version = "=0.2.16", default-features = false }
rand_chacha = { version = "0.9", default-features = false }
rand_core   = { version = "0.9", default-features = false }
```

- `smysl-measure` joins `PURE_CRATES` in `xtask/src/purity.rs`. Its tree contains none of
  `FORBIDDEN_DEPS` (probe, §2.9) and no runtime or socket; `rand_core` without `os_rng` brings no
  `getrandom`, so the crate cannot draw unseeded randomness. A source-grep addition forbids
  `thread_rng`, `OsRng`, `SystemTime` and `Instant` in `smysl-measure/src`.
- No C: brotli is the pure-Rust port; libm is pure Rust with `arch` intrinsics off.
- `brotli` uses `unsafe` internally (8 files contain the word); `#![forbid(unsafe_code)]` applies to
  `smysl-measure` itself, as in `smysl-eval`.

---

## 5. Tests, fixtures and harnesses

### 5.1 Golden values on synthetic texts

Generated in-test from a fixed seed with the crate's own generator, so fixtures are code, not files.

| test | input | expectation |
|---|---|---|
| T1 i.i.d. | uniform over k ∈ {2, 16, 256}, n = 10⁶ | `|Ĥ_MM − log₂k| < 10⁻³`; plug-in below it by ≈ (k−1)/(2n ln 2) |
| T1 degenerate | one repeated term | Ĥ = 0 exactly; Miller–Madow 0 |
| T2 i.i.d. | as above | within 0.5% of log₂k (measured 0.03%, §2.9) |
| T2 Markov | first-order chain with known rate | within 2%; *as built (M3)*, a chain brotli's literal contexts can model — eight characters in eight context classes, dyadic rows of rate 2.75 — since over letters of one class brotli does not see the chain (§3.7 T2) |
| T3 reference | random inputs ≤ 2,000 symbols | LPF-based Λᵢ equal to the naive O(n²) definition, element by element (property test, 10⁴ cases) |
| T3 calibration | i.i.d. H = 1, 4, 6; n = 2k, 8k, 32k | bit-exact golden payloads; values reproduce §2.9's table to 3 decimals; *as built (M4)*, every row of the calibration table to 10⁵ symbols bit for bit (values and payload hash), the rest `--ignored` in release, and §2.9's numbers checked as draws of the same estimator — each within three single-draw standard deviations of the mean over 20 seeds, plus its printed rounding — since §2.9's probe drew from another generator and no generator reproduces one draw to the digit |
| T4 Zipf | terms drawn from a discrete power law with α = 2.0, 2.5 | α̂ within the bootstrap CI; CI covers α in ≥ 90 of 100 seeds (slow test, `--ignored`) |
| T4 Heaps | synthetic V(n) = 10·n^0.6 by construction | β̂ = 0.6 ± 0.01 |
| T5 | a 30-token corpus, KN probabilities computed by hand in the test's comments | exact to 10⁻¹² before quantisation; *as built (M5)*, three segments at order 3, the three discounts exactly and six probabilities against fractions worked in the comments (a seen trigram, a seen context with an unseen word, the beginning marks, an unknown word, an unseen context) |
| T5 leave-one-out | random small corpora | subtraction equals retraining from scratch (property test); *as built (M5)*, 2,000 corpora at orders 1–5, every segment: the model with it removed equal to the retrained one as integers, its surprisal equal to the bit, and the full model restored |
| T6 | x vs x; x vs i.i.d. noise | NCD(x,x) < 0.1; NCD(x, noise) > 0.9; symmetric by construction |
| T6 window | two 9 MiB texts | `SMY-E484` |
| T7 | P = Q; disjoint supports | JSD = 0 exactly; JSD = 1 exactly; *as built (M6)*, as `f64` equality and as the payload integers 0 and 2²⁰, a text twice another's length in the same proportions included |
| T7 G² | a 2×2 table | equals an independent computation in the Python implementation's test harness (`python/`); *as built (M6)*, eight tables in `fixtures/measure/g2.tsv` with this crate's G² and its q20, recomputed by `python/tests/test_measure_g2.py` from the cells with Python's own `math.log` (expectations from the margins, then `O ln(O/E)`): equal to a relative 10⁻¹² and the q20 to the unit — not to the bit, since the platform's `log` is not `libm` |

### 5.2 Graph, channel, dialogue, assessment

- Synthetic stores built with `UnitCoreBuilder` with planted structure: k self-repetitions (A2 =
  k/units), a known status mix (A4), chains of known grounds depth (A5), salience all equal (A6 = 0).
- A3: a store where the class sequence is known; the curve's integral equals classes ÷ units.
- A8: D is non-increasing in budget (property test over random stores from the existing
  `fixtures/corpus`); points below the floor are `refused`.
- C1–C4: a source and a target store with planted drops (C1), additions (C2), one inflation per
  status step (C3) and holder rewrites (C4); exact ratios.
- D1–D6: a synthetic chat with planted answers, reversals and clock order.
- K3: a reader with planted answers: m correct, j inflated, h holder-changed, f fabricated, u
  unanswered, w unscorable (`SMY-W487`), one withdrawn item (`SMY-W488`). All counts exact.
- K4: a class whose id moves when a smaller member joins between two states; the diff reports it
  as kept.
- α: the worked example in Krippendorff (2011, "Computing Krippendorff's Alpha-Reliability")
  reproduces its published value (value to be transcribed; unverified here). A second,
  already-written check to agree with: TX-P7 step 4's GE-T5 harness
  (`crates/smysl-text/tests/ge_t5.rs`) needed α before this crate existed and implements the
  nominal, missing-tolerant coincidence-matrix form, validated against arithmetic spelled out in
  full in the test rather than against a recalled figure, with `None` for each of the three cases
  the formula has no answer in — one coder, one value, nothing pairable. `assess::agreement::alpha`
  should reproduce that case too, and when it lands the harness's copy should call it instead:
  two implementations of one formula are one implementation too many, and the only reason there
  are two is that the gate came first.

### 5.3 Determinism

- **Cross-platform bit identity.** A CI matrix job (x86_64-linux-gnu, x86_64-linux-musl,
  aarch64-apple-darwin, x86_64-pc-windows-msvc) runs every golden test and writes payload bytes;
  a final job compares the hashes and fails on any difference. The musl target matters because its
  system `libm` differs from glibc; with the `libm` crate both must agree.
- **No platform transcendentals.** The `clippy.toml` rule of §3.5, and a test that runs T1 over
  inputs where `f64::ln` and `libm::log` are known to differ (taken from the §2.9 probe) and asserts
  the payload matches the stored golden.
- **Bootstrap seeds.** Same seed → identical CI bytes; seeds 0 and 1 → different replicates and
  CIs within each other's width ×2; changing `boot:b` changes the recipe hash and the uid.
- **Idempotent writes.** Running `measure` twice adds no unit the second time: every unit record
  comes back in `AppendReport::duplicates`, and only the new run's attestations are added.
- *As built (TX-P8 M7).* The matrix job is `measure-platforms` in CI: `cargo test -p
  smysl-measure` on x86_64-linux-gnu, x86_64-linux-musl, aarch64-apple-darwin and
  x86_64-pc-windows-msvc. Every payload the crate produces is pinned in a committed golden —
  recipe CBOR, payload bytes, `libm` results by bit pattern, each measure over the Bible slices,
  the T3 calibration table with each row's payload hash — so each platform is compared with the
  same bytes, and two that pass agree; the golden is the comparison job. Windows checks out
  with `core.autocrlf` off, or every input's line endings would differ before any measure ran.
  *Idempotent writes* is `tests/cmd_measure.rs` over the five-Bible slice: the second run's
  units are all already in the store, and the store gains the run's four attestations and no
  unit.
- **`--seed-check`** passes for every pure id on the 5-Bible fixture (TX-P8) and the extraction
  fixture (TX-P9).

### 5.4 GE-T6 genre discrimination protocol

- **Data.** The five Bibles of TX-P1, books labelled by catalog `meta:genre`: law (Exod, Lev, Num,
  Deut), narrative (Gen, Josh–2 Kgs, Ruth, Esth, the Gospels, Acts), poetry (Job, Ps, Prov, Eccl,
  Song), prophecy (Isa–Mal, Rev), epistle (Rom–Jude). Labels are fixtures, reviewed once.
- **Length control.** Estimators depend on n (§2.9). Each book contributes **equal-length samples**:
  m = 20 windows of 20,000 characters (T2) or 50,000 symbols (T3) at offsets drawn with the
  recipe seed. Books shorter than one window are excluded and listed.
- **Statistic.** Kruskal–Wallis H across genres on per-book medians, per language; effect size
  ε² = H/(n − 1). Permutation p-value from 10,000 seeded label shuffles.
- **"Run-to-run variance" for pure measures** is zero by construction, so the kill criterion reads
  it as **within-genre variance plus window-sampling variance** (seeds 0–9). For A8, which
  depends on extraction, it is the variance across GE-T2's 5 extraction runs.
- **Kill.** A measure whose between-genre variance is ≤ that variance in ≥ 3 of 5 languages is
  dropped from defaults, as draft 3 §23 states.
- **Reported:** per language, per measure: H, ε², p, and the genre medians with CIs.
- *As built (TX-P8 M7): the pure arm.* `crates/smysl-measure/tests/ge_t6.rs`, `--ignored`, with
  `SMYSL_GE_T6=<dir>` naming GE-T1's five Bibles as USFM (converted by `vpl2usfm.py`, the same
  bytes as GE-T1's corpus: Ruth is the committed slice byte for byte); the labels are
  `fixtures/measure/genres.tsv` and the report `fixtures/measure/ge-t6.tsv`. Windows are measured
  by T2 and T3 themselves, through `run`; a book's value is the median of its 20. The kill
  criterion's two variances are read as the **between-genre mean square** against the
  **within-genre mean square plus the window-sampling variance** (each book's value under seeds
  0–9, its sample variance, averaged over books). The four books §5.4's list does not place
  (1–2 Chronicles, Ezra, Nehemiah) are excluded and listed. **Both measures are kept, in all
  five languages**: Kruskal–Wallis H 11.0–17.0 for T2 and 8.7–13.5 for T3, p from 0.0002 to
  0.0194, ε² 0.34–0.56, and the between-genre mean square 4.9 to 10.1 times the within-genre
  one plus window variance. **Law is the lowest genre in every language under both** (T2
  2.17–3.05 bits/char by language, T3 1.15–1.39), and under T2 poetry is the highest in every
  language; under T3 the English and French epistles, one and two books, sit above poetry. Two caveats the report carries: the equal-length
  rule excludes most of the New Testament — at 50,000 characters T3 keeps one or two epistles
  per language and none in the Russian and Spanish, so its "genres" are four there — and the
  windows of a book only just longer than one overlap almost entirely, which makes the window
  variance a floor. The run takes 51 s on 16 cores in release.

### 5.5 GE-T3, GE-T8, GE-T9 (measure side)

- **GE-T3.** 50 verses × 5 translations × 5 extraction runs. A1 and class count per verse. Two-way
  decomposition (language, run) of variance per verse, summed. Language variance > run variance:
  the bias is reported as a finding and cross-lingual A* carries it (draft 3 §23).
- **GE-T8.** 300 hand-labelled question/answer pairs from 3 chat exports. D1's precision on the
  `answers` edges it counts; < 0.8 drops D1 from defaults.
- **GE-T9.** α from `assess::agreement::alpha` over 2 annotators × 20 passages, en and ru, for
  units and same-as. It is the ceiling every model figure is reported against.

---

## 6. Delivery steps

### TX-P8 (text-level, `T1`–`T7`)

Requires SMYSL-2.4 TX-P1/P2 (parts, analyzers) and SMYSL-2.5 TX-P6 (`sq` scopes).

| step | work | exit test |
|---|---|---|
| M1 | crate skeleton; `Recipe`, `Num`, `Outcome`, payload writer, scope digest, basis rule, `rng.rs`, `num.rs`; purity entry; clippy rule; diagnostics `E480`–`E489` registered | golden payload bytes for a hand-built `Value`; `cargo xtask check-purity` passes with the crate listed; registry count test updated and green — **done 2026-10-18**, below |
| M2 | T1, T4, `boot.rs` | §5.1 T1 and T4 rows; bootstrap seed tests — **done 2026-10-18**, below |
| M3 | T2, T6 with pinned brotli | §5.1 T2 and T6 rows; peak memory of one q11 lgwin 24 compression recorded — **done 2026-10-18**, below |
| M4 | T3: SA-IS, LCP, LPF, both estimator forms, calibration fixture | LPF equals naive on 10⁴ random inputs; calibration goldens — **done 2026-10-18**, below |
| M5 | T5 with exact leave-one-out | hand-computed and property tests — **done 2026-10-18**, below |
| M6 | T7 | §5.1 T7 rows — **done 2026-10-18**, below |
| M7 | `measure` command for T-ids, `--seed-check`, `--dry-run`, three CLI gates updated | cross-platform matrix job green; `measure T1,T2,T3,T4 --scope 'from texts where collection = bible'` on the 5-Bible fixture adds no unit on a second run; GE-T6 pure arm (T2, T3) reported — **done 2026-10-19**, below |

*M1, done 2026-10-18.* `crates/smysl-measure`: `Recipe` with every §3.4 field and its canonical
CBOR and hash; `Num`'s three integer shapes, `Value`, `Outcome`, `Refusal` and `to_payload`;
`num` (`libm` wrappers, `q20` as `round_ties_even(v · 2²⁰)` refusing non-finite input, decimal
rendering in integer arithmetic for the gist); `rng` (ChaCha20 keyed as §3.5 says, the seed as
eight little-endian bytes, `index` as `(x · n) >> 64`); `scope` (`digest`, `resolve`, `Basis`);
the `Measure` trait, an empty `ALL`, `find`, `run_with` and `to_unit`. The exit holds:
`tests/payload.rs` pins the recipe CBOR and hash, the payload bytes of a hand-built value
holding every shape, a whole unit's uid, six `libm` results by bit pattern and the
generator's first draws in `fixtures/measure/golden.tsv`, language-free for the ports;
`check-purity` passes with ten pure crates and a crate-specific scan; the registry is 105.
Five decisions, each recorded where it applies. **The basis is the saved scope query**
(§3.1, OQ-55), decided when M1 found that the catalog units the rule names do not exist.
**Two codes of ten are registered**, `W483` and `E489`, on the registry's standing rule that a
code is registered with the code that raises it; the other seven are refusals a measure makes
and arrive with the measures, in an eleventh group, `Measure`. **No `HashMap` at all** rather
than a lint against iterating one: the crate has no need of one yet, and absence is a check
that cannot be argued with; the source scan enforces it with the clock, the environment and
unseeded generators. **Every field with a value is written to the recipe, defaults
included**, and an absent optional is not written as a null, so a default that changes later
changes the hash and an optional field added later does not. **`brotli` is not a dependency
yet**: it is T2's, in M3, and a pinned dependency nothing calls would be a pin nobody tests.
The clippy rule is `crates/smysl-measure/clippy.toml`, and a planted `f64::ln` fails `make
lint` with it.

*M2, done 2026-10-18.* T1 and T4, the first measures, and what they read: `smysl_measure::text`
(a scope's `from` resolved to texts, the parts read through `Library` at the recipe's level into
one term stream, `Corpus`), `boot` (segment draws, nearest-rank percentiles with integer ranks,
⌈B·(den∓num)/(2·den)⌉), `text::entropy` and `text::zipf_heaps`. `smysl_text::Chain` gained
`surface` and `stems`. `Measure::complete` writes a measure's defaults — `min:n`, `tail:min` —
into its recipe, and `run` stamps a text measure's recipe with the analyzer it was read under and
the digest over tids (§3.4), and refuses a recipe naming another analyzer or level (`SMY-E489`)
and a lossy text it does not admit (`SMY-E480`); an admitted one is flagged and reported
(`SMY-W408`), and a stem stream is flagged `stem-level`. The exit holds: `tests/text.rs` runs
§5.1's T1 rows (Ĥ_MM within 10⁻³ of log₂ k for k = 2, 16, 256 at n = 10⁶, Ĥ_MM − Ĥ equal to
(k−1)/(2n ln 2) to two quanta; one repeated term gives 0 and 0 exactly), its T4 rows (α̂ within its
interval and within 0.2 of α = 2.0 and 2.5; β̂ = 0.6 ± 0.01 on a text built to `⌊10·n^0.6⌋`), and
§5.3's seed rules (one seed, one payload; seeds 0 and 1 give two intervals within twice each
other's width; `boot:b` moves the hash and the uid). `tests/coverage.rs` is §5.1's slow row,
`--ignored`: 96 and 95 of 100. Both measures over the KJV's Ruth and Jude at verse level are
pinned in `fixtures/measure/text.tsv`, language-free; the two books are 3,191 tokens, so the
default recipe reports both not run (`SMY-W485`) and the pinned run lowers `min:n` to 1,000.
Three of the RFC's codes are registered, `W408`, `E480` and `W485`, and the registry is 108.
Three findings, each recorded where it applies (§3.7): **a text scope is the query's `from`**,
because an sq scope answers with units and an unextracted text has none; **α's interval resamples
types**, because the segment bootstrap's duplicated segments add no types and its interval missed
the estimate it was for; and **Heaps' fit starts at √N**, because §5.1's own synthetic text is
`V(n) = n` below 316 tokens. On the KJV the segment intervals of T1 and of β exclude their own
estimates — [7.510, 7.726] around Ĥ = 7.813 — which the flag `ci-off-estimate` says and OQ-89
asks about.

*M3, done 2026-10-18.* T2 and T6, the two that compress: `text::compress` (the codec, brotli
`=8.0.4` without `std` and with the crate's own allocator; `pieces` at segment boundaries; T2) and
`text::ncd` (T6, the window guard and average linkage), over the characters a corpus now keeps
beside its terms (`text::Doc`). The exit holds: `tests/compress.rs` runs §5.1's T2 rows (i.i.d.
over 2, 16 and 256 symbols within 0.5% of log₂ k — 0.03% at most — and a first-order chain
within 2%, 0.06%), T6's (NCD(x, x) < 0.1 and NCD(x, noise) > 0.9 with one value a pair, the
copies merging first; two 9 MiB texts refused with `SMY-E484`), a stream decoded back to its
input, a text past the window read in two pieces, and ties in the clustering. The peak memory
of one q11 lgwin 24 compression is recorded in §3.7 T2: 597 MB at a full window, measured with
`examples/codec_peak`. T2 over the KJV's Ruth and Jude at verse level (2.7038 bits/char,
14.1724 bits/word), T6 over the four slice books, and the codec's lengths for the four slice
files are pinned in `fixtures/measure/compress.tsv`, language-free. `SMY-E484` is registered,
and the registry is 109. Three findings, recorded in §3.7: **the codec is brotli without
`std`**, because with it the encoder's cost model reads the platform's `log2` and 24 of 29
files changed length between the two builds; **T2 is the codec's rate**, since brotli's
contexts for UTF-8 are classes of characters, so §5.1's Markov row is a chain the codec can
model; and **the peak is three times the estimate**, and follows the text as well as its length.
`brotli`'s dev and test profile is optimised (`[profile.dev.package.brotli]`), which changes no
output and keeps `make test` in seconds. One thing is owed to M7: `min:n` is in each measure's
own unit — tokens for T1 and T4, characters for T2, texts for T6 — and a base recipe sets it
for every id in a run, so `measure` must scope it per id.

*M4, done 2026-10-18.* T3, the match-length rate: `text::lz_match` (SA-IS, Kasai's LCP, LPF from
both, the two estimator forms, the harmonic aggregate), over each text's characters or its
terms (`params.symbols`), one text at a time. The exit holds: `tests/lz_match.rs` checks LPF
against its O(n²) definition on 10⁴ random inputs of at most 2,000 symbols — i.i.d. over 1 to
256 symbols, repeated blocks with errors and Fibonacci words, lengths log-uniform — with the
suffix array against a comparison sort and the LCP against a direct count up to 300; every row
of the calibration table up to 10⁵ symbols bit for bit, and the rest under `--ignored`;
§2.9's probe as a draw of the same estimator; the aggregate and the per-text values; and the
recipe's defaults and refusals. T3 over the KJV's Ruth and Jude at part level, characters (H
1.8090, H:mr 2.3926 bits/char) and terms (4.2854, 5.2850 bits/token), is pinned in
`fixtures/measure/lz_match.tsv` with `min:n` set to 2, since the default refuses books of 13k
and 4k characters (`SMY-W485`). No code is new and the registry is 109. Three findings,
recorded in §3.7 T3: **§2.9's table is one draw of another generator**, so §5.1's "to 3
decimals" is checked as agreement in distribution, which holds (within 0.021, and 0.003 at
32k); **the estimator steps in n on large alphabets**, which bears on OQ-59, since a
correction read off the table between its rows would be wrong; and **the peak is 23–29
bytes a symbol**, not 16 — about 100 MB for a 4.3 M-character Bible.

*M5, done 2026-10-18.* T5, n-gram surprisal: `text::kneser_ney` (`Model`, interpolated
Kneser–Ney over integer counts that a segment is added to and removed from, and the measure,
each segment scored by the model of the others). The exit holds: `tests/kneser_ney.rs` checks
a 30-token corpus worked by hand — D₁ = 3/13, D₂ = 7/12, D₃ = 3/5 exactly, and six probabilities
within 10⁻¹² of their fractions — and removal against retraining from scratch on 2,000 random
corpora at orders 1 to 5, as integers and to the bit. T5 over the KJV's Ruth and Jude at verse
level (7.0742 bits/token, `min:n` 2 since the default refuses 3,191 tokens) is pinned in
`fixtures/measure/kneser_ney.tsv`; its ten most surprising verses are all Jude's, which is what a
model of mostly Ruth should say about them. A corpus now carries a locator per segment. No code
is new and the registry is 109. Two findings, recorded in §3.7 T5: **leave-one-out is exact
only if the discounts and V move with the counts**, so they are recomputed from integers the
model maintains rather than frozen at the full model's; and **a payload series holds numbers,
not locators**, so the top-k are keyed by locator and the series carries their rank order. The
peak is 155 MB for 800,000 tokens: half again §3.7's 100 MB, with fewer distinct n-grams than it
assumed.

*M6, done 2026-10-18.* T7, divergences and collocations: `text::divergence` (`jsd`, `g2`,
`Pairs`, and the measure). The exit holds: `tests/divergence.rs` gives JSD 0 and 1 exactly at
§5.1's two ends, as floats and as payload integers; G² by hand (`40 ln 2` for a perfect
association, 0 exactly for independence); pairs that never cross a segment; G²'s ranking of
attraction only and PMI's threshold; and the parameters' refusals. The G² tables are
`fixtures/measure/g2.tsv`, recomputed by the Python harness's new `test_measure_g2.py`, and T7
over the four slice books at part level is pinned in `fixtures/measure/divergence.tsv`: Ruth
and Jude of the KJV at 0.4921, every cross-language pair 1 exactly, and the collocations of
both Bibles (`in law`, `said unto`, `beth lehem`; `дочь моя`, `и сказал`). No code is new and the
registry is 109. Two findings, recorded in §3.7 T7: **JSD's form decides whether §5.1's ends are
exact**, so it is computed over integer products rather than as three entropies; and **G² alone
does not say which way a pair leans**, so collocations are ranked among the pairs seen more often
than chance. This completes the text-level measures T1–T7; M7 is the `measure` command.

*M7, done 2026-10-19.* `smysl measure`, the thirty-fourth command (§4.4 as built), with
`--seed-check`, `--dry-run` and `--json`; the three CLI gates — `tests/cli-surface.txt`,
`tests/dispatch.rs`, `command_table_matches_section_23` and its purity census — updated; the
manual's command reference gains `measure`. The exit: **a second run adds no unit** —
`tests/cmd_measure.rs` runs `measure T1,T2,T3,T4 --scope 'from collection:bible'` over the
five-Bible slice twice, and the store gains five units the first time (four results and the
saved scope) and none the second, only the run's four attestations; `--seed-check` passes for
all seven ids. **GE-T6's pure arm is reported** (§5.4 as built): T2 and T3 both kept, in five
languages of five. **The cross-platform matrix job is green** (§5.3 as built): its first
run, CI run 39193002211 on `5856096`, passed on all four targets — glibc, musl, macOS arm64 and
Windows — which gives the same payload bytes on each. Three findings. **CI had been red since 2026-10-10**, fifty pushes, for three
reasons none of which the local sweep could see: the doc gate on CI's newer stable rustdoc,
which now calls `[`library`](crate::passes::library)` a redundant link; two CLI test files
(`cmd_ask`, `cmd_same_as`) not gated on the features their binary needs, so every
`--no-default-features` cell failed; and `make semver` in a shallow checkout with no tag to
compare against. All three are fixed here, and the matrix job would have been one more cell
nobody read. **A stale binary hid a fourth.** The same run failed eight `--no-default-features`
cells on `tests/cmd_q.rs`, one more file that runs the binary without asking for `cli`; every
local run had passed because `target/debug/smysl` was left from an earlier build. It is gated
now, and `tests/dispatch.rs` checks from the sources that every test file running the binary
carries the gate, which no leftover in `target/` can satisfy; with the gate removed it names
the file. **And `measure` runs one recipe per id** rather than one recipe for every id,
because a parameter shared across measures would sit in the recipe of a measure that never
reads it and change its uid.

### TX-P9 (graph, channel, dialogue, assessment)

Requires SMYSL-2.4 TX-P5 (ingest over text) and TX-P7 (classes); SMYSL-2.3's x.text/v1 holder,
mode, speaker; the effective-time index (TX-P3) for A3/A10/D*.

| step | work | exit test |
|---|---|---|
| N1 | A1–A7, A10; move E6 into `grounding.rs` | §5.2 graph tests; `smysl-eval` lib and `corpus_chain` tests pass unedited; a synthetic 10⁶-unit scope runs A1, A2, A4, A5, A7 in time within 2× of 10× the 10⁵ run (feeds GE-T7) — **done 2026-10-19**, below |
| N2 | A8; move `full_tokens`/`floor_tokens`; `SMY-E407` on `mul` | monotonicity property; bit identity across the matrix; refusal on `mul` + `utf8-div4` — **done 2026-10-19**, below |
| N3 | A9 | planted communities recovered exactly on a two-clique graph — **done 2026-10-19**, below |
| N4 | channel binding, C1–C5 | §5.2 channel tests; C1–C4 on the KJV/Synodal Psalms fixture reported — **done 2026-10-19**, below |
| N5 | D1–D6 (D5 lexical) | §5.2 dialogue tests; **GE-T8** — **done 2026-10-19**, below; GE-T8's instrument built, its measurement owed |
| N6 | `assess key|score|state|diff`, α | §5.2 assessment tests; **GE-T3**; GE-T9 α reported — **done 2026-10-20**, below; GE-T3's instrument built and its runs owed, GE-T9's human α owed |
| N7 | A8 across genres | **GE-T6** complete (A8 arm), with GE-T2's runs — **built 2026-10-20**, below; the A8 arm's instrument built and its runs owed |

TX-P9's draft-3 exit tests are GE-T3 and GE-T6; both are listed above.

*N1, done 2026-10-19.* `smysl_measure::graph`: A1 (`density`), A2 (`redundancy`), A3
(`novelty`), A4 (`epistemic`), A5 (`grounding`), A6 (`concentration`), A7 (`contention`) and
A10 (`dating`), registered after T1–T7, all pure. E6's count is `graph::grounding::
warrant_counts`, and `smysl-eval`'s E6 calls it, keeping its own ratio and its 1.0 over nothing
(§2.1). `measure` runs them: `--policy`, `--profile` and `--axis` are new, and the text is read
only when an id reads text, so a unit measure's scope may filter. The exit holds:
`crates/smysl-measure/tests/graph.rs` plants §5.2's structure — four units claimed pairwise the
same among ten is `[3, 10]` and none is `[0, 10]`; two speculative, two cited and four measured is
H = 1.5 bits exactly, 1.75 after one retraction; a chain of four claims is depth 0 to 3 as
`trace` says; six unconnected units give a Gini of 0 exactly; a hundred units whose second half
repeats the first are novelty `[50, 100]` with every class new once — and A1 and the per-text
figures over the KJV's Ruth and Jude; `smysl-eval`'s 11 lib and 9 `corpus_chain` tests pass
unedited; and `tests/graph_scale.rs`, `--ignored` in release, runs A1, A2, A4, A5 and A7 over
10⁵ units in 0.32 s and 10⁶ in 4.0 s, **12.5×** against the bound of 20 (A1 1.30 s, A2 0.60,
A4 0.18, A5 1.49, A7 0.45 at 10⁶), which is GE-T7's first figure. `SMY-E481` and `SMY-E482` are
registered, and the registry is 111. Six findings, each recorded where it applies in the
crate.

- **Draft 3's default filter selects no unit this build writes.** `holder = text and mode =
  asserted` (§3.4) needs both halves true of one unit, and SMYSL-2.4 §3.4 writes `asserted` only
  beside a holder that is an `x.text/entity`, and nothing writes `x.text/v1`'s `text` literal;
  sq also reads the bare names as payload keys no unit has. Every unit measure would have
  reported every scope empty. A unit measure's default filter is therefore **empty** — the
  scope is an sq query and already says which units — and an empty filter is not written to
  the recipe, so every text measure's recipe, which keeps draft 3's default, hashes as it did and
  no text result's uid moved. A filter that is given is applied as an sq `where` predicate.
- **A2 was quadratic, in `smysl-graph`.** The strict partition (A-12.4) tried every later
  vertex against every seed. Only a seed's neighbours can join its class, and a `BTreeSet`
  holds them in uid order, so iterating them gives the same classes — the Python reference's
  four fixture tests still agree — in O(V·d). At 10⁶ units with 10⁴ same-as pairs A2 went from
  12.3 s to 0.6 s and the scale ratio from 40× to 12.5×; the exit could not have held without
  it.
- **A5's depth is the longest chain, not `trace`'s.** `trace` walks breadth first, so its depth
  is the shortest distance to the farthest unit below, which does not compose and cannot be
  memoised in topological order as §3.8 asks. The longest chain does compose; on a tree the two
  agree, and on `a` grounded on `{b, c}` with `b` on `c` it is 2 where `trace` says 1.
- **A1 counts the parts' words, not the spans'.** Spans overlap and leave gaps, so words over
  spans would count a verse twice where two claims share it and not at all where nothing was
  extracted. The denominator is each part the scope's units are drawn from, once, under the
  recipe's analyzer or the parts' language's; a missing `profile` is `SMY-E489`, since §8 gives
  it no code of its own.
- **A3 reads in reading order by default**, by text, part, the span's first byte and the
  locator, because the said-axis time of every verse of a Bible is the edition's; `order=time`
  orders by effective time with `SMY-W415` and `SMY-E482` as §3.8 says. `known` is not an axis
  rule E derives and is refused for A3's time order and A10.
- **A6 is exact.** Salience is quantised to 1/1024 in [0, 1], so the Gini is a ratio of
  integers rounded once, and equal scores give 0 exactly rather than a rounding residue.

*N2, done 2026-10-19.* `smysl_measure::graph::rd`: A8, and `full_tokens` and `floor_tokens`,
which `smysl-eval::chain` now re-exports under the same names and signatures; its 11 lib and 9
`corpus_chain` tests pass unedited. `SMY-E407` is registered (draft 3's number, filed with the
measures as `SMY-W408` is), and the registry is 112. The budgets are `params.steps` (20) equal
fractions rather than a list, which is draft 3's 0.05 … 1.00 exactly and fits the recipe's
integer parameters; `lambda` is a ratio, ½. The exit holds: `crates/smysl-measure/tests/rd.rs`
runs A8 over every fixture of `fixtures/corpus` whole and over twenty seeded random halves of
each, at 20 and 50 steps, 534 curves, and every curve is non-increasing; the curve over each
fixture, its recipe hash and its payload bytes are pinned in `fixtures/measure/a8.tsv`, which
each matrix cell compares byte for byte; and Ruth in the KJV and the Synodal together is refused
under `smysl/utf8-div4` (`SMY-E407`), each alone runs, and a manifest tagged `mul` is refused on
its own. `measure` runs A8 through `--param`, and refuses it over five translations' units and
runs it over one. Measured, as §3.8 left it: A8 over the N1 synthetic store takes 0.22 s at 10⁴
units, 3.06 s at 10⁵ and 35.4 s at 10⁶, 11.6× per decade (`graph_scale.rs`, `--ignored`). Three
findings.

- **The greedy pack's own loss is not monotone in budget.** On 12 of the 534 curves, all over
  halves and none over a whole fixture, a larger budget kept fewer of the scope's units. The
  pack is greedy by value per token and value counts detail: on one two-unit half of
  `F8b-agent-beta`, 67 tokens ship a claim, its ground and its rebuttal at `L0`, and at 68 the
  ground at `L1` is the densest item, costs 68, and nothing else fits. §5.2's property is a
  property of the rate–distortion **function**, the least distortion at a rate at most R, and
  a pack within a smaller budget is within a larger one; so `series.D` is the running minimum
  of the packs' losses, `series.D:packed` is each budget's own pack beside it, and `n.rises`
  counts the budgets where the pack lost more than an earlier one. The area is over `D`. The
  test checks the curve against the packs point by point and requires a rise on some half, so
  the minimum is not untested.
- **A scope cut from its rebuttals does not end at 0.** `full_tokens(scope)` is the scope's
  own units at their richest, and rule R will not ship a claim without the unit rebutting it
  (`ClosureCost`); a scope holding the claim and not the rebuttal cannot be carried whole for
  its own full cost. Ten of the 534 curves end above 0, every one a half; every whole fixture
  ends at 0.
- **No budget is refused.** A pack refuses only below its mandatory floor, and the floor is the
  closure of its focus; A8's recipe names no focus, so the floor is empty and every point is
  admitted. The `refused` count and its handling stay, for a recipe that one day names a focus,
  and §5.2's "points below the floor are refused" has nothing to bind on in this build.

*N3, done 2026-10-19.* `smysl_measure::graph::entity`: A9, pure, with `top:k` (10) and
`betweenness` (false) as parameters, and `entity_graph`, `centrality`, `communities`,
`betweenness` and `chains` public. The exit holds: `crates/smysl-measure/tests/entity.rs` builds
two cliques of five entities from units that mention them in pairs, half the edges written
from the entity end, and label propagation recovers both exactly, each named by its smallest
uid, with and without a bridge unit between them, settled. Beside it, by hand: a four-leaf
star's centrality is 1 and ½ exactly, the eigenvector of `A + I`; a four-entity path's middle
betweenness is ⅔; a chain of four `sequences` with a shortcut, a `precedes` pair and a
two-unit cycle are a longest chain of 4, 8 units chained and 2 cyclic. `measure` runs it with
the other unit measures. Four decisions, each where §3.8 left a choice.

- **The graph is of entities, joined by co-mention.** §3.8 says "mention graph" for degree and
  centrality and "entity graph" for betweenness. A mention is a live `x.text/mentions` edge
  between a scope unit and an `x.text/entity` unit, read in either direction as
  `smysl_text::link` reads it; two entities are joined when one scope unit mentions both,
  weighted by how many do. An entity's degree is how many scope units mention it, and an
  entity need not be in the scope to be named by it.
- **Centrality iterates `A + I`, not `A`.** On a bipartite graph — one edge is one — `A`'s
  largest and smallest eigenvalues are equal in magnitude and 100 power steps oscillate rather
  than converge; the shift moves every eigenvalue by one and no eigenvector. Each step is
  scaled to sum 1 and the result to a maximum of 1, so no square root is taken.
- **An entity's own label counts in label propagation.** The synchronous rule §3.8 names swaps
  the two labels of a single edge for ever; with the entity's own label at weight 1 it settles
  in a round, on the smaller. At most 100 rounds, and a graph still moving is flagged
  `lpa-unsettled` rather than refused.
- **Chains are counted in units, and a cycle is counted, not broken.** `sequences` and
  `precedes` do not forbid a cycle, so the DAG is the part Kahn's order reaches; a unit on or
  after a cycle is in `n.cyclic` and in no chain.

*N4, done 2026-10-19.* `smysl_measure::channel`: the binding (`Channel::bind`, `pairs`,
`strongest`) and C1 (survival), C2 (support, addition, fabrication), C3 (inflation, the eleven
`delta:` counts, the mode changes), C4 (holder changed, gained, lost, moved) and C5 (T2 and T3
ratios), all pure, registered after A10. The recipe gains `channel` and `channel:digest`,
written only when present, so no earlier recipe's hash moved; `Needs::CHANNEL`,
`Input::with_channel` and `with_target_text` carry the target; `to_unit_on` grounds a channel
result on both saved scopes. `measure` takes `--channel`. No code is new: overlap is
`SMY-E489` and a missing policy `SMY-E481`, as §3.9 and §3.8 say, and a channel with no
aligned pair refuses C3 and C4 with `SMY-W485`. The exit holds:
`crates/smysl-measure/tests/channel.rs` plants ten source and eight target units with two
drops, an addition a model attested, one inflation per status step, a level pair beside a
weaker and a stronger source member, a deflation of two steps and three holder rewrites, and
C1 = `[7, 9]`, C2 = `[7, 8]` with addition and fabrication `[1, 8]`, C3 = `[4, 7]` with four
at `delta:1`, C4 = `[3, 7]`, exactly; C5 over Ruth in the KJV and the Synodal is 3.4943 /
2.5767 = 1.3561 bits per character, from the two stored integers, with T3 flagged
`t3-not-run` below its 50,000 symbols. **Reported, on the Psalms fixture** (TX-P4's, two
psalters in two numberings and the map between them): with the six places the map names
accepted as `same-as`, KJV → Synodal under `strict` is C1 `[6, 7]` (Psalm 15 is in no row),
C2 `[6, 8]` (the Synodal's Psalm 14 and Psalm 50's title have no source), C3 `[0, 6]`, C4
`[0, 6]`; the other way, C1 is `[6, 8]`. Three findings.

- **sq's `channel:` is a conversation.** §3.9 writes the binding `from channel kjv-1769 ->
  syn-1876`; TX-P6 defined `channel:<name>` as a chat (SMYSL-2.5 §3.3) and 2.9 fixed the
  grammar, so a second meaning would make `from channel:general` ambiguous. The target is a
  second sq query, `--scope S --channel T`, which is §4.4's flag as written.
- **A channel measure is as good as the `same-as` it is given.** `anchored` proposes every
  aligned place and its neighbour, and the reverse of `Ps.51 → Ps.50` carries Psalm 50's title
  onto 51:1, so it makes 16 proposals for the fixture's 6 places. Accepted wholesale, the
  `strict` classes pair all three verses of Psalm 51 with the wrong verse of Psalm 50 and
  C1–C4 give the same numbers to the digit: they count classes, not which verse a class
  paired. The test asserts both, so the blindness is on record rather than assumed away;
  `anchored`'s own header already says one engine's proposal is not an acceptance.
- **The mode matrix has an eighth row.** §3.9's 7×7 is over `text:mode`, and a unit a reader
  drew from a Bible has none; without a `none` row the matrix would be empty where the channel
  is not. Only cells with a count are written.

*N5, done 2026-10-19.* `smysl_measure::dialogue`: D1 (answered, time to answer), D2 (uptake),
D3 (class life), D4 (initiative), D5 (lexical drift) and D6 (reversals), all pure, registered
after C5. A speaker is `text:speaker`, a mode `text:mode`, and a time the effective time on
`axis` read as an instant, the interval's lower bound (its upper when the lower is open), the
key A3 sorts by. D1, D3 and D6 share A3's `undated:max` check, now `within_undated_max`, and
flag what they leave out, so `SMY-W415` comes from the same place. No code is new: a missing
policy is `SMY-E481` for D3, D4 and D6, a level D5 cannot read is `SMY-E489`, and nothing to
count is `SMY-W485`. The exit holds: `crates/smysl-measure/tests/dialogue.rs` plants one
conversation per measure — four questions with answers a minute and five minutes late, one
answered before it was asked and one answered only from outside, D1 `[3, 4]` with a median
of 60 s and a p90 of 300 s; three take-ups among eleven assertions, D2 `[3, 11]`, with a
self-elaboration, a speakerless rebuttal and an outside one not counted; five classes two of
which are returned to, D3 median 0 s and p90 800 s; six classes over six messages, D4's gaps
1/6 and −1/6 exactly; windows 0 and 1 bit apart, D5 a mean of 1/2; and three reversals in
four speaker–class pairs, D6, a tie in time broken by uid and a `reported` unit passed over.
`measure` needs no new flag: `--policy`, `--axis` and `--level window:<n>` already reach
them. Four decisions, each where §3.10 left a choice.

- **A response counts only when both its ends are in the scope.** A conversation is the
  scope, and an edge from outside it — a later thread rebutting this one — is a different
  conversation's; the tests plant one for D1, D2 and D3. D3's "a relation into a member"
  takes every kind but `x.query/answers`, the saved scope's edge to each unit its query found
  (§3.1), which a scope over every unit contains on a rerun. Kernel `answers` is not that
  edge — OQ-49 kept the saved query off it so as not to move salience — so D1 cannot mistake a
  query for a reply.
- **An answer said before its question has no time.** A clock that disagrees with the reply
  graph is what GE-T13 plants and rule E reports; D1 counts it (`n.inverted`) rather than
  storing a negative time to answer, which would describe the clocks.
- **D4 is in reading order, over the messages units were drawn from.** §3.10 names D1, D3 and
  D6 as time-ordered and not D4, so D4 takes A3's reading order, which for a chat export is the
  order the export recorded. A message is a unit's source reference (`<tid>#<locator>`); a
  unit measure sees no message nothing was extracted from, so D4 compares what a speaker
  introduced with how many of their messages said something.
- **D5 compares full windows.** A text's last window is what was left over, and a short
  window is far from a long one because it is short; it is counted (`n.partial`) and not
  compared. `min:n` is in full windows, two by default.

**GE-T8 is owed.** Its material is 300 hand-labelled question/answer pairs from 3 chat
exports, and neither half is here: an `answers` edge between chat units is written by
extraction, which costs model turns, and a label is a person's reading of an edge.
`crates/smysl-measure/tests/ge_t8.rs` is the instrument — D1's own edges
(`dialogue::d1::answered`) over every unit of a store, looked up on a sheet of
`answer question yes|no`, a precision in integers against 4/5 and at least 300 labelled
edges — and `SMYSL_GE_T8_STORE` and `SMYSL_GE_T8_SHEET` decide the clause with no code
change. Without them a scripted arm shows the bar does what it says: 8 of 10 right meets it,
7 does not, and an edge the sheet does not label is counted and left out of the precision.
"Drops D1 from defaults" has nothing to act on yet: `measure` runs the ids it is given.

*N6, done 2026-10-20.* `smysl_measure::assess`: K1 (`key_from_view`, `Key`), K3 (`score`,
`Score`), K4 (`state`, `State`, `diff`, `StateDiff`) and `agreement::alpha`, all pure, and
`smysl assess key|score|state|diff`, the thirty-fifth command, `Mixed` for the two model
invocations §4.4 gives it and pure for everything this build has. `SMY-W487` and `SMY-W488`
are registered; the registry is 114. The exit holds: `crates/smysl-measure/tests/assess.rs`
keys a view (a question two `grounds` steps from a span is `d_path` 2) and a dossier slot (its
saved query the question, its results the answers); scores a reader against seven planted
items — three correct, one of them inflated and one with its holder changed, two wrong of
which one is fabricated, one unanswered, one whose answer was retracted after the key was
made, and a unit that answers nothing — as accuracy `[3, 6]`, inflation `[1, 3]`, holder
`[1, 3]`, fabrication `[1, 5]`, one `W487` and one `W488`; and moves a class's id between two
states by joining a smaller cited member, which the diff reports as kept, with one class
gained and one rise in status. **α reproduces Krippendorff's worked example** (2011, four
observers, twelve units, nominal): his 0.743 is 113/152 exactly here, computed in integers
over a coincidence matrix scaled by the least common multiple of the `m − 1`, with no α for
one coder, one value or nothing pairable, as the GE-T5 harness has it. Six findings.

- **`reader:<id>` is not an agent id.** K2 names readers `reader:<id>`; SMYSL-2.3's agent
  grammar has three kinds, `human`, `model` and `tool`, so a reader is `human:<id>` or
  `model:<id>`, which also says which rung its typed answers carry.
- **K2 has no ingest path yet.** §3.11 says responses arrive through text ingest with
  `--agent`, `--recipe assess-response/1` and `--key`, and "not new code in this crate"; ingest
  has none of the three, and every ingested unit is attested by `tool:smysl-ingest`. A score
  reads whatever units a reader attested with an `answers` edge to an item, however they got
  there, so it is built and tested; the ingest flags are owed with the model arm below.
- **SMYSL-2.4's engines rank and do not threshold.** `lexical` proposes each unit's top-k
  BM25 neighbours, so asked within one item it pairs every response with that item's answer
  and every reader scores 1. The judge is asked once over the whole key's answers and
  responses with `k = 1`: a response matches when its nearest unit is its own item's answer.
  A proposal is a match for this score and is not written, so scoring moves no class.
- **A key keys whatever answers its questions.** Made after a reader has responded, a key
  would take the responses for answers, since both are `answers` edges into the question. The
  tests key first; nothing refuses the other order yet, and a key made late says nothing
  about it. Recorded rather than guessed at: the remedy is a rule about which agents may key,
  which is policy.
- **A diff reports splits and merges and does not resolve them.** Correspondence by shared
  membership can map one class to two, which OQ-58 asks about; `StateDiff` counts `split` and
  `merged` beside `kept`, `lost` and `gained`, and leaves OQ-58 open.
- **The GE-T5 harness keeps its own α.** §5.2 asks it to call `agreement::alpha` once that
  exists; `smysl-measure` depends on `smysl-text`, so `smysl-text`'s test cannot depend back
  without a dev-dependency cycle. The two agree on the same three no-answer cases, and the
  measure crate's is the one with the published example behind it.

**Owed, and why.** **GE-T3** needs 25 extraction runs — 50 verses in 5 translations, 5 runs
each — a model's each; `crates/smysl-measure/tests/ge_t3.rs` is the instrument, the per-verse
two-way decomposition into language and run mean squares, summed, its scripted arm telling a
language-only table from a run-only one exactly, and `SMYSL_GE_T3` takes the runs' table.
**GE-T9's human α** needs independent annotators, and there are none; the α is built and
validated, and what is owed is the people. **`key --generate` and `score --judge model`** are
the command's model invocations and are not in this build: both need the ingest arm above,
and `smysl-eval`'s `Judge` port has no production implementation to stand on.

*N7, built 2026-10-20.* GE-T6's A8 arm is `crates/smysl-measure/tests/ge_t6_a8.rs`, and the
statistics both arms read are `tests/support/genre.rs`: the labels, Kruskal–Wallis H with ε²
and the permutation p, the bootstrap medians, the two mean squares and the verdict, moved out
of `ge_t6.rs` with their seeds and their order of operations. The pure arm was re-run on the
shared module against `fixtures/measure/ge-t6.tsv`, over GE-T1's five Bibles converted again
by `vpl2usfm.py`, and is byte-identical (51.7 s, as before). The A8 arm is §5.4 with two
readings:

- **The noise is extraction's.** A book's value is A8's `area` over its units in each run and
  the median of its runs; the noise beside the within-genre mean square is each book's sample
  variance over its runs, averaged over books — the window variance's place in the pure arm.
  A book A8 refuses in any run, or a run did not extract, is excluded and listed with the run.
- **A book is the units drawn from its text**, by the rule A3's reading order uses: the
  manifest a unit was read under when its source names one, else the head manifest listing
  its part. A unit drawn from no part is in no book.
- **No length control.** A8's budgets are fractions of the book's own `full_tokens`, so the
  curve is in the book's units already, and an equal-length window would cut a claim from the
  grounds and rebuttals rule R makes a pack carry with it — the closure A8 measures. The
  equal-length rule in §5.4 and §7 is the pure measures'.

The scripted arm runs everywhere: three planted runs over Ruth and Jude find each unit in its
book and the unit drawn from no part in none, each run's value is `run`'s over exactly that
book, and a run that did not extract Jude excludes `JUD(none in run 3)`; and twenty planted
books in five genres are kept with runs 0.001 apart (p < 0.001) and counted against with runs
0.5 apart over the same medians, so the criterion reads extraction's variance and not only the
books'. Two findings:

- **GE-T2's runs are not A8's material.** The exit test says "with GE-T2's runs", and GE-T2's
  corpus is 59 lines per language — 26 verses of narrative, 26 of law and 7 of poetry — in
  three runs, not 66 books in five; and its harness extracts into a scratch library it removes,
  so even those runs left no store to measure. What the A8 arm needs is its own: 5 editions × 5
  runs of the canon, about 780,000 verses through a model, `SMYSL_INGEST_LIVE=1` as GE-T2's,
  kept as `<edition>/<run>` stores for `SMYSL_GE_T6_A8`.
- **A8's area is at most 1 + λ**, 1½ under the default recipe, not 1: `D` is `D_sal + λ·D_stat`
  with each part a fraction. A genre median of A8 is therefore not a share of anything, and the
  report says so by giving the area and not a percentage.

**Owed, and why.** **GE-T6's A8 arm** is owed with its runs: the harness, the report
(`fixtures/measure/ge-t6-a8.tsv`, under `SMYSL_BLESS=1`) and the verdict are built, and what
is owed is the extraction. With it, TX-P9's two draft-3 exit tests, GE-T3 and GE-T6, are both
built and both owe model runs; every step's own exit test is met.

---

## 7. Risks and mitigations

| risk | effect | mitigation |
|---|---|---|
| Estimator bias on short texts | T3 is 9–23% low at ≤ 32k symbols; T1 plug-in low by ~(V−1)/(2N ln 2); Zipf MLE unstable with few types above `f_min` | n stored with every value; `min:n` refusals (`SMY-W485`); Miller–Madow beside plug-in; T3 calibration fixture; GE-T6 compares equal-length windows only |
| Analyzer dependence | type counts, T1, T4, T7, A1 change with the analyzer and stream; Russian is stem-level | analyzer and stream in the recipe; cross-scope comparisons refused on mismatch (`SMY-E489`); `stem-level` flag |
| Class-policy dependence | A2, A3, C*, D2–D4, K* move with the policy and with same-as evidence; `component` chains drift | policy mandatory (`SMY-E481`); GE-T5 gates class measures as exploration-only below precision 0.9; states diffed by membership, not id |
| Extraction instability | graph measures inherit GE-T2's run-to-run variance | GE-T2 runs first; A* reported with run spread where several runs exist |
| Codec drift | a brotli upgrade changes compressed lengths; so does its `std` feature, by the platform's `log2` (M3) | exact pin; `codec` in the recipe, the build included; an upgrade is a new measure version; `std` off, and the codec's lengths for the slice files pinned |
| Platform maths | transcendentals differ (2% last-bit disagreement measured) | `libm` only, lint, cross-platform job |
| q11 cost | ~0.5 MB/s; NCD matrices quadratic | integer results parallelise; `--param quality=` is a recipe change, never a silent default |
| Unit growth | every run writes units | identical reruns deduplicate by uid; `--dry-run`; reports group by recipe hash |
| Over-reading assessment | a score read as a verdict on truth | wording rule in reports: "relative to <scope>"; fabrication defined against texts only |
| Basis rule rejected (OQ-55) | persistence model changes | the rule is isolated in `basis()` and `to_unit()` |

---

## 8. Diagnostics allocated in this RFC

| code | meaning |
|---|---|
| SMY-E480 | measure refused: the scope contains a lossy text and the recipe does not admit lossy texts (`SMY-W408` when it does) |
| SMY-E481 | class-level measure or assessment without a class policy in its recipe |
| SMY-E482 | time-ordered measure refused: the undated share of the scope exceeds the recipe's `undated:max` (excluded units are counted under `SMY-W415`) |
| SMY-W483 | measure computed but not persisted: the scope has no basis to ground a derived unit; the value is reported only |
| SMY-E484 | NCD pair exceeds the compressor window |
| SMY-W485 | sample below the recipe's `min:n`; the measure is reported not run |
| SMY-E486 | *withdrawn: the condition is `SMY-E416` (draft 3), shared with SMYSL-2.5. The number stays unused.* |
| SMY-W487 | response unit not scorable (no `answers` edge to a key item, or no reader agent); counted |
| SMY-W488 | key item drifted: its question or answer was withdrawn, retracted, superseded or became unfounded after the key was made; excluded and counted |
| SMY-E489 | recipe incompatible: unknown measure id or version, compared scopes under different analyzers, codecs or policies, or a channel whose source and target overlap |

Reused: `SMY-E407` (byte estimator on `mul`), `SMY-W408` (lossy admitted), `SMY-W415` (undated
excluded), `SMY-E030`/`SMY-E033` (rules M and T; never expected on measure units, checked in tests).

---

## 9. Open questions

| id | question |
|---|---|
| OQ-2 (draft 3) | Cross-language lemma statistics: this RFC ships stem-level with a `stem-level` flag. |
| OQ-7 (draft 3; D-7) | `strict` normative: assumed, so A2/C1 counts agree across implementations. |
| OQ-55 | Status and grounds of measure units. This RFC: `derived`, grounded on the catalog units of the texts in scope, scope pinned by digest. Alternatives: ground on a saved-query unit (`x.query/v1`), or treat a measure as an instrument reading (`measured`, `Op::Imported`, source = recipe hash), which rule T permits but which says "observed in the world" about a computation. **Decided in TX-P8 M1: the saved-query unit.** The catalog units do not exist — SMYSL-2.4 keeps manifests as records, and a record cannot be a ground — and the saved scope query is `cited`, holds the scope's text and result hash, and points at every unit in scope; `derived` and the digest are unchanged (§3.1, §6 M1). |
| OQ-56 | Payload numbers: integers, `[num, den]` ratios and `{q20}` fixed point, under `measure:*` keys. Should the layout be declared as an extension schema (`x.measure/v1`, FC-9 fields) in SMYSL-2.3 so other implementations read it, or stay a tool convention? |
| OQ-57 | Bootstrap generator: ChaCha20 as `rand_chacha` lays it out, or a generator specified in full in SMYSL-2.3 (SplitMix64 is ten lines in any language) so the Python, JavaScript and Go implementations reproduce intervals without porting a crate's internals? |
| OQ-58 | Class identity across time. Class ids are smallest member uids and move when evidence grows. Membership-overlap correspondence (used for K4 and A3) can map one class to two. Report splits and merges explicitly, or pin states to a policy snapshot? |
| OQ-59 | T3 bias: publish the raw estimator only (this RFC), or also a value corrected from the i.i.d. calibration table, knowing that natural text is not i.i.d. and the correction may be wrong in either direction? *M4 adds:* the table is not smooth in n on large alphabets (Λᵢ is an integer near log₂ i / H, so the estimator steps — §3.7 T3), so a correction would have to be computed at the text's own n and alphabet rather than interpolated between rows. |
| OQ-89 | **The segment bootstrap is biased for a statistic of rare types.** Raised by TX-P8 M2. A segment drawn twice adds no new type, so a replicate has fewer rare types than the text, and T1's entropy and Heaps' β run low in every replicate: over the KJV's Ruth and Jude (3,191 tokens, 703 types) T1's 95% interval is [7.510, 7.726] around Ĥ = 7.813, and β's [0.591, 0.700] around 0.745. M2 stores the percentile interval as §3.7 says, flags a result whose interval excludes its estimate (`ci-off-estimate`), and resamples types for α, where Clauset et al.'s bootstrap applies. Options for T1 and β: the basic (reverse-percentile) interval, which corrects by the bootstrap's shift and so assumes the shift mirrors the estimator's own bias — true for plug-in entropy and not for a duplication artifact; m-out-of-n subsampling without replacement, which has no duplicates and changes n, which a vocabulary statistic depends on; or no interval below a V/N threshold. The bias shrinks with V/N, so at a Bible's length it may be inside the interval's width, which M7's 5-Bible run can measure. |
