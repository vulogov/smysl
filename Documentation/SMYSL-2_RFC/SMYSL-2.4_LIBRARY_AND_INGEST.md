# RFC SMYSL-2.4 — Library and ingest

**Status:** draft 2 — re-planned against G0's pivot (SMYSL-2.0 §1.1) on 2026-10-08.
Implementation RFC (non-normative); normative rules are in SMYSL-2.3.
**Author:** Vladimir Ulogov
**Date:** 2026-10-02, re-planned 2026-10-08
**Part of:** RFC set SMYSL-2 — see SMYSL-2.0 (index and roadmap).
**For:** crate `1.10.0`, format `smysl/1.0`, kernel `smysl.kernel/0.1`. Draft 1 was written
against `1.9.0-dev` (`d25ec9e`), before TX-P0 and before the spike; §0.1 records what that
costs the reader.
**Derived from:** RFC SMYSL-2 draft 3, §3, §4, §5, §6 (implementation of the time engine), §7, §8,
§9, §10 (except §10.4), §14.1–14.3, §19.1–19.4, §20; phases of §22.
**Phases:** TX-P1, TX-P2, TX-P3, TX-P4, TX-P5, TX-P7.
**Depends on:** SMYSL-2.3 (records 14–19, ids, FC-1/2/3/5, rules E, N, Z, copy and omission
rules, `strict` classes); SMYSL-2.1 (TX-P0: F-13/F-14 surface fixes, normaliser v2, script-aware
estimator, opened enumerations, record-set digest, today's prompt-injection guard); SMYSL-2.2
(spike S0 — **which has now run**: its GE-T2 and GE-T5 pilot readings did not come out as draft 1
assumed, and §0.1 is the consequence).

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
- **Ingest changes in six places** (draft 3 §7.7), plus two found while verifying the code — a
  unit with no grounds cannot be capped at `inferred` (the constructor refuses it), and the
  `ingest:quote` payload key is inside identity today, so text units drop it once a span is
  attached — plus **two more from G0's pivot** (§0.1): extraction runs twice per window by
  default, and `text:holder`/`text:mode` are written from structure rather than asked of the
  model.
- **Security (D-10)**: reader limits are hard caps with diagnostics; the existing fence in
  `prompt.rs` is extended to windows, ledger digests and linker pairs; references a model may
  write are limited to its digest and batch; one writer per shard behind a lock file.
- **Measured risks found here**: `redb` 4.3 declares `rust-version = 1.90` against the
  workspace's — which was `1.79` when this was written and is `1.85` since **OQ-40 was answered
  by measurement in 1.10.0**, where the 1.79 claim turned out to be false already and in every
  selection (§4.5); `DetectionKind` is a closed enumeration, so time contentions cannot be
  written to a log yet; `unicode-segmentation` 1.13.3, already in `Cargo.lock`, declares 1.85 —
  which is now the base rather than above it.

Diagnostics allocated here: `SMY-E440`–`SMY-E452`. Open questions: OQ-35–OQ-44.

### 0.1 Re-planned against G0's pivot (2026-10-08)

Draft 1 was written before the spike ran, and names S0 as the thing that "decides whether TX-P5
starts as planned". S0 has now run — 365 runs, 29 inputs, five languages, two models — and eight
rows of its decision table fire. SMYSL-2.0 §1.1 states the pivot; this is where it lands. The
reason this is a re-plan and not an edit is that two phase exit tests change (TX-P5's and
TX-P7's), one step moves between phases, and one of the pivot's own six items turns out to
prescribe the wrong remedy for the thing it measured.

| # | §1.1 item | lands in | what the re-plan found |
|---|---|---|---|
| 1 | F-2 before TX-P1 | *done* — 1.10.0 | — |
| 2 | extraction is consensus by default | §3.4, §6 TX-P5 | **the remedy as stated is circular**, and the usable form is narrower. See below. |
| 3 | class measures ship as exploration only | §3.6, §5.5 | lands as written; `anchored` becomes the only Bible engine, and adjacency (C3) is load-bearing where draft 1 treated it as a filter |
| 4 | holder and mode structural from TX-P5's first ingest | §3.4, §6 TX-P5 | lands as written; the probe-set gate is a new exit clause, and the holder paragraph stays out of the prompt |
| 5 | GE-T5 and GE-T2 restated relative to α | §5.5, §6 TX-P7 | **the 0.9 bar was never attainable** against a gold of α 0.600, so restating it is not a loosening |
| 6 | FC-6 weights calibrated on M5; cost model revised | §3.7, §6 TX-P5 | **the weights are not what is miscalibrated.** S0 names the mechanism and it is the repair loop. Calibrating weights on pre-F-2 data would fit the estimator to a defect 1.10.0 removed. |

**Item 2: consensus extraction cannot be validated by the machinery that would judge it.**
"Extract twice per window and keep what both runs produce" needs a relation that says two units
are the same proposition. At uid level there is none to have: a uid covers label, gist, status and
quote, and S0 found **zero shared uids between the two models anywhere in the corpus**, the hosted
model failing to reproduce even itself at temperature 0 (J_uid 0.182 en). At class level the
relation is same-as — which items 3 and 5 have just declared unreliable, with a model-judge α of
0.600 and no cell of the cosine proposer reaching its precision bar. So the pivot's remedy for
unstable extraction rests on the one layer the same spike found least trustworthy.

What survives is the part that needs no judge. `identical-span` — same tid, same span, same gist —
is an exact relation, computed, not judged. Two runs that attach the same span and write the same
gist agree by construction. So **TX-P5's consensus gate is stated on `identical-span` agreement
only**, which is measurable at its own exit, and judged agreement waits for TX-P7 under item 5's
restated threshold. The second extraction is **not** deduplicated during ingest: both runs' units
are staged, and the consensus is read afterwards. That doubles the unit count per window, which is
a cost item 6 has to carry.

**A step moves.** Because of the above, `attested:n` and the class core must exist before TX-P5's
output can be read at all. TX-P7 step 1 — `proposition::classes`, strict and `component` and
`attested:n` — is pure, needs only `Store` and records, and depends on no ingest, no text and no
model. It moves to **TX-P2 step 5** (§6). This is the smallest form of the reordering S0's report
§6 asks for in its one-line plan state ("TX-P7's same-as core and F-2 move ahead of further ingest
work") and that §1.1's item 2 does not carry; T4, the row that would have mandated a full reorder,
did not fire, so the full reorder is not taken.

**Item 5: the bar could not have been passed.** GE-T5's "precision ≥ 0.9 at recall 0.7" was set
without reference to what its gold can support. S0's gold is a majority of three model coders
whose binary α is 0.600 [0.550, 0.646], and whose best-agreeing pair reaches 0.636; raw agreement
between the hosted pair is 61.7%. A perfect engine scored against a gold that disagrees with
itself at that rate cannot reach 0.9, so the measured 0.835 same-language and 0.712 cross-lingual
are not evidence that the engines are below the bar — they are evidence that the bar measures the
gold. The restatement in §5.5 is therefore not a loosening: it replaces a number that could not be
reached with one that can be, and the absolute bar returns only when GE-T9 supplies a human α.

**Item 6: the estimator is not the thing to recalibrate.** T13 fires because the hosted ru/en cost
ratio is 3.38x against the estimator's 1.96x — 73% under. But S0's own analysis names the cause:
*"The difference is the repair loop itself, not tokenization."* Against the model that never
enters that loop the estimator is 6% over, and the regression on arm R agrees with it to 2%. The
73% is retries, and the retries were caused by a byte-based bound that F-2 has since replaced. So
FC-6's weights are **not** changed on this data; what changes is that the cost model gains an
explicit retry term, and that the non-English figures are **re-measured after F-2** before TX-P5
is sized (§6 TX-P5 step 5). Fitting weights to pre-F-2 Russian would have baked a fixed defect
into the estimator and then measured the estimator as correct.

**Two findings outside the table.** `temperature` is correctness-relevant (MS-8: bit-identical at
T=0 over 145 runs, no shared uid at all at T=0.7 on five of six inputs), which §3.4 now states as
a precondition of item 2 rather than as a configuration note — consensus over two runs means
nothing at a temperature where a single run shares nothing with itself. And the language policy
F-6 added was an instruction with no verification (MS-5/MS-6); 1.10.0's `SMY-W436` counts
character ranges, so that one is closed in code rather than in this plan.

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
  `anchored`. Class derivation lands in TX-P2 rather than TX-P7 (§0.1), and class measures ship as
  exploration only (§3.6).
- Languages (§10.1–10.3, §10.5): `lingua` with five languages, analyzer chains, wiring of the
  content-aware estimator (`smysl/content/1`) that SMYSL-2.1 added and 1.10.0 shipped.
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

> **Re-plan note (2026-10-08).** This section was verified against `d25ec9e`, which is pre-TX-P0.
> 1.10.0 has since shipped TX-P0 in full, including F-2, F-16's `bundle --unknown`, the record-set
> digest, the four opened enumerations and `SMY-W436`. Where a statement here reads as a present
> fact about the tree, it is a fact about `d25ec9e`; §0.1 lists the premises the pivot changed, and
> the MSRV figures in §0 and §4.5 are the 1.10.0 ones.

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
- `xtask/src/purity.rs`: two lists since 1.10 (OQ-37). `NEVER` (`clap`, `tokio`, `ureq`,
  `ratatui`, …) is rule B and the not-a-front-end claim, checked on
  `cargo tree -p smysl --no-default-features` **and**, for each of
  `PURE_CRATES = [smysl-core, smysl-graph, smysl-check, smysl-pack, smysl-thread, smysl-render,
  smysl-retrieve]`, at both default features and `--all-features`. `NOT_IN_THE_CORE`
  (`serde_json`, with the feature allowed to pull it) is the weaker claim and is checked at
  **default** features only. `FORBIDDEN_SYMBOLS` greps sources for
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
- **A log never holds 15 or 18** (OQ-39, answered 1.10.0). `smysl text add` never wrote them
  there; now nothing may. A bundle carrying text is absorbed into the receiving library's object
  store, and a plain `Store` with no library beside it refuses the record with `SMY-E452` naming
  `text init` as the fix. The reason is that the alternative — rewriting the log to honour a
  redaction — is the one operation an append-only log cannot survive as evidence: it resets the
  same hash chain that would have shown tampering, so afterwards the log cannot distinguish the
  redaction from an edit. What the refusal costs is small and bounded: a plain `Store` holding
  carried text can neither resolve a locator nor check a span, both of which need `smysl-text`,
  so all it could do with the bytes is hold them.

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
| `md/1` | `reader-md` | TX-P1 | `pulldown-cmark` | **verified in step 3**: 0.13.4, MIT, rustc 1.71.1 |
| `usfm/1` | `reader-usfm` | TX-P1 | hand-rolled | |
| `osis/1`, `zefania/1` | `reader-osis`, `reader-zefania` | TX-P1 | `quick-xml` | DTDs refused (§3.9.1); pinned `=0.41.0`, see step 3 |
| `json/1` | `reader-json` | TX-P1 | `serde_json` (OQ-37 answered: yes) | JSON Pointer locators |
| `telegram/1`, `slack/1` | `reader-telegram`, `reader-slack` | TX-P2 | `serde_json`; Slack also a zip reader (unverified choice) | |
| `whatsapp/1` | `reader-whatsapp` | TX-P2 | hand-rolled | date pattern is a required parameter |
| `mbox/1`, `epub/1`, `html/1`, `pdf/1` | `reader-mbox` … | TX-P2 (opt-in) | `mail-parser`, `rbook`, `html2text`, `pdf-extract` | not in `cli` |

**Structure.** `Structure` is an arena tree: `Node { level, range: Range<u64> (bytes in the
part), locator: Locator, children: Range<u32>, parent: Option<u32> }`. Ranges of siblings are
disjoint and ordered. The structure hash is computed over its canonical CBOR encoding as
SMYSL-2.3 defines it; rdid is computed over record 18's body.

**Built in TX-P1 step 2, with one field fewer than this sketch.** `meta: Option<u32> (row in
the segment table)` presumed a structure table and a segment table side by side. There is one
table: A-5 says the structure hash covers "the canonical CBOR of the structure table carried in
the reading's segments (record 18 key 2)", so the rows *are* the nodes and the row index is the
node index — `meta` would have been a node's own index, which is nothing. `parent` is there
instead, because the tree is **derived** from the rows by containment rather than stored beside
them: one statement of the structure, and it is the one that is hashed. The cost is one pass
with a stack at `Structure::build`, which is also where a defect in the table is caught — a
*partial* overlap, which is what an off-by-one in a verse boundary produces and which nothing
downstream has the information to notice.

**Locators.** `locator::parse` is a hand-written recursive-descent parser (no regex
dependency). Resolution is through the structure tree only:
`Structure::resolve(&Locator) -> Option<Range<u64>>`; a range locator resolves to the hull of its
two ends. `Locator::to_string` is canonical (round-trip tested), so a locator in a `ref` is
written one way.

**The grammar is this crate's, not draft 3's** (found in TX-P1 step 2). This section pointed at
"draft 3 Appendix B". Draft 3 is the design record, it is not in the repository, and the set that
supersedes it carries no locator grammar — so there was nothing to implement against, and the
grammar is now written down in `smysl-text/src/locator.rs` and tested both ways. Four forms,
chosen against what the six TX-P1 readers have to be able to say rather than invented:

| form | example | reader |
|---|---|---|
| canonical | `Gen.1.1`, `1John.3`, `Ps.136` | `usfm`, `osis`, `zefania` |
| line | `L412` | `txt` |
| JSON Pointer (RFC 6901, `~0`/`~1`) | `/messages/3/text` | `json` |
| range | `Gen.1.1-Gen.1.3`, `L10-L14` | canonical or line ends, never pointers |

The canonical form is deliberately OSIS-shaped: the five Bibles of GE-T1 are distributed with
those identifiers, so a locator that had to be translated out of the source's own vocabulary
would be one nobody could check by eye.

**The vocabulary, settled in step 3, and one claim above withdrawn.** The sentence this
paragraph used to end with cited the spike's `in/align.tsv` as keyed by OSIS identifiers,
"(`Ex.20.1`)". It is not: the spike keys its rows `Ex.20.1`, `Ecc.1`, `1Ki.3` and `Jas.2`,
where OSIS writes `Exod`, `Eccl`, `1Kgs` and `Jas` — three of those four differ, so the
parenthetical was evidence for a conclusion it did not support. The conclusion stands on the
other ground: OSIS is the only one of the four vocabularies in play (OSIS, USFM's three-letter
codes, Zefania's book numbers, the spike's abbreviations) that is published and standardised
rather than local to a file format, and it is the one `osis/1` needs no table for. `books.rs`
holds the 66 rows and the two mappings into them. The spike is an input to an experiment and
not a corpus, so the table is what moves: GE-T1 re-keys it or carries a mapping.
Three decisions worth recording, because each is a refusal rather than a convenience:

- **One spelling per place.** `Gen.01.1` does not parse rather than parsing as `Gen.1.1`; a
  parser that accepted both would put both into a corpus and only the writer would know which
  was meant. The sole normalisation is a range whose ends are equal, which *collapses* to the
  single locator, so no corpus can hold `L7-L7` and `L7` as two things.
- **No pointer range syntax at all.** A pointer is the whole string or none of it (`-` is a
  legal character in a reference token), so `/a-/b` is one pointer. A range of pointers has no
  meaning this crate is willing to invent, and `Locator::range` refuses it.
- **`#` is not a locator character**, because a reference is `t3:…#<locator>` and a locator
  holding a `#` could not be read back out of one. Whitespace and controls are out for the
  related reason: they survive a CBOR text string and then make two locators that look identical
  two different keys.

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

**Consensus extraction** (§1.1 item 2, re-planned in §0.1). `ingest --text` extracts each window
**twice** by default — two models where two are configured, otherwise two runs of one — and stages
**both** results. Nothing is deduplicated during ingest: the agreement is read afterwards, because
the only relation that can establish it without a judge is `identical-span` (same tid, same span,
same gist), and that is computed over staged units, not during the call. `--single-pass` opts out
and is recorded in the recipe, since a corpus built one way is not comparable with one built the
other.

Two consequences the plan has to carry. The unit count per window roughly doubles, and so does
cost; §6 TX-P5 step 5 sizes it from the journal after F-2 rather than from a planning figure. And
**the second pass is worthless above temperature 0**: MS-8 measured arm L bit-identical across all
145 runs at T=0 and sharing *no uid at all* at T=0.7 on five of six inputs, so two runs at T>0
cannot agree by `identical-span` except by accident. `ingest --text` therefore refuses
`--temperature` above 0 unless `--single-pass` is given — `temperature` is a correctness
parameter, not a quality knob, and it is already inside the recipe (`recipe.rs`), so a corpus
records which it was.

**Holder and mode are structural from the first ingest** (§1.1 item 4). Draft 1 left this to the
prompt. S0 measured reported speech extracted as asserted in 11.1–24.5% of attribution-bearing
units across every model and coder pairing, every cell above the 10% threshold (T8), and the arm
that *added* a holder paragraph to the prompt did no better (23.8% and 16.7%, T9 does not hold)
while introducing `E022` degradations into a configuration that had none (MS-9, MS-11). So the
paragraph stays out of the prompt, `text:holder` and `text:mode` are written by the converter from
structure — reported-speech spans the reader marks, not the model's self-report — and TX-P5's exit
gains a probe-set gate (§6).

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
  `Store::is_withdrawn`. **The class core moves to TX-P2** (§0.1): it is pure, needs only records,
  and TX-P5's consensus output cannot be read until it exists.
- **Class measures ship as exploration only** (§1.1 item 3). Not a quality judgement on the
  implementation — a statement about what has been measured. S0's cosine proposer reached
  precision 0.789 same-language and 0.427 cross-lingual at recall 0.7 against a 0.9 bar, 0.835 and
  0.712 with adjacency, and **no cell reached the bar under either provisional gold**. Corpus
  measures over classes therefore default to `attested:2`, every reported class figure carries the
  gold's α, and SMYSL-2.6 reports no class-derived `A*` as settled until GE-T5 runs under §5.5's
  restated threshold.
- **Cross-lingual classes wait for TX-P10's S1 embeddings**, and `anchored` alone is used for the
  Bibles — where it is also the right engine, since a versification alignment is exact where a
  cosine is not. Adjacency (C3) turned out to be load-bearing rather than a filter: it lifts
  cross-lingual precision 0.427 → 0.712 by shrinking a candidate set cosine ranks badly across
  languages, so the `lexical` engine keeps it even once embeddings land.
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
- The estimator from SMYSL-2.1 is what text ingest uses to fill windows. Its id is
  **`smysl/content/1`**, not the `smysl/script-aware/1` draft 1 wrote: F-2 shipped in 1.10.0 under
  the registered id, and SMYSL-2.3 A-9 was corrected to match the wire.
- **The cost model, revised** (§1.1 item 6, and see §0.1 for why the weights are not touched).
  Draft 1 planned against "≈ 1.6k in + ≈ 1.2k out, ×1.3 for repairs" per chapter, a figure only
  ever given for English. S0 measured 1.42x that for hosted English and **3.96x for hosted
  Russian** (3.05x including the repair allowance), and the ru/en ratio at 3.38x against the
  estimator's 1.96x. The gap is not tokenization: against the model that never enters the repair
  loop the estimator is 6% over, and the arm R regression agrees with it to 2%. So the model gains
  a **retry term** — cost per window is `base(tokens) × (1 + r)` where `r` is the measured retry
  rate per language, recorded in the journal rather than assumed — and the per-language figures
  are re-measured after F-2, because every Russian number S0 has was taken under the byte-based
  bound F-2 replaced. No FC-6 weight changes on this data.

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
  `--break-lock` removes a lock explicitly; nothing removes one by timeout. No new dependency.
  **OQ-36 answered 1.10.0: this, not advisory OS locks.** `std::fs::File::lock` is unstable at
  1.88 (`rustc +1.88` says so; it compiles at 1.92), so it would put the floor of a pure-path
  crate above anything its dependencies need — an `EXCEEDS` entry in `scripts/verify-msrv.py`,
  argued for a lock — and `fs4` buys the same thing for a dependency. Two properties decide it
  without reference to that cost. An advisory lock cannot name its holder, and `SMY-E445` is
  specified to. And a lock the kernel releases on process death destroys the only evidence that
  a writer died mid-append: the stale lock *is* the crash notice, `--break-lock` is where an
  operator says they have read it, and the truncated tail it warns about is already tolerated by
  `from_cbor_seq`. A lock that cleans itself up would make the common case quieter and the
  interesting case invisible.
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

Dependencies as of TX-P1 step 3: `smysl-core`, `unicode-normalization`, and behind the reader
features `quick-xml` (`=0.41.0`), `pulldown-cmark` (`=0.13.4`), `serde` and `serde_json`.
The rest arrive with the modules that need them, which is the only way a purity gate over this
crate means anything — a dependency list written ahead of its callers is a list nobody can
check. The full set when the crate is finished: `smysl-core`, `smysl-graph`, `smysl-retrieve`,
`unicode-segmentation`, `caseless`, `rust-stemmers`, `lingua` (`default-features = false`, five
language features); optional
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
}
// No `part_text` and no `rewrite_redacted`: a log holds neither 15 nor 18 (OQ-39), so there is
// nothing for a `Store` to hand back and nothing for it to erase. `Library::part` is the
// accessor, over the object store, and erasure is an unlink (§4.3.2).
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
  `redactions: BTreeMap<Tid, BTreeSet<Redaction>>`, `by_tid: BTreeMap<Tid, BTreeSet<Uid>>`,
  `by_mid: BTreeMap<Mid, BTreeSet<Uid>>`. All derived in `absorb`, all order-independent (maps of
  sets). **No `part_texts` or `readings` map**: draft 1 had both, "for 15/18 held in a log", and
  OQ-39's answer is that no log holds either — so the maps could only ever have been empty.
- **By-part lookups.** `absorb` parses `source.reference` once per new unit: if it starts with
  `t3:` and the next 52 characters parse as a `Tid`, the uid goes into `by_tid`; `source.manifest`
  feeds `by_mid`. `units_with_source_prefix` is unchanged (it is public contract since 1.5); the
  draft-3 warning about short prefixes is answered by the new exact lookups.
- **Redaction filtering on append and merge (rule Z).** In `append`, before the duplicate check:
  1. collect tids redacted by the store **or by this batch** (records 19 in `records`);
  2. drop from the batch every `PartText` / `PartReading` whose tid is in that set, counting them
     in `AppendReport.redacted` (new field; `AppendReport` is already `#[non_exhaustive]`, so
     adding it is not a break). This is the only step: there is no in-memory 15/18 to clear
     afterwards, because none is kept, and the records would be refused by `E452` in any case —
     the filter is what makes a *redacted* one a counted drop rather than an error.
  Because `merge` is `append` plus detection (§2.2), merge inherits the filter with no further
  change. The filtered union is commutative, associative and idempotent: the redaction set only
  grows, and the filter is applied to the union, whatever the order.
- **Physical erasure is an unlink, because no log holds the bytes** (OQ-39, answered 1.10.0).
  Earlier drafts gave `Store` a `rewrite_redacted` that wrote the log again without the record,
  reset `log_hasher`/`log_len` and left the sidecar to be rebuilt (`W110`). That method is not
  written. It was correct and it was also the one operation that costs the log its own integrity
  evidence, and it existed for a case §3.1 now refuses outright: `append` rejects a record 15 or
  18 with `SMY-E452` rather than storing bytes it may later have to erase. Erasure is therefore
  always an object unlink, the log stays append-only in the strong sense, and there was nothing to
  migrate: when the question was answered the record enum stopped at 13, so the choice was made
  before any byte depended on it. TX-P1 step 1 has since added 14, 15 and 18, and it added them
  under this answer — the refusal is the first thing `Store::append` will do with a 15, not a
  behaviour retrofitted onto records already in circulation.
- **Heads and forks.** `heads(alias)` = mids of that alias not named by any other manifest's
  `supersedes`; more than one is `W418` (reported by `check`, not recorded).
- **Adjacency cost.** `absorb` currently calls `rebuild_adjacency()` whatever arrived. It gains a
  flag, written as a **deny-list**: the rebuild is skipped only when every record in the batch is
  text — a manifest, a part text, a part reading, and from the releases that add them a dating
  and a redaction — and taken for everything else, `Unknown` included (pending attestations are
  retried there). A `text add` that appends one manifest, or a `date set`, then costs `O(batch)`
  instead of the 1336 µs/record single-append figure of 1.8. Verified by a test that asserts
  `adjacency` is untouched after a manifest-only append, and by the 1.8 timing harness.

  This sentence said "rebuild only if the batch held a unit, relation, withdrawal or attestation"
  until step 4 implemented it, which is the same rule stated as an allow-list and is **not**
  equivalent: the two differ for every record type nobody has listed, and they differ in the
  direction of failure. §7's risk table already said deny-list, with the argument; the two
  sentences contradicted each other and this is the one that was wrong.
- **Withdrawals and commitments naming a did** need no change: both maps are keyed by `Uid` for
  targets "whether or not [they have] arrived", and `Did::as_uid` supplies the key.
- **`emit`** is SMYSL-2.1's (F-16). This RFC adds the arms: 14 and 19 always travel; 17 travels
  with its target; 15/18 only when the manifest allows `carry: text` (`E402` otherwise, raised by
  the facade before calling `emit`).

#### 4.3.3 `smysl-check`

New passes appended after `CommitmentSupport`, as the enum's comment requires:

| pass | number | codes | needs |
|---|---|---|---|
| `Library` | 12 | **built in step 5:** `E403` a reference claiming one of the four text prefixes and not parsing as that identity; `W418` two heads; `E452` a record 15 or 18 in a log (OQ-39); and with a resolver, `E446` an object whose bytes do not hash to its tid and `E401` a part entry whose `length` disagrees with the object. **Deferred:** `E404` span past the part's length and `W405` locator disagrees with span, both reading `SourceRef.span` (TX-P5, §4.3.1); `W406` holder/speaker uid not in `deps`, whose `text:holder`/`text:speaker` fields arrive with FC-9 field types in TX-P12 | store; resolver optional |
| `Time` | 13 | `E410` malformed EDTF (in `published`, manifest key 8, dating values, `text:when`); `W411` `observed` outside the `published` interval; `W412`/`W413` from the engine, reported as derived contentions; `W449` | `smysl-text` (feature `text`) |

- `CheckOptions` gains `parts: Option<Arc<dyn PartResolver + Send + Sync>>`. The trait is
  **`smysl-core`'s**, not this crate's: `smysl-check` may not depend on `smysl-text` (the `Time`
  pass's `text` feature is the only such edge) and the facade cannot implement a foreign trait
  for a foreign type, so the orphan rule decides it. It names no I/O, and `smysl-text`
  implements it for `ObjectStore` in step 5 and for `Library` when that handle lands in step 6.
  Its answer is a three-state enum (`Absent` / `Part` / `Unreadable`), because absence is
  ordinary and an unreadable object is `E446`. Without a resolver the object codes are skipped
  and the report says so (as `Pass::IMPLEMENTED` does for passes this build does not run).
- `ConformanceClass::Library` (`"C-Library"`, D-8): branches like `Merge`; forbids `E401`, `E403`,
  `E404`, `E446`, **`E452`** and a redaction violation; does not subsume `Produce`. `ALL` gains it
  at the end, and `Full` gains the family. `E452` was added to this list in step 5: a log holding
  text is precisely what OQ-39 refused, so a C-Library consumer must not accept it. `E402` stays
  outside the family — a licence refusing to let text travel describes a correct store.
- Fixture tree `fixtures/library/check/` in the existing `.smy` + `.expected` format. `E452`,
  `E446` and `E401` are **not** reachable from a `.smy` file — records 15 and 18 have no surface
  form and the object codes need bytes — so those are tested programmatically, as rule T's
  `E033` already is: `E452` in `tests/library_check.rs`, and the two object codes in
  `crates/smysl-text/tests/object_check.rs`, beside the only `PartResolver` implementation.

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
| `ingest --text <alias\|mid>[#range]` | `--lang-policy source\|pivot:L [--reextract] [--digest-budget N] [--window-level L] [--single-pass]` | model | P5 |
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

- **Purity gate** (OQ-37, answered 1.10.0; **done in TX-P1 step 2**). `smysl-text` joins
  `PURE_CRATES` with
  `default = []`, so its default tree has no `serde_json` and the readers needing it are outside
  the gate, as providers are. That was the plan before 1.10 and it was not enough on its own:
  with a feature per reader, "clean at default features" is barely a claim about this crate, and
  a runtime added behind `reader-slack` would have passed. So the gate now holds two lists. The
  runtime and socket crates are checked at `--all-features` as well, because an offline library
  is offline however it is configured; `serde_json` is checked at default features, where the
  claim is that the pure core carries no serde stack. `smysl-text` must also pass the
  `FORBIDDEN_SYMBOLS` grep, which is why the wall-time watchdog lives in `src/main.rs`, not in
  the crate.
- **`--no-default-features` facade tree** stays free of `serde_json`: `text` is not default for
  the library (only through `cli`). Enforced against both lists, since that tree is the library
  a consumer gets without asking for anything.
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
  remain; a merge with a stale peer that still holds record 15 does not bring it back; a bundle
  carrying a record 15 into a plain store is refused with `E452` and the log is byte-identical
  afterwards.
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
  check/           .smy + .expected (E403, W418 in step 5; E410, W411 with the Time pass;
                   E404, W405, W406 with the fields they read — and E446, E401, E452 are
                   not .smy-expressible, so they live in tests/library_check.rs and
                   crates/smysl-text/tests/object_check.rs)
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
| **GE-T2** (extraction stability) | rerun at TX-P5 | under the consensus policy (§3.4), not the single-pass policy draft 3 assumed. Stability is measured as `identical-span` agreement between the two passes — computed, not judged — and reported per language. The judged `J_class` form of GE-T2 waits for GE-T9 with GE-T5, for the reason in the row below. S0's pilot: `J_class` 1.000 local, 0.327 hosted, against a 0.6 threshold. |
| GE-T5 within-language arm, GE-T11 | TX-P7 | as draft 3 §23, **except the threshold** (§1.1 item 5, argued in §0.1). Draft 3's "precision ≥ 0.9 at recall 0.7" is measured against a gold, and S0's gold is three model coders whose binary α is 0.600 [0.550, 0.646] — so 0.9 is above what that gold can support, and the pilot's 0.835 and 0.712 measure the gold rather than the engines. Restated: an engine passes when its agreement with the pooled gold, scored on the coders' own scale, is **not distinguishable from the coders' agreement with each other** — it joins the pool rather than beating it. The absolute bar returns when **GE-T9** supplies a human α, which is still owed; until then every class figure is reported against the model-model ceiling, and `attested:2` is the default (§3.6). The cross-lingual arm of GE-T5 is SMYSL-2.5 and waits for TX-P10's S1 embeddings. |

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

1. ~~`smysl-core`: `Tid`/`Mid`/`Rdid`, records 14, 15, 18, keys, envelope, surface `@manifest`.~~
   **Done 2026-10-08.** All four identities (`Did` too — it costs nothing and TX-P3 needs it),
   `types/library.rs`, the four key tables, encoders and decoders, the `@manifest` reserved word
   with its parser and writer, and `fixtures/library/wire/` with `ids.json` carrying each
   preimage, body and identity apart. The exit held, and two things were found while meeting it:
   - the **format spec was silent** about records this build now writes. §2.6 was held open for
     "the release that writes them" and that release is this one, so A-3 and A-5 are folded in,
     `verify-spec-tables.py` now asserts codes 1..19, and the three C-Read implementations
     carry the names.
   - `is_known` in all three of those implementations was **derived from the name table** and
     documented as whether the implementation understands the record. That was already false for
     code 9 and harmless only because nothing emits a checkpoint; a manifest reported as *known*
     by a reader that decodes none of it is the silence `SMY-W014` exists to break. The two
     questions are now separate tables in each.
1a. ~~`smysl-core`: open `Admission` (SMYSL-2.3 A-8.1) with the same `Unknown` unit-variant design
   that SMYSL-2.1 §4.3.6 uses for the four enumerations opened in 1.9.0; code 255 is already
   reserved.~~ **Done 2026-10-08**, and it cost what reserving 255 in 1.9 promised: no registry
   change, no renumbering, no new diagnostic. The raw code travels on `GranularityProfile`, as an
   unknown source kind travels on `SourceRef`. The exit is met by a round-trip test rather than
   by a new codec fixture: `fixtures/conformance/codec/` holds *defective* inputs with expected
   diagnostics, and an unknown admission is a conforming one, so it belongs in
   `tests/roundtrip.rs` beside the other four. `SMY-W409` is asserted in
   `smysl-check/tests/unknown_codes.rs`, **with its control** — the same body still fails
   `SMY-E040` under the admission this build knows, so a bug that disabled the check outright
   cannot pass as the suspension this one requires.
2. ~~`smysl-text` skeleton: `norm`, `ids`, `objects`, `lock`, `limits`, `manifest`, `part`,
   `reading`, `structure`, `locator`. *Exit:* unit tests of §5.1 for these modules; limits
   suite.~~ **Done 2026-10-08.** All ten modules, 112 tests, and the crate holds no file
   format yet — every function in it is a function of its arguments, which is why all of it is
   testable without a corpus. Four things were decided or found while meeting the exit:
   - **The locator grammar did not exist** anywhere this repository can reach (§3.2). It is
     written down now, with the three refusals it turns on.
   - **`Node.meta` was a node's own index** once the single-table reading of A-5 is taken
     seriously (§3.2). Dropped; `parent` is there instead.
   - **The §5.1 exit as written cannot be met in this step.** "One crafted input per cap per
     reader" needs readers, which are step 3. What `tests/limits.rs` holds instead is one case
     per *cap*, with the arithmetic pinned and two properties asserted across all ten: every
     refusal is `SMY-E440` naming its cap and its flag, and a refusal is reproducible to the
     unit. A reader's crafted input joins that file in step 3 rather than replacing it.
   - **The purity gate got `smysl-text` here rather than in step 6**, where this plan put it.
     The readers land in step 3, and a gate that arrives after the code it is meant to
     constrain is a gate that has to be argued with instead of obeyed. Step 6 keeps the rest:
     the `cc` check in `make crate-features`, and the CLI.

   Two further deferrals, deliberate. There is **no `Library` handle** yet: §4.2's sketch has
   it opening a `Store` and an object store together, and the store work is step 4 — a handle
   written now would be a handle to half a library. And `smysl-text` is **not re-exported by
   the facade**, which is §4.3.6 and belongs with the CLI in step 6; its public surface is
   watched from this commit all the same (`Makefile`'s new `UNPUBLISHED` list feeds
   `api-check`, which needs no registry, while `semver` keeps a list of crates that have a
   published baseline to compare against).
3. ~~Readers `txt`, `md`, `usfm`, `osis`, `zefania`, `json`. *Exit:* `fixtures/library/readers`
   expected mids; reader fuzz targets run 10 minutes each with no finding.~~ **Done
   2026-10-08.** All six, each behind its own feature, with `fixtures/library/readers/`
   pinning every identity — the tid of each part, the structure hash, the rdid and the mid —
   generated and checked by one plain test, as the wire fixtures are. The fuzz target found a
   defect in the first minute (below). What was decided or found:
   - **`Input`, `Params`, `ReadOutput` and `ReadError` were names only.** §3.2 gave the trait
     signature and §4.1 listed the module; nothing said what any of them held. Designed here,
     as the locator grammar was in step 2. `Input` carries bytes and **no name**, so two
     copies of one file under different names name the same part; `ReadError` is not a type at
     all but two code-less `LibError` variants, because every code in `E440`–`W459` is about a
     *corpus* and a reader that cannot read its input is a refusal to begin.
   - **A reader's parameters are recorded nowhere.** `Manifest.reader` is `osis/1` and there is
     no parameters key, so a parameter — which changes a reader's output — would be the
     `part_policy` defect over again: a corpus that means something else on re-read. Key 3 can
     carry them exactly as key 17 carries the part policy, and `reader_field` /
     `parse_reader_field` do that round trip. **No spec change in step 3**, because none of the
     six TX-P1 readers takes a parameter and a format grammar should not widen before anything
     can produce a value for it. A test asserts that every reader in `READERS` declares no
     parameters, so it fails at the TX-P2 commit where `whatsapp/1`'s date pattern arrives and
     the spec, the three ports and this crate have to move together.
   - **A part policy whose boundary level names no node cuts a text into no parts at all.**
     The default level is `chapter` and `txt/1` has no chapters, so `ReadOutput` names the
     reader's own top level and grouping refuses a level that matches nothing. A text that
     silently becomes nothing is worse than a refusal.
   - **The vocabulary question, and one sentence of §3.2 withdrawn** — see §3.2.
   - **`quick-xml` 0.42 declares rustc 1.86**, above the pure tier's 1.85, and cargo quietly
     resolves to 0.41 to keep the floor. Pinned `=0.41.0` with the reason written down: an XML
     parser is not a reason to move the floor of eleven crates. Licences and MSRVs measured
     rather than assumed — `quick-xml` 0.41.0 MIT/1.79, `pulldown-cmark` 0.13.4 MIT/1.71.1,
     `serde` and `serde_json` 1.71 — which closes the "MSRV and licence unverified" note §3.2
     carried for `md/1` since draft 1. All four pinned with `=` per §7.
   - **`serde_json::Value` sorts object keys.** Parsing a chat export into a `Value` would
     assemble its text in alphabetical key order, and the tid would be over a text no reader of
     the source would recognise. The `preserve_order` feature is not available either: features
     unify across a build and `smysl-provider` serialises prompts with the same crate, so a
     flag set for a reader would change what a provider sends. `json/1` therefore walks the
     deserialiser, where entries arrive in the order the bytes have them.
   - **A row that takes its start at the marker which opened it holds its predecessor's
     separator.** One byte, invisible in a two-verse test, and under every span and alignment
     downstream. Found in `usfm/1` by a test that sliced the text instead of comparing one
     expected string; the fix is that a row starts at its first text and ends at its last, and
     it lives in one shared builder so the three structured readers cannot each get it wrong.
   - **The fuzzer's finding, in the first minute: a source may state one address twice.** A
     Zefania file with two verses numbered 1 (a mutation of this repository's own Luther
     fixture) produced two rows with one locator — a table `Structure::build` refuses, so the
     reader was emitting a reading nothing downstream could open. Refused now by the builder,
     naming the address and the offset of the second one. Not merged: merging two verses would
     be this crate deciding which of them a corpus holds.
   - **`pulldown-cmark` 0.13.4 panics on untrusted input, in the one API `md/1` needs.** Its
     `OffsetIter` — the iterator that carries source offsets, which is what the locators are
     made of — reaches `tree.cur().unwrap()` on a `None` while ending a tight paragraph
     (`parse.rs:2199`). The smallest input found is `` "- [:]:`\n \t\t" ``, eleven bytes, and
     the crate's *plain* iterator reads it without complaint, so the defect is the offset API
     alone. A library in this workspace may not panic on bytes somebody else wrote, so the
     parse is wrapped in `catch_unwind` and the panic becomes the refusal it should have been,
     naming the dependency so an operator knows it is not their text that is wrong. The
     alternative was hand-rolling CommonMark's block structure, which is new code in the one
     place untrusted bytes arrive — the argument OQ-37 settled the other way for JSON. The
     wrapper and the `=0.13.4` pin come off together when the upstream fix lands; until then
     the pin is load-bearing in a second sense, because a later 0.13.x could move the panic
     without fixing it. **To report upstream**, with that eleven-byte reproducer.

     This narrows the step's exit, and the narrowing is stated rather than glossed. The fuzz
     target drives **five** of the six readers: `libfuzzer-sys` installs a panic hook that
     aborts before unwinding — deliberately, so a target cannot swallow a panic — so a
     *contained* panic is still an abort under the fuzzer, and `md/1` would report the
     dependency's defect on every run for as long as it is unfixed. `md/1`'s containment is
     covered by a unit test instead, and it rejoins the target at the commit that drops the
     pin.
   - **`md/1`'s sections are flat and its locators are source lines**, both deliberate and both
     recorded in the module. Nesting sections by heading depth needs a level name per depth,
     and then the level a part boundary falls on would depend on which depth a document happens
     to start at; `md/2` is the place for hierarchy, which is what a versioned reader id is for.
4. ~~`smysl-graph` manifest state, heads, by-part maps, adjacency flag. *Exit:* store tests; 1.8
   append timing unchanged for unit batches, manifest-only append `O(batch)`.~~ **Done
   2026-10-08.** `store/library.rs`: manifests by mid, by alias, the superseded set, heads,
   and the by-part index; `SMY-E452` raised by `append`; the adjacency rebuilt only for what
   can move an edge. Both halves of the exit measured:

   | one record per call | a manifest | a unit |
   |---:|---:|---:|
   | at 1,000 records | 7.9 µs | 38.0 µs |
   | at 5,000 | 5.4 µs | 268.6 µs |
   | at 20,000 | 4.0 µs | ~1123 µs |

   The manifest column does not grow with the store; the unit column *is* the store, as 1.8
   documented. Unit batching is unchanged against 1.8's table (1 → 1123 µs against 1336,
   50 → 24.1 against 25, 1000 → 2.9 against 5), so nothing regressed to buy this.
   `tests/append_timing.rs` is the harness, `#[ignore]`d because a timing assertion in CI is a
   flake generator; what the suite pins is a **rebuild counter**, because comparing the
   adjacency before and after cannot detect a rebuild that changes nothing. Four findings:
   - **`by_mid` could only have been empty**, so it is not here. §4.3.2 lists it, fed from
     `source.manifest` — and §4.3.1 gives that field to TX-P5. A map with no source of values
     is exactly what OQ-39's answer removed when it deleted `part_texts` and `readings` from
     this plan, so the same argument applies to a map this plan adds. It arrives with the
     field, in the commit that can test it.
   - **Record 14 does not travel "always".** §4.3.2 says it does; narrowed to the manifests of
     texts a kept unit came from. A bundle is outbound, and "always" would put the sender's
     whole library inventory — the aliases of every text they hold — into a bundle that has not
     one unit from most of them. The by-part index is what makes the narrower question
     answerable, and it did not exist when the sentence was written. Ancestors do not travel
     either: a superseded manifest is not needed to read a span against its successor.
   - **The whole batch is refused on `SMY-E452`**, not the records before the offending one. A
     half-appended delivery would leave a store whose sender cannot say what landed.
   - **The step-3 fixtures pinned the short form of each identity.** `Display` writes 26 base32
     characters and is documented as "not canonical"; `Tid::parse` refuses it, for the reason
     `Uid::parse` does. So the expectations were pinning something nothing could parse back.
     Regenerated against `canonical()`, 52 characters.
   - **The adjacency flag was written as an allow-list, and §4.3.2 told it to be.** Four record
     types rebuild, the rest do not — shorter, and the version this plan asked for. §7's risk
     table asks for the opposite shape, naming what does *not* rebuild, and the two are not the
     same rule: they disagree about every record type neither sentence mentions, which is to say
     about every one added after it was written. The allow-list's omission is a stale adjacency
     and silence; the deny-list's is a rebuild nobody needed. Inverted, §4.3.2 corrected, and the
     test gained the control that distinguishes them — a view, which moves no edge today and must
     rebuild anyway. Found by reading the implementation back against §7 rather than by a failure,
     which is the only way this one shows up: an allow-list is correct until the day it is not.
5. ~~`smysl-check` `Library` pass (E403, E404, W405, W418, E446, E452); `ConformanceClass::Library`.
   *Exit:* `fixtures/library/check` green.~~ **Done 2026-10-08.** Pass 12, appended after
   `CommitmentSupport` as the enum's comment requires; `ConformanceClass::Library` appended
   after `Full` for the same reason; `fixtures/library/check/` with five documents and two
   controls. Six findings:
   - **Two of the six codes cannot be raised by this build, so they are not registered.**
     `E404` is a span past a part's length and `W405` is a locator disagreeing with a span, and
     both read `SourceRef.span` — a field §4.3.1 gives to TX-P5. The registry's own rule is
     that a code nothing can trigger is worse than a missing one, because a reader greps for it
     and finds a promise with nothing behind it, and that rule does not stop applying because a
     plan listed the code under this step. The pass names them in a `SKIPPED` constant with the
     reason, as **text rather than `Code` values** — they are not in the registry, so there is
     nothing to name them with, which is the property the constant is there to keep. The
     registry is 72, not 74.
   - **`E403`'s only carrier is `source.reference`.** §4.3.3 says "in a record or
     `source.manifest`", and `source.manifest` is the TX-P5 field again; every other identity
     in a record is typed (`PartEntry.tid`, `Manifest.parent`, `supersedes`) and cannot be
     malformed. So the check is: a reference whose head claims one of the four text prefixes
     and does not parse as that identity. A manifest's `origin` is a `SourceRef` too and is
     checked with the units — the one case with no unit to hang a diagnostic on, and therefore
     the one a pass over `store.units()` alone would have missed.
   - **`E446` needs bytes, and `E401` comes free with them.** A log holds no text (OQ-39), so
     the object half of the pass runs only when `check` is handed a resolver, and `CheckOptions`
     gains `parts` for it. Once the bytes are verified, comparing `PartEntry.length` to the
     object's length is six lines; §4.3.3's list does not mention `E401` — it was allocated for
     the structure hash and the rdid, which need a reading — but "does not match the part entry
     on re-read" is exactly what a part entry lying about its length is, and C-Library forbids
     `E401`, so leaving it unraisable would have made that row of the table decorative.
   - **`PartResolver` lives in `smysl-core`, which no section says.** `smysl-check` may not
     depend on `smysl-text` (§4.3.3 reserves that for the `Time` pass behind feature `text`)
     and the facade cannot implement a foreign trait for a foreign type, so the orphan rule
     picks the home. The trait names no I/O. Its answer is a **three-state** enum rather than
     an `Option`: "nothing is stored here" and "something is stored here and it is not a part"
     are opposite answers, and folding them together would have made every `carry: ref`
     manifest an error or every corrupt object silent.
   - **C-Library forbids `E452`, which §4.3.3's list omits.** A store whose log holds a part
     text is the one thing OQ-39 decided a log never does; a consumer promising C-Library would
     be reading text from the place the format says text never lives. `E402` is deliberately
     *outside* the family: a licence that refuses to let text travel describes a correct store.
     C-Full gains the family too, being the union.
   - **The manual's pass chapter had been stale since 1.7.** It said "ten passes … seven run
     inside `check`" while `Pass` had eleven and ran eight: the commitment pass never reached
     the chapter that documents passes. Twelve and nine now, with both new rows. Found by
     adding a pass, not by a gate — `doc-output` replays transcripts and `spec-tables` compares
     tables, and a sentence counting the rows of a hand-written table is neither.
6. ~~CLI `text add/ls/show`, `check --library`; purity gate with `smysl-text`; `cc` check in
   `make crate-features`; the facade re-exports the library types; **the manual's surface
   chapter gains `@manifest`**.~~ **Done 2026-10-08.** `smysl-text/src/library.rs` (the handle
   deferred from step 2), the `text` command with its three actions, `check --library`, the
   `text` feature on the facade, the `cc` check, and chapter 6's `@manifest` section. Six
   findings, and the first is the one that mattered:
   - **A text cut into more than one part dropped a byte at every cut, and the whole tree was
     blind to it.** `part::group` took each part's range from its nodes' extent, on a stated
     premise that the boundary nodes cover the text "without gaps or overlaps". They do not: a
     row starts at its first text byte and ends at its last (step 3 made it so, to stop a row
     carrying its predecessor's separator), so every separator between two nodes is in no node
     at all. Node-bounded parts therefore lost those bytes at each cut, and lost the text's head
     and tail outright — `notes.txt` is 471 bytes and its one part was `0..470`. A part is
     addressed by the hash of its bytes, so a text whose parts do not concatenate back to it has
     no span, alignment or locator range that means what it says across a cut. Parts now
     partition the text: the first starts at 0, each later one starts where the previous ended,
     and the last ends at the text's length, so the cut lands in the gap where there is nothing
     to cut through. **Nothing had ever run this**, because all eight reader fixtures produce
     exactly one part and for one part the shift is zero; it took step 6's first multi-part test.
     One fixture's identities moved (`notes.txt`, by one byte), which is the blast radius.
   - **A reading's offsets are part-local and the only code that built one kept them absolute.**
     Same cause, different symptom: `tests/readers.rs` filters a reader's rows into a part
     without shifting them, which is correct for a part starting at zero and nothing else. The
     shift now lives in one function in `library.rs` with a test, and `tests/library.rs` asserts
     that `Library::add` reproduces the four identities the fixture files pin — so the two
     implementations of this arithmetic cannot drift apart again.
   - **`-s/--store` taking a library root is one function, not one per command.** §4.4 says the
     flag accepts a library directory; the way to make that true for `check`, `trace` and
     everything else at once is for the two store readers to map a root to its catalog log.
     Recognised by the `LIBRARY` marker rather than by being a directory, so a mistyped path is
     still an error rather than an empty library.
   - **One log, not one per shard.** §14.2's layout is per-shard and SMYSL-2.8 owns the sharded
     union. A sharding scheme written here would be one chosen before anything can measure it,
     so the marker file carries a layout version instead and `Library::shards()` is the one place
     the lock set is named.
   - **`xtask determinism` registers neither `text add` nor `text show`**, which §4.4 asks for,
     and the reason is in the harness: it runs one fixed argv twice and compares stdout. `text
     add` is a *write*, so its second run legitimately prints `objects 0 written` — the first
     run made them — and `text show` needs a library on disk, which the repository would have to
     commit as binary objects. What determinism they have is pinned harder than the harness
     could: `fixtures/library/readers/` holds every identity for eight inputs across six
     readers, and `tests/library.rs` asserts the library's own pipeline reproduces them. The
     harness needs a setup hook before a writing command can join it; TX-P3's `date set` has the
     same shape, and is the right place to pay for it.
   - **Four gates were wrong, and two of them were wrong in the direction that passes.**
     Adding a command and a feature walked into all four:
     - `make cli-surface` records a positional by matching `[A-Z.]+` inside brackets, so the
       value name `FILE|REF` was recorded as **nothing** — a missing line, which the gate reads
       as correct. Renamed `TARGET`.
     - `scripts/verify-doc-cargo.py` split `[features]` into *physical* lines, so a feature
       whose array spans several lines (`cli` does now) gave `cli = [` with no quoted strings
       and the manifest side came out empty. It failed loudly here only because the manual's
       row was not also empty; a row naming nothing would have compared equal to nothing and
       passed. Parsed by bracket depth now.
     - the **`cc` check this step was supposed to add** was itself written wrong: `cargo tree
       -i <absent package>` prints "nothing to print" and exits **0**, so testing the exit
       status reported a C toolchain that is not there. It reads the output now, requires the
       run to have succeeded, and the pattern was checked against a package that *is* in the
       tree (`-i memchr`) — a grep that never matches is indistinguishable from a clean tree.
       The answer, once it could be asked: no `cc` in the `cli` tree, which draft 3 §18 had
       only ever cited.
     - `every_command_names_the_phase_that_wires_it` accepted `SM-P*` or a release number, so
       `TX-P1` failed it. There are two RFCs with two phase vocabularies now, and the test knew
       about one.
7. Ports: C-Read for 14, 15, 18 and the ids. Step 1 gave all three the **names** and nothing
   else — the spec table reaches them through `verify-spec-tables.py`, and each now reports a
   manifest as named but not understood. What is left is the part that matters: decoding the
   bodies and **deriving tid, mid and rdid** from `fixtures/library/wire/ids.json`, which is the
   first identity work in those implementations since uids, and the only way the domain-byte
   separation gets a second reading. *Exit:* **GE-T1** on Bibles and a JSON series.

### TX-P2 — segments, languages, chats, redaction

1. `segment`, `lang`, `analyze`; abbreviation lists. *Exit:* sentence-boundary F1 on a 500-sentence
   gold set per tier-1 language, proposed bars ≥ 0.97 (en, es, fr, de) and ≥ 0.95 (ru), not set by
   draft 3.
2. Readers `telegram`, `slack`, `whatsapp` with `--pseudonymise`; opt-in `mbox`, `epub`, `html`,
   `pdf`. *Exit:* chat round trip with `raw` intact; `E450` test; fuzz targets.
3. `text append`. *Exit:* growth test (§5.2).
4. Record 19, rule Z in `Store::append`, `text redact`, `@redact`; `E452` refuses a 15 or 18
   offered to a log (OQ-39, so there is no `rewrite_redacted` to write). *Exit:* redaction test;
   P-Z1–P-Z4 harness green; ports decode 19.
5. `proposition::classes` — strict per SMYSL-2.3, `component` with diameter, `attested:n` —
   **moved here from TX-P7 step 1** by §0.1. It is pure, needs only `Store` and records, and
   TX-P5's consensus output cannot be read before it exists. *Exit:* strict class counts match a
   fixture computed by the Python reference (SMYSL-2.3 conformance) and are invariant under record
   order.
6. *Exit for the phase:* **GE-T1** on two chat exports; **GE-T14** curve recorded and default part
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
3a. **Consensus extraction** (§3.4): two passes per window by default, both staged, nothing
   deduplicated during ingest; `--single-pass` recorded in the recipe; `--temperature` above 0
   refused unless `--single-pass` is given. *Exit:* a window extracted twice at T=0 by the
   `Scripted` provider yields the `identical-span` agreement the fixture states, and the same
   window at T=0.7 is refused rather than silently producing two disjoint sets.
3b. **Holder and mode written from structure** (§3.4), not from the prompt, and the holder
   paragraph is not added to the template. *Exit:* the probe set — attribution-bearing passages
   with known reported speech — shows the asserted-as-reported rate **below 10%**, which is the
   threshold T8 fired on at 11.1–24.5%. This is a new phase-exit clause, not a refinement.
4. Ledger digest, `E442`, journal, language policy, recipe fields, prompt templates with fences.
   *Exit:* red-team suite; resume test.
5. *Exit for the phase (draft 3 §22, and changed by §1.1 items 2, 4 and 6):* resume after kill
   loses ≤ one batch; ≥ 95% of units carry a span; entity duplication < 2% across windows; the
   probe-set gate of 3b; **GE-T2 rerun** under the consensus policy (§5.5); and a **cost report**
   from the journal that replaces draft 3 §7.7's planning figures — now with a per-language retry
   rate as a separate term (§3.7), measured **after** F-2, since every non-English figure S0 has
   was taken under the bound F-2 replaced. The phase is not sized from a planning number at all.

### TX-P7 — proposition layer and linking

(TX-P6, `sq`, is SMYSL-2.5. `link` and `summarise` accept alias/mid scopes until it lands.)

1. ~~`proposition::classes`~~ — **moved to TX-P2 step 5** (§0.1), because TX-P5's consensus
   output needs it first.
2. Engines `identical-span`, `lexical` (with adjacency, which S0 showed is load-bearing rather
   than a filter), `anchored`; `same-as propose/classes`. *Exit:* anthology test — a paragraph
   reprinted under two expressions forms one class by `identical-span`.
3. Linker and summary tree. *Exit:* GE-T11 (recall ≥ 0.6 on 50 planted far contradictions at the
   default candidate budget); summary nodes capped by rule M (asserted).
4. *Exit for the phase:* **GE-T5** within-language arm under §5.5's restated threshold — the
   engine's agreement with the pooled gold is not distinguishable from the coders' agreement with
   each other. Draft 1's "precision ≥ 0.9 at recall 0.7, else class measures ship as exploration
   only" is replaced on both halves. The bar was unattainable against a gold of α 0.600 (§0.1),
   and the *else* branch is no longer a branch: class measures ship as exploration only either
   way, by §1.1 item 3, until GE-T9 supplies a human α. What this exit decides is whether the
   engines are at the ceiling, not whether the measures are publishable.

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
| consensus extraction doubles cost and the agreement it buys is only `identical-span` | TX-P5 costs twice as much for a narrower claim than §1.1 item 2 implies | stated rather than mitigated (§0.1): the broader claim needs a judge, and S0 measured the judges at α 0.600. `--single-pass` exists and is recorded in the recipe, so the choice is visible in a corpus rather than assumed |
| the retry term is measured on a corpus built after F-2, and F-2 may not remove every retry | TX-P5 sized from an optimistic `r` | `r` comes from the journal per language, not from a constant, so an underestimate shows up as a measured rate rather than as a budget overrun; the first TX-P5 corpus is sized after the measurement, not before |
| GE-T9's human α never gets bought | GE-T5's restated threshold has no absolute form, and every class figure stays provisional indefinitely | the dependency is stated at both ends (§5.5, SMYSL-2.0 §7 step 8) rather than hidden in a default; `attested:2` and exploration-only shipping are safe in the meantime, and nothing downstream claims a settled class measure |

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
| `SMY-E452` | a record 15 or 18 was offered to a log; text lives in the object store (OQ-39) | `append`, `check` |

`453`–`459` are unallocated. SMYSL-2.3 may adopt `E446` as a C-Library obligation.

**Registered as of TX-P1 step 5:** `E401`, `E402`, `E403`, `W418`, `E440`, `E445`, `E446` and
`E452`, in a ninth diagnostic group (`Group::Library`) and in the manual's Appendix B. Step 2
registered the first five, step 4 added `E452` and step 5 `E403` and `W418`. Eight, and not the
twelve above plus the draft-3 codes this RFC emits, because those are the ones this build can
raise; the rest enter the registry with the pass, reader or ingest path that raises them —
including `E404` and `W405`, which step 5's own plan listed and which read a field TX-P5 adds. That is the repository's own rule — a code nothing can
trigger is worse than a missing one, because a reader greps for it and finds a promise with
nothing behind it.

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
| ~~OQ-36~~ | **Answered 1.10.0: `create_new`, and the parenthesis is now measured.** `File::lock` is unstable at 1.88 and compiles at 1.92, so it would raise a pure-path floor above anything the dependencies need — an `EXCEEDS` entry argued for a lock — and `fs4` pays a dependency for the same thing. Independently of cost: an advisory lock cannot name its holder and `E445` must, and a lock the kernel drops on process death erases the only evidence that a writer died mid-append. §3.9.3 |
| ~~OQ-37~~ | **Answered 1.10.0: `serde_json` behind `reader-json`, and the purity gate's list was the thing that needed fixing.** A strict mode of the HJSON parser is not a narrowing of what is there; measured against today's `parse_object`, it is five pieces of new code in the place untrusted bytes arrive. A surrogate pair `\uD83D\uDE00` is **refused** (`invalid code point`), which is how `json.dumps` writes every emoji by default. `9223372036854775808` becomes `Float(9.223372036854776e18)`, so a message id silently stops being an id. `1e400` becomes `Float(inf)`. A bare `nope` becomes `Str("nope")`. And `{ "t": 1 "u": 2 }` — malformed JSON — parses as `Str("1 \"u\": 2")`, the quoteless rule swallowing the rest of the line; a silent misparse where a reader owes an error is worse than the refusal. `serde_json` meanwhile is not what rule B is about: no runtime, no socket, no clock, deterministic, floor 1.71 — below this workspace's base, so it raises nothing. It was on `FORBIDDEN_DEPS` for a real reason (no serde stack in the pure core) that the list did not record, beside crates forbidden for a different one. `xtask/src/purity.rs` now holds two lists with their reasons, and checks the stronger one at `--all-features` as well — a hole the single list left, and one TX-P1 would have widened. §3.2, §4.5 |
| OQ-38 | Is a deterministic fuel cap enough, so the wall-time watchdog can be dropped, or is the watchdog worth keeping as a backstop? |
| ~~OQ-39~~ | **Answered 1.10.0: refuse, and `rewrite_redacted` is never written.** An append-only log that can be rewritten is not append-only, and the rewrite is unauditable — the operation that honours the redaction resets the same hash chain that would have shown an edit, so the log can no longer tell the two apart. The capability given up is nearly empty: a plain `Store` holding carried text can neither resolve a locator nor check a span, so it can only hold the bytes. And nothing has to be migrated, because records 14, 15, 17, 18 and 19 do not exist in 1.10.0 — the record enum stops at 13 — so this is decided before any byte depends on it. `SMY-E452`; §3.1, §4.3.2 |
| ~~OQ-40~~ | **Answered 1.10.0, and the question was wrong.** Neither: 1.79 was already false everywhere — the pure core cannot be *parsed* by a 1.79 Cargo, because `blake3` pulls an edition-2024 `constant_time_eq`. The measured floors (1.85 / 1.86 / 1.88) are declared per crate and gated by `make msrv`. `redb` 4.x's 1.90 is an ordinary bump in the release that ships it. §4.5 |
| OQ-41 | Removing `ingest:quote` from span-carrying text units changes their uids relative to prose ingest of the same text. Accept, or keep the quote and accept duplicate uids per span? |
| OQ-42 | **Resolved in SMYSL-2.3 A-12.2:** strata by status; a chain tightens at its weakest link's status. §3.3 implements exactly that. |
| OQ-43 | Should the digest-scoped reference check (`E442`) also admit units of the same expression not shown in the digest (a model that remembers an earlier window), or stay strict? |
| OQ-44 | `lexical` same-as proposals: auto-accept above a threshold at rung `computed`, or always require a judge (model or human)? |
