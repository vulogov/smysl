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
| 6 | FC-6 weights calibrated on M5; cost model revised | §3.7, §6 TX-P5 | **the weights are not what is miscalibrated.** S0 names the mechanism and it is the repair loop. Calibrating weights on pre-F-2 data would fit the estimator to a defect 1.10.0 removed. **Landed 2026-10-12** as a measurement rather than a revision: the cost model's `r` comes out of the journal per language (`ingest --text --cost`), and no FC-6 weight moved. |

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
  `tls-pure`, and `substrate-redb` since 1.10 (§3.8: the facade's name for `smysl-text`'s, so
  that a binary can be built with the index's backing store). Workspace `rust-version = "1.85"` since 1.10.0 — `1.79` when this was written,
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
- `fixtures/`: `corpus/` (F1–F13), `conformance/check` (`.smy` + exact `.expected` code sets, with `check/time/` joining the
  suite only when the feature-gated `Time` pass runs) and
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

**GE-T14 ran in TX-P2 step 6, and the default is now 1 KiB / 4 MiB.** The curve, over a year of
chat (365 days of 40 messages), a revised article (40 paragraphs) and a 1,189-chapter Bible:

| `target_min` | chat parts | mean part | redaction removes | rewritten on growth | on a re-length | manifest | catalog / chat-year |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 KiB | 365 | 4,041 | 1 day | 2 | 185 | 40 KB | 7.0 MB |
| 4 KiB | 197 | 7,487 | 2 days | 1 | 100 | 22 KB | 3.8 MB |
| 16 KiB | 73 | 20,205 | 5 days | 2 | 37 | 8 KB | 1.4 MB |
| 64 KiB | 22 | 67,045 | **16 days** | 1 | 12 | 2.5 KB | 0.4 MB |
| 256 KiB | 6 | 245,831 | 61 days | 1 | 4 | 0.7 KB | 0.1 MB |

Three of the four axes turn out not to depend on the size at all:

1. **Growth costs at most two parts, at every size** — the part that *was* last, which gains the
   separator joining it to its successor (step 3), and a new one where the last group did not
   absorb the new nodes. So the minimum buys nothing for the case a chat corpus is in every day.
2. **An in-place correction costs one part if its length is unchanged, and every part after it if
   it is not** — also at every size, because what decides it is whether the bytes after the edit
   kept their offsets. Smaller parts make the second case *worse* in object count: 185 parts
   rewritten at 1 KiB against 12 at 64 KiB, for the same correction.
3. So the only genuine trade is **object count and catalog size against redaction granularity**.

The decision follows from what the two remaining numbers are *about*. The boundary level is the
structural judgement the reader's author already made — a day, a chapter, a paragraph — and a
minimum above the typical node size silently overrides it: at 64 KiB a part is sixteen days
whatever the reader said. So the minimum's job is only to stop pathologically small parts, and
its value belongs **below** the node sizes that matter (a chat day and a Bible chapter both
measure about 4 KB). 1 KiB is below them, so the level governs and a redaction removes about what
it names. 4 KiB sits *at* them, where the grouping flips on fifty-seven bytes — the one value to
avoid. The cost is 7 MB of catalog per chat-year and 2,378 objects for a Bible, against a
redaction that takes one day instead of a fortnight, and that exchange is settled in favour of
the obligation that comes from outside the system.

`target_max` is unchanged and, at this minimum, inert: coalescing stops at 1 KiB, so the ceiling
binds only on a single node larger than it, with `Caps::part_bytes` behind that.

**The measurement is synthetic, and that is the honest instrument here.** GE-T14's row asks for
"real chats, revised articles, Bibles"; this repository holds three chat exports of two days and
five verses of Genesis. But all four axes are functions of the **structure alone** — the number
of parts, which of them a revision moves, how many bytes one part holds, and how long a manifest
is depend on the node sizes and their order and on nothing about the words. So a generated corpus
whose node-size distribution matches the real shapes measures the policy exactly, and what stays
owed is only whether those distributions are right — which anybody with a real corpus can check
in an afternoon, and which the fixtures anchor at one real point each (a telegram day is 113 and
235 bytes; a Genesis verse is 55–97). The harness is
`crates/smysl-text/tests/part_size.rs`, and it **asserts** the curve rather than printing one, so
the table above cannot rot.

**Growth and revision.** `text append <alias> <file>` re-reads only the new input, reuses tids for
unchanged parts (object already present: no write), and appends a manifest with `supersedes`. Two
heads under one alias are `SMY-W418`, reported by the library pass of `check` and by `text ls`.

**Built in TX-P2 step 3, and "unchanged" needed one word.** A part that **was the last one** is
rewritten exactly once. Parts partition the text — a cut may lose no byte, which is what TX-P1
step 6 fixed — and the separator between two parts belongs to the earlier of them, so the day
that was last gains the space that now joins it to its successor and its tid moves. Measured on
a three-day export cut per day: day one is `0..306` in the one-day export and `0..307` in every
export after it, and `0..307` is the same bytes in the two-day and the three-day one. So the
cost of an append is **one rewritten part, whatever the size of the corpus** — four objects, not
2·n — and every part before it is untouched. `objects N written` reports it.

The alternative was considered and not taken: give the separator to the *later* part, and every
earlier part is reused with nothing rewritten. It would also make every part but the first begin
with whitespace — visible in `text show --raw` and in every span offset inside a part — and move
every tid ever computed. The cost that was measured is `O(1)` per append; the one that was not
is in every reading.

Four further things the step settled:

1. **Tid reuse is a property of the part policy.** Under the 64 KiB `target_min` that was the
   default when this step landed, a chat export of a few kilobytes is *one* part, and one part
   that grew is a part with a new tid — so an append of such an expression rewrites its only
   object and reuses nothing. Correct, and not nothing: an operator who wants per-day parts says
   so in `--part-policy` at `add`, where it is recorded in the manifest and reused by every
   append afterwards. The library's doc comment and the manual both say so rather than leaving it
   to be discovered. **Step 6's GE-T14 moved the default to 1 KiB**, where a day *is* a part and
   this paragraph's advice is the default — and it moved it partly on the strength of this
   finding, since a minimum that buys nothing for growth is a minimum spending redaction
   granularity for nothing.
2. **An append that changes nothing writes nothing.** Re-running a sync before the export has
   grown is routine; without this it would write a manifest whose only difference from the head
   is that it supersedes it, which makes `supersedes` mean nothing. The head's own part *entries*
   are compared against the ones just built — entries, so "the same text read the same way"
   rather than "the same bytes" — and a match returns the head's mid with no record written.
3. **The spelling is `text append <file> --alias A`**, not `text append <alias> <file>`.
   `TARGET` is one positional that already means three things depending on the action; making it
   mean two things *at once* for one of them is the shape nobody can read back out of `--help`.
   `add` and `append` now put the same two pieces of information in the same places. The flags an
   append takes from the head manifest — `--reader`, `--licence`, `--lang`, `--carry`,
   `--part-policy` — are **refused** rather than ignored, because an ignored `--part-policy` is a
   second version somebody believes was cut one way and a corpus cut another.
4. **Appending to one side of a fork does not merge it.** `text append --alias A` on a forked
   alias is refused by name; naming one head by mid appends to that side, and the other head is
   still a head, so `SMY-W418` goes on being reported. Merging two heads is a different
   operation from growing one.

**Carry.** `bundle`/`text` commands refuse to emit 15/18 unless manifest key 5 is `text` and key 4
is `public-domain` or on the permissive list compiled into `smysl-text` (`licence::PERMISSIVE`, a
sorted SPDX list; changing it is a code change with a test). `SMY-E402`, no override flag.

**Redaction (rule Z).** Implementation in §4.3.2. Object deletion happens after record 19 is in
the catalog log, so a crash between the two is repaired on the next open (the open path enforces
Z against the object store).

**Built in TX-P2 step 4, and the rule moved one layer down.** Record 19 is decoded by this build
and by the three ports, it has a surface form (`@redact <tid> { agent, ts[, reason] }`), rule Z is
normative text in the format spec beside the record's key table, and `text redact` writes it.
Five things the step settled:

1. **Rule Z's enforcement point is the object store, not `Store::append`.** §4.3.2 puts a filter
   in `append` that drops a *redacted* 15 or 18 and counts it. That filter cannot run: `append`
   refuses **every** 15 and 18 with `SMY-E452` before it, redacted or not, because a log is not
   where text rests (OQ-39). So the drop-and-count belongs where the bytes actually are —
   `Library::add` withholds the objects for a redacted part, counts them in
   `Added.objects_redacted`, and still writes the manifest. `Store`'s job is to hold the
   redactions so the object store can ask: `is_redacted`, `redactions`, `redacted_count`.
2. **Drop rather than refuse, and that asymmetry is the design.** A peer that never heard of a
   redaction will offer the bytes in good faith on every merge; refusing would make one
   redaction anywhere in a network a permanent merge failure, and the workaround for that is to
   stop merging. The spec says so in rule Z's own words.
3. **Opening a library is a write, in exactly one case.** `redact` appends the record and *then*
   unlinks, so a process killed between the two leaves a catalog that has said what it is doing
   and bytes that have not caught up. `Library::open` is where that is put right, which also
   answers §4.3.3's "a redaction violation" with **no code**: the violation is unrepresentable
   rather than reportable — a `check` that opened the library would find the objects already
   gone — and a code nothing can raise is worse than none.
4. **A reading goes with its part.** Rule Z is about 15 **and** 18. A reading holds no text, but
   it holds the offsets, the speakers and the ids the text was read into, and it is the half
   that names people.
5. **A redacted passage is a sentence, not a missing file.** `LibError::Redacted` (no code, for
   the reason `NotText` has none: nothing is wrong) is what `text show` prints, where before it
   would have printed `No such file or directory` and an object path — which reads as a corrupt
   library rather than as a corpus doing what it was told.

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
| `telegram/1`, `slack/1` | `reader-telegram`, `reader-slack` | TX-P2 | `serde_json`; Slack also `zip` — **verified in step 2**: `=4.3.0`, not 9.0.0, which declares rustc 1.88 against this tier's 1.85 | built |
| `whatsapp/1` | `reader-whatsapp` | TX-P2 | hand-rolled | built; the date order **is** a required parameter, and it is why manifest key 3 now records a reader's settings |
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

**Built in TX-P2 step 1, and the lists turned out to be one rule of four.** Measuring UAX #29
against prose in the five languages found three further things it does, each of which loses or
invents a sentence, and version 1 of the id is all four because nothing has been written with it
yet:

| # | rule | what it is for |
|---|---|---|
| 1 | *soft wrap*: a single `\n` that no terminator precedes is not a boundary | UAX #29 breaks at every `\n` (SB4), so hard-wrapped prose came out one sentence per line. A blank line still ends a sentence. |
| 2 | *abbreviation*: the lists, plus a rule for single uppercase initials | `Mr. Smith`, `M. Dupont`, `Abb. 3.`, `ул. Ленина`. |
| 3 | *continuation*: a boundary before a lowercase word is suppressed | `He said "Stop!" and left.` — SB8 covers this after `.` but not after `!`, and a closing quote puts it out of reach either way. |
| 4 | *closing marks* (a reattachment, not a suppression): a leading run of marks that cannot open a sentence joins the sentence before it | French spaces its quotes: `« … jamais eu. »` ends at the `»`, and UAX #29 ends it at the period, handing over `» Sa sœur posa…` as one piece. Suppressing that boundary would have joined two sentences; moving the mark joins nothing. |

Two things a list cannot settle, recorded rather than resolved: German `f.` was **removed**
because `Vgl. S. 14 f. Dort steht es.` is two sentences and holding on `f.` loses the second,
and Russian `г.` was **kept** although `умерла в 1911 г. Ты сама…` loses a boundary the same
way — `в г. Тверь` needs the hold, and which is more frequent is a question about a corpus.
The 500-sentence gold set the step's exit asks for is what prices it.

**Chats.** One segment per message, one part per UTC day by default (`--part-policy` overrides).
Reply and thread ids go into the segment table as substrate edges. Pseudonymisation:
`spk:` + 26 base32 chars of keyed BLAKE3 (`blake3::keyed_hash`) of the platform user id. The key is
read from `<library>/secrets/pseudonym.key` (created with 32 random bytes on first use, mode 0600,
never logged). `text append` to a pseudonymised expression without the key is `SMY-E450`, because
new messages from the same person would otherwise get a different speaker.

**Built in TX-P2 step 2, with six corrections to the paragraph above.** The structure is
`day` → `message`, or `channel` → `day` → `message` for Slack, whose export is one archive
holding every conversation. The locator is `head.yyyymmdd.n` — `general.20240115.7` — where `n`
is the message's ordinal **within that UTC day**: Telegram has a message id and Slack has a
timestamp that works like one, but WhatsApp has nothing of the kind, and a locator a person
types in a citation cannot be `1705314225.000200`. The platform's own id is kept in the row's
`ids` instead.

1. **"One part per UTC day" is a boundary level, not a part count.** The part policy's
   `target_min` was 64 KiB when step 2 landed, so whole days were *grouped* into parts and a
   quiet month was one part. A day is never split; that is what cutting on the day level buys —
   and step 6's GE-T14 moved the minimum to 1 KiB, where a day is a part. The sentence above
   reads as a promise about parts and is a statement about boundaries.
2. **The day cannot come from the file.** Slack's export is one JSON file per channel per
   **local** day, so a `2024-01-15.json` holds messages belonging to two UTC days — the
   fixture has one at `02:00Z`. A reader that trusted the file name would cut a part on a
   boundary that is in no field of the data, so a channel's files are concatenated and the day
   is taken from each message's own timestamp. Telegram has the same trap in a different shape:
   its `date` field is a **local wall clock with no offset**, and only `date_unixtime` is an
   instant.
3. **Pseudonymisation does not change a single tid**, because the speaker is a field of a row
   and never a byte of the text. A pseudonymised library and a plain one hold the same parts
   and the same objects; only the reading differs. That is a design decision and not a
   consequence — the reader a person would naturally write renders `Alice: hello` into the
   text, and then `--pseudonymise` would be a different ingest of a different document. It also
   means **names inside the prose are untouched**: a mention, a signature, `<@U024BE7LH>`.
   Those are `text redact` and rule Z's business (step 4), which is right, because removing a
   name from the text *does* change the tid and therefore has to be recorded as a redaction
   rather than as a setting.
4. **It changes the `structure` hash as well as the `rdid`**, which was not expected. A-5
   defines `structure` as the hash of "the canonical CBOR of the structure table carried in the
   reading's segments", and that table is one row per node with every field a reader filled —
   including the speaker. So `structure` is a hash of the whole reading table rather than of the
   tree, and `SMY-E401`'s `which: "structure"` means "the table changed" rather than "the
   segmentation changed". Narrowing A-5 to the tree alone would be an amendment to the
   normative document and would move every structure hash ever computed, so it is recorded here
   as an open question (**OQ-70**) rather than decided in a reader's commit.
5. **The library layer cannot mint the key**, so it does not. `AddSpec` takes a
   `pseudonym_key`, not the `bool` §4.2's sketch has: a `bool` would mean `smysl-text` has to
   find the key, which means creating it when absent, which means a randomness source in the
   pure tier's default tree — the claim TX-P2 step 1 wrote into the purity gate when `lingua`
   brought `getrandom` with it. `smysl_text::secrets` knows where the key lives and what shape
   it has; the facade's `text` feature supplies the thirty-two bytes. A library that is a
   function of its arguments cannot invent a secret.
6. **`SMY-E450` waits for `text append`** (step 3), because the registry's rule in force is
   that a code nothing can trigger is worse than a missing one. The detection needs no new
   manifest key, which is worth recording now: a reading whose speakers are pseudonyms says so
   in its own rows (`speaker::is_pseudonym`), so step 4 can ask the corpus rather than a flag.

Two things the readers deliberately do **not** do, both for the same reason — a reader that
guessed would be undetectably wrong:

- **WhatsApp's media placeholders are kept as text.** `<Media omitted>` is localised
  (`<Без медиафайлов>`, `<Medien ausgeschlossen>`), so a list of them is wrong for the next
  locale, and it was the *exporter* that dropped the photo. `lossy` means "the reader dropped
  something the source carried", and the source carries a placeholder.
- **A display name is never a speaker.** Slack's `users.json` maps `U024BE7LH` to a name and is
  not read for it; Telegram's `forwarded_from` is a second person's name with no id beside it to
  key, so it is dropped and the read marked lossy. The speaker is the platform's id, which is
  what a pseudonym can be derived from and what does not change when somebody renames
  themselves.

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

**Landed 2026-10-12** (§6 TX-P5 step 3a), with the refusal **before egress**: a caller who asks
for two passes at T=0.7 is told so by `Ingestor::ingest` before a single call is made, beside the
prompt-override and granularity checks, rather than after paying for two answers that cannot be
compared. The agreement is recorded by the run rather than recomputed from the batch, because the
batch cannot answer it — two passes that found one claim produce one uid. Step 3a's note in §6
carries the four points that decided the shape.

**Holder and mode are structural from the first ingest** (§1.1 item 4). Draft 1 left this to the
prompt. S0 measured reported speech extracted as asserted in 11.1–24.5% of attribution-bearing
units across every model and coder pairing, every cell above the 10% threshold (T8), and the arm
that *added* a holder paragraph to the prompt did no better (23.8% and 16.7%, T9 does not hold)
while introducing `E022` degradations into a configuration that had none (MS-9, MS-11). So the
paragraph stays out of the prompt, `text:holder` and `text:mode` are written by the converter from
structure — reported-speech spans the reader marks, not the model's self-report — and TX-P5's exit
gains a probe-set gate (§6). The readers mark them from TX-P5 step 3b's reader work: `\wj … \wj*`
in USFM and `<q who="…">` in OSIS, both into reading **key 9**, a list of `{start, end, who?}` on
the row whose range holds them. A mark establishes `reported` and no holder even when it carries a
`who`, because the mark says somebody else is speaking and not that this corpus asserts the
edition's attribution of it.

**Absent quotes (behaviour change).** For `ingest --text` only: repair is attempted first, within
the existing budget. If the quote is still `Absent`, the unit is staged with no span and its status
capped at `inferred` **when it has grounds**, `speculative` when it has none — `UnitCoreBuilder`
refuses `inferred` without grounds (`ShapeError::GroundsRequired`), so draft 3's plain "capped at
inferred" cannot be built. Reported as `SMY-W441`. Prose ingest keeps `E307` non-local, as
`repair.rs` argues.

**Landed 2026-10-12** (§6 TX-P5 step 3), as `smysl_ingest::text::{window, attach}` behind the
`text` feature: windows of whole nodes with one node of overlap and owned ranges that partition
the text, the four-scope search for a quote, `SMY-W419` where the node named is itself ambiguous,
the cap above, and the removal of `ingest:quote` and `ingest:node` **only** from units that
received a span. Two pure functions over a reading and a batch; no model is called and nothing is
persisted, which is why the whole of it is tested without egress. The repair turn the paragraph
above names is `repair.rs`'s and is unchanged — attachment is what happens after it, and what
decides whether it was enough. Step 3's note in §6 carries the five points that decided the
shape.

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

**Built in TX-P7 step 3**, as `smysl_text::link` (the candidate generator, pure),
`smysl_ingest::link` (the judgement and the staging) and `smysl_ingest::summarise`. Five things
the two bullets above leave implicit and the implementation had to decide:

1. **Adjacency is off in the linker, and that is the whole difference from §3.6's engines.**
   `proposition::Lexical` filters its neighbours to the same or a neighbouring verse because S0
   measured that filter carrying precision from 0.789 to 0.835. A linker that did the same could
   not find what it is for: GE-T11's word is *far*, and the pairs the filter removes are exactly
   the ones a reader cannot find by reading forwards. The precision the filter was buying is
   bought here by the judge instead, which is the thing the linker has and the engines do not.

2. **A hub entity confirms and does not raise.** The first cut dropped entities that too many
   units mention — the inverse-document-frequency argument, and right as far as it goes. It also
   dropped the *conjunction*: a pair that another source already found and that also shares a
   name is a pair two independent kinds of evidence point at. Over a corpus that is one story
   about four people that is every pair worth having; in Ruth, everything mentions Naomi.
   Measured, the cutoff then moves neither the recall nor the candidate count between 4 and ten
   thousand, which is how one knows it was the wrong lever and the conjunction was the right one.

3. **The candidate budget is three numbers, and all three are in the recipe.** `per_unit` spends
   model turns, `neighbours` spends arithmetic, `hub` bounds what one name may cost. §3.5 says
   the budget is a recipe field and the reason is corroboration — two runs at different budgets
   were shown different pairs — so all three are hashed, not only the one that spends.

4. **`unrelated` is a verdict and the prompt spends its length on it.** At sixteen candidates per
   unit most pairs are merely near each other. A judge that felt it had to find something would
   turn every near pair into an entailment, and that — not misreading a contradiction — is the
   failure a pairwise judge actually has.

5. **The summary tree is the reading's tree.** A pass that split locators on dots would agree
   with the reading for USFM and disagree with it for every other reader, silently. So
   `summarise` takes the library and the expression as well as the levels, which §4.2's
   `summarise(&self, levels, scope)` has no way to do, and each level after the first is grounded
   on the level below rather than on the verses again: four sentences in and one out, with the
   chain of grounds reaching the text through the tree.

Rule M is **not** implemented in the summariser. `stage::prepare_under` applies it, exactly as
for an ingest batch, and that is the whole reason the cap can be trusted — a second
implementation of rule M, in the one pass whose output is likeliest to overclaim, is the one
that would be wrong. The visible consequence is that a summary's uid is not known until it is
staged: weakening moves the core, and `prepare_under` re-points the `x.text/summarises` edges
onto the new uids. `SummaryReport::capped` counts it, because a tree where every node was
lowered is a tree built over material that does not support it.

**Landed 2026-10-12** (§6 TX-P5 step 4), for the digest and the journal: `text::digest` packs
`smysl_pack` over what the expression already holds, ranked by salience then reading order, and
returns the text, the digest-local labels, the allowed uid set and a hash over what was sent. The
labels are `l/n1 … l/nn` rather than `l/1`, because A-3's grammar refuses a label segment that
begins with a digit. The per-window hash is in the journal entry, as this section says, and the
*policy* — budget and ranking id — is in the recipe. `link` and `summarise` are TX-P7's.

### 3.6 Proposition layer (draft 3 §9)

- **Edges.** `x.text/same-as` relations, staged, attested. Engines that are pure attest at rung
  `computed` as agent `tool:smysl-same-as-<engine>` (spelling per `AgentId` rules, unverified).
- **Classes.** `strict` follows SMYSL-2.3's normative definition (D-7): members in uid order, each
  unassigned unit opens a class and admits later units adjacent to every member; class id = smallest
  uid. `component` (union-find, reported with diameter) and `attested:n` (strict over edges with
  ≥ n distinct attesting agents) are tool-level. Withdrawn edges are excluded through
  `Store::is_withdrawn`. **The class core moves to TX-P2** (§0.1): it is pure, needs only records,
  and TX-P5's consensus output cannot be read until it exists.
  - **Built in TX-P2 step 5**, as `smysl_graph::proposition` — see that step for why the crate is
    `smysl-graph` and not this one. `component` is a breadth-first search rather than a union-find:
    the diameter is the thing being reported, a union-find gives components without one, and the
    search that gives the diameter gives the components on the way. Four further rules the
    paragraph above leaves implicit and the implementation has to decide: an edge whose endpoints
    are not both **present** is not live (as a withdrawn one is not); a **self-edge** is true and
    is not a class, since A-12.4's vertex set is "units with at least one edge"; only
    `x.text/same-as` builds a class, so `x.text/translates` and the kernel kinds do not; and
    `attested:n` counts **distinct agents**, so one agent attesting twice is one agent.
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
  - **Built in TX-P7 step 2** as `smysl_text::proposition`, beside the retrieval and the
    alignment they need, with the counting left in `smysl-graph` where a checker can reach it.
    Three rules the paragraph above leaves implicit and the implementation had to decide. A unit
    with no **span** is outside `identical-span` — its whole argument is "these bytes", and
    `anchored` is the engine for a unit located by name. A unit with no **`lang`** is in no
    language group and `lexical` does not propose it: absent means "the language of the view
    that presents it" (A-1), which is not a fact about the unit. And two places with no locator
    are adjacent when their spans **touch or overlap in one part**, which is adjacency in the
    only sense available without a structure tree; two units with no place at all are not
    adjacent, because a filter that passed everything it could not evaluate would report the
    unfiltered precision under the filtered name.
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
- **Built in TX-P2 step 1. Three corrections, all from running the thing:**
  - **Step 2 does more than the plan credited it with and step 4 less.** `caseless` is full case
    folding, so `ß`→`ss` is *its* doing and not an extra of ours; `¿ ¡` never reach the chain
    because `unicode_words` drops punctuation at step 1. What is ours is `ё`→`е`, which case
    folding leaves alone. Measured: `книгами → книг`, `continuellement → continuel`,
    `Häuser → haus`, `aujourd’hui` one token.
  - **`rust-stemmers` 1.2.0 depends on `serde` and `serde_derive` unconditionally**, with no
    feature to turn them off, and OQ-37's answer is that the pure core carries no serde stack.
    §4.1 listed the crate as a plain dependency; `cargo tree` is what found that the two cannot
    both hold. Step 4 is therefore behind `smysl-text/stem`, which `cli` turns on because
    `reader-json` has already paid for `serde` there. A build without it has one chain for every
    language and **says so in `Chain::id`** (`smysl/an/1`), so two incomparable indexes cannot
    claim the same analyzer.
  - **`lingua` is behind `smysl-text/detect`, and §4.5's "unverified figure" is now measured**:
    five language features bring **57 crates and 14.4 MB** of compiled-in models, among them
    `rayon`, `getrandom` and `wasm-bindgen`. No C toolchain and no MSRV move (the maximum
    declared floor in that tree is 1.85, the pure tier's own). The four are added to the purity
    gate's `NOT_IN_THE_CORE` list, permitted through `detect` and nothing else.
- **Detection is never automatic**, which the plan did not say and the identity rules require: a
  manifest's `lang` is part of its mid, so a fallback that only builds with `detect` had would
  give the same file two different manifest ids. `lang::Detector` is something a caller runs, and
  its answer becomes an ordinary `AddSpec::with_lang` — a guess turned into data before it turns
  into an identity. The detector id carries the `lingua` version (`smysl/lingua-1.8.0/1`) because
  §7's risk is that a version decides an rdid.
- **The short-text threshold is a precaution, not a measurement.** `MIN_CHARS = 30`: below it a
  row inherits its neighbours' language. The development set cannot locate it — all 146 sentences
  of `fixtures/segment/gold/` are identified correctly, 32 of them under the threshold. What *is*
  measured is the failure mode: on twenty one-word utterances the detector disagrees with the
  language the word was taken from eight times. GE-T1's chat corpora are what can price the
  number, because a chat message is the short row it exists for.
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
- **Landed 2026-10-12** (§6 TX-P5 step 5): `smysl_ingest::text::cost`, read by
  `ingest --text <who> --cost`. `r` is `retries / first attempts` — repair turns over the calls
  that were not repairs, because `base × (1 + r)` prices a first attempt — and the denominator
  matters: over *calls* a window that repaired once would read 0.5 where the model wants 1.0.
  The journal carries the language, the call count and the repair count per window, so the rate
  is grouped rather than averaged: a corpus sized on a total would under-price every Russian
  window in it by S0's 3.38x ratio. Two **measured routes** to a projection exist and must not be
  multiplied — `tokens_per_window` already contains the repairs it measured, and `(1 + r)` prices
  a projection made from the estimator's `base` over text nobody has extracted — which the module
  says in the one place a caller reads before using either.
- **The path choice was the same byte-based mistake F-2 fixed, in a second place.** A text ingest
  chose json-ast or surface from its widest window's *byte* length against a 4 KiB bound, so one
  expression in Russian took the surface path where the same passage in English took json-ast.
  `path::choose_tokens` asks it in tokens. Found by running the phase exit in two languages; see
  §6 TX-P5 step 5.

### 3.8 Substrate index (draft 3 §14.1–14.3)

Tables built in this RFC: `manifests`, `parts`, `by-part`, `redactions`, `segments`, `locators`,
`align`, `said`, `composed`, `observed`, `postings`, `journal`. (`vectors` and `queries` are
SMYSL-2.5; `imported`, `waiting` SMYSL-2.7; `schemas` SMYSL-2.5.) Each derived table has a
builder `fn rebuild(&Library) -> Table` and an incremental `fn on_append(&Library, &[Record])`; a
test asserts rebuild and incremental produce the same bytes.

**Amended in TX-P4 step 1** (§6), on three points found by building it:

- The incremental builder takes the **library** as well as the records. Three of the tables are
  built from records 15 and 18, which a log never holds (OQ-39): the records say what changed and
  the library is where the changed thing is read from.
- The backing is **in-memory maps plus a deterministic sidecar**, `index/substrate.idx`, checked
  against the catalog's length and hash on open. The sidecar costs one rebuild rather than one
  per open, which is better than the featureless path this section described.

  ~~`index.redb` behind feature `substrate-redb` is **owed**: `redb` 4.3 requires Rust 1.90,
  Cargo gives `rust-version` per package rather than per feature, and `smysl-text` is a base-tier
  crate at 1.85 — so the feature would move the floor for every build of the substrate.~~
  **The floor argument was wrong, and it was wrong about one release rather than about redb.**
  Checked 2026-10-11:

  | redb | declares |
  |---|---|
  | **2.6.4** | **1.85** — exactly this workspace's base |
  | 3.0.0 | 1.89 |
  | 4.0.0 | 1.89 |
  | 4.3.0 | 1.90 |

  So the version is a choice and not a wall, and the workspace already makes this choice twice
  for the same reason: `quick-xml` is pinned at `=0.41.0` because 0.42 declares 1.86, and `zip`
  at `=4.3.0` against a current 9.0.0 that declares 1.88. `substrate-redb` is pinned at
  `=2.6.4`, `smysl-text` keeps its declared 1.85, and `make msrv` confirms it: base 1.85 is
  still the floor of eleven crates and the workspace floor is still 1.88, set by the facade.
  `redb` is registered in the purity gate's second list — permitted through this feature and
  nowhere by default — so a later edit that made it a default feature has to be argued there.

  **Wired, 2026-10-11.** With the feature compiled in, a write puts the `postings` table in
  `index/substrate.redb` and the sidecar is written **without** it, its bit cleared — the format
  does not change, because a table an index does not hold has always been written as a table of
  no rows. So the two files cannot hold the same rows under one hash, and which of them to ask is
  a bit rather than a guess. Four consequences, each of which had to be decided rather than
  inherited:

  - **A byte comparison is unavailable here, and `Db::agrees` replaces it.** A redb file's page
    allocation depends on the history of writes, so two libraries with identical contents have
    different files. The comparison is over the logical table and returns the first difference as
    a sentence. §3.8's own gate survives one level up: a library whose postings live in the store,
    appended to and written again, produces an index whose `to_bytes` is byte-identical to a full
    rebuild's.
  - **An open never creates the file.** Opening a library is a write in exactly one case (rule Z,
    §4.2) and a cache is not it, so a library is handed its store only when it already has one;
    the first command that writes an index creates it. That keeps a read-only command on a
    read-only corpus working, which is the same rule the sidecar has.
  - **A writing command hydrates before it appends.** `on_append` extends a table it can see and
    the gate above is over a table, not a file, so a library whose postings are in the store reads
    them back and writes the whole table again — the `O(corpus)` a sidecar paid on *every* write,
    now paid only by the commands that write. Reading commands never pay it, which is the saving.
    The incremental write is what replaces this, and the shape that permits one is already the
    key: `(partition, term)`.
  - **A build without the feature says so by name.** It finds the bit clear and the file present,
    and reports that the postings are in `index/substrate.redb`, which it cannot read — not that
    the library has none. The remedy differs (a build, not a `--rebuild`), so the sentence has to.

  **Measured 2026-10-11**, 155,510 units in five texts, `--release`, both paths on one machine:

  | | resident sidecar | backing store |
  |---|---|---|
  | query p95 | 34.9 ms | 120 ms → **28.6 ms** |
  | view opened in | 0.001 ms | 18.4 ms |
  | command, end to end | 739 ms | **700 ms** |
  | index written in | 47 ms | 673 ms, plus the store |
  | sidecar | 176 MiB | **24.8 MiB** |
  | store file | — | 514 MiB long, 312 MiB allocated |

  **One table is resident and it is not the big one**, which the measurement decided rather than
  the design. With both tables on disk a query cost 120 ms against the sidecar's 34.9: a scorer
  asks `doc_len` once per *posting*, so a common term paid one b-tree descent per document, and
  the one query whose term has a single posting came back in 0.88 ms. So `doclen` is read into
  memory when a view opens and `postings` is read a term at a time. A length is 36 bytes a unit
  against a list's 1.1 KB — 5.6 MB at 155k and 36 MB at 10⁶, against the 7 GB this file exists
  for — so the table asked per posting is in memory and the table that is 97% of the bytes is not.

  What that buys is measured and modest: the same query, slightly faster, with 143 MB no longer
  resident. Two costs are owed and both are off the query path — the file is 1.8× the sidecar's
  bytes for the same rows, and writing it is an order of magnitude dearer. Neither is attributed
  yet: redb's `stats()` would say how much is fragmentation and how much is a 155k-row `doclen`
  b-tree, and one blob per partition instead of a row per document is the next experiment for
  both. The *written* cost of a posting is still the dense document id's to fix, and that is a
  third decision rather than either of these.
- A rebuild is **lenient** and completeness is tracked per manifest. An object can be missing
  (rule Z) or corrupt (`SMY-E446`), and a derived cache may not fail an open; a table that is
  partial must still never be half-trusted, which the per-manifest rule is what buys.

**Amended again in TX-P4 step 2** (§6), on three points found by building `postings`:

- *Which* tables an index holds is a **bitmask**, not a boolean, and the sidecar is at version 2.
  There are three cost classes rather than two: arithmetic over a catalog the open has already
  parsed (five tables), analysing every unit's text (`postings` — 765 ms at 155k units and no
  object read at all), and one object read plus a structure tree per part (`segments`,
  `locators`). An open affords the first class only.
- `postings` is partitioned by **manifest × language, derived**: a unit says neither, so the
  partition comes through `by_part` and `manifests` with the head manifest deciding. Scoring sums
  over every partition, which is exact because `df`, `docs` and total length are additive over
  disjoint document sets.
- All twelve tables are built as of TX-P4 step 4, so the `smysl_check::Pass` pattern this section
  borrowed — name them all, fill what this build fills, report the rest as not built — has no
  unfilled table left to report. What a `None` from `Index::rows` means now is that this
  *library* has not had the table built, which `--rebuild` fixes.
- `align` is derived from the library and from nothing else: the two manifests' `versification`
  declarations (key 14), the maps installed at `versification/*.tsv`, and the `locators` table.
  A map given on a command line would make the table unreproducible, so `text align` installs it.
- A **dense document id** is owed beside the store, and it is a second decision rather than the
  other half of this one — which is the correction the wiring above forced. A posting is a 32-byte content-addressed
  uid and a `u32`, so the table is about 1.1 KB per unit — 169 MB at 155k and 7 GB at 10⁶ — and a
  uid is a hash, so a postings list of them cannot be delta-encoded. The remedy is a `u32` per
  unit internal to the table, which is a format decision about the backing and is recorded with
  it.

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

**Landed 2026-10-12** (§6 TX-P5 step 4), with five fixtures and five tests. What the suite can
establish and a live run cannot: **when a model complies, the guard holds** — `measured` refused
by rule T, a store uid outside the digest refused by `SMY-E442`, a date, a speaker, a language
and a source dropped with `SMY-W444`, and a passage's own fence marker neutralised in the prompt
while the stored bytes keep it, because a `tid` is the hash of those bytes and a span is an
offset into them. Whether a given model complies is a live experiment whose answer changes with
the endpoint; whether compliance reaches a record is a property of this code, and that is the
half a corpus depends on.

#### 3.9.3 Concurrency

- **One writer per shard.** `log/<shard>.lock` (the shard's file stem since TX-P13c, so an alias's `/` is not a directory) is created with `OpenOptions::create_new(true)`
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
      telegram.rs  slack.rs  whatsapp.rs                                 (TX-P2 step 2)
      mbox.rs  epub.rs  html.rs  pdf.rs                                  (TX-P2, opt-in)
      build.rs      where a row begins, shared by every nesting reader    (TX-P1 step 3)
      chat.rs       day/message rows, the UTC calendar, the locator head  (TX-P2 step 2)
    structure.rs    arena tree, structure hash, resolve(locator)
    locator.rs      draft 3 Appendix B grammar; canonical Display
    segment.rs      seg-uax29+abbr/1
    speaker.rs      spk: + keyed BLAKE3 of a platform user id            (TX-P2 step 2)
    secrets.rs      secrets/pseudonym.key: load, create 0600, never log  (TX-P2 step 2)
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
    proposition/                                                            (TX-P7 step 2)
      mod.rs        Engine, Proposal, Place, adjacency; re-exports smysl_graph's classes()
      identical.rs  identical-span engine
      lexical.rs    lexical engine (BM25 via smysl-retrieve)
      anchored.rs   anchored engine (align table)
    link.rs         the linker's candidate generator: four sources, one budget (TX-P7 step 3)
    index/          feature substrate-redb
      mod.rs        Index handle, table definitions (§14.2 subset), rebuild, on_append
      postings.rs   persistent postings; implements smysl_retrieve::Postings
      journal.rs    ingest journal (operational)
    lock.rs         library and shard lock files (E445)
    limits.rs       Budget, Caps, defaults, fuel accounting (E440)
```

Dependencies as of TX-P2 step 1: `smysl-core`, `smysl-graph`, `unicode-normalization`,
`unicode-segmentation` (`=1.13.3`) and `caseless` (`=0.2.2`) in the default tree; behind the
reader features `quick-xml` (`=0.41.0`), `pulldown-cmark` (`=0.13.4`), `serde` and `serde_json`;
behind `stem` `rust-stemmers` (`=1.2.0`) and behind `detect` `lingua` (`=1.8.0`). The two gates
are findings of step 1, not plan: see §3.7. Every version is pinned exactly, because a segment
offset reaches a reading and a reading's rdid is an identity.
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
| `smysl-ingest` | `src/text/mod.rs`, `src/text/window.rs`, `src/text/attach.rs`, `src/text/digest.rs`, `src/text/journal.rs`, `src/link.rs` (built, step 3), `src/summarise.rs` (built, step 3), `src/same_as.rs` (staging of proposals) |

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
    // As built (TX-P2 step 3): `append(&mut self, input, who, key: Option<&speaker::Key>, caps)`.
    // Input first, as `add` has it. `who` is an alias *or* the mid of one head of a forked
    // alias. The key is an argument for the reason `AddSpec`'s is (§3.2, correction 5), and it
    // is the only thing an append is given: everything else comes from the head manifest.
    // The record-level `append(&[Record])` is now `append_records`, since *append* in this
    // crate's vocabulary is what the command means.
    pub fn part(&self, tid: &Tid) -> Result<PartText, LibError>;                  // verified (E446)
    pub fn reading(&self, rdid: &Rdid) -> Result<Reading, LibError>;
    pub fn structure(&self, mid: &Mid, tid: &Tid) -> Result<Structure, LibError>; // checks structure hash (E401)
    pub fn resolve(&self, mid: &Mid, loc: &Locator) -> Option<(Tid, Range<u64>)>;
    pub fn passage(&self, alias_or_mid: &str, range: &LocatorRange) -> Result<Passage, LibError>;
    pub fn redact(&mut self, tid: &Tid, agent: &AgentId, ts: Hlc, reason: Option<Uid>) -> Result<(), LibError>;
}
pub struct AddSpec { pub reader: ReaderId, pub params: Params, pub alias: String, pub lang: Option<LangTag>,
    pub licence: String, pub carry: Carry, pub pseudonymise: bool, pub policy: part::Policy }
    // As built (TX-P2 step 2): `pseudonym_key: Option<speaker::Key>`, not `pseudonymise: bool`.
    // A `bool` makes this crate find the key, which makes it create one, which puts a
    // randomness source in the pure tier's default tree. See §3.2, correction 5.

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
    // As built (TX-P7 step 2), with the reasons in §6: `scope` is an `Option` (as `classes`
    // already took it), `propose` returns `Vec<Proposal>` because a pair cannot say why,
    // `Engine::agent()` is provided so `tool:smysl-same-as-<id>` has one spelling, `Lexical`
    // carries `adjacent_only`, and `Anchored<'t> { table: &'t align::Table }` takes the
    // alignment rather than rebuilding it from a library and a scheme name.
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
pub enum LangPolicy { Source, Pivot }   // landed: the tag sits beside it, see below
impl Ingestor { pub fn ingest_text(&self, t: &TextIngest<'_>, attest: &dyn Attesting) -> Result<TextReport, ProviderError>; }
pub mod text { pub fn windows(s: &Structure, level: Level, budget: u32, est: &dyn Estimate) -> Vec<TextWindow>; }
pub fn link(&self, scope: &BTreeSet<Uid>, budget: LinkBudget) -> Result<Staged, ProviderError>;     // on Ingestor
pub fn summarise(&self, levels: &[Level], scope: &BTreeSet<Uid>) -> Result<Staged, ProviderError>;
// As built (TX-P7 step 3), and three departures, each because the sketch cannot say something:
//   the candidate generator is `smysl_text::link` and is pure, so GE-T11 is measured without
//     buying a model turn — a measurement that cost money is a measurement nobody runs;
//   both return a *report* rather than a `Staged`, because the counts are the thing a caller
//     reads (how many were asked, how many came back `unrelated`, how many rule M lowered) and
//     a batch cannot answer any of them — it holds only what survived;
//   `summarise` takes the library and the expression as well, because `chapter` is a fact about
//     a reading and there is no way to turn it into a set of units from a scope alone.
pub fn link(&self, store: &Store, scope: Option<&BTreeSet<Uid>>, table: Option<&align::Table>,
            budget: link::Budget, pairs_per_turn: usize) -> Result<LinkReport, ProviderError>;
pub fn summarise(&self, lib: &Library, alias: &str, levels: &[Level],
                 scope: Option<&BTreeSet<Uid>>) -> Result<SummaryReport, ProviderError>;
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

  **Not built, and the paragraph above says why in its own last clause** (TX-P2 step 4). "The
  records would be refused by `E452` in any case" is true *unconditionally*: `append` refuses
  every 15 and 18 whether or not anything is redacted, so a filter placed before that refusal
  would never change an outcome — it would be a drop nothing can reach, and `AppendReport.redacted`
  a counter that is always zero. What `Store` gained instead is the redaction **set**
  (`is_redacted`, `redactions`, `redacted_count`) and its absorb, which is the half that has to
  be here; the drop-and-count is in `Library::add`, where the bytes are (`Added.objects_redacted`).
  The algebra in the last sentence is unchanged and is now tested — `P-Z1`–`P-Z4`, §5.3.
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
  `E404`, `E446`, **`E452`** and a redaction violation; does not subsume `Produce`. **"A redaction
  violation" has no code, decided in TX-P2 step 4:** opening a library unlinks any object a
  redaction names, so a `check` can never meet one — the violation is unrepresentable rather than
  reportable, and a code nothing can raise is worse than a missing one. `ALL` gains it
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
| `src/prompt.rs` | templates `ingest.text.json` / `ingest.text.surface`: node-per-line rendering inside `FENCE`, the digest inside `DIGEST_FENCE`, the `ingest:node` field; `neutralise_fences(input) -> (String, bool)` (`W443`). The language instruction is **not** a parameter of these: `pivot:L` is a second call with a template of its own (`ingest.text.pivot`, `pivot_gists(tag)`), because a prompt that asked for extraction and translation at once was asking for two things and the extraction templates' bytes decide a recipe. |
| `src/recipe.rs` | `Conditions` gains `lang_policy`, `normaliser` (V1/V2, from SMYSL-2.1), `estimator`, `window_level`, `digest_policy` (budget and ranking id), `segmenter` and `reader` ids. Pushed in `push_shared`, so `family()` still drops only provider and model. |
| `src/text/cost.rs` (new, step 5) | the cost report (§3.7): `cost(&Journal, Option<&alias>) -> CostReport`, one `Row` per language plus a total, each with windows, calls, repair turns, tokens, verdicts and the entries it could not price. `Row::retry_rate` is §3.7's `r` — repairs per *first attempt* — and `tokens_per_window`/`project` are the measured route, which the two must not be combined. |
| `src/text/journal.rs` (new) | key `(alias, tid, node range, recipe)` → `{ staged batch hash, digest hash, tokens in/out, quote verdicts }`; step 5 adds the window's `lang`, its `calls`, its repair turns and the pivot pass's calls (`pivots`, out of the retry-rate denominator and in the token counts), and makes the token counts **the window's own** rather than the run's running total; stored in the redb `journal` table (or `<library>/journal.cbor` without `substrate-redb`). Rerun skips completed keys; within one expression a part extracted under an earlier manifest is skipped; `--reextract` ignores the journal for the head manifest. Resume after kill loses at most the batch in flight, because a key is written only after its batch is staged. |
| `src/lib.rs` | `Ingestor::ingest_text`, `link`, `summarise`; `chunk` stays `pub(crate)`. |
| `src/path.rs` (step 5) | `choose_tokens`, the path rule asked in **tokens** rather than bytes, with `SMALL_OUTPUT_TOKENS = SMALL_OUTPUT_THRESHOLD / 4`. The byte form sent a Russian expression down the surface path where the same passage in English took json-ast; `ingest_text` uses the token form and `TextReport::path` reports which was taken. |
| `src/same_as.rs` (new) | turns engine proposals into staged `x.text/same-as` relations with tool attestations; `lexical` proposals above the recipe's threshold, or judged (OQ-44). |
| `src/text/pivot.rs` (new, the pivot policy) | `translatable(&batch)` (claims, never entities), `render(&sources)` (numbered gists, one line each), `parse_answer(&answer, sent)` and `units(&sources, &answer, lang) -> Pivoted`. The units are written **here** from the ones they translate — the model supplies a sentence and nothing else — so the span, the status, the source and the payload are copied and a pivot pass cannot invent a place. `SMY-W453` for a line that did not come back, `SMY-W436` for one in the wrong script. |

Pivot policy: a second pass over `source`-policy units of the scope, one unit per source unit,
linked by `x.text/translates`, with the source unit in `deps` so rule M caps it.
**Landed 2026-10-13**, as written in intent but not in type, with five things the one-sentence
plan did not say and the implementation had to decide.

**It is not an extraction in another language.** Extraction runs under `source` whatever the
policy says — the quote is the passage's own words, because a translated quote cannot be
`Present` against the text it came from — and the pass translates the `gist` of each unit
afterwards. A pivot unit is therefore a claim about the same bytes as its source, carrying the
same span, and `TextReport::path`, `verdicts` and the agreement figures are unmoved by the
policy.

**`body` and `detail` do not come along.** FC-1's `lang` is one field for every text field of a
unit, so a unit with an English gist and a Russian body would be lying about one of them. The
pivot unit is an L0 statement whose L1 is one `deps` hop away.

**Entities are left alone.** A-2.4 rule 7 makes an entity's identity its name, its language and
its expression, so translating `Иерусалим` does not add a language to an entity — it mints a
second one. Saying those two are one thing is `x.text/same-as`, TX-P7's engine and its judgement
to make, not a side effect of a translation pass.

**The pass has no repair loop, and the asymmetry is deliberate.** A repair turn exists to fix an
answer that failed a check this crate can run, and the only check available to a translation is
whether a line came back. Asking again for a line a model chose not to write doubles the cost of
every pivoted window to buy a line that would be unverifiable if it arrived. A missing line is
`SMY-W453` and the unit is staged in its own language; `--reextract` is the operator's to choose.
Its call is also **out of the retry-rate denominator** (§3.7): it is a first attempt of a
different request, and counting it would make the same corpus report a lower `r` for having been
pivoted.

**The language is not in the variant, and that is semver's doing.** This plan wrote
`Pivot(LangTag)`, and it was built that way first. `LangPolicy` has been an all-unit enum since
1.9, so `LangPolicy::Source as isize` compiles against it; a variant carrying data takes those
discriminants away, which `cargo-semver-checks` reports as
`enum_discriminants_undefined_non_unit_variant` — a **major** change, and this project's
`SEMVER_BREAKING` is empty, so a major change is a 2.0. Naming a language is not worth a major
version. `Pivot` is therefore a unit variant and the tag is its parameter, held beside it in
`IngestOptions::pivot_lang` and `Conditions::pivot_lang`, with `IngestOptions::pivot()` the one
place the pair is read so that "pivot with no language" cannot reach the pass. The recipe pushes
the two as two components, `"lang-policy" ‖ "pivot"` then `"pivot-lang" ‖ "ru"`, which keeps
`pivot:en` and `pivot:ru` distinct recipes and leaves every `source` recipe byte-identical to
1.9's. `LangPolicy::parse` keeps the signature it published; `parse_with_lang` returns the pair.
The gate caught this after the code was written and tested, which is the argument for running it
before a commit rather than before a release.

One defect found by building it. An `x.text/translates` edge is the first record a text ingest
stages whose *kind* the check looks up, and `x.text/v1` was declared nowhere in a library — so
every check of such a store would report `SMY-W013`, "a warning that is correct and
unactionable", which is the state `smysl-check`'s own test describes. `ingest_text` now stages
the declaration with the first batch that uses the schema, once per library rather than once per
window. Entities had been in that state since step 3b; the edge is what made it visible.

#### 4.3.5 `smysl-retrieve`

- `src/analyze.rs`: the `Analyze` seam; `Tokenizer` implements it with ids `smysl/plain/1` and
  `smysl/fold-en/1`. `Bm25::index_with` keeps its signature and delegates.
- `src/postings.rs`: the `Postings` seam and an in-memory implementation built by today's path;
  `Bm25` can score over any `Postings`, which is the hook the redb table in `smysl-text::index`
  implements (persistent, per partition = manifest × language, incremental on append).
- Per-unit analyzer routing: a closure `Fn(&Uid) -> &dyn Analyze` chosen by unit `lang`, falling
  back to the view's; `find --lang L --analyzer A` overrides.

**Amended in TX-P4 step 2** (§6), on four points found by building it:

1. **`Bm25::over(postings, a)` cannot answer a query**, so it takes the store too. `Query` filters
   on kernel type, status, schema and payload, and none of the four is a fact about a term — a
   view without them would ignore `--kind`, `--min-status`, `--schema` and `--payload`, which is
   a confident answer to a different question. The filters are read from the units that *scored*,
   not from every unit in the corpus, so the cost is per candidate and the ordinary unfiltered
   query pays none of it.
2. **`Postings` has six methods, not four.** `total_len` because `avgdl` is corpus-wide and a
   scorer without it would walk every document's length per query, which is the scan the seam
   exists to remove; `tf(term, uid)` because decomposing a score (`--why`, R22) asks for one
   document's term frequency and the four-method form could only answer it by materialising a
   postings list that is as long as the corpus for a term the corpus all holds.
3. **Routing is per partition, not per unit**, because a unit has no `lang` until FC-1 (TX-P5
   step 1). What a unit has is a `source.reference` naming a part; the part is recorded by
   manifests; a manifest has one language. So the partition is derived through `by_part` and
   `manifests`, the head manifest deciding, and the language is in the partition key ready for
   the day a unit carries its own.
4. **Scoring sums over every partition.** `df`, `docs` and `total_len` are additive over disjoint
   document sets, so the sum is the whole-corpus statistic exactly — which is what makes "results
   identical to the in-memory path" a reachable claim. The partition is a unit of storage and
   maintenance, not of scoring.

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
| `text append <file>` | `--alias A [--pseudonymise]`; reader, parameters, policy, lang, licence and carry taken from the head manifest, and refused on the command line | pure | P2 (built, step 3) |
| `text ls` | `[--alias A] [--forks]` | pure | P1 |
| `text show <alias\|mid>#<locator>[-<locator>]` | `[--raw] [--segments]` | pure | P1 |
| `text align <a> <b>` | `--scheme S` | pure | P4 |
| `text redact <tid>` | `--as AGENT [--at MS] [--reason UID]` | pure | P2 (built, step 4) |
| `date set <target>` | `--axis said\|composed\|about --value EDTF\|offset:MS\|<allen>:<target> [--basis UID]` | pure | P3 |
| `date order <a> before <b>` | `[--basis UID]` (sugar for a relative `date set`) | pure | P3 |
| `date show <target>` | `[--axis A] [--why]` | pure | P3 |
| `ingest --text <alias\|mid>[#range]` | `--lang-policy source\|pivot:L [--reextract] [--digest-budget N] [--window-level L] [--single-pass] [--cost]` | model | P5 |
| `link` | `--scope <alias\|mid…> [--candidates] [--per-unit N] [-n K] [--hub N] [--pairs N] [--at MS] [--yes]` (an `sq` scope once SMYSL-2.5 lands) | mixed (built, step 3): pure under `--candidates` | P7 |
| `summarise` | `--scope <alias\|mid> [--levels chapter,book] [--at MS] [--yes]` | model (built, step 3) | P7 |
| `same-as propose` | `--engine identical-span\|lexical\|anchored [--scope …] [-n K] [--no-adjacent] [--at MS] [--yes] [--dry-run]` (`semantic` reserved for SMYSL-2.5) | pure (built, step 2) | P7 |
| `same-as classes` | `--policy strict\|component\|attested:N [--scope …]` | pure (built, step 2) | P7 |
| `find` | `--lang L --analyzer A` added | pure | P4 |
| `check` | `--library` resolves parts for `W405` | pure | P1 |

- **`text add --pseudonymise`** (P2, built). Creates `secrets/pseudonym.key` on first use and
  reads it afterwards, prints the *file's* name and never the key, and reports
  `speakers  pseudonymised under secrets/pseudonym.key`. **`text show --segments` now prints a
  row's speaker and timestamp** where the reader filled them, which is new in TX-P2 step 2 and
  needed a library method beside `Library::structure`: a node is a range, and a row is a range
  plus who said it. For the six TX-P1 readers the two print alike, because none of them fills
  those fields.

- **Library path.** No new global flag: `-s/--store` accepts a library directory, recognised by
  a `<dir>/LIBRARY` marker file holding the layout version. A plain file path keeps today's
  meaning.
- **`same-as`** (P7, built). One row rather than §4.4's draft `pure per engine; mixed as a
  command`: every engine in this build is a computation over records, and the one that would
  make the row mixed is `semantic`, which is SMYSL-2.5's and is the value clap refuses. A
  `--scope` is written in aliases and mids — that is how a person holds a corpus — and applied
  as a set of units, through the manifest a unit's span is an offset into, which is the only
  link to an expression A-2.4 lets a unit carry; an alias therefore needs a library to resolve
  and an `m3:` does not, and a plain store says so. `--dry-run` is the determinism matrix's,
  and `--yes` over a surface store is refused because an attestation has no surface form.

- **`link` and `summarise`** (P7, built in step 3). `link` is **mixed** rather than the draft's
  `model`, because `--candidates` lists the pairs the generator chose and calls nothing: the
  budget is the thing a reader has to be able to reproduce — GE-T11's figure is a recall *at a
  budget* — and a flag that shows the questions without buying the answers is what makes that
  checkable and what puts the operation in the determinism matrix. `--budget N` became three
  numbers, `--per-unit`, `--neighbours` and `--hub`, because they buy different things: one
  spends model turns and the other two spend arithmetic, and a single figure would have tied
  them together. `summarise` stays `model`: there is no pure invocation of a sentence.
  `--scope` on `summarise` names **one expression** and not a set of units, because the levels
  come from that expression's reading.

- **Gates touched.** `tests/cli-surface.txt` regenerated (`make cli-surface`, after adding the
  names to the Makefile's `COMMAND_NAMES`);
  `command_table_matches_section_23` and `tests/dispatch.rs::every_command_dispatches` gain the
  rows; `xtask determinism` registers `text add` (reader output bytes), `text show`, `date show`,
  `same-as propose --engine identical-span`, `same-as classes` and `link --candidates` — the
  `ru_RU.UTF-8` locale in its matrix is exactly the one case folding would betray.
- **Exit codes** reuse today's (`Staged`, `StagedWithCorrections` for staging commands).

### 4.5 Features, dependencies, purity gate, C toolchain

`crates/smysl-text/Cargo.toml`:

```toml
[features]
default = []                                         # the core the purity gate checks
reader-txt = []   reader-md = ["dep:pulldown-cmark"]   reader-usfm = []
reader-osis = ["dep:quick-xml"]   reader-zefania = ["dep:quick-xml"]
reader-json = ["dep:serde_json"]  reader-telegram = ["dep:serde", "dep:serde_json"]
reader-slack = ["dep:serde", "dep:serde_json", "dep:zip", "dep:flate2"]
reader-whatsapp = []                                 # as built, TX-P2 step 2
reader-mbox = ["dep:mail-parser"]  reader-epub = ["dep:rbook", "zip"]
reader-html = ["dep:html2text"]    reader-pdf = ["dep:pdf-extract"]
substrate-redb = ["dep:redb"]
```

Root `Cargo.toml`: `text = ["dep:smysl-text", "smysl-check/text", "smysl-retrieve/…"]`;
`cli` enables `text` and `reader-{txt,md,usfm,osis,zefania,json,telegram,slack,whatsapp}` and
`substrate-redb` (draft 3 §18); `epub`, `html`, `mbox`, `pdf` opt-in.

**As built, with two corrections.** `text` also carries `dep:getrandom`, which is the pseudonym
key's thirty-two bytes and costs no crate at all (`ureq`'s TLS stack already links it); the
reason it is in the facade rather than in `smysl-text` is correction 5 of §3.2. And
`reader-slack` needs `flate2` **named explicitly**: `zip`'s own `deflate-flate2` feature is
`dep:flate2` and nothing else, and `flate2` with no backend feature does not compile — fifteen
errors about a missing `MZ_DEFAULT_WINDOW_BITS`. So the inflater is a choice this project makes,
and it is `rust_backend` (`miniz_oxide`), which keeps §13's "zero C in the core path" true and
is what `make crate-features` checks. The gate's `NOT_IN_THE_CORE` list gained rows for both, so
a later edit that made `reader-slack` a default feature has to be argued rather than merged.

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
- **Crate size.** ~~`lingua` language models are large (unverified figure); five features only, and
  the per-language model crates are downloaded only with `text`.~~ **Measured in TX-P2 step 1:**
  14.4 MB across the five model crates (English 2.59 MB, French 2.56 MB, German 3.46 MB, Russian
  3.26 MB, Spanish 2.51 MB), compiled in via `include_dir`, and 57 crates in the tree. Not with
  `text` — with `smysl-text/detect`, which neither `text` nor `cli` turns on (§3.7).

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
| cost | the retry rate is repairs over *first attempts* and not over calls; rows are per language and never averaged; an alias nobody extracted has no figures rather than zeroed ones; a manifest with no language is a row of its own; an entry written before the per-window fields is counted and left out of every sum; a projection carries the repairs it measured |

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
  most one batch, `--reextract`, pivot policy produces `x.text/translates` with deps — **met**:
  one Russian unit per English one, each carrying its source in `deps` and an edge back to it,
  the span the source's, and `pivot:en`, `pivot:ru` and `source` three recipes. With two
  controls: a pass that returns nothing reports `SMY-W453` per unit and stages no edge, and a
  *document* ingest refuses the policy before egress rather than extracting in the passage's
  language.
- **Red team** (`crates/smysl-ingest/tests/redteam.rs`): for each fixture in
  `fixtures/library/redteam/`, the rendered prompt contains exactly two input fences and two digest
  fences; a scripted "obedient" answer that cites a store uid outside the digest (`E442`), claims
  `measured` (rule T), supplies `published`/`observed`/`text:speaker`/`lang` (`W444`), or invents a
  relation kind `supersedes` (refused today) never reaches a staged record unchanged.
- **The phase exit** (`crates/smysl-ingest/tests/phase_exit.rs`, added in TX-P5 step 5): the four
  clauses of §6 TX-P5 step 5 that a scripted provider can measure — resume after a truncated
  journal re-sends exactly one window of eight and the store is unchanged; span coverage, with a
  control at 0.857 built from a quote the window joins and the part does not; entity duplication
  across nine windows of a chat reading, against the speakers the reading has; and the cost
  report, per window and per language, with a control that drives the retry rate off zero.
- **GE-T2** (`crates/smysl-ingest/tests/ge_t2.rs`, added in TX-P5 step 5): the `identical-span`
  figure in English and Russian, a drifting pass, a single pass, a spanless unit — and that both
  languages choose one ingest path. The live arm (`SMYSL_GE_T2`, `SMYSL_INGEST_LIVE`) reports the
  clause as owed when it is given no corpus.
- **CLI cost report** (`tests/cmd_ingest_cost.rs`): `--cost` needs no provider and no `--yes`, and
  an expression the journal has never seen is told so rather than priced at zero.
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

**Redaction merge** (~~`crates/smysl-graph/tests/redaction_algebra.rs`~~
`crates/smysl-text/tests/redaction_algebra.rs`, `fuzz/fuzz_targets/redaction_merge.rs`). Moved in
TX-P2 step 4: `P-Z2` and `P-Z3` are about records 15 and 18, a `Store` holds neither, and the
place they have content is an object store — which is `smysl-text`'s. The fuzz target keeps the
record-level half, where a filesystem would only slow the search down. Generator: stores with parts, readings, manifests, units,
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
| **GE-T1** (determinism) | TX-P1 (5 Bibles, JSON series), TX-P2 (2 chat exports) | build each library on Linux x86-64 and one other platform (CI matrix, unverified availability); compare mids, structure hashes, rdids; Python, JavaScript and Go recompute tid/rdid/mid from the emitted records and agree. They do not re-implement readers. Any mismatch blocks the next phase. **Partly met at TX-P1 step 7:** the three ports derive every identity in `fixtures/library/wire/ids.json` and agree, and that half is done. Two halves are owed and neither is a port's. The **five Bibles and the JSON series** are a corpus this repository does not hold, so what is checked is a two-part fixture (one Latin, one Cyrillic) — enough to exercise NFC, not enough to be the criterion. The **second platform now exists** (1.10.0): the three port jobs run on `ubuntu-latest` and `macos-latest`, where before every job in CI was Linux. Worth naming what that tests, because it is not the identities — BLAKE3 over bytes and canonical CBOR are platform-independent by construction — it is *reading files*: line endings, NFC through a different Unicode table version, path behaviour, which is exactly the code a part's identity depends on. The python job also regenerates the uid fixtures and requires no diff, so that check is now made on both. **The corpus is paid in 1.10.0**, and the half that was never about ports is now measured rather than argued. `crates/smysl-text/tests/ge_t1_bibles.rs` takes `SMYSL_GE_T1=<dir>` of `<edition>/*.usfm` and builds a real library per edition — `Library::create`, then `Library::add` per book through `usfm/1` — over the five Bibles GE-T1 names: Luther 1912, the KJV, Segond 1910, the Russian Synodal and Reina-Valera 1909, each the 66-book canon, 1,189 chapters, 3.9 to 6.1 MB of text. What comes inside the repository is not the scripture but the **measurement**: `fixtures/library/bibles/expected.tsv`, seven kilobytes holding, per edition, the digest of the input bytes, the counts, a **roll** — BLAKE3 over the canonical listing of every alias, mid, tid, length, structure hash and rdid — and the first and last book's identities in full so a moved roll has somewhere to start. That is what makes a two-platform comparison exact instead of two people agreeing over a phone call: a run on any machine with the same `input_hex` either reproduces the roll or does not. **Two platforms have now run it**: macOS on arm64 and Linux on arm64 (glibc, a different Unicode table and a different line-ending history), identical rolls on all five editions. The architecture half of the clause's wording — Linux *x86-64* — is the one thing still outstanding, and it belongs to CI rather than to a desk. What the comparison tests is worth keeping in view: not BLAKE3 or canonical CBOR, which are platform-independent by construction, but *reading files*. **And CI compares on every push**, because 112 KiB of the corpus is committed: `fixtures/library/bibles/slice/` is Ruth and Jude in all five editions — whole books, five scripts, a four-part expression and a one-part one — measured by the same function against `slice.tsv`, with no corpus to arrange and no environment variable to remember. The slice does not replace the clause; it is what runs when nobody is looking. **The ports now recompute over corpus material too**: `fixtures/library/bibles/wire/` is the Russian Ruth as `usfm/1` read it, four parts and 89 segment rows, in the shape `wire/ids.json` already has, and Python, JavaScript and Go each derive its mid, tids, rdids and structure hashes and agree. One edition, because what a port derives is language-independent while what a port can get *wrong* is reading a **byte** offset out of a segment table over multibyte text — Cyrillic is where an implementation that indexed by character or by code unit produces every identity correctly and every span half as wide, so each port now decodes every row's span as UTF-8 and would fail if it had. What is still owed on this row is the **JSON series**, and it is owed alone. TX-P2 step 1 adds a **second** corpus to the same debt: the 500-sentence sentence-boundary gold set per tier-1 language that step's exit asks for. Its harness exists and takes a directory (`SMYSL_SEG_GOLD`); what is missing is prose nobody here wrote, annotated by someone who did not write the segmenter. Both are the same shape of gap — a measurement whose instrument is built and whose material is not — and neither is met by anything this repository can author. **TX-P2 step 2 closes the chat half of the first gap, in the one way it can be closed honestly:** `fixtures/library/readers/{telegram.json,whatsapp.txt,slack.zip}` are two chat exports in three formats, written for the purpose, with every identity pinned in a `.expected` file beside them. They are small and they are ours, so they are an instrument and not a measurement of real traffic — what they do establish is the half GE-T1 asks of a *reader*: that the same bytes produce the same tids, structure hashes, rdids and mid on re-read, and that the readings do not depend on the order a zip happens to be written in. |
| **GE-T4** (attribution fairness) | before TX-P5 | 200 hand-checked units per tier-1 language under normaliser v1 and v2 (SMYSL-2.1); v2 false-`Absent` > 1% in any language blocks TX-P5. **The harness is built and the corpus is scored** (`tests/ge_t4.rs`, 1.10.0); what is owed is the hand-checking, and it is now a short list rather than a thousand units. A `Present` verdict cannot be a false `Absent`, so only the units the check does *not* settle can be mislabelled — the rest of the sample is the denominator. Over the S0 spike's run tree (730 staged documents, the only corpus of extracted units with their sources that exists), the sample is **200 in each of five languages** and the adjudication worklist is **five units**, all English. Three narrowings got it there, and each is asserted rather than asserted-to. (1) Only an **`Absent`** can be a false `Absent` — `SMY-E307` refuses on `Absent` and not on `Loose` — so the first version's 45 rows were 16 too many: a ruling on a `Loose` unit cannot move the figure by one. That leaves 29, en 21 and ru 8. (2) A language the check **never refused** is at zero and owes nobody a ruling, which closes de, es and fr outright. (3) Two shapes are **decidable with evidence**: a quote with an elision in it, and a quote the check calls `Present` against a text the unit did not cite — the latter needs the full editions (`SMYSL_GE_T4_CORPUS`), because a corpus cut into chapters cannot witness a quote from the chapter *before* the one it holds. That decided **24 of the 29**: 23 quotes found in an edition and one elision. Every refusal in the sample turned out to have one shape — **a model quoting past the window it was shown**, from the book rather than the passage: Gen.3 in a Gen.4 window, Jas.1 in Jas.2, Ex.19 in Ex.20, 1Ki.2 in 1Ki.3, Prov in Ecc. A rule's label carries its evidence and **a person's ruling overrides it**. The five that survive all turn on one editorial question, which is why they are a reader's and not a rule's: the eBible edition brackets supplied words (`[having] sorrow`, `[thou hast] with thee`, `to all [men] liberally`) and the model's quote drops the brackets, so the quote is not present as written in *any* text and no rule may guess. En's rate is reported **provisional** while they stand, because it can only rise — five verbatim rulings would make it 0.0250 and block the phase. Verdict split per 200 under v2 — de 199/1/0, en 173/6/21, es 195/5/0, fr 198/2/0, ru 191/1/8 (present/loose/absent). Measured: de, es and fr **0.0000** with nothing to rule on, ru **0.0000** over 8 refusals all decided by rule, en **0.0000\*** over 16 of 21 with five outstanding. The harness takes the corpus as `SMYSL_GE_T4=<dir>`, writes the worklist, and takes the rulings back as `SMYSL_GE_T4_LABELS=<file>` (`<uid><TAB>verbatim|not-verbatim`); with no rulings it reports the rate as **owed** rather than 0.0, the choice `Row::retry_rate` makes and for the same reason. **The bar is two units.** 1% of 200 is 2, so three verbatim quotes among English's 21 `Absent`s block the phase — which is why the ruling is a measurement and not a formality. Two things the scoring already found: the v1/v2 disagreement set is **not** empty, against S0's empty set over 5,560 quotes — one Russian quote that carries its own verse locator is `Loose` under v1 and `Present` under v2, with a three-byte span difference — and the `Absent`s are dominated by two shapes a reader will rule *not* verbatim, an elision (`...` inside the quote) and a quote from a passage the unit did not cite. The units are the spike's, not this build's, and that is sound for this row and not for GE-T2's: GE-T4 asks about the quote **check**, which is the same code either way, while GE-T2 asks about this build's extraction. **Ruled and passed 2026-10-14.** All 29 refusals came back ruled by hand — `29 ruled by hand, 0 decided by rule, 0 left to read` — so the figure no longer rests on any rule of the instrument's, including the 24 a rule had decided. The five that were a reader's did **not** turn on the brackets after all, and that finding is worth more than the figure: every one of them is real KJV text from a **neighbouring chapter**, and each `source` is exactly one chapter — `[having] sorrow in my heart daily` is PSA 13:2 under a Ps.14 part, Shimei is 1KI 2:8 under 1Ki.3, the thunders are EXO 19:16 under Ex.20 and carry their own verse reference as a prefix, `all [men] liberally` is JAM 1:5 under Jas.2, and `The heart shall rejoice in thy salvation` is PSA 13:5 and reads `my heart` there. So the ruling was the definition the worklist's own header states — in the text **this unit cites** — and the brackets were a second difference on three of five rows rather than the question. Final: **0.0000 in all five languages** under both normalisers, against a bar of two units in 200. |
| **GE-T13** (effective time) | TX-P3 | synthetic chats with planted skews plus a public chronology with known relative orders; P-E4/P-E5 on real data; any planted skew undetected, or any false contested interval on consistent data, blocks TX-P4. **Ran in TX-P3 step 5** (`crates/smysl-text/tests/ge_t13.rs`), and three of its four clauses are met. *Synthetic chats with planted skews:* 64 generated chats, each a chain of messages quoting the one before it with one instant moved — every skew that contradicts an ordering is detected and every skew that leaves every ordering true is **not** reported, which is the half that matters equally: a detector that flagged everything would pass the first clause and fail the gate. *No false contested interval on consistent data:* the three chat fixtures, unmodified, report nothing. *P-E4/P-E5 on real data:* the same fixtures, with the real timestamps and the real reply graph — `telegram/1`'s `reply` and `slack/1`'s `thread` give five orderings between them, `whatsapp.txt` gives none because the export format records no reply — every interval contains the instant the export recorded, every planted skew is found, and one window dating with a `cited` basis puts the message back. The **units are synthesised**, and that is stated in the harness: a chat reaches a unit only through ingest, which is TX-P5, so there is no store of units from a chat for a gate that blocks TX-P4 to read. One row per message at its recorded instant and an `x.text/quotes` edge per reply is what ingest will build minus the model, which makes the *data* real and the units this harness's. *A public chronology with known relative orders* was **owed** and is **paid in 1.10.0**: `fixtures/chronology/public.tsv`, 45 pairs of public dates with a note naming the events on every line, so the order is checkable against what a reader already knows and the only authored part is which pairs were picked. They were picked to be awkward rather than to pass — approximate and uncertain values, unspecified digits, seasons, intervals, open ends, a negative year and a `Y`-year — and the file carries two relations, because one of them cannot fail. `before` (the published two-column form) says the earlier value's extent may not start after the later one's ends, which is all a pair of overlapping extents can claim and is also satisfied by an engine that widened everything to a century. `disjoint`, claimed by 39 of the 43 decidable pairs, says the extents may not overlap at all, and a widened extent fails it. A pair open on the side compared is **not decidable** and is counted and reported rather than passed, with most of the file required to be decidable — a run of satisfied-by-vacuity pairs reads exactly like a run of passing ones. It bit twice while being written: spring and autumn of 1918 are *not* ordered here, which is OQ-71(b) working as decided (a season is its year, EDTF fixing no hemisphere), and a `Y`-year of 13.8 billion is refused outright, this build bounding that year to 32 bits. `SMYSL_CHRONOLOGY=<file>` still replaces the material with somebody else's and needs no code change. |
| ~~**GE-T14** (part size)~~ | ~~end of TX-P2~~ | **Ran in TX-P2 step 6; the curve is in §3.1 and the default is fixed at 1 KiB / 4 MiB.** Three of the four axes are independent of the size — growth costs at most two parts, a same-length correction one, a re-length every part after it — so the only trade is object count and catalog size against redaction granularity, and 64 KiB made one redaction remove sixteen days of a conversation. The measurement is synthetic and says so: every axis is a function of the structure alone, so the material still owed is only whether the node-size distributions match real corpora. `crates/smysl-text/tests/part_size.rs` asserts the curve rather than printing it. |
| **GE-T2** (extraction stability) | rerun at TX-P5 | under the consensus policy (§3.4), not the single-pass policy draft 3 assumed. Stability is measured as `identical-span` agreement between the two passes — computed, not judged — and reported per language. The judged `J_class` form of GE-T2 waits for GE-T9 with GE-T5, for the reason in the row below. S0's pilot: `J_class` 1.000 local, 0.327 hosted, against a 0.6 threshold. **Ran in TX-P5 step 5** (`crates/smysl-ingest/tests/ge_t2.rs`): the **instrument** is met and the **corpus is owed**. `TextReport::agreement` records the figure per run and `TextReport::lang` carries the language it is reported under, so the grouping is data rather than a caller's memory. Measured scripted: 1.000 in English and in Russian where both passes found the same places, 0.500 where the second pass drifted, `None` for a single pass — and that last one was a defect, since one pass agrees with itself by construction and `Agreement::rate` used to divide anyway. The two-language arm earned its keep: it found that the ingest **path** was chosen in bytes, so the Russian corpus was being extracted through the surface parser and the English one through json-ast, which would have made a per-language figure a figure about two pipelines. **Paid 2026-10-15.** The corpus is `fixtures/stability/corpus/` — `<lang>.txt` for the five tier-1 languages, the S0 spike's own passages with the choice removed mechanically (every reference all five carry: 26 verses of narrative, 26 of law, 7 of poetry, 59 lines in each file), so the only thing varying between five figures is the language. `SMYSL_INGEST_LIVE=1` is still required, because the measurement spends a credential and is never taken by accident; `SMYSL_GE_T2=<dir>` and `SMYSL_GE_T2_MODEL=<name>` replace the corpus and the model. **Measured, three runs of all five languages against hosted `deepseek-chat` at temperature 0, span coverage 1.000 throughout:** de 0.024/0.412/0.078, en 0.452/0.769/0.132, es 0.000/0.333/0.047, fr 0.047/0.024/0.391, ru 0.265/0.481/0.030. The finding is the spread, not the figures — re-running one language moves it 0.333 to 0.637 and changing the language moves it 0.361 to 0.745, the same size, and the ranking does not survive a rerun — so **the per-language figure this row asks for is not resolved by one run per language**, and it is recorded as a spread over three runs rather than as five numbers of sampling noise. Temperature was checked and is not the cause (`"temperature": 0.0` reaches the wire); the result instead reproduces S0's local/hosted split under a metric that needs no judge, and **OQ-74** records that `Passes::admits` guards the configured temperature and not the provider's determinism. Reading the figure at all required one addition: `identical-span` includes the gist, so `Agreement` gained `comparable_places`/`agreed_places`/`place_rate`, the same comparison with the gist left out — French run B is 0.024 with it and 0.680 without, the two passes having found the same 17 of 25 places and re-worded nearly all of them. The figure is **reported, not thresholded**: the computed form has no bar, because the bar it had (0.6) belongs to the judged form and that waits for GE-T9's human α. The live arm also shipped a defect that made it unmeasurable against any corpus — `max_output` 800 carried over from the scripted instrument and `context_window` left at `ProviderConfig::default()`'s 4096, so every answer was cut off (`SMY-W304`), every span degraded to opaque prose and `comparable` was 0; it now configures SMYSL-2.2 §4's geometry (32768/8192) for every provider, so the windows are identical whichever one a machine reaches. |
| GE-T5 within-language arm, GE-T11 | TX-P7 | **GE-T11 ran and passed 2026-10-14** (`crates/smysl-text/tests/ge_t11.rs`): recall **0.6800**, 34 of 50 planted far contradictions, at the default candidate budget of 16 per unit over 32 retrieved neighbours. It is measured without a model, because the clause is about the *generator* — if the true pair is not in the candidate set, no judge however good will find the contradiction — so it runs on every `make test` rather than when somebody pays for it. The figure's limits are in §6 step 3 and in the harness: a planted set this repository wrote, over a distractor pool of 210 units, with two of the four sources contributing nothing for want of extracted themes and an alignment. `SMYSL_GE_T11_CORPUS` and `SMYSL_GE_T11_PLANTED` replace either without a code change. **GE-T5 ran 2026-10-14 and its instrument is met with its sheet owed** (`crates/smysl-text/tests/ge_t5.rs`, §6 step 4): over a second English expression of Ruth written by hand, precision **0.6724 at recall 0.9750**, reported against no bar — and the finding is that recall is nearly free while precision is the whole difficulty, because from a neighbour count of two upward the engine proposes all ten planted near-misses, a sentence at the same locator with the same names saying the opposite thing. The restated clause itself is implemented end to end — α nominal over a reliability matrix, a pooled gold by majority with ties dropped, the engine scored as one more coder, and a paired bootstrap on the difference whose 95% interval has to reach zero, one-sided — and over the committed *scripted* sheet it reads α(coders) 0.6000, α(gold, engine) 0.5594, δ [−0.1278, 0.2189]. That is what `Met` looks like and it is reported as **owed**, because three coders judging independently is material and this repository holds none; `SMYSL_GE_T5_SHEET=<file>` decides it. **GE-T5 as restated:** as draft 3 §23, **except the threshold** (§1.1 item 5, argued in §0.1). Draft 3's "precision ≥ 0.9 at recall 0.7" is measured against a gold, and S0's gold is three model coders whose binary α is 0.600 [0.550, 0.646] — so 0.9 is above what that gold can support, and the pilot's 0.835 and 0.712 measure the gold rather than the engines. Restated: an engine passes when its agreement with the pooled gold, scored on the coders' own scale, is **not distinguishable from the coders' agreement with each other** — it joins the pool rather than beating it. The absolute bar returns when **GE-T9** supplies a human α, which is still owed; until then every class figure is reported against the model-model ceiling, and `attested:2` is the default (§3.6). The cross-lingual arm of GE-T5 is SMYSL-2.5 and waits for TX-P10's S1 embeddings. |

### 5.6 Cross-implementation (C-Read additions, D-8)

| implementation | additions | test |
|---|---|---|
| `python/smysl` (`records.py`, `uid.py`) | decode/re-encode records 14, 15, 17, 18, 19 and unit key 9, source keys 4–6, byte-identically; `tid`, `mid`, `did`, `rdid`; record-set digest (from SMYSL-2.1) | `tests/test_library.py` over `fixtures/library/wire` |
| `nodejs/src` (`records.js`, `uid.js`) | same | `test/library.test.js` |
| `go/` (`records.go`, `uid.go`) | same | `library_test.go` |

**Added in TX-P2 step 6: `fixtures/library/chat-wire/`**, and the three suites above gained a
second file each (`test_chat_wire.py`, `chat_wire.test.js`, `chat_wire_test.go`). The fixture
above is hand-built in `smysl-core` — correctly, in a crate with no reader — so what it cannot
offer is the records a *reader* emits. The chat fixture is the output of three real `text add`
runs, and it is where the identities are derived over what scripture has none of: segment tables
with speakers, timestamps and platform ids in them, `mul` manifests, and `raw` metadata inside
the mid. The ports still implement no reader; they decode and hash.

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

     **And the fuzz target could not have found it, which was worth discovering.** Re-run after
     the step — 14,014,150 executions, no finding — and then read back: it stopped at
     `Structure::build` over the whole text and never cut a part, so none of the arithmetic this
     step changed was reachable from it. A clean run over code a target cannot reach is the most
     expensive kind of silence. The target now groups the text under a policy sized for fuzz
     inputs (`target_min` 16, where the default 64 KiB makes every input one part — the case
     that hid the defect) and asserts three things: that the parts partition the text, that each
     part's own rows build a structure of exactly the part's length with every locator resolving
     inside it — which is the assertion the absolute-offset version would have failed outright —
     and that a part whose bytes exist hashes to the tid a manifest would record for it.
     Coverage went from 5,431 features to 5,728, and 13,928,763 executions found nothing.
     `part_local_rows` moved from `library.rs` to `part::local_rows` to be reachable, which is
     its right home anyway: `part.rs` is the module that owns how a text becomes parts.
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
     *Shards arrived with SMYSL-2.8's TX-P13c step 1 (2026-10-25), as layout
     `smysl/library/2`:* `log/catalog.cbor` and one `log/shards/<alias>.cbor` per expression,
     an alias's `/` and `:` spelled `~` and `+` in the file name. An expression's units, and the
     relations, attestations, commitments and withdrawals on them, are routed to its shard;
     manifests, redactions, datings and whatever is about no expression's unit stay in the
     catalog. `Library` still opens every shard into one `Store`, so nothing that reads
     `Library::store` changed; the substrate index's stamp became a function of every shard's
     log, and is the catalog's own while the catalog is the only shard, so a sidecar written
     before shards stays current. A layout-1 library opens as one whose only shard is the
     catalog and is raised to layout 2, marker first, by the first write that creates a shard.
     `Library::shards()` returns every shard, and the shard locks are `log/<stem>.lock`.
   - **`xtask determinism` could not register a command that writes**, so `text add` and `text
     show` — which §4.4 asks for — were left out of it at this step. The harness ran one fixed
     argv twice and compared stdout, and `text add` is a *write*: its second run legitimately
     prints `objects 0 written`, because the first run made them. Both are correct and the bytes
     differ. `text show` has the neighbouring problem, needing a library that exists.

     **Fixed straight after the step** rather than deferred to TX-P3, because the fix is small
     and the exemption was the kind that outlives its reason. `Op` gained a `%SCRATCH%` token,
     replaced by a directory created fresh for each of the sixteen captures and removed
     afterwards, and a `setup` command run in the same directory whose output is discarded and
     whose *failure* is not — a setup that silently did nothing would leave the real command
     printing the same refusal sixteen times and passing. Rule D's question for a writing
     operation is then the one it should always have been: the same input *and the same starting
     state* give the same bytes. Seven operations registered, all identical across sixteen runs.
     TX-P3's `date set` has the same shape and now needs nothing.
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
7. ~~Ports: C-Read for 14, 15, 18 and the ids. Step 1 gave all three the **names** and nothing
   else — the spec table reaches them through `verify-spec-tables.py`, and each now reports a
   manifest as named but not understood. What is left is the part that matters: decoding the
   bodies and **deriving tid, mid and rdid** from `fixtures/library/wire/ids.json`, which is the
   first identity work in those implementations since uids, and the only way the domain-byte
   separation gets a second reading.~~ **Done 2026-10-08.** `python/smysl/library.py`,
   `nodejs/src/library.js`, `go/library.go`, each with its own test file: 180 tests in Python,
   166 in Node, 92 in Go, and every vector in `ids.json` reproduced by all three. The
   domain-separation claim is exercised rather than transcribed — one preimage under all four
   identities, asserted to give four different digests — and the identity of each record is
   derived twice over, once from the hex preimage the fixture supplies and once from the record
   as decoded.

   *Exit:* **GE-T1 is partly met, and the part that is not is stated below** rather than
   claimed. Five findings:
   - **The Node port accepted a byte order mark**, which §2.6 forbids outright.
     `isNormalised` was the same five lines in all three ports and wrong in exactly one:
     JavaScript's `TextDecoder` *strips* a leading BOM unless `ignoreBOM` is set — the flag is
     named for what it does to the output, not for what it ignores — so the function decoded
     `EF BB BF` away and then truthfully reported that the text did not begin with U+FEFF. A
     part carrying a BOM would have verified. All three now test the bytes before decoding,
     which has no such default to know about. Python's `bytes.decode` keeps the BOM and its
     identical lines were right, which is the whole argument for a second and third reading.
   - **A mid cannot be taken from a re-encoded body in JavaScript**, and so is taken from the
     body's own bytes in all three. `Record.reencode()` already returns the original bytes
     there, because the language has one number type and a `binary32` zero re-encodes as an
     integer; a manifest's key 16 is the reader's metadata *verbatim*, an arbitrary map that
     may hold a float, so a re-encoded body would be a different mid for the same manifest in
     that language alone. The body is a slice of the record, which is exactly the preimage
     §2.6 names, and whether the port's own encoder reproduces those bytes is asked separately
     — the producer's question, with its own answer. All three pass both.
   - **`verify-spec-tables.py` could not see a hyphenated field name.** Its row patterns matched
     `(\w+)`, and no field in any table was hyphenated before 1.10 — so the pattern had always
     matched everything and dropped nothing. The manifest table has `parent-kind` and
     `part-policy`, and the gate skipped both rows, then reported them as *keys the
     implementation was missing*. Widened in all three language patterns. The gate now also
     covers the manifest and reading key tables in each port, the four domain bytes — asserting
     each equals the record code it names, and that every one is below `0xa0`, so none can
     collide with the head of a CBOR map and therefore with a uid — and `ids.json`'s own copy
     of that table. 68 comparisons.
   - **No reader was ported, deliberately**, and this plan says so already; it is restated
     because it is the question a reader of the ports will ask. The exit asks these three to
     recompute identities from the records the reference *emits*. Three parsers of USFM
     agreeing with each other would be a different and much weaker claim, with nothing to
     appeal to when they disagreed.
   - **GE-T1 is now met except for its corpus.** The port half is done: Python, JavaScript and
     Go recompute tid, mid, rdid and the structure hash from the emitted records and agree. The
     **second platform** was outstanding when this step landed and is not any more — the three
     port jobs run on `ubuntu-latest` and `macos-latest`, where every job in CI had been Linux,
     and the python job's uid-fixture regeneration is therefore checked on both. Worth being
     precise about what that buys, because it is not the identities: BLAKE3 over bytes and
     canonical CBOR are platform-independent by construction, so what a second runner exercises
     is *reading files* — line endings, NFC through a different Unicode table version, path
     behaviour — which is exactly the code a part's identity depends on, and exactly the shape
     of the defect the Node port had. (The ports were in fact developed and run on
     darwin/arm64, so the Linux leg is the one CI was already providing; what changed is that
     both are now required rather than one being incidental.) What is still owed is the
     **five Bibles and the JSON series**: a corpus this repository does not hold, whose
     licensing is TX-P2's problem, and what is checked here instead is a two-part fixture, one
     Latin and one Cyrillic, which exercises NFC and is not the criterion. It is a reader risk
     and arrives with the readers' own fixtures. Recorded on GE-T1's row in §6, where the next
     phase will look for it rather than in a step record it has no reason to re-read.

### TX-P2 — segments, languages, chats, redaction

1. ~~`segment`, `lang`, `analyze`; abbreviation lists. *Exit:* sentence-boundary F1 on a
   500-sentence gold set per tier-1 language, proposed bars ≥ 0.97 (en, es, fr, de) and ≥ 0.95
   (ru), not set by draft 3.~~ **Built 2026-10-09**, exit **partly met**: the harness is the exit
   test and the corpus is not.
   - `segment.rs` (`smysl/seg-uax29+abbr/1`), five abbreviation lists with their hash pinned to
     the id, `analyze.rs` (six chain ids), `lang.rs` behind `detect`. 71 new tests.
   - `tests/segment_f1.rs` **is** the exit test, and `SMYSL_SEG_GOLD=<dir>` points it at any
     corpus in `fixtures/segment/README.md`'s format — no code change. Against the ~30
     sentences per language in `fixtures/segment/gold/`, F1 is **1.000** for de, en, es and fr
     and **0.963** for ru. That set is a *development* set: it was written here beside the
     segmenter, two list rows and one algorithm rule were added because it failed on them, and
     30 sentences cannot distinguish 0.97 from 1.00. **The 500-sentence gold set per tier-1
     language is outstanding**, recorded on GE-T1's row in §6 beside the Bibles — it needs prose
     nobody here wrote and boundaries annotated by someone who is not the segmenter's author.
   - Four findings, all in §3.2 and §3.7: the abbreviation lists are one rule of four, not the
     whole algorithm; `rust-stemmers` hard-depends on `serde` and had to be gated; `lingua`'s
     cost is 57 crates and 14.4 MB; and detection cannot be automatic, because a manifest's
     language is part of its mid.
2. ~~Readers `telegram`, `slack`, `whatsapp` with `--pseudonymise`; opt-in `mbox`, `epub`,
   `html`, `pdf`. *Exit:* chat round trip with `raw` intact; `E450` test; fuzz targets.~~
   **The three chat readers and `--pseudonymise` are built; the four opt-in readers and the
   `E450` test are not, and the second of those is deliberate.**
   - The exit's first and third parts are met: `a_chat_export_round_trips_with_its_raw_metadata_intact`
     adds each fixture through `Library::add` and compares the manifest's key 16 against the
     bytes the reader produced, and all three readers are in the `readers` fuzz target — which
     `make seed-fuzz` now seeds from `fixtures/library/readers/`, once per choice byte, because
     random bytes are not a zip and the `slack/1` arm would otherwise never be reached.
   - **`E450` moves to step 3**, where `text append` exists. The registry rule in force is that
     a code nothing can trigger is worse than a missing one, and nothing can trigger this one
     until there is an append to refuse. What step 2 settled is that it needs no new manifest
     key: a reading whose speakers are pseudonyms says so in its own rows.
   - **The opt-in four (`mbox`, `epub`, `html`, `pdf`) are not built.** They are opt-in, in no
     feature `cli` turns on, and each brings a parser of its own to measure; the three the phase
     is named for are the ones a chat corpus needs. Recorded as outstanding here rather than
     quietly dropped.
   - Six corrections and two deliberate refusals, all in §3.2: the day is a boundary level and
     not a part count; the day cannot come from a Slack file name or a Telegram `date`;
     pseudonymisation changes no tid, and therefore no name in the prose; it does change the
     `structure` hash (**OQ-70**); the key is an argument because a pure crate cannot mint a
     secret; and `whatsapp/1`'s date order is the parameter that widened manifest key 3's
     grammar in the format spec.
3. ~~`text append`. *Exit:* growth test (§5.2).~~ **Built.** `tests/cmd_text_append.rs` is the
   growth test §5.2 asks for — three days of a Telegram export appended one day at a time,
   through the binary — beside seven cases at the library layer. `SMY-E450` is registered and
   raisable, which is the item step 2 moved here. Five findings, all in §3.1: a part that was
   last is rewritten once (and only one is, whatever the corpus size); tid reuse is a property
   of the part policy; an unchanged append writes nothing; the command's spelling is
   `--alias` rather than a second positional, and the flags it inherits are refused rather than
   ignored; and appending to one side of a fork does not merge it.
   - Two things found by writing it, neither in the plan. `Library::add` wrote the bare reader
     **id** into record 18 key 1 while the manifest's key 3 held the id *and its parameters*,
     so the two spellings of one thing gave the two paths two different rdids — invisible until
     step 2 added the only sample with a parameter. And `tests/readers.rs` *printed* `lossy` and
     never recorded manifest key 15, so the mid it pinned for a lossy sample was not the mid
     `Library::add` produces; it pinned nothing wrong until step 2, because until the chat
     samples arrived no sample was lossy. Both are fixed, and the library-level cross-check now
     uses a parameterised reader instead of `notes.txt`, which has neither property.
4. ~~Record 19, rule Z in `Store::append`, `text redact`, `@redact`; `E452` refuses a 15 or 18
   offered to a log (OQ-39, so there is no `rewrite_redacted` to write). *Exit:* redaction test;
   P-Z1–P-Z4 harness green; ports decode 19.~~ **Built, with rule Z one layer down from where
   this said it would be.**
   - Record 19 decodes in this build and in all three ports, which now carry its key table and
     are checked against the spec's by `make spec-tables` (72 comparisons, up from 68). The wire
     fixture carries a **real** redaction instead of the `Unknown { code: 19 }` placeholder it
     had — over a part the fixture deliberately does not hold, which is the state honouring a
     redaction leaves behind.
   - `@redact` parses, writes and round-trips; `text redact <tid> --as AGENT [--at MS]
     [--reason UID]`; `xtask determinism` registers `text_redact` as its ninth operation and
     fourth that writes.
   - The exit is met: `tests/cmd_redact.rs` is the redaction test, `P-Z1`–`P-Z4` are green in
     `crates/smysl-text/tests/redaction_algebra.rs` — **moved there from `smysl-graph`**, because
     two of the four are about records 15 and 18 and a `Store` never holds one, so in a store
     they are vacuously true — and `fuzz/fuzz_targets/redaction_merge.rs` fuzzes the record-level
     half (70,459 runs, nothing found).
   - Five findings in §3.1, of which the load-bearing one is that §4.3.2's filter in
     `Store::append` is unreachable: `SMY-E452` already refuses every 15 and 18 before it. The
     others: a reading goes with its part; opening a library is a write in exactly one case;
     "a redaction violation" needs no diagnostic code because the open path makes it
     unrepresentable; and a redacted passage is now a sentence rather than a missing file.
   - And one gap that was not this step's: **`make fuzz` never ran the `readers` target.** It has
     existed since TX-P1 step 3, `fuzz-build` compiled it and `seed-fuzz` seeded it, and the
     `FUZZ_TARGETS` list did not name it. Found by reading the list while adding
     `redaction_merge` to it.
5. ~~`proposition::classes` — strict per SMYSL-2.3, `component` with diameter, `attested:n` —
   **moved here from TX-P7 step 1** by §0.1. It is pure, needs only `Store` and records, and
   TX-P5's consensus output cannot be read before it exists. *Exit:* strict class counts match a
   fixture computed by the Python reference (SMYSL-2.3 conformance) and are invariant under record
   order.~~ **Built, in `smysl-graph` rather than in `smysl-text`.**
   - **The crate moved, and A-13 is why.** §4.1's tree puts `proposition/` in `smysl-text`,
     beside the engines that propose the edges — and those belong there, since `lexical` needs
     retrieval and `anchored` needs a `Library`. Counting the classes needs neither, and it has
     to be reachable from **`smysl-check`**: A-13 obliges `C-Library` to "derive `strict` classes
     per A-12.4", and §4.3.3 allows `smysl-check` exactly one edge to `smysl-text` (the `Time`
     pass's). A module a conformance class needs and a checker cannot reach would be an
     obligation nothing can discharge. `smysl::proposition` re-exports it.
   - **The exit is two implementations, and the fixture is split so that it is.**
     `crates/smysl-graph/tests/proposition_classes.rs` writes `fixtures/proposition/store.cbor`
     and never the expectation; `scripts/gen-proposition-classes.py` — an independent reading of
     A-12.4 over the Python port's decoder — writes `classes.json` and never the store; each
     compares against the other's file, `python/tests/test_proposition.py` keeps the Python
     reading live, and `make proposition-classes` is a CI job. One generator producing both
     halves would be one implementation agreeing with itself, which is the failure
     `verify-spec-tables.py`'s own header records.
   - **`scope` is an `Option`**, not §4.2's bare `&BTreeSet<Uid>`. With a bare set the empty one
     has to mean something and both readings are wrong: "every unit" makes a caller whose
     computed scope came out empty count the whole store, and "no unit" makes the obvious way to
     ask for all of them return nothing.
   - Three things the fixture pins that a smaller one would not. A path is **two** strict classes
     and one component of diameter 2 — the case a union-find implementation gets silently wrong,
     since it agrees about the triangle and merges the path. A singleton class is a normal
     outcome: the loose end of a path has an edge, so it is in A-12.4's vertex set, and it cannot
     join the class its neighbour opened. And uid order is over the **32 bytes**, not over the
     base32 text: §2.1's alphabet puts the digits after the letters, so the two orders differ —
     the fixture holds a uid beginning with `4` so that they differ *here*, and a Python
     assertion that sorted the text failed on it before the reference did anything wrong.
   - **A-12.4 is not folded into the spec**, and that is deliberate. D-7 (OQ-7) was **not**
     carried by the G0 acceptance — SMYSL-2.3 §0 says so explicitly — so the class definition is
     an accepted *implementation* of an amendment that is still for discussion. The code and the
     fixtures point at A-12.4; `SMYSL_FORMAT_SPEC.md` says nothing about classes until D-7 is
     accepted, which is what A-14's fold rule requires rather than a gap.
6. ~~*Exit for the phase:* **GE-T1** on two chat exports; **GE-T14** curve recorded and default
   part policy fixed.~~ **Both done, and the phase is closed.**
   - **GE-T14**: the curve is recorded in §3.1 and the default moved from 64 KiB to **1 KiB**.
     Three of its four axes turned out not to depend on the size at all, which is what made the
     decision a short argument rather than a balance: the boundary level is the structural
     judgement, and a minimum above the typical node size silently overrides it. Changing the
     default moved every fixture's mid and no fixture's *cut* — they are all under 1 KiB — which
     is exactly the event manifest key 17 exists for.
   - **GE-T1, the chat half**: `fixtures/library/chat-wire/` holds the records a chat `text add`
     emits — three exports, three readers, one catalog — and the three ports recompute every tid,
     structure hash, rdid and mid from them. That is the half `fixtures/library/wire/` could not
     give them: its records are hand-built in `smysl-core`, correctly, in a crate with no reader,
     so no port had ever hashed a segment table with a **speaker** in it, a `mul` manifest, or a
     manifest carrying opaque `raw` metadata inside its mid. A port that dropped an opaque key
     would decode the record and disagree about the identity, with nothing else to notice.
   - **GE-T1, the second platform**: the three port jobs have run on Linux and macOS since
     1.10.0, and the Rust library — the implementation that *writes* the identities — did not.
     It does now (`library-platforms`), over the suites that pin identities. What that tests is
     not the hashing, which is platform-independent by construction, but **reading files**: line
     endings, NFC through another Unicode table version, path behaviour — and the Slack reader's
     zip path, which is the one place a path separator could reach a locator.
   - What stays owed is corpora and not code, as it was before: the five Bibles and the JSON
     series for GE-T1, the 500-sentence boundary gold set from step 1, and the question of
     whether GE-T14's synthetic node-size distributions match real traffic.

### TX-P3 — time

1. ~~`smysl-core`: `edtf.rs`, `source.published` (FC-2), record 17, `@date`, `Did`; time
   `DetectionKind` values for derivation only. *Exit:* EDTF fixtures; round trips; ports decode
   17.~~ **Done.** `Did` already existed (TX-P1 step 1 built all four identities). The exit is
   met: `fixtures/library/edtf/cases.json` is the EDTF fixture, `tests/dating.rs` round-trips
   every target kind against every value kind through CBOR and every spellable one through text,
   and all three ports decode record 17 out of `fixtures/library/wire/`, with its did, against
   three datings chosen one per target kind and one per value kind.

   **Four departures, each recorded where it lands.**

   - **The two `DetectionKind` values move to step 2.** `DetectionKind::code()` is total — every
     kind names the diagnostic it is reported as — so kinds 4 and 5 cannot land before `W412` and
     `W413` can be raised, and nothing raises them until the engine computes an interval. The
     registry's rule is that a code nothing can trigger is worse than a missing one; honouring it
     and keeping the mapping total means the variants arrive with `time::stn`. Nothing in step 1
     needed them: `ContentionId::derive` takes a kind and does not enumerate them.
   - **`Store` gained `datings` and `datings_by_target`** (§4.3.2), which the plan does not
     assign to a step. Left out, `absorb` would have decoded record 17 and dropped it on the
     floor — the silent-drop class this plan keeps closing — and the adjacency deny-list's own
     comment asks for the variant to join it in the commit that adds it.
   - **The interval conversion stays for step 2.** §3.3 puts `Instant`, `Bound` and `Interval` in
     `smysl-text::time`, and nothing in `smysl-core` needs them: EDTF *syntax* is what the
     surface parser and the CBOR producers have to refuse, and that is all step 1 ships.
   - **A-2.4's copy and omission rules are folded into the spec but not yet enforced.** They are
     a producer's rules and `provenance::stamp` is step 2's. Writing them down with the key they
     govern is the point of a fold; the gate that tests them is `P-E` and arrives with the code.

   Two things found on the way, neither in the plan. The `spec-tables` gate was comparing the
   `source` sub-map as a **named subset of three keys**, so `observed` — a key since 1.8 — was
   missing from the Python and Node producers and from `fixtures/wire/uid/cases.json`, and no
   gate asked. The subset is now the whole table, both producers write both keys, and the uid
   fixture carries a case for each. And a dating's offset is the **first signed integer any
   record in this format has ever carried**, so `Enc::int`/`Dec::int` are new and the three ports
   had never decoded CBOR major type 1 out of a smysl store; the fixture's offset is negative for
   that reason.
2. ~~`time::{calendar, constraints, stn, engine}`; copy and omission rules in
   `provenance::stamp`; `DetectionKind` 4 and 5 with `W412`/`W413` (moved from step 1, above).
   *Exit:* P-E1–P-E8 green; omission rule tested at second vs millisecond precision.~~
   **Done.** `crates/smysl-text/tests/time_algebra.rs` holds `P-E1`–`P-E8` and three worked
   cases; `fuzz/fuzz_targets/{time_engine,edtf}.rs` drive the same properties from the shared
   generator. A-12.2 is folded into the spec as §6.4 and `make spec-tables` reads rule E's row,
   its four time statuses and its five free constraints out of the document (85 comparisons).

   **The omission rule fires almost never, and that is the rule working.** A-2.4 rule 3 omits
   `published` when its interval *equals* the one millisecond an `observed` denotes. EDTF level
   1 cannot write a millisecond, so the narrowest value it can express is a second — and
   `2026-09-30T14:05:00Z` is a one-second interval that is not equal to any instant inside it.
   What the rule prevents is a producer reasoning "these say the same thing, drop one" and
   dropping the field that carried a second of uncertainty, which would move the unit's uid. The
   exit asks for the test at both precisions and `provenance.rs` has it.

   **Decisions this step had to make because the amendment names words and no arithmetic**, all
   three recorded as **OQ-71** with the implemented reading as the proposal: a qualifier is not
   a width; a season is its year; and four of the seven Allen relations collapse to "at the same
   time as" for a subject that is an instant with uncertainty rather than a duration. Every
   reading chosen is **weaker** than the relation it stands for, so the collapse costs precision
   and never soundness — which is what `P-E4` checks over generated corpora.

   **Two things the step found.** `SPFA` did not terminate on a network with a negative
   self-loop beside a zero-weight edge: the zero edge tied at the runaway distance, the
   tie-break re-enqueued the node, and the relaxation counter never fired because a tie moves
   nothing. Found by `P-E6`'s generator on a five-variable network, at case 108 of 400. The fix
   is that a tie updates attribution and does not propagate — which is also correct, since what
   a node records is the edge it was *reached by*, so a tie at one node tells nothing downstream.
   And the **first-seen** constraint is sharper than it looks: a unit whose `observed` is after
   the earliest attestation of it is a contradiction with no dating in it at all, which is now
   its own test.

   **What this release cannot compute, stated rather than absent.** A manifest-scoped dating
   resolves through the manifest's part list, because a unit does not name its manifest until
   FC-3 (TX-P5); that over-selects on a supersession chain, where two manifests share a part,
   and `Effective::inexact_scopes` counts it. The **reply** free constraint has no data: the
   ids it would be built from live in a segment row and reach a unit only through ingest. And
   nothing dates a manifest, which is **OQ-72**.
3. ~~`smysl-check` `Time` pass. *Exit:* `E410`, `W411`, `W412`, `W413`, `W449` fixtures.~~
   **Done**, as pass 13. Five fixtures are `.smy` documents in
   `fixtures/conformance/check/time/` — one per code, plus the `E060` a dating's target raises,
   which is the one reference the integrity pass cannot see. They sit in a subdirectory of
   their own because the pass is behind a feature: the suite picks them up when
   `Pass::Time.is_implemented()` and not otherwise, and keying that on the directory rather
   than on the code set is what lets `E060` be expected by both a time fixture and an
   integrity one. `E410`'s is a Rust test, and the reason is a property of the
   design rather than a gap in the suite — **the surface parser refuses a malformed EDTF value
   as it reads it, so no document can produce a store that holds one.** The only way a store
   gets one is CBOR from an implementation that wrote it, and the only way to test for that is
   to build the store. The registry is 78, which completes this RFC's `E401`–`W459` range
   except `E404` and `W405`, both of which read `source.span` (TX-P5).

   **The pass is behind a `text` feature on `smysl-check`.** Rule E's engine is in
   `smysl-text`, so the pass needs a dependency edge this crate did not have; §4.1 sanctions
   it (`smysl-text` has no `smysl-check` dependency, so no cycle, and its default tree is pure,
   so no crates). With the feature off, `Pass::Time` is in `ALL` and not in `IMPLEMENTED`, so a
   report says the pass did not run — the machinery `Codec`, `Retraction` and `Hashes` already
   use. A `Pass` enumeration that changed shape with a feature would make `--pass time` an
   unknown name in one build and a known one in another.

   **Two placements the step had to decide.** `SMY-E410` joins the **shape** family in
   `ConformanceClass::forbids`, not the library one: A-13 gives C-Produce "write only
   well-formed EDTF", so a store carrying a malformed value was not produced conformantly,
   while C-Library's obligation is to derive effective time and report contested subjects —
   which it can do with one unreadable date in the store. And rule E's four warnings block
   nothing at any class, because a contested subject is a fact about a corpus rather than a
   defect in it.

   **A dating's target is the first reference the integrity pass cannot see**, because a dating
   moves no edge and is therefore not in the adjacency. A target naming nothing is `SMY-E060`,
   raised here; a target that is present and selects nothing — a window over instants no unit
   has — is `SMY-W412`, because the records it would select may arrive later and a dating
   written ahead of an ingest is how a known clock error gets recorded first.

   **Found: the surface form cannot spell a did.** A-6 lets a withdrawal (key 0) and a
   commitment (key 0) name a dating, and `@withdraw`/`@commit` take a uid or a label — so a
   held dating has no surface spelling on the way *in*, and on the way *out* the writer prints
   its 32 bytes as `b3:…`, which claims to be a uid. The round trip is byte-exact, so this is
   a legibility defect rather than a data one, and it is why the held-dating test builds its
   store rather than parsing a document. Recorded as **OQ-73**.
4. ~~`date set/order/show`, `--why`. *Exit:* §19.2 scenario (§5.2).~~ **Done**, as the
   twenty-eighth command, with `tests/cmd_date.rs` as the scenario and `date show` registered
   with `xtask determinism`. The exit is met in full: a window-target dating with
   `offset:-93000` and a `cited` basis re-times exactly the messages in the window, the
   quotation-order `W413` disappears, **no uid moves** (asserted with `diff`, which answers
   that question exactly), the same dating with no basis gives `W412` and is not applied, and
   a `canonical` commitment on a dating holds a later one until a resolution names the derived
   contention.

   **Four defects found by running the scenario, none of them in the command the step added.**

   - **`persist` had no arm for a dating**, so `date set` on a surface document was refused by
     the one writer every record-writing command goes through. `@date` had been in the grammar
     since step 1 and `dating_has_surface_form` existed to answer this exact question; nothing
     called it. A predicate with no caller is not covered by the tests that cover the predicate.
   - **An offset sat outside A-12.2's no-silent-override.** Step 2 applied one unconditionally
     and took `min(dating status, seed status)`, so an unevidenced `offset:` shifted a
     `measured` instant and relabelled it `speculative`. A-12.2 and §5.2 agree the move is not
     made and is reported. Found because the test that appeared to cover it covered the
     *absolute* path: its comment described an offset and its body built an absolute dating.
   - **A held dating named no contention**, and `resolve` refused every derived id, so the
     documented release path was unreachable for every rule-E contention. `engine::hold_contention`
     is now public and the diagnostic prints the id; `resolve` consults the derived set when the
     recorded lookup fails.
   - **`commit` could not name a dating by did**, which is how A-12.2's lock is written (A-6
     key 0). The did's bytes are the key `held_by_lock` already reads.

   **One asymmetry decided.** A hold's contention is over the dating's **target**; an outranked
   move's is over the **subject**. A hold is one fact about one dating however many subjects it
   would have moved; an outranked move is one fact per subject, because a dating may outrank the
   value over one unit and not another. Keying a hold by subject would make releasing a
   part-target hold take one resolution per unit in the part.

   **Two spellings the plan does not give.** A window is `--window FROM..TO` beside a tid
   target, mirroring the surface form's `window: [from, to]` rather than inventing a
   `<tid>@a..b` compound — one spelling per thing, in the document and on the command line. And
   `date order` accepts all seven Allen words rather than only `before`, because `during` is the
   commonest of the other six and restricting the sugar would have left `--value during:<b>` as
   the only way to say it.

   **`engine::subjects` is public** so the CLI expands a target exactly as the engine does. A
   second implementation of "a part is the units drawn from it, a manifest is the units of its
   parts, a window is the subset whose as-recorded instant falls inside it" would drift from
   this one the first time a target kind gained a rule.
5. ~~*Exit for the phase:* **GE-T13**.~~ **Ran**, and §5.5's row records what it measured.
   Three clauses of four are met and the fourth is a corpus debt, so the phase exits the way
   TX-P1 and TX-P2 did: on an instrument that is built, against material that is partly owed,
   with the owing recorded against the gate that wants it rather than counted as done.

   **What the gate found: nothing.** That is worth stating rather than passing over, because
   the three steps before it each found something, and the difference is informative. Steps
   1–4 were each the first caller of what the step before had built, and every defect they
   found was at that seam — a `DetectionKind` with no diagnostic, a surface form with no
   writer, a predicate with no caller, a diagnostic with no id. GE-T13 is not a new caller:
   it is the same `effective` the `Time` pass and `date show` already drive, over different
   data. A gate that finds nothing after four that found something is evidence about where
   this codebase's faults live, which is at the joins and not in the arithmetic.

   **One thing the gate could not reach, and it is a property of the sources.**
   `whatsapp.txt` contributes to the consistent-data clause and nothing else: a WhatsApp
   export records no reply, so there is no ordering in it for a skew to contradict, and a
   harness that detected one would be inventing a fault. The two readers that do record an
   ordering spell it differently — `telegram/1` gives the message replied to and `slack/1`
   gives the thread root — and a thread root names *itself*, which is a message and not an
   ordering. Reading it as one would put a unit before itself and plant a fault the export
   does not contain.

### TX-P4 — substrate index and retrieval

1. ~~`index` tables of §3.8 with rebuild and incremental builders. *Exit:* rebuild ≡ incremental
   (byte comparison of a table dump); deleting `index.redb` and rebuilding restores every derived
   table; the operational `journal` is rebuilt by re-checking staged/stored units.~~ **Done
   2026-10-10.** `crates/smysl-text/src/index/` — `mod.rs` (the `Table` enumeration, the `Index`
   handle, the deterministic dump, the sidecar), `tables.rs` (six `rebuild`/`on_append` pairs)
   and `journal.rs` (the seventh). Seven of §3.8's twelve tables are built and the other five
   name the step that will build them, on the pattern `smysl_check::Pass` set. `text index`
   reports them and `--rebuild` writes the sidecar; `text add`, `text append` and `text redact`
   maintain it incrementally and persist it. `crates/smysl-text/tests/index.rs`, 14 tests.

   **The exit's first clause caught two of the five shapes it had to.** `rebuild ≡ incremental`
   over a grown library is necessary and **not sufficient**, which was measured rather than
   reasoned about: each of the five table shapes was broken in turn and the suite rerun. A
   rebuild walks the corpus in identity order and an append walks it in arrival order, so five
   tables needed a shape that does not depend on which record was seen last — and the
   whole-corpus comparison failed for only two of them.

   | shape | caught by the whole-corpus comparison? |
   |---|---|
   | `by_part` mids inserted in order, not appended | yes |
   | `manifests.head` unseated by a successor's arrival | yes |
   | `segments` rows dropped when rule Z unlinks a reading | no — needs a redaction in the corpus |
   | `locators` tables dropped for every manifest of a redacted part | no — same |
   | `parts` holding a **set** of what each manifest records | no — needs two manifests that disagree about one part |

   The three the comparison missed are each a corpus the fixture did not contain. The last is
   the sharpest: every manifest a grown library here produces comes from `txt/1`, so every
   shared part is recorded identically and a last-wins table agrees with a rebuild **by
   accident**. Two readers over one part — same bytes, one tid, two readings — is the real case,
   and it is reached by appending a manifest directly. So the clause names a gate that needs a
   corpus chosen to defeat it, and the four cases are named in the test file rather than left to
   the next person to rediscover.

   **`on_append(&[Record])` cannot build three of the six tables.** `parts`, `segments` and
   `locators` are built from records **15** and **18**, and OQ-39's answer is that a log never
   holds one — they are in the object store, addressed by the identities a manifest records. A
   builder handed only the appended records can keep `manifests`, `by_part` and `redactions`
   current and has no way to reach the other three. The signature is `on_append(&Library,
   &[Record])`: the records say what changed, the library is where the changed thing is read
   from. §3.8's text is amended.

   **`index.redb` is not here, and the reason is `rust-version`.** Cargo gives `rust-version`
   per **package**, not per feature — the workspace manifest's own comment says so, which is why
   `smysl-tui` and the facade declare 1.88 for every build and not only for the feature that
   needs it. `redb` 4.3 requires 1.90, and `smysl-text` is a base-tier crate at 1.85 that
   `smysl-check` and the facade depend on. Adding the dependency here would move the floor for
   every build of the substrate to buy a backing nobody had turned on, which is a cost §3.8 did
   not weigh because it was written before the MSRV tiers were measured (OQ-40, closed 1.10.0).
   What this step owes is the tables, the builders and the gate; where the bytes rest is a
   backing decision, and the right place for it is a crate of its own. **Recorded as owed**, and
   §3.8's own sentence — "the same queries run against in-memory maps rebuilt on open, slower,
   same answers" — is the licence to take it separately.

   The sidecar is a third option §3.8 did not have: `index/substrate.idx`, in the directory §4.1
   reserves, written deterministically and checked against the catalog's length and hash on open,
   exactly as `smysl_graph::store::index` has done since SM-P3. It makes the featureless path
   cost **one** rebuild rather than one per open, which is strictly better than the path §3.8
   described. A sidecar that is stale, truncated, from another version or not an index at all is
   a rebuild and never a refusal: a derived cache that can stop a library opening is worse than
   no cache.

   **The journal's clause splits, and only half of it is recoverable.** "Rebuilt by re-checking
   staged/stored units" is two claims. *Coverage* — which units came out of which part, at what
   status — is recomputable from the units alone and is recomputed; this is what makes a lost
   journal survivable, since the corpus can still say what was ingested, which is the question a
   resume asks. *Cost* — the per-language retry rate and the token figures TX-P5 step 5 wants a
   report of — is written by the ingest that spends it and no amount of re-checking recovers a
   number nobody recorded. There is no writer in this build, so there is nothing to recover and
   nothing to pretend about. The **staged** half is the same story from the other end: today's
   staging is `.smysl/staged.smy` under a *project* root, and the per-shard
   `<library>/stage/<shard>/staged.smy` of §3.9 arrives with TX-P5. A scanner written here would
   be a reader for a file no writer in this build produces — the shape of mistake TX-P2 step 4
   removed when it deleted `by_mid` for having no source of values.

   **Rule Z's ordering makes the incremental builder deaf**, and that is the one place the two
   builders learn of a change differently. `redact` appends record 19 **before** it unlinks the
   objects, deliberately, so that a process killed between the two is repairable on the next open
   — which means the incremental builder runs while the objects are still readable and cannot
   learn of the loss by failing to read one. It has to be told by the record, in two arms.

   **An index may be partial and is never half-trusted.** A rebuild reads objects, and an object
   is missing for a redacted part and unreadable for a corrupt one; neither may fail an open. So
   the rebuild is lenient and completeness is tracked **per manifest**: a mid has a locator table
   only when every one of its parts resolved into it, and `resolve` falls back to the search for
   a mid that is not there. The rule that forces this is that the index must not change an
   answer — a redacted part makes the search *refuse*, and a locator table with the redacted
   addresses quietly removed would answer `Ok(None)` where the library refuses. Half-built and
   complete cannot be told apart at lookup time, so the distinction has to be made per manifest
   or not at all.

   **`store::library::part_of` is `pub(super)` and `Store::part_of_reference` is public.** The
   journal's two builders both have to answer "does this source name a part", and the answer's
   whole content is that it parses the canonical 52-character form and nothing shorter. A second
   copy would be a second place that rule could be relaxed by accident.

   **The three ports gain nothing, and that is the correct answer rather than a gap.** Every
   TX-P1 and TX-P2 step ended with a port change, because each added something to the *wire*:
   a record type, an identity, a surface form. An index is derived, never travels in a bundle,
   and is addressed by nothing — so D-8's C-Read obligation does not reach it. A second
   implementation that wanted one would build its own; a second implementation that wanted to
   read a corpus needs none, which is the same reason a stale sidecar is a rebuild and not a
   refusal. The one thing the index touches that a port could see is `Store::part_of_reference`,
   and that is a parse the ports already perform.

   **Opening a library stays cheap, which §3.8 did not ask for and would have cost.** "The same
   queries run against in-memory maps rebuilt on open" is affordable for the five tables derived
   from the catalog — arithmetic over a log the open has already parsed — and not for the two
   derived from the object store, which cost one object read and one structure tree **per part**.
   At the measured 1 KiB part minimum (GE-T14) a single Bible is thousands of parts, so an
   unconditional rebuild would put tens of thousands of reads in front of `text ls`, which wants
   none of them. So an open builds the five and leaves the two; `text index --rebuild` and the
   three writing commands — which hold the locks and are already paying for I/O — build all
   seven. The split is a field in the dump rather than an assumption: a partial index reports its
   two object tables as *not built for this library* instead of as empty, answers no locator
   question, and refuses to be persisted, because a half-built sidecar would load as current on
   the next open and the catalog's hash would agree.
2. ~~`Analyze`/`Postings` seams; persistent postings per partition; `find --lang --analyzer`.
   *Exit:* `find` p95 < 200 ms at 155k units (13 s today, F-10) on the Appendix E corpus;
   results identical to the in-memory path.~~ **Done 2026-10-10.** The seams in
   `crates/smysl-retrieve/src/{analyze,postings}.rs` — `Analyze` with `Tokenizer` implementing
   it, `Postings` with `MemPostings` implementing it, and `Bm25View` scoring over either. The
   table in `crates/smysl-text/src/index/postings.rs`, which is §3.8's eleventh table and the
   first one step 1 left unbuilt. `find --lang`, `--analyzer` and `--library`; `text index`
   reports the partitions and the chains that analysed them. `smysl-retrieve/tests/postings.rs`
   (10 tests), three more in `smysl-text/tests/index.rs`, and the harness in
   `smysl-text/tests/postings_timing.rs`.

   **The second half of the exit clause is met in its strongest form: the two paths agree bit
   for bit.** Not about the ranking — about the whole result, uid, `f32` score and per-term
   decomposition, over twelve query shapes and both tokenisers. That is a brittle test and the
   brittleness is the point: a scorer that agreed about the order and disagreed in the fourth
   decimal place would pass a looser test and would have changed every `--why` line the tool has
   printed, which `doc-output` compares. So `Bm25View` reproduces the `bm25` crate's arithmetic
   operation for operation — `(1 + (N - df + 0.5)/(df + 0.5)).ln()`, `tf·(k1+1)/(tf + k1·((1-b) +
   b·(len/avgdl)))`, `avgdl` as an `f64` quotient cast to `f32`, and the sum taken over the
   query's tokens **in order and with repetition**.

   The summation order is the part that is easy to lose, and the fast implementation *looks* like
   it loses it. Scoring term-by-term across documents rather than document-by-document across
   terms is not a reassociation: each document's accumulator receives its contributions in query
   order either way. The two differences are exact — a term the document lacks contributed
   `idf · 0.0`, and adding zero to a non-negative float is the identity; a term the corpus lacks
   contributed the same zero to every document and is skipped whole.

   **The one place the two paths can disagree is the one place this path is right.** The `bm25`
   crate identifies a term by `hash32` of it and counts document frequency per *hash*, so two
   terms that collide share a `df` and a `tf` there. A postings table keyed by the term cannot
   collide. Over a 50k vocabulary the birthday bound puts a colliding pair at a few parts in a
   thousand, so this is not theoretical at corpus scale — and the divergence is recorded rather
   than reproduced, because a seam that imitated a 32-bit hash to stay bit-compatible with it
   would be preserving a defect.

   **The first half of the clause is met for the query and not for the command, and the gap is a
   floor this phase cannot move.** Measured on this machine (`--release`, 155,510 synthetic units
   with Appendix E's 25-word gists, five manifests):

   | quantity | before | after |
   |---|---:|---:|
   | one query, p95 over 20 queries | 2,034 ms | **34 ms** |
   | opening the library | 613 ms | 625 ms |
   | `find`, end to end | ~2.6 s | **659 ms** |
   | building every table once | — | 765 ms |
   | the sidecar | — | 169 MB |

   So the query is **60× faster** and comfortably inside 200 ms; the command is 4× faster and
   outside it, because 625 ms of it is parsing a 155k-unit catalog. That is SMYSL-2.8's M-7 —
   "the sidecar never short-circuits a load" — and it belongs to TX-P13's layered store, not
   here. A clause written as "`find` p95" cannot be met by any index while the load costs three
   times the budget, and saying which half was moved is more useful than reporting a number for
   the half that was.

   **The exit clause's corpus is a store and this table is a library table.** "The Appendix E
   corpus" is `bench-en-155510.smy`, and a partition is a manifest and a language — a bare store
   has neither. Nothing careful makes a figure from one a figure about the other, so the harness
   builds a library of the same size and times the two quantities the clause is actually
   comparing: a query against a corpus that has been analysed, and a query against one that has
   not. Giving a *store* persistent postings would mean an analyzer in `smysl-graph`, which is
   the layering this crate tree exists to prevent.

   **Step 1's `journal` rebuild was quadratic, and this step's harness is what found it.** Each
   row was inserted in `(tid, uid)` order into a `Vec`, which moves the tail: at 155k units that
   is **4.53 s** of a 4.96 s open — six times the whole postings table — and step 1 had put the
   table in the class it called arithmetic over a parsed log. The reasoning was right about the
   table and wrong about the loop. Collected, sorted and deduped, it is **58 ms**, and the cost
   class step 1 claimed is true after all. The lesson is the one this phase keeps relearning: a
   cost class is a measurement, and a fixture of nine lines measures nothing.

   **`objects_read` became a bitmask**, which bumped the sidecar to version 2. Step 1's boolean
   was the right shape for two expensive tables and the wrong one for a third: `postings` reads no
   object and is still the dearest table here, so "which tables does this index hold" stopped
   being a boolean. `Built` is a `u16` over `Table::ALL` positions, `Table::ON_OPEN` names the
   five an open can afford, and steps 3 and 4 will need the same distinction.

    **The 169 MB sidecar is what will force §3.8's redb decision.** A posting is a 32-byte
   content-addressed uid and a `u32`, so 25 terms a unit is about 1.1 KB a unit — 169 MB at 155k,
   and 7 GB at the 10⁶ units SMYSL-2.8 sizes for, all of it in memory while the library is open.
   The usual remedy does not apply: a uid is a hash, so a postings list of uids **cannot** be
   delta-encoded, and the fix is an internal dense document id — a `u32` per unit, which would
   take the posting from 36 bytes to about 5. That is a format decision about the table and
   belongs with the backing, so it is **recorded as owed** beside `index.redb` rather than taken
   here. At 155k the sidecar costs about as much to decode as the five tables cost to rebuild,
   which is why it is adequate now and will not be.

   **Decided 2026-10-11, and the two halves turned out not to be one decision.** The owner chose
   the redb backing and chose it *before* TX-P5 step 3, on the reasoning that step 3 is the first
   thing to write units at corpus scale and G1 calls this the last cheap point to change the
   table. What the options review found is that the 169 MB is two costs — bytes written and bytes
   resident — and only one of them is the dense id's to fix:

   - A **dense document id** fixes the written half cheaply and the resident half only if the id
     is resident, and a resident one cannot be maintained incrementally without giving up this
     section's byte-for-byte rebuild gate. A content-derived id is a rank within its partition —
     that is what makes it a function of the library rather than of arrival order, which the gate
     requires — so appending one window to a partition of size *p* re-ranks *p* postings. Ingest
     commits window by window, which makes it quadratic per manifest. Assigned at *encode* time
     instead it keeps the gate and the appends and buys the disk win alone.
   - **Paging the table** makes the resident cost a page cache the caller sizes, and leaves the
     gate alone.

   So redb is the decision and the dense id becomes an on-disk encoding to take afterwards,
   where it is a change to one value format and nothing else. Scope: `postings` first, since that
   is where both figures live; `segments` and `locators` follow, where what the store buys is
   persistence without a rebuild rather than memory; the five tables an open can afford stay in
   the sidecar, which is reproducible and which this file is not.

   **`--library` has no corpus the CLI can give it yet**, and that is TX-P5's to fix. No command
   in this build puts a unit in a library: `text` writes manifests, parts and readings, and
   `import` and `merge` write stores. The flag's path is exercised in the library's own suite,
   where a test can append unit records, and end to end by merging a store of units into a
   catalog by hand — which is how the transcript in the manual was produced and not a workflow
   anyone should copy. Ingest is what fills a library, and it is the next phase.

   **The analyzer is settled before the table is looked for.** The first version of
   `find --library` fell back to `Bm25::index` when the index held no postings, which splits the
   query with the plain tokeniser — so the fallback returned 3.40 for a query the table scored at
   5.13, the same ranking under a different analysis. The chain is now chosen from the library's
   own texts first (the language its head manifests agree on, the plain chain when they do not)
   and used by both paths, so the fallback is a timing difference and nothing else.

   A query whose chain is in none of the table's partitions is **said once, not refused**:
   per-document routing is what partitions are for, so a multi-language library legitimately
   holds several chains and no one of them is the right way to split a query the caller did not
   qualify.
3. ~~`said`/`composed`/`observed` range tables. *Exit:* a `said during` range over 155k units answers
   without sorting the store (asserted by an instrumented test).~~ **Done 2026-10-10.**
   `crates/smysl-text/src/index/ranges.rs`: rows in `lo` order with a max-`hi` segment tree over
   them, both builders, and the tables in the dump. `crates/smysl-text/tests/ranges.rs` (6 tests)
   and `a_said_range_over_a_reopened_library` in the timing harness.

   **The exit clause is met with room to spare, and it is asserted as a count rather than a
   time.** "Without sorting the store" is a claim about work, and a timing would not assert it —
   a fast machine passes a scan and a slow one fails an index. So the query counts the rows it
   looked at, *including the ones it rejected*, and over 155,510 rows a day's window of an
   hourly corpus reads **24 rows for 24 answers**: the descent prunes by subtree maximum, so what
   it reads is the answer and the boundary of the descent. End to end through a reopened library
   the same query is **8.4 µs**.

   **Undated is not a row, and that is A-2.4 rule 5 rather than an optimisation.**
   `Interval::UNDATED` has no bounds, so it intersects every window — a table holding undated
   rows would answer "said during the 1990s" with every unit nobody has dated. The table holds
   the subjects *known* to overlap something; a half-bounded interval ("from 1984 onwards") is
   such a subject and an unbounded one is not.

   **The incremental builder recomputes for anything that can bear on time, and that is the only
   correct answer.** Effective time is produced by constraint propagation: one dating can move any
   subject's interval and the only way to know which is to solve the network. So the local path —
   extend the table with an arriving subject's own seed — is taken only when the batch holds
   nothing but units, manifests and text objects **and** the corpus has nothing dangling, which
   the engine already reports as *unresolved* datings and *unbuilt* constraints. That bit is in
   the dump, because while it is set an arriving unit can resolve a dating and the local path is
   unsound. `observed` has a genuinely local builder throughout: nothing propagates into a field.

   **The step found a second quadratic, four orders of magnitude worse than the first.** Computing
   one axis of effective time took **64.4 s** at 155k subjects against 38 ms for the two axes that
   seed nothing, and the reason was one line: `seed` found a unit with
   `store.units().find(|(u, _)| *u == uid)` — a scan of the store per subject — where
   `store.get(uid)` was available. It is **51 ms** now, a 1,250× difference, and it changed the
   step's design space rather than just its timings: a recompute-on-dating incremental builder is
   affordable at 155k, and the three range tables can be built on open. Neither that defect nor
   step 1's quadratic `journal` was visible on any fixture in the repository, which is the second
   time in this phase that a corpus of the size the phase is sized for has been the instrument
   that found something.

   **The three range tables are in `Table::ON_OPEN`**, on the measured numbers rather than on a
   principle: at 155k units `journal` is 55 ms, `said` 99, `composed` 41, `observed` 10 — about
   205 ms together — against 1,898 ms for `postings`. Four tables within a factor of ten of each
   other belong in one class, and the one an order of magnitude past them belongs in another.

   `about` has no table, which is §3.8's choice and not an omission: an `about` time is reached
   through datings alone, so the `said` machinery with the axis changed gives the same answer, and
   a twelfth table to keep in agreement for a question nobody asked is a cost without a buyer.
4. ~~`text align` and the `align` table (needs OQ-4 settled for shipped maps). *Exit:* KJV ↔ Synodal
   Ps 14 ↔ Ps 13 alignment.~~ **Done 2026-10-10.**
   `crates/smysl-text/src/versification.rs` (the scheme and the maps a library holds),
   `crates/smysl-text/src/index/align.rs` (the table), `text align` and
   `text add --versification`; `crates/smysl-text/tests/align.rs`, 9 tests. **All twelve tables
   §3.8 names are now built**, and the `text index` arm that said which phase step would fill a
   table has no table left to say it about — the compiler found that, by making the catch-all arm
   of `Index::rows` unreachable.

   **The exit clause is met, and what is synthetic about the fixture is said rather than
   smuggled.** `Ps.14 ↔ Ps.13` aligns in both directions, and both sides resolve to text in two
   different parts of two different expressions. The *numbering* is the fact under test and it is
   a published convention — from Psalm 10 to Psalm 147 the Hebrew and Septuagint numberings differ
   by one. The *verse text* is synthetic, because the KJV and the Synodal translation are texts
   this repository does not hold (GE-T1's standing debt at §5.5) and a fixture reciting them from
   memory would be a corpus claim nobody could check. What an alignment maps is locators, so the
   content behind them is irrelevant to the claim.

   **OQ-4 is untouched and stays open**, and the step did not need it settled. A map *installed in
   a library* is a mechanism; a map *distributed with the crate* is a dataset with a provenance
   and a licence. §4.1 reserved `data/versification/` for the second and this step built the
   first.

   **A map has to be installed, and the reason is the gate rather than tidiness.** The `align`
   table is derived, and §3.8's gate is that a rebuild from the library alone reproduces it. A map
   passed on a command line and not kept would make the table reproducible only by whoever still
   had the command line — the next rebuild would produce a different table, or none, and the gate
   would be measuring a coincidence. So `text align --scheme FILE` writes it to
   `versification/<name>.tsv` under the locks, and the rebuild reads what is there. A malformed
   map is refused at install and *skipped* at rebuild, for the reason a stale sidecar is a rebuild:
   a derived table may not stop a library opening.

   **A chapter row carries its verses, one level and no further.** `Ps.14 → Ps.13` maps `Ps.14.3`
   to `Ps.13.3`, because a whole-chapter offset is what most of a versification difference *is* and
   a map stating every verse of every psalm would be thousands of rows to say one thing. An exact
   row always wins, which is what Psalm 51 needs — the Septuagint numbering counts the Hebrew
   title as verse 1, so every later verse moves — and the carry drops exactly one component, so a
   book-level row cannot renumber a Bible through two levels of guessing.

   **Both directions are in the table and the reverse is built, not read.** `align(a, b)` and
   `align(b, a)` are different lookups and a caller holding one mid should not have to know which
   way the map was written. The reverse comes from the forward map so the two cannot disagree, and
   it is **refused** when the forward map is not injective rather than dropping one of two rows
   that point at the same locator — a reverse that guessed would answer with the wrong verse.

   **`SMY-W451` is registered now, and the registry's own rule is why not earlier.** A code
   nothing in a build can raise is the thing that count is kept for; until there was a command
   that aligned two expressions, nothing could find an uncovered locator. The count is in the
   table rather than a line per locator, because a psalter has thousands and the number is what
   says whether a map covers the corpus.

   **`versification` is carried across an append.** An append is a new version of the same
   expression and an expression does not change its numbering between versions; a successor that
   dropped the declaration would unseat an aligned head with an unaligned one and the table would
   quietly empty.

### TX-P5 — ingest over texts

Precondition: **GE-T4** passed; SMYSL-2.2's spike result read.

1. ~~FC-1 (`lang`, core key 9; D-2 reserved word) and FC-3 (`span`, `manifest`); ports.
   *Exit:* old stores re-encode unchanged; `W406`/`E404` fixtures.~~ **Done 2026-10-10.**
   `UnitCore.lang` (A-1), `SourceRef.span` and `SourceRef.manifest` (A-2.1 keys 5 and 6), the
   surface spellings of all three, `SMY-E404` and `SMY-W405` raised by the library pass, the copy
   set in `provenance::stamp`, the spec's §2.2 amended in place, and all three ports.
   `crates/smysl-core/tests/passage_provenance.rs` (10 tests), two in
   `crates/smysl-text/tests/object_check.rs`, two in `crates/smysl-text/src/provenance.rs`, and
   three new cases in `fixtures/wire/uid/cases.json`.

   **The exit clause names the wrong code.** `SMY-W406` is holder and speaker uids not in `deps`,
   which §4.3.1 and the pass table both give to **TX-P12** with FC-9's field types — it has
   nothing to do with FC-3. The pair this step owes is `SMY-E404` and `SMY-W405`, the two codes
   TX-P1 step 5 allocated and deliberately left unregistered because the field they read did not
   exist. Both are raised now and both are in the registry, which took the count from 79 to 81.

   **"Old stores re-encode unchanged" is asserted twice, and the second is the one that counts.**
   A unit with no `lang` and no span writes no key 9, 5 or 6, so its bytes are the bytes it always
   had — and the byte-pinned wire fixtures, written by releases that had never heard of these
   keys, still round-trip. The first is a property of this build; the second is evidence about
   the format.

   **The quoting is load-bearing, and that needed the parser to look at the source.** A-10 item 2
   makes a bare `lang:` core key 9 and a quoted `"lang":` a payload key — and `HObject` strips
   quotes from a key, so after parsing the two are the same four characters. The distinction is
   recoverable from the key's span, which is what `HObject::key_span` is for. The writer's side is
   the mirror: `lang` is the one payload key written **quoted**, where the other five header-word
   collisions have no surface form at all. That asymmetry is the right way round — a `lang`
   payload key was ordinary before this release, and refusing it a spelling would take it away
   from every document that has one.

   **`SMY-W432` has changed what it says rather than being retired.** The lint shipped a release
   early to announce that these four characters were about to move between two hashed places; the
   move has happened, so the warning now says the language belongs in the core key rather than
   that it is going to. The code stays because stores written before this release still hold such
   units.

   **Rule 1 is enforced at three doors and unreachable through the fourth.** `SourceRef::in_part`
   sets the span and the manifest together and is the only public way to set either — the fields
   are private for that reason — the decoder refuses the pair's absence, and the surface parser
   names the rule. The decoder refusing is defensible precisely because no release before this one
   could write key 5: there is no corpus to keep readable, which is the opposite of the argument
   that keeps a malformed `published` or an unknown `source.kind`.

   **`SMY-W405` needed the resolver to grow a method.** A locator is `smysl-text`'s grammar and
   needs a reading to resolve against; `smysl-check` may not depend on that crate (§4.3.3 spends
   its one edge on the `Time` pass). So `PartResolver` gained `locator`, whose default answer is
   "I do not answer locator questions" — and a checker that gets it reports nothing, because *I
   could not look* is not a disagreement. An object store alone genuinely cannot answer: a locator
   is a name in a reading.

   **`provenance::stamp` writes five of the copy set's six fields now**, where it wrote three.
   The language is the **caller's** and not the manifest's: a manifest says what language the text
   is in, and a gist written about a Russian passage may be written in English — which is exactly
   what `SMY-W436` reports, and taking the field from the manifest would make that unreportable.
2. ~~`x.text/v1` declaration shipped as a schema record; entity units without span via
   `provenance` (`ref: "alias:<alias>"`). *Exit:* the same entity from 50 windows is one uid.~~
   **Done 2026-10-11.** `smysl_core::schemas::text` holds the declaration — the id, the revision,
   the five types, the ten relation kinds, the eight payload keys with their A-7 types and enum
   sets — plus `entity`, `rule_7_violations` and the `alias:` reference grammar.
   `schemas::text::decl()` is the record. The library pass raises `SMY-E403` for a rule 7
   violation; `fixtures/library/check/entity-{clean,provenance}.smy` are the control and the
   defect, and `tests/library_check.rs` gains two tests that a code set cannot express.

   **The exit clause is a unit test and the better form of it is the control beside it.** Fifty
   calls give one uid, which is true and weak — a function that ignored its arguments would pass
   it. `name_language_and_expression_each_separate_two_entities` is the other half: five calls
   differing in one of the three things rule 7 says the identity is a function of give five
   uids, and an absent language is a fifth answer rather than a synonym for `und`.

   **The signature is the rule.** `entity(name, alias, lang)` returns a finished `UnitCore` and
   not a builder, because rule 7's point is that nothing else is in the identity: a caller handed
   a builder could add a `body`, and a `body` is hashed. The status is `cited` and not the
   caller's for the same reason, and `cited` is the only status the rule leaves available —
   `measured` is false, and `inferred` and `derived` both require `grounds`, which rule 7 forbids,
   so `UnitCoreBuilder` would refuse them. `text:kind` is in the declaration and deliberately
   unwritten: the payload is hashed, so a kind would split Jerusalem into a place and a concept
   the first time two windows disagreed.

   **`SMY-E403` now says two things, and the amendment asked for that.** A-2.4 rule 7 names
   `SMY-E403` for a violation, where the code's other reading is a reference claiming a library
   identity it does not have. The two are different defects, so the registry entry widened —
   "a malformed library identity, or provenance the unit's type forbids" — rather than a new code
   being allocated against accepted normative text. `SMY-W432` changed what it says one step ago
   for a comparable reason. The diagnostic names *which* field broke the rule, because a producer
   told only that its entity is wrong has four to guess between.

   **Rule 7 lives in `smysl-core`, not in `provenance`.** The copy set is in `smysl-text` because
   its hazard is a producer copying a manifest's title into ten thousand uids; an entity copies
   nothing from a manifest and has no manifest to copy from, so that hazard cannot arise. What
   decided it is the checker: `smysl-check` may not depend on `smysl-text` (§4.3.3 spends its one
   edge on the `Time` pass), and a rule with one implementation in the producer and another in
   the checker is a rule with two readings — which is how `SMY-W405` ended up needing
   `PartResolver::locator` one step ago.

   **What the declaration fixed that was already broken.** `x.text/same-as` was a constant in
   `smysl-graph`, `x.text/quotes` another in `smysl-text`, `x.text/translates` a literal inside a
   test: three spellings of a vocabulary whose entire purpose (A-11) is that two implementations
   agree on the strings. Both constants are now re-exports of the declaration's. And every store
   holding one of the ten kinds warned `SMY-W013` on every check, correctly and unactionably,
   because nothing could declare them; `the_standard_text_schema_declares_every_kind_it_names`
   pins both directions — ten warnings without the record, none with it, and no `SMY-E012`,
   since registered is not kernel.

   **`payload_shape` is absent, and that is A-7 rather than an omission.** Appendix C's
   declaration carries a `fields:` map, and the structure that puts one on the wire — key 4
   gaining a shape, key 5 gaining lenses — is A-7, which G0 did not accept. So the field table is
   in code, where step 3b's converter needs it to write `text:holder` and `text:mode` from
   structure, and key 4 stays the opaque CBOR it has been since the field existed. Writing a
   structure into a hashed-adjacent record before the structure is agreed would put bytes in
   stores that a later reading would have to honour or break. `x.query/v1` is absent for the
   kindred reason: G0 accepted A-11's `x.text/v1` and left `x.query/v1` open. It arrives with the
   saved query, in SMYSL-2.5.

   **The spec gained rule 6 as well as rule 7.** Step 1's row in 2.3's landed-amendments table
   claimed rule 6 — the copy set — and `provenance::stamp` did implement it, but §2.2 never said
   it: the rule existed in code and in this RFC and in no normative document. Both are in §2.2
   now, and §5.1 registers `x.text/v1` by name.
3. ~~Windows, owned ranges, node hint, span attachment, `Absent` behaviour, payload quote
   removal. *Exit:* §5.1 ingest tests.~~ **Done 2026-10-12.** `smysl_ingest::text` behind a new
   `text` feature: `window.rs` (`Pane`, `Placed`, `TextWindow`, `windows`) and `attach.rs`
   (`Scope`, `Attachment`, `Attached`, `Opts`, `attach`), with 17 in-crate tests, the facade
   re-export `smysl::ingest_text`, and `SMY-W419`, `SMY-W441` and `SMY-W447` registered — the
   registry's first three codes about a *place* in a text, which is why none could be raised
   before there was code that chose one.

   **Five things this step had to settle that the clause does not say.**

   **The model is never asked for a span, and that is what makes attachment a step.** It is
   asked for a quote, which is checkable against the text it was shown — `SMY-E307` and
   `SMY-W308` have checked exactly that since 1.3 — and the span is computed from the quote
   here. A prompt that asked for byte offsets would be asking a model to count, and a wrong
   count is indistinguishable from a right one until someone opens the part.

   **"Owned ranges partition the text" needed the overlap to be excluded from them.** A window
   after the first opens with the last node the previous window owned, so the *nodes* of two
   windows overlap by one while the *owned* ranges do not overlap at all. `TextWindow::owned` is
   an index range and `overlap` is the one carried node, and the test that matters is the one
   asserting the owned nodes of every window, concatenated, are every node exactly once in
   order. Without that split, a claim near a boundary would be attached twice, and GE-T2's
   stability figure would be measuring double counting.

   **A span must not cross a node boundary unless the bytes do.** §4.3.4 asks for the search to
   run per contiguous same-part run; contiguity is checked here rather than assumed, because a
   window's text inserts its own separator between nodes and two nodes that are adjacent in a
   *window* need not be adjacent in a *part*. A run is therefore a maximal sequence of one
   part's nodes whose part ranges are adjacent with exactly the separator between them, which
   makes the window-offset-to-part-offset map one affine shift across the run. A quote spanning
   a gap gets no span rather than a span over bytes the part does not hold in that order.

   **The hint can only narrow, and `SMY-W419` is where even the narrowest scope cannot.** The
   four scopes are tried narrowest first and the first with a match wins, so a model naming a
   node it did not read loses nothing a model naming none would have had — the test asserts
   exactly that. Psalm 136 is in the fixture because it is the case the code is for: a quote
   that occurs twice inside the node named is ambiguous in the one scope whose job was to be
   unambiguous, the earliest occurrence is taken because an arbitrary choice would be unstable
   across reruns, and the warning is what says the choice was arbitrary.

   **The quote is removed only when a span was attached.** Draft 3 reads as though removal were
   a step; it is a *consequence*, and the condition is the point (OQ-41). A unit that got a span
   has something better than its quote and the quote is inside its uid, so it goes. A unit that
   got none keeps it, because then the quote is the only evidence a reviewer has of what was
   claimed — and the payload surgery is done on the entries' own CBOR bytes rather than through
   the HJSON value model, since every other key belongs to a schema this crate knows nothing
   about and re-encoding a float through a quantising model would move a uid for a key nobody
   asked about.

   **What the `Absent` cap cannot be.** §3.4 already says `inferred` with grounds and
   `speculative` without, because `UnitCoreBuilder` refuses `inferred` without grounds. The
   thing worth adding is the other half: a unit already at or under its ceiling is left
   **byte-identical**, uid included. A cap that rewrote a speculative unit would move a uid to
   enforce a rule that changed nothing.

   **Not in this step, and deliberately.** No `Ingestor::ingest_text` and no command: the
   prompt templates, the ledger digest and the journal are step 4, and an end-to-end
   `text_gate.rs` without them would be testing a path half of which does not exist. What is
   here is two pure functions over a reading and a batch, which is also what makes them
   testable with no egress at all. `smysl-pack` stays out of the `text` feature until the digest
   needs it.
3a. ~~**Consensus extraction** (§3.4): two passes per window by default, both staged, nothing
   deduplicated during ingest; `--single-pass` recorded in the recipe; `--temperature` above 0
   refused unless `--single-pass` is given. *Exit:* a window extracted twice at T=0 by the
   `Scripted` provider yields the `identical-span` agreement the fixture states, and the same
   window at T=0.7 is refused rather than silently producing two disjoint sets.~~
   **Done 2026-10-12.** `smysl_ingest::consensus` (`Passes`, `Agreement`, `agreement`),
   `IngestOptions::passes`, the recipe field, and six tests in
   `crates/smysl-ingest/tests/consensus.rs` against `fixtures/ingest/consensus/`, which states
   the agreement for an identical second pass and for a drifting one.

   **Four things this step had to settle that the clause does not say.**

   **"Nothing is deduplicated during ingest" is already true, and it is not what makes the
   agreement readable.** Two passes that find the same claim produce the same uid, so the staged
   batch holds one record for it — that is content addressing, and no policy can switch it off.
   So a count of staged units cannot tell "both passes said this" from "one did": the agreeing
   fixture stages 3 units and the drifting one stages 4, which on the count alone reads as the
   *richer* extraction rather than the less stable one. The agreement is therefore **recorded by
   the run** (`IngestReport::agreement`) over per-pass unit lists the ingestor keeps, rather than
   recomputed later from the batch. Draft 3's wording would have left the figure unrecoverable.

   **`identical-span` needs a span, and span attachment is step 3.** The key is tid, span, gist —
   the tid and not the whole reference, since `reference` may carry `#<locator>` and the span is
   authoritative where the two disagree (A-2.1 key 5). A unit with no span cannot establish the
   relation in either direction, so `Agreement::spanless` counts those and `rate()` returns
   `None` rather than 1.0 for a window where nothing was comparable: an empty comparison is not
   perfect stability, and GE-T2 reports this figure per language. The fixture supplies the
   window's own span through `--source`, which has existed since 1.6; per-unit spans narrow the
   key and change nothing about the arithmetic.

   **Two passes is not the default, and cannot be yet.** §3.4 scopes the doubling to
   `ingest --text`, which is step 3's command. `IngestOptions::passes` defaults to
   `Passes::Single`, because a default of two would double the cost and the call count of every
   `ingest` in this release — document ingest, which no part of the pivot was about.
   `Passes::TEXT` is the constant the text path takes, and the only thing step 3 owes this one is
   to pass it.

   **The recipe field is absent for a single pass.** `Some(Single)` and `None` hash the same,
   following `normaliser` and `estimator`, so every recipe computed before consensus existed is
   byte for byte the recipe it was — and a corpus already extracted still aggregates with one
   extracted after this step. Only `consensus` is pushed, and it has to be: it is the policy
   GE-T2's stability is measured under, so a figure computed across both policies would be a
   figure about neither.

   **One consequence found by the drifting fixture, and kept.** A second pass that gives a label
   to a *different* unit loses the label: the first binding stands and the disagreeing unit is
   staged without a name (`SMY-W054`). That is the pre-existing cross-chunk rule, and it is the
   right outcome — a name that moved between two readings of one window names nothing reliably —
   but it means a disagreeing pass loses its labels rather than its units, which a reviewer
   reading a staged file should expect.
3b. **Holder and mode written from structure** (§3.4), not from the prompt, and the holder
   paragraph is not added to the template. *Exit:* the probe set — attribution-bearing passages
   with known reported speech — shows the asserted-as-reported rate **below 10%**, which is the
   threshold T8 fired on at 11.1–24.5%. This is a new phase-exit clause, not a refinement.
   **Built 2026-10-12; the exit is owed, and the reason is a finding.**
   `smysl_ingest::text::attribution` writes `text:mode`, `text:speaker` and `text:holder` from
   the node a span landed in, and `crates/smysl-ingest/tests/probe_set.rs` measures the clause
   when a probe set is supplied (`SMYSL_PROBE_SET`) and reports it as owed when one is not.

   **The paragraph is out of the prompt and there is an assertion keeping it out.**
   `the_text_prompt_asks_for_no_provenance_and_names_no_holder` fails if the word appears in
   either text template. That is T9 as a test rather than as a decision recorded once.

   **`asserted` is written only where a holder is known, and never as a default.** The three
   outcomes are: inside a reported-speech mark → `reported` and *no holder*, because the mark
   says somebody else is speaking and not who; a node with a speaker → `asserted`, with that
   speaker as `text:speaker` and an `x.text/entity` for them as `text:holder`, staged beside the
   claim and in its `deps` (`SMY-W406`); neither → **nothing written**. A unit with no
   structural evidence about its mode is a different record from one asserted by a known holder,
   and §3.4 says `asserted` must not be a default.

   **The finding was: no reader in this build marks reported speech.** §3.4 says
   "reported-speech spans the reader marks". The two markers real sources carry are USFM's
   `\wj … \wj*` and OSIS's `<q who="…">`, and neither reader read them; chat readers mark a
   *speaker* per message, which is the holder and not the mode. So the marks were an **input**
   to windowing (`Pane::with_reported`) and `attribution::quoted_spans` — paired quotation
   marks — was the one supplier that works on any text. A modern edition that punctuates its
   quotations was handled; a 1769 Bible was not. The harness asserts exactly that pair: the same
   claim comes out `reported` with punctuation, nothing without it, and `reported` again as soon
   as a mark is supplied — which located the gap in the **reader** rather than in the converter.

   **Closed 2026-10-15, in the readers, where it was.** `usfm/1` reads `\wj … \wj*` and
   `osis/1` reads `<q who="…">`, in both the container and the milestone form, and both write
   `reading` **key 9**: a list of marks on the row whose range holds them, each a start, an end
   and the source's own `who` where it gave one. `smysl_ingest::text::marks_of` takes them off
   the rows, and `crates/smysl-ingest/tests/reported_speech.rs` asserts the whole path with no
   model in it — a claim drawn from inside `\wj` comes out `reported`, the narration beside it
   comes out with **no mode at all**, and the same verse with the marker removed comes out with
   none either, which is the control the finding needs.

   Five decisions the sentence "read `\wj`" turned out to contain.

   1. **A new row key, not a new level and not the `speaker` field.** Both markers name a *span
      inside a node*, so a row of its own would need a locator nothing addresses, and the row's
      `speaker` means the node's speaker — a verse row whose speaker was Jesus would make the
      surrounding narration `asserted` by him, which is the opposite of what the mark says.
   2. **The mark carries `who`, and nothing reads it.** OSIS has an attribution and USFM has
      none. §3.4 is explicit that a mark establishes `reported` and *no holder*, so the field is
      recorded for a later phase rather than wired to anything — the chat readers' `ids`
      decision, for the same reason: the source said it, and dropping it is irreversible.
   3. **`who` is not pseudonymised.** It is a label an editor put on a public text, not a
      platform's identifier for a living person, and keying it would destroy its only use while
      protecting nobody. A transcript whose speakers *are* people belongs in a chat reader,
      where `speaker` is keyed.
   4. ~~**The reader's marks replace the punctuation scan rather than joining it.** Per part: an
      edition that marks its reported speech has said which clauses are reported, and adding the
      scan on top would re-admit the heuristic the marks replace. A part whose reader marked
      nothing still gets the scan, because an empty list means "nothing is marked".~~
      **Reversed 2026-10-15 by the probe set, and the premise was simply false.** `\wj` does not
      mark reported speech; it marks the **words of Jesus**, which is one holder out of however
      many a passage has. Over Mark 14-15 in the World English Bible — an edition that marks *and*
      punctuates — the reader finds 30 marks and the scan finds 54: Pilate, the high priest, the
      maid in the courtyard, the centurion and the crowd are all punctuated by that edition and
      none of them is `\wj`. Ranking the suppliers therefore lost every speaker but one, and lost
      them precisely *because* the edition had marked anything at all. Nothing in either format
      asserts that every reported clause carries a marker, so the two are **joined**: both are
      positive evidence, neither claims completeness, exact duplicates are said once, and overlap
      is left alone because attribution asks whether a claim's span is contained in **any** mark.
      What the join costs is the scan's false positives in parts that also carry marks, and that
      cost was already accepted everywhere else — the scan was the sole supplier for every
      unmarked part before the readers learned either marker, and a mode is only ever written
      *because* some supplier says `reported`, never because none does.
   5. **Marks merge across a separator and not across a word.** One clause arrives in as many
      fragments as its source divides it into — an XML entity, a `\nd` inside a `\wj`, a
      quotation reopened after a line break — so two marks with only whitespace between them and
      the same `who` become one. With a *word* between them they stay two: that word is
      narration, and a mark covering it would say somebody said something they did not.

   The identity consequences are real and, on this corpus, nil: a part with no marks encodes
   exactly as it did before key 9 existed, so every expectation in `fixtures/library/readers/`
   and all five editions of GE-T1's bibles are byte-identical. Two new samples
   (`matt4-wj.usfm`, `matt5-q.osis`) pin the marked case, and the three ports name key 9 — their
   identities never depended on the name, since a structure hash is taken over the table's
   bytes and rule X keeps what a port does not know.

   ~~**The probe set is owed for GE-T13's reason.** Attribution-bearing passages with known
   reported speech are material this repository does not hold, and authoring them here would
   make them a development set — the passages would be chosen by whoever wrote the detector.~~
   **Paid 2026-10-15, and the clause is met at 0.0% over 17 probes.**
   `fixtures/attribution/probes/` is Mark 14-15 in two editions from eBible's USFM
   distributions — `eng-kjv`, the 1769 text, which marks the words of Jesus and punctuates
   nothing; and `engwebp`, the World English Bible, which marks them and punctuates every
   quotation. The same 47 verses in both, so the two arms differ in nothing but typography.
   `SMYSL_PROBE_SET=<dir>` still replaces the material with somebody else's, which is GE-T13's
   arrangement.

   The development-set objection is answered the way GE-T13 answered it — by removing the
   choice rather than defending it. **Every utterance in Mark 15 is probed, in both editions**,
   one probe each and at the verse it begins in, so no verse was picked; the chapter was picked,
   for the reason that the Passion narrative is where ten speakers take their turn — Pilate,
   Jesus, the crowd, the soldiers, the passers-by, the chief priests, a bystander, the man with
   the sponge, the centurion, and a verse of scripture quoted as if it spoke. The labels are two facts and no opinions:
   `expected` is `reported` where the verse frames the clause with a **verb of saying**
   attributing it to somebody and `none` where it does not, so the gloss `which is, being
   interpreted, The place of a skull` and the superscription that `was written over` are not
   sayings; `marking` is `wj`, `quoted`, `both` or `bare`, which anybody can check by opening
   the file.

   **What is gated and what is reported.** Only the 17 probes whose own edition marks or
   punctuates the clause are in the gated denominator, and they measure **0.0%**. The 13 whose
   edition leaves the clause to a comma and a capital letter are the typography arm: they come
   out at **100%**, and they are counted and reported rather than passed or failed, which is
   GE-T13's rule for a date pair open on the side being compared. Folding them in would make
   the figure a figure about punctuation — §3.4's supplier list is the reader's marks and the
   punctuation scan, and a 1769 Bible offers neither. Six further probes expect **no** mode and
   measure the error in the other direction: **3 of 6** got one, all of them in the edition that
   puts quotation marks round a gloss or an inscription, which is the price of the join in
   decision 4 and is reported rather than thresholded, T8's defect being the opposite direction.

   **It found two defects on its first run, both hidden by the same thing — one part.**
   `part::local_rows` shifts a row to its part's own offsets, and left the marks *inside* the
   row at the whole text's, so chapter 15's marks pointed past the end of chapter 15; every
   reader fixture in the tree until now produced exactly one part, for which the shift is zero.
   And a probe needs a **locator**, not just a quote: the quote search has four scopes and
   matches under a normaliser, so `Thou sayest it.` at 15:2 attached to Peter's `I know not,
   neither understand I what thou sayest.` at 14:68 — the previous chapter, the previous part —
   and was scored against the wrong verse, which is how the gate first reported 11.8%. A probe
   now names its verse, only the part holding that verse is searched, and an attachment landing
   anywhere else is reported rather than scored. Chapter 14 is carried and not probed for
   exactly this reason: it makes chapter 15's probes read out of a part that does not begin at
   byte zero.
4. ~~Ledger digest, `E442`, journal, language policy, recipe fields, prompt templates with
   fences. *Exit:* red-team suite; resume test.~~ **Done 2026-10-12.** `text/digest.rs`,
   `text/journal.rs`, `text/run.rs` (`Ingestor::ingest_text`, `TextIngest`, `Target`,
   `TextReport`), `json_ast::{Scope, convert_scoped}`, `repair::convert_scoped`,
   `prompt::{ingest_text_json, ingest_text_surface, DIGEST_FENCE, neutralise_fences}`, four
   recipe fields, `SMY-E442`, `SMY-W443` and `SMY-W444` registered, the `ingest --text` command
   with `--window-level`, `--digest-budget`, `--reextract` and `--single-pass`, and both exit
   clauses: `tests/text_gate.rs` (9 tests) and `tests/redteam.rs` (5 tests over
   `fixtures/library/redteam/`).

   **Six things this step had to settle that the clause does not say.**

   **Commit then journal, and that ordering *is* the resume property.** An entry is appended
   after its batch is committed, so a kill between the two costs one window — extracted again,
   producing the same uids — and the unrecoverable case, a window recorded as done that no store
   holds, cannot happen. It is why `ingest --text` commits per window and therefore requires
   `--yes`: a run that staged to a file and committed later could not journal at all.

   **The journal is a file, not the redb table §4.3.4 offers.** The correctness property is
   *append then flush*, which a length-prefixed append-only file gives directly; the access
   pattern argues nothing the other way, since a journal is read once per run and appended once
   per window. A truncated tail — what a kill leaves — stops the read and keeps everything
   before it, the rule `from_cbor_seq` already applies to a log.

   **§3.5's `l/1` is not a label in this format.** A-3's grammar requires each segment of a
   label to begin with a lowercase letter, so the digest labels are `l/n1 … l/nn`: one character
   more, against a second label grammar for digests alone.

   **The digest's fence is always rendered, even when the digest is empty.** The template is one
   per run because the *recipe* is one per run, so a window that skipped the substitution would
   send the literal `<<<SMYSL-DIGEST>>>` and an unfilled `{digest}`. An empty digest inside its
   fence says "nothing has been extracted yet", which is what the first window of every
   expression means.

   **Dropping the answer's `source` forces the tool to supply one.** `cited` requires a source
   or `UnitCoreBuilder` refuses the unit (`SMY-E032`), and a text answer's own `source` is
   dropped (`SMY-W444`) — so `ingest_text` passes the window's own reference under
   `SourcePolicy::Override`, **without a span**, and attachment adds the span where a quote
   supports one. Without a span, because §3.4 says an unsupported quote is staged with no span:
   a window-wide span would be true, useless, and would make the phase exit's coverage figure
   meaningless. There is a test for the refusal in `json_ast`, because the constraint is what
   makes the window source mandatory rather than an optimisation.

   **A refused window used to lose its warnings.** The repair loop reported only errors from a
   chunk it gave up on, so a red-team answer that cited a store uid *and* supplied a speaker
   reported the citation and not the speaker: a reader saw one reason and never learned the
   other. The last attempt's warnings are carried into the degrade path now. Found by the
   red-team suite, which asserts both codes.

   **Not in this step.** `pivot:<lang>` is refused by name (`LangPolicy::parse`) while GE-T2's
   figures are being measured under `source`; it landed **after** step 5, see §4.3.4 and the end of
   the step 5 note below. `link`, `summarise` and `same_as` are TX-P7's. The path is chosen from the **largest window** rather than the expression's
   size, or every Bible would take the surface path on the strength of its total length while
   each call sees a few verses.
5. *Exit for the phase (draft 3 §22, and changed by §1.1 items 2, 4 and 6):* resume after kill
   loses ≤ one batch; ≥ 95% of units carry a span; entity duplication < 2% across windows; the
   probe-set gate of 3b; **GE-T2 rerun** under the consensus policy (§5.5); and a **cost report**
   from the journal that replaces draft 3 §7.7's planning figures — now with a per-language retry
   rate as a separate term (§3.7), measured **after** F-2, since every non-English figure S0 has
   was taken under the bound F-2 replaced. The phase is not sized from a planning number at all.
   **Measured 2026-10-12.** `smysl_ingest::text::cost` (the report), four journal fields, the
   `ingest --text <who> --cost` command, and two harnesses:
   `crates/smysl-ingest/tests/phase_exit.rs` (6 tests) and `crates/smysl-ingest/tests/ge_t2.rs`
   (5 tests). **Three of the six clauses were met here, one was built and reported, and two were
   owed** — and the division was not about effort, it was about what a provider that says what
   the test told it to say can establish. The two that were owed were owed as *material*, and
   both were paid on 2026-10-15, so **all six are settled** and the phase owes nothing.

   | clause | figure |
   |---|---|
   | resume loses ≤ one batch | **met.** 8 windows, the journal truncated mid-append, **1** window re-sent, and the store unchanged afterwards |
   | ≥ 95% carry a span | **met, 1.000**, against a control at **0.857** |
   | entity duplication < 2% | **met, 0.000.** 3 entity records for 3 speakers across 9 windows |
   | probe-set gate (3b) | **met at 0.0%, over 17 probes whose edition marks or punctuates the clause** (2026-10-15). `usfm/1` reads `\wj … \wj*` and `osis/1` reads `<q who="…">`, both into reading key 9; `fixtures/attribution/probes/` is Mark 14-15 in two editions of scripture with every utterance of Mark 15 probed in both. The 13 probes whose edition punctuates nothing come out at 100% and are reported, not gated — GE-T13's rule for a pair open on the side compared |
   | GE-T2 rerun | **instrument met, corpus paid, figure measured.** Three runs of five languages, hosted, span coverage 1.000; the reported spread is 0.000-0.769, and the finding is that one run per language does not resolve it |
   | cost report | **built and measured.** Per language, from the journal, with the retry rate as its own term |

   **Four things this step had to settle, and three of them are defects it found.**

   **Step 4's journal recorded the *run's* running total against each window.** `input_tokens`
   was `report.usage.input_tokens` at the moment the window was journalled, so summing a corpus
   charges the first window once per window — the figure grows with position. It is the window's
   own now, by difference, and three fields joined it: `lang`, `calls` and `retries`. An entry
   written before this release has no call count, and that absence is what `Entry::priced` reads:
   such entries are counted in `Row::unpriced` and left out of every total, because a sum of two
   different measurements is a figure about neither. Rule X in the small, with a reason.

   **`Agreement::rate` reported 1.000 for a single pass.** One pass produced every key every pass
   produced, so `agreed == comparable` by construction, and the function's own documentation said
   the rate was `None` for it — which was false. The document path never hit it because step 3a
   wrapped the field in an `Option` and only filled it for two passes; the text path reports the
   agreement as a plain field, which is how it surfaced. `rate` now requires two passes, and the
   test that said "one pass agrees with itself" asserts the `None` beside the count.

   **The path was chosen in bytes, so the same corpus in Russian took a different pipeline.**
   `path::choose` compares an expected output size against 4 KiB; a text ingest passed its widest
   window's **byte** length, and two bytes per Cyrillic character put a Russian expression over
   the bound where the same passage in English was under it — so the Russian run's answers were
   parsed by the *surface* parser while the English run's went through json-ast, which §3.9.2 does
   not treat as equivalent. GE-T2 reports per language, and the figure would have been about two
   pipelines. `path::choose_tokens` asks the same question in the unit a window is already
   budgeted in, with the threshold expressed as `SMALL_OUTPUT_THRESHOLD / 4` so English does not
   move. This is F-2's lesson in a second place: F-2 replaced a byte-based *window* bound, and
   this was the byte-based *path* bound beside it. `ge_t2.rs` asserts both languages choose one
   path, and `TextReport::path` reports which — a run says, because it was not configured.

   **What makes the coverage clause a measurement is a quote the window holds and a part does
   not.** A quote the window does *not* hold never reaches the figure: the repair loop refuses it
   (`SMY-E307`) and the unit is degraded to opaque prose, which removes the quote, so attachment
   never sees it. The case the clause is about is the one step 3's contiguity rule exists for — a
   quote spanning two nodes that the *window* joins and the *part* does not, which is staged with
   no span and a capped status (`SMY-W441`). That is the control, at 0.857, and it is also why
   `TextReport::spanless` and `Agreement::spanless` are two counters: the first counts units that
   attributed a quote and got no span, the second counts units that cannot establish
   `identical-span` however they came to have no span.

   **What was owed, and both halves are paid.** The probe set of 3b and GE-T2's corpus were
   *material*, not code: the harnesses take them (`SMYSL_PROBE_SET`, `SMYSL_GE_T2=<dir>` with
   `SMYSL_INGEST_LIVE=1`) and reported the clause as owed without them, which is GE-T13's
   precedent. **Both were paid 2026-10-15** the way GE-T13's chronology was: by taking public
   material and removing the choice of what to measure, rather than by authoring it and defending
   the choice. GE-T2's corpus is `fixtures/stability/corpus/`, and the choice is removed twice
   over — the passages are the S0 spike's, fixed in SMYSL-2.2 before this code existed, and the
   rule that takes three of its eleven is mechanical: every reference all five tier-1 languages
   carry. That leaves 26 verses of narrative, 26 of law and 7 of poetry in each language, 59 lines
   in every file, so the only thing varying between five reported figures is the language. The
   other eight references are English and Russian only, and including them would have given two
   languages five genres the other three lack — a figure reported *per language* that was partly
   about genre. The phase's precondition **GE-T4** was ruled 2026-10-14, so what is left is the
   cost report, which nothing blocks: F-2 shipped in 1.10.0 and the per-language figures come out
   of the journal rather than out of a coefficient.

   **The rerun ran, and it found that it cannot be run once.** Three runs of all five languages
   against hosted `deepseek-chat` at temperature 0. Span coverage was **1.000** in every language
   of every run. The `identical-span` rate was de 0.024/0.412/0.078, en 0.452/0.769/0.132,
   es 0.000/0.333/0.047, fr 0.047/0.024/0.391, ru 0.265/0.481/0.030 — re-running one language
   moves the figure by 0.333 to 0.637, while changing the language inside one run moves it by
   0.361 to 0.745. The two are the same size, and the ranking does not survive a rerun: the best
   language is en, then en, then fr, and the worst is es, then fr, then ru. **So the per-language
   figure §5.5 asks for is not resolved by one run per language**, and five numbers from a single
   run would have been five numbers of sampling noise. The figures are therefore recorded as a
   spread over three runs, which is the honest shape of what was measured; the clause is reported
   and not thresholded, so nothing is blocked by it either way.

   Temperature is not the explanation, and that was checked rather than assumed: `IngestOptions`
   defaults it to 0, `openai_compat::body` puts `"temperature": 0.0` on the wire, and
   `Passes::admits` refuses anything above it. The result is instead consistent with what S0
   already had — `J_class` 1.000 local against 0.327 hosted, and MS-8's 145 bit-identical runs,
   which were the **local** arm. A hosted model is not reproducible at temperature 0, and
   `admits` cannot see it, because it checks the temperature a caller configured and not whether
   the provider honours it. **OQ-74** records the gap.

   **The figure could not be read as it stood**, which cost one addition to the instrument.
   `identical-span` includes the gist (§3.5), so a pass that found the same bytes and wrote a
   different sentence about them counts as a disagreement — and over a live corpus that is most
   of the figure. French run B is 0.024 by `identical-span` and **0.680** with the gist left out:
   the two passes found the same 17 of 25 places and re-worded nearly all of them. So
   `Agreement` gained `comparable_places`, `agreed_places` and `place_rate`, reported beside the
   figure and never instead of it — GE-T2's reported figure stays `rate`. A scripted provider
   cannot tell the two apart, because it returns one string by construction and its passes agree
   on the wording for free, which is why four passing instrument tests never revealed it.

   **`pivot:<lang>`, the last thing the phase owed in code. Landed 2026-10-13.** Left until
   after the exit deliberately, and the reason is in the exit itself: GE-T2's figures are
   measured under `source`, and a policy that added a second call per window while they were
   being taken would have been a change to the thing being measured. With them taken, the policy
   is what §4.3.4 specifies — extraction under `source`, then one translated unit per unit with
   the source in `deps` and an `x.text/translates` edge — and it is `crates/smysl-ingest/
   tests/text_gate.rs` that pins the clause (§5.2) with two controls beside it. Three decisions
   the one-line plan did not contain, each argued at §4.3.4: `body` and `detail` do not come
   along, because one `lang` covers every text field; entities are not translated, because A-2.4
   rule 7 would make a translation a second entity rather than a second language; and the pass
   has no repair loop, because the only check available to a translation is whether a line came
   back. It also found the `x.text/v1` declaration missing from every library a text ingest had
   ever written to. **TX-P5 owes no material as of 2026-10-15**: GE-T4 was ruled 2026-10-14 at
   0.0000 in all five languages, 3b's probe set is met at 0.0%, and GE-T2's corpus is committed
   and its rerun measured.

### TX-P7 — proposition layer and linking

(TX-P6, `sq`, is SMYSL-2.5. `link` and `summarise` accept alias/mid scopes until it lands.)

1. ~~`proposition::classes`~~ — **moved to TX-P2 step 5** (§0.1), because TX-P5's consensus
   output needs it first.
2. Engines `identical-span`, `lexical` (with adjacency, which S0 showed is load-bearing rather
   than a filter), `anchored`; `same-as propose/classes`. *Exit:* anthology test — a paragraph
   reprinted under two expressions forms one class by `identical-span`.

   **Built 2026-10-14, and the exit is met.** `smysl_text::proposition` holds the three engines
   behind one `Engine` trait; `smysl_ingest::same_as::propose` is the only thing that turns a
   proposal into a record; `same-as propose|classes` is the command. The anthology test is
   `crates/smysl-ingest/tests/same_as.rs` and it is deliberately not the easy version of the
   clause: the library holds **two** paragraphs, each reprinted, so the exit asserts *two
   classes of two* rather than one class of four — an engine that proposed every pair would
   pass the sentence as written and fail here. Beside them are a unit carrying one pair's gist
   over the other paragraph's bytes (an engine keyed on the gist alone proposes it) and a
   second extraction under one of its own manifests (an engine that ignored the manifest
   proposes that). Neither may be proposed, and neither is.

   Five decisions the one-line plan did not contain, each written where it is made.

   A proposal is a `Proposal { a, b, why }` and not §4.2's `(Uid, Uid)`, because a pair cannot
   say why. An engine that proposed 4,000 edges over five Bibles is either finding the
   alignment or finding that every verse looks like every other verse, and the difference is
   one sentence per edge — which `same-as propose` prints and GE-T5's sheet needs. The pair is
   held in uid order, so a symmetric claim has one spelling and `attested:n` cannot count one
   engine as two.

   `Anchored` takes the **alignment table**, not §4.2's `{ lib, scheme }`. A library plus a
   scheme name means rebuilding the alignment inside `propose` — every locator table in the
   corpus — and makes the engine's answer depend on a map installed since the index was built.
   §3.8's gate is that the table is reproducible from the library alone, `Index` already holds
   it, and an engine handed it is pure in the arithmetic sense. The scheme is per manifest pair
   and is in the table's own rows, so naming one would have let a caller ask for an alignment
   that was not the one computed.

   **Adjacency is a field, defaulting to on.** §3.6 calls it load-bearing from S0's
   measurement (precision 0.789 → 0.835 same-language, 0.427 → 0.712 cross-lingual), so the
   engine's default is the measured configuration — and `--no-adjacent` exists because the
   0.789 figure has to stay reproducible. An engine that could only be run in its good
   configuration could never be shown to be better than its bad one. The rule is "same stem,
   final component equal or one apart", and it deliberately does **not** cross a chapter seam:
   `Gen.3.24` and `Gen.4.1` are consecutive verses of scripture and are not adjacent by this
   rule, because the rule cannot know Genesis 3 has 24 verses without reading the structure,
   and a rule that guessed would be wrong at the end of every chapter in the other direction.

   An edge the store already holds is **not staged again** and is counted: a rid is derived
   from the kind and the endpoints, so a rerun produces byte-identical records, and staging
   them would be a lie about what the run found. A *withdrawn* edge is the exception and is
   proposed again — withdrawal is a person judging that two units are not one proposition, and
   an engine disagreeing with a person is the thing that should be visible. `Store::is_withdrawn`
   keeps it out of the classes either way.

   And `propose` **stages**, which is why §4.4's `Purity::Mixed` is recorded as `Pure` instead:
   every engine in this build is a computation over records, and `semantic` — the one that
   would make the row mixed — is SMYSL-2.5's and is the value clap refuses. `--dry-run` was
   added for the determinism matrix, because §4.4 registers `same-as propose` there and an
   operation run sixteen times to prove it is a function of its input must not write a sidecar
   each time. `propose` additionally needs the staging layer and `classes` does not, so a build
   with `text` and no `ingest` answers the pure half and says so about the other.

   Two defects found by building it. `--yes` over a **surface** store is refused, because an
   attestation has no surface form and committing would keep the edge and lose the agent that
   proposed it — which is the one record making `attested:n` answerable. That refusal is 1.9's
   and was never exercised by a command whose batch is *all* attestations; it is now asserted.
   And the engines needed one reading of A-2.2's `t3:…#locator` grammar rather than three, so
   `proposition::place` is the one place a unit's source is taken apart: three engines parsing
   a reference three ways would have made their proposals incomparable, which is exactly what
   `attested:n` reads.

   What TX-P7 still owes: **step 3** (`link`, `summarise`) and **step 4** (GE-T5).
3. Linker and summary tree. *Exit:* GE-T11 (recall ≥ 0.6 on 50 planted far contradictions at the
   default candidate budget); summary nodes capped by rule M (asserted).

   **Built 2026-10-14, and both clauses are met.** GE-T11 reports **0.6800, 34 of 50**, at the
   default budget; the rule M clause is asserted in `crates/smysl-ingest/tests/link.rs`. §3.5
   above carries the five decisions the two sentences did not contain. What follows is what the
   measurement cost and what it is worth.

   **The figure's two limits, stated before the figure.** The fifty pairs in
   `fixtures/library/link/planted.tsv` were written by hand, by the same hand that wrote the
   generator, so their difficulty is a difficulty that hand chose. And the distractor pool is
   Ruth and Jude — 110 verses, 210 units with the plants in — which is small enough that a pair
   has little to be lost among. Both are one environment variable away from being fixed:
   `SMYSL_GE_T11_CORPUS` takes a directory of `*.smy` and `SMYSL_GE_T11_PLANTED` a set somebody
   else wrote. The harness prints the corpus size beside the recall for exactly this reason: a
   recall without a denominator is not a measurement.

   **The defaults were chosen from the measurement, after it, and the measurement is also the
   gate.** That is worth saying plainly rather than leaving in a constant. What was *not* done is
   taking the smallest numbers that cleared the bar: `per_unit` is 16, giving 0.68, where 12
   gives 0.62 — a default sitting exactly on a threshold is a default that fails on the next
   corpus. `neighbours` is 32 because 24 → 32 moved recall from 0.52 to 0.58 and 32 → 48 moved
   it not at all, and because retrieval depth is arithmetic while turns are money. The curve
   through `per_unit` is shallow — 0.48, 0.62, 0.68, 0.78 at 8, 12, 16, 24 — and that shallowness
   is the honest shape of a lexical generator hunting contradictions.

   **Two things the measurement found that no amount of reading would have.** A BM25 score is not
   comparable between two queries, so sorting every pair in a corpus by raw score spends the
   budget on long units' neighbourhoods and never reaches a short unit's best match; reciprocal
   rank fusion replaced it, and a pair found in both directions sums both, which is how
   mutuality earns its place without being a special case. And the harness's own stand-in for an
   extractor had to be calibrated on usage rather than on punctuation: taking every capitalised
   token except the sentence-initial one loses the *subject*, which in this corpus is the name,
   and the entity source then found nothing on precisely the pairs it exists for. Counting a
   token as a name when it is capitalised more often than not moved recall from 0.34 to 0.48.

   **What the committed run does not measure.** Of the four sources, `theme` contributes nothing
   (no extractor has named a theme in this corpus) and `anchored` nothing (one edition, no
   alignment). Three of the sixteen misses are *not generated at all* at unlimited depth, and
   several more sit at BM25 ranks beyond 80 — pairs that share a subject and almost no
   vocabulary, which no lexical method reaches. Those are TX-P10's S1 embeddings, and the gate
   passing without them says the other three sources carry the corpus, not that the fourth is
   unnecessary.
4. *Exit for the phase:* **GE-T5** within-language arm under §5.5's restated threshold — the
   engine's agreement with the pooled gold is not distinguishable from the coders' agreement with
   each other. Draft 1's "precision ≥ 0.9 at recall 0.7, else class measures ship as exploration
   only" is replaced on both halves. The bar was unattainable against a gold of α 0.600 (§0.1),
   and the *else* branch is no longer a branch: class measures ship as exploration only either
   way, by §1.1 item 3, until GE-T9 supplies a human α. What this exit decides is whether the
   engines are at the ceiling, not whether the measures are publishable.

   **Built 2026-10-14. The instrument is met and the sheet is owed**, which is GE-T2's shape in
   TX-P5 step 5 and for the same reason: what is missing is material. The harness is
   `crates/smysl-text/tests/ge_t5.rs`, pure, on every `make test`.

   **What there was to measure, and what had to be made.** The `lexical` engine exists for two
   expressions of one language, and the committed slice holds exactly one edition per language —
   so there was no within-language pair in this repository to measure at all. A second English
   expression of Ruth was written by hand:
   `fixtures/proposition/ge-t5/paraphrase.tsv`, forty rows restating one King James verse each
   in modern English, and ten more placed at a locator that already carries one of the forty,
   close to it in vocabulary and saying something else. That gold is **of record by
   construction** — a row's `relation` is what its author wrote it to be — and the hand is its
   limit, as it is GE-T11's. `SMYSL_GE_T5_PARAPHRASE` replaces it.

   **Draft 1's statistic, for the record and against no bar.** Over 135 units (Ruth's 85 verses
   and the 50 planted), with the engine's neighbour count swept: **precision 0.6724 at recall
   0.9750** at `k = 1`, falling to 0.3421 at the default `k = 5` while recall only reaches
   0.9750 → 1.0000. The shape is the opposite of what draft 1 anticipated. Recall is nearly free
   — a restatement sits at the same locator and keeps the proper names, so adjacency and term
   overlap both point at it — and **precision is the whole difficulty**. From `k = 2` upward the
   engine proposes **all ten** planted near-misses, every one of them: a sentence at the same
   locator, with the same names, saying the opposite thing is a sentence this engine cannot tell
   from a translation. That is not a defect to fix in `lexical` — a term overlap has no access to
   the difference — and it is the clearest argument in this repository for TX-P10's S1
   embeddings and for `attested:2` in the meantime (§3.6).

   **The restated clause, end to end.** α (nominal, missing-tolerant, coincidence-matrix form)
   over a reliability matrix; the pooled gold by majority with ties dropped and counted; the
   engine scored as one more coder against the pool; and a **paired bootstrap** on
   `δ = α(coders) − α(gold, engine)`, 2,000 resamples of the pairs, both α recomputed on each
   resample. The clause is met when δ's 95% interval reaches zero — one-sided, because an engine
   that agrees with the gold *better* than the coders agree with each other has joined the pool,
   which is all §5.5 asks.
   The pool is built by **rule** and not by the engine's own answers: each planted row against
   the verse at its own locator, and against the verse next to it. A hundred pairs, base rate
   0.40 — stated because α is chance-corrected, and a pool four-fifths positive would let an
   engine that proposes everything look agreeable on accuracy while α called it what it is. The
   first version of this harness had exactly that pool, and the engine scored **α −0.1124** on
   it.

   **What is owed is three coders.** `fixtures/proposition/ge-t5/sheet.tsv` is three *scripted*
   columns: the pool's constructed truth with disagreement written in at fixed intervals, chosen
   so that α over the three columns comes out at **0.600** — what S0's three model coders
   actually reached (SMYSL-2.2 §3.7, T1). Over it the harness reports α(coders) 0.6000,
   α(gold, engine) 0.5594, δ 95% **[−0.1278, 0.2189]** — an interval that straddles zero, which
   is what `Met` looks like. It does **not** report `Met`: the sheet is scripted, so α(coders) is
   a fact about that file, and [`Decision::Owed`] is what a measurement over it is worth.
   `SMYSL_GE_T5_SHEET=<file>` decides the clause with no code change, and S0's own sheet would
   do it.

   **α is validated against arithmetic a reader can follow**, not against a number recalled from
   a paper. SMYSL-2.6 §5.2 asks its `assess::agreement::alpha` to reproduce the published worked
   example in Krippendorff (2011) and says the value is "to be transcribed; unverified here" —
   so the test here derives one instead and spells out every term of it, and the three places α
   has no answer (one coder, one value, nothing pairable) each return `None` rather than 1.0.

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
| `SMY-E450` | appending to a pseudonymised expression without its pseudonym key | `text append` (**raisable since TX-P2 step 3**; `--pseudonymise` landed in step 2 and the code was deliberately left unregistered until there was an append to refuse) |
| `SMY-W451` | an alignment scheme does not cover a locator; the pair is left unaligned and counted | `text align`, `anchored` engine |
| `SMY-E452` | a record 15 or 18 was offered to a log; text lives in the object store (OQ-39) | `append`, `check` |
| `SMY-W453` | the pivot pass returned no translation for a unit; it is staged in the source language only | `ingest --text --lang-policy pivot:L` |

`454`–`459` are unallocated. SMYSL-2.3 may adopt `E446` as a C-Library obligation.

**Registered as of TX-P5 step 3:** the eight of TX-P1 step 5 — `E401`, `E402`, `E403`, `W418`,
`E440`, `E445`, `E446` and `E452`, in a ninth diagnostic group (`Group::Library`) and in the
manual's Appendix B; step 2 registered the first five, step 4 added `E452` and step 5 `E403` and
`W418` — then `E410` (TX-P3 step 1), `W412` and `W413` (step 2), `W411` and `W449` (step 3),
`W451` (TX-P4 step 4), `E404` and `W405` (TX-P5 step 1), and `W419`, `W441` and `W447` in TX-P5
step 3, where windowing a reading and attaching a span to a quote exist. Twenty, and the registry
stands at 84. Eight, and not the
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
| OQ-4 | open, and TX-P4 step 4 shipped without it: a map **installed in a library** is a mechanism and needed no answer, where a map **distributed with the crate** is a dataset with a provenance and a licence and still needs one. §4.1's `data/versification/` is empty |
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
| OQ-70 | **Found in TX-P2 step 2.** A part entry's `structure` is A-5's hash of the whole segment table, so it moves when a row's *metadata* moves — a speaker, a timestamp, an id — and not only when the segmentation does. `SMY-E401` then reports `which: "structure"` for a change that is not structural. Should A-5 narrow the hash to the tree (level, range, locator) and leave the rest to the rdid, which already covers it? It is an amendment to the normative document and it would move every structure hash ever computed, so it is a question for SMYSL-2.3 rather than for a reader's commit. The cost of leaving it: pseudonymising a corpus re-identifies every part *entry* — no part, no object, no tid. §3.2 |
| OQ-74 | **Found in TX-P5 step 5, measuring GE-T2's corpus.** `Passes::admits` refuses two passes above temperature 0, on MS-8's measurement that two runs at T>0 cannot agree by `identical-span` except by accident. It checks the temperature a **caller configured**, and nothing checks whether the provider honours it. Measured against hosted `deepseek-chat` with `"temperature": 0.0` on the wire: three runs of five languages gave `identical-span` rates spread 0.333 to 0.637 *within* one language, which is the size of the spread *between* languages — the second pass bought much of what MS-8 says a T>0 run buys, at a configured T of 0. S0 had the same split from the other side (`J_class` 1.000 local, 0.327 hosted; MS-8's 145 bit-identical runs were the local arm). The consensus policy's premise is therefore a property of the *provider*, not of the configuration, and this build cannot tell the two apart. What would close it is a determinism probe — the same request twice at T=0 before the run, recorded beside the figure — which is cheap and is not a policy change; the alternative is to say in §3.4 that `identical-span` is defined for providers that are reproducible at T=0 and is a lower bound elsewhere. §3.4, §5.5 |
