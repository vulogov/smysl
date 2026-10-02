# RFC SMYSL-2.0 — Index and roadmap

**Status:** draft 1, for discussion. Non-normative: this document organises the set.
**Author:** Vladimir Ulogov
**Date:** 2026-10-02
**For:** crate `1.9.0-dev` (`d25ec9e`), format `smysl/1.0`, kernel `smysl.kernel/0.1`.
**Supersedes:** RFC SMYSL-2 draft 3 (`RFC_SMYSL-2_TEXT_CORPORA_DRAFT3.md`) as the working
document. Draft 3 remains the design record. Where this set and draft 3 differ, the set wins, and
§7 lists every difference.

---

## 0. What this set is

Draft 3 described using smysl as an instrument for text research:
- a library of books, articles and chats in five languages
- time and dating
- a query language and discovery dossiers
- information-theoretic and knowledge-assessment measures
- schema evolution
- import and export

It was one 2,700-line document. The evaluation of draft 3 recommended splitting it, running the
riskiest experiment first, and settling the decisions that fix identity before writing units at
scale. This set does that. It is **nine documents, one normative**:

| id | document | kind | phases | lines |
|---|---|---|---|---:|
| **2.0** | `SMYSL-2.0_INDEX_AND_ROADMAP.md` (this) | index | all | — |
| **2.1** | `SMYSL-2.1_HARDENING.md`: fixes to today's tree | implementation | TX-P0 | 954 |
| **2.2** | `SMYSL-2.2_EXTRACTION_SPIKE.md`: extraction and proposition spike | experiment protocol | S0 | 587 |
| **2.3** | `SMYSL-2.3_FORMAT_AMENDMENT.md`: library, time, schema evolution, round trip | **normative** | per amendment | 827 |
| **2.4** | `SMYSL-2.4_LIBRARY_AND_INGEST.md`: library, readers, time engine, ingest, linking, propositions, languages | implementation | TX-P1–P5, TX-P7 | 1,199 |
| **2.5** | `SMYSL-2.5_QUERY_AND_DOSSIERS.md`: `sq`, dossiers, thread schemas, schema-evolution tooling, embeddings | implementation | TX-P6, TX-P8 (dossiers), TX-P10, TX-P12 | 1,216 |
| **2.6** | `SMYSL-2.6_MEASURES_AND_ASSESSMENT.md`: measures, assessment | implementation | TX-P8 (T1–T7), TX-P9 | 901 |
| **2.7** | `SMYSL-2.7_IMPORT_EXPORT.md`: `lib import` / `lib export`, round trip | implementation | TX-P11 | 993 |
| **2.8** | `SMYSL-2.8_SCALE.md`: `StoreRead`, shards, disk-backed store | implementation | TX-P13 | 1,080 |

**Rules of the set:**
- **Only 2.3 is normative.** Implementation RFCs refer to it and do not redefine records, keys,
  identities, rules or enumeration codes. When 2.3 is accepted, its amendments are folded into
  `SMYSL_FORMAT_SPEC.md` one at a time, as their phases land (2.3 §1.2).
- **Every claim about today's code was checked against `d25ec9e`** by the document that makes it,
  with file and function cited. Items that could not be checked are marked *(unverified)* in
  place.
- **Numbering continues draft 3's.** FC-1 to FC-9, records 14–19, GE-T1 to GE-T16, OQ-1 to OQ-28,
  and TX-P0 to TX-P13 mean what they meant there. New diagnostics and open questions take
  per-document ranges (§6).
- **No AI attribution** appears in any document of the set. Authorship is the owner's.

**Reading order:**
- Read 2.0, then 2.3, then the RFC for whatever is being built.
- 2.1 and 2.2 can be read alone. They are the first things to do.

---

## 1. The path

```
   owner accepts 2.3 A-4, A-8.1, A-9, A-10 (items 2–3)   ← needed before 1.9.0 ships
            ┌─────────────── weeks 1–2 ───────────────┐
            │  TX-P0  (2.1)  ‖  S0 spike  (2.2)        │   owner reviews the rest of 2.3
            └──────────────┬──────────────┬────────────┘
                           │              │
                    gate G0: spike decision table (2.2 §3.7) + 2.3 accepted
                           │
            TX-P1 → TX-P2 → TX-P3 → TX-P4 → TX-P5          (2.4)   library, time, index, ingest
                                              │
                         ┌────────────────────┼───────────────────┐
                       TX-P6 (2.5)          TX-P7 (2.4)           TX-P13a (2.8)*
                       sq core             propositions,          StoreRead in memory
                         │                 linker, summaries      (needs only TX-P0)
                         ├──────────┬─────────┘
                       TX-P8      TX-P11 (2.7)    TX-P10 (2.5)
                       dossiers   import/export   embeddings, NL front end
                       (2.5), T1–T7 (2.6)
                         │           │
                       TX-P9 (2.6)  gate G2: GE-T15 (round trip exact)
                       TX-P12 (2.5)  │
                       schema       TX-P13d (2.8): disk-primary
                       evolution

   TX-P13b (2.8) redb as an index: after TX-P4 (redb, MSRV)
   TX-P13c (2.8) shards and union: after TX-P5 (library shards) and TX-P13b
```

\* SMYSL-2.8 splits TX-P13, and its gating is the one this set adopts:
- **13a** (fixes, the `StoreRead` trait, consumers ported in memory) needs only TX-P0. It can run
  in parallel with the library work.
- **13b** (redb as an index) and **13c** (shards and the virtual union) need the library's redb
  and shards (TX-P4, TX-P5).
- **Only 13d** (redb as the primary store) is gated on GE-T15.

**Gates:**

| gate | after | passes when | if it fails |
|---|---|---|---|
| **G−1** | before 1.9.0 is released | the owner accepts the parts of 2.3 that TX-P0 activates: A-4 (digest), A-8.1 (four enumerations opened, 255 reserved in five), A-9 (estimator key), A-10 items 2–3 (`lang` lint, strict `source`). Otherwise 1.9.0 ships wire behaviour no normative text covers. | TX-P0 ships without those items. |
| **G0** | S0 + TX-P0 | 2.2's decision table (§3.7) returns *continue* or a stated pivot, and the owner accepts the rest of 2.3 that TX-P1 to TX-P5 activate (A-1, A-2, A-3, A-5, A-6, the `admission` opening in A-8.1, A-11 `x.text/v1`, A-12 rules E and Z, A-13) | 2.2 §3.7 names the pivot: consensus extraction (`attested:2`), narrative out of scope, or reordered phases. 2.4 onwards is re-planned before TX-P1. |
| **G1** | TX-P5 | GE-T1, GE-T4, GE-T13 and GE-T14 pass inside 2.4's phases, and the TX-P5 cost report replaces the planning figures | units are not written at scale until fixed. This is the last cheap point to change identity (A-1, A-2). |
| **G2** | TX-P11 | GE-T15: P1, P2 and P3 hold on the whole matrix | TX-P13d does not start: export is the way out of a disk store and must be exact |
| **G3** | TX-P13d | GE-T7 at library scale | the CBOR log stays primary, and redb stays an index |

---

## 2. Phases and owners

| phase | RFC | delivers | exit (summary) | depends on |
|---|---|---|---|---|
| S0 | 2.2 | extraction stability and proposition-trust spike with today's CLI | 2.2 decision table | — |
| TX-P0 | 2.1 | F-2, F-3, F-4, F-6, F-12, F-13, F-14, F-16, F-17, F-18 and H-1 to H-20 | the probes reversed, F-16/F-17 reproductions fixed, full suite and `make semver` clean | G−1 for release |
| TX-P1 | 2.4 | parts, readings, manifests, object store, first readers, structure, locators; `admission` opened (2.3 A-8.1) | GE-T1 (Bibles and JSON) | TX-P0, G0 |
| TX-P2 | 2.4 | segmenter, analyzers, `lingua`, chat readers, `text append`, redaction (rule Z) | GE-T1 (chats), redaction survives a stale peer | TX-P1 |
| TX-P3 | 2.4 | EDTF, `published`, datings, rule E engine, locks, `date` commands | GE-T13 | TX-P2 |
| TX-P4 | 2.4 | substrate index, persistent postings, `text` commands | `find` p95 < 200 ms at 155k units | TX-P3 |
| TX-P5 | 2.4 | ingest over texts, owned-range spans, ledger digest, journal, FC-1/FC-3, entities without span | G1 | TX-P4 |
| TX-P6 | 2.5 | `sq` core (pure) | `--seed-check` on every pure query | TX-P5 |
| TX-P7 | 2.4 | same-as, `strict` classes, linker, summary tree | GE-T5, GE-T11 | TX-P5 |
| TX-P8 | 2.5 + 2.6 | dossiers, saved queries, thread schemas 6–8; measures T1–T7 | GE-T10 | TX-P6, TX-P7 |
| TX-P9 | 2.6 | assertion, channel, dialogue and temporal measures; `assess` | GE-T3, GE-T6 | TX-P8 |
| TX-P10 | 2.5 | S0/S1 embeddings, semantic sources and bridges, NL front end | GE-T5 cross-lingual arm, GE-T12 | TX-P6 |
| TX-P11 | 2.7 | `lib import/status/accept/discard/export/verify` | G2 (GE-T15) | TX-P6, TX-P7 (TX-P12 for `--target-revision`) |
| TX-P12 | 2.5 | FC-9 lenses, rule N checks, pinned queries, `migrate`, pointer lenses, `--target-revision` | GE-T16 | TX-P8 |
| TX-P13 | 2.8 | 13a trait in memory; 13b redb index; 13c shards and union; 13d disk-primary | G3 (GE-T7) | 13a: TX-P0. 13b: TX-P4. 13c: TX-P5, 13b. 13d: G2. |

---

## 3. Decisions taken for the set: owner checklist

These are **proposals**, applied consistently across the set. Each can be overruled. Each row
names where a change would land.

| # | decision | resolves | where it lives | if overruled, change |
|---|---|---|---|---|
| D-1 | the copy set on units: ref, span, manifest, published or observed, lang, speaker | OQ-17 | 2.3 A-2.4 | 2.3, 2.4 §3.4 |
| D-2 | `published`, `span`, `manifest` inside `source`; unknown source keys rejected; bare `lang:` reserved; lint W432 first | OQ-1 | 2.3 A-10, 2.1 | 2.3, 2.1 |
| D-3 | statuses of free constraints | OQ-10 | 2.3 A-12.2 | 2.3, 2.4 §3.3 |
| D-4 | a scholarly dating takes its basis's status; equal-status conflicts become contentions | OQ-12 | 2.3 A-12.2 | 2.3 |
| D-5 | closed lens op set, open tag space | OQ-23 | 2.3 A-12.1, App. B | 2.3, 2.5 |
| D-6 | holder, mode and speaker stay in `x.text/v1` | OQ-6 | 2.3 A-11 | 2.3 |
| D-7 | `strict` classes normative (seed-and-grow greedy partition) | OQ-7 | 2.3 A-12.4 | 2.3, 2.4 §3.6 |
| D-8 | conformance: C-Read additions, new **C-Library**, lenses not obligatory | — | 2.3 A-13 | 2.3 |
| D-9 | new rules N, E, Z | — | 2.3 A-12 | 2.3 |
| D-10 | security owners: prompt injection (2.1, 2.4), reader limits (2.4), concurrency (2.4, 2.8) | — | §5 | — |
| D-11 | property and fuzz harnesses in the same phase as each complex feature | — | §5 | — |
| D-12 | spike S0 before TX-P1; TX-P0 in parallel | — | §1 | — |
| — | open enumerations as a unit `Unknown` variant, code 255 reserved, not `Other(u8)` (keeps `as` casts compiling: no 2.0) | OQ-30 (part) | 2.3 A-8.1, 2.1 §4.3.6 | 2.1, 2.3 |
| — | estimator is a wire field (granularity key 5) | OQ-29 | 2.3 A-9 | 2.3, 2.1 |
| — | time contentions derived, not recorded; detection kinds 4 and 5 | OQ-35 | 2.3 A-8.2 | 2.3, 2.4 |
| — | a chain of constraints tightens at its weakest link's status | OQ-42 | 2.3 A-12.2 | 2.3, 2.4 |
| — | saved-query results by `x.query/answers`, not kernel `answers` | OQ-49 | 2.3 A-11 | 2.3, 2.5 |
| — | `converged_with` compares the record-set digest and `state_hash` | OQ-27 | 2.1 §4.3.10 | 2.1, 2.7 |
| — | the lens engine lives in `smysl-core` | OQ-48 | 2.5 | 2.5 |
| — | export inclusion table and P1–P3 are tool-level, owned by 2.7; 2.3 reserves `import:` (manifest aliases) and `export/` (view ids) | OQ-61 | 2.7, 2.3 A-5 | 2.7, 2.3 |
| — | one lens-chain hash, prefixed `smysl/lens-chain/1` | — | 2.3 App. B | 2.3, 2.5 |
| — | part bytes that do not match the tid: `SMY-E446`; `SMY-E401` keeps its draft-3 meaning | — | 2.3 A-3 | 2.3, 2.4 |
| — | `ingest --temperature` and the model in the recipe ship in TX-P0 | OQ-32, OQ-33 | 2.1 H-12, H-9 | 2.1, 2.2 |
| — | `find` and `pack` relabelled `Mixed`; `--seed-check` enforced | OQ-54 | 2.1 H-19, H-20 | 2.1, 2.5 |

---

## 4. Open questions, by the phase they block

Everything not listed here is either resolved (§3) or does not block anything.

| must be answered before | open questions | owner |
|---|---|---|
| **S0 / TX-P0** | OQ-31 estimator reference and objective (F-2 may slip to 1.10.0) · OQ-34 licensing of model outputs as fixtures | 2.1, 2.2 |
| **TX-P1** | **MSRV: OQ-40 = OQ-66** (redb 4.x needs Rust 1.90; the workspace declares 1.79 and does not test it). Proposal: raise `rust-version` to 1.90 at TX-P1, with an MSRV CI job. · OQ-36 lock mechanism · OQ-37 JSON parsing inside or outside the purity gate · OQ-39 physical erasure of text in plain logs | 2.4, 2.8 |
| **TX-P3** | OQ-13 speaker and device selectors for datings (window targets are already in) | 2.4 |
| **TX-P4** | OQ-4 source and licence of versification maps · OQ-14 default IDF scope | 2.4, 2.5 |
| **TX-P5 (G1)** | OQ-41 `ingest:quote` removed from span-carrying units changes their uids · OQ-43 strictness of the digest-scoped reference check | 2.4 |
| **TX-P6** | OQ-45 RRF constant · OQ-46 fuse or route `~hybrid` · OQ-47 is `question` interpretive · OQ-50 `match` subset · OQ-21 the name `sq` | 2.5 |
| **TX-P7** | OQ-44 auto-accept lexical same-as · OQ-15 partial dependence between witnesses | 2.4, 2.5 |
| **TX-P8 / P9** | OQ-52 dossier slot ranking · OQ-22 intent classifier · OQ-55 status and grounds of measure units · OQ-56 payload number layout · OQ-57 bootstrap generator · OQ-58 class identity across time · OQ-59 T3 bias correction | 2.5, 2.6 |
| **TX-P11** | OQ-60 what P1 compares · OQ-62 `--format pack` and `-o DIR` · OQ-63 clock of exported attestations · OQ-64 threads spanning files · OQ-25, OQ-26, OQ-28 | 2.7 |
| **TX-P12** | OQ-53 nested payload keys in lenses · OQ-24 always state the lens chain | 2.5 |
| **TX-P13** | OQ-65 generic entry points vs `_in` twins · OQ-67 cross-process readers · OQ-68 home of library-wide tables · OQ-69 tiering policy · OQ-16 lazy contentions (proposal: lazy) | 2.8 |
| **not blocking** | OQ-2 (stems only, settled in practice) · OQ-3 · OQ-5 · OQ-8 (2.4 stores 15 and 18 as objects) · OQ-9 (hand-rolled EDTF) · OQ-11 · OQ-18, OQ-19, OQ-20 · OQ-30 (rest) | various |

---

## 5. Cross-cutting concerns

### 5.1 Conformance and the other implementations

| class | what it gains | implementations |
|---|---|---|
| C-Read | records 14–19, key 9, source keys 4–6, schema-declaration and granularity key 5, unknown enumeration codes, tid/mid/did/rdid, record-set digest | Rust; then Python, JavaScript and Go: first target, about 15 lines each for the digest, plus records |
| C-Consume | the *said* axis; unknown enumeration values | Rust |
| C-Produce | copy and omission, `lang`, EDTF, rule N, canonical uids in payload | Rust |
| C-Merge | did-targeted withdrawals and commitments, rule Z, convergence by digest | Rust |
| **C-Library** | part and reading verification, rule Z, rule E, `strict` classes | Rust. A brute-force rule-E reference on small networks (2.4 §5.3, P-E6) is the test oracle. |

### 5.2 Security

| concern | owner | summary |
|---|---|---|
| prompt injection | 2.1 (today's ingest), 2.4 (text ingest, ledger digest, linker) | fences around untrusted text, explicit instructions, model references limited to the batch and digest (`E442`), model-supplied metadata dropped (`W444`), red-team fixtures |
| reader resource limits | 2.4 §3.9 | caps on decompressed size, entry count, nesting, part bytes, and a deterministic work budget. Exceeding a cap is an error, never a partial read. |
| concurrent writers | 2.4 (lock file, one writer per shard), 2.8 (redb, cross-process readers) | readers are lock-free on immutable objects |
| erasure | 2.3 rule Z, 2.4 | redaction survives merge. Older peers can still pass text on: stated. |
| licences | 2.4, 2.7 | `carry` and licence gate every export of text |

### 5.3 Property and fuzz harnesses (D-11)

| subject | harness | RFC |
|---|---|---|
| effective time (rule E) | P-E1–P-E8, with a brute-force reference on small networks | 2.4 |
| redaction merge (rule Z) | P-Z1–P-Z4 | 2.4 |
| readers | cargo-fuzz targets per reader | 2.4 |
| lens laws | forward and backward on invertible ops, composition, query compilation equivalent to migration (GE-T16) | 2.5 |
| inclusion rules and round trip | random stores → export → import → digest equality; fuzzed import inputs | 2.7 |
| `StoreRead` | one conformance suite run over every implementation, which must give identical bytes | 2.8 |
| bundle closure | no bundled record names an absent unit | 2.1 |

The repository has no `proptest` today. The RFCs use its existing convention (seeded generators
plus `cargo fuzz` targets under `fuzz/`) rather than adding a dependency.

---

## 6. Registries

### 6.1 Diagnostics

| range | owner | allocated |
|---|---|---|
| E401–W431 | draft 3 / 2.3 | as in draft 3 Appendix D (table below). 2.3 uses E401, E403, E404, W405, W409, E410–W414, W420, E426, W429, W430, and 2.4's E446. |
| W432–W439 | 2.1 | W432 bare `lang:` lint · W433 fence marker in input · W434 bundle unknown-record count · W435 repaired attempt history |
| E440–W459 | 2.4 | E440, W441, E442, W443, W444, E445, E446, W447, W448, W449, E450, W451 (452–459 free) |
| E460–W479 | 2.5 | E460, E461, E462, W463, W464, W465, E466, W467, E468, W469, E470, E471, W472, E473, E474, W475, W476, E477, W478, W479 |
| E480–E489 | 2.6 | E480, E481, E482, W483, E484, W485, W487, W488, E489 (E486 withdrawn: use E416) |
| W490–W499 | 2.7 | W490, W491, E492, E493, E494, W495, W496, W497, W498 (W499 reserved) |
| W500–W509 | 2.8 | W500, W501, E503, W504, E505 (E502 withdrawn: use E445) |

Each RFC's §8 has the meanings of its own range. The draft-3 codes, which every document uses,
are:

| code | meaning |
|---|---|
| E401 | structure hash or rdid mismatch on re-read (part bytes that do not match the tid are `E446`, 2.4) |
| E402 | `carry: text` refused by licence |
| E403 | malformed tid, mid, did or rdid, in a record or in `source.manifest` |
| E404 | span out of range |
| W405 | locator and span disagree (span wins) |
| W406 | holder or speaker not in `deps` |
| E407 | byte estimator on a `mul` scope |
| W408 | measure on a lossy text, when allowed |
| W409 | unknown enumeration value or estimator id preserved (thread schema, role, source kind, detection kind, admission; estimator) |
| E410 | malformed EDTF |
| W411 | `observed` outside the `published` interval |
| W412 | dating not applied (lower status than the value it would override, or held by a curated lock); contention opened |
| W413 | temporal inconsistency; contention opened |
| W414 | source redacted |
| W415 | units excluded from a time-scoped result for lack of a time on the requested axis (undated units; units with no known time under `--as-of`); count reported |
| E416 | `--seed-check` on a model-dependent query |
| W417 | ambiguous dossier subject |
| W418 | expression fork: two manifest heads |
| W419 | ambiguous attribution: the quote occurs more than once in its node |
| W420 | a lens cannot map a value; the unit is excluded from predicates on that field and counted |
| E421 | a query names a field that no schema in scope declares |
| E422 | a pinned query names a key or value retired without a mapping, or `lib export --target-revision` meets a `retire` on its lens path |
| W423 | an imported record failed a check and was quarantined |
| E424 | round-trip property violated (P1, P2 or P3); the export index is not written as complete |
| W425 | unknown records exported to `unattributed.cbor`, or dropped with `--unknown drop`; count reported |
| E426 | the naming rule is violated (§15.2): a name gets a different meaning without a paired widening, or a retired name is reintroduced |
| E427 | unsupported `sq` version |
| W428 | a surface export omitted records surface text cannot spell (F-18); count reported |
| W429 | schema fork: two declarations of one id and version with different content; contention opened |
| W430 | a `payload_shape` that does not match FC-9's structure; it is treated as opaque, as today |
| W431 | an extracted unit lacks a key required by the revision its recipe names |

**One condition, one code.** Two duplicates found in review were withdrawn: 2.6's E486 (use
E416) and 2.8's E502 (use E445).

**This registry is the set's.** The format spec does not carry a diagnostics table, and its
`spec-tables` gate checks format constants, not diagnostics. Codes enter the `Code` enum in
`smysl-core/src/diag.rs` (`#[non_exhaustive]`, so additions are additive) as their phase
lands.

### 6.2 Open questions

| range | owner |
|---|---|
| OQ-1–OQ-28 | draft 3. Status per question in §3 and §4. |
| OQ-29–OQ-31 | 2.1 |
| OQ-32–OQ-34 | 2.2 |
| OQ-35–OQ-44 | 2.4 |
| OQ-45–OQ-54 | 2.5 |
| OQ-55–OQ-59 | 2.6 |
| OQ-60–OQ-64 | 2.7 |
| OQ-65–OQ-69 | 2.8 |

### 6.3 Format registry

2.3 Appendix A is authoritative: records, keys, identities, open enumerations, new codes,
estimator ids and rules.

---

## 7. Differences from draft 3 (errata and refinements)

Found while writing the set. **The set is authoritative where it differs.**

| # | draft 3 said | the set says | found by |
|---|---|---|---|
| E-1 | §2.2 scale probe: "surface → CBOR" times and "store" sizes | **`fmt --format cbor` writes surface text** (the flag is ignored, H-17), so those figures measured surface parsing and surface files. Re-measured in 2.8 §2: about 2.6 KB per unit resident, every unit core held twice, and loading about 90% of `check`'s time. | 2.8, 2.1 |
| E-2 | export views `v/export/<key>` | that is not a legal view id (surface parsing silently turns it into `v/doc`; CBOR rejects it). Use `export/<slug>`. | 2.7 |
| E-3 | GE-T15 uses fixtures F1–F13 in `fixtures/corpus` | there is no F10 there. It is `fixtures/wire/F10-lifecycle.cbor`. | 2.7 |
| E-4 | open enumerations as `Other(u8)` | a unit variant `Unknown = 255`. `Other(u8)` breaks downstream `as` casts, which the project treats as a 2.0. | 2.1 |
| E-5 | an `Absent` quote caps the unit at `inferred` | `inferred` needs grounds. Without them, the cap is `speculative` (`W441`). | 2.4 |
| E-6 | saved-query results linked by kernel `answers` | `x.query/answers`: kernel edges feed salience | 2.5 → 2.3 |
| E-7 | time contentions materialised as contentions | derived and reported, not recorded (2.3 A-8.2) | 2.4 → 2.3 |
| E-8 | "no `cc`" for the pure-Rust stack | `blake3`'s `pure` feature still lists `cc` as a build dependency *(whether it compiles C is unverified)*. Today the `remote` feature already needs a C compiler and `semantic` a C++ compiler. The claim is narrowed to "no new C/C++ in the default build", with a CI check. | 2.5, 2.2 |
| E-9 | `bundle` loses commitments, schema declarations and unknown records (F-16) | it also loses attestations on relations, and units reachable only as notes, reasons or contention positions (H-14, H-15) | 2.7 → 2.1 |
| E-10 | `--seed-check` asserts reproducibility | it is accepted but read by nothing. `find` and `pack` are labelled pure even with model-dependent engines (H-19, H-20). | 2.5, 2.6 → 2.1 |
| E-11 | ingest staging reused for imports | ingest staging rewrites units and loses records, so import staging is CBOR-only | 2.7 |
| E-12 | `strict` classes informally specified | a seed-and-grow greedy partition in uid order (2.3 A-12.4) | 2.4, 2.3 |
| E-13 | the toolchain was not considered | MSRV: redb 4.x needs 1.90, quick-xml 0.42 needs 1.86, unicode-segmentation 1.13 needs 1.85. The workspace declares 1.79 and does not test it (OQ-40 = OQ-66). | 2.4, 2.8 |
| E-14 | `derive_thread` scales | it is quadratic: 24.5 s at 171k units, 0.55 s after a one-arm fix with byte-identical output (H-18) | 2.8 → 2.1 |

**Defects in today's tree** that the set documents and 2.1 fixes:
- F-2, F-3, F-4, F-6, F-12, F-13, F-14, F-16, F-17 and F-18
- H-1 to H-20, including the six operational issues the spike found:
  - `-C/--config` is ignored
  - the model is not in the recipe
  - Ollama's `num_ctx` is not sent
  - repaired failures leave no trace
  - there is no temperature control
  - `--yes` does not commit

**2.1 is worth shipping on its own merit.**

---

## 8. Size and effort

These figures are rough, extrapolated from the current tree: about 69k lines of Rust including
tests. Releases 0.6 (31 July) to 1.8 (22 September) were built at roughly 35k lines a month.

| RFC | new and changed code, incl. tests | notes |
|---|---:|---|
| 2.1 | 4–6k | about 30 fixes, most of them small, with tests |
| 2.2 | throwaway scripts | about 7 days, about 6 hours per annotator, 1.3–1.9M tokens (2.2 §6) |
| 2.4 | 18–25k | the largest: readers, time engine, ingest, propositions |
| 2.5 | 12–18k | parser, planner, lenses, dossiers, embeddings |
| 2.6 | 7–10k | estimators, graph measures, assessment |
| 2.7 | 5–8k | selection, layouts, index, verify, matrix tests |
| 2.8 | 6–9k | trait, consumer ports, redb store, union |
| **total** | **≈ 52–76k** | roughly doubles the codebase |

**Calendar:** at the historical pace, 1.5–2.5 months of coding. The binding constraint is the
experiments, not the coding:
- gold sets and annotators (GE-T9, the spike's same-as judging)
- multi-model extraction runs (GE-T2)
- the 1,000-work scale test (GE-T7)

Coding speed cannot compress those.

---

## 9. Next actions

1. **Start TX-P0** from 2.1's ordering (12 steps, plus 11a–11c added for H-8 to H-20). Release as
   1.9.0. F-2 may slip to 1.10.0 if OQ-31 blocks.
2. **Run S0** from 2.2 in parallel. It needs no code beyond 2.2's scripts. Two of its
   workarounds disappear when 2.1 lands: `-C` being ignored, and the model missing from the
   recipe.
3. **Owner review of 2.3 Appendix E and §3 of this document.** Confirm or overrule each decision.
4. **Answer the TX-P1 blockers** in §4, MSRV first.
5. **At G0, re-plan 2.4 if the spike's decision table (2.2 §3.7) says so**, then begin TX-P1.
