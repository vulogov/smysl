# RFC SMYSL-2.4 — Library and ingest

**Status:** draft 1, for discussion. Implementation RFC (non-normative); normative rules are in SMYSL-2.3.
**Author:** Vladimir Ulogov
**Date:** 2026-10-02
**Part of:** RFC set SMYSL-2 — see SMYSL-2.0 (index and roadmap).
**For:** crate `1.9.0-dev` (`d25ec9e`), format `smysl/1.0`, kernel `smysl.kernel/0.1`.
**Derived from:** RFC SMYSL-2 draft 3, §3, §4, §5, §6 (implementation of the time engine), §7, §8,
§9, §10 (except §10.4), §14.1–14.3, §19.1–19.4, §20; phases of §22.
**Phases:** TX-P1, TX-P2, TX-P3, TX-P4, TX-P5, TX-P7.
**Depends on:** SMYSL-2.3 (records 14–19, ids, FC-1/2/3/5, rules E, N, Z, copy and omission
rules, `strict` classes); SMYSL-2.1 (TX-P0: F-13/F-14 surface fixes, normaliser v2, script-aware
estimator, opened enumerations, record-set digest, today's prompt-injection guard); SMYSL-2.2
(spike S0, whose GE-T2/GE-T5 readings decide whether TX-P5 starts as planned).

Section numbers of the form "draft 3 §N" refer to the design source. They are kept unchanged:
FC-n, records 14–19, GE-Tn, OQ-n and TX-Pn mean what draft 3 says they mean.

---

## 0. Summary

This RFC is the plan for building the library, the readers, the time engine, text ingest, linking
and the proposition layer on top of the tree as it is at `d25ec9e`. It says what exists, what each
crate gains file by file, what the new crate `smysl-text` contains, which commands appear, how each
piece is tested, and in what order it lands.

- **One new crate, `smysl-text`**: normalisation, parts, readings, manifests, a content-addressed
  object store, readers (one feature each), structure and locators, segmentation, language
  identification, analyzer chains, the effective-time engine, the redb substrate index, the
  library lock and reader resource limits.
- **Record types and identifiers go into `smysl-core`**, not `smysl-text`. `Record` is the enum
  every store decodes into, and the Python, JavaScript and Go implementations work at that level
  (D-8: C-Read computes tid, mid, did and rdid). `smysl-text` re-exports them.
- **`Store` changes little.** It learns the new records, enforces redaction on every append
  (rule Z), keeps a by-part map, and stops rebuilding the adjacency for batches that cannot change
  it. `StoreRead` and sharded stores stay in SMYSL-2.8.
- **Ingest changes in six places** (draft 3 §7.7), plus two found while verifying the code: a
  unit with no grounds cannot be capped at `inferred` (the constructor refuses it), and the
  `ingest:quote` payload key is inside identity today, so text units drop it once a span is
  attached.
- **Security (D-10)**: reader limits are hard caps with diagnostics; the existing fence in
  `prompt.rs` is extended to windows, ledger digests and linker pairs; references a model may
  write are limited to its digest and batch; one writer per shard behind a lock file.
- **Measured risks found here**: `redb` 4.3 declares `rust-version = 1.90` against the
  workspace's — which was `1.79` when this was written and is `1.85` since **OQ-40 was answered
  by measurement in 1.10.0**, where the 1.79 claim turned out to be false already and in every
  selection (§4.5); `DetectionKind` is a closed enumeration, so time contentions cannot be
  written to a log yet; `unicode-segmentation` 1.13.3, already in `Cargo.lock`, declares 1.85 —
  which is now the base rather than above it.

Diagnostics allocated here: `SMY-E440`–`SMY-W451`. Open questions: OQ-35–OQ-44.

---

## 1. Scope

**In.**
- Library model (draft 3 §4): records 14, 15, 18, 19 handled in code; object store; growth and
  revision; metadata levels; carry and licence gating; redaction enforcement.
- Readers, structure, locators, segmentation, chats (§5).
- Time (§6): EDTF level 1, calendars, record 17, free constraints, the temporal network, locks,
  effective-time index. Normative rules (copy and omission, rule E, lock semantics) are SMYSL-2.3's;
  this RFC implements them.
- Assertions and ingest (§7): FC-1, FC-3, `x.text/v1` (FC-5) as used by ingest, entities without
  span, text ingest, the language policy.
- Linking (§8): ledger digest, linker pass, summary tree, thread-following windows for chats.
- Proposition layer (§9): same-as staging, class derivation, engines `identical-span`, `lexical`,
  `anchored`.
- Languages (§10.1–10.3, §10.5): `lingua` with five languages, analyzer chains, wiring of the
  script-aware estimator that SMYSL-2.1 adds.
- Substrate index (§14.1–14.3): the redb tables this phase set needs, persistent postings.
- Security gaps owned by this RFC under D-10.

**Out.**
- `sq`, dossiers, schema-evolution tooling, embeddings, the `semantic` engine and the NL front end
  (SMYSL-2.5). `same-as propose --engine semantic` is reserved here and implemented there.
- Measures and assessment (SMYSL-2.6). This RFC provides class derivation, which 2.6 reads.
- `lib import` / `lib export`, quarantine and the `imported`/`waiting` tables (SMYSL-2.7).
- `StoreRead`, sharded virtual union, redb-backed unit store (SMYSL-2.8). Logs here are already
  one per shard on disk (§14.2 layout), but each command opens the shards it needs into one
  in-memory `Store`, which F-1 shows is enough at 10⁵ units.
- FC-9 surface spelling of `fields:`/`lenses:` (SMYSL-2.5, TX-P12). Consequence for TX-P5 is in
  §3.4.

---

## 2. What exists today (verified against `d25ec9e`)

Every statement below was read from the tree. `cargo test --offline -p smysl-core -p smysl-graph
-p smysl-retrieve -p smysl-check` passes at `d25ec9e` (rustc 1.95.0 on the build host).

### 2.1 `smysl-core`

| item | where | what it is today |
|---|---|---|
| `UnitCore` | `src/types/unit.rs` | `#[non_exhaustive]`, fields `schema, gist, body, detail, deps, grounds, status, source, payload, extra`. Built only through `UnitCoreBuilder` / `UnitCore::new`, which NFC-normalises and refuses `detail` without `body`, authored `unfounded`, `cited`/`measured` without source, and **`derived`/`inferred` with empty grounds** (`ShapeError::GroundsRequired`). |
| `SourceRef` | `src/types/epistemics.rs` | `kind: SourceKind, reference: String, captured: Option<Date>, observed: Option<u64>, extra: Extra`. `extra` preserves unknown keys since 1.7. `Date` is `year: u16`, closed by design. |
| `SourceKind` | same file | `Url, File, Metric, Tool, Doc, Node` (0–5). `from_u8` returns `None` for anything else, and `dec_source` then fails the record (F-12). |
| `Record` | `src/types/record.rs` | `#[non_exhaustive]` enum, codes 1–13 in `mod code` with a `KNOWN` list; `Record::Unknown { code, payload }` for anything else (`SMY-W014`). `is_hashed()` is true for `Unit` only. |
| envelope | `src/cbor/envelope.rs` | `to_cbor` / `from_cbor` / `from_cbor_seq`; one `*_bytes` encoder and one `dec_*` decoder per record. Unknown codes go through `d.skip_item()`, which validates canonical form strictly. `from_cbor_seq` stops cleanly at a truncated tail. `dec_source` keeps unknown source keys in `extra` through `read_map`. |
| keys | `src/cbor/keys.rs` | one module per record. `unit::HIGHEST = PAYLOAD (8)`; `source::HIGHEST = OBSERVED (3)`; `schema_decl::HIGHEST = PAYLOAD_SHAPE (4)`. Test `key_tables_are_contiguous_from_zero` forbids gaps. |
| ids | `src/ids.rs`, `src/hash.rs`, `src/types/relation.rs` | `Uid([u8; 32])`, text form `b3:` + 52 base32 chars (`FULL_CHARS`), 26 for display. The rid preimage starts with byte `0x03` (`relation.rs:233`). `LangTag` exists (view key 6). |
| nesting cap | `src/cbor/mod.rs` | `MAX_NESTING = 128`, applied by the CBOR skipper and the HJSON parser (`surface/hjson.rs`, `check_depth`). |
| surface parser | `src/surface/parse.rs`, `src/surface/lex.rs` | `@doc @rel @thread @schema @withdraw @resolve @commit` are reserved line classes (`lex.rs`); unknown `@x.dom/type` with a label lexes as a record. `source()` reads only `kind`, `ref`/`reference`, `captured`, `observed`, **silently drops** malformed `captured`, negative `observed` and any other key (F-13, F-14 still present; SMYSL-2.1 fixes them). |
| surface writer | `src/surface/write.rs` | `write_surface`; `schema_decl_has_surface_form`, `withdrawal_has_surface_form`, `resolution_has_surface_form` decide what stays CBOR-only. |
| quote check | `src/quote.rs` | `Support::{Present, Loose, Absent}`; `support`, `support_span` (1.5: byte range into the source as given, earliest subsequence for `Loose`), `support_in_span(quote, &[(name, text)])` (Present anywhere beats Loose anywhere), `verify(units, source)` → `E307` / `W308`. `QUOTE_KEY = "ingest:quote"`, read from the **payload** — so the quote is inside the unit's identity. |
| HJSON | `src/surface/hjson.rs` | span-tracking, untyped, hand-rolled; `smysl-core` has no serde dependency. Its `Cargo.toml` depends on `blake3` and `unicode-normalization` only. |
| contentions | `src/types/annex.rs`, `src/types/lifecycle.rs` | `DetectionKind` = `SupersessionFork, LiveRebuttal, LabelCollision, CommitmentFork`; `from_u8` closed. `ContentionId::derive(kind, over, positions)` hashes `kind.as_u8()` and the sorted positions, so an id exists without a recorded contention. `ResolutionTarget::Contention(ContentionId)`. |
| diagnostics | `src/diag.rs` | `registry!` macro, groups `Parse, Identity, Lod, Epistemics, Merge, PackRender, Extension, Provider`; test `the_registry_is_the_size_it_is_meant_to_be` asserts 57 codes. No 4xx codes yet. |

### 2.2 `smysl-graph`

| item | where | what it is today |
|---|---|---|
| `Store` fields | `src/store/mod.rs` | `records: Vec<Record>` (the log in order), units, relations, threads, views, contentions, `withdrawals: BTreeMap<Uid, …>` and `commits: BTreeMap<Uid, …>` keyed by target "whether or not that relation/unit has arrived", `rids`, **`record_hashes: BTreeSet<[u8; 32]>`** (private; BLAKE3 of every record's canonical encoding), `unfounded`, `adjacency`, `pending_attestations`, rolling `log_hasher`. |
| `open` / `open_with` | same | read the whole log, `from_cbor_seq`, `W110` on trailing bytes or a stale sidecar (`.smysl/index/<name>.idx`), `absorb`, optional hash verification. |
| `append` | same | per record: encode, hash, skip if in `record_hashes` or repeated in the batch; append bytes to the file; `absorb(fresh)`; advance the rolling hash. **`absorb` ends with `rebuild_adjacency()` on every call**, whatever the batch held. |
| append cost | doc comment on `append`; CHANGELOG 1.8.0 "Appending stopped costing the store" | adjacency rebuilt once per call: 1336 µs/record at 1 record per call, 25 µs at 50, 5 µs at 1000; 20,000 records in batches of 100 take 0.30 s. Lazy rebuild rejected to keep `Store: Sync`. |
| `absorb` | same | arms for every known record; `_ => {}` for the rest, which are still kept in `records` and hashed. |
| merge | `src/merge/mod.rs` | `merge(store, other, opts)` = `store.append(other.iter())` then contention **detection, which is reported, not recorded** (detection is not monotone; recording it would break associativity). |
| `units_with_source_prefix` | `src/store/mod.rs` | linear scan over every unit, `reference.starts_with(prefix)` (1.5). |
| `units_in_observed_order` | same | collects and sorts every unit with `observed`; units without it are left out (F-15). |
| `emit` (bundle) | same | ends its filter with `_ => false`, dropping schema declarations, pack infos, commitments and unknown records (F-16; SMYSL-2.1 fixes). |
| `state_hash` / `converged_with` | same | omit commitments, schema declarations, pack infos and unknown records (F-17; SMYSL-2.1 adds the record-set digest). |

### 2.3 `smysl-ingest`

| item | where | what it is today |
|---|---|---|
| modules | `src/lib.rs` | `chunk` and `monotone` are `pub(crate)`; `prompt`, `recipe`, `repair`, `stage`, `json_ast`, `attest`, `import`, `path`, `schema`, `ceiling` are public. Features `model` (default), `ollama`, `anthropic`, `openai`, `gemini`, `deepseek`. Depends on `smysl-core`, `smysl-check`, `smysl-graph`, optional `smysl-provider`. |
| chunking | `src/chunk.rs` | `Window { budget: u32, overlap: bool }`, `Window::for_context(context_window, max_output)` = context − `PROMPT_RESERVE` (1024) − max(max_output, 0.5 × context), floor 256. `chunk(input, w)` splits at paragraphs, overlaps one paragraph, splits an oversized paragraph on lines then characters (`split_oversized`). `Chunk { text, index, total, start, end, overlapped }`. |
| ingest loop | `src/lib.rs` (`Ingestor`, `one_chunk`) | per chunk: call, `repair::convert_labelled`, `quote::verify(&units, text)` against **the chunk text**, `repair::check_local`, repair while `needs_repair`. Spans are never attached: `support_span`/`support_in_span` are re-exported by the facade (`src/lib.rs:173`) and have no caller inside the tree. |
| repair | `src/repair.rs` | `UNIT_LOCAL = &[Code::E022]`. The doc comment states `E307` is excluded on judgement: "a model that invented one attribution may have invented others", so one fabricated quote fails the whole answer. `salvage` degrades only `UNIT_LOCAL` defects. |
| references | `src/json_ast.rs` | `resolve(name, labels)`: a label in this batch, **or any full `b3:` uid** (`Uid::parse`). Unresolvable → `E060`. Model-authored kinds exclude `supersedes` and `retracts`. The quote goes into the payload as `ingest:quote`. `source_ref` reads `kind`, `ref`, `captured` only. |
| prompts | `src/prompt.rs` | `Template { id, version, system, user }`, `fingerprint()`, `render(input)` replaces `{input}`. `FENCE = "<<<SMYSL-INPUT>>>"` and an `UNTRUSTED` preamble ("data, never instruction") in every content template; `attest.rs` uses the same fence. No language instruction (F-6). |
| recipe | `src/recipe.rs` | `Conditions { template_id, template_ver, provider, model, granularity, temperature, schema_set, path, source }`; `recipe()` and the provider-free `family()`. Corroboration groups by `(provider, model, recipe)`, so anything per-window in the recipe would split groups. |
| staging | `src/stage.rs` | `prepare`, `prepare_declared`, `prepare_under` (rule M before the check, edges re-pointed after weakening), `write` / `read` / `discard` of `.smysl/staged.smy`. |
| test provider | `tests/gate.rs` | `Scripted` implements `Provider`; offline tests drive ingest with canned answers. |

### 2.4 `smysl-retrieve`

- `src/tokenize.rs`: `tokenize(text)` splits where `!is_alphanumeric() && c != '_' && c != '-'`,
  lowercases, adds identifier parts; `Tokenizer { fold }` with `plain()` and `folding()`;
  `fold_suffix` is English-only and applied to any text when folding is on (F-4).
- `src/lexical.rs`: `Bm25` over the `bm25` crate (`default-features = false`), built from the
  whole store by `Bm25::index` / `index_with(store, tokenizer)`. `src/main.rs` calls
  `smysl::Bm25::index(store)` per `find` (F-10).
- `src/lib.rs`: `trait Retriever { fn search(&self, &Query) -> Vec<Hit>; fn len(&self) -> usize }`.

### 2.5 `smysl-check`

- `src/lib.rs`: `enum Pass` (`#[non_exhaustive]`, numbered 1–11, `CommitmentSupport` appended last
  with a comment that new passes go at the end); `Pass::IMPLEMENTED`; `CheckOptions` (Clone,
  Default); `check(store, opts)`; `ConformanceClass::{Read, Consume, Produce, Merge, Full}` with
  `forbids(code)`.
- `src/passes/`: `closure, commitment, epistemics, extension, granularity, integrity, shape,
  trust`; each `run(store, …, &mut Report)`, none may stop the pipeline.

### 2.6 Facade, CLI, gates, features

- Root `Cargo.toml`: features `default = ["cli", "local", "render-typst"]`, `cli`, `tui`,
  `semantic`, `providers`, `stage`, `ingest`, `local`, `remote`, `render-*`, `exact-pack`,
  `tls-pure`. Workspace `rust-version = "1.85"` since 1.10.0 — `1.79` when this was written,
  and false then (§4.5, OQ-40) — with `smysl-provider` and `smysl-ingest` declaring 1.86 and
  `smysl-tui` and the facade 1.88. `resolver = "2"`, `fuzz` excluded.
- `src/main.rs`: `enum Purity { Pure, Mixed, Model }`; `struct Cmd { name, about, purity, phase }`;
  `const COMMANDS` (26 rows; `ingest`, `attest` are `Model`, `thread` is `Mixed`; the comment in `tests/cli-surface.txt` still says twenty-two). `cli()` builds
  one subcommand per row and adds arguments in a `match c.name`; `tests/cli-surface.txt` records
  every `command argument` pair and `command_table_matches_section_23` checks the table.
- `xtask/src/purity.rs`: `FORBIDDEN_DEPS` (incl. `serde_json`, `clap`, `tokio`, `ureq`) checked on
  `cargo tree -p smysl --no-default-features` **and** on `cargo tree -p <crate> -e normal` for each
  of `PURE_CRATES = [smysl-core, smysl-graph, smysl-check, smysl-pack, smysl-thread, smysl-render,
  smysl-retrieve]` — at the crate's **default** features. `FORBIDDEN_SYMBOLS` greps sources for
  `std::net`, `TcpStream`, `async fn` and runtimes. Rule A: nothing in `src/` but `lib.rs` may
  name a sibling crate.
- `xtask/src/determinism.rs`: runs registered pure operations under `LC_ALL ∈ {C, ru_RU.UTF-8}`,
  `TZ ∈ {UTC, Asia/Tokyo}`, two hash seeds; asserts identical stdout.

### 2.7 Tests, fixtures, fuzzing, other implementations

- Property tests use a fixed-seed xorshift under `cargo test` (e.g.
  `crates/smysl-graph/tests/merge_algebra.rs`); `fuzz/` (cargo-fuzz, `libfuzzer-sys`, excluded
  from the workspace) has targets `surface, cbor, merge_algebra, pack_constraints, pack_exact,
  pipeline` driven by a `Choices` decision source in `fuzz/src/lib.rs` with the xorshift's shape.
  `proptest` is not in `Cargo.lock`.
- `fixtures/`: `corpus/` (F1–F13), `conformance/check` (`.smy` + exact `.expected` code sets) and
  `conformance/codec`, `wire/` (`uid/cases.json` generated by
  `crates/smysl-core/tests/gen_uid_fixtures.rs`, `contention-id`, `relation-id`, `invalid`).
- `python/smysl`, `nodejs/src`, `go/`: encoder/decoder/uid level, 2,931 lines of source in total
  (tests excluded), each with a conformance test over `fixtures/`.

### 2.8 Dependencies the plan adds, as published today

| crate | latest | `rust-version` | licence | note |
|---|---|---|---|---|
| `redb` | 4.3.0 | **1.90** | MIT OR Apache-2.0 | 3.1.3 needs 1.89, 2.6.3 needs 1.85, 1.5.2 needs 1.70 |
| `lingua` | 1.8.0 | not declared | Apache-2.0 | five language features only (§10.1) |
| `rust-stemmers` | 1.2.0 | not declared | MIT/BSD-3-Clause | |
| `unicode-segmentation` | 1.13.3 (in `Cargo.lock`) | **1.85** | MIT OR Apache-2.0 | today only via `ratatui` (`tui`) and `tokenizers` (`semantic`) |
| `caseless` | 0.2.2 | not declared | MIT | |
| `quick-xml` | 0.42.0 | **1.86** | MIT | |

Read from crates.io on 2026-10-02. Three declare a `rust-version` above the workspace's `1.79`;
the consequence is in §7 and OQ-40 — **answered in 1.10.0**, and not in the direction this table
implies. The question was never whether these three exceed the declared floor. It was whether the
declared floor was true of anything, and it was not: `blake3` has pulled `constant_time_eq` 0.4.2
for some time, whose manifest is edition 2024, so a 1.79 Cargo could not parse the dependency
tree of `smysl-core` — let alone compile it. See §4.5.

---

## 3. Design (adapted from draft 3)

This section restates draft 3 only where the implementation needs a decision. Layouts, keys,
identifiers and the rules themselves are SMYSL-2.3's.

### 3.1 Library (draft 3 §4)

**Placement.** Records 14, 15, 17, 18, 19 and the four identifiers are `smysl-core` types: the
envelope decodes them, `Store` holds them, C-Read implementations compute them. Everything that
interprets them (normalising text, reading files, deriving structure, writing objects) is
`smysl-text`.

**Where bytes live.**
- Logs hold records 14 (manifest), 17 (dating), 19 (redaction), units, relations and the rest.
- Part texts (15) and readings (18) live in the object store as their **record envelopes**: the
  file `objects/t3/ab/cd…` holds exactly `to_cbor(&Record::PartText(..))`. A bundle or export can
  then emit them verbatim, and verification is decode-then-hash.
- A plain `Store` (no library) that receives 15/18 in a bundle keeps them in its log, as it keeps
  any record. `smysl text add` never writes them to a log.

**Normalisation** (`norm`): UTF-8 validation, BOM strip, CRLF and lone CR → LF, NFC
(`unicode-normalization`, already a core dependency). A `Normalised` newtype is the only input
`Tid::of` accepts, so a tid cannot be computed over unnormalised bytes.

**Part policy.** A reader emits structure nodes; `part::Policy { target_min, target_max,
boundary_level }` groups whole nodes into parts. Defaults 64 KiB / 4 MiB at the reader's top
level are provisional until GE-T14 (end of TX-P2). The policy string is written to manifest key
17, so corpora built before the default changes stay valid under the policy they recorded.

**Growth and revision.** `text append <alias> <file>` re-reads only the new input, reuses tids for
unchanged parts (object already present: no write), and appends a manifest with `supersedes`. Two
heads under one alias are `SMY-W418`, reported by the library pass of `check` and by `text ls`.

**Carry.** `bundle`/`text` commands refuse to emit 15/18 unless manifest key 5 is `text` and key 4
is `public-domain` or on the permissive list compiled into `smysl-text` (`licence::PERMISSIVE`, a
sorted SPDX list; changing it is a code change with a test). `SMY-E402`, no override flag.

**Redaction (rule Z).** Implementation in §4.3.2. Object deletion happens after record 19 is in
the catalog log, so a crash between the two is repaired on the next open (the open path enforces
Z against the object store).

### 3.2 Readers, structure, locators (draft 3 §5)

**Reader contract.**

```rust
pub trait Reader {
    fn id(&self) -> ReaderId;                       // "osis/1"
    fn read(&self, input: &Input<'_>, params: &Params, limits: &mut Budget)
        -> Result<ReadOutput, ReadError>;
}
```

`read` is a pure function of the input bytes, the reader id and its parameters. It never reads a
clock, the environment or the filesystem beyond `Input` (an archive reader receives the archive
bytes, not a path). The `Budget` (§3.9.1) is the only side channel, and it is deterministic.

**Reader set by phase.**

| reader | feature | phase | crate dependency | note |
|---|---|---|---|---|
| `txt/1` | `reader-txt` | TX-P1 | std | |
| `md/1` | `reader-md` | TX-P1 | `pulldown-cmark` | MSRV and licence unverified |
| `usfm/1` | `reader-usfm` | TX-P1 | hand-rolled | |
| `osis/1`, `zefania/1` | `reader-osis`, `reader-zefania` | TX-P1 | `quick-xml` | DTDs refused (§3.9.1) |
| `json/1` | `reader-json` | TX-P1 | `serde_json` (OQ-37) | JSON Pointer locators |
| `telegram/1`, `slack/1` | `reader-telegram`, `reader-slack` | TX-P2 | `serde_json`; Slack also a zip reader (unverified choice) | |
| `whatsapp/1` | `reader-whatsapp` | TX-P2 | hand-rolled | date pattern is a required parameter |
| `mbox/1`, `epub/1`, `html/1`, `pdf/1` | `reader-mbox` … | TX-P2 (opt-in) | `mail-parser`, `rbook`, `html2text`, `pdf-extract` | not in `cli` |

**Structure.** `Structure` is an arena tree: `Node { level, range: Range<u64> (bytes in the
part), locator: Locator, children: Range<u32>, meta: Option<u32> (row in the segment table) }`.
Ranges of siblings are disjoint and ordered. The structure hash is computed over its canonical
CBOR encoding as SMYSL-2.3 defines it; rdid is computed over record 18's body.

**Locators.** `locator::parse` implements draft 3 Appendix B as a hand-written recursive-descent
parser (no regex dependency). Resolution is through the structure tree only:
`Structure::resolve(&Locator) -> Option<Range<u64>>`; a range locator resolves to the hull of its
two ends. `Locator::to_string` is canonical (round-trip tested), so a locator in a `ref` is
written one way.

**Segmentation.** `segment::Segmenter` = UAX #29 sentence boundaries (`unicode-segmentation`) plus
per-language abbreviation lists in `smysl-text/data/abbr/<lang>.txt`, compiled in with
`include_str!`. The segmenter id `smysl/seg-uax29+abbr/1` covers the lists; editing a list bumps
the id (a test hashes the lists and pins the hash to the id).

**Chats.** One segment per message, one part per UTC day by default (`--part-policy` overrides).
Reply and thread ids go into the segment table as substrate edges. Pseudonymisation:
`spk:` + 26 base32 chars of keyed BLAKE3 (`blake3::keyed_hash`) of the platform user id. The key is
read from `<library>/secrets/pseudonym.key` (created with 32 random bytes on first use, mode 0600,
never logged). `text append` to a pseudonymised expression without the key is `SMY-E450`, because
new messages from the same person would otherwise get a different speaker.

### 3.3 Time engine (draft 3 §6; rules are SMYSL-2.3's)

**EDTF.** The level-1 syntax parser and AST live in `smysl-core/src/edtf.rs` so the surface parser
and CBOR producers can reject a malformed `published` (`SMY-E410`) without depending on
`smysl-text`. `smysl-text::time::edtf` re-exports it and adds normalisation to intervals.
Covered: level 0 dates and date-times (`Z` and offsets), level 1 qualifiers `?` `~` `%`,
unspecified digits `X`, intervals with open (`..`) and unknown (empty) ends, negative years,
`Y`-prefixed years, seasons 21–24. Widening for `~` and `?` is SMYSL-2.3's rule; the AST keeps the
qualifiers so the rule is applied in one place.

**Instants.** `Instant(i64)`: milliseconds since 1970-01-01T00:00:00Z, proleptic Gregorian. Range
about ±2.9 × 10⁸ years. An EDTF value outside it is kept as text and indexed as an open bound
(`SMY-W449`). `Interval { lo: Bound, hi: Bound }` with `Bound::{Open, At(Instant)}`, half-open at
millisecond precision, so the omission rule's interval equality (SMYSL-2.3 A-2.4 rule 3) is exact
integer comparison.

**Calendars.** `time::calendar::julian_to_gregorian` via Julian Day Numbers; applied only in the
index, using the calendar of the manifest a unit names (`source.manifest`, key 18 of that
manifest). Never written back.

**Effective time.** `time::engine` computes, for a scope (a connected component of the constraint
graph, OQ-11), what rule E specifies. Implementation:

1. **Seed.** Each dated subject (unit, or group via part/manifest/window targets) gets the
   as-recorded interval and its **time status** from the table of draft 3 §6.7 (`observed` from
   `Op::Imported` at rung `computed` → `measured`; reader-supplied → `cited`; manifest
   `published` → `cited`; hand-supplied → `speculative`).
2. **Liveness.** A dating is live unless `store.withdrawals` holds its did (keyed as a `Uid`;
   `withdrawals` is already keyed "whether or not that relation has arrived"), its basis is
   unfounded (`Store::is_unfounded`), or it is **held** (SMYSL-2.3 A-12.2, liveness): a
   `canonical` commitment names the dating, or names the target's existing dating, and the new
   dating would change the effective value (`store.commitment_of(did_as_uid)`), until a
   resolution names the derived detection-kind-4 contention.
3. **Status of a dating** = status of its basis unit (D-4), `speculative` without one.
4. **Stratified propagation.** For each status level `s` from `measured` down to `speculative`:
   build the simple temporal network from absolute datings, offsets, relative datings and free
   constraints whose status is ≥ `s`; run Bellman–Ford (SPFA with a FIFO in node-id order) per
   component; a bound moves only if the stratum's status is ≥ the time status behind the bound,
   otherwise the would-be move is recorded as a `W412` (detection kind 4) contention position;
   `W413` (kind 5) is only for inconsistency, step 5. Each stratum
   is a plain STN, whose shortest-path solution is unique, so the result does not depend on
   record order (rule U).
5. **Inconsistency.** A negative cycle or empty interval in a stratum marks every node on it
   `Contested` with the datings involved as positions; nothing is chosen.
6. **Why-chains.** Each bound carries the dating or constraint that set it, so `date show --why`
   prints the chain of draft 3 §6.7.

This stratification is rule E as SMYSL-2.3 A-12.2 words it: a chain tightens at its weakest
link's status (OQ-42, resolved there). The property
harness (§5.3) checks it against a brute-force reference on small networks either way.

**Free constraints** (D-3): quotation (status of the quoting unit), derivation and supersession
(`cited`), reply (`cited`, evaluated on the first version's as-recorded send time), first-seen
(`derived`, the earliest HLC `ts` of any attestation of a unit grounded on the part).

**Contentions from time.** `W412`/`W413` contentions use `ContentionId::derive` with two new
`DetectionKind` values 4 (dating not applied) and 5 (temporal inconsistency), assigned by
SMYSL-2.3 A-8.2. SMYSL-2.1 opens `DetectionKind` in 1.9.0, but SMYSL-2.3 still forbids writing
record 6 with kind 4 or 5, so stores stay readable by older readers. Time contentions are
therefore **derived and reported, never written**: the same choice merge already makes for
detection. A resolution (record 12) names the derived id, which needs no contention record
(OQ-35, resolved in SMYSL-2.3).

### 3.4 Assertions and ingest (draft 3 §7)

**FC-1/FC-3 on the producer side.** `smysl-text::provenance::stamp(builder, &Attach)` is the one
function that writes `source.ref` (`t3:<tid>#<locator>`), `source.span`, `source.manifest`,
`source.published` (copy rule) or keeps `observed` (omission rule), core `lang`, and
`text:speaker` — the D-1 copy set and nothing else. Ingest, `lib import` (2.7) and tests call it;
nobody assembles these fields by hand.

**Holder and speaker before FC-9.** Draft 3 §7.4 has the surface parser resolve labels in
`text:holder`/`text:speaker` through FC-9 field types, which land in TX-P12. TX-P5 does not wait:
the ingest converter (`json_ast`) resolves those two keys itself against the batch and digest and
writes canonical `b3:` uids into the payload. Hand-authored surface documents write full uids
until FC-9's surface form ships. `SMY-W406` checks deps membership on uids, so it works in both
cases.

**`ingest:quote` and identity.** Today the quote is a payload key, hence inside the uid (§2.1).
Two windows quoting one claim with different wording give two uids for one span. For units that
receive a span, ingest removes `ingest:quote` (and the new `ingest:node`) from the payload after
attaching the span; the verdict (`Present`/`Loose`) and the quote go into the journal. Units that
get no span keep the quote so a reviewer can see what was claimed (OQ-41).

**Absent quotes (behaviour change).** For `ingest --text` only: repair is attempted first, within
the existing budget. If the quote is still `Absent`, the unit is staged with no span and its status
capped at `inferred` **when it has grounds**, `speculative` when it has none — `UnitCoreBuilder`
refuses `inferred` without grounds (`ShapeError::GroundsRequired`), so draft 3's plain "capped at
inferred" cannot be built. Reported as `SMY-W441`. Prose ingest keeps `E307` non-local, as
`repair.rs` argues.

### 3.5 Linking (draft 3 §8)

- **Ledger digest.** `smysl_pack::pack` over units already staged or stored for the expression,
  ranked by salience then reading order, budget `D` from the recipe, rendered as `label: gist`
  lines with digest-local labels `l/1…l/n`.
- **Digest-scoped references (new check).** The allowed set for a window is: labels declared in
  the batch, digest labels, and the uids behind them. Anything else that resolves (a full uid
  elsewhere in the store) is `SMY-E442`, repairable like `E060`. `E060` keeps its meaning.
- **Digest hash.** Draft 3 puts the digest hash in the recipe. The recipe groups corroboration
  (`recipe.rs`), so a per-window value there would split every window into its own group. The
  recipe records the digest **policy** (budget, ranking id); the per-window digest hash goes into
  the journal entry, which is what a rerun reads.
- **Linker** (`smysl link`): candidates from same entity, BM25 neighbours within scope and
  language, same theme, anchored alignment; judged pairwise by a model under a fenced template;
  staged (rule S). Candidate budget per unit is a recipe field.
- **Summary tree** (`smysl summarise`): levels from the structure (`chapter`, `book`, …); each
  summary is grounded on its children with `x.text/summarises` edges; rule M caps it, applied by
  `stage::prepare_under` as for any batch.
- **Chats.** Windows follow reply threads (substrate edges in the segment table), not wall-clock
  order.

### 3.6 Proposition layer (draft 3 §9)

- **Edges.** `x.text/same-as` relations, staged, attested. Engines that are pure attest at rung
  `computed` as agent `tool:smysl-same-as-<engine>` (spelling per `AgentId` rules, unverified).
- **Classes.** `strict` follows SMYSL-2.3's normative definition (D-7): members in uid order, each
  unassigned unit opens a class and admits later units adjacent to every member; class id = smallest
  uid. `component` (union-find, reported with diameter) and `attested:n` (strict over edges with
  ≥ n distinct attesting agents) are tool-level. Withdrawn edges are excluded through
  `Store::is_withdrawn`.
- **Engines.** `identical-span` (same tid, span and gist under different manifests; pure),
  `lexical` (BM25 neighbours within the same `lang`, through `smysl-retrieve`; pure; proposes, a
  judge or threshold policy accepts — OQ-44), `anchored` (units grounded on aligned spans via the
  `align` table; pure given the alignment). `semantic` is SMYSL-2.5.
- **Entailment and contradiction** go through the same propose → stage → attest path. A cross-text
  contradiction is a derived contention under the planned "rival explanations" detection; this
  RFC only stages the edges.

### 3.7 Languages (draft 3 §10.1–10.3, §10.5)

- `lang::detect` wraps `lingua` built with features for en, ru, es, fr, de only. Manifest language
  from reader metadata first, detection over the whole text otherwise; segment detection only in
  `mul` manifests, with a minimum length below which a segment inherits its neighbours' language.
- `analyze::Chain` per language: UAX #29 words → fold (`caseless`; ru ё→е; de ß→ss) → Snowball
  stem emitted **beside** the surface term (`rust-stemmers`) → extras (es `¿ ¡` dropped; fr elision
  kept). Chain ids such as `smysl/an-ru/1` are recorded wherever terms are used.
- `smysl-retrieve` gains an `Analyze` seam that `Tokenizer` implements; text stores default to the
  unit's language chain (unit `lang`, falling back to the view's). `fold_suffix` is gated to `en`
  by SMYSL-2.1.
- The estimator from SMYSL-2.1 (`smysl/script-aware/1`) is what text ingest uses to fill windows.

### 3.8 Substrate index (draft 3 §14.1–14.3)

`index.redb` behind feature `substrate-redb`. Tables built in this RFC: `manifests`, `parts`,
`by-part`, `redactions`, `segments`, `locators`, `align`, `said`, `composed`, `observed`,
`postings`, `journal`. (`vectors` and `queries` are SMYSL-2.5; `imported`, `waiting` SMYSL-2.7;
`schemas` SMYSL-2.5.) Each derived table has a builder `fn rebuild(&Library) -> Table` and an
incremental `fn on_append(&[Record])`; a test asserts rebuild and incremental produce the same
bytes. Without the feature, the same queries run against in-memory maps rebuilt on open — slower,
same answers.

### 3.9 Security (D-10)

#### 3.9.1 Reader resource limits

Every reader runs under a `Budget`. Exceeding any cap is an error with a diagnostic naming the cap
and the value; nothing is written (objects are staged under `objects/tmp/` and moved into place
only after the manifest is complete). There is no partial manifest and no silent truncation.

| cap | applies to | default | flag | diagnostic |
|---|---|---|---|---|
| `input_bytes` | any input file | 1 GiB | `--max-input` | `E440` |
| `decompressed_bytes` | total over an archive (EPUB, Slack zip) | 2 GiB | `--max-decompressed` | `E440` |
| `entry_bytes` | one archive entry, decompressed | 256 MiB | `--max-entry` | `E440` |
| `entries` | archive entry count | 100,000 | `--max-entries` | `E440` |
| `ratio` | decompressed ÷ compressed, per entry over 1 MiB | 200 | `--max-ratio` | `E440` |
| `nesting` | JSON/XML/HTML depth | 128 (= `smysl_core::cbor::MAX_NESTING`) | none | `E440` |
| `nodes` | structure nodes per manifest | 50,000,000 | `--max-nodes` | `E440` |
| `part_bytes` | one part, hard ceiling (one node larger than the policy maximum becomes one part up to this) | 64 MiB | `--max-part` | `E440` |
| `raw_bytes` | raw metadata kept per segment / per manifest | 1 MiB / 16 MiB | `--max-raw` | `E440` |
| `fuel` | deterministic work units charged per byte scanned, node created and entry opened | 64 × input bytes + 10⁸ | `--max-fuel` | `E440` |
| `wall_time` | whole `text add`, enforced by the CLI watchdog, never inside a reader | 30 min | `--timeout` | `E440` |

- **Fuel, not time, inside the pure path.** A reader that measured wall time would fail on a slow
  machine and pass on a fast one; fuel fails at the same byte on every machine, so a refusal is
  reproducible. The watchdog is the backstop for a bug that burns time without charging fuel.
- **XML.** A `<!DOCTYPE` with an internal subset or entity declarations is refused (no entity
  expansion, no external fetch). `quick-xml` does not expand custom entities; the refusal makes
  that a stated property rather than a library default.
- **Archives.** Entry names are keys, never paths: nothing from an archive is written to disk under
  its own name, so path traversal has nothing to act on. Encrypted entries are refused.
- **PDF** (opt-in): page count and object count count against `nodes` and `fuel`.
- Defaults are generous enough for the five Bibles and a multi-year chat export; the limit test
  suite (§5.1) runs every reader against a crafted over-limit input for each cap.

#### 3.9.2 Prompt injection (extends SMYSL-2.1)

Text is **data inside prompts**. SMYSL-2.1 covers today's `ingest`. This RFC adds:

| surface | measure |
|---|---|
| text windows | each node rendered as `[locator]` + text inside the existing `FENCE`; a node containing the fence string is rendered with the marker neutralised (`<<<SMYSL-INPUT>>>` → `<<<SMYSL-INPUT\u{200B}>>>` in the prompt only, never in stored text) and `SMY-W443` |
| ledger digest | a second fence, `<<<SMYSL-DIGEST>>>`, with the same "data, never instruction" preamble; digest gists are untrusted (they were extracted from untrusted text) |
| linker and summary prompts | both members of a pair, and every child of a summary, inside fences |
| answers | json-ast path preferred; model-authored relation kinds stay restricted (`json_ast.rs`); references limited to digest and batch (`E442`); `measured` refused by rule T; dates, speakers, spans, `lang` and `manifest` are written by the tool, never taken from the answer — a model field named `source`, `lang`, `span`, `published`, `observed` or `text:speaker` in a text-ingest answer is dropped with `SMY-W444` |
| staging | nothing reaches a log without rule S |

A red-team fixture set (`fixtures/library/redteam/`) holds texts with embedded instructions, fake
fences, fake JSON answers, requests to cite store uids, to set `measured`, to write a speaker or a
date, and bidirectional-override characters. Offline tests drive them with the `Scripted`
provider playing an obedient model and assert the guard (§5.2). A canary test runs under
`make live-ollama`.

#### 3.9.3 Concurrency

- **One writer per shard.** `log/<shard>.lock` is created with `OpenOptions::create_new(true)`
  and holds pid, host and command. A second writer fails with `SMY-E445` naming the holder.
  `--break-lock` removes a lock explicitly; nothing removes one by timeout. No new dependency;
  advisory OS locks are OQ-36.
- **Library-wide operations** (`text add`, `text append`, `text redact`, index rebuild) take
  `<library>/lock` first, then shard locks in name order, so two such operations cannot deadlock.
- **Readers take no lock.** Logs are append-only and `from_cbor_seq` stops at a truncated tail;
  objects are immutable, written to `objects/tmp/` and renamed into place (an existing target is
  success: same name, same content), and verified by hash when read (`SMY-E446` on mismatch);
  redb serves readers from read transactions.
- **Ingest** stages per shard (`<library>/stage/<shard>/staged.smy`), so ingests of different
  expressions run in parallel. SMYSL-2.8 revisits this for the redb unit store.

---

## 4. Implementation plan

### 4.1 Crates and modules

**New crate `crates/smysl-text`** (pure core; readers and redb behind features):

```
crates/smysl-text/
  Cargo.toml
  data/abbr/{en,ru,es,fr,de}.txt     abbreviation exception lists (segmenter data)
  data/versification/               mapping tables, licence per OQ-4 (TX-P4)
  src/
    lib.rs          re-exports; Library handle
    norm.rs         Normalised newtype: UTF-8, BOM, LF, NFC
    ids.rs          re-export of smysl_core::{Tid, Mid, Did, Rdid}; Tid::of(&Normalised); parsing helpers
    manifest.rs     ManifestBuilder; licence::PERMISSIVE; carry gate (E402); head/fork per alias (W418)
    part.rs         part::Policy, grouping nodes into parts, PartText construction and verification
    reading.rs      segment table codec (record 18 key 2), raw metadata (key 3), rdid
    objects.rs      ObjectStore: objects/{t3,r3}/ab/cd…; tmp + rename; verify on read (E446)
    readers/
      mod.rs        trait Reader, ReaderId, Input, Params, ReadOutput, registry by feature
      txt.rs  md.rs  usfm.rs  osis.rs  zefania.rs  json.rs              (TX-P1)
      telegram.rs  slack.rs  whatsapp.rs  mbox.rs  epub.rs  html.rs  pdf.rs  (TX-P2)
      zip.rs        archive access under Budget (Slack, EPUB)
    structure.rs    arena tree, structure hash, resolve(locator)
    locator.rs      draft 3 Appendix B grammar; canonical Display
    segment.rs      seg-uax29+abbr/1
    lang.rs         lingua with 5 languages; manifest/segment detection
    analyze.rs      per-language chains; implements smysl_retrieve::Analyze
    provenance.rs   stamp(): the D-1 copy set onto a UnitCoreBuilder
    time/
      mod.rs        Axis, Instant, Bound, Interval
      edtf.rs       re-export of smysl_core::edtf; to_interval(); calendars
      calendar.rs   Julian ↔ Gregorian via JDN
      constraints.rs free constraints (D-3)
      stn.rs        stratified simple temporal network
      engine.rs     effective(): rule E, locks, why-chains, derived contentions
    proposition/
      mod.rs        Policy, Class, classes()
      identical.rs  identical-span engine
      lexical.rs    lexical engine (BM25 via smysl-retrieve)
      anchored.rs   anchored engine (align table)
    index/          feature substrate-redb
      mod.rs        Index handle, table definitions (§14.2 subset), rebuild, on_append
      postings.rs   persistent postings; implements smysl_retrieve::Postings
      journal.rs    ingest journal (operational)
    lock.rs         library and shard lock files (E445)
    limits.rs       Budget, Caps, defaults, fuel accounting (E440)
```

Dependencies: `smysl-core`, `smysl-graph`, `smysl-retrieve`, `unicode-segmentation`, `caseless`,
`rust-stemmers`, `lingua` (`default-features = false`, five language features); optional
`quick-xml`, `serde_json`, `pulldown-cmark`, `redb`, `mail-parser`, `rbook`, `html2text`,
`pdf-extract`, a zip reader (unverified choice, must be pure Rust). No `smysl-check` dependency, so
`smysl-check` may depend on `smysl-text` without a cycle.

**Dependency direction** (no cycles): `smysl-core` ← `smysl-graph` ← `smysl-retrieve` ←
`smysl-text` ← {`smysl-check` (optional, feature `text`), `smysl-ingest` (feature `text`)} ← facade.
Putting analyzers in `smysl-text` and the `Analyze`/`Postings` seams in `smysl-retrieve` is what
keeps `retrieve` below `text`.

**New code in existing crates:**

| crate | new files |
|---|---|
| `smysl-core` | `src/edtf.rs`; `src/types/library.rs` (Manifest, PartEntry, PartText, PartReading, Dating, DatingTarget, DatingValue, Allen, Axis, Redaction, Carry, Calendar, ParentKind) |
| `smysl-graph` | `src/store/library.rs` (by-part maps, manifests and heads, datings, redaction filter) |
| `smysl-check` | `src/passes/library.rs`, `src/passes/time.rs` |
| `smysl-retrieve` | `src/analyze.rs` (seam), `src/postings.rs` (seam + in-memory impl) |
| `smysl-ingest` | `src/text/mod.rs`, `src/text/window.rs`, `src/text/attach.rs`, `src/text/digest.rs`, `src/text/journal.rs`, `src/link.rs`, `src/summarise.rs`, `src/same_as.rs` (staging of proposals) |

### 4.2 Public API (sketch level)

`smysl-core` (records and ids; field lists follow SMYSL-2.3 and are abbreviated here):

```rust
// src/ids.rs — same shape as Uid; text forms per SMYSL-2.3 (t3:, m3:; did/rdid forms there).
pub struct Tid([u8; 32]);   pub struct Mid([u8; 32]);
pub struct Did([u8; 32]);   pub struct Rdid([u8; 32]);
impl Tid {
    pub const PREFIX: &'static str = "t3:";
    pub fn from_normalised_bytes(b: &[u8]) -> Tid;      // BLAKE3(0x0F ‖ b); caller guarantees NFC/LF
    pub fn parse(s: &str) -> Result<Tid, IntegrityError>;
    pub fn canonical(&self) -> String;
    pub const fn as_uid(&self) -> Uid;                  // for withdrawal/commit keys and contention positions
}
// Mid, Did, Rdid: same, domain bytes 0x0E, 0x11, 0x12, computed over the record body.

// src/types/library.rs
#[non_exhaustive] pub struct Manifest { pub alias: String, pub parts: Vec<PartEntry>, pub lang: LangTag,
    pub reader: String, pub licence: String, pub carry: Carry, pub title: Option<String>,
    pub creators: Vec<String>, pub published: Option<String>, pub identifiers: BTreeMap<String, String>,
    pub origin: Option<SourceRef>, pub parent: Option<(Mid, ParentKind)>, pub supersedes: Option<Mid>,
    pub versification: Option<String>, pub lossy: bool, pub raw: Option<Vec<u8>>,
    pub part_policy: String, pub calendar: Option<Calendar>, pub extra: Extra }
impl Manifest { pub fn mid(&self) -> Mid; }
#[non_exhaustive] pub struct PartEntry { pub tid: Tid, pub length: u64, pub structure: [u8; 32],
    pub rdid: Rdid, pub lang: Option<LangTag>, pub extra: Extra }
#[non_exhaustive] pub struct PartText { pub tid: Tid, pub text: Vec<u8>, pub extra: Extra }
impl PartText { pub fn verify(&self) -> bool; }          // tid == BLAKE3(0x0F ‖ text)
#[non_exhaustive] pub struct PartReading { pub tid: Tid, pub reader: String,
    pub segments: Vec<u8>,                               // canonical CBOR table, opaque at this level
    pub raw: Option<Vec<u8>>, pub extra: Extra }
impl PartReading { pub fn rdid(&self) -> Rdid; }
#[non_exhaustive] pub struct Dating { pub target: DatingTarget, pub axis: Axis, pub value: DatingValue,
    pub basis: Option<Uid>, pub agent: AgentId, pub ts: Hlc, pub extra: Extra }
impl Dating { pub fn did(&self) -> Did; }
#[non_exhaustive] pub enum DatingTarget { Unit(Uid), Part(Tid), Manifest(Mid),
    Window { part: Tid, from_ms: u64, to_ms: u64 } }
#[non_exhaustive] pub enum DatingValue { Absolute(String), OffsetMs(i64), Relative(Allen, DatingTarget) }
#[non_exhaustive] pub struct Redaction { pub tid: Tid, pub agent: AgentId, pub ts: Hlc,
    pub reason: Option<Uid>, pub extra: Extra }

// src/types/record.rs — new variants, codes 14, 15, 17, 18, 19; 16 stays Unknown (reserved).
pub enum Record { /* … */ Manifest(Manifest), PartText(PartText), Dating(Dating),
    PartReading(PartReading), Redaction(Redaction), Unknown { code: u64, payload: Vec<u8> } }

// src/edtf.rs
pub struct Edtf { /* AST: Date | DateTime | Interval, qualifiers, unspecified digits */ }
pub fn parse(s: &str) -> Result<Edtf, EdtfError>;      // level 1; EdtfError carries the byte offset
```

`smysl-graph`:

```rust
impl Store {
    pub fn units_from_part(&self, tid: &Tid) -> &BTreeSet<Uid>;          // replaces the prefix scan for t3: refs
    pub fn units_under_manifest(&self, mid: &Mid) -> &BTreeSet<Uid>;
    pub fn manifests(&self) -> impl Iterator<Item = (&Mid, &Manifest)>;
    pub fn manifest(&self, mid: &Mid) -> Option<&Manifest>;
    pub fn heads(&self, alias: &str) -> Vec<Mid>;                       // >1 means fork (W418)
    pub fn datings(&self) -> impl Iterator<Item = (&Did, &Dating)>;
    pub fn datings_on(&self, target: &DatingTarget) -> Vec<&Did>;
    pub fn redactions(&self) -> impl Iterator<Item = (&Tid, &BTreeSet<Redaction>)>;
    pub fn is_redacted(&self, tid: &Tid) -> bool;
    pub fn part_text(&self, tid: &Tid) -> Option<&PartText>;            // only for 15 held in a log
    pub fn rewrite_redacted(&mut self) -> Result<RewriteReport, Error>;  // physical erasure, §4.3.2
}
```

`smysl-text`:

```rust
pub struct Library { /* root path, catalog + shard logs opened into one Store, ObjectStore, Option<Index> */ }
impl Library {
    pub fn open(root: &Path, opts: OpenOptions) -> Result<Library, LibError>;
    pub fn store(&self) -> &Store;
    pub fn add(&mut self, input: &Input<'_>, spec: &AddSpec, caps: &Caps) -> Result<Added, LibError>;
    pub fn append(&mut self, alias: &str, input: &Input<'_>, caps: &Caps) -> Result<Added, LibError>;
    pub fn part(&self, tid: &Tid) -> Result<PartText, LibError>;                  // verified (E446)
    pub fn reading(&self, rdid: &Rdid) -> Result<Reading, LibError>;
    pub fn structure(&self, mid: &Mid, tid: &Tid) -> Result<Structure, LibError>; // checks structure hash (E401)
    pub fn resolve(&self, mid: &Mid, loc: &Locator) -> Option<(Tid, Range<u64>)>;
    pub fn passage(&self, alias_or_mid: &str, range: &LocatorRange) -> Result<Passage, LibError>;
    pub fn redact(&mut self, tid: &Tid, agent: &AgentId, ts: Hlc, reason: Option<Uid>) -> Result<(), LibError>;
}
pub struct AddSpec { pub reader: ReaderId, pub params: Params, pub alias: String, pub lang: Option<LangTag>,
    pub licence: String, pub carry: Carry, pub pseudonymise: bool, pub policy: part::Policy }

pub mod time {
    pub fn effective(store: &Store, lib: Option<&Library>, axis: Axis, scope: &Scope) -> Effective;
    pub struct Effective { pub by_unit: BTreeMap<Uid, Eff>, pub undated: usize, pub contentions: Vec<Contention> }
    pub enum Eff { Interval { lo: Bound, hi: Bound, status: Status, why: Why }, Contested { why: Why } }
}
pub mod proposition {
    pub enum Policy { Strict, Component, Attested(u32) }
    pub fn classes(store: &Store, policy: Policy, scope: &BTreeSet<Uid>) -> Vec<Class>;  // pure
    pub trait Engine { fn id(&self) -> &'static str; fn propose(&self, store: &Store, scope: &BTreeSet<Uid>) -> Vec<(Uid, Uid)>; }
    pub struct IdenticalSpan; pub struct Lexical { pub k: usize } pub struct Anchored<'l> { pub lib: &'l Library, pub scheme: String }
}
pub mod provenance {
    pub struct Attach<'m> { pub manifest: &'m Manifest, pub tid: Tid, pub span: Option<Range<u64>>,
        pub locator: Option<Locator>, pub observed: Option<u64>, pub lang: LangTag, pub speaker: Option<Uid> }
    pub fn stamp(b: UnitCoreBuilder, a: &Attach<'_>) -> Result<UnitCoreBuilder, StampError>; // copy + omission rules
}
pub mod limits {
    pub struct Caps { pub input_bytes: u64, pub decompressed_bytes: u64, pub entry_bytes: u64, pub entries: u32,
        pub ratio: u32, pub nesting: u32, pub nodes: u64, pub part_bytes: u64, pub raw_bytes: (u64, u64), pub fuel: u64 }
    pub struct Budget { /* remaining fuel and counters */ }
    impl Budget { pub fn charge(&mut self, what: Cost, n: u64) -> Result<(), LimitExceeded>; }
}
```

`smysl-retrieve`:

```rust
pub trait Analyze { fn id(&self) -> &str; fn terms(&self, text: &str) -> Vec<String>; }
impl Analyze for Tokenizer { /* "smysl/plain/1" or "smysl/fold-en/1" */ }
pub trait Postings { fn df(&self, term: &str) -> u32; fn postings(&self, term: &str) -> Vec<(Uid, u32)>;
                     fn doc_len(&self, uid: &Uid) -> Option<u32>; fn docs(&self) -> u32; }
impl Bm25 { pub fn index_analyzed(store: &Store, a: &dyn Analyze) -> Bm25;
            pub fn over(postings: &dyn Postings, a: &dyn Analyze) -> Bm25View<'_>; }
```

`smysl-ingest` (feature `text`):

```rust
pub struct TextIngest<'l> { pub lib: &'l Library, pub target: TextTarget, pub lang_policy: LangPolicy,
    pub reextract: bool, pub digest_budget: u32 }
pub enum LangPolicy { Source, Pivot(LangTag) }
impl Ingestor { pub fn ingest_text(&self, t: &TextIngest<'_>, attest: &dyn Attesting) -> Result<TextReport, ProviderError>; }
pub mod text { pub fn windows(s: &Structure, level: Level, budget: u32, est: &dyn Estimate) -> Vec<TextWindow>; }
pub fn link(&self, scope: &BTreeSet<Uid>, budget: LinkBudget) -> Result<Staged, ProviderError>;     // on Ingestor
pub fn summarise(&self, levels: &[Level], scope: &BTreeSet<Uid>) -> Result<Staged, ProviderError>;
```

### 4.3 Changes to existing crates, file by file

#### 4.3.1 `smysl-core`

| file | change | phase |
|---|---|---|
| `src/types/record.rs` | variants and `mod code` constants `MANIFEST = 14`, `PART_TEXT = 15`, `DATING = 17`, `PART_READING = 18`, `REDACTION = 19`; `KNOWN` gains them; **16 is not added** and keeps decoding to `Unknown`; `type_name`s `manifest`, `parttext`, `dating`, `partreading`, `redaction`; `is_hashed` stays `Unit` only (the other ids are computed on demand and verified where SMYSL-2.3 says) | P1 (14, 15, 18), P2 (19), P3 (17) |
| `src/cbor/keys.rs` | modules `manifest` (0–18), `part_entry` (0–4), `part_text` (0–1), `part_reading` (0–3), `dating` (0–5), `redaction` (0–3); `unit::LANG = 9`, `unit::HIGHEST = LANG`; `source::PUBLISHED = 4`, `SPAN = 5`, `MANIFEST = 6`, `source::HIGHEST = MANIFEST`; every new table added to `key_tables_are_contiguous_from_zero` | P1–P5 |
| `src/cbor/envelope.rs` | one encoder and decoder per new record, written in the existing `read_map` style so unknown keys land in `extra`; `enc_source`/`dec_source` gain keys 4–6, written only when present (so every existing source encodes to the bytes it always did); `dec_unit` reads key 9; `PartText` decode does **not** verify the tid (a bad record must not stop `Store::open`, F-12's lesson) — `Store` does | P1–P5 |
| `src/types/unit.rs` | `UnitCore.lang: Option<LangTag>`, `UnitCoreBuilder::lang()`; absent `lang` encodes as today | P5 |
| `src/types/epistemics.rs` | `SourceRef.published: Option<String>` (validated by `edtf::parse` in the builder), `span: Option<(u64, u64)>` (`start < end` enforced), `manifest: Option<Mid>`; `observed_at`-style builders `published()`, `span()`, `manifest()` | P3 (published), P5 (span, manifest) |
| `src/edtf.rs` (new) | level-1 parser and AST; conformance cases in `fixtures/library/edtf/` | P3 |
| `src/ids.rs`, `src/hash.rs` | `Tid`, `Mid`, `Did`, `Rdid` with domain-byte hashing; `LangTag` checked lowercase NFC as FC-1 requires | P1 |
| `src/surface/lex.rs` | reserved words `manifest`, `date`, `redact` → `ManifestStart`, `DateStart`, `RedactStart`, added after `CommitStart` (the enum's ordering comment applies) | P1, P3, P2 |
| `src/surface/parse.rs` | `@manifest`, `@date`, `@redact` records; `source()` reads `published`, `span`, `manifest` (D-2: inside `source { }`), rejecting a malformed value with `E410`/`E403`/`E001` (SMYSL-2.1 has already made unknown keys an error); unit-level bare `lang:` becomes core key 9 from the release that ships FC-1 (D-2); a payload key named lang must be quoted | P1–P5 |
| `src/surface/write.rs` | writers for the three forms; records 15 and 18 have no surface form and are counted among records surface cannot hold; `manifest_has_surface_form` false when `raw`/unknown keys are present | P1–P3 |
| `src/types/annex.rs`, `src/types/lifecycle.rs` | `DetectionKind` values 4 and 5 for time contentions (SMYSL-2.3 A-8.2), used only in `ContentionId::derive`: never written to record 6 | P3 |
| `src/diag.rs` | draft-3 codes used by this RFC (`E401–E404`, `W405`, `W406`, `E410–W414`, `W418`, `W419`) and `E440–W451`, in a new group `Library`; `the_registry_is_the_size_it_is_meant_to_be` updated per phase | P1–P7 |

#### 4.3.2 `smysl-graph` (`src/store/mod.rs`, new `src/store/library.rs`)

- **New state.** `manifests: BTreeMap<Mid, Manifest>`, `by_alias: BTreeMap<String, BTreeSet<Mid>>`,
  `superseded: BTreeSet<Mid>`, `datings: BTreeMap<Did, Dating>`, `datings_by_target`,
  `redactions: BTreeMap<Tid, BTreeSet<Redaction>>`, `part_texts`/`readings` for 15/18 held in a log,
  `by_tid: BTreeMap<Tid, BTreeSet<Uid>>`, `by_mid: BTreeMap<Mid, BTreeSet<Uid>>`. All derived in
  `absorb`, all order-independent (maps of sets).
- **By-part lookups.** `absorb` parses `source.reference` once per new unit: if it starts with
  `t3:` and the next 52 characters parse as a `Tid`, the uid goes into `by_tid`; `source.manifest`
  feeds `by_mid`. `units_with_source_prefix` is unchanged (it is public contract since 1.5); the
  draft-3 warning about short prefixes is answered by the new exact lookups.
- **Redaction filtering on append and merge (rule Z).** In `append`, before the duplicate check:
  1. collect tids redacted by the store **or by this batch** (records 19 in `records`);
  2. drop from the batch every `PartText` / `PartReading` whose tid is in that set, counting them
     in `AppendReport.redacted` (new field; `AppendReport` is already `#[non_exhaustive]`, so
     adding it is not a break);
  3. after `absorb`, remove in-memory 15/18 for newly redacted tids from `part_texts`/`readings`.
  Because `merge` is `append` plus detection (§2.2), merge inherits the filter with no further
  change. The filtered union is commutative, associative and idempotent: the redaction set only
  grows, and the filter is applied to the union, whatever the order.
- **Physical erasure.** A log that already holds a record 15 for a tid redacted later still has the
  bytes on disk. `Store::rewrite_redacted` writes a new log without them to `<log>.tmp`, fsyncs,
  renames over the log, resets `log_hasher`/`log_len`, and leaves the sidecar to be rebuilt
  (`W110`). It drops the hashes of the removed records from `record_hashes` so they are not
  "already present"; they are refused anyway by step 2. `text redact` calls it under the shard
  lock. A library's own logs never hold 15/18, so in the common case this is a no-op (OQ-39).
- **Heads and forks.** `heads(alias)` = mids of that alias not named by any other manifest's
  `supersedes`; more than one is `W418` (reported by `check`, not recorded).
- **Adjacency cost.** `absorb` currently calls `rebuild_adjacency()` whatever arrived. It gains a
  flag: rebuild only if the batch held a unit, relation, withdrawal or attestation (pending
  attestations are retried there). A `text add` that appends one manifest, or a `date set`, then
  costs `O(batch)` instead of the 1336 µs/record single-append figure of 1.8. Verified by a test
  that asserts `adjacency` is untouched after a manifest-only append, and by the 1.8 timing harness.
- **Withdrawals and commitments naming a did** need no change: both maps are keyed by `Uid` for
  targets "whether or not [they have] arrived", and `Did::as_uid` supplies the key.
- **`emit`** is SMYSL-2.1's (F-16). This RFC adds the arms: 14 and 19 always travel; 17 travels
  with its target; 15/18 only when the manifest allows `carry: text` (`E402` otherwise, raised by
  the facade before calling `emit`).

#### 4.3.3 `smysl-check`

New passes appended after `CommitmentSupport`, as the enum's comment requires:

| pass | number | codes | needs |
|---|---|---|---|
| `Library` | 12 | `E403` malformed tid/mid/did/rdid in a record or `source.manifest`; `E404` span past the part's length (length from the named manifest's part entry, so no object access); `W405` locator disagrees with span (needs a `PartResolver`); `W406` holder/speaker uid not in `deps`; `W418` two heads; `E446` record 15 held in the log whose text does not hash to its tid | store; resolver optional |
| `Time` | 13 | `E410` malformed EDTF (in `published`, manifest key 8, dating values, `text:when`); `W411` `observed` outside the `published` interval; `W412`/`W413` from the engine, reported as derived contentions; `W449` | `smysl-text` (feature `text`) |

- `CheckOptions` gains `parts: Option<Arc<dyn PartResolver + Send + Sync>>`; `smysl-text`
  implements `PartResolver` for `Library`. Without a resolver, `W405` is skipped and the report says
  so (as `Pass::IMPLEMENTED` does for passes this build does not run).
- `ConformanceClass::Library` (`"C-Library"`, D-8): branches like `Merge`; forbids `E401`, `E403`,
  `E404`, `E446` and a redaction violation; does not subsume `Produce`. `ALL` gains it at the end.
- Fixture tree `fixtures/library/check/` in the existing `.smy` + `.expected` format.

#### 4.3.4 `smysl-ingest`

| file | change |
|---|---|
| `Cargo.toml` | feature `text = ["dep:smysl-text", "dep:smysl-pack"]` |
| `src/text/window.rs` (new) | windows filled with whole structure nodes at the chosen level up to `budget` (from `Window::for_context`, whose arithmetic is reused), never splitting a node; one-node overlap; `TextWindow { nodes, owned: Range<usize>, overlap: Option<usize>, text, map }` where `map` takes window byte offsets to `(tid, part offset)`. A node larger than the budget is split at sentence boundaries (`W447`), which is the only place a node is split. Windows may cross parts; a node never does. Chats: windows follow reply threads. |
| `src/text/attach.rs` (new) | span attachment: search order (1) the node the model named (`ingest:node`, a locator), (2) the owned range of the part holding it, (3) owned ranges of other parts in the window, (4) overlap nodes; each scope searched with `support_span` per contiguous same-part run (a span never crosses a part); `Present` beats `Loose` within a scope; the first scope with a match wins. More than one occurrence in the named node → `W419`. Then `provenance::stamp`. Then `ingest:quote`/`ingest:node` removed from the payload (§3.4). `Absent` after repair → §3.4 (`W441`). |
| `src/text/digest.rs` (new) | ledger digest via `smysl_pack::pack`; rendering with `l/n` labels; returns `(text, labels, allowed: BTreeSet<Uid>, hash)` |
| `src/json_ast.rs` | `convert_scoped(raw, &Scope)`: `resolve` unchanged, then a resolved uid outside `Scope.allowed ∪ batch` is `E442`; text answers drop tool-owned fields (`W444`); `text:holder`/`text:speaker` labels resolved to canonical uids (§3.4). The surface path (`repair::convert_labelled`) gets the same scope check after parsing. |
| `src/repair.rs` | `needs_repair` treats `E442` like `E060`; a text-ingest variant of `salvage` that keeps a unit whose only error is `E307`, applying §3.4's cap. `UNIT_LOCAL` is unchanged for prose ingest. |
| `src/prompt.rs` | templates `ingest.text.json` / `ingest.text.surface`: node-per-line rendering inside `FENCE`, the digest inside `DIGEST_FENCE`, the language instruction for `source` / `pivot:L`, the `ingest:node` field; `neutralise_fences(input) -> (String, bool)` (`W443`). |
| `src/recipe.rs` | `Conditions` gains `lang_policy`, `normaliser` (V1/V2, from SMYSL-2.1), `estimator`, `window_level`, `digest_policy` (budget and ranking id), `segmenter` and `reader` ids. Pushed in `push_shared`, so `family()` still drops only provider and model. |
| `src/text/journal.rs` (new) | key `(alias, tid, node range, recipe)` → `{ staged batch hash, digest hash, tokens in/out, quote verdicts }`; stored in the redb `journal` table (or `<library>/journal.cbor` without `substrate-redb`). Rerun skips completed keys; within one expression a part extracted under an earlier manifest is skipped; `--reextract` ignores the journal for the head manifest. Resume after kill loses at most the batch in flight, because a key is written only after its batch is staged. |
| `src/lib.rs` | `Ingestor::ingest_text`, `link`, `summarise`; `chunk` stays `pub(crate)`. |
| `src/same_as.rs` (new) | turns engine proposals into staged `x.text/same-as` relations with tool attestations; `lexical` proposals above the recipe's threshold, or judged (OQ-44). |

Pivot policy: a second pass over `source`-policy units of the scope, one unit per source unit,
linked by `x.text/translates`, with the source unit in `deps` so rule M caps it.

#### 4.3.5 `smysl-retrieve`

- `src/analyze.rs`: the `Analyze` seam; `Tokenizer` implements it with ids `smysl/plain/1` and
  `smysl/fold-en/1`. `Bm25::index_with` keeps its signature and delegates.
- `src/postings.rs`: the `Postings` seam and an in-memory implementation built by today's path;
  `Bm25` can score over any `Postings`, which is the hook the redb table in `smysl-text::index`
  implements (persistent, per partition = manifest × language, incremental on append).
- Per-unit analyzer routing: a closure `Fn(&Uid) -> &dyn Analyze` chosen by unit `lang`, falling
  back to the view's; `find --lang L --analyzer A` overrides.

#### 4.3.6 Facade (`src/lib.rs`)

Feature `text`; re-exports `smysl::text::{Library, Manifest, Part, Tid, Mid, Reader, Structure,
Locator, Segmenter, Analyzer}`, `smysl::time::{Edtf, Interval, Dating, Axis, effective}`,
`smysl::proposition::{Policy, classes, propose}`. The CLI reaches them only through the facade
(rule A).

### 4.4 CLI

New rows in `COMMANDS` (`src/main.rs`), appended in table order:

```rust
Cmd { name: "text",      about: "Library: add, append, list, show, align, redact texts",  purity: Purity::Pure,  phase: "TX-P1" },
Cmd { name: "date",      about: "Write datings; show effective time and why",              purity: Purity::Pure,  phase: "TX-P3" },
Cmd { name: "link",      about: "Find far relations between units; staged",                purity: Purity::Model, phase: "TX-P7" },
Cmd { name: "summarise", about: "Grounded summary tree over a scope; staged",              purity: Purity::Model, phase: "TX-P7" },
Cmd { name: "same-as",   about: "Propose same-as edges; derive classes",                   purity: Purity::Mixed, phase: "TX-P7" },
```

| command | flags | purity | phase |
|---|---|---|---|
| `text add <file>` | `--reader R --alias A [--lang L] --licence SPDX [--carry none\|ref\|text] [--pseudonymise] [--part-policy P] [--param k=v]… [--max-* N] [--timeout S]` | pure | P1 (`--pseudonymise` P2) |
| `text append <alias> <file>` | as `add`, reader and parameters taken from the head manifest | pure | P2 |
| `text ls` | `[--alias A] [--forks]` | pure | P1 |
| `text show <alias\|mid>#<locator>[-<locator>]` | `[--raw] [--segments]` | pure | P1 |
| `text align <a> <b>` | `--scheme S` | pure | P4 |
| `text redact <tid>` | `[--reason UID]` | pure | P2 |
| `date set <target>` | `--axis said\|composed\|about --value EDTF\|offset:MS\|<allen>:<target> [--basis UID]` | pure | P3 |
| `date order <a> before <b>` | `[--basis UID]` (sugar for a relative `date set`) | pure | P3 |
| `date show <target>` | `[--axis A] [--why]` | pure | P3 |
| `ingest --text <alias\|mid>[#range]` | `--lang-policy source\|pivot:L [--reextract] [--digest-budget N] [--window-level L]` | model | P5 |
| `link` | `--scope <alias\|mid…> [--budget N]` (an `sq` scope once SMYSL-2.5 lands) | model | P7 |
| `summarise` | `--scope … --levels chapter,book` | model | P7 |
| `same-as propose` | `--engine identical-span\|lexical\|anchored [--scope …]` (`semantic` reserved for SMYSL-2.5) | pure per engine; mixed as a command | P7 |
| `same-as classes` | `--policy strict\|component\|attested:N [--scope …]` | pure | P7 |
| `find` | `--lang L --analyzer A` added | pure | P4 |
| `check` | `--library` resolves parts for `W405` | pure | P1 |

- **Library path.** No new global flag: `-s/--store` accepts a library directory, recognised by
  a `<dir>/LIBRARY` marker file holding the layout version. A plain file path keeps today's
  meaning.
- **Gates touched.** `tests/cli-surface.txt` regenerated (`make cli-surface`, after adding the
  names to the Makefile's `COMMAND_NAMES`);
  `command_table_matches_section_23` and `tests/dispatch.rs::every_command_dispatches` gain the
  rows; `xtask determinism` registers `text add` (reader output bytes), `text show`, `date show`,
  `same-as propose --engine identical-span`, `same-as classes` — the `ru_RU.UTF-8` locale in its
  matrix is exactly the one case folding would betray.
- **Exit codes** reuse today's (`Staged`, `StagedWithCorrections` for staging commands).

### 4.5 Features, dependencies, purity gate, C toolchain

`crates/smysl-text/Cargo.toml`:

```toml
[features]
default = []                                         # the core the purity gate checks
reader-txt = []   reader-md = ["dep:pulldown-cmark"]   reader-usfm = []
reader-osis = ["dep:quick-xml"]   reader-zefania = ["dep:quick-xml"]
reader-json = ["dep:serde_json"]  reader-telegram = ["dep:serde_json"]
reader-slack = ["dep:serde_json", "zip"]   reader-whatsapp = []
reader-mbox = ["dep:mail-parser"]  reader-epub = ["dep:rbook", "zip"]
reader-html = ["dep:html2text"]    reader-pdf = ["dep:pdf-extract"]
substrate-redb = ["dep:redb"]
```

Root `Cargo.toml`: `text = ["dep:smysl-text", "smysl-check/text", "smysl-retrieve/…"]`;
`cli` enables `text` and `reader-{txt,md,usfm,osis,zefania,json,telegram,slack,whatsapp}` and
`substrate-redb` (draft 3 §18); `epub`, `html`, `mbox`, `pdf` opt-in.

- **Purity gate.** `xtask/src/purity.rs` checks each `PURE_CRATES` entry with
  `cargo tree -p <crate> -e normal` at **default** features. `smysl-text` joins the list with
  `default = []`, so its checked tree has no `serde_json`; the readers needing it are outside
  the gate, as providers are. `smysl-text` must also pass the `FORBIDDEN_SYMBOLS` grep, which is
  why the wall-time watchdog lives in `src/main.rs`, not in the crate.
- **`--no-default-features` facade tree** stays free of `serde_json`: `text` is not default for
  the library (only through `cli`).
- **No C toolchain.** Research builds cited in draft 3 §18 found no `cc` for `lingua` (5
  languages), `redb`, `pdf-extract`, `mail-parser`, `rbook`, `html2text`. Not re-verified at this
  commit; TX-P1's exit adds `cargo tree -e normal -i cc` for the `cli` feature set to `make
  crate-features` so the claim is checked rather than cited.
- **MSRV — OQ-40, answered by measurement in 1.10.0.** The declared `rust-version = "1.79"` was
  false in every selection and had been for releases, and the three crates named here were not
  why. Measured floors, each confirmed by compiling at it and at the version below:

  | selection | floor | set by |
  |---|---|---|
  | the nine pure-path crates, `--no-default-features` | **1.85** | `constant_time_eq` 0.4.2 via `blake3` — manifest edition 2024, which a 1.79 Cargo cannot parse |
  | `smysl-provider`, `smysl-ingest` | **1.86** | `icu_*` 2.2 via `idna_adapter` ← `idna` ← `url` ← `ureq` |
  | `smysl-tui`, and the facade with `tui` | **1.88** | `instability` 0.3.12 and `darling` 0.23 via `ratatui` |

  Those are now declared: the workspace base is 1.85 and the four crates above it carry their
  own, because Cargo has no per-feature `rust-version` and a package's number must therefore be
  its maximum. `redb` 4.3's 1.90 becomes an ordinary bump when `store-redb` lands, not a policy
  question — and OQ-66's alternative, *keep 1.79 for builds without `store-redb`*, was never
  available, because no build reached 1.79.

  **`make msrv`** (`scripts/verify-msrv.py`) compares every crate's declared floor against the
  maximum its transitive non-dev, non-platform-gated dependencies require, and fails in **both**
  directions: a floor below what the dependencies need is the false claim, and a floor above it
  is one nobody has a reason for, which rots into the first. The CI job runs it and then compiles
  the base tier at 1.85, because the script cannot see our own source using a newer language
  feature than any dependency needs.
- **Crate size.** `lingua` language models are large (unverified figure); five features only, and
  the per-language model crates are downloaded only with `text`.

---

## 5. Tests, fixtures and harnesses

### 5.1 Unit tests (in-crate)

| area | tests |
|---|---|
| ids and records | each new record round-trips byte-identically through `to_cbor`/`from_cbor`; unknown keys preserved in `extra`; key tables contiguous; code 16 decodes to `Unknown` with `W014`; a source with none of keys 4–6 and a unit without key 9 encode to the bytes they had at `d25ec9e` (`fixtures/golden/cbor/corpus.cbor`, read by `crates/smysl-core/tests/roundtrip.rs`) |
| normalisation | BOM, CRLF, lone CR, NFD input, invalid UTF-8 refused; `Tid::of` refuses a non-`Normalised` value at compile time |
| EDTF | every level-0/level-1 production accepted and printed back; rejects (`E410`) with byte offsets; interval endpoints for each precision; Julian conversion at the 1582 and 1918 boundaries and for a negative year |
| locators | Appendix B grammar both ways; canonical form round trip; ranges; JSON Pointer escaping (`~0`, `~1`) |
| segmenter | per-language abbreviation cases from draft 3 §5.3 ("Mr.", "Dr.", "M." before a capital; "т.е.", "z.B.", "p.m." before lowercase); list hash pinned to the segmenter id |
| analyzers | `книгами → книг`, `continuellement → continuel`, `Häuser → Haus`; stems emitted beside, never instead of, the surface term; `aujourd’hui` one token under the fr chain |
| structure | structure hash stable under re-read; `resolve` for every locator a reader emitted returns that node's range |
| limits | one crafted input per cap per reader (deep JSON, XML with an entity declaration, zip with a 1000:1 entry, 100,001 entries, a part over the hard ceiling, fuel exhaustion on a pathological WhatsApp line); each fails with `E440` naming the cap and leaves `objects/` and the logs unchanged |
| objects | tmp + rename; existing target treated as success; a flipped byte yields `E446` on read |
| locks | second writer gets `E445` with the holder; `--break-lock`; library lock then shard locks in name order |
| store | manifest-only append leaves `adjacency` untouched; `by_tid`/`by_mid` agree with `units_with_source_prefix("t3:<full tid>#")`; heads and `W418` |
| ingest | windows never split a node except with `W447`; owned ranges partition the text; span attachment order (named node, owned, other owned, overlap); `W419` on Psalm 136's refrain; `Absent` → no span, cap per §3.4, `W441`; `ingest:quote` removed when a span is attached; tool-owned fields dropped (`W444`); `E442` on a uid outside the digest |

### 5.2 Integration tests

- **Library end to end** (`tests/cmd_text.rs`): `text add` for each TX-P1 reader over small
  public-domain samples in `fixtures/library/readers/`, then `text ls`, `text show`, `check`;
  expected mids, tids and rdids pinned in `.expected` files.
- **Growth** (`tests/cmd_text_append.rs`): three days of a Telegram export appended one day at a
  time; earlier tids reused (no object write), `supersedes` chain, one head; a concurrent second
  head gives `W418`.
- **Redaction** (`tests/cmd_redact.rs`): redact a tid; objects gone; units, manifests and spans
  remain; a merge with a stale peer that still holds record 15 does not bring it back; a plain
  store that held 15 in its log is rewritten without it (`rewrite_redacted`).
- **Time** (`tests/cmd_date.rs`): draft 3 §19.2's clock fault: one window-target dating with
  `offset:-93000` and basis `cited` re-times exactly the messages in the window, the reply-order
  `W413` disappears, no uid moves; the same dating with an `inferred` basis gives `W412` and is not
  applied; a `canonical` commitment on a dating holds a later one until a resolution names the
  derived contention id.
- **Text ingest offline** (`crates/smysl-ingest/tests/text_gate.rs`, `Scripted` provider): spans
  attached, D-1 copy set exact, journal resume after a simulated kill between batches re-sends at
  most one batch, `--reextract`, pivot policy produces `x.text/translates` with deps.
- **Red team** (`crates/smysl-ingest/tests/redteam.rs`): for each fixture in
  `fixtures/library/redteam/`, the rendered prompt contains exactly two input fences and two digest
  fences; a scripted "obedient" answer that cites a store uid outside the digest (`E442`), claims
  `measured` (rule T), supplies `published`/`observed`/`text:speaker`/`lang` (`W444`), or invents a
  relation kind `supersedes` (refused today) never reaches a staged record unchanged.
- **CLI gates**: `cli-surface.txt`, `dispatch.rs`, `command_table_matches_section_23`,
  `xtask determinism` with the new registrations, `xtask check-purity` with `smysl-text` in
  `PURE_CRATES`.

### 5.3 Property and fuzz harnesses (D-11)

In the repository's style: a fixed-seed xorshift generator under `cargo test` (as
`crates/smysl-graph/tests/merge_algebra.rs`), and a cargo-fuzz target driving the same generator
through `fuzz/src/lib.rs`'s `Choices`. `proptest` is not added.

**Effective time** (`crates/smysl-text/tests/time_algebra.rs`, `fuzz/fuzz_targets/time_engine.rs`).
Generator: a ground-truth assignment of instants to N subjects, as-recorded values derived from it
with chosen statuses, datings (absolute, offset, relative, window) with chosen basis statuses,
withdrawals, commitments and resolutions; optionally planted skews and contradictions.

| property | statement |
|---|---|
| P-E1 order independence | effective output is byte-identical for every permutation of the record list and for every split into two stores merged either way |
| P-E2 idempotence | duplicating any record changes nothing |
| P-E3 no silent override | every bound that differs from the as-recorded value has a why-chain whose applied status is ≥ the bound's time status |
| P-E4 soundness | with no planted fault, every effective interval contains the ground truth and nothing is contested |
| P-E5 detection | each planted skew or contradiction yields a `W412`/`W413` position naming a dating or constraint involved |
| P-E6 reference | for N ≤ 8, the engine equals a brute-force evaluation (Floyd–Warshall per stratum) |
| P-E7 withdrawal | withdrawing a dating returns the output to that of the store without it |
| P-E8 monotone evidence | adding a live dating at a status no higher than all bounds it touches never changes an applied bound; it only adds contention |

**Redaction merge** (`crates/smysl-graph/tests/redaction_algebra.rs`,
`fuzz/fuzz_targets/redaction_merge.rs`). Generator: stores with parts, readings, manifests, units,
and redactions spread across peers, including peers that never saw a redaction.

| property | statement |
|---|---|
| P-Z1 | `merge` is commutative, associative and idempotent on the record set (via SMYSL-2.1's record-set digest) |
| P-Z2 | after any merge sequence, no record 15/18 exists for a tid any participant redacted |
| P-Z3 | re-merging with a stale peer never restores a redacted record |
| P-Z4 | redaction removes only 15/18: units, manifests, datings and relations are identical with and without it |

**Reader fuzz targets** (`fuzz/fuzz_targets/reader_<id>.rs`, one per reader in `cli`): arbitrary
bytes under default caps must return `Ok` or a typed error, never panic, never exceed the caps, and
on `Ok` the output must re-read to the same mids (determinism) and every emitted locator must
resolve. Seed corpora from `fixtures/library/readers/`. Also `fuzz_targets/edtf.rs` (parse/print
round trip) and `fuzz_targets/locator.rs`. `fuzz/Cargo.toml` gains `smysl-text` with the reader
features.

### 5.4 Conformance fixtures (`fixtures/library/`)

```
fixtures/library/
  README.md
  wire/            records 14, 15, 17, 18, 19 (.cbor) + ids.json: tid, mid, did, rdid per record;
                   units with key 9 and sources with keys 4–6; generated by
                   crates/smysl-core/tests/gen_library_fixtures.rs (as gen_uid_fixtures.rs is)
  edtf/            valid.txt, invalid.txt (with expected byte offsets), intervals.tsv (OQ-9)
  readers/<id>/    input file, params, expected manifest (.cbor), mids
  check/           .smy + .expected (E403, E404, W405, W406, E410, W411, W418, E446)
  time/            stores + expected effective intervals and contentions (C-Library)
  redaction/       peer stores and the expected merged record set (C-Library)
  redteam/         untrusted texts and scripted answers
```

The `.expected` format is the existing one (`fixtures/README.md`): exact code sets, one per line.

### 5.5 Gated experiments run in these phases

| experiment | phase | how |
|---|---|---|
| **GE-T1** (determinism) | TX-P1 (5 Bibles, JSON series), TX-P2 (2 chat exports) | build each library on Linux x86-64 and one other platform (CI matrix, unverified availability); compare mids, structure hashes, rdids; Python, JavaScript and Go recompute tid/rdid/mid from the emitted records and agree. They do not re-implement readers. Any mismatch blocks the next phase. |
| **GE-T4** (attribution fairness) | before TX-P5 | 200 hand-checked units per tier-1 language under normaliser v1 and v2 (SMYSL-2.1); v2 false-`Absent` > 1% in any language blocks TX-P5 |
| **GE-T13** (effective time) | TX-P3 | synthetic chats with planted skews plus a public chronology with known relative orders; P-E4/P-E5 on real data; any planted skew undetected, or any false contested interval on consistent data, blocks TX-P4 |
| **GE-T14** (part size) | end of TX-P2 | real chats, revised articles, Bibles at 64 KiB–4 MiB; object count, reuse on revision, redaction granularity; the curve is recorded and the default fixed before TX-P3 |
| GE-T5 within-language arm, GE-T11 | TX-P7 | as draft 3 §23; the cross-lingual arm of GE-T5 is SMYSL-2.5 |

### 5.6 Cross-implementation (C-Read additions, D-8)

| implementation | additions | test |
|---|---|---|
| `python/smysl` (`records.py`, `uid.py`) | decode/re-encode records 14, 15, 17, 18, 19 and unit key 9, source keys 4–6, byte-identically; `tid`, `mid`, `did`, `rdid`; record-set digest (from SMYSL-2.1) | `tests/test_library.py` over `fixtures/library/wire` |
| `nodejs/src` (`records.js`, `uid.js`) | same | `test/library.test.js` |
| `go/` (`records.go`, `uid.go`) | same | `library_test.go` |

The record bodies are maps of integers, text, byte strings and arrays, already within each
port's decoder; the work is per-record field tables and four hash functions. Planned at about
150–250 lines per port (estimate).

---

## 6. Delivery steps

Each step lists its exit test. A phase's exit is the draft 3 §22 test plus the refinements here.

### TX-P1 — library core

1. `smysl-core`: `Tid`/`Mid`/`Rdid`, records 14, 15, 18, keys, envelope, surface `@manifest`.
   *Exit:* round-trip and golden tests; `fixtures/library/wire` generated; key-table test green.
1a. `smysl-core`: open `Admission` (SMYSL-2.3 A-8.1) with the same `Unknown` unit-variant design
   that SMYSL-2.1 §4.3.6 uses for the four enumerations opened in 1.9.0; code 255 is already
   reserved. *Exit:* `fixtures/conformance/codec/enum-unknown.cbor` gains an unknown admission
   code that round-trips byte-identically, with `SMY-W409`.
2. `smysl-text` skeleton: `norm`, `ids`, `objects`, `lock`, `limits`, `manifest`, `part`,
   `reading`, `structure`, `locator`. *Exit:* unit tests of §5.1 for these modules; limits suite.
3. Readers `txt`, `md`, `usfm`, `osis`, `zefania`, `json`. *Exit:* `fixtures/library/readers`
   expected mids; reader fuzz targets run 10 minutes each with no finding.
4. `smysl-graph` manifest state, heads, by-part maps, adjacency flag. *Exit:* store tests; 1.8
   append timing unchanged for unit batches, manifest-only append `O(batch)`.
5. `smysl-check` `Library` pass (E403, E404, W405, W418, E446); `ConformanceClass::Library`.
   *Exit:* `fixtures/library/check` green.
6. CLI `text add/ls/show`, `check --library`; purity gate with `smysl-text`; `cc` check in
   `make crate-features`. *Exit:* gates green.
7. Ports: C-Read for 14, 15, 18 and the ids. *Exit:* **GE-T1** on Bibles and a JSON series.

### TX-P2 — segments, languages, chats, redaction

1. `segment`, `lang`, `analyze`; abbreviation lists. *Exit:* sentence-boundary F1 on a 500-sentence
   gold set per tier-1 language, proposed bars ≥ 0.97 (en, es, fr, de) and ≥ 0.95 (ru), not set by
   draft 3.
2. Readers `telegram`, `slack`, `whatsapp` with `--pseudonymise`; opt-in `mbox`, `epub`, `html`,
   `pdf`. *Exit:* chat round trip with `raw` intact; `E450` test; fuzz targets.
3. `text append`. *Exit:* growth test (§5.2).
4. Record 19, rule Z in `Store::append`, `rewrite_redacted`, `text redact`, `@redact`. *Exit:*
   redaction test; P-Z1–P-Z4 harness green; ports decode 19.
5. *Exit for the phase:* **GE-T1** on two chat exports; **GE-T14** curve recorded and default part
   policy fixed.

### TX-P3 — time

1. `smysl-core`: `edtf.rs`, `source.published` (FC-2), record 17, `@date`, `Did`; time
   `DetectionKind` values for derivation only. *Exit:* EDTF fixtures; round trips; ports decode 17.
2. `time::{calendar, constraints, stn, engine}`; copy and omission rules in `provenance::stamp`.
   *Exit:* P-E1–P-E8 green; omission rule tested at second vs millisecond precision.
3. `smysl-check` `Time` pass. *Exit:* `E410`, `W411`, `W412`, `W413`, `W449` fixtures.
4. `date set/order/show`, `--why`. *Exit:* §19.2 scenario (§5.2).
5. *Exit for the phase:* **GE-T13**.

### TX-P4 — substrate index and retrieval

1. `index` tables of §3.8 with rebuild and incremental builders. *Exit:* rebuild ≡ incremental
   (byte comparison of a table dump); deleting `index.redb` and rebuilding restores every derived
   table; the operational `journal` is rebuilt by re-checking staged/stored units.
2. `Analyze`/`Postings` seams; persistent postings per partition; `find --lang --analyzer`.
   *Exit:* `find` p95 < 200 ms at 155k units (13 s today, F-10) on the Appendix E corpus;
   results identical to the in-memory path.
3. `said`/`composed`/`observed` range tables. *Exit:* a `said during` range over 155k units answers
   without sorting the store (asserted by an instrumented test).
4. `text align` and the `align` table (needs OQ-4 settled for shipped maps). *Exit:* KJV ↔ Synodal
   Ps 14 ↔ Ps 13 alignment.

### TX-P5 — ingest over texts

Precondition: **GE-T4** passed; SMYSL-2.2's spike result read.

1. FC-1 (`lang`, core key 9; D-2 reserved word) and FC-3 (`span`, `manifest`); ports.
   *Exit:* old stores re-encode unchanged; `W406`/`E404` fixtures.
2. `x.text/v1` declaration shipped as a schema record; entity units without span via
   `provenance` (`ref: "alias:<alias>"`). *Exit:* the same entity from 50 windows is one uid.
3. Windows, owned ranges, node hint, span attachment, `Absent` behaviour, payload quote removal.
   *Exit:* §5.1 ingest tests.
4. Ledger digest, `E442`, journal, language policy, recipe fields, prompt templates with fences.
   *Exit:* red-team suite; resume test.
5. *Exit for the phase (draft 3 §22):* resume after kill loses ≤ one batch; ≥ 95% of units carry a
   span; entity duplication < 2% across windows; a **cost report** from the journal (tokens per call,
   per language) replaces draft 3 §7.7's planning figures.

### TX-P7 — proposition layer and linking

(TX-P6, `sq`, is SMYSL-2.5. `link` and `summarise` accept alias/mid scopes until it lands.)

1. `proposition::classes` (strict per SMYSL-2.3, component, attested:n). *Exit:* strict class
   counts match a fixture computed by the Python reference (SMYSL-2.3 conformance) and are
   invariant under record order.
2. Engines `identical-span`, `lexical`, `anchored`; `same-as propose/classes`. *Exit:* anthology
   test: a paragraph reprinted under two expressions forms one class by `identical-span`.
3. Linker and summary tree. *Exit:* GE-T11 (recall ≥ 0.6 on 50 planted far contradictions at the
   default candidate budget); summary nodes capped by rule M (asserted).
4. *Exit for the phase:* **GE-T5** within-language arm (precision ≥ 0.9 at recall 0.7, else class
   measures ship as exploration only).

---

## 7. Risks and mitigations

| risk | consequence | mitigation |
|---|---|---|
| ~~MSRV~~ **closed 1.10.0.** The claimed MSRV was already false for every crate, not just `cli` | — | OQ-40 answered by measurement: base 1.85, provider and ingest 1.86, tui and facade 1.88, each declared and gated by `make msrv` plus a CI job compiling the base tier. `redb` 4.3's 1.90 is a normal bump when `store-redb` lands |
| a time contention written as record 6 | stores unreadable to readers older than SMYSL-2.1's fixes | never written: SMYSL-2.3 A-8.2 forbids kinds 4 and 5 in record 6 (OQ-35 resolved) |
| readers drift (a dependency changes output across versions) | mids change, GE-T1 fails later | reader id includes the reader's own version; dependency versions pinned with `=` for readers; GE-T1 rerun on every dependency bump of a reader |
| `lingua` detection changes between versions | segment languages in readings change → rdid changes | `lingua` pinned exactly; its version is part of the reader id for `mul` manifests |
| stratified STN differs from rule E's wording | two implementations disagree on contested sets | P-E6 brute-force reference against SMYSL-2.3 A-12.2's wording (OQ-42 resolved there) |
| owned-range attribution still depends on the model's node hint | rerun with a different hint gives a different span | the hint narrows only; the span is the earliest occurrence in the first matching scope; `W419` flags ambiguity; GE-T2 measures stability |
| ledger digest grows prompt cost | P + D dominates (draft 3 §7.7) | `D` is a recipe budget; the journal's cost report measures it |
| adjacency flag misses a record type that affects traversal | stale adjacency | the flag lists what does **not** rebuild (manifests, datings, redactions, part texts, readings); everything else rebuilds, including unknown records |
| physical erasure leaves copies (old peers, backups, sidecars, staged files) | erasure is incomplete | `text redact` also scans `<library>/stage/` and the redb tables; the limit for pre-RFC peers is stated in draft 3 §4.8 and in `text redact --help` |
| lock files left by a crash | writes refused | explicit `--break-lock`, holder printed; no automatic breaking |
| fuel caps refuse a legitimate large input | user friction | defaults sized on the five Bibles and a multi-year chat; every cap has a flag; the diagnostic names the flag |

---

## 8. Diagnostics allocated in this RFC

Range `SMY-E/W440`–`459`. Draft-3 codes this RFC emits (`E401`–`W419`) keep their numbers and
meanings and are not repeated here.

| code | meaning | emitted by |
|---|---|---|
| `SMY-E440` | reader resource cap exceeded (names the cap, the limit and the flag); nothing written | readers, `text add/append` |
| `SMY-W441` | attributed quote absent after repair; unit staged without span, status capped (`inferred` with grounds, `speculative` without) | `ingest --text` |
| `SMY-E442` | a model answer references a unit outside its batch and ledger digest | `ingest --text`, `link`, `summarise` |
| `SMY-W443` | untrusted text contained a prompt fence marker; neutralised in the prompt only | prompt rendering |
| `SMY-W444` | a tool-owned field (`source`, `lang`, `span`, `published`, `observed`, `text:speaker`) in a model answer was dropped | `ingest --text` |
| `SMY-E445` | library or shard is locked by another writer (holder printed) | every writing command |
| `SMY-E446` | object or record 15/18 does not hash to its tid/rdid | object store, `check` |
| `SMY-W447` | a structure node larger than the window was split at sentence boundaries | `ingest --text` |
| `SMY-W448` | a reply or quotation constraint names a message or unit not found; constraint skipped | time engine |
| `SMY-W449` | EDTF value outside the index's representable range; indexed as an open bound | time engine, `check` |
| `SMY-E450` | appending to a pseudonymised expression without its pseudonym key | `text append` |
| `SMY-W451` | an alignment scheme does not cover a locator; the pair is left unaligned and counted | `text align`, `anchored` engine |

`452`–`459` are unallocated. SMYSL-2.3 may adopt `E446` as a C-Library obligation.

---

## 9. Open questions

**Draft-3 questions this RFC depends on** (numbers unchanged):

| id | status here |
|---|---|
| OQ-1 | decided by D-2; implemented in §4.3.1 |
| OQ-2 | stems only; Russian type statistics labelled as stem-level |
| OQ-3 | open; a reader parameter `--param orthography=pre1918` is the cheaper path and keeps one manifest |
| OQ-4 | open; blocks shipping maps in TX-P4 step 4, not the `align` table itself |
| OQ-8 | this RFC stores 15/18 as objects beside logs; bundles may carry them under `carry: text` |
| OQ-9 | hand-rolled parser with `fixtures/library/edtf` as the published suite |
| OQ-10, OQ-12 | decided by D-3, D-4; implemented in §3.3 |
| OQ-11 | per connected component of the constraint graph, recomputed on change; revisit at GE-T7 |
| OQ-13 | open; only window targets are implemented |
| OQ-17 | decided by D-1; `provenance::stamp` writes exactly that set |
| OQ-18, OQ-19, OQ-20 | open; the segment table keeps offsets, edit history and pointers so either answer is possible later |

**New questions:**

| id | question |
|---|---|
| OQ-35 | **Resolved in SMYSL-2.3 A-8.2:** `DetectionKind` is opened by SMYSL-2.1, and time contentions stay derived; kinds 4 and 5 are never written to record 6. |
| OQ-36 | Lock files by `create_new` with explicit breaking, or OS advisory locks (`std::fs::File::lock` needs a newer toolchain; `fs4` adds a dependency)? |
| OQ-37 | JSON readers: `serde_json` behind features (outside the purity gate), or a strict-JSON mode of `smysl-core`'s hand-rolled HJSON parser, which would keep `json/1` and `telegram/1` inside the gate? |
| OQ-38 | Is a deterministic fuel cap enough, so the wall-time watchdog can be dropped, or is the watchdog worth keeping as a backstop? |
| OQ-39 | Physical erasure when record 15 sits in a plain log: rewrite the log (as proposed), or refuse 15/18 in logs entirely and keep them only as objects? |
| ~~OQ-40~~ | **Answered 1.10.0, and the question was wrong.** Neither: 1.79 was already false everywhere — the pure core cannot be *parsed* by a 1.79 Cargo, because `blake3` pulls an edition-2024 `constant_time_eq`. The measured floors (1.85 / 1.86 / 1.88) are declared per crate and gated by `make msrv`. `redb` 4.x's 1.90 is an ordinary bump in the release that ships it. §4.5 |
| OQ-41 | Removing `ingest:quote` from span-carrying text units changes their uids relative to prose ingest of the same text. Accept, or keep the quote and accept duplicate uids per span? |
| OQ-42 | **Resolved in SMYSL-2.3 A-12.2:** strata by status; a chain tightens at its weakest link's status. §3.3 implements exactly that. |
| OQ-43 | Should the digest-scoped reference check (`E442`) also admit units of the same expression not shown in the digest (a model that remembers an earlier window), or stay strict? |
| OQ-44 | `lexical` same-as proposals: auto-accept above a threshold at rung `computed`, or always require a judge (model or human)? |
