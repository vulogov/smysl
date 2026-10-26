# RFC SMYSL-2.8 — Scale: StoreRead, shards and the disk-backed store

**Status:** draft 1, for discussion. Implementation RFC (non-normative); normative rules are in SMYSL-2.3.
**Author:** Vladimir Ulogov
**Date:** 2026-10-02
**Part of:** RFC set SMYSL-2 — see SMYSL-2.0 (index and roadmap).
**For:** crate `1.9.0-dev` (`d25ec9e`), format `smysl/1.0`, kernel `smysl.kernel/0.1`.
**Derived from:** RFC SMYSL-2 draft 3, §14 (esp. §14.4), §11.4, §2.2, F-1, F-10, F-15, GE-T7, GE-T15, §22 TX-P13.
**Phases:** TX-P13 (split here into P13a–P13d, §6).
**Depends on:** SMYSL-2.1 (record-set digest, TX-P0), the existing spec's rules D and U, SMYSL-2.3 (record-set digest A-4, rule Z), SMYSL-2.0 §6.1 (diagnostic registry), SMYSL-2.4 (library layout, substrate index, library lock), SMYSL-2.7 (GE-T15 gates the disk-primary mode).

---

## 0. Summary

`Store` is one in-memory struct, and every pure operation in eleven crates takes `&Store`. At
155k units it holds about 2.6 KB per unit, rebuilds its adjacency on every `append`, and spends
roughly 90% of a `check` loading the log rather than checking it. A library of 10⁶ units does
not fit that shape.

This RFC turns `Store` into one implementation of a read trait, and adds two more:

- **`StoreRead`**, a sealed, generic (not object-safe) trait in `smysl-graph`. Its method set is
  taken from an inventory of what the consumers actually call (§2.6), not from `Store`'s 59
  public functions. Units come back through an associated `UnitRef` type, so the in-memory
  store keeps handing out `&Unit` and a paged store hands out `Arc<Unit>`.
- **`StoreWrite`** for append, implemented by `Store` and by a redb-backed shard.
- **`RedbStore`**, a module of `smysl-graph` behind feature `store-redb`. The interpretive
  layer (relations, commitments, withdrawals, resolutions, labels, threads, views) and a fixed
  per-unit header with the adjacency stay **hot**. Unit cores stay **cold** and are paged.
- **`Union`**, the virtual union of shards (one log per expression plus a catalog shard), with
  dense ids computed over the union's uid set, so determinism (rule D) survives sharding.
- **`Overlay`**, a store plus a batch. Ingest staging uses it instead of copying the whole
  store into a new one.

The in-memory `Store` stays the reference implementation. A conformance suite runs every
pure operation against every implementation and requires identical bytes. The CBOR log stays
the authority: redb is an index until GE-T15 passes, and afterwards may become primary, with
`lib export` as the way out.

Measuring for this RFC also found two defects that do not need a disk store to fix. `derive_thread`
is quadratic in units × relations (24.5 s at 171k units; 0.55 s with the fix, same bytes,
§2.8). `append` can drop its `O(store)` term without losing `Sync` (§3.4).

## 1. Scope

**In:**
- extraction of `StoreRead` and `StoreWrite` from `Store`, and porting every consumer to them;
- the in-memory fixes found by measurement: lazy adjacency, single hashing in `from_records`,
  the thread matcher;
- a redb-backed unit store: table layout, hot/cold tiering, caching, open and rebuild,
  crash behaviour;
- shards, the catalog shard, the virtual union, cross-shard waiting edges, contentions over a
  union (OQ-16);
- concurrency for the redb store (revisiting D-10);
- migration for existing users, and the disk-primary switch after GE-T15;
- GE-T7 as a test protocol, plus a synthetic scale ladder that runs in CI.

**Out:**
- the substrate index tables of draft 3 §14.2 (`postings`, `said`, `segments`, …): SMYSL-2.4.
  This RFC adds only the per-shard unit store and one directory table (§3.6).
- the object store for parts and readings: SMYSL-2.4.
- record layouts and the record-set digest: SMYSL-2.3 and SMYSL-2.1. They are used here,
  not defined.
- `lib import` / `lib export`: SMYSL-2.7. This RFC depends on GE-T15 and does not change it.
- distributed or networked stores. The pure crates stay offline (rule B).

## 2. What exists today

Verified against `d25ec9e`. Paths are relative to the repository root. Measurements are on
a 2-core machine, release build (`--no-default-features --features cli` for the CLI), and are
mine unless attributed to draft 3.

### 2.1 The `Store` struct

`crates/smysl-graph/src/store/mod.rs`, `pub struct Store` (derives `Debug, Clone`):

| field | type | role |
|---|---|---|
| `path` | `Option<PathBuf>` | log file, if any |
| `records` | `Vec<Record>` | every record in log order — **the authority in memory** |
| `log_len`, `log_hash` | `u64`, `[u8; 32]` | what the sidecar is validated against |
| `pending_attestations` | `Vec<Attestation>` | attestations whose subject has not arrived (1.8) |
| `log_hasher` | `Rolling` | running BLAKE3 so `append` hashes only new bytes (1.8) |
| `units` | `BTreeMap<Uid, Unit>` | units by uid; `absorb` inserts `Unit::new(u.clone())` |
| `relations` | `BTreeMap<(String, Uid, Uid), Relation>` | keyed by (kind, from, to) |
| `threads` | `BTreeMap<(ThreadId, AgentId), Thread>` | LWW register per key, ties by bytes |
| `views` | `BTreeMap<ViewId, View>` | |
| `contentions` | `Vec<Contention>` | recorded ones only, idempotent by id |
| `withdrawals` | `BTreeMap<Uid, BTreeSet<Withdrawal>>` | by the rid they name |
| `resolutions` | `BTreeSet<Resolution>` | |
| `commits` | `BTreeMap<Uid, BTreeSet<Commit>>` | by the unit they name (1.7) |
| `rids` | `BTreeMap<Uid, (String, Uid, Uid)>` | rid → relation key |
| `record_hashes` | `BTreeSet<[u8; 32]>` | BLAKE3 of every record's encoding; dedup for `append` |
| `unfounded` | `BTreeSet<Uid>` | strict-policy `unfounded` set, re-derived on each rebuild |
| `adjacency` | `Adjacency` | rebuilt in `rebuild_adjacency` on every `absorb` |

Two properties matter for scale:
- **Every unit core is held twice**: once inside `records` and once, cloned, in `units`
  (`absorb`: `Unit::new(u.clone())`, then `self.records.extend(records)`).
- Schema declarations, label bindings, pack infos and unknown records have **no field of their
  own**. Readers find them by scanning `records` (§2.6, `iter`).

### 2.2 Public methods

59 `pub fn` in `impl Store` (counted by marking each `#[deprecated]` in a scratch copy, §2.6):

- **construct / IO (14):** `new`, `from_records`, `open`, `open_with`, `index_path`, `append`,
  `write_index`, `reindex`, `index`, `verify_against`, `log_bytes`, `log_len`, `log_hash`, `path`.
- **units (10):** `get`, `contains_uid`, `units`, `matching_prefix`, `resolve_prefix`,
  `units_in_observed_order`, `observed_at`, `units_with_source_prefix`, `len`, `is_empty`.
- **episodes (4):** `hop_of`, `hops`, `latest_hop`, `at_hop`.
- **relations and edges (8):** `relations`, `relations_of_kind`, `relation_by_id`,
  `has_relation`, `adjacency`, `rebuttals_of`, `supports_of`, `report_dangling`.
- **lifecycle (9):** `is_withdrawn`, `is_live_rebuttal`, `is_unfounded`, `withdrawals`,
  `resolutions`, `is_resolved`, `contentions`, `contention_status`, `open_contentions`.
- **commitments (3):** `commits_of`, `commitment_of`, `units_at_commitment`.
- **attestations (3):** `attestations_of`, `attested_by`, `agreement`.
- **records, threads, views and convergence (8):** `iter`, `duplicate_records`, `bundle`, `bundle_with`,
  `state_hash`, `converged_with`, `threads`, `views`.

`path` is called nowhere in the workspace. There is no `record_set_digest` yet; SMYSL-2.1 adds
it in TX-P0 (F-17).

### 2.3 The index sidecar

`crates/smysl-graph/src/store/index.rs`. Magic `SMYSLIDX`, `VERSION = 1`, deterministic
big-endian encoding; `Index` holds `entries` (uid → offset, len, type code), `fwd_adj` and
`rev_adj` keyed by uid, `labels`, `threads`, `contentions` offsets and `cache` (status,
salience quantised to 1/1024). `Index::matches(log_len, log_hash)` decides staleness;
`SMY-W110` reports a rebuild.

What the sidecar does **not** do today:
- `Store::open_with` absorbs every record whether or not the sidecar is current. The sidecar is
  consulted only to set `index_rebuilt` and, with `verify_hashes`, for `verify_against`.
  Opening is therefore never faster for having a sidecar.
- The CLI's `load_store` (`src/main.rs`) reads bytes and calls `Store::from_records`; it does
  not open the sidecar at all. `Store::open`/`open_with` are called only from `cmd_retract`,
  `cmd_compact` and `cmd_reindex`.
- `reindex` opens (one absorb) and then calls `Store::reindex`, which absorbs again.

The sidecar is 2.1× the log size on the 155k edged corpus (20.0 MB against 23.9 MB).

### 2.4 Adjacency

`crates/smysl-graph/src/adjacency.rs`. `Adjacency::build(&BTreeMap<Uid, Unit>, &[Relation])`
collects every uid mentioned anywhere (units, deps, grounds, relation endpoints, notes) into
a `BTreeSet`, so **dense `NodeId`s are assigned in ascending uid order**, and referenced but
absent uids are nodes with `present = false` (the `SMY-E060` candidates). Edge lists are
sorted by `(kind, target)` and deduplicated, which is what makes traversals canonical without
a sort per step. Extension relation kinds are interned per store (`EdgeKind::Extension(u16)`).

`NodeId` is a public `u32` alias, blessed as contract in `Documentation/API_CONTRACT.md`
(bucket 3): stable for one store at one moment, never persisted or compared across stores.
That contract is exactly what a union needs (§3.7): a union is "one store at one moment".

### 2.5 Append cost and the `Sync` note

The doc comment on `Store::append` and CHANGELOG 1.8 ("Appending stopped costing the store")
record that the log and its hash became `O(new)` in 1.8, and that **the adjacency is rebuilt
from every unit and relation once per `append` call**: 1336 µs per record at one record per
call, 5 µs at 1000. CHANGELOG 1.8 gives 20,000 single appends 34.6 s → 24.0 s, and 0.30 s in
batches of 100.

Both say lazy rebuilding "was not done" because it needs interior mutability and "would make
`Store` no longer `Sync`". That holds for `RefCell`/`Cell`. It does not hold for
`std::sync::OnceLock`, which is `Sync` when its contents are `Send + Sync` and is stable since
Rust 1.70 (the workspace declared `rust-version = "1.79"` when this was written; it declares
1.85 since 1.10.0, and the 1.79 was false — OQ-66). I checked by compiling an
assertion against `d25ec9e`: `Store: Send + Sync` and `OnceLock<Adjacency>: Send + Sync` both
hold. No test in the tree asserts that `Store` is `Sync`; nothing would notice if it stopped
being.

Measured at 155k units, one record appended in memory: **104 ms** (no edges) and **172 ms**
(15.5k relations).

### 2.6 Consumer inventory

**Method.** In a scratch clone I put `#[deprecated(note = "INV:…")]` on every `pub fn` of
`impl Store` and `impl Adjacency`, ran `cargo check --workspace --all-targets --features cli
--message-format=json`, and collected each warning's primary span. This counts every call the
compiler resolves to those methods, including calls inside `smysl-graph`, and nothing a text
grep would mis-attribute. Lines after a `#[cfg(test)] mod` in a source file count as tests.
Feature-gated code not built by that command (e.g. `semantic` arms of `src/main.rs`) is missed;
the `semantic` arm of `ranked` calls only `Bm25::index` and `Semantic::index`, both listed below.

**Table A — Store and Adjacency methods called, per consumer (non-test code).**

| consumer | `Store` methods | `Adjacency` methods |
|---|---|---|
| smysl-check `lib.rs` | views | — |
| smysl-check `passes/closure.rs` | matching_prefix, resolve_prefix, units | — |
| smysl-check `passes/commitment.rs` | commitment_of, units | — |
| smysl-check `passes/epistemics.rs` | adjacency, get | uid |
| smysl-check `passes/extension.rs` | iter (schema decls, unknown records), relations, units, views | — |
| smysl-check `passes/granularity.rs`, `shape.rs`, `trust.rs` | units | — |
| smysl-check `passes/integrity.rs` | adjacency, duplicate_records, relation_by_id, report_dangling, withdrawals | uid |
| smysl-pack `bound.rs` | get | — |
| smysl-pack `closure.rs` | contains_uid, get, open_contentions, rebuttals_of, supports_of | — |
| smysl-pack `constraints.rs` | contains_uid, get, open_contentions, rebuttals_of, relations_of_kind, supports_of | — |
| smysl-pack `solve.rs` | contains_uid, get, units | — |
| smysl-thread `derive.rs` | adjacency, contains_uid, get, observed_at, relations_of_kind, units | id, incoming, out, uid |
| smysl-render `ir.rs` | contention_status, contentions, get, is_withdrawn, relations | — |
| smysl-graph `lineage.rs` | adjacency, contains_uid, get, relations_of_kind, supports_of, units | id, in_edges, len, out_edges, uid |
| smysl-graph `salience.rs` | adjacency, get, hop_of, units, views | id, is_present, len, out |
| smysl-graph `merge/mod.rs` | append, contentions, iter | — |
| smysl-graph `merge/retraction.rs` | append, contains_uid, get, relations_of_kind, units | — |
| smysl-graph `merge/contention.rs` | commits_of, contention_status, has_relation, is_live_rebuttal, is_unfounded, iter (label bindings), relations_of_kind, threads, units | — |
| smysl-graph `merge/review.rs` | contains_uid, contention_status, contentions, is_live_rebuttal, is_resolved, is_unfounded, relations_of_kind | — |
| smysl-graph `relink.rs` | get, has_relation, relations_of_kind, units | — |
| smysl-graph `compact.rs` | get, iter, relations, relations_of_kind, units | — |
| smysl-graph `labels.rs` | iter (label bindings) | — |
| smysl-graph `traverse.rs` | — (takes `&Adjacency`) | incoming, len, out |
| smysl-retrieve `lib.rs`, `lexical.rs` | get, units | — |
| smysl-embed `lib.rs` | get, units | — |
| smysl-ingest `stage.rs` | iter + `from_records` (copies the store, §3.7) | — |
| smysl-ingest `attest.rs`, `monotone.rs`, `repair.rs` | units; get; from_records | — |
| smysl-tui `app.rs`, `draw.rs` | contention_status, contentions, get, rebuttals_of, threads, units | — |
| smysl-eval (unpublished) `chain.rs`, `measure.rs`, `prose.rs` | get, rebuttals_of, relations_of_kind, units | — |
| `src/main.rs` (CLI) | adjacency, append, bundle_with, commitment_of, from_records, get, index_path, is_live_rebuttal, is_withdrawn, iter, len, log_bytes, new, open, open_with, reindex, relations, resolutions, resolve_prefix, threads, units, units_with_source_prefix, views, write_index | edge_kind |

**Table B — per method: who calls it outside `store/mod.rs` (non-test), and where it goes.**
"core" = required `StoreRead` method; "prov." = provided (default) method; "W" = `StoreWrite`;
"inh." = stays an inherent method of the in-memory `Store` only.

| method | non-test callers outside `store/mod.rs` | test calls | goes to |
|---|---|---:|---|
| get | 15 consumers (every crate but core/provider) | 20 | core |
| units | 15 consumers | 23 | core |
| contains_uid | lineage, retraction, review, pack, thread | 17 | core |
| relations_of_kind | eval, compact, lineage, contention, retraction, review, relink, pack, thread | 6 | prov. (over `relations`) |
| adjacency | check, cli, lineage, salience, thread | 12 | core |
| iter | check, cli, compact, labels, contention, merge, ingest | 25 | core as `records()`; plus typed accessors (§3.2) |
| relations | check, cli, compact, render | 3 | core |
| contention_status | contention, review, render, tui | 3 | prov. |
| contentions | merge, review, render, tui | 6 | core |
| rebuttals_of | eval, pack, tui | 15 | prov. |
| threads | cli, contention, tui | 7 | core |
| views | check, cli, salience | 2 | core |
| is_live_rebuttal | cli, contention, review | 1 | prov. |
| is_unfounded | contention, review | 2 | core |
| is_withdrawn | cli, render | 2 | core |
| supports_of | lineage, pack | 0 | prov. |
| has_relation | contention, relink | 0 | prov. (over `relation_by_key`) |
| commitment_of | check, cli | 9 | prov. |
| commits_of | contention | 2 | core |
| hop_of | salience | 5 | core (header field, §3.5) |
| observed_at | thread | 2 | core (header field) |
| relation_by_id | check | 4 | core |
| open_contentions | pack | 3 | prov. |
| matching_prefix, resolve_prefix | check (cli: resolve_prefix) | 0 | prov. over a uid range (§3.2) |
| report_dangling | check | 9 | prov. |
| duplicate_records | check | 2 | core |
| withdrawals, resolutions, is_resolved | check; cli; review | 1 | core / core / prov. |
| units_with_source_prefix | cli | 5 | core (indexed in redb) |
| append | cli, merge, retraction | 31 | W |
| from_records, new | cli, ingest | 490 | inh. (constructors) |
| open, open_with, reindex, write_index, index_path, bundle_with, log_bytes, len | cli | 51 | inh. (log-specific); `len` and `log_bytes` also on redb as inherent |
| hops, latest_hop, at_hop, units_in_observed_order, units_at_commitment, attestations_of, attested_by, agreement | tests only (and `store/mod.rs`) | 26 | prov. (cheap to keep; tested API) |
| index, verify_against, state_hash, converged_with, log_hash, log_len, bundle, is_empty | `store/mod.rs` and tests | 59 | inh.; `state_hash`/`record_set_digest` core (§3.2) |
| path | nobody | 0 | inh. |

Of 59 methods, **31 read methods** are called by non-test code outside `store/mod.rs`. The
trait needs 24 required methods (four of them new: `unit_count`, `units_in_range`, `hot_records`, `record_set_digest`) plus 18 provided ones (§4.2).

**Public functions that take `&Store`** (they become generic, §3.2), from a grep of
`crates/*/src` for `&Store` in `pub fn` signatures, single- and multi-line:
`smysl_check::{check, check_and_fail_on, granularity_distribution, fidelity, passes::*::run}`;
`smysl_pack::{pack, verify, warrants_of, violations, fractional, required, required_via, delta,
delta_via, reasons, reasons_via, exact::solve}` and the cache method `required(&mut self,
store: &Store, …)` in `closure.rs`; `smysl_thread::{derive_thread, satisfies_rule_l}`;
`smysl_render::build`; `smysl_retrieve::Bm25::{index, index_with}`; `smysl_embed::Semantic::index`;
`smysl_ingest::{prepare, prepare_declared, prepare_attested, prepare_under, attest,
monotone::apply}`; `smysl_graph::{trace, trace_via, dependents, dependents_via, rests_on, diff,
hop_diff, membership, relink, compact, depth_by_unit, effective_status, report_retractions,
plan_retraction, detect, merge (second argument), review, review_with, salience, view_roots,
label_bindings, labels_of, label_index, resolve_label}`; `smysl_tui::App::new(store: Store)`
and `App::store() -> &Store`; `smysl-eval` (unpublished) has eight.

### 2.7 Semver and tooling facts

- `Makefile`: `BASELINE := 1.8.0`; `PUBLISHED` lists **12 crates** (11 libraries and the
  facade; `smysl-eval` has `publish = false`); `SEMVER_BREAKING :=` is empty, and its comment
  says that after 1.0 an entry "means a 2.0". `make semver` runs `cargo semver-checks
  check-release --baseline-version 1.8.0 --release-type patch -p <crate>` per crate.
- `make api-check` fails on **any** change to `tests/public-api.txt`,
  `tests/public-api-pure.txt` or `tests/public-api-counts.txt`, additions included. Every step
  that adds a public name therefore runs `make api` deliberately and reviews the diff.
- `API_CONTRACT.md`, "Three rules the 1.0 review left behind": an **unsealed** public trait's
  method set is frozen; every method added after 1.0 needs a default, or it is a 2.0.
- `cargo-semver-checks` and `cargo-public-api` are not installed in the environment I verified
  in, so semver claims below about what the tool reports are **(unverified)**.
- ~~`rust-version = "1.79"` is declared; CI uses `stable` and `nightly` only, so the declared
  MSRV is not tested **(observed in `.github/workflows/ci.yml`)**.~~ **This observation was
  right and the consequence was worse than it sounds.** 1.10.0 measured it: the claim was false
  for every crate in the workspace, and the pure core could not be *parsed* at 1.79, because
  `blake3` pulls an edition-2024 `constant_time_eq`. The floors are now declared per crate
  (1.85 base, 1.86 for provider and ingest, 1.88 for tui and the facade) and there is an MSRV
  CI job. OQ-66.
- redb 4.3.0 (2026-09-15) declares `rust-version = 1.90`, edition 2024, MIT OR Apache-2.0. Its
  only normal dependency on Linux and macOS is `libc` (no `cc`, no C toolchain). Its
  multi-process modes are behind the `experimental-multiprocess` feature; by default a writer
  takes a whole-file exclusive lock, read-only handles take a shared lock, and opening against
  a held lock fails with `DatabaseError::DatabaseAlreadyOpen` (`docs/design.md` "Multi-process
  concurrency", `src/tree_store/page_store/file_backend/optimized.rs`). `Durability` has
  `None` and `Immediate`. It exposes `ReadOnlyDatabase`, `Builder::set_cache_size`,
  `check_integrity`, `compact`, and a `StorageBackend` trait with `create_with_backend`.
- `xtask/src/purity.rs`: `smysl-graph` is in `PURE_CRATES`; checks are `cargo tree -p
  <crate> -e normal` against `FORBIDDEN_DEPS` (tokio, ureq, clap, ratatui, crossterm, reqwest,
  hyper, async-std, smol, rustls, serde_json) and a source grep for runtime and socket symbols.
  redb is not on either list.

### 2.8 Measurements today

**Corpora.** `bench-en-*` is draft 3 Appendix E unchanged (25-word `@prose`, `cited`, `doc`
source, no edges). `graph-*` is a variant I added for this RFC: 10-word `@prose` units, one
`@claim` (`derived`, `grounds` on 3 earlier units) per 10 units, and one `backs` relation per
claim; it keeps `SMY-E022` quiet and gives the graph something to traverse. Generator:
`gen_graph.py`, reproduced in §5.4.

**A correction to Appendix E.** `smysl fmt … --format cbor -o out.smy` writes **surface text**:
`cmd_fmt` calls `write_surface` unconditionally and ignores `--format`. Draft 3 table 2.2's
"store" column (5.9 MB, 29.6 MB) is therefore the size of canonical surface, and its `check`
and `find` figures include surface parsing. A CBOR log is produced by `smysl merge in.smy
empty.smy --format cbor -o out.log`. The accepted-and-ignored flag is reported to SMYSL-2.1 as
a defect of the F-13 class; it is not fixed here.

| corpus | units | relations | records | CBOR log | sidecar |
|---|---:|---:|---:|---:|---:|
| bench-en-31102 | 31,102 | 0 | 62,205 | 5.9 MB | 2.5 MB |
| graph-31102 | 34,212 | 3,109 | 71,534 | 4.7 MB | 4.0 MB |
| bench-en-155510 | 155,510 | 0 | 311,021 | 29.7 MB | 12.4 MB |
| graph-155510 | 171,061 | 15,550 | 357,673 | 23.9 MB | 20.0 MB |

(Records exceed units because every labelled unit carries a label-binding record.)

**CLI, end to end, CBOR input** (wall time, peak RSS):

| command | bench 31k | graph 31k | bench 155k | graph 155k |
|---|---:|---:|---:|---:|
| `reindex` (open + reindex + write) | 0.47 s, 95 MB | 0.54 s, 107 MB | 2.69 s, 455 MB | 3.28 s, 517 MB |
| `check` | 0.30 s, 93 MB ¹ | 0.29 s, 103 MB | 1.71 s, 444 MB ¹ | 1.97 s, 497 MB |
| `pack --budget 4000` | 0.39 s, 97 MB | 0.42 s, 108 MB | 2.22 s, 467 MB | 2.55 s, 520 MB |
| `thread --derive analysis` | 0.47 s, 107 MB | 1.09 s, 103 MB | 2.52 s, 464 MB | **28.25 s**, 512 MB |
| `find -n 2 "lord king"` | 1.54 s, 145 MB | 0.90 s, 110 MB | 7.59 s, 674 MB | 4.35 s, 514 MB |
| `salience` | 0.30 s, 79 MB | 0.33 s, 86 MB | 1.82 s, 375 MB | 1.96 s, 413 MB |

¹ exit 3, `SMY-E022` (F-2). With surface input the same `check` takes 0.55 s / 3.14 s and `find`
2.39 s / 20.38 s at 31k / 155k; `fmt` took 0.78 s, 127 MB and 3.51 s, 554 MB, matching draft 3.

**In process, per stage** (`examples/scale_probe.rs` in a scratch clone, release, no default
features; RSS after the stage):

| stage | bench 31k | graph 31k | bench 155k | graph 155k |
|---|---:|---:|---:|---:|
| read file | 3 ms | 3 ms | 13 ms | 13 ms |
| `from_cbor_seq` | 26 ms | 26 ms | 103 ms, 142 MB | 118 ms, 147 MB |
| `Store::from_records` | 195 ms, 90 MB | 231 ms, 97 MB | **1240 ms, 410 MB** | **1506 ms, 450 MB** |
| of which `Adjacency::build` (alone) | 10 ms | 17 ms | 71 ms | 159 ms |
| encode + BLAKE3 every record, once | — | — | 147 ms | 165 ms |
| `index()` (sidecar) | 49 ms | 52 ms | 287 ms | 355 ms |
| `check` | 13 ms | 18 ms | 103 ms | 181 ms |
| `salience` | 18 ms | 22 ms | 121 ms | 166 ms |
| `pack` (budget 4000) | 90 ms | 103 ms | 593 ms | 675 ms |
| `derive_thread` (analysis) | 90 ms | 654 ms | 563 ms | **24,532 ms** |
| `Bm25::index` | 1108 ms | 549 ms | 7059 ms | 3095 ms |
| `Bm25` search | 12 ms | 9 ms | 80 ms | 54 ms |
| `append` 1 record (in memory) | 26 ms | 36 ms | 104 ms | 172 ms |
| `units_in_observed_order` / `units_with_source_prefix` | 1 / 2 ms | 1 / 1 ms | 6 / 10 ms | 7 / 11 ms |

**Where the time goes.**

- **M-1. Loading dominates.** `check` proper is 181 ms of a 1.97 s command at 171k units (9%);
  reading, decoding and `from_records` are ≈1.64 s. Same shape for `pack`, `salience`, `thread`.
- **M-2. `from_records` hashes every record twice.** It hashes each record to deduplicate,
  then `absorb` hashes it again into `record_hashes`; then `log_bytes` re-encodes everything
  once more for `log_hash`. Two encode+hash passes cost ≈300 ms at 155k and the re-encode
  ≈110–120 ms. Adjacency is 71–159 ms. The remaining ≈0.7–0.9 s (by subtraction, not
  profiled) is `canonical_uid` per unit, clones of every unit core, and `BTreeMap` inserts.
- **M-3. Memory is ≈2.6 KB/unit after load** (410 MB / 155,510; 450 MB / 171,061), about 3.5×
  the decoded record vector, partly because every core is held twice (§2.1). Peak during
  load is 15% above that. Extrapolated, 10⁶ units need ≈2.6–3 GB in memory **(extrapolated,
  not run)**.
- **M-4. `derive_thread` is quadratic.** *(Fixed in TX-P0 by SMYSL-2.1 H-18, though **not** by
  the adjacency lookup proposed below: see SMYSL-2.1 §10.2. H-18 indexes the ends of each named
  kind once per derivation, which is equality-based like `relations_of_kind` and so cannot
  mis-handle a non-kernel kind the adjacency folds into `elaborates`. The 171k figures here were
  not re-measured; `crates/smysl-thread/tests/scaling.rs` records its own, and confirms the
  shape: 4.0x per doubling of units before, 2.2x after, 173x at 8000 units, and flat in R. Kept
  here because the measurement is this RFC's.)* `Matcher::SourceOf(k)` / `TargetOf(k)`
  (`crates/smysl-thread/src/derive.rs`, `matches`) call `store.relations_of_kind(k)`, which
  scans and filters every relation and allocates a `Vec`, once per unit in scope and per rule.
  At 171k units × 15.5k relations that is the 24.5 s. Replacing it with a lookup on the
  adjacency (`edge_kind(k)`, then `out_edges`/`in_edges` of the unit) gives **0.55–0.65 s at
  171k and 98–122 ms at 34k, with byte-identical thread records** (BLAKE3 of the encoded
  thread compared before and after on both graph corpora), and `cargo test -p smysl-thread`
  passes. Semantics match: `relations_of_kind` excludes withdrawn edges, and
  `rebuild_adjacency` builds the adjacency without them.
- **M-5. `find` is BM25 construction** (F-10): 7.06 s of 7.59 s at 155k. That is SMYSL-2.4's
  persistent postings; this RFC only makes sure `Bm25::index` can read a `StoreRead`.
- **M-6. Append is `O(store)` per call** (§2.5), 104–172 ms at 155k for one record.
- **M-7. The sidecar never short-circuits a load** (§2.3).

## 3. Design

### 3.1 Three layers

A store at scale is three layers with different access patterns:

| layer | contents | size per unit (est.) | access |
|---|---|---:|---|
| **header** (hot) | uid, status, schema intern, `observed`, newest hop, salience override, flags; adjacency rows | ≈150–200 B | every traversal, every scan that filters by status or time |
| **interpretive** (hot) | relations, rids, withdrawals, resolutions, commitments, recorded contentions, threads, views, label bindings, schema declarations, pack infos, unknown records | proportional to judgements, not to text | every check pass, contention detection, pack constraints |
| **cores** (cold) | `UnitCore` (gist, body, detail, payload, source, extra), attestations | ≈1–3 KB | `get`, full scans, rendering |

The estimate for the header is arithmetic, not a measurement: 32 B uid, ≈36 B for the uid →
`NodeId` map entry (or 0 if replaced by a binary search over the sorted uid vector), 1 B
presence, 2 × 24 B for the edge-list headers, 8 B per edge each way, ≈24 B of fixed fields. At
10⁶ units that is ≈150–200 MB, against ≈2.6–3 GB fully in memory (M-3). Step P13b measures it.

Draft 3 asks that "the interpretive layer [stay] hot". This RFC reads that as the second
row, plus the cores of interpretive units (§3.5.4).

### 3.2 `StoreRead`

**Where:** `crates/smysl-graph/src/store/read.rs`, re-exported as `smysl_graph::StoreRead`
and from the facade.

**Method set.** From Table B: everything non-test code calls, nothing more. Required methods
are the ones an implementation must answer from its own storage; provided methods are
compositions that every implementation would otherwise copy. 24 required, 18 provided (§4.2).

**Sealed.** `StoreRead: sealed::Sealed`, with the `Sealed` trait private to `smysl-graph`.
Reasons:
1. `API_CONTRACT.md` freezes the method set of an unsealed public trait. This trait will grow
   (record-set digest, time ranges, per-part lookups from SMYSL-2.4) and must be able to grow
   in a minor version. Adding a method to a sealed trait does not break a downstream crate,
   because no downstream crate can implement it **(cargo-semver-checks' treatment of sealed
   traits: unverified here)**.
2. Every implementation this RFC needs (in-memory, redb, union, overlay) lives in
   `smysl-graph`. A third-party backend is not a requirement. If one becomes one, unsealing is
   additive.

**Generic, not object-safe.** Consumers become `fn f<S: StoreRead + ?Sized>(store: &S, …)`.
Reasons:
1. Units must come back by reference from the in-memory store and by owned handle from the
   paged one. That needs a generic associated type (`type UnitRef<'a>: Deref<Target = Unit>`),
   and a trait with GATs is not object-safe.
2. Iterators as associated types cost nothing for `Store`; `Box<dyn Iterator>` would add an
   allocation and a virtual call per element in the hottest loops (every check pass iterates
   `units()`).
3. Runtime choice of backend (the CLI) does not need `dyn`: an enum `AnyStore` that implements
   `StoreRead` by `match` is enough, and stays inside `smysl-graph` because the trait is sealed.

**Associated types.**
- `UnitRef<'a>: Deref<Target = Unit> + Clone` — `&'a Unit` for `Store`, `Arc<Unit>` for paged
  stores. `Arc` because a cache can then evict while a caller still holds a unit.
- `Units<'a>: Iterator<Item = (&'a Uid, UnitRef<'a>)>` — in **ascending uid order**, always.
  For `Store` this is `btree_map::Iter<'a, Uid, Unit>`, so `units()` keeps its exact item type.
  For a paged store the `&'a Uid` borrows from the hot header's sorted uid vector.
- `Records<'a>: Iterator<Item = Cow<'a, Record>>` — the log in log order.

**Hot accessors are concrete.** The interpretive layer is one crate-private struct shared by
every implementation (§4.1), so `relations()`, `threads()`, `views()`, `contentions()`,
`commits_of()` and the rest return plain references and public iterator newtypes.

**Typed record accessors.** `iter()` is used for four things (Table A): schema declarations
and unknown records (`extension.rs`), label bindings (`labels.rs`, `contention.rs`), and whole
log copies (`merge`, `compact`, `stage`, CLI). The first three get hot accessors
(`schema_decls()`, `label_bindings()`, `unknown_records()`, `pack_infos()`), so a check pass
on a paged store does not stream the log. `records()` remains for whole-log work.

**Adjacency.** `fn adjacency(&self) -> &Adjacency` stays, returning the existing type.
- Dense ids are a determinism property (§2.4). Every implementation must assign them in
  ascending uid order over every uid mentioned, present or not, and sort edge lists by
  `(kind, target)`. The conformance suite compares adjacencies field by field (§5.1).
- Paged stores build `Adjacency` at open from the header and relation tables. A new
  crate-private constructor, `Adjacency::from_parts`, takes sorted uids and edges; `build`
  keeps its signature and calls it.
- Extension kinds are interned per store. In a union, interning is over the union's kinds;
  `EdgeKind::Extension(i)` is therefore not comparable across a shard and the union, which
  `adjacency.rs` already says about stores.

**Iteration order is uid order**, for `units()`, `matching_prefix`, `units_with_source_prefix`
and every provided method that returns `Vec<Uid>`. redb compares `[u8; 32]` keys bytewise, and
`Uid`'s derived `Ord` over `[u8; 32]` is bytewise too (`crates/smysl-core/src/ids.rs`), so redb
range scans come back in uid order with no sort. `matching_prefix` becomes a range scan in
every implementation (today it filters every key).

**Blanket impls.** `impl<S: StoreRead + ?Sized> StoreRead for &S`, `&mut S`, `Box<S>`,
`Rc<S>`, `Arc<S>`. These keep `check(&arc_store, …)` compiling once `check` is generic: today
`&Arc<Store>` coerces to `&Store`; a generic parameter infers `S = Arc<Store>` instead, and
without the blanket impl that would fail. The residual break is a caller's own type that
`Deref`s to `Store` (§7, OQ-65).

**No `Sync` supertrait.** `Rc<S>` must be able to implement it. Instead a test asserts `Send +
Sync` for `Store`, `RedbStore` and `Union<'_, Store>` (§5.1).

### 3.3 `StoreWrite`

```rust
pub trait StoreWrite: StoreRead + sealed::Sealed {
    fn append(&mut self, records: &[Record]) -> Result<AppendReport, Error>;
}
```

Implemented by `Store` (in memory and log-backed) and `RedbShard` (a single shard opened for
writing). `Union` and `Overlay` are read-only. `merge` keeps `store: &mut Store` as its first
argument in this RFC (it is one of the twelve contract operations) and generalises only the
second, `other: &S where S: StoreRead + ?Sized`. A `merge_into<W: StoreWrite>` is added later
only if a caller needs it.

### 3.4 The in-memory `Store` stays the reference

`Store` keeps every inherent method and signature. Changes are internal:

1. **Lazy derived state.** `adjacency` and `unfounded` move into one
   `OnceLock<Derived>`; `append(&mut self)` resets it (`self.derived = OnceLock::new()`),
   and readers call `get_or_init`. `Store` stays `Send + Sync` (§2.5). The pending-attestation
   retry stays eager: it is `O(pending)` and changes `units`, which readers see directly.
   Effect: an append is `O(new)` again; the first read after a run of appends pays one
   rebuild (71–159 ms at 155k). The 1.8 table on `Store::append` is re-measured and rewritten.
   `#[derive(Clone)]` still works (`OnceLock: Clone` when `T: Clone`).
2. **One hash per record in `from_records`.** Deduplicate on the hash and pass the hash to
   `absorb`, which today recomputes it (M-2). Compute `log_hash` from the encodings already
   made. Expected saving ≈150–250 ms at 155k **(estimate)**.
3. **Interpretive fields grouped** into the crate-private `Interpretive` struct (§4.1), plus
   four new hot vectors: schema declarations, label bindings, pack infos, unknown records, in
   log order. `iter()` still walks `records`.
4. The duplicate core (`records` and `units`) stays. `Record::Unit(UnitCore)` and
   `Unit.core: UnitCore` are public fields; sharing one allocation would change public types.
   The paged store does not have the duplication, which is the point of it.

### 3.5 The redb-backed store

#### 3.5.1 Placement

**A module of `smysl-graph`, `store::redb`, behind feature `store-redb` (off by default).**
Not a new crate, and not a module of `smysl-text`:
- the trait is sealed, so implementations live in `smysl-graph`;
- `smysl-text` (SMYSL-2.4) depends on `smysl-graph`, not the other way round, and the unit
  store is not text-specific (design rule 7: telemetry shards use it too);
- `smysl-graph` already does file IO (`Store::open`, `append`, `write_index`).

`redb` becomes a `[workspace.dependencies]` entry shared with `smysl-text`'s `substrate-redb`
feature, so the two files are written by one redb version.

#### 3.5.2 Files

Within the library layout of draft 3 §14.2 / SMYSL-2.4:

```
<library>/
  log/<shard>.smy          CBOR log, authoritative (unchanged)
  store/<shard>.redb       unit store for that shard (this RFC); derived in index mode
  store/<shard>.lock       single-writer lock file (§3.8)
  index.redb               substrate index (SMYSL-2.4), plus the `uid_shard` table (§3.6)
```

One redb file per shard rather than tables in `index.redb`: redb's default locking is per
file, so a shared file would make the whole library single-writer; and a shard must be
rebuildable and discardable on its own (draft 3 §14.4).

A standalone store (no library) is `x.smy` plus `.smysl/store/x.redb`, beside the existing
`.smysl/index/x.idx` (`Store::index_path`).

*As built (P13c.3, OQ-90): one store for the whole library, `index/units.redb`*, beside the
substrate's `index/substrate.redb`, describing every shard's log; not a store per shard. P13c.1
put one beside each log, and GE-T7's stand-in showed what that cost at a thousand shards: each
store's fixed part paid a thousand times, and a `Union` over them holding its own uid list,
layers and adjacency on top of theirs — 1.19 GB to open where one store of the same units opens
in 277 MB, and 3.5 GB on disk for 164 MB of logs, where one store is 540 MB. The argument above
for a file per shard was per-file locking, and in index mode it does not bind: the library's
writer is the library, which writes the shards' logs under their own lock files, and the unit
store folds what they gained the next time it is opened. What one file costs is that two
processes cannot both have the store open — which redb 2.6 already refused per file — and the
second reads the logs into memory after `reader_wait_ms` (`SMY-W504`), as for one log.

#### 3.5.3 Tables

All keys big-endian; all values deterministic CBOR or fixed-width. `seq` is the record's
ordinal in the log.

*As built (P13c.3): layout 2, every row naming its log.* A `logs` table maps a log's name to
its position and the length, hash and record count the store describes of it; `record_hash`
is keyed `(hash, log)`, `attest` `(subject, hash, log)`, `interp` `(log, seq)`; a header names
the log its core is read from (two of its spare bytes); `holds` lists `(uid, log)` for every
log holding a unit. A log is folded on its own, so one that grows moves no other log's rows,
and what the store answers is decided when it is read: over several logs, the logs in name
order and each record once (`Store::from_records`, which drops repeats); over one, as
`Store::open`, repeats kept. Opened for some logs, the store reads only their headers,
interpretive rows and attestations, and `holds` says which unopened log a dangling reference
waits on (`SMY-W500`), so the redb path needs no `Union` and no uid index of its own. A layout-1
store is rebuilt (`SMY-W501`).

| table | key → value | layer | notes |
|---|---|---|---|
| `meta` | `&str` → bytes | — | `layout` (u16, starts at 1), `mode` (`index`/`primary`), `log_len`, `log_hash`, `record_set_digest`, `shard_id`, counts |
| `log` | `seq: u64` → `(offset: u64, len: u32, type: u8)` in index mode; CBOR record bytes in primary mode | cold | `records()` streams this |
| `record_hash` | `[u8; 32]` → `seq` | cold | dedup for `append`; replaces the in-memory `BTreeSet` |
| `core` | `uid` → `seq` (index) / CBOR `UnitCore` (primary) | cold | `get` |
| `header` | `uid` → 24-byte fixed record | hot | status, schema intern, flags, `observed` (u64, `u64::MAX` = none), newest hop, salience quantised as in the sidecar |
| `deps` | `(uid, EdgeKind code, target uid)` → `()` | hot | deps and grounds of each core, so adjacency builds without reading cores |
| `attest` | `(uid or rid, record hash)` → CBOR attestation | cold | joined into `Unit` on `get` |
| `interp` | `seq` → CBOR record | hot | every non-unit, non-attestation record: relations, withdrawals, resolutions, commits, contentions, threads, views, label bindings, schema declarations, pack infos, unknown |
| `schema_intern` | `u16` → schema id string | hot | |
| `observed` | `(observed_ms: u64, uid)` → `()` | index | `units_in_observed_order` becomes a scan (F-15) |
| `source_ref` | `(reference bytes, uid)` → `()` | index | `units_with_source_prefix` becomes a range scan |

Open loads `header`, `deps`, `interp` and `schema_intern` into memory, builds `Adjacency`
with `from_parts`, and derives `unfounded`. Cores and attestations stay on disk.

#### 3.5.4 Tiering and caching

- **Pinned:** cores of units that are relation endpoints or whose status is `speculative`,
  `inferred` or `derived` — claims, findings and hypotheses, which every interpretive
  operation reads. Loaded on first touch, never evicted.
- **Paged:** cores of `measured` and `cited` units (text extractions, telemetry readings).
  An LRU of `Arc<Unit>` bounded by a byte budget (`RedbOptions::core_cache_bytes`, default
  64 MiB), on top of redb's page cache (`Builder::set_cache_size`). *As built (P13b.1): 32 MiB,
  scan-resistant — scans read through and `get` fills — both measured; §6.1.*
- The policy is a function of the header only, so it is deterministic and does not change
  answers, only latency. Which statuses are pinned is OQ-69.

Answers never depend on cache state: the conformance suite runs every operation with the
cache at 0 bytes and at "everything fits" (§5.1).

#### 3.5.5 Modes and authority

- **Index mode** (default, until GE-T15 passes). The CBOR log is the authority, exactly as for
  the sidecar today. `meta.log_len`/`log_hash` are compared at open; a mismatch is a rebuild
  from the log, reported as `SMY-W501`. Deleting the `.redb` loses nothing.
- **Primary mode** (opt-in, after GE-T15). The redb file holds record bytes and the CBOR log
  is no longer written on append. `lib export` (SMYSL-2.7) is the escape hatch and the
  conformance oracle: `export → import into an empty Store` must reproduce the redb store's
  record-set digest. A mismatch on `lib verify` is `SMY-E505`.

The order of writes in index mode is **log first, then redb**: append the CBOR bytes and
`fsync`, then commit a redb transaction (`Durability::Immediate`) carrying the new
`log_len`/`log_hash`. After a crash, redb is either current or behind, never ahead; behind
means replaying the log tail from `meta.log_len`, not a full rebuild. A log tail with trailing
bytes is handled as today (`W110` with the byte count).

#### 3.5.6 Determinism

- `units()` and every `Vec<Uid>` answer are in uid order (§3.2).
- `records()` is in `seq` order, which is log order.
- Interned schema ids and extension kinds are assigned by sorting, not by arrival.
- The redb file itself is **not** byte-reproducible (page allocation depends on history).
  Nothing hashes it; what is compared is the record-set digest and every operation's output.

### 3.6 Shards and the catalog

From draft 3 §14.4, unchanged in intent:
- **One shard per expression**: a translation, a book, a chat channel, a telemetry source.
  A shard is `log/<shard>.smy` plus `store/<shard>.redb`.
- **One catalog shard**: manifests, `x.text/meta`, catalog entities (SMYSL-2.4).
- **Shard ids** are the aliases SMYSL-2.4 assigns; this RFC treats them as opaque strings.

`index.redb` gains one derived table, `uid_shard: uid → [shard id]` (a multimap, because the
same unit can be extracted into two shards and is then one unit by content). It answers
"where would this dangling uid resolve?" without opening every shard. It is rebuilt from the
shards' `header` tables. Its home (here or per-shard) is OQ-68.

*As built (P13c.1):* the library's shards are `log/catalog.cbor` and `log/shards/<alias>.cbor`
(an alias's `/` and `:` spelled `~` and `+`, which A-3 does not allow, so the mapping is
one-to-one); `uid_shard` is `smysl_graph::UidShards`, kept per shard as
`index/shards/<alias>.uids` — each shard's uids, stamped with the length of the log they list —
and merged on read (OQ-68). Which records go to which shard is SMYSL-2.4's routing, in
`smysl_text::shards`: an expression's units, and the relations, attestations, commitments and
withdrawals on them, go to its shard; everything else to the catalog.

### 3.7 The virtual union

`Union<'s, S: StoreRead>` borrows a set of shards and is itself a `StoreRead`.

- **Well-defined because merge is a join-semilattice** (rule U; `Store::absorb` is
  order-independent by construction, `merge/mod.rs`). The union of shards A, B, C answers as
  `Store::from_records(records(A) ∪ records(B) ∪ records(C))` would, whatever order the shards
  are listed in. That equality is the union's conformance test (§5.1).
- **Interpretive layer: materialised.** At construction the union absorbs every shard's hot
  records into one `Interpretive` (relations, withdrawals, commitments, labels, …), using the
  same fold as `absorb`. That is proportional to judgements, not to text.
- **Headers and adjacency: materialised.** The union's uid set is a k-way merge of the
  shards' sorted uid vectors; dense ids are positions in it, so they are ascending-uid as
  everywhere else. Edges are the union of shard edges plus relation edges.
- **Cores: virtual.** `get(uid)` asks the shards in a fixed order (sorted by shard id) and
  takes the first core found. A core is content-addressed, so any shard's copy is the same
  bytes. **Attestations** on the same uid in two shards are unioned, so `UnitRef` for such a
  uid is an owned `Arc<Unit>` built from both.
- **`units()`** is the k-way merge of shard iterators by uid, deduplicating equal uids.

**Waiting edges.** An edge whose endpoint is in a shard not in the union is a dangling node
(`present = false`) like any other. `report_dangling` on a union consults `uid_shard`: if the
uid lives in an unloaded shard, the diagnostic is `SMY-W500` ("waits on shard S") instead of
`SMY-E060`. Loading that shard into the union resolves it. Nothing is written. This is the
cross-shard counterpart of `pending_attestations`.

*As built (P13c.1):* `Union<S: StoreRead>` **owns** its shards rather than borrowing them, so that
`AnyStore` can hold one (`AnyStore::Shards`, for `--shard`); a caller keeping its stores passes
references, since `&Store` is a store. The equality is with `from_records` of the shards'
records concatenated **in shard-id order**: most answers do not depend on that order, and the
four that do — the log, the records of a `HotKind`, recorded contentions (first by id) and views
(last by id) — follow it, so a union is a function of which shards it holds. Materialised at
construction: the uid list with the first shard holding each, the interpretive and library
layers (the shards' own, joined), the hot records, and a merged copy of each unit two shards
hold or that an attestation in another shard lands on. The adjacency is derived lazily from the
shards' own adjacencies (their `deps`/`grounds` edges) and the union's relations, so it reads
no core. `with_index(Arc<UidShards>)` turns `E060` into `W500` for a uid the index places in a
shard not loaded.

**Contentions over a union (OQ-16).** Detection is already "reported, not recorded"
(`merge/mod.rs`, the comment after `new_contentions`): `contention::detect` is a function of
the store, re-run on every merge. Its four detectors read `relations_of_kind(Supersedes)`,
live rebuttals, label bindings and commitments — all in the interpretive layer, which the
union materialises. So **lazy per-query detection is the proposal**: `detect(&union, ctx)` at
query time, cached on the union instance. A materialised contention index is added only if
GE-T7 shows detection over the union's interpretive layer above 1 s at 10⁶ units. Position on
OQ-16 recorded in §9.

*As built (P13c.2):* two of the four detectors did not read only the interpretive layer.
Commitment forks walked every unit to ask each for its commitments, and label collisions read
every unit's own `labels` — which a stored unit never has, every store building its units from
their cores, and labels having no wire record. On a store that pages its cores both were a scan
of every core to find nothing. Commitment forks now walk the commitments, which the layer keys
by unit; the unit loop is gone from label collisions; and live rebuttals build each unit's
threads once instead of asking every thread about every rebuttal. `Union::detected()` is the
cache: `detect` with no labels and no clock, run on the first call and kept. GE-T7's stand-in
measured detection at 10⁶ units under the threshold on every implementation (§6.1, P13c.2), so
there is no contention index.

**`Overlay<'s, S>`** is a union of one store and an in-memory batch. `smysl_ingest::stage`
currently copies every record of the store into a new `Store` to check a batch against the
union (`stage.rs`, `store.iter().cloned().collect()` then `Store::from_records`); `relink` in
the CLI does the same. With `Overlay` that check costs the batch plus the interpretive layer,
not the store. Its conformance test: `check(&Overlay::new(&s, batch))` produces the same
report as `check(&Store::from_records(s.records ++ batch))`.

### 3.8 Concurrency

D-10 assigns the library lock to SMYSL-2.4 and asks this RFC to revisit it for redb.

- **One writer per shard.** A writer creates `store/<shard>.lock` with `create_new`, holding it
  for the process's write session, as SMYSL-2.4's library lock does. **Not an advisory OS lock**:
  OQ-36 was answered in 1.10.0 against them, and this paragraph said "advisory" while that
  question was still open. A second writer fails fast with `SMY-E445` (SMYSL-2.4's "shard locked
  by another writer"; one meaning, one code), naming the holder's pid, host and command — which
  the lock file records, and which is half the reason the answer went that way, since the kernel
  will not tell you who holds a flock. *As built (P13b.2): `.smysl/store/<name>.lock` for a
  standalone log; the `Lock` type moved from `smysl-text` to `smysl_graph::lock` so the writer
  below it can take one, and is now written whole and linked into place — created and then
  written, a racing writer read an empty lock and was refused by nobody (§6.1). Within one
  process every open of a store shares one database, because redb 2.6's `flock` conflicts with
  a second open in the same process as with another's.*
- **Readers in the same process**: redb read transactions are MVCC snapshots. A `RedbStore`
  opened for reading holds one read transaction for its lifetime, so every answer it gives is
  from one snapshot (one moment, in `NodeId`'s sense). Writers in the same process do not
  block it.
- **Readers in another process**: with redb's default locking, a read-only open fails while a
  writer holds the file. The reader retries for a bounded time (default 2 s), then **reads the
  CBOR log directly** (the authority; already readable mid-append, `Store::open_with`) and
  builds an in-memory `Store`, with `SMY-W504`. Correct, slower, and never blocking forever.
- **Writers keep sessions short**: the CLI opens the shard read-write only for the duration of
  one `append`/`merge`, so the fallback is rare in practice.
- **Library-wide operations** (rebuilding `uid_shard`, adding a shard) take SMYSL-2.4's
  library lock.
- When redb's multi-process mode leaves `experimental-multiprocess`, single-writer /
  many-reader across processes becomes possible without the fallback (OQ-67).

### 3.9 Migration for existing users

1. **1.x as shipped**: nothing changes. `Store` keeps every signature; the CBOR log and the
   sidecar are untouched.
2. **P13a–P13c**: `smysl` commands accept `--backend memory|redb` (default `memory`). With
   `redb`, the first open builds `.smysl/store/<name>.redb` from the log (`W501` once). Delete
   it at will.
3. **After GE-T15 passes** (SMYSL-2.7): `smysl lib migrate --primary <shard>` switches a shard
   to primary mode, after an export round trip that must match the record-set digest. `smysl
   lib export` turns it back into a CBOR log at any time. The default stays index mode until
   GE-T7 has run on primary shards too.
4. A store written by a later `layout` version is refused with `SMY-E503`; in index mode the
   remedy (delete and rebuild) is printed with it.

## 4. Implementation plan

### 4.1 Crates and modules

| path | new / changed | content |
|---|---|---|
| `crates/smysl-graph/src/store/read.rs` | new | `StoreRead`, `StoreWrite`, `sealed`, blanket impls, iterator newtypes |
| `crates/smysl-graph/src/store/interp.rs` | new | `pub(crate) struct Interpretive` (hot layer, shared), its `absorb_hot` fold |
| `crates/smysl-graph/src/store/mod.rs` | changed | fields regrouped; lazy `Derived`; single hashing; `impl StoreRead/StoreWrite for Store` |
| `crates/smysl-graph/src/store/union.rs` | new | `Union`, `Overlay` |
| `crates/smysl-graph/src/store/any.rs` | new | `AnyStore` enum for runtime backend choice |
| `crates/smysl-graph/src/store/redb/{mod,tables,cache,open}.rs` | new, feature `store-redb` | `RedbStore`, `RedbShard`, `RedbOptions` |
| `crates/smysl-graph/src/adjacency.rs` | changed | `pub(crate) fn from_parts`; `build` delegates |
| `crates/smysl-graph/tests/store_conformance.rs` | new | §5.1 |
| consumer crates | changed | §4.3 |

### 4.2 Public API (sketch)

The trait skeleton below was compiled against `d25ec9e` in a scratch example (with `Store`
implementing it by delegation, a generic consumer, and a `Send + Sync` assertion); the full
method list is sketch level.

```rust
// smysl-graph/src/store/read.rs
mod sealed { pub trait Sealed {} }

pub trait StoreRead: sealed::Sealed {
    type UnitRef<'a>: Deref<Target = Unit> + Clone + 'a where Self: 'a;
    type Units<'a>: Iterator<Item = (&'a Uid, Self::UnitRef<'a>)> + 'a where Self: 'a;
    type Records<'a>: Iterator<Item = Cow<'a, Record>> + 'a where Self: 'a;

    // -- required: units and headers
    fn get(&self, uid: &Uid) -> Option<Self::UnitRef<'_>>;
    fn contains_uid(&self, uid: &Uid) -> bool;
    fn units(&self) -> Self::Units<'_>;                       // ascending uid
    fn unit_count(&self) -> usize;
    fn units_in_range(&self, lo: &Uid, hi: &Uid) -> Vec<Uid>; // inclusive, ascending
    fn observed_at(&self, uid: &Uid) -> Option<u64>;
    fn hop_of(&self, uid: &Uid) -> Option<u32>;
    fn units_with_source_prefix(&self, prefix: &str) -> Vec<Uid>;
    fn units_in_observed_order(&self) -> Vec<Uid>;
    // -- required: graph and interpretive layer (hot in every implementation)
    fn adjacency(&self) -> &Adjacency;
    fn relations(&self) -> Relations<'_>;                     // canonical (kind, from, to) order
    fn relation_by_id(&self, rid: &Uid) -> Option<&Relation>;
    fn threads(&self) -> Threads<'_>;
    fn views(&self) -> Views<'_>;
    fn contentions(&self) -> &[Contention];
    fn withdrawals(&self) -> Withdrawals<'_>;
    fn resolutions(&self) -> Resolutions<'_>;
    fn commits_of(&self, uid: &Uid) -> &BTreeSet<Commit>;
    fn is_withdrawn(&self, rel: &Relation) -> bool;
    fn is_unfounded(&self, uid: &Uid) -> bool;
    fn hot_records(&self, kind: HotKind) -> HotRecords<'_>;   // schema decls, label bindings, pack infos, unknown
    // -- required: log
    fn records(&self) -> Self::Records<'_>;                   // log order
    fn duplicate_records(&self) -> usize;
    fn record_set_digest(&self) -> [u8; 32];                  // lands with SMYSL-2.1 (TX-P0)

    // -- provided (bodies move from store/mod.rs unchanged in meaning)
    fn relations_of_kind(&self, kind: &RelKind) -> Vec<&Relation> { /* filter relations() */ }
    fn has_relation(&self, kind: &RelKind, from: &Uid, to: &Uid) -> bool { /* … */ }
    fn is_live_rebuttal(&self, rel: &Relation) -> bool { /* … */ }
    fn rebuttals_of(&self, uid: &Uid) -> Vec<Uid> { /* traverse::rebuttals_of on adjacency() */ }
    fn supports_of(&self, uid: &Uid, edges: &EdgeSet) -> Vec<Uid> { /* lineage::one_hop */ }
    fn is_resolved(&self, target: &ResolutionTarget) -> bool { /* … */ }
    fn contention_status(&self, c: &Contention) -> ContentionStatus { /* … */ }
    fn open_contentions(&self) -> Vec<&Contention> { /* … */ }
    fn commitment_of(&self, uid: &Uid) -> Option<Commitment> { /* … */ }
    fn units_at_commitment(&self, level: Commitment) -> Vec<Uid> { /* … */ }
    fn matching_prefix(&self, p: &UidPrefix) -> Vec<Uid> { /* units_in_range(lo(p), hi(p)) */ }
    fn resolve_prefix(&self, p: &UidPrefix) -> Result<Uid, IntegrityError> { /* E072 */ }
    fn report_dangling(&self, report: &mut Report) { /* E060 */ }
    fn attestations_of(&self, uid: &Uid) -> Vec<Attestation> { /* … */ }
    fn attested_by(&self, uid: &Uid) -> BTreeSet<AgentId> { /* … */ }
    fn agreement(&self, uid: &Uid, n: usize) -> bool { /* … */ }
    fn hops(&self) -> BTreeSet<u32> { /* … */ }
    fn latest_hop(&self) -> Option<u32> { /* … */ }
}

pub trait StoreWrite: StoreRead {
    fn append(&mut self, records: &[Record]) -> Result<AppendReport, Error>;
}

impl<S: StoreRead + ?Sized> StoreRead for &S { /* delegate */ }   // also &mut S, Box, Rc, Arc

pub struct Union<'s, S: StoreRead> { /* shards, Interpretive, header, OnceLock<Derived> */ }
impl<'s, S: StoreRead> Union<'s, S> {
    pub fn new(shards: impl IntoIterator<Item = (ShardId, &'s S)>) -> Union<'s, S>;
    pub fn shards(&self) -> impl Iterator<Item = &ShardId>;
}
pub struct Overlay<'s, S: StoreRead> { /* base + Store of the batch */ }
impl<'s, S: StoreRead> Overlay<'s, S> {
    pub fn new(base: &'s S, batch: Vec<Record>) -> Overlay<'s, S>;
}

#[non_exhaustive]
pub enum AnyStore { Memory(Store), #[cfg(feature = "store-redb")] Redb(RedbStore) }

#[cfg(feature = "store-redb")]
pub mod redb {
    #[derive(Debug, Clone, Default)] #[non_exhaustive]
    pub struct RedbOptions { pub core_cache_bytes: usize, pub page_cache_bytes: usize,
                             pub reader_wait_ms: u64, pub verify_hashes: bool }
    pub struct RedbStore { /* ReadTransaction, Interpretive, headers, OnceLock<Derived>, LRU */ }
    impl RedbStore {
        pub fn open(log: &Path, opts: RedbOptions) -> Result<(RedbStore, OpenReport), Error>;
        pub fn len(&self) -> usize;          // records, as Store::len
        pub fn log_bytes(&self) -> Vec<u8>;
    }
    pub struct RedbShard { /* Database, lock file, RedbStore view */ }
    impl RedbShard {
        pub fn open_rw(log: &Path, opts: RedbOptions) -> Result<(RedbShard, OpenReport), Error>;
    }
}
```

Notes:
- `attestations_of` returns owned `Vec` in the trait: the in-memory store returns
  `&BTreeSet` today, and keeps that inherent signature. Generic callers (tests only today) get
  the owned form.
- `open_contentions` returns `Vec<&Contention>` in the trait; the inherent method keeps `impl
  Iterator`.
- `HotKind` is a `#[non_exhaustive]` enum; `HotRecords` iterates `&Record` in log order.
- Inherent methods of the same name keep their exact signatures on `Store`. Inherent methods
  win method resolution, so existing code calling them on a concrete `Store` is unaffected.

### 4.3 Changes to existing crates, in dependency order

Each line is one step of §6. Order follows the crate graph (`smysl-graph` ← `check`, `pack`,
`thread`, `retrieve` ← `render` (needs thread), `embed` (needs retrieve), `ingest` (needs
check), `tui` (needs pack, thread, render), `eval`, facade).

| # | file(s) | change |
|---|---|---|
| 1 | `smysl-graph/src/store/{read,interp,mod}.rs`, `adjacency.rs` | trait, `impl for Store`, regrouping, lazy derived state, single hashing |
| 2 | `smysl-graph/src/labels.rs` | `label_bindings`, `labels_of`, `label_index`, `resolve_label` generic; read `hot_records(LabelBindings)` instead of scanning the log |
| 3 | `smysl-graph/src/lineage.rs` | `trace`, `trace_via`, `dependents`, `dependents_via`, `rests_on`, `hop_diff`, `membership` generic; `diff(a, b)` with two parameters `A, B` |
| 4 | `smysl-graph/src/salience.rs` | `salience`, `view_roots` generic |
| 5 | `smysl-graph/src/merge/{retraction,contention,review,mod}.rs` | `effective_status`, `report_retractions`, `plan_retraction`, `detect`, `review`, `review_with` generic; `merge`'s second argument generic; `contention.rs` label bindings from hot records |
| 6 | `smysl-graph/src/{relink,compact}.rs` | generic over `S` |
| 7 | `smysl-check/src/lib.rs`, `passes/*.rs` | every `run(store: &Store, …)` and `check`, `check_and_fail_on`, `granularity_distribution`, `fidelity` generic; `extension.rs` reads `hot_records(SchemaDecls / Unknown)` |
| 8 | `smysl-pack/src/{solve,closure,constraints,bound,exact}.rs` | `pack`, `verify` and helpers generic; the cache's `required(&mut self, store: &S, …)` generic per call |
| 9 | `smysl-thread/src/derive.rs` | `derive_thread`, `satisfies_rule_l` generic; the M-4 fix itself ships earlier, in SMYSL-2.1 H-18 (TX-P0); this port keeps it |
| 10 | `smysl-retrieve/src/{lib,lexical}.rs`, `smysl-embed/src/lib.rs` | `Bm25::index`, `index_with`, `Semantic::index` generic |
| 11 | `smysl-render/src/ir.rs` | `build` generic |
| 12 | `smysl-ingest/src/{stage,attest,monotone,lib}.rs` | `prepare*`, `attest`, `apply` generic; `stage` uses `Overlay` instead of `from_records` over a copy |
| 13 | `smysl-tui/src/app.rs` | `App` keeps `Store`; adds `App::with_store(AnyStore)` (additive); `store()` keeps returning `&Store` for `Memory` |
| 14 | `smysl-eval/src/*.rs` | generic (unpublished; no semver gate) |
| 15 | `src/lib.rs`, `src/main.rs` | facade re-exports `StoreRead`, `StoreWrite`, `Union`, `Overlay`, `AnyStore` (and `redb::*` under the facade feature); CLI loads through `AnyStore` |

`traverse.rs` takes `&Adjacency` already and does not change.

### 4.4 CLI

Integration with the `Cmd` table in `src/main.rs`; no new subcommand in this RFC except under
`lib` (SMYSL-2.4 owns `lib`).

- Global `--backend <memory|redb>` (default `memory`), honoured by `load_store`, which returns
  `AnyStore`. Surface input and stdin always load into memory. *As built (P13b.2): also by
  `check`, which reads its own input, and by every command's write, which goes through
  `RedbShard`; a build without `store-redb` refuses `redb` with exit 2, by name.*
- Global `--shard <ALIAS>` (repeatable) on read commands inside a library: the union of the
  named shards; `--shard all` is every shard. *As built (P13c.1): each shard through the backend
  `--backend` chose; under `--backend redb` a library is always read as a union of its shards,
  each through its own unit store. A union of some shards reads the library's uid lists for
  `W500`. Without either flag a library is read as one log, every shard's records in shard-id
  order. A write under `--shard` is refused.* *As built (P13c.3): under `--backend redb` a
  library is the one unit store at `index/units.redb`, opened for the shards named; the memory
  backend keeps the union.*
- `smysl reindex --backend redb` builds or rebuilds the unit store; `--verify` compares the
  redb store's record-set digest and adjacency against an in-memory rebuild.
- `smysl lib migrate --primary <shard>` and `--index <shard>` (§3.9), registered by SMYSL-2.4's
  `lib` table; exit 9 (`HashVerification`) on a digest mismatch.
- `tests/cli-surface.txt` is regenerated (`make cli-surface`) in the step that adds the flags,
  and `COMMAND_NAMES` in the Makefile is unchanged.

### 4.5 Features, dependencies, purity, toolchain

- `smysl-graph`: feature `store-redb = ["dep:redb"]`. Facade: feature `store-redb =
  ["smysl-graph/store-redb"]`, not in `default`.
- `[workspace.dependencies] redb = { version = "4", default-features = false, features =
  ["std"] }` — shared with SMYSL-2.4's `substrate-redb`. *As built (P13b.1): the existing
  `=2.6.4` pin, which declares 1.85; 4.x's 1.90 would have raised every crate that depends on
  `smysl-graph`, which is all of them. §6.1 and OQ-67 record what 2.6 does not have.*
- **Purity gate**: redb is synchronous, opens no sockets and links no runtime; it is not in
  `FORBIDDEN_DEPS` and the source grep finds nothing. `cargo tree -p smysl-graph -e normal`
  (default features) is unchanged. `make purity` must pass with `--features store-redb` added
  to the pure-crate tree check (a one-line extension of `xtask/src/purity.rs`).
- **No C toolchain**: redb 4.3's normal dependencies are `libc` on Linux/macOS/WASI and three
  optional crates not enabled here (§2.7). `cargo tree -e normal --features store-redb | grep
  -E '^.*\bcc\b'` is part of the step's exit test.
- **MSRV**: redb 4.x needs Rust 1.90 and edition 2024. The workspace declares **1.85** since
  1.10.0, and the MSRV CI job that OQ-66 proposed exists (`make msrv`). So this is now an
  ordinary bump of `smysl-graph`'s own `rust-version` in the release that ships `store-redb` —
  and because the gate fails in both directions, forgetting to raise it is a red build rather
  than a false claim. OQ-66 is answered.
- `make crate-features` gains `smysl-graph` with `store-redb` alone.

## 5. Tests, fixtures and harnesses

### 5.1 Trait conformance suite (rule D: same results, same bytes)

`crates/smysl-graph/tests/store_conformance.rs`, plus a facade-level
`tests/store_conformance_ops.rs` for operations in other crates.

**Implementations under test**, built from the same record set R:
1. `Store::from_records(R)` — the oracle;
2. `Store::open` on a log of R;
3. `RedbStore` in index mode, core cache 0 bytes (every `get` hits disk);
4. `RedbStore`, cache large enough for everything;
5. `RedbStore` in primary mode (from P13d);
6. `Union` of R partitioned into k shards by a seeded partition (k = 1, 2, 7), in every shard
   order for k ≤ 3 and 5 seeded orders for k = 7; *as built (P13c.1), with about one unit in
   eight dealt into two shards, over opened logs (one holding repeats), through `AnyStore`, under
   an `Overlay`, and with `store-redb` over `RedbStore` shards with the core cache off;*
7. `Overlay` of `Store::from_records(R₁)` and batch R₂ (R = R₁ ⊎ R₂, three split points);
8. *as built (P13c.3):* one `RedbStore` over the same k logs (`open_shards`), the first holding
   repeats, against `from_records` of their records in shard-id order; reopened with the cache
   on and nothing to fold; opened for every other shard, against a `Union` of those shards
   given the index of all (so a reference into another is `W500` in both); and after the last
   shard's log grows, folded without a rebuild.

**What must be identical** across all seven (byte-for-byte where the output is bytes):
- every `StoreRead` method, on every uid in R and on 100 seeded absent uids;
- the `Adjacency` field by field (`order`, `present`, `fwd`, `rev`, `extensions`) via a
  `#[cfg(test)]` accessor;
- `record_set_digest`, `state_hash`;
- operations: `check` report (sorted diagnostics, encoded), `pack` output bytes for 3 budgets,
  `derive_thread` for all five schemas (encoded thread), `salience` report, `trace` from 10
  roots, `effective_status` under 3 policies, `review`, `detect`, `compact` output,
  `relink` output, `bundle` bytes for each view, `Bm25` search results for the
  `fixtures/retrieval` queries, `render::build` IR for each thread.

**Inputs:** `fixtures/corpus/F1`–`F13`, `fixtures/conformance`, the `graph-31102` synthetic
corpus, and a proptest generator (D-11) over the existing `smysl_fuzz::generate` record
generator (`fuzz/src`), 256 cases per run, which also drives the shard partition and shard
order. A new cargo-fuzz target `fuzz/fuzz_targets/store_union.rs` asserts implementation 1 ≡
6 on fuzzer-chosen records and partitions.

Plus: a compile-time `Send + Sync` assertion for `Store`, `RedbStore`, `Union<'_, Store>`,
`Overlay<'_, Store>`.

### 5.2 Downstream-shape compile test (semver guard)

`tests/downstream_shapes.rs` at the facade calls each generalised public function with:
`&Store`, `&&Store`, `&mut Store` (reborrowed), `&Box<Store>`, `&Arc<Store>`, `&Rc<Store>`, a
function pointer `let f: fn(&Store, CheckOptions) -> Report = smysl::check;`, and
`smysl::check` passed to a higher-order function expecting `Fn(&Store, …)`. It runs in every
port step (§6) before `make semver`.

### 5.3 Crash, recovery and concurrency (redb)

- **Fault injection**: a `StorageBackend` wrapper (redb's public trait) that fails or
  truncates the n-th write; for n over every write of a 1,000-record append sequence: reopen,
  and assert the store equals `Store::open` of the log (index mode), or the last committed
  state (primary mode).
- **Kill test**: a child process appending in batches of 50 is `SIGKILL`ed at 20 seeded
  points; reopen must report `W110`/`W501` at most, never an error, and the record-set digest
  must equal the log's.
- **Log/redb ordering**: inject a crash between the log `fsync` and the redb commit; reopen
  replays exactly the tail (asserted via `OpenReport`).
- **Single writer**: two processes open the same shard read-write; exactly one gets `E445`.
- **Readers**: 8 reader threads on one `RedbStore` while a writer appends; each reader's
  answers come from one snapshot (its `record_set_digest` does not change during its life).
- **Cross-process reader during a write**: falls back to the log within `reader_wait_ms`,
  reports `W504`, and returns the same answers as an in-memory open of the same log prefix.
- `check_integrity()` after every crash case.

### 5.4 Scale ladder (CI-runnable) and GE-T7 (gated)

**Ladder** (`make scale`, `#[ignore]` tests run with `--ignored`, measurement not gate, as
`crates/smysl-check/tests/scaling.rs` already argues): synthetic `bench` and `graph` corpora at
31k, 155k and 10⁶ units, generated in-process (no surface step). Reported per backend: open
time, peak RSS, per-operation time for the §2.8 stages, single-record append, and the core
cache hit rate. Output recorded under `Documentation/` in the release that ships each step.

`gen_graph.py` (the corpus used in §2.8):

```python
import random, sys
random.seed(7); n = int(sys.argv[1])
voc = ("and the of to that in he shall unto for i his a lord they be is him not them it with all "
      "thou thy was god which my me said but ye their have thee from as are when this out were "
      "by you up made one king people house come day earth land son father").split()
out = ["@doc smysl/0.1 {\n  id: v/bench\n  intent: text-research\n  lang: en\n  roots: [v/c0]\n}\n"]
claims = []
for i in range(n):
    w = [random.choice(voc) for _ in range(10)]; w[0] = w[0].capitalize()
    out.append('@prose v/x%d { status: cited, source: { kind: doc, ref: "bench:g#%d" } }\n~ %s.\n' % (i, i, " ".join(w)))
    if i % 10 == 9:
        c = len(claims); g = sorted(set(random.randrange(0, i + 1) for _ in range(3)))
        w = [random.choice(voc) for _ in range(10)]; w[0] = w[0].capitalize()
        out.append('@claim v/c%d { status: derived, grounds: [%s] }\n~ %s.\n'
                   % (c, ", ".join("v/x%d" % k for k in g), " ".join(w)))
        claims.append(c)
for c in claims[1:]:
    out.append('@rel v/c%d --backs--> v/c%d\n' % (c, random.randrange(0, c)))
open("graph-%d.smy" % n, "w").write("\n".join(out))
# CBOR log: smysl merge graph-N.smy empty.smy --format cbor -o graph-N.log
```

**GE-T7** (draft 3 §23, gated experiment, run once per release candidate of P13c/P13d):
- corpus: 1,000 public-domain works across 5 languages, one shard per expression plus the
  catalog shard, built by SMYSL-2.4's pipeline;
- thresholds (draft 3): library build < 1 h; a measure (SMYSL-2.6) over 10⁶ units within 2× of
  linear (fit on 10⁵, 3·10⁵, 10⁶); a scoped query (SMYSL-2.5) p95 < 1 s;
- added by this RFC: peak RSS of `check` over the 10⁶-unit union ≤ 1 GB with the redb backend
  (default cache budgets); `derive_thread` over a 10⁵-unit scope < 2 s; contention detection
  over the full union < 1 s (decides OQ-16); single-record append to a 10⁵-unit shard < 5 ms;
- reported: every §2.8 stage for memory, redb (index), redb (primary) and the union.

*As built (P13c.2): a stand-in, `tests/ge_t7.rs`*, run by hand
(`cargo test --release --all-features --test ge_t7 -- --ignored --nocapture`). The repository
carries ten public-domain works, not a thousand: Ruth and Jude in five languages. Each of the
thousand works is one language's 110 verses, rotated by its number and headed by it, so the
readers, the segmenter and the indexes read a thousand distinct texts — about 12 MB, where a
thousand books would be hundreds. Its units are built, not extracted, at §5.4's proportions plus
every shape a contention detector looks for, once per thousand units, some reaching into the
next work's shard; one work holds 10⁵ units, for the append threshold. Its build is readers,
segmentation, the records and the indexes — OQ-69's proposal, which the stand-in adopts and
GE-T7 proper can still revise. GE-T7 on a real corpus remains owed.

### 5.5 Cross-implementation

The Python, JavaScript and Go implementations read CBOR logs, not redb files. Nothing in this
RFC changes what they read; the conformance fixtures stay CBOR. `lib export` from a primary
shard must be readable by all three (that is GE-T15's matrix, SMYSL-2.7).

## 6. Delivery steps

TX-P13 is split. P13a needs only TX-P0 (for `record_set_digest`); it touches no disk format and
is safe to start early. P13d is the one that needs GE-T15.

**Every step's common exit test:** `cargo test --workspace --all-features`; `make lint`;
`make gates` (purity, determinism); `make conformance`; §5.2 compile test; `make semver`
clean for all 12 published crates with `SEMVER_BREAKING` empty; `make api` run and the
`tests/public-api*.txt` diff reviewed to contain **only additions**; the store conformance
suite (§5.1) for every implementation that exists by then.

| step | work | specific exit test |
|---|---|---|
| **P13a.1** | *moved to SMYSL-2.1 H-18 (TX-P0, step 11a); here only re-measured* | `derive_thread` on graph-155510 < 1 s; thread bytes identical to `d25ec9e` on graph-31102, graph-155510 and F1–F13 for all five schemas |
| P13a.2 | `from_records` single hashing; `reindex` without the second absorb | `from_records` at 155k ≥ 15% faster; index bytes identical (SM-P3 gate: `reindex --verify`) |
| P13a.3 | lazy `Derived` in `Store` (`OnceLock`); `Send + Sync` assertion test | one-record append at 155k < 5 ms in memory; the 1.8 append table re-measured and the `Store::append` doc comment and CHANGELOG updated; `xtask determinism` unchanged |
| P13a.4 | `StoreRead`/`StoreWrite` in `smysl-graph`, `impl for Store`, blanket impls, `Interpretive` regrouping, hot record accessors | conformance suite with implementations 1–2; every `Store` inherent signature unchanged (public-api diff is additions only) |
| P13a.5 | port `labels`, `lineage`, `salience` (§4.3 #2–4) | suite + no measurable regression: §2.8 in-process stages within ±5% (3 runs, median) |
| P13a.6 | port `merge/*`, `relink`, `compact` (#5–6) | as above; `merge_algebra` fuzz target 10 min clean |
| P13a.7 | port `smysl-check` (#7) | as above; `check` reports byte-identical on the fixtures |
| P13a.8 | port `smysl-pack` (#8) | as above; `pack_constraints`, `pack_exact` fuzz targets 10 min clean |
| P13a.9 | port `smysl-thread`, `smysl-retrieve`, `smysl-embed`, `smysl-render` (#9–11) | as above; `fixtures/golden` render outputs unchanged |
| P13a.10 | `Overlay`; port `smysl-ingest` (#12) with `stage` on `Overlay` | staging a 50-unit batch against a 155k store < 50 ms (today ≈ a full `from_records`, 1.2 s); staged records identical to today's on `crates/smysl-ingest/tests/gate.rs` |
| P13a.11 | port `smysl-tui`, `smysl-eval`, facade, CLI through `AnyStore` (memory only) (#13–15) | `make doc-output`, `make cli-surface` diff reviewed |
| **P13b.1** | `store::redb` index mode: tables, open/rebuild, `RedbStore` read path, tiering, caches | suite with implementations 3–4; §5.3 fault-injection and kill tests; peak RSS of `check` on graph-155510 ≤ 40% of the memory backend |
| P13b.2 | `RedbShard` write path, lock file, reader fallback; `--backend redb`; `reindex --backend redb` | §5.3 concurrency tests; ladder at 10⁶ units completes on a 7 GB machine with the redb backend |
| **P13c.1** | shards in a library (with SMYSL-2.4), `uid_shard`, `Union`, `--shard` | suite implementation 6; `store_union` fuzz target 10 min clean; `W500` on a two-shard fixture with a cross-shard `quotes` edge, resolved when the second shard is added |
| **P13c.2** | contentions over the union, measured | GE-T7 run; OQ-16 decided from its contention-detection number |
| **P13c.3** | one redb unit store per library, every row naming its shard's log (OQ-90) — **before P13d** | `check` over GE-T7's stand-in on redb ≤ 1 GB; the library's unit store at most 2× its logs on disk; §5.3 and the conformance suite (implementations 4–6) on the new layout; the stand-in's other GE-T7 numbers no worse |
| **P13d** | primary mode, `lib migrate`, `E505` — **only after GE-T15 passes** | suite implementation 5; export → import round trip reproduces the record-set digest on every GE-T15 corpus; GE-T7 rerun on primary shards |

### 6.1 Progress

Measured on `tests/scale_ladder.rs` (§5.4's `graph` corpus generated in process — its shape and
proportions, not `gen_graph.py`'s bytes, and without the label bindings the surface step adds;
`--release`, median of three for `from_records`). 171k units is graph-155510.

- **P13a.1 — done (2026-10-23).** `derive_thread` at 171k units: 9.6–19.8 s per schema at
  `d25ec9e`, 0.33–0.37 s now (exit: < 1 s). Thread bytes are identical to `d25ec9e`'s for all
  five schemas at both 34k and 171k units (BLAKE3 of the encoded thread). **Finding:** at
  `d25ec9e` the same corpus checks in 58 ms; before step 3 it took 181 ms. That is the passes
  TX-P1 to TX-P12 added, not this phase; it is recorded, not chased.
- **P13a.2 — done (2026-10-23).** `from_records` encodes each record once and uses the bytes as
  dedup key, record hash and log-hash input; `absorb` takes the hashes it is handed. 537 ms →
  436 ms at 171k (−19%; exit −15%). `smysl reindex` indexes the store `open_with` has just
  built instead of absorbing the log a second time; a test pins that a store fresh from its log
  indexes byte-identically to its rebuild and to a `from_records` load (SM-P3).
- **P13a.3 — done (2026-10-23).** `adjacency` and `unfounded` live in one `OnceLock<Derived>`;
  `append` invalidates it and the first read rebuilds it. One-record append at 171k: 58 ms →
  **1.3 ms** (exit: < 5 ms). The first read after it pays 56 ms — the rebuild moved, not removed,
  and so does a load's: `from_records` is 380 ms and the first `check` includes the adjacency.
  `Store: Send + Sync` is asserted; `Store::prepare()` builds the derived state on demand. The
  pending-attestation retry stays eager. The `append` doc comment and CHANGELOG carry the new
  figures. **Finding:** `Store` loses the `Freeze` auto trait (an `UnsafeCell` inside
  `OnceLock`). `Freeze` is unstable and cannot be named or bounded on stable Rust, and `Store`
  has no `const` constructor, so nothing downstream can observe it; it is the one line the
  public-API diff of `smysl-graph` removes.
- **P13a.4 — done (2026-10-23).** `store/read.rs`: `StoreRead` (sealed, three associated types)
  with **27 required** methods — §4.2's 24 plus `relation_by_key` (Table B's "over
  `relation_by_key`", which the sketch left out), `commits` (`units_at_commitment` iterates
  committed uids, present or not, and nothing else lists them) and `state_hash` (§5.1 compares
  it) — and the **18 provided** ones; `StoreWrite`; iterator newtypes `Relations`, `Threads`,
  `Views`, `Withdrawals`, `Resolutions`, `Commits`, `HotRecords`; `HotKind`. `impl` for `Store`
  overrides every provided method with its inherent one, so generic and concrete callers get
  one answer at one cost; delegating impls for `&S`, `&mut S`, `Box<S>`, `Rc<S>`, `Arc<S>`.
  `store/interp.rs` holds the interpretive layer and its fold (`absorb_hot`); the hot records
  are positions into the log, by kind. `UidPrefix::range` (smysl-core, additive) makes
  `matching_prefix` a range scan. Exit: the conformance suite (`tests/store_conformance.rs`)
  compares every method on every mentioned uid and a hundred absent ones, the adjacency, the
  record-set digest and the state hash, for implementations 1–2 behind every pointer, over
  F1–F13, the check fixtures, all of them at once and a lifecycle store; a unit test runs every
  provided *default* body (through a wrapper that forwards only the required methods) against
  the overrides. Public API: additions only, apart from `Freeze` above. **Not done here:**
  `Adjacency::from_parts` (§3.2) has no second caller until P13b, and §4.1's `any.rs`,
  `union.rs` are P13a.10–11 and P13c.

Steps 5–11 below were measured against the step-4 commit (`6143768`) on the same ladder, median
of three; "within ±5%" holds for every stage at 171k units except `units_in_observed_order`
(2.8 → 3.0 ms, a function no step touched). At 34k units `check` read +6.1% and `salience`
+5.7% on that run; an interleaved A/B of seven runs each puts them at +0.0% and +1.1%, so they are
recorded as noise rather than as regressions. Derived thread bytes are unchanged throughout.

**One decision taken for all seven steps: `impl StoreRead` in argument position, not a named
type parameter.** §3.2 wrote `fn f<S: StoreRead + ?Sized>(store: &S)`. `cargo-semver-checks`
0.50 reports that form as **major** — `function_requires_different_generic_type_params`, "0 → 1
generic types" — because a caller naming the type parameters by turbofish would break, and
`make semver` gates on it. `store: &(impl StoreRead + ?Sized)` admits exactly the same callers
and has no nameable parameter, and the tool reports it as needing no version change. It is used
in every ported signature; §3.2's sketch and §5.2's shapes are otherwise unchanged.

- **P13a.5 — done (2026-10-23).** `labels`, `lineage`, `salience` ported. `labels.rs` reads
  `hot_records(LabelBindings)` instead of scanning the log. `tests/downstream_shapes.rs` (§5.2)
  is new: every ported function through `&Store`, `&&Store`, a reborrowed `&mut Store`,
  `&Box`, `&Arc`, `&Rc` (and, from step 10, an `Overlay`), each answer compared with the
  `&Store` one, plus the function-pointer and higher-order shapes. `make semver`: smysl-graph
  needs no version change.
- **P13a.6 — done (2026-10-23).** `merge/*` (`merge`'s second argument only, §3.3), `relink`,
  `compact` ported; `contention.rs`'s label scan reads hot records. `plan_retraction` simulates
  a retraction on a copy of the store, which a generic store cannot make: the sealed supertrait
  gained a crate-private `owned() -> Cow<Store>` — the in-memory store lends itself, so the
  clone is the one it always paid. `merge_algebra` fuzzed 601 s clean (286,431 runs).
- **P13a.7 — done (2026-10-23).** `smysl-check` ported, `extension.rs` on hot records. **Two
  things Table B predates.** (1) The library pass reads manifests, heads, datings and parts
  (SMYSL-2.4): the store's library state became a public `StoreLibrary` with the read methods
  `Store` already had, `StoreRead` gained a 28th required method `library()`, and `Store`'s
  inherent methods delegate to it with their signatures unchanged. It is interpretive — sized by
  manifests, not text — so every implementation holds one. (2) The time pass calls
  `smysl_text::time::engine::effective`, so the engine and `constraints::free` were ported too
  (`smysl-text` is unpublished; two linear `units().find` lookups became `get`). `check`'s
  output is byte-identical to step 6's binary on every fixture store under `fixtures/` (text and
  `--json`, 189 invocations, 63 with diagnostics). The conformance suite compares
  `library()` and gained a store with manifests, a supersession, a redaction and a dating.
- **P13a.8 — done (2026-10-23).** `smysl-pack` ported, the cache's `required` included.
  `pack_constraints` and `pack_exact` fuzzed 600 s each, clean.
- **P13a.9 — done (2026-10-23).** `smysl-thread`, `smysl-retrieve`, `smysl-embed`,
  `smysl-render` ported. **Not ported: `Bm25View` and `Bm25::over`**, which hold a
  `&'p Store`; generalising the struct would add a type parameter to a public type, which is a
  break, and neither is on §4.3's list. Three sites that borrowed through a unit handle now
  copy a gist or nest the borrow, since a paged store's handle is owned.
- **P13a.10 — done (2026-10-23).** `Overlay` (`store/union.rs`) and `smysl-ingest` ported, with
  `stage` checking a batch through an overlay instead of `from_records` over a copy of the
  store. The batch becomes a small store, less what the base holds; the interpretive layer and
  the library are the base's cloned with the batch folded in by the same `absorb_hot`; units read
  through, except batch units and base units a batch attestation lands on; the adjacency and
  `unfounded` are derived lazily over the union. This gave `Adjacency::from_parts` (§3.2) its
  second caller, so it landed here rather than in P13b: `build` delegates to it, and presence
  is a binary search over the sorted units. The sealed supertrait carries what an overlay needs
  of its base (the interpretive layer, pending attestations, record hashes), crate-private.
  Conformance: implementation 7 at four split points, over an opened base, nested, and
  interleaved (attestations before their units); `tests/store_conformance_ops.rs` (new, the
  facade half of §5.1) runs `check`, salience, pack at three budgets, five threads with rule L
  and render IR, trace, effective status, detect, review, compact, relink and BM25 on an overlay
  and on the store it stands for. `ingest/tests/gate.rs` passes unchanged. **Staging 50 units:
  670 → 312 ms at 171k units, 110 → 49 ms at 34k. The < 50 ms exit is not met at 171k, and
  cannot be by this design:** staging checks the batch against the whole union, and `check` at
  171k is 254 ms on its own. The figure was written when `check` there was 58 ms, and even
  then it and the adjacency (56 ms) exceeded it. What step 10 removed is the copy; what is left
  is `check`, which is the finding P13a.1 already recorded. An incremental check is the way
  under 50 ms and is not in this RFC.
- **P13a.11 — done (2026-10-23).** `AnyStore` (`store/any.rs`), in-memory only: its unit,
  units and records types are newtypes over a private enum, so the redb variant changes no type
  a caller can name, and the enum is `#[non_exhaustive]`. `App::with_store(AnyStore)` (additive).
  `smysl-eval` ported except the three functions that call `smysl-measure`, which takes
  `&Store` and is not in P13a. The facade re-exports `StoreRead`, `StoreWrite`, `Overlay`,
  `AnyStore` and `StoreLibrary`; the public-API diff is those five lines. The CLI's
  `load_store` returns `AnyStore`; commands that append, bundle, or hand the store to the
  unported library, query and measure layers load through `load_memory`. No flag is added, so
  `tests/cli-surface.txt` does not change. **What P13a leaves on `&Store`:** `smysl-text`
  outside the time engine, `smysl-query`, `smysl-measure`, `smysl-lib`, `Bm25View` — the crates
  that postdate §2.6's inventory — for whichever phase gives them a second backend to serve.
- **P13b.1 — done (2026-10-24).** `store::redb` behind `store-redb`: `RedbStore`,
  `RedbOptions`, `CacheStats`, `AnyStore::Redb` and `AnyStore::open_redb`, re-exported as
  `smysl::redb`. Index mode only. What changed from §3.5, each for a reason measured or found:
  - **redb 2.6.4, not 4** (§4.5). The workspace's existing pin, shared with `substrate-redb`.
    4.x declares 1.90, and since every crate depends on `smysl-graph` it would have raised the
    whole workspace's floor, not one crate's. 2.6 has no read-only handle — every open takes
    `flock(LOCK_EX)` — so OQ-67's fallback serves readers too, not only readers of a writer.
  - **Tables** (§3.5.3): `meta`, `record_hash`, `header`, `deps`, `attest`, `interp`,
    `observed`, `source_ref`. No `log` table: index mode reads the log sequentially, and the
    table returns with primary mode, where it holds the bytes. No `core` table: the header holds
    the core's offset and length, which is 32 bytes rather than 24 and saves a b-tree descent
    on every `get`. No `schema_intern`: nothing reads a schema without its core yet.
  - **No `verify_hashes` option**: every open hashes the log prefix the store describes, so a
    core cannot change under a store without a rebuild.
  - **Tiering** (OQ-69, as recommended): cores of `speculative`, `inferred` and `derived` units
    and of relation endpoints are pinned once read; the rest go to a byte-budgeted tier that
    `get` fills and `units()` scans read through. Scan-resistance was measured in: a 64 MiB
    LRU that scans filled made `check` slower than no cache (2.51 s against 2.19 s at 155k),
    because each scan evicted every core before the next reached it. **Default 32 MiB, not
    64**, because resident size tracks the budget: on graph-155510, open + `check` was 2.45 s
    and 110 MB with the cache off, 2.35 s / 144 MB at 16 MiB, 2.25 s / 164 MB at 32, 1.99 s /
    205 MB at 64, and 1.07 s / 228 MB with everything cached (256 MiB).
  - **`OpenReport::replayed`** (additive): records folded in from the log at open.
  - **Exit: peak RSS of `check` on graph-155510, 165 MB against 474 MB in memory — 35%.** Met
    at the default budget, and only after a finding outside this crate. The first measurement
    read 61%, and with the cache off the floor was still 41%: `check`'s `Time` pass ran rule E's
    solve on every axis over every unit — a subject list, an index and a network each — on a
    store with no dating, about 140 MB at 155k on *either* backend. `quiescent` (new, in
    `smysl-text`'s engine) says when the solve could find nothing — no dating, no free
    constraint, no as-recorded interval already empty — and the pass skips it then. A property
    test holds `quiescent` to the solve on 200 generated stores and the corpora. In memory the
    same fix takes `check`'s peak from 609 to 473 MB. Open with the adjacency alone is 109 MB,
    23%. A second finding: the first cache charged a core its encoded length, and a decoded
    unit is several times that, so a 64 MiB tier held all 155k cores while accounting 12 MB.
  - **Conformance**: implementations 3 and 4 — the cache at 0 and at its default — on every
    corpus, fixture and the lifecycle and library stores; reopened (no rebuild), caught up
    after the log grew (exactly the tail), under `AnyStore`, under an `Overlay` with no batch
    and with half the log as its batch, and through `into_memory`; a log with repeats and a
    torn tail against `Store::open`, byte for byte; `Send + Sync`. The operations suite runs
    every operation on the store with the cache off.
  - **§5.3** (what applies without a writer): faults through redb's `StorageBackend`, a failed
    write and a torn one, at every write point of a 1,000-record append sequence — all recovered,
    each reopening as `Store::open` of its log with nothing worse than `W501`/`W110`, and redb's
    `check_integrity` clean wherever the file still opened: 1,558 write points, 3,116 cases
    (`--ignored`, 33 min); by default 41 seeded points and the last, 82 cases, two of which left
    a file redb would not open and were rebuilt. A writer `SIGKILL`ed at 20 seeded moments, the same. The crash between
    the log's `fsync` and the commit folds in exactly the tail. A store held elsewhere is read
    from the log after `reader_wait_ms` with `W504`; one released within the wait is opened.
    The 8-reader and single-writer cases need the writer, and are P13b.2's.
  - **Codes**: `SMY-W501`, `SMY-E503`, `SMY-W504`, in a fourteenth group, `Store` (143).
  - **What it costs**, at 155k (`--release`, the `backends` ladder): building from the log
    1.4 s (`Store::open` 0.37 s); opening a current store 36 ms; `check` 2.1 s against 175 ms,
    salience 280 against 51 ms, `derive_thread` 1.25 s against 0.36 s, `Bm25::index` 1.57
    against 1.26 s — every scan decodes the cores it reads, and the 32 MiB tier held 59k of
    155k (42% of reads hit). That is the trade this step was for, and the first thing to look
    at when a scan's latency matters: decoding only the fields a pass reads would cut it, and
    is not in this RFC. On graph-31102 the RSS ratio is 55%, the fixed costs being most of a
    small resident set. The file is 68 MB for a 15.7 MB log — redb pages, three indexes and the
    attestations — and, as §3.5.6 says, not byte-reproducible.

- **P13b.2 — done (2026-10-25).** The writer, `--backend`, and the rest of §5.3:
  - **`RedbShard`** (`open_rw`, `append`, `reader`): the shard's lock file, taken before
    anything is opened, so a second writer fails with `SMY-E445` naming the holder; then the
    log, `fsync`ed; then one commit with `Durability::Immediate` carrying `log_len`/`log_hash`.
    Duplicates are dropped against `record_hash` and within the batch, as `Store::append` drops
    them, and the same records are refused, by one function both call (`refuse_unloggable`).
    `open_rw` waits `reader_wait_ms` for a store another process is reading and then fails —
    a writer has nothing to fall back to that leaves the store describing its log. A torn tail
    found at `open_rw` (a killed writer whose lock was broken) is cut, with `W110` saying so,
    because a record appended after it could never be read; one found mid-session is refused,
    since this writer did not write it and somebody is appending without the lock.
  - **One database per file per process** (`redb/shared.rs`). redb 2.6's `flock` conflicts
    with a second open in the same process as with another process's, so a reader and the
    writer in one process could not both hold a store — §3.8 says they do not block each
    other, and on redb's side they do not. Opens now share one `Database` through a registry of
    weak handles, with a lock around each catch-up so two cannot fold the same tail at once.
  - **`AnyStore`'s `StoreWrite` for the unit store**: the shard opened for the one append, the
    lock released, the store replaced by a snapshot that includes what was appended — §3.8's
    "writers keep sessions short". `AnyStore::len` and `is_empty` (additive).
  - **CLI**: global `--backend memory|redb`. `load_store` and `check` read a CBOR log through
    the unit store (`W501` on the first open, nothing after); surface text and stdin still load
    into memory; every command's write goes through `RedbShard`. `reindex --backend redb`
    builds the store; `--verify` compares record-set digest, units, state and adjacency against
    an in-memory rebuild and exits 9 on a difference, or on a store that had to be rebuilt to
    open. A build without the feature refuses `--backend redb` by name, exit 2.
    `make crate-features` gains `smysl-graph --features store-redb`.
  - **Found: a plan wrote.** `plan_retraction` simulated a retraction by appending it to a copy
    of the store — and a copy of a store opened from a log keeps its path, so the copy's
    `append` wrote the retraction to the log. A plan, authorised or refused, retracted the unit
    on disk, on any store from `Store::open`, since the simulation was written. The CLI never
    met it because its stores came from `from_records`, with no path, until `--backend redb`
    gave them one: a refused `retract` then wrote its record. The simulation is an `Overlay`
    now, which writes nothing and does not copy the store; the redb store's in-memory copy has
    no path, as `from_records`'s has none (it would otherwise also have written the record
    twice, once by the writer and once by the copy). A test holds a plan to the log's bytes,
    and fails on the old simulation.
  - **Found: a lock caught half-written.** `Lock::acquire` created the file and then wrote the
    holder; a second writer between the two read an empty file and was refused by "pid 0 on
    running ``". Two processes racing for a shard hit it on the first run. The lock is written
    beside itself and hard-linked into place, which fails if the lock exists, exactly as
    `create_new` did. Eight threads racing fifty times now each see a named holder.
  - **§5.3, now whole**: two processes `open_rw` the same shard, exactly one gets `E445` and is
    told the other's pid; eight reader snapshots taken at different points of a writer's
    appends each answer from their moment — record-set digest, units, and the attestations of
    a unit that later records attest, read from the store's table with the cache off — and
    each equals an in-memory open of exactly its log prefix; a reader in another process while
    a writer writes falls back within the wait with `W504` and answers as an in-memory open of
    the prefix it read; a store held by another process, and one released within the wait,
    across real processes now that one process shares its opens. The kill test kills the real
    writer at even points and a lock-less writer at odd ones; a killed writer leaves its lock
    naming its pid, the next `open_rw` gets `E445`, and after `break_lock` the next append
    reads back. `check_integrity` after every kill.
  - **Conformance, implementation 5**: the log written by three appends through `RedbShard`
    and through `AnyStore`'s append — the second carrying repeats — answers as the in-memory
    store given the same appends, append by append, with the same reports and the same log
    bytes, and reopens with nothing to fold.
  - **Exit: the ladder at 10⁶ units completes on the redb backend in 3.3 GB** (`SMYSL_BACKENDS=redb`,
    which never builds the in-memory store; 1.1 M units, a 102 MB log, a 540 MB store). The
    whole run's peak is `Bm25::index`'s own index, 2.39 GB measured alone, which the in-memory
    backend would hold as well. On redb: build from the log 14 s (535 MB peak), open 0.29 s
    (528 MB, the headers and adjacency), `check` 17 s at 686 MB peak against 2.86 GB for the
    in-memory store — 24% — salience 2.1 s (1.0 GB), `derive_thread` 9.7 s (1.4 GB),
    `Bm25::index` 11.5 s; the core cache hit 36%.
  - **Single-record append: 6–8 ms** on redb at 31k, 155k and 10⁶ units, against 0.1 ms for the
    in-memory store, which does not sync. The two syncs — the log, then redb's commit — are the
    cost; GE-T7's < 5 ms on a 10⁵-unit shard is not met here, and is GE-T7's to decide.
  - **Noted: a rebuild in place does not shrink the file.** Clearing the tables leaves their
    pages free, not returned, so a store rebuilt over itself is about twice its size (1.08 GB
    at 10⁶); redb reuses the pages on the next writes, so it does not grow past that.
    Compacting needs the database alone (`&mut`), which a shared one is not.

- **P13c.1 — done (2026-10-25).** Shards in a library, their union, and `--shard`:
  - **`Union<S: StoreRead>`** (`store/shards.rs`), implementation 6 of §5.1, against
    `from_records` of the shards' records in shard-id order; every order of k ≤ 3 and five of
    k = 7 give one answer. It owns its shards — §4.2's sketch borrowed them — because `AnyStore`
    has to hold one for `--shard`, and `&Store` is a store, so a borrowing caller loses nothing.
    `AnyStore::Shards` is the new variant; its append is refused, since choosing a shard is the
    library's routing.
  - **`UidShards`** and **`SMY-W500`** (registry 144). `with_index` makes `report_dangling` say
    "waits on shard S" for a uid the index places in a shard not loaded. The exit's fixture is a
    `x.text/quotes` edge from one chat's reply to another chat's message, in the graph
    (`shards.rs`), the library (`smysl-text/tests/shards.rs`) and the CLI
    (`tests/cmd_shard.rs`): `W500` naming `chat/messages` with one shard, nothing with both, no
    byte written to either.
  - **Library layout 2** (`smysl_text::shards`, with SMYSL-2.4): `log/shards/<alias>.cbor` per
    expression beside the catalog, routed by what a record is about (§3.6's as-built note).
    `Library` still opens every shard into one in-memory `Store`, so nothing reading
    `Library::store` changed. The substrate index is stamped over every shard, and with the
    catalog alone the stamp is the catalog's length and hash, as it was, so sidecars written
    before shards stay current. A layout-1 library opens as one shard and is raised to layout 2,
    marker first, by the write that creates the second. A shard's torn tail is cut before the
    next write; each write is `fsync`ed, which the single catalog's was not.
  - **`--shard ALIAS`**, global, repeatable, `all` for every shard; each shard through
    `--backend`'s choice, and `--backend redb` on a library always reads it as a union of its
    shards' unit stores (each at `<log dir>/.smysl/store/<stem>.redb`, beside its log, not
    §3.6's `store/<shard>.redb`). Without either flag a library is read as one log, its shards
    concatenated in id order — what `check -s <library>` read before, now with every shard.
  - **Fuzzed.** `fuzz/fuzz_targets/store_union.rs`: a generated store dealt into up to five
    shards by the input, records dealt twice and attestations away from their subjects, the
    union in an input-chosen order against its records' store and the whole. 10 minutes clean
    (602,848 runs on the final code). In `make fuzz` and CI's algebra job.
  - **Measured: opening a library**, 155,510 units in five expressions (now five shards),
    `postings_timing`, release, one run each against `4fe6328`: 749 ms against 697 ms with a
    current sidecar, 903 against 857 ms without one (+5–7%). Building the library (33 s) did not
    slow for the `fsync`s. Recorded rather than chased: the open reads each shard and builds one
    store from all their records, where it used to open one log.

- **P13c.2 — done (2026-10-26).** Contentions over the union, and GE-T7 run — on a stand-in:
  - **Detection reads the interpretive layer only** (§3.7's as-built note): commitment forks
    walk the commitments, label collisions no longer read units' own (always empty) labels,
    live rebuttals index threads once. `Union::detected()` runs `detect` once and keeps it. A
    test builds a fork whose two successors are in two shards and a commitment fork whose two
    agents' words are in two, finds neither in either shard, both in the union, and the union's
    answer equal to the materialised store's.
  - **GE-T7's stand-in** (`tests/ge_t7.rs`, §5.4's as-built note): 1,000 works in five
    languages, 1,001 shards with the catalog, 1,004,595 units, 1,155,458 records; 1,099 of
    each contention kind, every one found by the materialised store, the in-memory union and
    the redb union alike, and nothing else found. Release, one run:

    | GE-T7 threshold | measured | |
    |---|---|---|
    | library build < 1 h | 528 s (read and segment 8.5 s; the records 521 s in 1,000 appends) | met |
    | a measure (A1 A2 A4 A5 A7) within 2× of linear on 10⁵, 3·10⁵, 10⁶ | 3.84 s, 7.95 s, 33.6 s; 1.25× off the line | met |
    | scoped query p95 < 1 s | 11.7 s over a twenty-work shelf, 20.1 s over a language, 154 s over the library | **not met** |
    | `check` peak RSS on redb ≤ 1 GB | 2.7 GB (memory 4.0 GB) | **not met** |
    | `derive_thread` over 10⁵ units < 2 s | 361 ms in one shard, 368 ms across 111 | met |
    | contention detection over the union < 1 s | 444 ms store, 638 ms union, 638 ms redb union; kept: 0 | met — **OQ-16 decided: lazy** |
    | single-record append to a 10⁵-unit shard < 5 ms | 5.01 ms through the library, 7.04 ms through the shard's redb store | **not met**, by 0.01 ms |

    `Library::open` of it: 5.6 s.
  - **Found and fixed, all in the library's write path, all quadratic in the library:**
    `UidShards::extend` re-sorted every pair to add a batch's, and `write_uids` rewrote the
    shard's whole list — 34 ms of a 36 ms single-record append at 10⁶ units. The index is now a
    sorted run and a sorted delta folded in at a sixteenth of the run; a uid list is grown in
    place, the uids after the others and the stamp last, and every shard a batch grows is
    restamped, so a relation-only batch no longer leaves its list stale. `Library::open` built
    the index with a `set` per shard, each re-sorting all before it, and hashed every unit a
    second time: it now reads each current uid list and sorts once (`UidShards::from_lists`),
    20.5 s to 5.6 s. The text journal inserted a batch's rows one at a time into a sorted
    million-row table, 0.68 s of a 0.69 s batch append; it merges in place from the end now.
    Building the stand-in went from 832 s to 528 s.
  - **Found and not changed: the time axes are rebuilt by any append that carries a relation,
    attestation, commitment or withdrawal** (`index/ranges.rs`, `bears_on_time`), which at 10⁶
    units is most of what a build batch now costs. The list is a deny-list on purpose, failing
    towards a rebuild, and the time engine does read several of those kinds — `quotes` edges,
    attestations, withdrawals, rule R's unfounded units — so narrowing it is SMYSL-2.4's
    question, not a scale fix. A batch of units alone does not rebuild.
  - **Changed: a redb union shares one cache budget.** `--backend redb` on a library gave each
    of its shards the default core and page caches, which across a thousand shards cached every
    core; each now gets its share. `check` went from 3.7 GB to 2.7 GB.
  - **The two thresholds not met are decided as steps of their own.** `check` on redb: the
    floor is the stores, not their caches — the union opens at 1.85 GB where one 10⁶-unit store
    opens at 528 MB, and each small shard's store is 3.7 MB on disk for a 168 KB log; one
    database per library is OQ-90 and step P13c.3, before P13d. The scoped query: `find` builds
    its postings over the whole store, per query and per analyzer, and reads none of the
    library's; that is SMYSL-2.5's OQ-91 and its step.
  - **Single-record append:** 5.01 ms through the library, by the median of five, against GE-T7's 5 ms, and 7.04 ms through redb, which syncs twice (P13b.2). A bare 200-byte write and `sync_data` on the machine measured is 3.6–4.1 ms, so the threshold is within a millisecond of the disk's own; the library's part is about 1 ms. Recorded as not met rather than rounded.

- **P13c.3 — done (2026-10-26).** One unit store for a library (OQ-90):
  - **Measured before building.** The redb union's open floor at 1,001 shards was its parts: the
    shard stores held open, 868 MB, and the union over them, +323 MB, before its adjacency
    (+456 MB) and `check`. A single store of the same 10⁶ units opens at 528 MB. So the step
    became one store over the logs, not one database of per-shard stores, which would have
    saved only part of the first number.
  - **`RedbStore::open_shards`**: every row names its log (layout 2, §3.5.3), each log is
    folded on its own, and what the store answers is decided at open — over several logs,
    `from_records` of theirs in name order; over one, `Store::open`, unchanged. Opened for some
    shards it reads only theirs, and `holds` gives `SMY-W500` for a reference into another.
    `AnyStore::open_redb_shards` waits for a store another process holds and then reads the
    logs into a union (`SMY-W504`). Under `--backend redb` the CLI reads a library through it,
    at `index/units.redb` (`smysl_text::shards::UNIT_STORE`); appending through it is refused,
    the library being the writer. Conformance implementation 8 (§5.1).
  - **Found and fixed while measuring: a pool of logs.** A store of 1,001 logs read in uid
    order reads them all over and over; with 64 kept open, `check` spent 132 s of 168 opening
    files. The pool now holds as many as the process allows, up to 4,096, and halves when the
    system refuses an open for want of a descriptor; `check` took 43 s.
  - **GE-T7's stand-in on the new layout** (one run, release): `check` on redb **1.69 GB** in
    44 s (2.69 GB before; memory 4.08 GB); open 277 MB; the store 540 MB on disk (3.5 GB of
    per-shard stores before), built from the 1,001 logs in 75.6 s; contention detection on it
    374 ms; every other GE-T7 number as at P13c.2 (§6.1 above), the single-record append
    4.99 ms against 5 — under it this run, over it by 0.01 ms the last, so at the disk's
    `fsync`, not under it.
  - **The exit's ≤ 1 GB is not met, and the store is not what misses it.** Of `check`'s peak,
    opening the store is 277 MB, its adjacency +345 MB and the cores it pins +153 MB, about
    775 MB; `check` with the core cache off still peaks at 1.42 GB. The `time` pass alone peaks
    at 1.41 GB and takes 28 of the 44 s: rule E (`smysl_text::time::engine::effective`) builds a
    subject, a seed and a row of its network for every unit, reading every core to list them,
    to solve the stand-in's five thousand quotation constraints. The passes but `time` peak
    between 0.47 and 0.85 GB. Recorded for SMYSL-2.4 beside P13c.2's finding about the time
    axes, not changed here.
  - **Found and fixed: `check --pass` with a name that is no pass** dropped it, so the list was
    empty and every pass ran, exit 0. Refused now with the names there are, exit 2
    (`tests/cmd_date.rs`).
  - **Superseded, not removed:** P13c.1's per-shard stores, `log/shards/.smysl/store/*.redb`,
    which only this release's development builds wrote; nothing reads them now, and deleting
    them loses nothing.

- **Rule E at scale — done (2026-10-26), after P13c.3, with SMYSL-2.4 §3.3.** The two time
  findings of P13c.2 and P13c.3, taken up:
  - **Found and fixed: one contradiction contested the whole store.** The STN's cycle search
    spread from the reference variable along every unary bound, so a date that contradicted
    one unit's own instant marked every dated subject contested (four of four in a probe, a
    million on the stand-in). The reference is on no cycle now; a cycle through it is the
    subjects whose bounds cross, by their own bounds. P-E6 still agrees with Floyd–Warshall on
    400 networks.
  - **The network holds only the subjects anything bears on**, which is exact once the cycle
    search is local (SMYSL-2.4 §3.3's as-built note). `findings()` for `check`, without the map
    of every subject; `findings_among()` for the axis tables, which now rewrite only the rows
    of the subjects involved before or after an append that bears on time, and splice them in
    rather than sort every row. The first-seen constraint reads `StoreRead::attested_units`, a
    dating's basis is looked up rather than found by a scan, and three passes over every core
    are gone.
  - **GE-T7's stand-in after it** (one run, release): **`check` on redb 885 MB in 20 s** (1.69 GB
    and 44 s at P13c.3) — **the ≤ 1 GB threshold met**; the `time` pass alone 554 MB and 5.2 s
    (1.41 GB, 28 s); **building the library 68 s** (528 s), a 1,000-unit append with relations
    83 ms (835 ms); single-record append 4.92 ms; every other number as before. Six of GE-T7's
    seven thresholds are met on the stand-in; the scoped query (SMYSL-2.5 OQ-91) is the one
    left.

## 7. Risks and mitigations

| risk | why it is real | mitigation |
|---|---|---|
| **Semver break from generalising `&Store` to `&S`** | Inference changes: a caller's own type that `Deref`s to `Store` stops coercing. `--release-type patch` forbids any break, and a listed crate in `SEMVER_BREAKING` means 2.0 | Blanket impls for the five standard pointers (§3.2); the compile test (§5.2) per step; one crate per step so a failure is localised. If the owner treats the `Deref` case as a break, the fallback is additive twins (`check_in<S>`) for the twelve contract operations (OQ-65). The Cargo SemVer guide classes generalising to a generic that admits the original type as minor **(cited from memory; unverified here)**. |
| A trait method added after release breaks implementors | `API_CONTRACT.md`'s rule for unsealed traits | Sealed (§3.2) |
| `make api-check` fails on every addition | it diffs the whole list | each step regenerates with `make api` and the reviewer checks the diff is additions; public-api counts per crate grow and are re-recorded |
| **Performance regression from generic code** | monomorphisation can bloat; `UnitRef` clones (`Arc`) in hot loops on redb | §6 per-step ±5% gate against §2.8 numbers for the memory backend; `UnitRef` is `&Unit` for `Store`, so the memory path compiles to what it is today |
| Paged store slow on full scans | `check` passes iterate every unit's core | interpretive cores pinned; scans stream `core` in key order (sequential pages); the GE-T7 RSS/time thresholds catch it; the memory backend stays default |
| **Determinism drift between implementations** | dense ids, interning, iteration order, union shard order, cache state | one shared `Interpretive` fold; `from_parts` used by every implementation; the conformance suite runs every operation on seven implementations, cache 0 and full, all shard orders |
| Union semantics differ from a merged store | duplicate uids across shards, attestations split across shards | `get` on a duplicated uid unions attestations; proptest over partitions asserts union ≡ `from_records(all)` |
| Crash leaves redb ahead of the log | write ordering | log first, `fsync`, then redb commit (§3.5.5); kill and fault-injection tests |
| Cross-process reader blocked by a writer | redb default locking (§2.7) | bounded wait, then the CBOR log (`W504`); short writer sessions; OQ-67 |
| MSRV jump to 1.90 | redb 4.x | the MSRV job exists since 1.10.0 (`make msrv`), so raise `smysl-graph`'s own `rust-version` with the feature and let the gate check it; OQ-66 answered |
| Lazy adjacency hides cost in the first read | one rebuild after appends | documented on `append`; `Store::prepare()` (a no-op read) lets a caller pay it at a chosen time |
| Disk-primary loses records | export is the only way out | P13d gated on GE-T15; `lib verify` digest check (`E505`); index mode default until GE-T7 reruns on primary |
| Scope creep into SMYSL-2.4's index | both use redb | separate files, separate tables, one shared workspace redb version; only `uid_shard` is added to `index.redb` |

## 8. Diagnostics allocated in this RFC

Range SMY-E/W500–509, registered in the set's registry (SMYSL-2.0 §6.1). The `Code` enum in
`crates/smysl-core/src/diag.rs` is `#[non_exhaustive]`, so adding codes is additive. (The
`spec-tables` gate checks the format's constants against the spec's tables, not diagnostics.)

| code | severity | meaning | where |
|---|---|---|---|
| SMY-W500 | warn | a reference names a uid in a shard not loaded in this union; it waits, as an early attestation does | `report_dangling` on a `Union` (§3.7) |
| SMY-W501 | warn | the unit store does not describe this log (length, hash or layout); rebuilt from the log | `RedbStore::open` in index mode (§3.5.5) |
| SMY-E502 | — | *withdrawn: the condition is `SMY-E445` (SMYSL-2.4). The number stays unused.* | — |
| SMY-E503 | error | unit store layout version not supported by this build (index mode: delete and rebuild) | `RedbStore::open` (§3.9) |
| SMY-W504 | warn | the unit store is locked by a writer; read from the CBOR log instead | reader fallback (§3.8) |
| SMY-E505 | error | a primary-mode shard and its export disagree on the record-set digest | `lib verify`, `lib migrate` (§3.5.5) |

SMY-*506–509 are unallocated.

## 9. Open questions

| id | question | proposal |
|---|---|---|
| ~~**OQ-16**~~ (draft 3) | Virtual union: are contentions computed lazily per query enough, or does the library need a materialised contention index? | **Decided 2026-10-26 (P13c.2): lazy**, as proposed. Detection reads only the interpretive layer, which the union materialises (§3.7, after two detectors were made to); GE-T7's stand-in measured it at 10⁶ units in 1,001 shards at 444 ms over the materialised store, 638 ms over the in-memory union and 638 ms over the redb union — under the 1 s that would have called for an index. `Union::detected()` keeps the answer. |
| **OQ-65** | Generalise the public entry points in place (`fn check<S: StoreRead + ?Sized>`), accepting that a caller's own `Deref<Target = Store>` type stops coercing, or add `_in` twins for the twelve contract operations and leave their signatures alone? | In place, with blanket impls and the §5.2 compile test; twins only if `make semver` flags it or the owner rules the `Deref` case a break. |
| ~~**OQ-66**~~ | **Answered 1.10.0 by measurement, and the second option never existed.** There was no 1.79 build to keep: a 1.79 Cargo cannot resolve this tree at all — the last sentence of the old answer asked exactly that, and the answer is no, and not because of an *optional* dependency but because of `blake3`, via `constant_time_eq` 0.4.2 at edition 2024. Measured floors: 1.85 for the nine pure-path crates, 1.86 for `smysl-provider` and `smysl-ingest` (`icu_*` 2.2 under `ureq`), 1.88 for `smysl-tui` and the facade (`instability`, `darling` under `ratatui`). Each is declared on its own crate, since Cargo has no per-feature MSRV and a package's number must be its maximum. `make msrv` compares each declaration against what its dependencies require and fails in both directions; a CI job runs it and compiles the base tier at 1.85. redb 4.x's 1.90 is now a one-line bump on `smysl-graph` when `store-redb` lands. |
| ~~**OQ-67**~~ | **Answered 2026-10-24 (P13b.1): wait, then read the log. Revisited at P13b.2 and kept**: the CLI's writer holds the store for one append (6–8 ms), so a reader meets it rarely, and in one process readers and the writer share the database and never meet it at all; at 10⁶ units the fallback costs a 2.9 GB in-memory open against 0.3 s, which is what redb 4's read-only handles would buy, still at Rust 1.90. On redb 2.6.4, which the workspace pins, there is no shared lock to adopt — every open takes `flock(LOCK_EX)`, readers included — so the fallback serves any second process, not only one meeting a writer. `AnyStore::open_redb` waits `reader_wait_ms` (2 s), then opens the log into memory with `W504`. redb 4's read-only handles are the way out and cost Rust 1.90 for the whole workspace; revisit at P13b.2, where a writer makes collisions likelier and the 10⁶ ladder shows what the fallback costs. | *(was: wait and fall back now; revisit when the feature leaves `experimental`)* |
| **OQ-68** | Where do library-wide derived tables live: `uid_shard` and the `observed` instant index (draft 3 §14.2 lists `observed` in `index.redb`; this RFC has a per-shard `observed` table for `StoreRead`)? | **Half answered 2026-10-24.** Per-shard tables are the source, and P13b.1 has them (`observed`, `source_ref`). The library-wide ones go in **a file of their own, `store/library.redb`**, not `index.redb`: `smysl-text` owns that file's layout and writes it only under `substrate-redb`, and `smysl-graph` sits below `smysl-text`, so writing into it would be a crate writing a file whose layout belongs to one above it — and a library without text indexing would have no file to write to. Derived from the shards' `header` tables and rebuildable. Decided for P13c, which first needs them; SMYSL-2.4 to confirm. **Answered at P13c.1 (2026-10-25), differently:** `uid_shard` is a file **per shard**, `index/shards/<alias>.uids`, not a table in `store/library.redb`. A library must answer it without `store-redb` too, so it cannot live in redb alone; a write touches one shard, so one file per shard is rewritten and the others are not; and a reader merges the lists, which is the multimap. Each is stamped with the length of the log it lists — append-only, so the length is the log — and one that is stale or missing is rebuilt from its log. The `observed` index stays per shard, where P13b.1 put it; a union merges it (`units_in_observed_order`). |
| **OQ-69** | Tiering policy: pin interpretive cores by status (`speculative`/`inferred`/`derived` + relation endpoints, §3.5.4), by source kind (`doc`/`file`/`node` cold), or by kernel type? And GE-T7's "library build" — does it include model extraction or only readers, segmentation and indexes? | **Tiering answered 2026-10-24 (P13b.1): by status and relation endpoint**, a pure function of the header, as proposed; the paged tier is scan-resistant (scans read through, `get` fills), and its default is **32 MiB**, not 64, measured (§6.1 P13b.1). The cache at 0 bytes is a test mode beside the default, so answers are shown not to depend on it. GE-T7's build definition stays open: it does not bear on 13b, and the proposal (exclude model extraction) stands until the first GE-T7 run. |
| **OQ-90** | One redb unit store per shard, beside its log (P13c.1's as-built), or one per library? | **Decided 2026-10-26 (P13c.2), from GE-T7's stand-in: one per library**, with a set of tables per shard, as step P13c.3 and before P13d makes these files the record. Per shard, a store's fixed cost is paid a thousand times: over 1,001 shards the union opens at 1.85 GB where one 10⁶-unit store opens at 528 MB, `check` peaks at 2.7 GB (GE-T7: ≤ 1 GB), and the stores are 3.5 GB on disk for 164 MB of logs, 3.7 MB each for a 168 KB log. It grows with the number of expressions, not units, so a library of small ones — a chat import, a channel per expression — is the worst case. The cost of one database: redb takes one write transaction per database and its file lock admits one process, so two shards can no longer be written at once from two processes, where each has its own lock file today; a reader in another process already falls back to the logs (`SMY-W504`). Packing only the small shards together was the cheaper alternative, and leaves two layouts to keep. **Done at P13c.3 (2026-10-26), and narrowed by measuring first: one *store* per library, not one database of per-shard tables.** Split across 1,001 shards the redb side opened at 1.19 GB, of which the shard stores were 868 MB (one store of the same units: 528 MB) and the `Union` over them the rest — its own uid list, joined layers and an adjacency built over theirs. Tables per shard in one file would have saved part of the 340 MB; one store saves the union too. Built: `index/units.redb`, layout 2 (§3.5.3's as-built note); 277 MB to open, 540 MB on disk, `check` 1.69 GB. Not ≤ 1 GB: of that peak the store is about 775 MB and the `time` pass alone 1.41 GB (§6.1). After rule E's network was made to hold only the subjects anything bears on (§6.1), `check` is 885 MB: met. |
