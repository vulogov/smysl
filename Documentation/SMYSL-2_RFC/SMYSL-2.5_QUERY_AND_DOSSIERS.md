# RFC SMYSL-2.5 — Query, dossiers and schema evolution tooling

**Status:** draft 1, for discussion. Implementation RFC (non-normative); normative rules are in SMYSL-2.3.
**Author:** Vladimir Ulogov
**Date:** 2026-10-02
**Part of:** RFC set SMYSL-2 — see SMYSL-2.0 (index and roadmap).
**For:** crate `1.9.0-dev` (`d25ec9e`), format `smysl/1.0`, kernel `smysl.kernel/0.1`.
**Derived from:** RFC SMYSL-2 draft 3, §10.4, §11, §12, §15, §19.5, Appendix A (`x.query/v1`), Appendix C.
**Phases:** TX-P6, TX-P8 (dossiers, saved queries, thread schemas), TX-P10, TX-P12.
**Depends on:** SMYSL-2.3 (FC-5 `x.query/v1`, FC-8 thread schemas, FC-9 record 8 keys 4–5, rule N, lens ops and the lens-chain hash in Appendix B, thread-schema and role codes in A-8.2); diagnostics E416–E427 and W429–W431 from the set's registry (SMYSL-2.0 §6.1); SMYSL-2.1 (open `ThreadSchema`/`Role` enumerations, prompt-injection guard); SMYSL-2.4 (`smysl-text`: catalog, partitions, analyzers, effective time, entities, classes, alignment); SMYSL-2.6 (the `measure` output stage); SMYSL-2.7 (`lib export --target-revision` calls the backward lens defined here); SMYSL-2.8 (virtual union over shards).

---

## 0. Summary

This RFC turns draft 3's query, dossier and schema-evolution chapters into work items over the
existing tree. Four deliverables:

1. **`smysl-query`**, a new pure crate: a hand-written parser for `sq/1`, a typed AST, a planner
   that lowers each stage onto an existing function where one exists, an executor, `--explain`
   and `--why`, reciprocal rank fusion across partitions, language bridges, purity labelling,
   saved queries (`x.query/v1`) and reruns that separate the data effect from the schema effect.
2. **Schema evolution tooling.** FC-9 decoding into typed structures in `smysl-core`, a lens
   engine (forward and backward application, composition, chain hashing, unmappable counting),
   field resolution and pinned compile-forward of queries in `smysl-query`, `smysl schema
   show|check`, `smysl migrate`, and pointer lenses for `json/1`.
3. **Dossiers.** Intents and slot templates written in `sq`, the `dossier` thread schema (FC-8),
   `smysl ask`, and an optional model answer step whose units are capped by rule M.
4. **Embeddings and the natural-language front end.** S0 moves `smysl-embed` to `model2vec-rs 0.3`
   with the `wasm` feature (verified here: builds, existing tests pass, no `cc` from that
   path); S1 is a new optional `smysl-embed-candle`; the NL → `sq` front end is a port with no
   provider linked.

The CLI gains `q`, `ask`, `schema` and `migrate`, and `find` gains language and analyzer flags.
Lens laws get seeded property tests and a cargo-fuzz target from the phase they land in (D-11).

Two findings from reading the code change the design and are called out where they apply:
- **`answers` carries salience rank** (`RelKind::carries_support`, `smysl-core/src/types/relation.rs`).
  Saving a query with kernel `answers` edges to its results, as Appendix A of draft 3 proposes,
  would raise the salience of those results, so a saved query would change the answer of any
  later query ordered by salience (§3.6, OQ-49).
- **`Hybrid` deliberately does not merge engines.** A rank merge measured 0.78 MRR against pure
  semantic's 0.84 (`smysl-embed/src/lib.rs`, `Hybrid` docs). RRF in this RFC therefore fuses
  *partitions of one engine*, never lexical with semantic (§3.4, OQ-46).

## 1. Scope

**In:**
- `sq/1` grammar (draft 3 Appendix C, with the additions in §3.1), parser, AST, planner,
  executor, `--explain`, `--why`, `--seed-check`, `--save`, reruns, `diff` of result sets.
- Operator-to-API mapping for every stage (§3.3).
- Partitions and RRF fusion (§3.4), language bridges (§3.5), the independent-witness hook (§3.7).
- Saved queries as `x.query/v1` units (§3.6).
- FC-9 typed decoding, the lens engine, field resolution, `using` pins, schema/data effect split,
  `schema show|check`, `migrate`, pointer lenses (§3.8–§3.12).
- Dossier pipeline, intents, templates, the `dossier` thread, `ask`, the answer step (§3.13).
- S0 model upgrade, S1 crate, semantic sources and bridges, NL front-end port (§3.14–§3.15).

**Out:**
- Record layouts, keys, ids, the FC-9 op encoding, rule N's text, FC-8 codes: SMYSL-2.3.
- The catalog, readers, analyzers, effective time, entities, same-as and classes, the substrate
  index (persistent postings, `vectors`, `schemas` tables): SMYSL-2.4. This RFC consumes them
  through traits so `smysl-query` builds and tests against a plain `Store` first.
- `measure` implementations (SMYSL-2.6); the `measure` stage only dispatches to them.
- `lib export --target-revision` wiring (SMYSL-2.7); the backward lens it calls is here.
- `StoreRead`, shards, the virtual union as a disk structure (SMYSL-2.8). Until then,
  `from store:a, store:b` merges into a scratch in-memory `Store` (§3.3).
- Lens functions (unit conversion, arithmetic): deferred by D-5 (OQ-23).

## 2. What exists today

Everything below was read at `d25ec9e` in `/tmp/smysl`. "(unverified)" marks what was not.

### 2.1 Retrieval: `smysl-retrieve`

- `crates/smysl-retrieve/src/lib.rs`:
  - `pub trait Retriever { fn search(&self, &Query) -> Vec<Hit>; fn len(&self) -> usize; }`.
    Contract: deterministic, ties broken by uid.
  - `Query` (`#[non_exhaustive]`): `text`, `limit`, `kinds: Vec<KernelType>`,
    `min_status: Option<Status>`, `within: BTreeSet<Uid>`, `payload: Option<PayloadFilter>`,
    `schemas: BTreeSet<SchemaId>`. Builders `kinds`, `min_status`, `within`, `with_payload`,
    `schemas`. Predicates `admits_unit`, `admits_schema` (kernel `kinds` never admit an extension
    type), `admits_status`, `admits_payload`.
  - `within` restricts candidates **without** changing IDF: "IDF still comes from the whole index".
    So a per-partition IDF (draft 3 §11.2) cannot be obtained from one index plus `within`.
  - `Hit { uid, score, terms: Vec<(String, f32)> }`, `#[non_exhaustive]`, `Hit::new`,
    `with_terms`. `terms` is advisory: empty means "this retriever does not say".
  - `candidates(store)` takes **every** unit (since 1.7), superseded ones included. Nothing in
    retrieval prefers the head of a `supersedes` chain.
- `crates/smysl-retrieve/src/lexical.rs`: `Bm25::index(store)` (= `index_with(store,
  Tokenizer::plain())`). Gist, body and detail are repeated 4/2/1 times. Payload strings per uid
  come from `smysl_core::surface::payload::payload_strings`. `contributions()` gives the exact
  per-term summands that `--why` prints. Results sorted by score desc, then uid; `terms` filled
  after truncation.
- `crates/smysl-retrieve/src/tokenize.rs`: `Tokenizer { plain(), folding(), terms() }`,
  `fold_suffix`, `tokenize` splits on `!is_alphanumeric() && c != '_' && c != '-'` (line 99).
- `bm25 = { version = "2", default-features = false }` (`crates/smysl-retrieve/Cargo.toml`), so
  the crate is pure and listed in `PURE_CRATES` (`xtask/src/purity.rs`).
- `payload_strings` (`crates/smysl-core/src/surface/payload.rs`) collects **string** values
  (and string arrays) at any depth, dotted paths for nesting. Booleans and integers are not
  collected, so a `--payload text:verbatim=true` filter can never match a boolean field.

### 2.2 Semantic retrieval: `smysl-embed`

- `crates/smysl-embed/src/lib.rs`: `Model(StaticModel)` with `from_dir` (local directory only)
  and `from_bytes`; `Semantic::index(store, model)` embeds **gists only**, in uid order, in one
  batch; cosine; scores `<= 0` dropped.
- `Hybrid<L, S>` routes, it does not merge: `routes_to_lexical` is true when
  `looks_like_identifier(text)` (no whitespace, contains one of `_ . / : -`) or when `kinds` is
  non-empty and contains no interpretive kind. `INTERPRETIVE` is `Claim, Finding, Hypothesis,
  Question` — four kinds, where draft 3 §11.2 names three.
- The test `one_engine_answers_and_the_other_is_not_consulted` pins the absence of a merge.
- `crates/smysl-embed/Cargo.toml` pins `model2vec-rs = { version = "0.2", default-features =
  false, features = ["fancy-regex"] }`; `Cargo.lock` resolves `0.2.1`, `tokenizers 0.21.4`,
  `esaxx-rs 0.1.10`. The manifest comment says no feature combination of 0.2.1 avoids
  `esaxx_fast` (C++), and that model2vec-rs requires `clap` and `serde_json`.
- **Verified for this RFC (F-11):** in a scratch worktree, changing the pin to `{ version = "0.3",
  default-features = false, features = ["wasm"] }` builds offline and `cargo test -p
  smysl-embed` passes (unit 7, evaluation 1, gate 5, semantic 4; the semantic tests skip
  without `SMYSL_EMBED_MODEL`). `model2vec-rs 0.3.0`'s `wasm` feature is `["local-only",
  "tokenizers/unstable_wasm"]`, which enables `fancy-regex` and not `esaxx_fast`; `esaxx-rs`
  stays in the tree without its `cpp` feature. `clap` is optional in 0.3.0; `serde_json` is
  not. `StaticModel::from_pretrained` and `from_bytes` have the same signatures in 0.2.1 and
  0.3.0, so `Model` compiles unchanged.
- **Correction to F-11's wording.** `cargo tree -p smysl-embed -e normal,build -i cc` on the
  patched tree still shows `cc v1.4.0` — as a **build-dependency of `blake3`**, through
  `smysl-core`, not through the embedder. The workspace takes blake3 with `features = ["std",
  "pure"]` (root `Cargo.toml`), whose comment says this keeps the C SIMD backend out; that no C
  file is compiled was not checked from a build log (unverified). The accurate claim is
  "the S0 path adds no `cc`", not "no `cc` in the tree".
- Not checked: that `potion-multilingual-128M` loads through `from_dir` (no model in the
  sandbox; unverified). `fixtures/embed-tiny/` holds a small model used by the gate tests.

### 2.3 Graph: lineage, traversal, salience, store queries

- `crates/smysl-graph/src/lineage.rs`:
  - `TraceKind { Parents, Grounds, Both, Rests }`, `parse`, `as_str` (`rests-on`).
  - `trace(store, root, kind, depth: Option<u32>) -> Lineage` and `trace_via(store, root,
    &EdgeSet, depth)`: breadth-first, frontier sorted, result sorted by `(depth, uid)`, each
    node with `via` (`Root, Parent, Supersedes, Grounds, Deps, Rests`).
  - `dependents(store, uid)`: reverse closure over `EdgeSet::support()`, no depth.
  - `dependents_via(store, uid, &EdgeSet)` and `rests_on(store, uid, &EdgeSet)`: depth-first,
    "in the order first reached", **no depth bound**. Both handle the direction asymmetry:
    `deps`/`grounds` are stored dependent → premise, relations premise → dependent for
    `conditions, causes, enables, warrant, backs`.
  - `membership(store, roots)`: closure over `EdgeSet::all()`, the members of a view.
  - `diff(a, b) -> StoreDiff { only_in_a, only_in_b, … }` by unit membership.
- `crates/smysl-graph/src/adjacency.rs`: `EdgeKind { Deps, Grounds, Kernel(u8), Extension(u16) }`,
  `EdgeSet::{all, of, support, dependency, premises, ordering, support_rank, one, with, union}`,
  `Adjacency::{out, incoming, out_edges, in_edges, edge_kind(&RelKind)}` (extension kinds are
  interned per store).
- `crates/smysl-graph/src/traverse.rs`: `closure`, `reverse_closure`, `topo`, `cycles`,
  `rebuttals_of(g, node)` (all incoming `rebuts`).
- `crates/smysl-graph/src/salience.rs`: `salience(store, &SalienceRequest) -> SalienceReport`
  with `get`, `explain`, `top`, `renormalise(within)`. `corroboration()` (private) groups
  attestations by `(agent, recipe)` and counts only groups with **disjoint ancestry**
  (`groups`, `dependent_groups` in `SalienceTerms`), capped at `CORROBORATION_CAP = 4`. This is
  the existing independence mechanism the witness hook generalises (§3.7).
- `crates/smysl-graph/src/store/mod.rs` (`Store`):
  - `units()`, `get`, `relations()`, `threads()`, `views()`, `iter()` (all records),
    `adjacency()`, `resolve_prefix`, `matching_prefix`.
  - `units_with_source_prefix(prefix)`: full scan, canonical order.
  - `units_in_observed_order()`: collects and sorts every unit with `source.observed`; units
    without one are left out. `observed_at(uid)`.
  - `relations_of_kind(&RelKind)`: canonical order, withdrawn excluded.
  - `open_contentions()`: contentions whose `contention_status` reads `Open`.
  - `commitment_of(uid)`: latest by `(wall_ms, counter, agent)`; `units_at_commitment(level)`.
  - `rebuttals_of(uid)`: **live** rebuttals only (not withdrawn, rebutter present and not
    `unfounded`). `is_live_rebuttal`, `supports_of(uid, &EdgeSet)` (one hop).
  - `attestations_of`, `attested_by`, `agreement(uid, n)`.
  - There is **no accessor for schema declarations**: they are reachable only through `iter()`
    as `Record::SchemaDecl`, as `smysl-check/src/passes/extension.rs` does.
- `crates/smysl-graph/src/relink.rs`: `relink(store) -> Relinked { records, moved, dangling,
  forked }`. The supersession-head computation is the private `fn successors(store)`; forks
  resolve to nothing. There is no public "head of chain" helper.
- `crates/smysl-graph/src/merge/mod.rs`: `merge(&mut Store, &Store, MergeOptions)`.

### 2.4 Packing

- `crates/smysl-pack/src/solve.rs`: `pack(store, &SalienceReport, &PackRequest) -> Result<Pack,
  PackError>`. `PackRequest::budget(n)`, `.scoped(uids)`, `.focusing(uids)`, `.resting_on(EdgeSet)`,
  `.reserving(n)`, `.capped(Lod)`, `.exact()`. Rule R is applied by the packer through
  `traverse::rebuttals_of` (`crates/smysl-pack/src/closure.rs`; module doc of `traverse.rs`).
- `crates/smysl-pack/src/cost.rs`: `Estimator` (`#[non_exhaustive]`, `ALL`, one variant
  `Utf8Div4`, id `smysl/utf8-div4`, `ceil(len/4) + 2`); `CostModel`, `ExternalCost`.
  FC-6's script-aware estimator is SMYSL-2.1/2.4's; the planner records whichever is used.

### 2.5 Threads and rendering

- `crates/smysl-core/src/types/thread.rs`: `ThreadSchema { Analysis=0, Narrative=1, Brief=2, Qa=3,
  Plan=4 }`; `from_u8` returns `None` for anything else (F-12). `Role` has 24 values `0..=23`
  (`Context … Decision`); `Role::from_u8` indexes `ALL` and returns `None` past it. `Step { role,
  unit, note: Option<String> }`. `Thread { id, schema, owner, gist, steps, ts, extra }`.
  `timeline` is not in the tree (CHANGELOG lists it as carried into 1.9).
- `crates/smysl-thread/src/schema.rs`: `SchemaDef { schema, roles, arity, rules: &[(Matcher,
  Role)], weights }`, `definition(schema) -> &'static SchemaDef`, `Matcher { Type, SourceOf,
  TargetOf, StatusAtLeast, SalienceTop, At(Position), Any }`.
- `crates/smysl-thread/src/derive.rs`: `derive_thread(store, schema, &DeriveOptions) -> (Thread,
  DeriveReport)`; four stages (assign, select, order, repair); `DeriveOptions::scoped(uids)`;
  `ts` is supplied, never read.
- `crates/smysl-render`: `build(store, &Thread, &Profile, &BuildOptions) -> Ir`, `emit`,
  `Target::{Markdown, Typst, Html, Slides, Json, Text}`. No file under `smysl-render/src`
  reads a unit's payload, so "rendering goes through lenses" (draft 3 §15.3) is a constraint on
  future code, not a change to existing code.

### 2.6 The CLI

- `src/main.rs`: `enum Purity { Pure, Mixed, Model }`, `struct Cmd { name, about, purity, phase }`,
  `const COMMANDS: &[Cmd]` with **26** entries. Tests pin it: `command_table_matches_section_23`
  (`COMMANDS.len() == 26` and the name list), `only_ingest_and_attest_are_model_dependent`
  (model = `[ingest, attest]`, mixed = `[thread]`), `every_command_names_the_phase_that_wires_it`
  (`SM-Pnn` or a dotted release). `tests/dispatch.rs` runs all 26; `tests/cli-surface.txt` and
  `tests/public-api*.txt` are snapshots.
- `cmd_find` (`src/main.rs` ≈ line 3710): builds `smysl::Query` from `--kind`, `--min-status`,
  `--source` (→ `units_with_source_prefix` → `within`; an empty match returns early rather than
  searching everything), `--schema`, `--payload KEY=VALUE[,VALUE]` (`payload_filter`),
  `--engine lexical|semantic|hybrid` with `--model` or `SMYSL_EMBED_MODEL` (`engine_for`),
  `ranked()` (rebuilds `Bm25::index` per call, F-10; the semantic path is behind
  `#[cfg(feature = "semantic")]`). `--why` prints per-term contributions on stderr. `find` is
  tagged `Purity::Pure` even though `--engine semantic|hybrid` is model-dependent.
- `cmd_thread` (≈ line 3849): `--derive [S] | --schema S` with a hard-coded `value_parser(["analysis",
  "narrative", "brief", "qa", "plan"])`, `--scope UID` (repeatable), `--arity ROLE=N`,
  `--explain`; `Hlc::new(0, 0, owner)` keeps derivation pure.
- **`--seed-check` is declared as a global flag (line 149) and read nowhere.** Probe:
  `smysl --seed-check find --why -n 3 pool fixtures/corpus/F1-incident.smy` exits 0 and prints
  the ranking with `matched: pool 0.9139` lines; the flag has no effect.
- Rule A (`xtask/src/purity.rs`): nothing under `src/` except `lib.rs` may name a sibling crate;
  every CLI capability must be reachable through the facade `src/lib.rs`.

### 2.7 Schema declarations

- `crates/smysl-core/src/types/annex.rs`: `SchemaDecl { id: SchemaId, version: u32, types:
  Vec<SchemaId>, relations: Vec<RelKind>, payload_shape: Option<Vec<u8>>, extra: Extra }`,
  `redefines_kernel()`.
- `crates/smysl-core/src/cbor/keys.rs`: `schema_decl::{ID=0, VERSION=1, TYPES=2, RELATIONS=3,
  PAYLOAD_SHAPE=4, HIGHEST=4}`. `cbor/envelope.rs`: `schema_decl_bytes` writes `payload_shape`
  as a CBOR **byte string** holding deterministic CBOR; `dec_schema_decl` requires `id` and
  `version` and keeps unknown keys in `extra`. Key 5 (`lenses`) therefore survives today in
  `extra`, opaque.
- `smysl_core::cbor::{Dec, Enc}` are public (`cbor/mod.rs`), so FC-9 decoding needs no new codec.
- Surface: `schema_decl()` in `crates/smysl-core/src/surface/parse.rs` accepts only `version`,
  `types`, `relations`; any other key is `SMY-E001`. `schema_decl_has_surface_form` in
  `surface/write.rs` is false when `payload_shape` or `extra` is present (F-18).
- Payload decoding: `payload_to_object(bytes) -> Result<HObject, CodecError>`; `HValue { Null,
  Bool, Int, Float, Str, Array, Object }` (`surface/hjson.rs`). The lens engine works on this.

### 2.8 Staging, attestations, diagnostics, tests

- `Op { Authored=0, Transformed=1, Imported=2, Attested=3 }` (`types/provenance.rs`).
- `smysl-ingest/src/stage.rs`: `prepare`, `prepare_declared` (with `SchemaDecl`s),
  `prepare_attested`, `prepare_under`; rule M first via `monotone::apply`, which remaps uids
  and rewrites edges; `Staged::to_surface()`. `SMY-W036` reports a rule-M lowering at ingest.
  Re-exported by the facade under feature `stage`.
- Diagnostics: one macro table in `crates/smysl-core/src/diag.rs`; the facade test pins
  `Code::ALL.len() == 57`.
- Property tests use an in-repo seeded xorshift generator (`crates/smysl-graph/tests/merge_algebra.rs`),
  not `proptest` (absent from `Cargo.lock`). `fuzz/` is a cargo-fuzz crate with targets `surface,
  cbor, merge_algebra, pack_constraints, pack_exact, pipeline` and a shared generator library.
- Golden tests regenerate with `SMYSL_BLESS=1` (`smysl-render/tests/golden.rs`) or
  `SMYSL_REGENERATE_GOLDEN=1` (`smysl-pack/tests/golden.rs`): two spellings for one convention.
- `nom 7.1.3` is in `Cargo.lock` (transitively); no parser crate is a direct dependency of a
  pure crate.

## 3. Design

### 3.1 The language: `sq/1` as implemented

Grammar: draft 3 Appendix C. The implementation fixes what the sketch leaves open:

- **Version.** A query without `using sq/N` runs under `sq/1`. `SUPPORTED_SQ = ["sq/1"]`;
  anything else is `SMY-E427`.
- **Comments.** `#` starts a comment only at the start of a token (after whitespace or at line
  start). `text(kjv-1769#Ps.14)` keeps its `#`; `… # scope` is a comment.
- **Multi-word keywords** (`order by`, `group by`, `align by class`, `compare by`, `in reading
  order`, `in time order`, `as recorded`, `as view`) are keywords only at stage-head position.
- **Field names** may contain `:` and `.` (`text:mode`, `payload.code.kind`). `lang:any` after a
  `find` string is the source option, not a field.
- **Time ranges** are lexed in a time context only, so `2026-09-01T00:00:00Z .. 2026-09-15`
  does not collide with arithmetic-looking tokens; EDTF validation is SMYSL-2.4's parser
  (`SMY-E410` on failure).
- **Additions within `sq/1`** (additive, so no version bump, §15.7 of draft 3):
  - `$name` parameters, bound by the caller and recorded in the saved query; used by dossier
    templates. An unbound parameter is `SMY-E460`.
  - `explain` and `why` are CLI flags, not stages.
  - `with lens none` on a stage disables lensing for qualified access only; unqualified access
    is always lensed (§3.9).
- **`match`** (Cypher-like patterns) is parsed into an AST node and refused by the planner with
  `SMY-E461` until a later phase (OQ-50). Every other stage in Appendix C is planned.

**Parser approach: hand-written recursive descent with a small mode-aware lexer.** Reasons:
1. The tree's own parsers are hand-written (`surface/lex.rs`, `surface/parse.rs`, 2,243 lines)
   with `Code`-tagged, span-carrying diagnostics. `sq` errors should look the same.
2. A grammar implemented as plain functions ports to the Python, JavaScript and Go
   implementations line by line. A combinator or PEG crate does not, and saved queries are
   meant to be readable by another implementation.
3. Contextual lexing (`#`, `lang:any`, time ranges, multi-word keywords) is awkward in a
   generated lexer and trivial in a hand-written one.
4. The purity gate (`FORBIDDEN_DEPS` in `xtask/src/purity.rs`) would allow `nom` or `winnow`,
   but each dependency of a pure crate is a cost the project has argued against case by case
   (`bm25` with `default-features = false`). The grammar is about 40 productions; a parser crate
   does not pay for itself.

### 3.2 Planner

The planner lowers the AST in a fixed order. Each phase has one job and records what it did, so
`--explain` prints the plan rather than reconstructing it.

1. **Header.** Resolve `using` pins: `sq/N` (else `E427`), `schema-id@revision` against the
   `SchemaSet` (§3.8). Unknown revision: `SMY-E466`.
2. **Field resolution.** Every field and relation name is resolved against the declarations of
   the schemas in scope (§3.9). Undeclared field: `SMY-E421`. Undeclared relation kind in a
   traversal: `SMY-E462`. A pinned query is compiled forward here.
3. **Scope.** `from` resolves to a set of stores and a candidate uid set, through the `Scope`
   trait (§4.2). Plain `Store`: `store:` and the whole store. `smysl-text` (SMYSL-2.4) adds
   `texts`, `library`, `collection:`, `channel`, `import:` and catalog predicates.
4. **Partitions.** The candidate set is split by `(manifest, lang)` when the catalog provides
   them, else one partition. IDF scope (partition, collection, library; default collection,
   OQ-14) is recorded.
5. **Sources.** Each source stage yields a ranked or unranked set per partition.
6. **Filters and time.** Predicates evaluate over **lensed** unit views (§3.9). Time predicates
   use SMYSL-2.4's effective-time engine; undated units are counted (`SMY-W415`).
7. **Traversals** (`expand`) and **closures** (`with`).
8. **Set operations** on sub-plans.
9. **Ordering and grouping**, then **shape** (`limit`, `pack`), then **output**.

A **supersession pass** runs after sources and traversals: units with a single newest successor
are replaced by it, as relink resolves chains; a fork keeps every successor and raises
`SMY-W464`. `--all-versions` turns the pass off.

**Purity labelling.** Every plan step carries `Purity::Pure` or `ModelDependent(reason)`. The
plan is model-dependent if any step is: `~sem`, `~hybrid` that routes semantic, `near`, a
semantic bridge, a model intent classifier, a dossier template that uses one, or the NL front
end (the translation, not the run). `--seed-check` on a model-dependent plan is `SMY-E416`
before anything executes. On a pure plan it executes twice — the second time with freshly built
indexes and scratch buffers — and compares result hashes; a mismatch is an internal error.

### 3.3 Operator-to-API mapping

| `sq` stage | existing function (file) | new code |
|---|---|---|
| `from store:P [, store:Q]` | `Store::open`; `merge(&mut scratch, &other, MergeOptions)` (`smysl-graph/src/merge/mod.rs`) | union into a scratch store until SMYSL-2.8's virtual union; contentions between stores computed by merge's detection |
| `from texts / library / collection: / channel / import:` | — | `Scope` impl in `smysl-text` (SMYSL-2.4) |
| `find "…" ~lex` | `Bm25::index_with`, `Retriever::search`, `Query::{kinds,min_status,within,schemas}` | `Bm25::index_subset(store, uids, tokenizer)` for per-partition IDF; RRF (§3.4) |
| `find "…" ~sem` | `Semantic::index`, `search` (`smysl-embed`) | `SemanticSource` port; S1 via `smysl-embed-candle` |
| `find "…" ~hybrid` | `Hybrid::routes_to_lexical`, `looks_like_identifier` | routing decision recorded in the plan |
| `about entity("…")` | `Adjacency::edge_kind(&RelKind)` for `x.text/mentions` | entity resolution through aliases and same-as (SMYSL-2.4) |
| `about class(<uid>)` | — | class membership (SMYSL-2.4, TX-P7) |
| `text(alias#loc)`, `text(alias @ range)` | — | locator and part lookup (SMYSL-2.4) |
| `uid b3:…` | `Store::resolve_prefix`, `resolve_label` | — |
| `view v/…` | `Store::views()`, `membership(store, roots)` | — |
| `pointer("…")`, `metric("…")` | `units_with_source_prefix` | pointer lenses (§3.12) |
| `question "…"` | — | dossier (§3.13) |
| `where kind / schema / status` | `Query::admits_schema`, `Status: Ord` | typed predicate evaluator |
| `where lang` | — | core key 9 (FC-1, SMYSL-2.4) |
| `where holder / speaker / mode / <payload key>` | `payload_to_object` | lensed views (§3.9) |
| `where commitment` | `Store::commitment_of` | — |
| `where matches "…"` | `Tokenizer::terms`, `Bm25` contributions | term presence test |
| `where near("…", t)` | `Semantic` cosine | model-dependent |
| `said/observed … during/overlaps/before/after` | `Store::observed_at` (observed axis only) | effective time and range index (SMYSL-2.4) |
| `expand ->grounds`, `->deps`, `->k` | `trace_via(store, root, &EdgeSet::one(k), depth)` | — |
| `expand <-k` | `dependents_via` (no depth) | `dependents_trace(store, root, &EdgeSet, depth) -> Lineage` in `smysl-graph`, BFS mirror of `trace_via`, sorted `(depth, uid)` |
| `expand -[k]-` | `Adjacency::out` + `incoming` | both-direction BFS, same output order |
| `expand rests_on / dependents / lineage` | `rests_on(…, EdgeSet::premises())`, `dependents_via(…, EdgeSet::dependency())`, `trace(…, TraceKind::Both, depth)` | depth for the first two via the new BFS |
| `expand mentions` | extension edge via `Adjacency::edge_kind` | — |
| `with rebuttals` | `Store::rebuttals_of` (live only) | — |
| `with grounds` | `UnitCore.grounds`; `trace_via` depth 1 | — |
| `with holders / speakers` | payload uid keys through lenses | — |
| `with spans / datings` | — | SMYSL-2.4 |
| `union / intersect / except` | `BTreeSet` operations | — |
| `order by salience` | `salience(store, SalienceRequest::default().seeded(result))`, `renormalise(result)` | — |
| `order by <field>` | — | typed comparator, ties by uid |
| `in reading order / in time order` | `units_in_observed_order` (observed axis) | span order and effective time (SMYSL-2.4) |
| `group by`, `compare by` | — | new |
| `align by class [policy]` | — | class matrix (SMYSL-2.4 classes) |
| `limit n` | `Vec::truncate` | — |
| `pack N tokens [as view]` | `pack(store, &sal, &PackRequest::budget(N).scoped(result).focusing(result))` | emit a `View` record for `as view` |
| `count / units / json / show` | `smysl::json_escape` pattern in `cmd_find` | output writers |
| `spans` | — | SMYSL-2.4 |
| `measure <ids>` | — | dispatch to SMYSL-2.6 |
| `match` | — | refused (`E461`) in this RFC |

Two small `smysl-graph` additions follow from the table: `dependents_trace` (depth-bounded
reverse BFS) and `pub fn supersession_heads(store) -> (BTreeMap<Uid, Uid>, Vec<Uid>)`, which is
the existing private `relink::successors` made public so query and relink cannot disagree.

### 3.4 Partitions and fusion

- One BM25 index per partition, built with `Bm25::index_subset` so IDF is the partition's
  (or the collection's, by IDF scope). Until SMYSL-2.4's persistent postings, the planner caches
  per-partition indexes for the duration of one invocation only.
- Each partition is searched with its own analyzer (SMYSL-2.4 chain id, recorded).
- **RRF.** `score(u) = Σ_p 1 / (k + rank_p(u))`, `k = 60`, ranks 1-based, partitions summed in
  sorted partition-key order, f64, ties by uid. The fused list is a function of integer ranks
  and a fixed summation order, so it is bit-reproducible. `k` is recorded in the recipe (OQ-45).
- **What RRF never does:** fuse lexical with semantic. `~hybrid` keeps the measured routing of
  `Hybrid::routes_to_lexical`; the routing outcome is recorded per partition. A future merge
  must be argued against the 0.78 vs 0.84 MRR number that removed it (OQ-46).
- `--why` on a fused result prints, per hit, the partition ranks and the per-term contributions
  inside each partition (from `Hit::terms`).

### 3.5 Language bridges

`lang:any` asks the planner for the strongest bridge per pair of partitions:

| bridge | source | purity | phase |
|---|---|---|---|
| structural alignment | SMYSL-2.4 `align` table | pure | TX-P6 when the table exists |
| proposition classes | SMYSL-2.4 classes (`strict` normative, D-7) | pure | after TX-P7 |
| entity aliases | `x.text/entity` + `x.text/same-as` | pure | TX-P6 |
| multilingual embeddings | S0 / S1 (`SemanticSource`) | model-dependent | TX-P10 |
| query-term translation | `Translator` port (§3.15) | model-dependent | TX-P10 |

The bridge used is a column of the result and a line of `--explain`. A partition reachable by no
bridge answers in its own language only and is counted (`SMY-W465`).

### 3.6 Saved queries (`x.query/v1`)

A saved query is one unit of type `x.query/saved` (FC-5, SMYSL-2.3) with payload keys from draft
3 Appendix A:

| key | content |
|---|---|
| `query:text` | the query as written, after parameter binding |
| `query:recipe` | recipe hash: `sq` version, analyzer ids, IDF scope, estimator id, RRF `k`, class policy, template-set id, parameters |
| `query:models` | model hashes used (empty for pure plans) |
| `query:result-hash` | BLAKE3 over `"smysl/sq-result/1\0"` ‖ sorted result uids (tool-level, not format) |
| `query:sq` | `sq/1` |
| `query:schemas` | `id@revision` list in force |
| `query:lens-chain` | hash of every lens chain the plan applied (§3.10) |

Status `cited`, `source { kind: doc, ref: "sq:<recipe-hash-short>" }`: the unit cites the query
text it carries. That matches how `@question` units are authored in `fixtures/corpus/F4-qa.smy`
(`status: cited` with a `doc` source).

**Results.** Draft 3 links results with kernel `answers` edges from the saved query. That edge
kind carries salience rank (`RelKind::carries_support` returns true for `Causes | Answers`), so
saving a query would raise its results' salience and change every later `order by salience`
— including the rerun of the query that was saved. This RFC therefore:
- writes the result set as edges of one relation-kind constant, `RESULT_EDGE`;
- uses `x.query/answers` (an extension kind, which `EdgeKind::carries_support` treats as
  non-supporting) as its value, as SMYSL-2.3 A-11 decides (OQ-49, resolved);
- refuses `--save` with kernel `answers` unless salience excludes edges whose source is an
  `x.query/*` unit, which is a change to `smysl-graph/src/salience.rs` this RFC does not make.

**Reruns.** Let `P` be the previous result (from the result edges), `E_pin` the rerun pinned to
the recorded revisions over today's data, `E_cur` the same query under current revisions:
- `q --rerun <uid>` (default, pinned): returns `E_pin`; **data effect** = `diff(P, E_pin)`.
- `q --rerun <uid> --current`: returns `E_cur`; data effect = `diff(P, E_pin)`, **schema effect**
  = `diff(E_pin, E_cur)`. Both use only revisions recorded in the unit, never dates.
- `--current --save` writes a new `x.query/saved` that `supersedes` the old one.
- A model-dependent saved query rerun under a different model hash raises `SMY-W467`; its diffs
  are printed and labelled not comparable.
- A recorded lens-chain hash that the store's declarations no longer reproduce (a declaration was
  never merged here) is `SMY-E466`: the query cannot be pinned, and it does not answer from part
  of its meaning.

### 3.7 Independent witnesses

`support` and dossier ranking count independent witnesses through a trait:

```rust
pub trait Witnesses {
    /// Groups of sources that independently support `uid`, and groups dropped as dependent.
    fn groups(&self, store: &Store, uid: &Uid) -> WitnessGroups;
}
```

- **Default implementation:** attestation groups with disjoint ancestry, i.e. today's
  `salience::corroboration` logic, made public as `smysl_graph::salience::corroboration_groups`
  so there is one definition.
- **Library implementation** (SMYSL-2.4): sources are manifests; two are dependent when their
  derivation graphs (draft 3 §4.7) meet. Partial dependence is OQ-15.
- When a unit's sources are not all known to the derivation graph, the count falls back to
  attestation groups for that unit and says so (`SMY-W479`).

This is the 1.9 "corroboration" carry-over (CHANGELOG, Unreleased) served by one mechanism.

### 3.8 FC-9 decoding (in `smysl-core`)

FC-9's byte layout is normative in SMYSL-2.3. Decoding it into types lives in `smysl-core` so
that `check`, retrieval, rendering and `smysl-query` share one reading:

- `payload_shape` bytes decode as `{0: fields}`; each field `{0: type, 1: values, 2: retired-in,
  3: note, 4: required-from, 5: required-when}`. A shape that does not match is kept opaque and
  reported `SMY-W430` — exactly today's behaviour plus the warning.
- Key 5 moves from `extra` into a typed `lenses: Vec<LensStep>`. The step's **original bytes are
  kept**, so re-encoding is byte-identical and the chain hash is over exactly what was received.
- An op tag this build does not know decodes to `LensOp::Unknown { tag, bytes }`. Per D-5, the
  step is unusable (`SMY-W469`) and units needing it are unmappable (`SMY-W420`).
- `SchemaSet::from_records(store.iter())` collects every revision of every id. A new
  `Store::schema_decls()` accessor avoids each caller re-filtering `iter()`.
- **Current revision** is the highest version. Two declarations of one `(id, version)` with
  different bytes are a fork: `SMY-W429`, and that id has **no** current revision until resolved.
- The `@schema` surface form for `fields:` and `lenses:` (spelling normative in SMYSL-2.3) is
  implemented in `surface/parse.rs` and `surface/write.rs` in TX-P12;
  `schema_decl_has_surface_form` becomes "no `extra` keys this build cannot spell".

### 3.9 The lens engine and field resolution

**Unit views.** A lensed view of a unit is `UnitView { ty: SchemaId, payload: BTreeMap<String,
HValue>, edges renamed lazily }` built from `payload_to_object`. Lenses never write: the unit's
bytes, uid and attestations are untouched.

**Revision inference** (draft 3 §15.2): a unit's revision interval is narrowed by (a) retired
names it uses, (b) keys required from revision n (with their condition) that it lacks, (c) for
extracted units, the revisions named in its attestation recipe. The engine lenses from the
**highest** revision consistent with the clues. By rule N, any consistent revision yields the
same view; that is a tested law (§5.3), not an assumption.

**Application, forward** (one step, ops in declared order):

| op | forward effect on a view | backward |
|---|---|---|
| `rename-type a → b` | `ty = b` if `ty = a` | `b → a` |
| `rename-relation r → s` | edge kind `r` followed as `s` | `s → r` |
| `rename-key k → k'` | move `k` to `k'` | move back |
| `map-value k: {old → new}` | rename value | inverse map; non-injective map is not invertible |
| `split-value k = v → {k: v', k2: x}` | `k = v` becomes `k = v'` plus `k2 = x` | `k = v'` and `k2 = x` → `k = v`, drop `k2` |
| `require k [when c]` | none (validation only) | none |
| `default k = v [when c]` | absent `k` under `c` reads as `v` | drop `k` when it equals `v`; other values left for an earlier op |
| `retire k` | `k` reads as absent | not invertible: `SMY-E422` on any backward path |

Backward application runs steps in reverse and ops in reverse. After it, any key or value no op
took back makes the unit unmappable (`SMY-W420`), counted.

**Unmappable values.** A value the chain cannot carry (an enum value outside every `map-value`
and the target revision's `values`) leaves that field unmappable on that unit: the unit is
excluded from predicates on that field and counted; other fields still evaluate. `--explain`
prints counts per lens step: "312 units read through `x.text/v1` 1→2; 0 unmappable".

**Widening detection.** A `split-value` whose `v'` already exists in the source revision's
`values` is a widening. The engine requires a `require k2 when k = v'` and a `default k2 = d when
k = v'` in the same step; their absence is a lens defect (`SMY-E471`), which `schema check` also
reports as a rule N violation (`SMY-E426`).

**Field resolution** (draft 3 §15.4), implemented in `smysl-query`:
- Unqualified names resolve against the current revision of each schema in scope. If two schemas
  in scope both declare the name, the field is ambiguous and must be qualified (`SMY-E421` with
  the candidates listed).
- Qualified `x.text/v1@1:text:mode` reads revision 1 raw, no lens.
- **Pinned compile-forward.** For `using x.text/v1@1`, every predicate is rewritten through the
  chain 1 → current:
  - renames rename the field or value;
  - `k = v` under a `split-value k = v → {k: v', k2: x}` becomes `k = v' and k2 = x`;
  - `k = v'` where that split is a **widening** becomes `k = v' and k2 = d`, `d` from the paired
    `default` — the old, narrow meaning;
  - a predicate on a retired key, or a retired value with no mapping, is `SMY-E422`.
  Lenses never run backwards for a query.

### 3.10 Chains, composition and hashing

- `SchemaSet::chain(id, from, to)` finds the step path (steps are `from → to` with `to > from`;
  gaps or cycles: `SMY-E470`).
- `LensChain::compose(a, b)` concatenates steps when `a.to == b.from`.
- **Chain hash:** BLAKE3 over `"smysl/lens-chain/1\0"` ‖ deterministic CBOR array
  `[id, from, to, [step bytes…]]`, where step bytes are the received record-8 key-5 elements.
  Hashing received bytes rather than a re-encoding makes the hash identical across
  implementations that agree on the record. This is the definition SMYSL-2.3 Appendix B adopts.
- Composition is associative on application and on the hash (both are concatenation); tested.

### 3.11 `schema show`, `schema check`, `migrate`

`smysl schema show <id>`: revisions, field table per revision (type, values, required-from/when,
retired-in), lens steps with op lists, chain hashes from each revision to current, forks.

`smysl schema check <id>` (pure; draft 3 §15.2–15.3), every revision against **every** earlier one:
1. No name (type, relation, key, enum value) changes meaning except by a declared widening with
   its paired key (`SMY-E426`).
2. A retired name never reappears in a later revision's names or in any lens target (`SMY-E426`).
3. Forks (`SMY-W429`).
4. Each step well-formed: `default` paired with `require` under the same condition, ops naming
   declared keys (`SMY-E471`); unknown ops (`SMY-W469`).
5. Totality over the store: every unit of the schema is lensed to current; unmappable units are
   listed (`SMY-W420`).
6. Extracted units missing a key their recipe's revision requires (`SMY-W431`).

`smysl migrate --schema <id> --to <rev> [--recipe R]`:
- **Deterministic** (no recipe): for each unit at an older revision, apply the chain forward,
  rebuild a `UnitCore` with the lensed payload and type, attest `op: Transformed` naming the
  chain hash in the recipe, and emit `supersedes` new → old. Units go through
  `stage::prepare_declared` with the target `SchemaDecl`, so rule M (`monotone::apply`) caps
  each new unit at its source's status and the batch is staged (rule S).
- If the target revision requires a key the chain cannot supply (no `default` covers it), the
  migration needs information the units lack: `SMY-E473`, unless `--recipe` names an extraction,
  which runs through `ingest` and its rules (T, M, S) and makes the run model-dependent.
- References are re-pointed by the existing `relink` after the staged batch is accepted.
- Commitments and datings are **proposed** for the new uids, attributed to the migrating agent,
  staged for review, not transferred (`SMY-W472` with counts).
- Never automatic, never in place. Old units remain; the supersession pass (§3.2) prefers heads.

### 3.12 Pointer lenses (`json/1`)

The `json/1` reader profile (SMYSL-2.4) declares pointer lenses with the same op set, where a
"key" is a JSON Pointer and a version marker selects the step (a manifest field or a shape
fingerprint). `pointer("/database/pool/max_size")` compiles to the set of pointers that map to it
across versions, and matches units by their locator through `units_with_source_prefix` (today)
or SMYSL-2.4's `locators` table (later). Locators in units are never rewritten. A snapshot with
no version marker is read without a lens and counted (`SMY-W475`). Unit conversions stay out
(OQ-23).

### 3.13 Dossiers

**Pipeline** (draft 3 §12.1):
1. **Subject.** Resolve X to `x.text/entity` units and classes through aliases and same-as
   (SMYSL-2.4). More than one candidate: list them, `SMY-W417`, stop. `--subject <uid>` picks.
2. **Intent.** A closed rule table per tier-1 language (`define, explain, verify, attribute,
   trace, locate, depend, compare`): ordered regular patterns over the folded question ("what
   is", "что такое", "qu'est-ce que", "was ist", "qué es", …). No rule matches: `SMY-E468`,
   `--intent` overrides. A model classifier is a `Classifier` port, labelled model-dependent
   (OQ-22). Pure by default.
3. **Slots.** Each intent is a list of slot templates, each an `sq` file with `$subject`,
   `$scope`, `$lang`, compiled into the binary (`include_str!`) under a template-set id
   `smysl/dossier/1` whose hash goes into the recipe. Unknown set id on rerun: `SMY-E477`.
   Example, `define/counter.sq`:
   ```
   about entity($subject) | where kind in (claim, finding)
   | expand <-rebuts depth 1 | union (about entity($subject) | with rebuttals)
   | where status >= inferred | order by salience desc | limit 12
   ```
4. **Rank within a slot** by the product of retrieval score (RRF-normalised), derived salience
   (`renormalise` over the slot), status rank and independent-witness count, each quantised
   with `smysl_core::quantise`; ties by uid. Multiplying incomparable scales is a choice to
   test against GE-T10 (OQ-52).
5. **Pack** with `PackRequest::budget(N).scoped(members).focusing(slot units)`; rule R pins live
   rebuttals.

**Representation** (draft 3 §12.3):
- Root: a `question` unit with the question text (`status: cited`, `source { kind: doc, ref:
  "ask:<recipe-hash-short>" }`).
- One `x.query/saved` unit per slot, with its template text and parameters, linked to its
  results by `RESULT_EDGE` (§3.6).
- A `Thread` of schema `dossier` (FC-8 code and role codes from SMYSL-2.3), one `Step` per filled
  unit with the slot's role, in template slot order. **An empty slot is a step with role `gap`
  whose unit is the slot's saved query** and whose `note` names the slot: the absence is a
  visible, rerunnable finding. `evidence` and `support` reuse today's `Role::Evidence` (17) and
  `Role::Support` (13), and `question` reuses 16; the other slot roles are codes 24–48
  (SMYSL-2.3 A-8.2).
- A `View` with `roots = {question, slot queries}`, `threads = {dossier thread}`, `requires =
  {x.query/v1, x.text/v1}`, `intent = "dossier"`. `membership` reaches results through the
  result edges.
- `smysl_thread::definition(ThreadSchema::Dossier)` returns a `SchemaDef` with the full role
  list, weights, and **no matcher rules**: `thread --derive dossier` is refused, because dossier
  threads are produced only by `ask`. `exposition` and `dialogue` derivation (pure, from spans,
  reply edges and effective time) need SMYSL-2.4 data and are added to `smysl-thread` in TX-P8.
- Rendering uses `smysl-render::build` on the dossier thread; render profiles gain headings for
  the new roles. A profile without them falls back to the role name (an unknown role after
  SMYSL-2.1 opens `Role` is `SMY-W409`).

**Answer step** (`ask --answer`, model-dependent):
- Input: the packed dossier rendered with `Target::Text`, delimited as data with the
  prompt-injection guard of SMYSL-2.1 (D-10): instructions inside the text are not followed.
- Output requested as surface units; every unit must ground only on dossier members. A unit
  grounding elsewhere is dropped and counted (`SMY-W478`).
- Accepted units go through `stage::prepare`, so rule M caps each at its weakest ground
  (`SMY-W036` when lowered). The answer is "AI reads AI and passes it to humans" with the hedges
  intact.

### 3.14 Embeddings: S0 and S1

**S0** (`smysl-embed`): pin `model2vec-rs 0.3` with `default-features = false, features =
["wasm"]` (verified, §2.2). Update the manifest comment: no `esaxx_fast`, no C++ from this path,
`clap` optional, `serde_json` still required (so the crate stays outside `PURE_CRATES`). Default
multilingual model `potion-multilingual-128M` (256-d), loaded through the existing `from_dir`
(unverified until a model directory is tried).

**S1** (`smysl-embed-candle`, new, optional, feature `semantic-candle`):
- Encoders: `multilingual-e5-small`/`-base` (XLM-R family), `bge-m3` (dense head). Loaded from a
  local directory: `config.json`, `tokenizer.json`, `model.safetensors`; no network, as S0.
- Pooling: mean pooling over the attention mask for e5 (with the `query: `/`passage: ` prefixes
  the model card prescribes), CLS for bge-m3 dense; L2-normalised. The pooling id is part of the
  model hash.
- **Model hash:** BLAKE3 over the three files' bytes and the pooling id. **Vector cache:** keyed by
  `(model hash, uid)`; until SMYSL-2.4's `vectors` table, a sidecar `<store>.vec-<hash12>`
  (CBOR seq of `(uid, f32[])`). An entry under another model hash is ignored and re-embedded
  (`SMY-W476`). Embeddings are a cache, never identity.
- Purity: model-dependent; no cross-machine bit-agreement claimed (as for S0).
- Dependencies: `candle-core`, `candle-nn`, `candle-transformers`, `tokenizers` (shared with S0).
  Versions, the XLM-R implementation in `candle-transformers`, and the absence of `cc`/`-sys`
  crates with default features are **unverified** here (not in the offline registry); TX-P10
  step 1 verifies them before any code is written.

**Semantic sources and bridges** use a port so `smysl-query` stays pure:

```rust
pub trait SemanticSource {
    fn model_hash(&self) -> [u8; 32];
    fn search(&self, query: &str, within: &BTreeSet<Uid>, limit: usize) -> Vec<Hit>;
    fn similarity(&self, a: &str, b: &str) -> f32; // for near(…)
}
```

S0's `Semantic` and S1's encoder implement it in their own crates; the facade wires them under
features `semantic` and `semantic-candle`.

### 3.15 Natural-language front end

```rust
pub trait Translator {
    fn model_hash(&self) -> [u8; 32];
    fn translate(&self, question: &str, ctx: &TranslateContext) -> Result<String, TranslateError>;
}
```

- `smysl-query` defines the port and `TranslateContext` (declared fields and enum values from
  the `SchemaSet`, stage list, few-shot examples). No provider is linked.
- The facade implements it over `smysl-provider` under feature `providers` (rule A: the CLI
  reaches it only through the facade).
- The model's output is untrusted. It must parse as `sq/1`, pass field resolution, and use only
  allowed stages; otherwise `SMY-E474` and nothing runs. The translated query is printed before
  it runs; `q --from-question "…"` runs it only with `--yes` or interactive confirmation.
- The run itself is labelled by the query's own purity: the model wrote the query; the query,
  not the model, produces the answer. The saved query records the translator's model hash in
  `query:models` and the original question in the unit's body.

## 4. Implementation plan

### 4.1 Crates and modules

**New `crates/smysl-query`** (pure; joins `PURE_CRATES`):

| module | contents |
|---|---|
| `src/lib.rs` | re-exports, `SUPPORTED_SQ`, `Purity` |
| `src/lex.rs` | mode-aware lexer, spans |
| `src/parse.rs` | recursive descent per Appendix C + §3.1 additions |
| `src/ast.rs` | `Query`, `Stage`, `Pred`, `Source`, `Expand`, … |
| `src/resolve.rs` | field and relation resolution, `using` pins, compile-forward |
| `src/plan.rs` | planner phases, `Plan`, `PlanStep`, purity |
| `src/exec.rs` | executor over `Env` |
| `src/fuse.rs` | RRF |
| `src/bridge.rs` | bridge selection |
| `src/witness.rs` | `Witnesses` trait, attestation default |
| `src/explain.rs` | `--explain` and `--why` text and JSON |
| `src/saved.rs` | `x.query/v1` encode/decode, rerun, effect split |
| `src/dossier/{mod,intent,templates}.rs` + `templates/*.sq` | dossier pipeline |
| `src/nl.rs` | `Translator`, `TranslateContext`, output validation |
| `src/ports.rs` | `Scope`, `SemanticSource`, `Classifier`, `TimeIndex` |

**New `crates/smysl-embed-candle`** (impure tier, optional): `src/{lib,encoder,pool,cache}.rs`.

**Changed:** `smysl-core` (new `src/schema/` module, `cbor/keys.rs`, `cbor/envelope.rs`,
`surface/parse.rs`, `surface/write.rs`, `diag.rs`), `smysl-graph`, `smysl-retrieve`,
`smysl-embed`, `smysl-thread`, `smysl-render` (profile headings), facade `src/lib.rs`,
`src/main.rs`, `xtask/src/purity.rs`, `fuzz/`.

### 4.2 Public API (sketch)

`smysl-core`, `src/schema/mod.rs`:

```rust
#[non_exhaustive]
pub enum FieldType { Text, Int, Bool, Uid, Edtf, Enum, List(Box<FieldType>), Other(String) }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cond { pub key: String, pub value: HValue }

#[non_exhaustive]
pub struct Field {
    pub ty: FieldType,
    pub values: Vec<String>,
    pub retired_in: Option<u32>,
    pub note: Option<String>,
    pub required_from: Option<u32>,
    pub required_when: Option<Cond>,
}

#[non_exhaustive]
pub enum Fields { Typed(BTreeMap<String, Field>), Opaque /* SMY-W430 */, Absent }

#[non_exhaustive]
pub enum LensOp {
    RenameType { from: SchemaId, to: SchemaId },
    RenameRelation { from: RelKind, to: RelKind },
    RenameKey { from: String, to: String },
    MapValue { key: String, map: BTreeMap<String, String> },
    SplitValue { key: String, from: String, to: String, add: BTreeMap<String, HValue> },
    Require { key: String, when: Option<Cond> },
    Default { key: String, value: HValue, when: Option<Cond> },
    Retire { key: String },
    Unknown { tag: String, bytes: Vec<u8> },
}

pub struct LensStep { pub from: u32, pub to: u32, pub ops: Vec<LensOp>, bytes: Vec<u8> }

pub struct Revision {
    pub id: SchemaId, pub version: u32, pub decl_hash: [u8; 32],
    pub types: Vec<SchemaId>, pub relations: Vec<RelKind>,
    pub fields: Fields, pub lenses: Vec<LensStep>,
}

pub fn decode(decl: &SchemaDecl, report: &mut Report) -> Revision;
pub fn encode_lenses(steps: &[LensStep]) -> Vec<u8>; // byte-identical for decoded steps

pub struct SchemaSet { /* id -> revisions, forks */ }
impl SchemaSet {
    pub fn from_records<'a>(records: impl IntoIterator<Item = &'a Record>, report: &mut Report) -> SchemaSet;
    pub fn current(&self, id: &SchemaId) -> Option<&Revision>;   // None while forked
    pub fn revisions(&self, id: &SchemaId) -> &[Revision];
    pub fn forks(&self) -> Vec<(SchemaId, u32)>;
    pub fn chain(&self, id: &SchemaId, from: u32, to: u32) -> Result<LensChain, LensError>;
    pub fn infer_revision(&self, unit: &UnitCore) -> RevisionGuess;
}

pub struct LensChain { pub id: SchemaId, pub from: u32, pub to: u32, steps: Vec<LensStep>, pub hash: [u8; 32] }
impl LensChain {
    pub fn identity(id: SchemaId, rev: u32) -> LensChain;
    pub fn compose(self, next: LensChain) -> Result<LensChain, LensError>;
    pub fn is_invertible(&self) -> bool;
    pub fn forward(&self, view: &mut UnitView) -> Mapped;
    pub fn backward(&self, view: &mut UnitView) -> Result<Mapped, LensError>; // Retire -> E422
    pub fn widenings(&self) -> Vec<Widening>;
}

pub struct UnitView { pub ty: SchemaId, pub payload: BTreeMap<String, HValue> }
impl UnitView { pub fn of(core: &UnitCore) -> UnitView; }

#[non_exhaustive]
pub enum Mapped { Whole, Partial { unmappable: BTreeSet<String> } }
```

`smysl-graph` additions:

```rust
impl Store { pub fn schema_decls(&self) -> impl Iterator<Item = &SchemaDecl>; }
pub fn dependents_trace(store: &Store, root: Uid, edges: &EdgeSet, depth: Option<u32>) -> Lineage;
pub fn supersession_heads(store: &Store) -> (BTreeMap<Uid, Uid>, Vec<Uid>); // relink::successors, public
pub mod salience { pub fn corroboration_groups(store: &Store, uid: &Uid) -> (Vec<String>, Vec<String>); }
```

`smysl-retrieve` additions:

```rust
impl Bm25 {
    /// An index over `uids` only, so IDF is theirs (per-partition IDF, §3.4).
    pub fn index_subset(store: &Store, uids: &BTreeSet<Uid>, tokenizer: Tokenizer) -> Bm25;
}
```

`smysl-query`:

```rust
pub fn parse(text: &str) -> Result<ast::Query, Vec<Diagnostic>>;                  // E460
pub fn plan(q: &ast::Query, env: &Env<'_>, opts: &PlanOptions) -> Result<Plan, Vec<Diagnostic>>;
pub fn run(plan: &Plan, env: &Env<'_>) -> Result<Outcome, QueryError>;

pub struct Env<'a> {
    pub store: &'a Store,
    pub schemas: &'a SchemaSet,
    pub scope: &'a dyn Scope,
    pub time: Option<&'a dyn TimeIndex>,
    pub semantic: Option<&'a dyn SemanticSource>,
    pub witnesses: &'a dyn Witnesses,
}

#[non_exhaustive]
pub struct PlanOptions { pub params: BTreeMap<String, String>, pub all_versions: bool,
                         pub idf_scope: IdfScope, pub rrf_k: u32, pub current: bool }

#[non_exhaustive]
pub enum Purity { Pure, ModelDependent(Vec<&'static str>) }

#[non_exhaustive]
pub struct Plan { pub sq: SqVersion, pub purity: Purity, pub steps: Vec<PlanStep>,
                  pub pins: Vec<(SchemaId, u32)>, pub lenses: Vec<LensUse>,
                  pub bridges: Vec<BridgeUse>, pub recipe_hash: [u8; 32] }

#[non_exhaustive]
pub struct Outcome { pub rows: Vec<Row>, pub uids: Vec<Uid>, pub result_hash: [u8; 32],
                     pub counts: Counts /* undated, unmappable, truncated, forks */,
                     pub report: Report }

pub fn explain(plan: &Plan, outcome: Option<&Outcome>) -> String;
pub fn why(outcome: &Outcome, uid: &Uid) -> Vec<Contribution>;

pub mod saved {
    pub fn to_records(plan: &Plan, out: &Outcome, agent: &AgentId, ts: Hlc) -> (UnitCore, Vec<Relation>);
    pub fn load(store: &Store, uid: &Uid) -> Result<SavedQuery, QueryError>;
    pub fn rerun(saved: &SavedQuery, env: &Env<'_>, current: bool) -> Result<Rerun, QueryError>;
    pub struct Rerun { pub result: Outcome, pub data_effect: SetDiff, pub schema_effect: Option<SetDiff> }
}

pub mod dossier {
    #[non_exhaustive] pub enum Intent { Define, Explain, Verify, Attribute, Trace, Locate, Depend, Compare }
    pub fn classify(question: &str, lang: &str) -> Result<(Intent, String /* subject */), Diagnostic>;
    pub fn ask(question: &str, env: &Env<'_>, opts: &AskOptions) -> Result<Dossier, QueryError>;
    pub struct Dossier { pub records: Vec<Record>, pub view: View, pub thread: Thread,
                         pub slots: Vec<SlotResult>, pub purity: Purity }
}
```

`smysl-embed-candle`:

```rust
pub struct Encoder { /* candle model, tokenizer, pooling */ }
impl Encoder {
    pub fn from_dir(path: impl AsRef<Path>, pooling: Pooling) -> Result<Encoder, Error>;
    pub fn model_hash(&self) -> [u8; 32];
    pub fn embed(&self, texts: &[String], role: TextRole /* Query | Passage */) -> Vec<Vec<f32>>;
}
pub struct VectorCache { /* (model hash, uid) -> Vec<f32> */ }
pub struct CandleSemantic { /* implements smysl_query::SemanticSource and smysl_retrieve::Retriever */ }
```

Facade `src/lib.rs`: `pub mod query { pub use smysl_query::*; }`, `pub mod schema` re-exporting
`smysl_core::schema::*` plus `smysl_query::resolve`, and feature-gated adapters
(`semantic`, `semantic-candle`, `providers`) for `SemanticSource` and `Translator`. This matches
draft 3 §17's facade names (`smysl::query::{Query, Scope, Plan, Purity, Dossier, Intent}`,
`smysl::schema::{Revision, Field, FieldType, Lens, LensOp, LensChain, resolve}`; `Lens` is
`LensStep` here).

### 4.3 Changes to existing crates, file by file

| file | change |
|---|---|
| `crates/smysl-core/src/schema/{mod,decode,lens,infer}.rs` | new: §4.2 types, FC-9 decode, lens engine, revision inference |
| `crates/smysl-core/src/cbor/keys.rs` | `schema_decl::LENSES = 5`, `HIGHEST = LENSES`; FC-9 shape keys |
| `crates/smysl-core/src/cbor/envelope.rs` | `dec_schema_decl`/`schema_decl_bytes`: key 5 stays raw bytes on `SchemaDecl` (new field `lenses: Option<Vec<u8>>`, written back verbatim) so the record round-trips byte-identically |
| `crates/smysl-core/src/types/annex.rs` | `SchemaDecl.lenses` field (struct is not `#[non_exhaustive]` today: a minor-API break, listed in CHANGELOG) |
| `crates/smysl-core/src/surface/parse.rs`, `write.rs` | `@schema … fields: {…}, lenses: […]` (SMYSL-2.3 spelling); `schema_decl_has_surface_form` updated |
| `crates/smysl-core/src/diag.rs` | 20 codes of §8; facade test count updated with SMYSL-2.3's 4xx codes |
| `crates/smysl-graph/src/store/mod.rs` | `schema_decls()` |
| `crates/smysl-graph/src/lineage.rs` | `dependents_trace` |
| `crates/smysl-graph/src/relink.rs` | `successors` → `pub fn supersession_heads`; `relink` calls it |
| `crates/smysl-graph/src/salience.rs` | `corroboration` → `pub fn corroboration_groups` (score unchanged) |
| `crates/smysl-retrieve/src/lexical.rs` | `Bm25::index_subset` |
| `crates/smysl-embed/Cargo.toml`, `src/lib.rs` | model2vec-rs 0.3 `wasm`; manifest note; `SemanticSource` impl lives in the facade adapter to avoid an embed → query dependency |
| `crates/smysl-thread/src/schema.rs` | tables for `exposition`, `dialogue`, `dossier` once SMYSL-2.1 opens `ThreadSchema`/`Role` and SMYSL-2.3 assigns codes; `dossier` has no rules |
| `crates/smysl-thread/src/derive.rs` | refuse `Dossier`; `exposition`/`dialogue` ordering from SMYSL-2.4 span order and reply edges |
| `crates/smysl-render/src/profile.rs` | headings for new roles |
| `src/lib.rs` | `query`, `schema` modules; adapters |
| `src/main.rs` | §4.4 |
| `xtask/src/purity.rs` | `PURE_CRATES += "smysl-query"`; new `check-native` (below) |
| `fuzz/fuzz_targets/{sq_parse,lens_laws}.rs`, `fuzz/Cargo.toml` | new targets |

### 4.4 CLI

New `Cmd` rows (phase string = the release that wires them; written here as `<rel>`):

| name | about | purity |
|---|---|---|
| `q` | Run an sq query; explain, save, rerun | `Mixed` |
| `ask` | A dossier for a question; optional model answer | `Mixed` |
| `schema` | Show schema revisions; check the naming rule and lens totality | `Pure` |
| `migrate` | Re-derive units under a newer revision; staged | `Mixed` |

Flags:
- `q '<sq>' | --file Q.sq` · `--explain` · `--why` · `--save [--agent A]` · `--rerun <uid>
  [--current] [--save]` · `--param k=v` · `--all-versions` · `--idf partition|collection|library`
  · `--from-question "…" [--yes]` · `--engine-model DIR` (S0) / `--candle-model DIR` (S1).
  Output: `--json` (global), else a table; diagnostics and counts on stderr, as `find` does.
- `ask "…" [--scope '<sq scope>'] [--intent I] [--subject UID] [--budget N] [--answer]
  [--provider P] [--save]`.
- `schema show <id>` / `schema check [<id>]`.
- `migrate --schema <id> --to <rev> [--recipe R] [--stage DIR]`.
- `find` gains `--lang L` and `--analyzer A` (SMYSL-2.4 analyzers) and, through the shared planner
  helper, `--explain`. `find`'s `Cmd.purity` becomes `Mixed`, since `--engine semantic|hybrid`
  already depends on a model (OQ-54).
- `--seed-check` (global, today inert) is honoured by `q`, `ask`, `schema` and `migrate`
  (§3.2); `SMY-E416` on a model-dependent plan.

Test updates: `command_table_matches_section_23` (30 names), `only_ingest_and_attest_are_model_dependent`
(mixed = `thread, find, q, ask, migrate`), `tests/dispatch.rs` (30), `tests/cli-surface.txt`,
`tests/public-api*.txt`.

`--explain` format (text; `--json` gives the same fields):

```
sq/1  purity: pure  recipe: 7f3c…a1
pins:      x.text/v1@1 (compiled forward to @2)
lenses:    x.text/v1 1->2  chain 3b9e…02  312 units read, 0 unmappable
rewrites:  mode = reported  =>  mode = reported and text:verbatim = false   (widening, paired key)
scope:     store:bible.smy  → 41,287 candidates  (supersession: 12 heads substituted, 0 forks)
partitions (IDF: collection): kjv-1769/en  synodal/ru  luther-1912/de
  find "covenant" ~lex   analyzer en/v1, ru/v1, de/v1   fused RRF k=60
bridges:   ru↔en structural (verse ids); de↔en structural
filters:   kind in (claim, finding); status >= cited; mode = asserted
time:      said during 1600..1920   undated excluded: 0 (W415 not raised)
expand:    <-grounds depth 3  (truncated frontier: 0)
closure:   with rebuttals (live)
order:     salience desc (renormalised over result)
output:    pack 2000 tokens  estimator smysl/script-aware/1
```

### 4.5 Features, dependencies, purity gate, no-C status

- `smysl-query` dependencies: `smysl-core`, `smysl-graph`, `smysl-retrieve`, `smysl-pack`,
  `smysl-thread`, and `smysl-text` (SMYSL-2.4) for its core types. No third-party dependency.
  Joins `PURE_CRATES`; must pass the source grep (no `tokio`, `std::net`, `async fn`).
- Facade features: `query` is always on (pure). `semantic` (S0, existing), `semantic-candle`
  (S1, new, implies `semantic`), `providers` for the `Translator` adapter and `ask --answer`.
  `cli` enables `semantic` only if the project decides to ship a default model path (it does not
  today: `default = ["cli", "local", "render-typst"]`).
- **No-C status, as a check rather than a sentence.** A new `xtask check-native` runs `cargo tree
  -e normal,build` for each feature set (`--no-default-features`, `cli`, `cli,semantic`,
  `cli,semantic-candle`) and fails on any `-sys` crate other than `linux-raw-sys`, and on `cc`
  reached by anything except `blake3`'s build script (verified present, §2.2). This puts F-11's
  and draft 3 §18's no-C claims under CI, and would have caught the imprecision in F-11.

## 5. Tests, fixtures and harnesses

### 5.1 Grammar golden tests (`crates/smysl-query/tests/grammar.rs`)

- `fixtures/sq/accept/*.sq` with `*.ast` (pretty-printed AST): every Appendix C production, the
  §11.1 and §11.5 examples, comments and `#` in locators, multi-word keywords, parameters.
- `fixtures/sq/reject/*.sq` with `*.diag`: one expected `SMY-E460` (span, message) per file:
  unterminated string, dangling `|`, `depth` without number, `using sq/2` (`E427`), unbound `$x`.
- Regenerated with `SMYSL_BLESS=1` (render's convention; the pack golden uses another spelling,
  which this RFC does not add to).
- Round trip: `parse(print(parse(q))) == parse(q)` for every accept fixture.

### 5.2 Planner snapshot tests (`tests/plan.rs`)

- For each query in `fixtures/sq/plan/` over `fixtures/corpus/F1…F13` (and the SMYSL-2.4 Bible
  fixtures once present), the `--explain --json` output is snapshotted. A changed plan is a
  reviewable diff.
- Mapping coverage: one test per row of §3.3 asserting the plan names the expected function, so a
  stage silently reimplemented beside an existing API fails a test.
- Determinism: plans and results identical when the store's records are appended in shuffled
  order (seeded), as `merge_algebra.rs` does for merge.

### 5.3 Lens laws (D-11): seeded property tests + fuzz

`crates/smysl-core/tests/lens_laws.rs`, using the repository's seeded xorshift generator (no new
dev-dependency; `proptest` is not in `Cargo.lock`). Generators: random revision histories of
1–4 steps over closed ops, random payloads drawn from the declared fields.

| law | statement |
|---|---|
| L1 round trip | for an invertible chain `c` and a unit `u` in its domain: `backward(forward(u)) == u` |
| L2 image | `forward(backward(v)) == v` for `v` in the image of `forward` |
| L3 associativity | `compose(compose(a,b),c)` and `compose(a,compose(b,c))` agree on hash and on application |
| L4 identity | `identity ∘ c == c` |
| L5 inference harmless | lensing from any revision consistent with a unit's clues gives the same view |
| L6 retire | any chain containing `retire` refuses `backward` with `E422` |
| L7 never guess | a value outside every map is reported unmappable, never mapped |
| L8 GE-T16 (query-compile equivalence) | for a pinned query `q@r` and a store `S`: `eval(compile_forward(q), lens(S)) == eval(q', migrate(S))` with `q'` the same compile run on migrated data; equality of result uid sets |
| L9 byte stability | `encode_lenses(decode(bytes)) == bytes` |

Fuzz: `fuzz/fuzz_targets/lens_laws.rs` (arbitrary bytes → decode → L1, L3, L9 must hold or the
decode must report) and `sq_parse.rs` (never panics; accepted text re-prints and re-parses).

### 5.4 Saved queries and reruns

- Save, append data, rerun pinned: `schema_effect` absent, `data_effect` equals the appended set.
- §19.5 reproduction: revision 2 with the `quoted → reported + verbatim` widening; a saved
  `mode = reported` query; rerun pinned shows only the data effect; `--current` reports the
  schema effect exactly equal to the injected `quoted` units.
- `E466` when a declaration is missing; `W467` when a model hash changes (stub source).
- Salience isolation: saving a query does not change `salience` of any result (guards OQ-49's
  hazard whichever edge kind is chosen).

### 5.5 Dossiers (GE-T10)

- Unit: intent rules per tier-1 language (positive and negative examples), `W417` on an
  ambiguous subject fixture ("Mary"), `E468` on an unclassifiable question, `gap` steps for
  empty slots, the view's membership reaching every slot result, rule R in the pack.
- Answer step with a stub provider: units grounding outside the dossier dropped (`W478`), a
  `cited`-grounded answer over an `inferred` slot lowered (`W036`), injected instructions in a
  passage ignored (canary from SMYSL-2.1).
- **GE-T10 harness** (`crates/smysl-query/tests/ge_t10.rs`, ignored by default): 30 questions ×
  5 languages, expert-filled slots as uid sets in `fixtures/dossier/gold/`; reports slot
  precision and recall per intent and slot. Kill criterion from draft 3 §23: precision < 0.8 for
  *definition* or *counter-evidence* keeps `ask` out of defaults.

### 5.6 NL front end (GE-T12)

- Unit: model outputs that do not parse, name undeclared fields, or use a disallowed stage are
  rejected (`E474`) and never executed; the translated query is printed before running.
- **GE-T12 harness** (ignored by default, needs a provider): 100 questions with expert-written
  queries; the metric is **equivalent results** (equal result-uid sets), not textual equality.
  < 70% keeps the front end experimental.

### 5.7 Seed-check, schema check, migrate, embeddings

- Every pure query in `fixtures/sq/plan/` passes `--seed-check`; every model-dependent one fails
  with `E416` before executing (no model loaded).
- `schema check`: fixtures for each rule N violation (renamed meaning, reintroduced retired name,
  unpaired widening, fork) with the expected code.
- `migrate`: deterministic migration of the GE-T16 corpus; attestations `op: Transformed` with
  the chain hash; rule M respected; old units superseded; `relink` re-points; commitments only
  proposed (`W472`).
- S0: existing `smysl-embed` tests pass on 0.3 (done in a scratch worktree, §2.2); add a
  multilingual smoke test gated on `SMYSL_EMBED_MODEL`.
- S1: encoder determinism within a build (two runs equal), cache invalidation by model hash
  (`W476`), pooling unit tests against reference vectors computed once and stored (tolerance
  stated, since cross-machine agreement is not claimed).

### 5.8 Cross-implementation

- The Python, JavaScript and Go implementations target C-Read first (D-8). For this RFC they
  need only FC-9 decoding and `encode_lenses` byte stability (L9) on the fixtures in
  `fixtures/schema/fc9/`; lens application is not a conformance obligation (D-8).

## 6. Delivery steps

Each step has an exit test. Steps within a phase are ordered.

**TX-P6 — `sq` core (pure)**
1. `smysl-query` skeleton, lexer, parser, AST printer. *Exit:* §5.1 accept/reject goldens pass;
   `sq_parse` fuzz runs 10 minutes clean.
2. `Scope` port with a `Store` implementation; planner phases 1–4; `--explain`. *Exit:* plan
   snapshots for `from store:` queries over `fixtures/corpus`.
3. Lexical source with `Bm25::index_subset`, RRF, `--why`. *Exit:* single-partition results equal
   today's `find` ranking on `fixtures/retrieval/queries.tsv`; RRF determinism test.
4. Filters (kernel fields, raw payload — lenses come in TX-P12), traversals with
   `dependents_trace`, closures, set ops, ordering, `limit`, `pack`, outputs. *Exit:* §3.3
   mapping-coverage tests; shuffled-append determinism.
5. Supersession pass with `supersession_heads`; `relink` refactored onto it. *Exit:* existing
   relink tests unchanged; `W464` on a fork fixture.
6. `smysl-text` scope, partitions, time stages, entity bridge (needs SMYSL-2.4 TX-P1–P5).
   *Exit:* §11.1 and §11.5 examples run on the Bible and chat fixtures; `W415` counts match.
7. CLI `q`, `--seed-check`; purity gate. *Exit:* draft 3 TX-P6 exit — every pure query passes
   `--seed-check`, plans stable; `xtask check-purity` green with `smysl-query` listed.

**TX-P8 — dossiers, saved queries, thread schemas** (after TX-P7 for classes)
1. `x.query/v1` save/load, result edges (`x.query/answers`, OQ-49 resolved), reruns with the data effect. *Exit:*
   §5.4 first and last bullets.
2. FC-8 tables in `smysl-thread` (after SMYSL-2.1 opens the enumerations and SMYSL-2.3 assigns
   codes); `exposition`/`dialogue` derivation; `dossier` refused by `--derive`. *Exit:* derivation
   tests on chat and book fixtures; old binaries' behaviour per F-12 documented.
3. Intents, templates, `ask`, dossier view and thread, rendering headings. *Exit:* §5.5 unit tests.
4. GE-T10 run. *Exit:* draft 3 TX-P8 exit (GE-T10 numbers recorded; kill criterion applied).

**TX-P10 — embeddings and NL front end**
1. Verify S1 dependencies: versions, XLM-R support, `cargo tree` with no `-sys`/`cc` beyond the
   allowlist; add `xtask check-native`. *Exit:* `check-native` green on all four feature sets.
2. S0 upgrade to model2vec-rs 0.3 `wasm`; manifest note. *Exit:* `smysl-embed` tests pass (already
   shown in a worktree); multilingual smoke test with `potion-multilingual-128M` loads.
3. `smysl-embed-candle`: encoders, pooling, cache. *Exit:* §5.7 S1 tests.
4. `SemanticSource` adapters; `~sem`, `~hybrid`, `near`, embedding bridge. *Exit:* purity labels
   and `E416` on every semantic plan; GE-T5 cross-lingual arm run (SMYSL-2.4 owns GE-T5).
5. `Translator` port and provider adapter; `q --from-question`. *Exit:* §5.6 unit tests; GE-T12 run.

**TX-P12 — schema evolution**
1. FC-9 decode/encode in `smysl-core`, `SchemaDecl.lenses`, `Store::schema_decls`, `W430`, `W429`.
   *Exit:* L9 on fixtures; round-trip suite (`smysl-core/tests/roundtrip.rs`) green.
2. Lens engine and chains. *Exit:* laws L1–L7 at 10,000 seeded cases each; `lens_laws` fuzz.
3. Field resolution, lensed predicates in `sq`, pins, compile-forward, `E421`/`E422`, explain
   lines. *Exit:* §19.5 rewrite printed exactly as in §4.4.
4. Schema/data effect split. *Exit:* §5.4 §19.5 reproduction.
5. `schema show|check`. *Exit:* rule N fixtures.
6. `migrate`. *Exit:* §5.7 migrate tests.
7. `@schema` surface form for `fields:`/`lenses:`. *Exit:* `fmt` round trip of an FC-9 declaration.
8. Pointer lenses (with the `json/1` reader, SMYSL-2.4). *Exit:* a two-version snapshot series
   answers `pointer(…)` across the rename.
9. GE-T16. *Exit:* draft 3 TX-P12 exit — lens path equals migrate path on all 100 saved queries
   and 30 dossiers; the effect split reproduces the injected change; unmappable rate reported.

## 7. Risks and mitigations

| risk | mitigation |
|---|---|
| Saved-query result edges perturb salience (`answers` carries rank) | `RESULT_EDGE` constant, `x.query/answers` proposed, salience-isolation test (§5.4); no `--save` with kernel `answers` until salience excludes `x.query/*` sources |
| RRF over engines reintroduces the merge measured worse than routing | RRF only across partitions of one engine; `Hybrid` routing kept and recorded (OQ-46) |
| Per-partition indexes rebuilt per call make `q` slower than `find` (F-10: 13 s at 156k) | cache within one invocation; persistent postings in SMYSL-2.4 TX-P4; plan snapshots record partition counts so regressions are visible |
| Revision inference wrong for hand-written units | rule N makes ambiguity harmless; law L5 tests it; `W431` covers extracted units |
| Unknown lens ops from a newer peer | `LensOp::Unknown`, `W469`, units unmappable and counted; bytes preserved |
| `SchemaDecl` gains a field (not `#[non_exhaustive]`) | one minor-API break in the release that ships FC-9, recorded in CHANGELOG; `tests/public-api.txt` updated deliberately |
| Dossier ranking by product of incomparable scales | quantised terms, GE-T10 decides; OQ-52 |
| NL translator emits a query that reads more than asked | parse + resolve + stage allowlist, query shown before running, `--yes` required |
| Prompt injection through dossier passages in `ask --answer` | SMYSL-2.1 guard; ground-only-on-dossier filter (`W478`); rule M cap |
| S1 dependencies bring native code | verification is TX-P10 step 1; `check-native` in CI |
| Templates drift from GE-T10 gold | template-set id and hash in every recipe; reruns refuse an unknown set (`E477`) |
| `smysl-query` grows a dependency the gate forbids | `PURE_CRATES` membership checked per crate by `xtask check-purity` |

## 8. Diagnostics allocated in this RFC (SMY-E/W460–479)

| code | meaning |
|---|---|
| E460 | `sq` syntax error, or an unbound `$parameter` (span reported) |
| E461 | a stage is not available in this build or phase (`match`; `~sem` without a semantic feature; `text(…)` without `smysl-text`) |
| E462 | a traversal names a relation kind no schema in scope declares and the kernel does not define |
| W463 | a traversal stopped at `depth` with an unexplored frontier; count reported |
| W464 | supersession fork in scope: no single head, every successor kept; count reported |
| W465 | `lang:any` found no bridge for some partitions; they answered in their own language only; count reported |
| E466 | a saved or pinned query cannot be pinned: a recorded revision or lens-chain hash is not reproducible from the store's declarations |
| W467 | a model-dependent saved query was rerun under a different model hash; diffs are not comparable |
| E468 | the question matches no intent rule; `--intent` required |
| W469 | a lens step carries an op this build does not know; the step is unusable (units counted under W420) |
| E470 | no lens path between the requested revisions (gap or cycle) |
| E471 | a lens step is ill-formed: `default` without a paired `require` under the same condition, an op on an undeclared key, a widening without its paired key |
| W472 | migration proposed commitments or datings for review instead of transferring them; counts reported |
| E473 | migration needs information the old units lack; a model recipe is required |
| E474 | the natural-language front end produced text that is not an acceptable `sq/1` query; nothing was executed |
| W475 | a `json/1` snapshot has no version marker; read without a pointer lens; count reported |
| W476 | a cached vector under another model hash was ignored and recomputed |
| E477 | unknown dossier template set on `ask` or rerun |
| W478 | an answer-step unit grounds outside its dossier and was dropped |
| W479 | independent-witness counting fell back to attestation groups for units whose sources the derivation graph does not cover |

Codes from draft 3 used here (meanings in the set's registry, SMYSL-2.0 §6.1; W409, E410, W420,
E426, W429, W430 are also referenced by SMYSL-2.3): W409, E410, W415, E416, W417, W420,
E421, E422, E426, E427, W429, W430, W431. Existing: W036, E001.

## 9. Open questions

Draft 3 questions this RFC touches: **OQ-14** (IDF scope; default collection, recorded per
query), **OQ-15** (partial dependence; `Witnesses` trait leaves room), **OQ-16** (contentions
across stores; computed by merge into the scratch store until SMYSL-2.8), **OQ-21** (name `sq`;
saved queries in the format — this RFC needs only FC-5's declaration), **OQ-22** (intent
classifier; rules by default, `Classifier` port), **OQ-23** (closed op set; `Unknown` preserved),
**OQ-24** (state the lens chain always or only under `--explain`; proposed: one stderr line
whenever a lens touched a unit, silence when none did).

New:

| id | question |
|---|---|
| OQ-45 | RRF constant: fixed `k = 60` recorded in the recipe, or a per-query parameter? A parameter makes reruns depend on one more number. |
| OQ-46 | Should `~hybrid` ever fuse lexical and semantic results (RRF) rather than route? The 0.78 vs 0.84 MRR measurement says no for the English corpus it was taken on; a multilingual corpus may differ. |
| OQ-47 | `INTERPRETIVE` in `smysl-embed` includes `Question`; draft 3 §11.2 lists claim, finding, hypothesis. Which is the routing rule `sq` records? |
| OQ-48 | **Decided (SMYSL-2.0 §3):** the lens engine lives in `smysl-core`, because check, render and retrieval need it below `smysl-query`. |
| OQ-49 | **Resolved in SMYSL-2.3 A-11:** `x.query/answers`, inert for salience. |
| OQ-50 | `match` (graph patterns): which subset, and in which phase? Refused (`E461`) until decided. |
| OQ-51 | NL front end: is printing the query and requiring `--yes` enough, or must a saved translated query also record a human approval attestation? |
| OQ-52 | Dossier slot ranking: product of quantised terms (draft 3), lexicographic, or RRF over per-criterion ranks? GE-T10 should compare at least two. |
| OQ-53 | Lens ops address top-level payload keys. Do nested keys (`payload_strings`' dotted paths) need lenses, and if so, is the dotted path the key? |
| OQ-54 | **Resolved by SMYSL-2.1 H-20** (TX-P0): `find` and `pack` are retagged `Mixed`. |
