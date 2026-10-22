# RFC SMYSL-2.7 — Import and export

**Status:** draft 1, for discussion. Implementation RFC (non-normative); normative rules are in SMYSL-2.3.
**Author:** Vladimir Ulogov
**Date:** 2026-10-02
**Part of:** RFC set SMYSL-2 — see SMYSL-2.0 (index and roadmap).
**For:** crate `1.9.0-dev` (`d25ec9e`), format `smysl/1.0`, kernel `smysl.kernel/0.1`.
**Derived from:** RFC SMYSL-2 draft 3, §16 (all), §3.1 rule 8, findings F-16, F-17, F-18, §17 (`lib` rows), §18 (`smysl-lib`), §19.5, §22 (TX-P11, the `--target-revision` part of TX-P12), §23 (GE-T15), §25 (OQ-25..OQ-28).
**Phases:** TX-P11; the `--target-revision` slice of TX-P12.
**Depends on:** SMYSL-2.1 (TX-P0: `Store::record_set_digest`, the `bundle` fix), SMYSL-2.3 (record-set digest A-4, records 14–19 A-5, the alias grammar A-5), SMYSL-2.4 (library, object store, manifests, substrate index, redaction, span checks, `identical-span` same-as), SMYSL-2.5 (`sq` scopes, lens engine).

---

## 0. Summary

`smysl lib import` takes smysl files written elsewhere (surface, CBOR logs, bundles, packs, and
collections written by `lib export`) into the library as a staged, checked, idempotent merge
with an import manifest per file. `smysl lib export` cuts an `sq` scope into views by a layout and
writes one canonical file per view, with one inclusion rule per record type and an index file.
`smysl lib verify` checks the index and runs the round trip. The contract is design rule 8: a
**record-set digest** (normative in SMYSL-2.3, code in SMYSL-2.1 as `Store::record_set_digest`)
must be equal on both sides of the trip (P1), an import must be exportable again (P2), and a
re-export must be byte-identical (P3).

Everything lands in a new crate, `smysl-lib`. It reuses what the tree already has: the store's
byte-identity of records (`record_hashes`), `Store::append` and its `AppendReport`,
`traverse::closure` with `EdgeSet`, `Store::rebuttals_of` (rule R), the check pipeline, merge's
reported (not recorded) contention detection, and the surface writer's per-record predicates.

Verifying the existing code for this RFC found six things the design has to absorb. Each is
stated in §2 with its evidence:

1. `v/export/<key>`, the view id draft 3 proposes for generated views, **is not a legal
   `ViewId`** (two `ident`s around exactly one `/`). Surface parsing turns it into `v/doc`
   silently; CBOR decoding rejects it. This RFC uses `export/<slug>` (§3.4, OQ-61).
2. `Store::emit` drops **attestations on relations** as well as the records F-16 lists. Its
   attestation arm tests the attested uid against the kept *unit* set, and an edge attestation
   names a rid.
3. Units named only as a relation `note`, a commitment `note`, a withdrawal `reason` or a
   resolution `note`, and a contention's `positions`, are not reached by
   `traverse::closure`. A closure built on it alone exports dangling references (§3.6).
4. Record 7 (pack info) lists no member units: it holds `dropped`, `degraded` and `thread`.
   "When all its units are included" has to be defined over the uids it does name (§3.6).
5. `merge --format surface` counts commitments as omitted while writing every `@commit`. F13
   reports "4 record(s) have no surface form" over output with four `@commit` lines. `SMY-W428`
   must not reuse that filter (§4.3).
6. The global `--format` flag accepts only `surface` and `cbor`, and `-o` is a global *path*.
   `lib export --format pack -o DIR` needs both changed (§4.4, OQ-62).

---

## 1. Scope

**In:**
- `smysl-lib`: import (decode, check, classify, stage, merge, import manifest, `imported` and
  `waiting` tables, originals, span re-verification and pure joins, redaction at the door), and
  export (scope → layout → files, inclusion closure, `full`/`ref`, CBOR/surface/pack,
  canonical order, index file, `--as-of`, `--target-revision`), and `verify` (P1, P2, P3).
- CLI: `smysl lib import|status|accept|discard|export|verify`.
- Changes to existing crates that this needs: `smysl-core` (diagnostic subject, surface
  predicate), `smysl-graph` (accessors), `src/main.rs` (command table, global flags), xtask
  purity list, fuzz targets.
- Tests: the GE-T15 matrix as an automated suite, seeded property tests and fuzz targets (D-11),
  cross-implementation round trips with `python/`, `nodejs/` and `go/`.

**Out (owned elsewhere):**
- The record-set digest's definition (SMYSL-2.3) and its code and the `converged_with` change
  (SMYSL-2.1, TX-P0; OQ-27 is answered there).
- The `bundle` fix (F-16) in `Store::emit` (SMYSL-2.1). This RFC states the extra gaps found in
  `emit` (§2.3) so SMYSL-2.1 can close them. Export does **not** call `emit`.
- Records 14–19, tid/mid/did/rdid, the object store, redaction, span checks (SMYSL-2.3,
  SMYSL-2.4). Lens semantics and the lens engine (SMYSL-2.3 FC-9, SMYSL-2.5). `sq` (SMYSL-2.5).
- Shards, `StoreRead` and a disk-backed unit store (SMYSL-2.8, TX-P13). Until then a shard is an
  in-memory `Store` over one log file (§4.1).

---

## 2. What exists today (verified against `d25ec9e`)

All paths are relative to the repository root. "Verified" means read in the source and, where
marked, run against the release build `target/release/smysl` (1.9.0, `cli` only).

### 2.1 Store: open, append, identity, digest

| item | where | behaviour that matters here |
|---|---|---|
| `StoreOptions { verify_hashes, force_reindex }`, `StoreOptions::strict()` | `crates/smysl-graph/src/store/mod.rs` | `verify_hashes` compares recomputed uids against the **sidecar index** (`SMY-E070`) and runs `report_dangling` (`SMY-E060`). It only acts when a current sidecar exists (`open_with`: `if let Some(ix) = sidecar.as_ref().filter(\|_\| current)`). A foreign file has no sidecar, so `strict()` checks nothing for it. This confirms draft 3 §16.2. |
| `Store::open`, `Store::open_with` | same | Reads the whole log, `from_cbor_seq`, tolerates a **truncated tail** (`SMY-W110`, `OpenReport.trailing_bytes`). Keeps duplicates as on disk. |
| `Store::from_records` | same | Builds in memory and **dedupes by record hash**. Repeats only union edge attestations. |
| `Store::append(&[Record]) -> Result<AppendReport, Error>` | same | Dedupes against `record_hashes` and within the batch. `AppendReport { added, duplicates, bytes_written }`. Writes through to the log when the store has a path. Derived state is rebuilt **once per call**: the doc comment measures 1336 µs/record for 1-record calls and 5 µs at 1000. Import must append in one call per file. |
| `record_hash(&Record)` (private), `record_hashes: BTreeSet<[u8;32]>` (private) | same | BLAKE3 of `to_cbor(r)`, for every record type including `Unknown`. This is the hash the digest uses (SMYSL-2.3). No public accessor at `d25ec9e`. SMYSL-2.1 adds `record_set_digest`; this RFC also needs `contains_record` (§4.3). |
| `pending_attestations: Vec<Attestation>` (private) | same | Attestations whose unit or edge has not arrived, retried in `rebuild_adjacency`. This is the precedent for **wait**. There is no public accessor. |
| `Store::state_hash`, `converged_with` | same | Hashes units (with attestations, salience, labels), relations (with attestations), threads, views, withdrawals, resolutions, contentions. It **omits commitments, schema declarations, pack infos and unknown records** (F-17, confirmed by reading). |
| `Store::iter()` | same | Records in log order. Export iterates **records, not derived maps**: `relations` is keyed by `(kind, from, to)` and keeps the first of two records that differ in weight, while the log keeps both. |
| `Store::rebuttals_of(&Uid)` | same | Live rebuttals (rule R): incoming `rebuts`, edge not withdrawn, rebutter present and not `unfounded` under the strict policy. |
| `attach` / relation encoding | same; `crates/smysl-core/src/cbor/envelope.rs` `relation_bytes` | A relation's attestations are **not** in its encoding (`relation_bytes` writes kind, from, to, weight, note, extra). They travel as separate record-2 attestations whose `uid` is the **rid**. |

### 2.2 Merge, diff, compact

- `smysl_graph::merge::merge(&mut Store, &Store, MergeOptions)`
  (`crates/smysl-graph/src/merge/mod.rs`) is `append` plus contention **detection**. Detected
  contentions are "**reported, not recorded**" (comment in `merge`): detection is not monotone,
  and recording would break associativity. So importing into an empty library adds no records
  beyond the input. P1 depends on this.
- `cmd_merge` (`src/main.rs`) fixes `opts.now = Hlc::new(0, 0, tool:smysl-merge)` so merge is
  bit-reproducible, and with `--staged` commits `smysl::stage::read` output.
- `cmd_diff` / `smysl_graph::lineage::diff` compare stores by **unit membership only**
  (`StoreDiff { only_in_a, only_in_b, common }`). It cannot see a lost commitment. `lib verify`
  uses the record-set digest instead, and a record-level difference listing (§4.2 `verify`).
- `smysl_graph::compact` (`crates/smysl-graph/src/compact.rs`), doc comment: "**Compaction does
  not survive a merge.** A peer that still holds what was dropped will bring it back on the next
  union." The same holds for `lib import`: importing an uncompacted peer's export restores what
  local compaction dropped. That is correct under rule U and is stated in the import report
  (§3.2), not prevented.

### 2.3 `bundle` and `emit` (F-16, plus two further gaps)

`Store::bundle` and `bundle_with` take `traverse::closure(g, roots, &EdgeSet::all())` and call
the private `emit(view, &keep)`, which filters `self.records`.
- Its last arm is `_ => false`. Pack info, schema declarations, **commitments** (no arm) and
  unknown records are dropped.
- **Reproduced** with the release build (`merge F.smy -o F.cbor; bundle F.cbor`; records counted
  with `python/smysl/records.py`):

| fixture | store | bundle |
|---|---|---|
| F13-canon | unit 3, label_binding 3, commitment 4, view 1 | unit 2, label_binding 2, view 1 |
| F12-extension-types | schema_decl 1, unit 3, label_binding 3, view 1 | unit 1, label_binding 1, view 1 |
| F11-extension-kinds | schema_decl 1, unit 4, label_binding 4, view 1 | unit 1, label_binding 1, view 1 |

- **Further gap A (verified by reading, not reproduced: attestations have no surface syntax):**
  `Record::Attestation(a) => keep.contains(&a.uid)`. `keep` holds unit uids. An attestation on
  an edge names the rid, so every edge attestation is dropped, even when the edge is kept.
- **Further gap B (verified by reading):** `Adjacency::build`
  (`crates/smysl-graph/src/adjacency.rs`) adds a relation's `note` as a node with **no edge**.
  `Commit.note`, `Withdrawal.reason`, `Resolution.note` and `Contention.positions`
  (`crates/smysl-core/src/types/{lifecycle,annex}.rs`) are not in the adjacency at all. So
  `closure` never reaches them.

SMYSL-2.1 owns the `bundle` fix. Export implements its own selector (`export::select`, §3.6)
and never calls `emit`.

### 2.4 Record types and decoding

- `Record` (`crates/smysl-core/src/types/record.rs`): `Unit`, `Attestation`, `Relation`,
  `Thread`, `View`, `Contention`, `PackInfo`, `SchemaDecl`, `LabelBinding`, `Withdrawal`,
  `Resolution`, `Commit`, `Unknown { code, payload }`. Code 9 (`CHECKPOINT`) is reserved and has
  no variant. It decodes as `Unknown`. Records 14–19 decode as `Unknown` until SMYSL-2.4 adds
  them.
- `from_cbor` is strict: the `Unknown` payload is parsed strictly too (comment in `from_cbor`),
  and `fuzz/fuzz_targets/cbor.rs` asserts that anything accepted re-encodes to itself. So the
  hash of `to_cbor(decoded)` equals the hash of the input bytes, and the digest of a decoded
  file is the digest of its bytes as read.
- `from_cbor_seq` stops at a truncated tail **without error** and returns the consumed length.
  `lib import` treats a short read as an error (`SMY-E493`, §8). Partial input must not import
  silently (D-10).
- Unit uids are never stored. `canonical_uid` hashes the core
  (`crates/smysl-core/src/hash.rs`), so a foreign file cannot carry a wrong uid (draft 3 §16.2).
- `Attestation.sig` is "COSE_Sign1, reserved. Not implemented in 0.1 (N9)"
  (`crates/smysl-core/src/types/provenance.rs`). **Signature verification is not available**:
  the only integrity checks on import are canonical decoding, recomputed uids and the check
  pipeline. `Op` already has `Transformed` and `Attested`, which export and `--attest` need.
- `ViewId`, `Label`, etc. are `label_shaped!` (`crates/smysl-core/src/ids.rs`): exactly
  `ident/ident`, with `ident = [a-z][a-z0-9_-]*`. **Run:** a document with `id: v/export/kjv`
  checks clean, and `merge --format surface` writes it back as `id: v/doc`. `parse.rs` line 713
  falls back to `v/doc` when `ViewId::new` fails, which is the F-13 class of silent loss. CBOR
  `dec_view` returns a malformed-envelope error for the same id.
- `LangTag::new` accepts `und` and `mul`.

### 2.5 Surface writer

`crates/smysl-core/src/surface/write.rs`:
- `write_surface` writes units, relations, threads, `@schema` (when
  `schema_decl_has_surface_form`), `@withdraw`/`@resolve` (when
  `withdrawal_has_surface_form` / `resolution_has_surface_form`) and `@commit` (always).
- It writes no attestations, contentions, pack infos or unknown records, and only the view
  passed in as `@doc`.
- `schema_decl_has_surface_form(d) = d.payload_shape.is_none() && d.extra.is_empty()`.
  Withdrawals and resolutions additionally need `ts.agent == agent` (F-18 confirmed).
- **Defect, run:** `cmd_merge`'s omitted-record filter ends in `_ => true`, which includes
  `Record::Commit`. On F13 it prints "4 record(s) have no surface form and were omitted" while
  the output holds four `@commit` lines.

### 2.6 Check pipeline

- `smysl_check::check(&Store, CheckOptions) -> Report` (`crates/smysl-check/src/lib.rs`) runs
  passes integrity, shape, closure, granularity, epistemics, trust, extension and commitment.
  It never short-circuits.
- It is **store-level**. A `Diagnostic` names `Subject::Span`, `Subject::Unit(uid)` or
  `Subject::Store` (`crates/smysl-core/src/diag.rs`; the enum is `#[non_exhaustive]`).
- Relation findings are attributed to a unit (`integrity.rs` uses `Subject::Unit(rel.to)`).
- `report_dangling` names the **missing** uid, not the referrer. So per-record classification
  cannot be read off the report as it is (§3.3).
- `ConformanceClass::forbids` defines which codes are structural, epistemic or shape errors.
  Classification reuses it.

### 2.7 Staging today (rule S)

`crates/smysl-ingest/src/stage.rs`:
- One batch at `.smysl/staged.smy` (surface) plus `.smysl/staged.cbor` (a sidecar for
  attestations). `read` re-attaches attestations only to units unchanged in the reviewed text.
- `prepare_*` **weakens** over-claiming units (rule M at ingest, `SMY-W036`), which changes
  their uids.
- Both properties are right for model output and wrong for import: surface loses records, and
  weakening alters them. `lib import` keeps its own CBOR-only staging (§3.3) and never calls
  `stage::prepare`.

### 2.8 Command line

- `COMMANDS` in `src/main.rs` is a flat table of 26 `Cmd { name, about, purity, phase }` rows.
  `tests/dispatch.rs` runs "the twenty-six".
- No command has clap subcommands. `thread` uses flags.
- `smysl import` exists: "Tabular readings to measured units, without a model" (`cmd_import` →
  `smysl::from_csv`, writes `op: Imported` at rung `computed`). Hence `lib import` (draft 3
  §16.1).
- The facade already exports `ImportOptions` and `Imported` (CSV) at the crate root
  (`src/lib.rs`), so `smysl-lib` types stay under `smysl::lib::` and are not glob re-exported.
- Global args (`cli()`): `--format` with `value_parser(["surface", "cbor"])`, and `-o/--output`
  "Output path; defaults to stdout".
- Exit codes (`crates/smysl-core/src/error.rs`): `CheckErrors` = 3, `Contentions` = 5,
  `HashVerification` = 9, `Staged` = 10, `StagedWithCorrections` = 11.

### 2.9 Fixtures, fuzz, other implementations

- `fixtures/corpus`: F1–F9, F11, F12, F13 as `.smy` + `.expected`. F8 is two files (F8a, F8b).
  **There is no F10 in `corpus`.** `fixtures/wire` holds `F10-lifecycle.cbor` with F1, F4, F5,
  F9, F12-reserved-pack and F13-commitment as CBOR. F6 is "supposed to fail" (`SMY-E030`).
- `fuzz/`: cargo-fuzz targets `surface`, `cbor`, `merge_algebra`, `pack_constraints`,
  `pipeline`, `pack_exact`, `readers`, `redaction_merge`, `edtf`, `time_engine`, `sq_parse`,
  and a shared generator `smysl_fuzz::generate(&mut Choices, max)` (`fuzz/src/lib.rs`). The
  list is the Makefile's `FUZZ_TARGETS`, which is what `make fuzz` runs; five of the eleven
  arrived after this inventory was written and were added to it on 2026-10-15. There is **no
  `proptest`** in the workspace. Property tests use seeded
  xorshift generators (`crates/smysl-graph/tests/merge_algebra.rs`, `smysl-pack`,
  `smysl-thread`, `smysl-core` tests). D-11 is met here in the same two forms.
- `python/` (`smysl/records.py`: `decode_store`, `Record{code, body, raw}`, `encode_store`),
  `nodejs/` (`decodeStore`/`encodeStore`), `go/` (`DecodeStore`/`EncodeStore`). All decode
  **any** record code generically, keep unknown ones, re-encode byte-identically, and derive
  unit uids, rids and contention ids. Together they are 3,960 lines. None computes a record-set
  digest or reads an export index.
- Purity gate: `xtask/src/purity.rs` `PURE_CRATES` (core, graph, check, pack, thread, render,
  retrieve), `FORBIDDEN_DEPS` (includes `clap`, `serde_json`, `tokio`), `FORBIDDEN_SYMBOLS`,
  and rule A: nothing under `src/` except `lib.rs` names a sibling crate.

### 2.10 Measured cost at 10⁵ units

Measured on the release build, 2 cores. The input is a synthetic `@prose` store with 100,000
units, each grounded on one earlier unit: 200,001 records, 20.5 MB CBOR.

| operation | wall | peak RSS |
|---|---:|---:|
| `merge store.cbor -o out.cbor` (decode, `from_records`, detection, write) | 8.0 s | 672 MB |
| `bundle store.cbor` (decode, `from_records`, closure of one root) | 4.0 s | 312 MB |

The merged log is byte-identical to its input (`cmp`). Loading costs about 1.5 KB per record.
10⁶ records in memory (GE-T15's upper size) is about 1.5 GB to load and about 3.4 GB through
merge. That is the memory risk in §7.

---

## 3. Design

Normative format content (the digest, records 14–19 including the manifest keys an import uses,
the alias grammar) is in SMYSL-2.3. **The inclusion table and P1–P3 are tool-level and are defined
here** (SMYSL-2.3 §0, "not in this document, by design"). This section states how `smysl-lib`
realises them and what the verification
above forces.

### 3.1 Inputs and identity of an import

`lib import` accepts:
- surface `.smy` (detected as `looks_like_surface` does in `src/main.rs`)
- CBOR logs, `bundle` output and packs (all CBOR sequences)
- directories (non-recursive by default, files in byte order of name)
- an `export-index.cbor` (the whole collection, files verified against the index first:
  `SMY-E494` on mismatch)

Each input file is one **import unit**. It gets one import manifest (record 14, keys per draft
3 §16.3 and SMYSL-2.3). The manifest body is a function of the file bytes and the options only:
- alias `import:<name>`. `<name>` comes from `--name`, else the file stem slugged (§3.4). For an
  index it is `<collection>/<file stem>`. The alias grammar is SMYSL-2.3 A-5's: `import:` is reserved there for manifest aliases and `export/` for view ids. A whole alias is at most 128 bytes. If `import:<collection>/<stem>` would exceed it, the collection part is slugged and truncated like any key, with the hash suffix (§3.4).
- licence (`--licence`, default `unknown`), origin (`--origin <url>`, else kind `file` with the
  **basename**, never the absolute path, so the same file imported on two machines has one mid)
- `identifiers.original` = BLAKE3 of the input bytes; `identifiers.records` = record-set digest
  of the decoded records

Re-importing the same file under the same options yields the same mid. `append` reports it as a
duplicate, and the report says "already imported".

### 3.2 Import pipeline

```
bytes ─decode─▶ records ─redaction filter─▶ ─combine─▶ check ─classify─▶ accept | stage | wait | quarantine
                                                                          │
                       manifest, `imported` rows, original (o3) ◀─────────┘ ─join─▶ staged batch `join:<name>`
```

1. **Decode.** Surface: `parse_surface`. CBOR: `from_cbor_seq` must consume the whole file,
   else `SMY-E493` and nothing from that file is used. Unsupported format version: rejected as
   today (`SMY-E003`). Unknown records are kept (rule X). Duplicates within the file collapse by
   record hash.
2. **Redaction at the door.** Records 15 and 18 whose tid is redacted in the library, or by a
   record 19 in the same input, are dropped and counted (`SMY-W414`, rule Z of SMYSL-2.3). Needs
   SMYSL-2.4. Before TX-P2 the filter is a no-op.
3. **Combine.** `combined = shard.clone(); combined.append(&incoming)` in memory, with no path,
   in one call. Before TX-P13 the target shard is held in memory anyway.
4. **Check.** `smysl_check::check(&combined, CheckOptions::default())`, then span checks for
   units whose `source.manifest` names a library manifest (SMYSL-2.4: `SMY-E404`, `SMY-W405`,
   quote re-verification with the normaliser the recipe names).
5. **Classify** (§3.3).
6. **Commit or stage.** With `--accept`: `merge(&mut shard, &Store::from_records(accepted),
   opts)` with the fixed clock `Hlc::new(0, 0, tool:smysl-lib)`. Detected contentions are
   reported, not recorded (§2.2). Without it: the accepted records go to the staged batch
   `lib/staged/<name>.cbor` (§3.3) and the command exits `Staged` (10).
7. **Provenance.** Write the manifest, add `imported` rows for every **retained** record
   (accepted, staged, waiting, quarantined), and with `--keep-original` put the bytes at
   `objects/o3/<blake3-hex>`.
   - The manifest is written even when content is only staged: it records that the file was
     read, not that its content was accepted.
   - `--attest` adds one `Op::Attested` attestation per accepted unit and relation, by the
     importing agent, at commit time.
8. **Join** (§3.5).
9. **Promote waiting.** Every record in the `waiting` table whose missing ids are now held is
   re-classified (steps 3–6) under its original import's name and state.

### 3.3 Classification, staging, waiting, quarantine

The check report names units, not records (§2.6). Classification therefore works in three
layers.

**Attribution.** `names(r)` is the set of ids a record refers to:

| record | names |
|---|---|
| unit | `deps ∪ grounds`, plus tid/mid in `source` (SMYSL-2.4) |
| attestation | `uid` (a unit uid or a rid), `parents` are not required to be held |
| relation | `from`, `to`, `note` |
| thread | every `step.unit` |
| view | `roots` |
| contention | `over`, `positions` |
| label binding | `uid` |
| withdrawal | `relation` (rid), `reason` |
| resolution | target (rid or contention id), `note` |
| commitment | `unit`, `note` |
| 14, 15, 17, 18, 19 | per SMYSL-2.3 (manifest parents/supersedes, tid, dating target) |
| pack info, schema declaration, unknown | nothing (never wait) |

**Quarantine.** A diagnostic of error severity whose subject is a unit introduced by this
import, or (after §4.3's change) a record introduced by it, quarantines:
- that unit record
- every incoming record that names it **directly**: its attestations, bindings, commitments,
  edges and contentions

Units that rest on a quarantined unit are **not** quarantined. They wait for it: they name a unit
the library does not hold, which is exactly the wait condition. A reviewer who releases the
quarantined unit releases them too.

Error-severity diagnostics with `Subject::Store` that the shard alone does not produce
quarantine the **whole file**. That is conservative until passes attribute to records. Warnings
never quarantine. `SMY-W423` is reported per quarantined record, with the originating code in
its message.

**Waiting.** Let `held` = shard units and rids ∪ (incoming units and rids not quarantined) ∪
library tids/mids. Then repeat until nothing changes: a non-quarantined incoming record waits if
any id in `names(r)` is not in `held`, and a waiting unit or relation leaves `held`. The fixpoint
terminates because `held` only shrinks. Waiting records are kept with their names in the
`waiting` table (§4.1). Count: `SMY-W496`.

**Accept/stage.** Everything else. The staged batch is a CBOR file holding the exact records:
- never surface, since surface would lose attestations, contentions and unknown records
- no rule M weakening, since that changes uids

`lib accept <name>` merges it (exit 0, or 5 under `--fail-on-contention`). `lib discard <name>`
deletes it. Both are additions beyond draft 3 §17, which gives rule S's confirmation for import
no command.

**OQ-25 (staging defaults), proposed answer.** Keep staging the default for every source,
including the library's own verified exports. Without signatures (N9 unimplemented, §2.4), "our
own export" cannot be told apart from a file that claims to be one. `--accept` stays explicit
and is one flag.

### 3.4 Generated view ids and file names

`v/export/<layout key>` is not a `ViewId` (§2.4). Generated views are `export/<slug>`:
- `slug(key)`: lowercase ASCII. Map every byte outside `[a-z0-9_-]` to `-` and collapse runs.
  If the result is empty or does not start with `[a-z]`, prefix `k`. Truncate to 48 bytes.
- If the mapping was not the identity on `key`, or two keys in one export map to one slug, append
  `-` and the first 10 lowercase base32 characters of BLAKE3(`key`). This is reported as
  `SMY-W497` (informative, count).
- Keys by layout: `single` → `all`. `by-expression` → expression alias. `by-collection` →
  collection alias. `by-view` → the view id with `/` mapped to `-`. `by-time` → `t` + the
  window start in `YYYYMMDDTHHMMSSZ` (or the EDTF lower bound) with `--window`. `by-query` → the
  group value's canonical text.
- File name: `<slug>.cbor` (surface `.smy`, pack `.pack.cbor`), plus `unattributed.cbor` and
  `export-index.cbor`. `slug` never yields `unattributed` or `export-index`: those two keys get
  the hash suffix.
- The generated view record: `id` = `export/<slug>`; `roots` = the units selected for that file;
  `threads` = included threads whose steps are all in the file; `requires` = the schema ids
  declared by included declarations, if any; `granularity` = default; `intent` = `"export"`;
  `lang` = the language of every selected unit if they agree (core key 9, SMYSL-2.3), else
  `mul`, else `und` when none has one. Every field is a function of the selection, which is what
  P3 needs.

### 3.5 Joining imported units

Per draft 3 §16.4, with these implementation decisions:
- Span re-verification runs in the check step (§3.2 step 4), so a failing span quarantines.
  Requires SMYSL-2.4 (TX-P1, TX-P5).
- Pure joins: `identical-span` same-as (TX-P7) and lexical entity same-as within a language.
  They write new relations and their tool attestations. **These are not records of the input**,
  so they go into a separate staged batch named `join:<name>`, with `imported` rows pointing at a
  `join` marker instead of the manifest. P1 excludes them, as it excludes manifests (§3.9).
  `--no-join` skips the step, and `lib verify` always uses it.
- Disagreement: merge's detections, reported in the import report. Nothing recorded (§2.2).

### 3.6 Export selection: the inclusion closure

**Snapshot.** Before TX-P13 the export source is one in-memory `Store` built with `from_records`
over the shards in scope. That dedupes duplicate log records. `--with-staged` and
`--with-quarantine` add those batches to the snapshot. Without them, staged and quarantined
records are never in it.

**Selection.** `Sel` = the units the `sq` scope returns (SMYSL-2.5), partitioned by the layout
key into `Sel_k`. A unit with no key under the layout (for example no expression under
`by-expression`) goes to key `unkeyed`. It is never dropped.

**Unit closure** `U = close(Sel_k)`, a fixpoint:

```
U := Sel_k
loop
  U  := closure(adjacency, U, EdgeSet::support() ∪ --edges)      # rule L always; --edges default all
  U  := U ∪ ⋃_{u∈U} store.rebuttals_of(u)                         # rule R: live rebuttals
  A  := annotation uids of records selected over U:               # gap B, §2.3
          relation.note, commit.note, withdrawal.reason, resolution.note, contention.positions
  U' := U ∪ { a ∈ A : store.contains_uid(a) }
  if U' = U: break
  U  := U'
```

- `closure` is `smysl_graph::traverse::closure` over dense ids, which returns a sorted,
  walk-order-independent result.
- The loop terminates because `U` grows inside a finite set.
- **Retracted units are never dropped** (unlike `bundle_with`'s default). Export is exact, and
  the `retracts` edge needs both ends.

**Record selection** (draft 3 §16.6, one rule per type; "in `U`" means a unit uid in `U`; `Rel`
is the set of rids of selected relations):

| code | rule as implemented |
|---:|---|
| 1 | unit record with `canonical_uid ∈ U` |
| 2 | `a.uid ∈ U ∪ Rel` (fixes gap A) |
| 3 | `from ∈ U ∧ to ∈ U`, every record, withdrawn or not, each weight variant |
| 4 | every thread record (all register versions) with all steps in `U`, else left out (`SMY-W490`) |
| 5 | the generated view; with `--with-views`, corpus views with `roots ⊆ U` **and id outside `export/`** (§3.9) |
| 6 | `over ∈ U` (positions are in `U` by the fixpoint) |
| 7 | with `--with-packs`: every uid in `dropped`, `degraded` and, if `thread` is set, that thread's included steps, is in `U`. Pack info names no member list (§0 item 4). Else left out (`SMY-W490`). |
| 8 | every declaration (all versions, forks included) of every schema id `s` such that some decl of `s` lists a selected unit's `core.schema` in `types`, a selected relation's kind in `relations`, or `s` is in an included view's `requires` |
| 9 | never (decodes as `Unknown { code: 9 }`; dropped, not sent to `unattributed`) |
| 10 | `uid ∈ U` |
| 11 | `relation ∈ Rel` (or its dating is included, SMYSL-2.4) |
| 12 | target is a rid in `Rel`, or a contention id of an included contention |
| 13 | `unit ∈ U` (or dating included) |
| 14 | every mid an included unit names or grounds on; import manifests with `--with-provenance` (the mids the `imported` table lists for included records) |
| 15, 18 | `--with-text`, the manifest's carry and licence allow it, tid not redacted |
| 17 | target included |
| 19 | always, for every tid an included unit references |
| other | `Unknown` → `unattributed.cbor` (`SMY-W425`), or dropped with `--unknown drop` (still counted) |

Records the source store holds that name uids it does not hold (dangling at source, `SMY-E060`
there) are exported as they are and counted (`SMY-W495`). The recipient will hold them waiting.

The annotation pull-in, the pack-info rule, and the exclusion of `export/` views from rule 5 are
completions of draft 3's table. This RFC owns that table (tool-level) and adopts them (OQ-61, resolved here).

### 3.7 Closure modes and canonical writing

**`full`.** File `k` holds `R_k`, everything selected for key `k`.

**`ref`.** Every record is written once:
- `owner(r)` is the file that selects `r`'s unit as a root, if exactly one does. Otherwise it is
  the lowest file path in byte order among the files whose `R_k` contains `r`.
- `written(k) = {r ∈ R_k : owner(r) = k} ∪ {view_k}`
- `requires(k)` = the sorted, deduplicated set of `owner(r)` over `r ∈ R_k` with
  `owner(r) ≠ k`. It is stored in the index, never in the view (draft 3 §16.7).
- Cycles among `requires` are allowed: a collection is imported whole.

**Duplication factor**, reported in the index for both modes and measured by GE-T15:
`Σ_k |R_k| / |⋃_k R_k|`.

**Order** (rule D). The file's view first. Then records sorted by `(type_code, BLAKE3(to_cbor
r))` ascending, with `Unknown` sorted by its `code`. The bytes are the concatenated `to_cbor`.
Nothing depends on log order, HashMap order or the walk.

**Digests.**
- `file.blake3` = BLAKE3 of the file bytes.
- `file.rsd` = `record_set_digest(written(k))`.
- `index.rsd` = `record_set_digest(⋃_k written(k) ∪ unattributed)`. That is `rsd(X)` in P1.
- One function serves all three: `smysl_graph::record_set_digest(impl Iterator<Item = &Record>)`
  from SMYSL-2.1. `smysl-lib` does not re-implement it.

### 3.8 Formats

- **`cbor`**: the canonical form. P1–P3 are stated for it.
- **`surface`**: `write_surface(Some(view), records, ctx)`, with `ctx` built from the included
  label bindings.
  - The omitted count uses the new `surface::has_surface_form(&Record, &WriteContext)` (§4.3).
    It is `SMY-W428` with a count per record type. After writing, `parse_surface` of the output
    is compared with the CBOR selection minus the omitted records. A difference is `SMY-E424`.
  - GE-T15's "report exactly the records F-18 cannot spell" is this check.
- **`pack`**: `smysl_pack` over each file's selection, with `--budget`. Labelled lossy in the
  index (`format = pack`, `derived = lossy`), outside P1/P3.

### 3.9 Round-trip comparison sets

Let `X` be the export's records as above (generated views included, `unattributed` included
unless `--unknown drop`). Let `I(F)` be `lib import --accept --no-join` of the whole collection
into an empty library.

- **Import side record set** = every record the import **retained**: merged, staged, waiting and
  quarantined. That excludes:
  - the manifests **this** import wrote (by mid, from `ImportReport.manifests`; manifests carried
    *in* `X` under `--with-provenance` stay in)
  - join output (§3.5)
  - `--attest` attestations

  Counting quarantined and waiting records makes P1 a statement about transport. Whether P1
  should instead require them all accepted is OQ-60. The harness reports both.
- **P1**: `rsd(retained(I(F))) == index.rsd`, for every layout and both closure modes.
- **P2** (an arbitrary file `f`): import `f` with `--accept`. Then export with scope `from
  import:<f>`, layout `single`, `full`, all `--with-*`. Then check that
  `S_f ⊆ records(E)`, where `S_f` = accepted records of `f` minus checkpoints, minus 15/18
  whose carry or licence forbids export, and minus text redacted since the import. Equality is
  required when `f` was written by `lib export`, **except** the `export/` view of `f` when the
  re-export's layout key differs (OQ-61).
- **P3**: re-export from `I(F)` with the index's layout, closure, format, flags and agent. The
  scope `from import:<collection>` resolves, for a collection written by `lib export`, to **the
  union of the roots of its `export/` views**, not to every unit that arrived. Otherwise closure
  units would become selected and change the partition and the roots. Compare every file
  byte-for-byte, and the index's `files` and `rsd` fields. The index's `scope` text differs by
  construction and is not compared.

### 3.10 `--as-of`

Export runs over a **known-time subset** of the snapshot, built before selection:

1. Records with their own HLC (attestations, datings, commitments, withdrawals, resolutions,
   redactions, threads): keep if `ts.physical ≤ as_of.upper`.
2. Units: known time = min `ts` of their attestations. A unit with none is excluded and counted
   (`SMY-W415`).
3. Relations: earliest attestation on the rid, else `max(known(from), known(to))`.
4. Unit closure is re-checked. A kept unit whose `deps`/`grounds` name a unit the full snapshot
   holds but the subset does not is excluded too, to a fixpoint, and counted (`SMY-W498`). A file
   that rests on what was not yet known would not be closed.
5. Contentions, manifests, part texts and readings, views, bindings and pack infos have no clock.
   They enter only through the inclusion rules on the subset.
6. Schema declarations: per schema id, the highest version that an included record needs (its
   type or kind is listed there, or its recipe names it, SMYSL-2.4). Versions above are cut, and
   all versions at or below are kept.

The subset is `Store::from_records(kept)`, and the normal pipeline runs on it. Imported records
keep their authors' clocks (OQ-28: no change in this phase; an import-event record would be new
format and belongs in SMYSL-2.3 if adopted).

### 3.11 `--target-revision ID@N` (TX-P12)

Pure, nothing written to the library.

1. Resolve the lens path from each included unit's revision to `N` with SMYSL-2.5's engine. Any
   step without an inverse, or a `retire` on the path, refuses the export (`SMY-E422`).
2. `order = traverse::topo(&adjacency, &EdgeSet::support())`, which lists dependencies before
   dependents (doc comment in `traverse.rs`). If `order.cyclic` intersects `U`, refuse
   (`SMY-E492`).
3. For each `u` in order, restricted to `U`:
   - `core' = lens⁻¹(core)` with `deps`/`grounds` mapped through `map`
   - if `core' == core`, then `map[u] = u`, and the original attestations are kept
   - otherwise `map[u] = canonical_uid(core')`, and a new attestation is added:
     `Op::Transformed`, agent `--agent` (default `tool:smysl-lib`),
     `parents = {u}`, `ts` = the export clock (OQ-63)
4. Relations with a mapped end become new relation records with new rids. Each gets an
   attestation the same way, with `parents = {old rid}`. Label bindings, thread steps and the
   view's roots are re-pointed.
5. Commitments, datings, withdrawals, resolutions and contentions that name a mapped uid or rid
   are dropped and counted (`SMY-W491`).
6. Index: `derived = target-revision ID@N`. No P1/P3.

---

## 4. Implementation plan

### 4.1 Crate and modules

New crate `crates/smysl-lib` (workspace member through `crates/*`, `version.workspace`).

```
crates/smysl-lib/
  Cargo.toml          # deps: smysl-core, smysl-graph, smysl-check, smysl-text, smysl-query, blake3
  src/lib.rs          # re-exports; LibError; TOOL = concat!("smysl-lib/", env!("CARGO_PKG_VERSION"))
  src/access.rs       # LibraryRead / LibraryWrite traits (the seam to SMYSL-2.4's library type)
  src/tables.rs       # `imported`, `waiting`, import batches; CBOR fallback until the substrate index
  src/import/mod.rs   # import(), status(), accept(), discard(); ImportOptions, ImportReport
  src/import/decode.rs    # sniff, decode, E493, index verification (E494)
  src/import/classify.rs  # names(), quarantine, waiting fixpoint, accept set
  src/import/stage.rs     # lib/staged/<name>.cbor, lib/quarantine/<name>/…
  src/import/manifest.rs  # record 14 body for an import; mid via smysl-text
  src/import/join.rs      # span re-verification hooks, identical-span/lexical same-as, join:<name>
  src/export/mod.rs   # export() → Collection; ExportOptions
  src/export/select.rs    # §3.6 inclusion table, one fn per record type
  src/export/layout.rs    # keys, slug(), partition
  src/export/closure.rs   # unit fixpoint; full/ref ownership; duplication factor
  src/export/write.rs     # canonical order, cbor/surface/pack encoders
  src/export/index.rs     # ExportIndex encode/decode (export-index.cbor) + HJSON rendering
  src/export/as_of.rs     # §3.10
  src/export/target_revision.rs  # §3.11 (TX-P12)
  src/verify.rs       # file checks, P1, P2, P3, record-level difference listing
  tests/…             # §5
```

**Library-side storage** (paths relative to the library root of SMYSL-2.4; unverified until that
RFC fixes the layout):

| path | contents |
|---|---|
| `objects/o3/<blake3-hex>` | original input bytes (`--keep-original`) |
| `lib/staged/<name>.cbor` | exact records of a staged import, canonical order |
| `lib/staged/join:<name>.cbor` | join output of that import |
| `lib/quarantine/<name>/<file-slug>.cbor` | quarantined records, exact |
| `lib/quarantine/<name>/<file-slug>.report` | their diagnostics, one per line, `Display` form |
| `lib/waiting/<name>.cbor` | waiting records (bytes); the `waiting` table indexes them |
| shard logs | per SMYSL-2.4 §3.8 until SMYSL-2.8; opened with `Store::open` |

**Tables.** They live in the substrate index (SMYSL-2.4, `substrate-redb`), keyed as below. A
CBOR-file fallback (`lib/tables/*.cbor`) serves builds without it.

| table | key | value | notes |
|---|---|---|---|
| `imported` | record hash `[u8;32]` | sorted set of `(mid, state)`; state ∈ accepted, staged, waiting, quarantined, discarded | operational, not derived (draft 3 §16.3); rebuildable only from originals |
| `import_batches` | import name | mids, state, counts, tool version, report time | `lib status` |
| `waiting` | missing id (tagged: `0x01` uid, `0x02` rid, `0x0F` tid, `0x0E` mid) | sorted set of record hashes | **shared** with SMYSL-2.4's early-attestation and early-dating waits; one table, one promote pass |
| `waiting_by_record` | record hash | `(import name, missing ids)` | reverse index for promotion |

### 4.2 Public API (sketch)

```rust
// smysl-lib/src/access.rs — implemented by SMYSL-2.4's library type.
pub trait LibraryRead {
    fn snapshot(&self, shards: &ShardSel) -> Result<smysl_graph::Store, LibError>;
    fn manifest(&self, mid: &Mid) -> Option<Manifest>;               // smysl-text
    fn is_redacted(&self, tid: &Tid) -> bool;
    fn table(&self) -> &dyn Tables;
}
pub trait LibraryWrite: LibraryRead {
    fn shard_mut(&mut self, alias: &ShardAlias, create: bool) -> Result<&mut smysl_graph::Store, LibError>;
    fn put_object(&mut self, class: ObjClass, bytes: &[u8]) -> Result<[u8; 32], LibError>; // o3
    fn table_mut(&mut self) -> &mut dyn Tables;
    fn root(&self) -> &std::path::Path;
}

// smysl-lib/src/import/mod.rs
pub enum Input { File(PathBuf), Dir(PathBuf), Index(PathBuf), Bytes { name: String, bytes: Vec<u8> } }

#[non_exhaustive]
pub struct ImportOptions {
    pub shard: Option<ShardAlias>, pub accept: bool, pub attest: Option<AgentId>,
    pub keep_original: bool, pub licence: String, pub origin: Option<String>,
    pub name: Option<String>, pub join: bool,
}

#[non_exhaustive]
pub struct FileOutcome {
    pub path: String, pub mid: Mid, pub original: [u8; 32], pub rsd: [u8; 32],
    pub added: usize, pub duplicates: usize,                  // from AppendReport
    pub accepted: usize, pub staged: usize, pub waiting: usize, pub quarantined: usize,
    pub dropped_redacted: usize, pub already_imported: bool,
}

#[non_exhaustive]
pub struct ImportReport {
    pub files: Vec<FileOutcome>, pub manifests: Vec<Mid>,
    pub contentions: Vec<smysl_core::Contention>,             // detected, not recorded
    pub report: smysl_core::Report,                           // W423, W414, W496, E493, E494
}

pub fn import(lib: &mut impl LibraryWrite, inputs: &[Input], opts: &ImportOptions)
    -> Result<ImportReport, LibError>;
pub fn status(lib: &impl LibraryRead, name: Option<&str>) -> Result<Status, LibError>;
pub fn accept(lib: &mut impl LibraryWrite, name: &str, fail_on_contention: bool)
    -> Result<ImportReport, LibError>;
pub fn discard(lib: &mut impl LibraryWrite, name: &str) -> Result<usize, LibError>;

// smysl-lib/src/export/mod.rs
pub enum Layout { Single, ByExpression, ByCollection, ByView, ByTime { window: Window }, ByQuery(smysl_query::Query) }
pub enum Closure { Full, Ref }
pub enum Format { Cbor, Surface, Pack { budget: u64 } }
pub enum UnknownRecords { Include, Drop }

#[derive(Default)] #[non_exhaustive]
pub struct With { pub text: bool, pub views: bool, pub packs: bool, pub provenance: bool,
                  pub quarantine: bool, pub staged: bool }

#[non_exhaustive]
pub struct ExportOptions {
    pub scope: smysl_query::Scope, pub layout: Layout, pub closure: Closure, pub format: Format,
    pub with: With, pub edges: Option<smysl_graph::EdgeSet>, pub unknown: UnknownRecords,
    pub as_of: Option<smysl_text::Edtf>, pub target_revision: Option<(SchemaId, u32)>,
    pub agent: AgentId,
}

#[non_exhaustive]
pub struct ExportedFile { pub path: String, pub bytes: Vec<u8>, pub records: usize,
                          pub rsd: [u8; 32], pub view: Option<ViewId>, pub requires: Vec<String> }

#[non_exhaustive]
pub struct Collection { pub files: Vec<ExportedFile>, pub index: ExportIndex, pub report: Report }

/// Pure: no I/O, no clock, no environment. Same snapshot and options → same bytes (rule D).
pub fn export(lib: &impl LibraryRead, opts: &ExportOptions) -> Result<Collection, LibError>;
/// Same, over a store the caller already holds (tests, `verify`, the fuzz target).
pub fn export_store(store: &smysl_graph::Store, sel: &Selection, opts: &ExportOptions)
    -> Result<Collection, LibError>;
pub fn write_collection(c: &Collection, dir: &Path) -> std::io::Result<()>; // writes index last

// smysl-lib/src/verify.rs
#[non_exhaustive]
pub struct VerifyReport { pub files: Vec<FileCheck>, pub p1: Property, pub p3: Property,
                          pub p2: Vec<(String, Property)> }
#[non_exhaustive]
pub enum Property { Holds, Fails { missing: Vec<[u8; 32]>, extra: Vec<[u8; 32]>, first_diff: Option<(String, u64)> }, NotApplicable(&'static str) }
pub fn verify(index: &Path, opts: &VerifyOptions) -> Result<VerifyReport, LibError>;
```

`Selection` is the `Sel` partition of §3.6. `Window`, `Edtf`, `Mid`, `Tid`, `Manifest` and
`Scope` come from SMYSL-2.4/2.5 and are unverified names.

**`ExportIndex`** (`export-index.cbor`). Deterministic CBOR via `smysl_core::cbor::writer::
MapBuilder` (sorted integer keys, shortest form). It is not a smysl record, and it has no type
envelope.

| key | field | type |
|---:|---|---|
| 0 | `version` | text `"smysl/export-index/1"` |
| 1 | `files` | array, sorted by path, of maps: 0 path (text), 1 blake3 (bytes 32), 2 records (uint), 3 rsd (bytes 32), 4 view id (text, absent for `unattributed`), 5 requires (array of text, `ref` only), 6 role (0 data, 1 unattributed) |
| 2 | `scope` | text: the `sq` source as given |
| 3 | `sq_version` | text |
| 4 | `schemas` | sorted array of `[schema id, version]` included |
| 5 | `lens_chain` | bytes 32, when lenses are in scope (SMYSL-2.5) |
| 6–8 | `layout`, `closure`, `format` | text |
| 9 | `as_of` | text EDTF, optional |
| 10 | `tool` | text, `TOOL` |
| 11 | `rsd` | bytes 32: digest of the whole selection |
| 12 | `flags` | sorted array of text (`with-text`, `with-views`, …, `unknown-drop`, `edges=…`) |
| 13 | `derived` | text, optional (`target-revision x.text/v1@1`, `lossy`) |
| 14 | `complete` | bool; false after `SMY-E424` (draft 3 §16.8) |
| 15 | `counts` | map code → uint for W415, W425, W428, W490, W491, W495, W497, W498 |
| 16 | `duplication` | `[Σ|R_k|, |⋃R_k|]` as two uints |

The HJSON rendering (`--json` on `lib verify`/`lib export`, hand-written, no `serde_json` in the
pure core) is informative only.

### 4.3 Changes to existing crates

| file | change | why |
|---|---|---|
| `crates/smysl-core/src/diag.rs` | `Subject::Record([u8; 32])` (the enum is `#[non_exhaustive]`, so this is additive) | lets passes, and import, name a record that is not a unit (§3.3) |
| `crates/smysl-check/src/passes/integrity.rs` and others | attribute relation, attestation, commitment and binding findings with `Subject::Record` where they now use `Subject::Unit(rel.to)` or `Subject::Store` | precise quarantine; gradual, each pass on its own |
| `crates/smysl-core/src/surface/write.rs` | `pub fn has_surface_form(r: &Record, ctx: &WriteContext, view: Option<&ViewId>) -> bool`, the single predicate `write_surface` honours | `SMY-W428` and `merge`'s warning share one rule. Fixes the commitment over-count (§2.5). |
| `src/main.rs` `cmd_merge` | replace the inline filter with `smysl::surface::has_surface_form` | same defect |
| `crates/smysl-graph/src/store/mod.rs` | `pub fn contains_record(&self, h: &[u8; 32]) -> bool`; `pub fn record_hash(r: &Record) -> [u8; 32]` made public (associated fn); `pub fn pending_attestations(&self) -> &[Attestation]` | classification and promotion without re-hashing the store; wait precedent made visible |
| same, `emit` | edge attestations by rid; annotation units (gaps A and B, §2.3) | **owned by SMYSL-2.1**, listed here so it is not lost |
| `crates/smysl-graph/src/lib.rs` | re-export `record_set_digest(impl IntoIterator<Item=&Record>)` (SMYSL-2.1) | one digest function |
| `src/lib.rs` (facade) | `pub mod lib { pub use smysl_lib::{import, export, verify, …}; }` behind feature `text` (draft 3 §17 facade) | rule A |
| `xtask/src/purity.rs` | add `"smysl-lib"` to `PURE_CRATES` | its core has no I/O beyond `std::fs` in `write_collection` and `import`, as `smysl-graph`'s store already does |
| `tests/dispatch.rs`, `tests/cli-surface.txt` | 27 commands; `lib` and its subcommands recorded | the existing guard for unrouted commands |
| `fuzz/Cargo.toml` | `smysl-lib` dependency; targets `lib_import`, `lib_roundtrip` | D-11 |

### 4.4 CLI

One `Cmd` row, with nested clap subcommands. This is the first nested group: `cli()` builds `sub`
per name, and the `"lib"` arm adds `.subcommand(...)` six times.

```rust
Cmd { name: "lib", about: "Import smysl files into the library; export it as files", purity: Purity::Pure, phase: "TX-P11" },
```

| subcommand | flags | exit |
|---|---|---|
| `lib import <inputs…>` | `--shard A`, `--accept`, `--attest`, `--keep-original`, `--licence SPDX`, `--origin URL`, `--name N`, `--no-join` | 0 accepted; 10 staged; 3 if anything was quarantined and `--strict`; 1 on `E493`/`E494` |
| `lib status` | `--import NAME`, `--json` | 0 |
| `lib accept <name>` | `--fail-on-contention` | 0, or 5 |
| `lib discard <name>` | | 0 |
| `lib export` | `--scope SQ` (required), `--layout single\|by-expression\|by-collection\|by-view\|by-time\|by-query`, `--window D`, `--group-by Q`, `--closure full\|ref`, `--format cbor\|surface\|pack`, `--budget N`, `-o DIR`, `--as-of EDTF`, `--with-text`, `--with-views`, `--with-packs`, `--with-provenance`, `--with-quarantine`, `--with-staged`, `--edges LIST`, `--unknown include\|drop`, `--target-revision ID@N`, `--agent ID`, `--no-verify` | 0; 3 on `E424` (index written with `complete: false`) |
| `lib verify <index>` | `--p2 <file>…`, `--keep-scratch` | 0 holds; 9 (`HashVerification`) on file/digest mismatch; 3 on `E424` |

- `--with-packs`, `--with-quarantine`, `--with-staged`, `--edges`, `--window` and `--shard`
  come from draft 3 §16, not its §17 table. They are added here. `accept`/`discard` are new
  (§3.3).
- **Global flags.** `--format` is global with `value_parser(["surface", "cbor"])`. The proposal
  (OQ-62) is to widen it to `["surface", "cbor", "pack"]`; every command except `lib export`
  then refuses `pack` with `Usage`.
- `-o` stays the global output *path*. `lib export` reads it as a directory, created if absent
  and refused if non-empty without `--force`.
- `--edges` reuses `edge_set` from `src/main.rs` after it moves behind the facade
  (`smysl::parse_edge_set`), which rule A requires anyway.
- Export verifies by default: after writing, it runs P1 and P3 in memory (`verify` on the
  `Collection` without touching disk), unless `--no-verify` is given. That is draft 3 §16.8's
  "refuses to write its index as complete".

### 4.5 Features, dependencies, purity, no C toolchain

- `smysl-lib` adds no third-party dependency beyond `blake3` (already `pure`, workspace).
- The surface writer and parser come from `smysl-core`; pack from `smysl-pack`.
- `smysl-lib` joins `PURE_CRATES`. `FORBIDDEN_SYMBOLS` (`std::net`, `async fn`, …) does not
  appear.
- The facade feature is `text` (draft 3 §17). `cli` enables it.
- No C toolchain: nothing new needs `cc`.

---

## 5. Tests, fixtures and harnesses

### 5.1 Unit tests (in `smysl-lib`)

- `select.rs`: one test per row of §3.6, including the reproductions of §2.3:
  - F13's four commitments travel
  - F11/F12's declaration travels
  - an edge attestation travels with its edge (gap A)
  - a relation `note`, a commitment `note` and a contention position are pulled in (gap B)
  - a withdrawn relation and its withdrawal both travel
  - two weight variants of one edge both travel
  - an `Unknown { code: 42 }` goes to `unattributed.cbor`, and `Unknown { code: 9 }` goes nowhere
- `layout.rs`: `slug` table tests (empty, uppercase, digits first, `unattributed`, collisions).
  Every generated id passes `ViewId::new`.
- `write.rs`: ordering invariance. Shuffle the snapshot's log order 50 times (seeded) and the
  bytes stay identical.
- `classify.rs`:
  - F6 imported: exactly the units with `SMY-E030` are quarantined, and dependents wait
  - an attestation arriving before its unit waits, then is promoted when the unit arrives in a
    second import
  - truncated CBOR gives `E493` and imports nothing
- `manifest.rs`: the same file imported twice gives one mid, `added = 0` and
  `already_imported = true`. Changing `--licence` gives a new mid.
- `index.rs`: encode → decode → encode is the identity. A tampered byte gives `E494`.

### 5.2 GE-T15 as an automated suite

`crates/smysl-lib/tests/ge_t15.rs`, driven by a table. Every cell computes P1 and P3 and records
the duplication factor. Cells that need later phases are `#[ignore]` until their dependency lands
and are listed by name, never silently skipped.

| axis | values |
|---|---|
| corpus | `fixtures/corpus/*.smy` (F1–F9, F11–F13, F8a+F8b merged), `fixtures/wire/*.cbor` (incl. F10-lifecycle, F12-reserved-pack), a generated store with unknown codes 14–19 and 42, the five-Bible corpus (TX-P1), two chats with datings, commitments, withdrawals, resolutions, contentions and a redaction (TX-P2/P3), a JSON snapshot series (TX-P1) |
| layout | `single`, `by-view` (now); `by-expression`, `by-collection` (TX-P1); `by-time` (TX-P3); `by-query` (TX-P6) |
| closure | `full`, `ref` |
| format | `cbor` (P1, P3); `surface` (W428 exactness check only) |
| flags | none; all `--with-*` |
| platform | CI on Linux x86-64 and macOS arm64 (two platforms); file BLAKE3s must agree across them |

Per cell:
1. Export.
2. `verify` P1 and P3. P2 runs for every corpus file imported raw.
3. Surface cells: the omitted set equals `{r : !has_surface_form(r)}` per type, and the parsed
   output equals the rest.
4. Assert no record type present in `X` is absent from the import side (the "any record type
   lost" criterion, checked per type code, not only by digest).

F6 runs with P1 over retained records and an extra assertion that the quarantined set is exactly
the `E030` units (OQ-60).

**Scale cells** (`--ignored`, run in the GE-T15 job): generated stores at 10⁵ and 10⁶ records.
Throughput and peak RSS of export and import are recorded against §2.10's baseline.

### 5.3 Property tests (D-11)

The repository's own convention is used, no new dependency: seeded xorshift cases in `cargo
test`, plus coverage-guided fuzz targets on `smysl_fuzz::generate`.

- `tests/property.rs` (300 seeded stores of up to 60 units, relations, threads, attestations on
  units and edges, commitments, withdrawals, resolutions, schema decls, label bindings, unknown
  records). For each store and each `(layout ∈ {single, by-view}, closure)`:
  - P1: `rsd(retained(import(export(S)))) == index.rsd`
  - P3: byte equality of a second export
  - **inclusion soundness**: every id in `names(r)` of an exported record is held by the
    importer, unless the source itself lacked it (`W495` count equals the source's dangling
    count)
  - **idempotence**: importing the collection twice gives `added = 0` on the second import, and
    the library digest is unchanged
  - **commutativity**: importing files A then B and importing B then A give equal library
    digests
- `fuzz/fuzz_targets/lib_roundtrip.rs`: `generate(&mut Choices, 40)` → `export_store` → in-memory
  import → assert P1 and P3. Layout and closure are chosen by `Choices`.
- `fuzz/fuzz_targets/lib_import.rs`: arbitrary bytes, and bytes from the `cbor` and `surface`
  corpora, as an import input into a small fixed library. Asserts:
  - no panic
  - either `E493`/`E003`, or every input record is classified exactly once (accepted + staged +
    waiting + quarantined + duplicates + dropped-redacted = decoded)
  - the library digest changes only by accepted records

### 5.4 Cross-implementation (D-8 C-Read additions)

The minimum each of `python/`, `nodejs/`, `go/` needs, with no new dependency:
1. `record_set_digest(records)`: BLAKE3 over `"smysl/rsd/1" ‖ 0x00 ‖ sorted unique
   BLAKE3(raw)`. About 15 lines each, using their existing hand-rolled BLAKE3 and the `raw`
   bytes they already keep. Test vectors are committed to `fixtures/wire/rsd/` (inputs plus
   expected hex), produced by the Rust.
2. `read_export_index(bytes)`: generic CBOR decode (already present) plus field names for §4.2's
   keys. Verify each file's BLAKE3, record count and rsd, and the selection rsd.
3. A writer test: decode each `fixtures/wire/*.cbor` and re-encode it (already done), and also
   build a small store of unit cores and relations with their own encoders. Write both to
   `target/xi/<impl>/*.cbor`.

Rust harness `crates/smysl-lib/tests/cross_impl.rs` (skipped with a message when `python3`, `node`
or `go` is absent; required in the GE-T15 CI job):
- import every `target/xi/<impl>/*.cbor`, then check P2 for each
- export F1–F13 and the generated stores into `target/xi/export/`, then run each
  implementation's `verify_export` and require byte-identical re-encoding and equal digests

Decoding records 14, 15, 17, 18 and 19 *typed* (D-8) is SMYSL-2.4's C-Read work. These round
trips already work on them generically, because all three keep unknown codes verbatim (§2.9).

---

## 6. Delivery steps (TX-P11; target-revision in TX-P12)

TX-P11 starts after TX-P7: `--scope` needs `sq` (TX-P6), and joins need `identical-span` (TX-P7).
Steps 1–4 do not need either and can start once TX-P0 has landed.

| # | work | needs | exit test |
|---|---|---|---|
| 1 | §4.3 prerequisites: `Subject::Record`, `has_surface_form` (+ `cmd_merge` uses it), `contains_record`, `record_hash`, `pending_attestations` | TX-P0 (`record_set_digest`) | `merge F13 --format surface` reports 0 omitted; existing suite green; `public-api*.txt` updated — **done 2026-10-21**, below |
| 2 | `smysl-lib` skeleton; `export_store` with `select`, `closure` (fixpoint), `write` (canonical), layouts `single`/`by-view`, `full` | 1 | §5.1 select tests; F1–F13 single-layout exports, in-memory P1 against `rsd(X)` — **done 2026-10-21**, below |
| 3 | `index`, `verify` (file checks, P1, P3), `ref` mode, duplication factor | 2 | `verify` passes on F1–F13 × {single, by-view} × {full, ref}; a flipped byte gives exit 9 — **done 2026-10-21**, below |
| 4 | import: decode, classify, CBOR staging, quarantine, waiting, tables (CBOR fallback), manifest (without mid until TX-P1 → placeholder hash of the body, replaced by `mid`), `accept`/`discard`/`status` | 3 | idempotence and commutativity property tests; F6 quarantine test; early-attestation promotion test — **done 2026-10-21**, below |
| 5 | CLI `lib` group, global `--format` change, dispatch and cli-surface tests | 4 | `tests/dispatch.rs` with 27 commands; `lib export … && lib verify` exits 0 on F1–F13 |
| 6 | Manifest-backed parts: real mids, `--keep-original` (o3), `--with-text`, records 14/15/18/19 rows, redaction at the door, layouts `by-expression`/`by-collection` | TX-P1, TX-P2 | Bible corpus cells of GE-T15 pass P1/P3; a redacted tid's 15/18 are dropped on import (`W414`) |
| 7 | Datings and time: rows 11/13/17 with datings, `by-time`, `--as-of` | TX-P3 | chat cells pass; `--as-of` excludes exactly the planted late records, with W415/W498 counts |
| 8 | `--scope` through `sq`, `by-query`, `from import:<…>` resolution (§3.9) | TX-P6 | P3 with scope `from import:<collection>` is byte-identical on all landed cells |
| 9 | Joins: span re-verification, `identical-span` + lexical same-as into `join:<name>` | TX-P5, TX-P7 | an imported unit with an out-of-range span is quarantined (`E404`); same-span units from two imports gain one same-as edge; P1 unaffected (`--no-join` and with joins) |
| 10 | `surface` and `pack` formats, W428 exactness | 5 | surface cells of GE-T15 |
| 11 | Fuzz targets, cross-implementation additions and harness | 5 | 1 h fuzz on each target with no finding; all three implementations verify every export |
| 12 | GE-T15 full run, scale cells at 10⁵/10⁶ | 6–11 | GE-T15 kill criteria (draft 3 §23): no P1/P3 failure, no record type lost; duplication factor per layout recorded (OQ-26) |
| 13 (TX-P12) | `--target-revision` | SMYSL-2.5 lens engine | GE-T16's synthetic revisions: a `split-value` lens exports; a `retire` refuses with `E422`; a two-level grounds chain gets new uids in dependency order and every rewritten core references only mapped uids |

*Step 1, done 2026-10-21.* The five prerequisites of §4.3, against the tree as it is now and not
`d25ec9e`:

- **`Subject::Record([u8; 32])`** in `smysl_core::diag`, with `Diagnostic::on_record` and
  `Diagnostic::record()`. It prints as `(at rec:<26 base32>)`, a uid's short width under its own
  prefix, so a record hash is never read as a uid. No pass attributes with it yet: §4.3 makes
  that gradual, one pass at a time, and step 4's quarantine is the first reader.
- **`smysl_core::surface::has_surface_form(r, ctx, view)`**, composed of the per-type predicates
  `write_surface` already guards each arm with; `merge` and `bundle` count omissions with it
  and keep no list of their own. `tests/surface_form.rs` holds it to the writer record by
  record: over the corpus, the wire fixtures, the manifest fixture and a dating fixture, a
  record it calls written is written on its own and one it calls omitted writes nothing.
- **`Store::record_hash`** made public, **`Store::contains_record`** and
  **`Store::pending_attestations`**; and **`smysl_graph::record_set_digest(records)`**, the
  digest of records not held in a store, which the store's own method and it share one framing
  for (§3.1's `smysl/rsd/1`, repeats once, order free).

`merge F13 --format surface` reports nothing omitted; it already did, since H-16 fixed the
commitment over-count this step was first written for. One finding:

- **The count still over-counted, by every redaction and every dating.** The list `merge` kept
  beside the writer ended in `_ => true`, and `@redact` (1.10, rule Z) and `@date` (TX-P3)
  arrived after it; the manifest fixture was reported as 4 records omitted — its 3 datings and
  1 redaction — and a one-dating document as 1, over output holding every one.
  `tests/cmd_merge.rs` now asserts none omitted on all three and that every `@commit`,
  `@manifest`, `@redact` and `@date` comes back. A second list drifted from the writer a third
  time, which is the case for §4.3's one predicate.

*Step 2, done 2026-10-21.* Crate `smysl-lib`, registered as pure and unpublished. It depends on
`smysl-core`, `smysl-graph` and `blake3`, and on nothing from §4.1's list that step 2 does not
read: `smysl-check`, `smysl-text` and `smysl-query` arrive with import, manifests and `sq`.

- **API.** `export_store(&Store, &Selection, &ExportOptions) -> Result<Collection, LibError>`.
  - `Selection::all` or `Selection::of` stands in for the `sq` scope until step 8.
  - `Layout` is `Single` or `ByView`, `Closure` is `Full`, and `With` covers `views` and
    `packs`. `UnknownRecords` is `Include` or `Drop`. Every enum and options struct is
    `#[non_exhaustive]`, so the later steps add to them.
  - A `Collection` holds its files sorted by path, `rsd` (that is `rsd(X)`), the duplication
    factor's two terms, and a `Report`. The free function `slug` is §3.4's.
  - `LibError` has one variant, a selected uid the store does not hold.
- **Codes.** `SMY-W425`, `W490`, `W495` and `W497` are registered in a twelfth group, `Export`,
  with the code that raises them (118 codes).
- **`by-view`, made exact.** A unit's key is the least corpus view id, in byte order, whose
  membership (roots closed along `deps` and `grounds`) holds it. A unit in no view goes to
  `unkeyed`. `export/` views are never keys. Each unit is a root of exactly one file.
- **Rows 14, 17 and 19 as written, ahead of steps 6 and 7.** The types exist in this tree, so a
  table that skipped them would lose them.
  - A manifest travels when an included unit's `source` names its mid.
  - A dating travels when its target is included. A part target counts when an included
    unit's `source.reference` names that part.
  - A redaction travels with any part an included unit references.
  - A dating's `basis` and a redaction's `reason` join the annotation pull-in, and the
    datings' dids count for rows 11 and 13.
  - Records 15 and 18 are not selected until step 6 brings `--with-text`. That is the rule,
    not a loss.
- **The generated view's `lang`** counts only the selected units that name a language. If they
  all name one, the view takes it; if they name several, `mul`; if none names any, `und`.
- **Duplication** counts selected records and leaves generated views out of both terms. Every
  file has its own view, so counting them would only add one to each side.

The exit test asks that, with every unit selected, nothing is lost but what W490 counts. Over F1–F13
and F14 under both layouts, that found three places where §3.6's table is not complete. Each
is now completed in `select.rs`:

1. **Row 8 misses a declaration that lists neither a type nor a kind.** F11's `x.code/v1`
   declares nothing; its units use it through payload keys (`"code:kind"`), and §5.1's own
   "F11's declaration travels" failed. A declaration now also travels with a unit whose
   payload has a key prefixed by the schema's domain (`code` for `x.code/…`). That is the one
   link the format makes between a payload and a schema. §5 of the format spec lists
   `meta:` keys under `x.text/v1`, so it is not complete, and a payload-only schema outside
   its own domain still needs a view's `requires`.
2. **Row 12 misses a resolution of a detected contention.** Merge detects contentions and never
   records them (§2.2), so "a contention id of an included contention" never matched a
   resolution of a detected one. Detection over the snapshot now counts. A detected
   contention whose `over` is in the file has its positions pulled in, as a recorded one's
   are, so the recipient derives the same id.
3. **"Exported as they are" had no file to go to.** F10 resolves `k/ccm3…`, which its store
   neither records nor detects (no thread holds the rebuttals). Row 12 dropped it silently,
   and §3.6's sentence on records dangling at source says they travel. A record whose
   target the snapshot does not hold at all is an attestation, binding, withdrawal,
   resolution, commitment, contention, dating or relation naming nothing held. No key owns
   it, so it now goes into **every** keyed file and is counted once (`W495`). A relation
   with one missing end goes where its held end goes. Step 3's `ref` mode will give each
   one owner, the lowest path.

The test is `crates/smysl-lib/tests/export.rs`, 15 tests:
- **§5.1's select rows.** These include §2.3's reproductions: F13's four commitments, F11's and
  F12's declarations, an edge attestation, the annotation pull-in, a withdrawn relation with its
  withdrawal, two weights of one edge, and `Unknown { 42 }` against `Unknown { 9 }`.
- **Rules L and R.**
- **A thread left out and counted.**
- **A refused selection.**
- **Byte-identical files over 50 seeded shuffles of every fixture's log.**
- **The exit.** Over F1–F13 and F14 under `single` and `by-view`, it checks:
  - P1, with `Store::from_records` of the files standing in for an import into an empty
    library until step 4;
  - each file's `rsd` against its decoded bytes;
  - each file's name against its view id;
  - and that, with every unit selected, a record is lost only as W490 counts.

`layout.rs`'s unit tests are §5.1's slug table. Every generated id passes `ViewId::new`.

*Step 3, done 2026-10-21.* `ref` closure, the index, `verify`, and the duplication factor in
the index. `SMY-E494` is registered in the Export group, which makes the registry 119 codes.

- **API.**
  - `Closure::Ref`, with `as_str` and `parse`. `Layout::parse`.
  - `ExportedFile.requires`.
  - `Collection` now holds its `index`. `rsd()` and `duplication()` read from it, as §4.2's
    sketch has it.
  - `index.rs`: `ExportIndex` and `IndexFile`, with `to_cbor` and a strict `from_cbor`, plus
    `INDEX_FILE`.
  - `verify.rs`:
    - `verify(index path)`, which reads the files next to the index;
    - `verify_collection`, in memory, for export's own check from step 5;
    - `verify_files`;
    - `write_collection`, which writes the index last;
    - `VerifyReport` with `holds()` and `exit_code()` (0, or 9 `HashVerification`);
    - `Property`;
    - `FileCheck`.
- **The index.** §4.2's table, field by field.
  - Keys 3, 5, 9 and 13 are absent until `sq`, lenses, `--as-of` and `--target-revision`
    arrive. `scope` is empty until step 8, and `format` is `cbor`.
  - `counts` holds all eight codes every time, zero or not, keyed by code text.
  - `flags` names the edges by kind as the store holds them (`edges=deps,grounds,…`), so P3
    can rebuild the set over the store an import makes. Extension kinds are interned per
    store and have no stable number.
  - Decoding refuses an unknown key, a missing required key, a digest that is not 32 bytes,
    and anything that does not re-encode to the same bytes. Encode, decode and encode again
    is the identity.
- **`ref`** follows §3.7.
  - A unit record is owned by the file that selects it as a root. The layouts give every
    selected unit exactly one.
  - Any other record is owned by the least path among the files that select it.
  - `requires` is the sorted set of the other owners' paths, stored in the index and never in
    the view. The view is built from the selection, so it is the same under both modes.
  - The duplication factor counts what files *need* (`Σ|R_k|`, `|⋃R_k|`), so it is the same
    under `full` and `ref`. Generated views and `unattributed.cbor` are in neither term.
- **`verify`.**
  - Each file the index names must exist and match its BLAKE3, record count and `rsd`.
    Otherwise it raises `E494`, and P1 and P3 are not applicable.
  - P1 compares `Store::from_records` of all the files with `index.rsd`. When it fails, it
    lists the missing and extra hashes.
  - P3 re-exports that store with the index's layout, closure and flags. The selection is the
    union of the roots of its `export/` views (§3.9). It compares every file byte for byte,
    then the index's `files` and `rsd`. When it fails, it names the first differing file and
    byte offset.
  - P2 needs import and arrives with it.
- **A fourth completion of §3.6, found by P3.**
  - Under `by-view`, a file's key is the least corpus view holding its units. Without
    `--with-views`, no corpus view travelled.
  - So a recipient had nothing to key by: a re-export put everything in `unkeyed`, and P3
    could not hold.
  - Now the view a `by-view` file is keyed by travels in that file, whatever `--with-views`
    says. Row 5 still governs every other view.
  - The re-export then finds the same least view for every unit. The views a recipient holds
    are a subset of the source's that contains every view used as a key.

The test is `crates/smysl-lib/tests/verify.rs`, 8 tests:
- **The exit.** `verify` holds, P1 and P3 included, on F1–F13, F10 and F14 under {`single`,
  `by-view`} × {`full`, `ref`} × {no `--with-*`, views and packs}. `ref` writes every record
  once, with the same `rsd` and duplication factor as `full`, and the factor matches what
  `full` writes.
- **A flipped byte gives exit 9,** with `E494`, under both closures, through
  `write_collection` and `verify` on disk. A deleted file does too.
- **The index decodes to itself on every fixture.** A flipped bit either fails to decode or
  gives a different index, and a cut-short index is an error.
- **The index records what the export was**: options, flags, schemas, counts, and per-file
  digests.
- **`ref`'s ownership and `requires`** on a grounded pair across two views, with duplication
  (5, 4) under both modes. A `ref` file verified without the file it requires fails.
- **P3 fails when an index misstates the closure,** even though every file matches its own
  entry.
- **The keying view travels without `--with-views`.**

`tests/export.rs`'s by-view duplication is now (5, 5): the three units plus each file's
keying view.

*Step 4, done 2026-10-21.* Import, against the library as it is now. Three codes are
registered in the Export group: `SMY-E493`, `SMY-W496`, and `SMY-W423` (SMYSL-2.0's, raised
only here). That makes the registry 122. `smysl-lib` now depends on `smysl-check` and
`smysl-text`, both pure.

- **The seam.**
  - `LibraryRead`, with `snapshot` and `root`, and `LibraryWrite`, with `append`. Both are
    implemented for `smysl_text::Library`.
  - It is narrower than §4.2's sketch on purpose. A library has one log until SMYSL-2.8
    shards it, text objects are step 6's, and the tables have their CBOR fallback. The traits
    grow as those land.
  - The caller holds the locks, as for `Library::add`.
- **API.**
  - `import(lib, inputs, opts)`, `accept`, `discard`, `status`, and `retained` (the import
    side of §3.9).
  - `Input::{File, Dir, Index, Bytes}`.
  - `ImportOptions` (`name`, `accept`, `licence`, `origin`), `ImportReport`, `FileOutcome`.
  - `Batch` and `State` (`accepted`, `staged`, `waiting`, `quarantined`, `discarded`).
- **Decoding.**
  - A file is surface when `smysl check` would read it as surface. Otherwise it is a CBOR
    sequence, which must decode to its last byte. Anything else is `E493` with no manifest.
  - An index's files are checked against it first, and one mismatch refuses the whole
    collection (`E494`).
  - Records 15 and 18 are counted (`FileOutcome.text`) and left out. A log refuses them
    (OQ-39), and step 6 puts them in the object store.
- **The manifest** follows §3.1, one per file:
  - alias `import:<name>`, or `import:<name>/<stem>` when the import holds several files;
  - reader `smysl-lib/import/1`, lang `und`, and the default part policy;
  - origin `--origin`, or the basename;
  - `identifiers.original` and `identifiers.records` in hex;
  - no parts.

  A manifest already in the library makes the file `already_imported`, and nothing else
  happens. That is idempotence.
- **The tables** live in `lib/tables/{imported,batches,waiting}.cbor`, deterministic CBOR in key
  order, written by rename.
  - `waiting` is stored by record (§4.1's `waiting_by_record`), and the by-id view is built
    when promotion asks.
  - A contention id gets tag `0x03`, which §4.1's table lacks: a resolution of a contention
    nobody holds waits as an attestation does.
  - Units, rids and dids share tag `0x01`, as they share the uid space on the wire. An
    attestation's `uid` does not say which of the three it names.
- **Classification** follows §3.3, with two findings from the exit:
  - **The check runs over what does not wait.** A missing ground is `SMY-E060`, an error.
    Checked whole, a unit whose ground had not yet arrived would have been quarantined, not
    held waiting.
  - **A unit whose finding rests on another unit with one waits for it.** It is not
    quarantined beside it. Each round quarantines only the minimal offenders. F6's finding
    breaks rule M itself and rests on the two claims that break it first. Quarantined beside
    them when it arrived with them, it waited when it arrived first, and the commutativity
    test caught the difference. Only a cycle of findings is quarantined whole.
  - A quarantined unit takes its attestations, bindings, commitments, edges and contentions
    with it. Views, threads and the rest wait.
- **Staging and promotion.**
  - Without `accept`, the accepted set goes to `lib/staged/<name>.cbor`, exact and in canonical
    order, and only the manifests are appended. `accept` appends the batch. With
    `fail_on_contention`, a new detected contention refuses it (`LibError::Contentions`, exit 5
    in step 5).
  - `discard` drops the staged and waiting records. Manifests and quarantine stay, since the
    files were read.
  - Promotion re-classifies every waiting record of every import that is not discarded, to a
    fixpoint, after each accepted import and each `accept`. Each record goes where its import's
    content went. A staged import holds its own staged records as held.

The test is `crates/smysl-lib/tests/import.rs`, 10 tests:
- **Commutativity.** Every fixture is cut into three files at random (seeded), twice, and
  imported in all six orders. The library's content and the waiting and quarantined counts
  agree.
- **Idempotence.** Every fixture in two pieces, imported twice: every file says
  `already_imported`, and the library and its tables are unchanged.
- **Promotion to the whole.** Every fixture in four pieces gives the library that the fixture
  imported whole gives.
- **F6.** Exactly the two minimal `E030` claims are quarantined, with `W423` and a `.report`
  naming `E030`. The finding and the view rooted on it wait (`W496`). The retained set is the
  file's.
- **An early attestation** waits in the table and in `lib/waiting/`, and is appended when a
  second import brings its unit.
- **Staging.** A staged-then-accepted import equals an accepted one, record for record, and a
  discarded one retains nothing.
- **Truncated CBOR** gives `E493` and writes nothing, not even a manifest.
- **One mid per file and options.** Importing twice gives `added = 0`. A new licence gives a
  new mid and no new content. The manifest names the basename.
- **P1 through a real import,** replacing step 3's stand-in in the test: every fixture under
  {`single`, `by-view`} × {`full`, `ref`}, written, imported as an index into an empty
  library. `rsd(retained)` equals `index.rsd`.
- **A collection with a flipped byte** gives `E494` and imports nothing.

---

## 7. Risks and mitigations

| risk | effect | mitigation |
|---|---|---|
| **Closure duplication** under `full` on a densely grounded library | output size × files; GE-T15 kill at 3× for `by-expression` | duplication factor in every index; `ref` mode ready from step 3; OQ-26 decides the default from measurement |
| **Unknown records** cannot be attributed | they leave the files they relate to | `unattributed.cbor` plus `W425`; included in `rsd(X)`; `--unknown drop` explicit. Once SMYSL-2.4 types records 14–19 they stop being unknown, and GE-T15 runs both before and after |
| **Memory at 10⁶ records** | §2.10 extrapolates to about 1.5 GB to load and about 3.4 GB through merge; import also clones the shard to check | one `append` per file; the check runs over the shard plus increment, not the library; documented limit until TX-P13; scale cells measure it; TX-P13 (`StoreRead`) is the fix, and draft 3 already gates TX-P13 on GE-T15 |
| **Check is store-level** | over-quarantine (whole file on a `Subject::Store` error) | `Subject::Record` and per-pass attribution (§4.3); every quarantine names its code; `lib accept --quarantined` is deliberately absent, so a reviewer edits or re-imports |
| **P3 drift through generated views** | a second export pulls in views from other files | `export/` namespace excluded from `--with-views`; `from import:<collection>` = generated roots (§3.9, OQ-61) |
| **Dangling at source** | recipients wait forever | `W495` at export; waiting is retained and visible in `lib status`; P1 counts it (OQ-60) |
| **Cost of check per import** | `check` is O(store) per file | directories and indexes are checked as one combined batch (one check per import call, not per file); the measured `check` scaling (`crates/smysl-check/tests/scaling.rs`) bounds it |
| **Clock in exported attestations** | non-determinism (rule D) if read from the wall clock | `--target-revision` uses a clock derived from the selection (OQ-63); import uses the fixed `Hlc::new(0,0,tool:smysl-lib)` merge already uses |
| **Global flag change** | `--format pack` is accepted by commands that cannot honour it | each command refuses with `Usage`; `tests/global_flags.rs` gains the case |
| **No signatures** | an import cannot authenticate an origin | staging stays default (OQ-25); `--attest` records who vouched, not who wrote |

---

## 8. Diagnostics allocated in this RFC (SMY-W/E490..499)

The set's registry is SMYSL-2.0 §6.1. These codes are allocated in this RFC's range. Existing codes used here: `E003`,
`E060`, `E404`, `W405`, `W414`, `W415`, `E422`, `W423`, `E424`, `W425`, `W428`, `W429`.

| code | meaning |
|---|---|
| SMY-W490 | a record was left out of an export file because the records it names are not all included (a thread's steps, a pack info's uids); count per type |
| SMY-W491 | `--target-revision` dropped commitments, datings, withdrawals, resolutions or contentions naming original uids; count |
| SMY-E492 | `--target-revision` cannot order the units to rewrite (cycle in `deps`/`grounds` within the selection); export refused |
| SMY-E493 | a `lib import` input is truncated or undecodable; the file is rejected whole, nothing from it is imported |
| SMY-E494 | an export collection's file does not match its index (BLAKE3, record count or record-set digest); import or verify refused |
| SMY-W495 | exported records reference uids the source store does not hold; the recipient will hold them waiting; count |
| SMY-W496 | imported records are waiting for a unit, edge, part or manifest the library does not hold; count per import |
| SMY-W497 | a layout key is not a view-id segment, or two keys collide; the generated id carries a hash suffix; count |
| SMY-W498 | `--as-of` excluded units whose `deps`/`grounds` were not yet known on that date; count |
| SMY-W499 | reserved |

---

## 9. Open questions

Draft 3 numbers where they apply. New ones are OQ-60..OQ-64.

| id | question | proposed answer |
|---|---|---|
| OQ-25 | Should imports from a trusted source skip staging? | No, while signatures are unimplemented (§3.3). Revisit with N9. |
| OQ-26 | `full` duplication against `ref`'s whole-collection requirement | Default `full`; switch to `ref` if GE-T15 measures > 3× for `by-expression`. The factor is in every index. |
| OQ-27 | Digest beside or instead of `state_hash` in `converged_with` | Answered in SMYSL-2.1 (TX-P0). This RFC uses only the digest. |
| OQ-28 | An import-event time for `--as-of` | Not in TX-P11. It would be a new record type (format change), so it is SMYSL-2.3's call. |
| **OQ-60** | Does P1 compare all **retained** records (accepted, staged, waiting, quarantined) or only accepted ones? | Retained: P1 is about transport, and F6 must round-trip. The accepted-only digest is reported beside it, and it must equal `rsd(X)` when the source checks clean. |
| **OQ-61** | Generated views: id `export/<slug>` (draft 3's `v/export/<key>` is illegal); exclusion of `export/` views from `--with-views`; `from import:<collection>` = roots of generated views; P2's exception for a re-export under another layout; annotation units and the pack-info rule in the inclusion table | **Resolved here:** all adopted in this RFC (tool-level). SMYSL-2.3 reserves the `export/` and `import:` alias prefixes. |
| **OQ-62** | `--format pack` and `-o DIR` against today's global flags | Widen the global `--format` parser and refuse `pack` elsewhere. The alternative is a local `--as` flag, which breaks draft 3's spelling. |
| **OQ-63** | The clock and agent of attestations export writes (`--target-revision`) | `--agent` (default `tool:smysl-lib`). `ts` = `(max physical ts in the selection, max logical + 1)`: deterministic, after everything it rewrites. An explicit `--clock` overrides. |
| **OQ-64** | Threads and pack infos whose members span files are left out (`W490`). Should `ref` mode instead write them into the owner file of their first step, with `requires`? | Keep draft 3's rule for TX-P11 and measure how many are left out on the chat corpora. Decide before TX-P13. |
