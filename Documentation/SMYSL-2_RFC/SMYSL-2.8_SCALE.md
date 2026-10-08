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

#### 3.5.3 Tables

All keys big-endian; all values deterministic CBOR or fixed-width. `seq` is the record's
ordinal in the log.

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
  64 MiB), on top of redb's page cache (`Builder::set_cache_size`).
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

**Contentions over a union (OQ-16).** Detection is already "reported, not recorded"
(`merge/mod.rs`, the comment after `new_contentions`): `contention::detect` is a function of
the store, re-run on every merge. Its four detectors read `relations_of_kind(Supersedes)`,
live rebuttals, label bindings and commitments — all in the interpretive layer, which the
union materialises. So **lazy per-query detection is the proposal**: `detect(&union, ctx)` at
query time, cached on the union instance. A materialised contention index is added only if
GE-T7 shows detection over the union's interpretive layer above 1 s at 10⁶ units. Position on
OQ-16 recorded in §9.

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
  will not tell you who holds a flock.
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
  `AnyStore`. Surface input and stdin always load into memory.
- Global `--shard <ALIAS>` (repeatable) on read commands inside a library: the union of the
  named shards; `--shard all` is every shard.
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
  ["std"] }` — shared with SMYSL-2.4's `substrate-redb`.
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
   order for k ≤ 3 and 5 seeded orders for k = 7;
7. `Overlay` of `Store::from_records(R₁)` and batch R₂ (R = R₁ ⊎ R₂, three split points).

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
| P13c.2 | contentions over the union, measured | GE-T7 run; OQ-16 decided from its contention-detection number |
| **P13d** | primary mode, `lib migrate`, `E505` — **only after GE-T15 passes** | suite implementation 5; export → import round trip reproduces the record-set digest on every GE-T15 corpus; GE-T7 rerun on primary shards |

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
| **OQ-16** (draft 3) | Virtual union: are contentions computed lazily per query enough, or does the library need a materialised contention index? | Lazy. Detection reads only the interpretive layer, which the union materialises (§3.7). Materialise only if GE-T7 measures detection over the full union above 1 s at 10⁶ units. |
| **OQ-65** | Generalise the public entry points in place (`fn check<S: StoreRead + ?Sized>`), accepting that a caller's own `Deref<Target = Store>` type stops coercing, or add `_in` twins for the twelve contract operations and leave their signatures alone? | In place, with blanket impls and the §5.2 compile test; twins only if `make semver` flags it or the owner rules the `Deref` case a break. |
| ~~**OQ-66**~~ | **Answered 1.10.0 by measurement, and the second option never existed.** There was no 1.79 build to keep: a 1.79 Cargo cannot resolve this tree at all — the last sentence of the old answer asked exactly that, and the answer is no, and not because of an *optional* dependency but because of `blake3`, via `constant_time_eq` 0.4.2 at edition 2024. Measured floors: 1.85 for the nine pure-path crates, 1.86 for `smysl-provider` and `smysl-ingest` (`icu_*` 2.2 under `ureq`), 1.88 for `smysl-tui` and the facade (`instability`, `darling` under `ratatui`). Each is declared on its own crate, since Cargo has no per-feature MSRV and a package's number must be its maximum. `make msrv` compares each declaration against what its dependencies require and fails in both directions; a CI job runs it and compiles the base tier at 1.85. redb 4.x's 1.90 is now a one-line bump on `smysl-graph` when `store-redb` lands. |
| **OQ-67** | Cross-process readers: wait-then-read-the-log (this RFC), or adopt redb's `experimental-multiprocess` single-writer mode? | Wait and fall back now; revisit when the feature leaves `experimental`. |
| **OQ-68** | Where do library-wide derived tables live: `uid_shard` and the `observed` instant index (draft 3 §14.2 lists `observed` in `index.redb`; this RFC has a per-shard `observed` table for `StoreRead`)? | Per-shard tables are the source; SMYSL-2.4's library-wide `observed` and this RFC's `uid_shard` are derived from them, in `index.redb`. SMYSL-2.4 to confirm. |
| **OQ-69** | Tiering policy: pin interpretive cores by status (`speculative`/`inferred`/`derived` + relation endpoints, §3.5.4), by source kind (`doc`/`file`/`node` cold), or by kernel type? And GE-T7's "library build" — does it include model extraction or only readers, segmentation and indexes? | Status-based pinning, default core cache 64 MiB; GE-T7 build excludes model extraction (it is measured by TX-P5's cost report). Both to be revisited after the first GE-T7 run. |
