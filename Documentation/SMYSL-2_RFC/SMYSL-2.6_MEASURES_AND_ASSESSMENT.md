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
| `analyzer` | text | analyzer chain id and stream: `en/1:surface`, `ru/1:stem` | per `lang`; refused on `mul` without one |
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
| `codec` | text | encoder and version, `brotli-rs/8.0.4 q11 w24` | pinned |
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

**T2 — compression rate.**
- `brotli` quality 11, `lgwin` 24, NFC UTF-8 of each manifest's in-scope text, segments joined by
  `\n`. `bits/char = 8·C / chars`, `bits/word = 8·C / words`.
- The window is 2²⁴ − 16 bytes. A manifest longer than that is compressed in window-sized pieces
  at segment boundaries and the bits summed; the report flags it.
- Scope aggregate: `Σ 8·Cᵢ / Σ charsᵢ` over manifests.
- `miniz_oxide` and `lz4_flex` are excluded (draft 3 §13.2: windows of 32 KiB and 64 KiB).
- Cost: measured ~0.5 MB/s on English text (§2.9), O(window) memory, ~ 200 MB peak for q11 lgwin 24
  (unverified; measured in M3).

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

**T6 — normalised compression distance** (Cilibrasi, Vitányi 2005, *IEEE Trans. IT* 51(4)).
- `NCD(x,y) = (min(C(xy), C(yx)) − min(C(x), C(y))) / max(C(x), C(y))`, C = T2's codec. The
  min over both orders removes the compressor's asymmetry.
- Refused when `|x| + |y|` exceeds the window (`SMY-E484`): the compressor would not see x while
  coding y, and the distance would mean nothing.
- Matrix over the recipe's grouping (manifests, books, speakers); average-linkage clustering with
  ties broken by label order. Lengths are integers, so pairs run in parallel.
- Cost: 66 books → 2,145 pairs × ~130 KB ≈ 280 MB compressed ≈ 9 min on one core at the measured
  rate; linear speed-up across cores.

**T7 — divergences and collocations.**
- Jensen–Shannon: `JSD(P,Q) = H(M) − ½H(P) − ½H(Q)`, `M = ½(P+Q)`, bits, in [0, 1]; `√JSD` (a
  metric) beside it. Both scopes must use one analyzer and stream (`SMY-E489` otherwise).
- PMI: `log₂ (c(xy)·N / (c(x)·c(y)))` over adjacent pairs with `c(xy) ≥ params.min_pair` (default 5).
- Log-likelihood G² (Dunning 1993, *Computational Linguistics* 19(1)): `2 Σᵢⱼ Oᵢⱼ ln(Oᵢⱼ/Eᵢⱼ)` on
  the 2×2 contingency table, `0·ln 0 = 0`. Top-k by G², ties by term order.

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
  `full_tokens` and `floor_tokens` are the functions moved from `smysl-eval` (§4.3).
- For each, `pack()` with the recipe's estimator, salience request and `scope`. A budget below the
  mandatory floor gives `PackError`; the point is recorded as **refused**, not as distortion 1,
  the way `run_smysl_arm` records `refused`.
- Distortion `D(b) = D_sal + λ·D_stat`, with `D_sal = Σ_{u∉sel} s(u) / Σ s(u)` (salience mass
  excluded) and `D_stat = Σ_{u∉sel} w(status u) / Σ w(status u)`, `w = status.as_u8()` (speculative
  1 … measured 5); `λ = params.lambda`, default 1/2.
- Signature: the curve and its area by the trapezoid rule over admitted points.
- Refused on a `mul` scope with a byte estimator (`SMY-E407`). Cross-lingual A8 therefore waits for
  FC-6 (SMYSL-2.1).
- Cost: 20 packs. Greedy pack is about O(U log U) each (unverified at 10⁶; measured in N2).

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

### 4.5 Features, dependencies, purity, C toolchain

```toml
[dependencies]
smysl-core = { workspace = true }   smysl-graph = { workspace = true }
smysl-pack = { workspace = true }   smysl-text  = { workspace = true }   # SMYSL-2.4
smysl-query = { workspace = true }                                      # SMYSL-2.5
brotli      = { version = "=8.0.4", default-features = false, features = ["std"] }
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
| T2 Markov | first-order chain with known rate | within 2% |
| T3 reference | random inputs ≤ 2,000 symbols | LPF-based Λᵢ equal to the naive O(n²) definition, element by element (property test, 10⁴ cases) |
| T3 calibration | i.i.d. H = 1, 4, 6; n = 2k, 8k, 32k | bit-exact golden payloads; values reproduce §2.9's table to 3 decimals |
| T4 Zipf | terms drawn from a discrete power law with α = 2.0, 2.5 | α̂ within the bootstrap CI; CI covers α in ≥ 90 of 100 seeds (slow test, `--ignored`) |
| T4 Heaps | synthetic V(n) = 10·n^0.6 by construction | β̂ = 0.6 ± 0.01 |
| T5 | a 30-token corpus, KN probabilities computed by hand in the test's comments | exact to 10⁻¹² before quantisation |
| T5 leave-one-out | random small corpora | subtraction equals retraining from scratch (property test) |
| T6 | x vs x; x vs i.i.d. noise | NCD(x,x) < 0.1; NCD(x, noise) > 0.9; symmetric by construction |
| T6 window | two 9 MiB texts | `SMY-E484` |
| T7 | P = Q; disjoint supports | JSD = 0 exactly; JSD = 1 exactly |
| T7 G² | a 2×2 table | equals an independent computation in the Python implementation's test harness (`python/`) |

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
  reproduces its published value (value to be transcribed; unverified here).

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
| M1 | crate skeleton; `Recipe`, `Num`, `Outcome`, payload writer, scope digest, basis rule, `rng.rs`, `num.rs`; purity entry; clippy rule; diagnostics `E480`–`E489` registered | golden payload bytes for a hand-built `Value`; `cargo xtask check-purity` passes with the crate listed; registry count test updated and green |
| M2 | T1, T4, `boot.rs` | §5.1 T1 and T4 rows; bootstrap seed tests |
| M3 | T2, T6 with pinned brotli | §5.1 T2 and T6 rows; peak memory of one q11 lgwin 24 compression recorded |
| M4 | T3: SA-IS, LCP, LPF, both estimator forms, calibration fixture | LPF equals naive on 10⁴ random inputs; calibration goldens |
| M5 | T5 with exact leave-one-out | hand-computed and property tests |
| M6 | T7 | §5.1 T7 rows |
| M7 | `measure` command for T-ids, `--seed-check`, `--dry-run`, three CLI gates updated | cross-platform matrix job green; `measure T1,T2,T3,T4 --scope 'from texts where collection = bible'` on the 5-Bible fixture adds no unit on a second run; GE-T6 pure arm (T2, T3) reported |

### TX-P9 (graph, channel, dialogue, assessment)

Requires SMYSL-2.4 TX-P5 (ingest over text) and TX-P7 (classes); SMYSL-2.3's x.text/v1 holder,
mode, speaker; the effective-time index (TX-P3) for A3/A10/D*.

| step | work | exit test |
|---|---|---|
| N1 | A1–A7, A10; move E6 into `grounding.rs` | §5.2 graph tests; `smysl-eval` lib and `corpus_chain` tests pass unedited; a synthetic 10⁶-unit scope runs A1, A2, A4, A5, A7 in time within 2× of 10× the 10⁵ run (feeds GE-T7) |
| N2 | A8; move `full_tokens`/`floor_tokens`; `SMY-E407` on `mul` | monotonicity property; bit identity across the matrix; refusal on `mul` + `utf8-div4` |
| N3 | A9 | planted communities recovered exactly on a two-clique graph |
| N4 | channel binding, C1–C5 | §5.2 channel tests; C1–C4 on the KJV/Synodal Psalms fixture reported |
| N5 | D1–D6 (D5 lexical) | §5.2 dialogue tests; **GE-T8** |
| N6 | `assess key|score|state|diff`, α | §5.2 assessment tests; **GE-T3**; GE-T9 α reported |
| N7 | A8 across genres | **GE-T6** complete (A8 arm), with GE-T2's runs |

TX-P9's draft-3 exit tests are GE-T3 and GE-T6; both are listed above.

---

## 7. Risks and mitigations

| risk | effect | mitigation |
|---|---|---|
| Estimator bias on short texts | T3 is 9–23% low at ≤ 32k symbols; T1 plug-in low by ~(V−1)/(2N ln 2); Zipf MLE unstable with few types above `f_min` | n stored with every value; `min:n` refusals (`SMY-W485`); Miller–Madow beside plug-in; T3 calibration fixture; GE-T6 compares equal-length windows only |
| Analyzer dependence | type counts, T1, T4, T7, A1 change with the analyzer and stream; Russian is stem-level | analyzer and stream in the recipe; cross-scope comparisons refused on mismatch (`SMY-E489`); `stem-level` flag |
| Class-policy dependence | A2, A3, C*, D2–D4, K* move with the policy and with same-as evidence; `component` chains drift | policy mandatory (`SMY-E481`); GE-T5 gates class measures as exploration-only below precision 0.9; states diffed by membership, not id |
| Extraction instability | graph measures inherit GE-T2's run-to-run variance | GE-T2 runs first; A* reported with run spread where several runs exist |
| Codec drift | a brotli upgrade changes compressed lengths | exact pin; `codec` in the recipe; an upgrade is a new measure version |
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
| OQ-55 | Status and grounds of measure units. This RFC: `derived`, grounded on the catalog units of the texts in scope, scope pinned by digest. Alternatives: ground on a saved-query unit (`x.query/v1`), or treat a measure as an instrument reading (`measured`, `Op::Imported`, source = recipe hash), which rule T permits but which says "observed in the world" about a computation. |
| OQ-56 | Payload numbers: integers, `[num, den]` ratios and `{q20}` fixed point, under `measure:*` keys. Should the layout be declared as an extension schema (`x.measure/v1`, FC-9 fields) in SMYSL-2.3 so other implementations read it, or stay a tool convention? |
| OQ-57 | Bootstrap generator: ChaCha20 as `rand_chacha` lays it out, or a generator specified in full in SMYSL-2.3 (SplitMix64 is ten lines in any language) so the Python, JavaScript and Go implementations reproduce intervals without porting a crate's internals? |
| OQ-58 | Class identity across time. Class ids are smallest member uids and move when evidence grows. Membership-overlap correspondence (used for K4 and A3) can map one class to two. Report splits and merges explicitly, or pin states to a policy snapshot? |
| OQ-59 | T3 bias: publish the raw estimator only (this RFC), or also a value corrected from the i.i.d. calibration table, knowing that natural text is not i.i.d. and the correction may be wrong in either direction? |
