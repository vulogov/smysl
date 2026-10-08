# RFC SMYSL-2.3 — Format amendment: library, time, schema evolution, round trip

**Status:** draft 1. **Partly accepted — normative for the accepted amendments**, which are
folded into `SMYSL_FORMAT_SPEC.md`, section by section, as the phases that need them land. The
amendments nothing has activated yet remain for discussion.

| accepted | at | amendments |
|---|---|---|
| 2026-10-02 | **G−1**, before 1.9.0 | A-4, A-8.1 (four enumerations opened, 255 reserved in five), A-9, A-10 items 2–3 |
| 2026-10-07 | **G0**, after S0 and TX-P0 | A-1, A-2, A-3, A-5, A-6, the `admission` opening in A-8.1, A-11 `x.text/v1`, A-12 rules **E** and **Z**, A-13 |
| still for discussion | — | A-7, A-10 (the rest), A-11 `x.query/v1` and uid-typed payload keys, A-12 rule **N**, proposition classes and time contentions, A-14 |

The G0 acceptance carries exactly the Appendix E decisions that sit inside the amendments it
names: **D-1** (copy set, OQ-17), **D-3** (statuses of free constraints, OQ-10), **D-4**
(scholarly datings take their basis's status, OQ-12), **D-6** (holder, mode and speaker stay in
`x.text/v1`, OQ-6), **D-8** (conformance mapping), **D-9** for rules **E** and **Z** only, the
chain-tightening rule (OQ-42, A-12.2 step 3), and the `admission` half of the code-255 decision
(OQ-30). It does **not** carry D-5 (OQ-23) or D-7 (OQ-7), which live in A-12.1 and A-12.4 — rule
N and the proposition classes — nor OQ-35 or OQ-49, which live in A-8.2 and A-11 `x.query/v1`.
Those stay open. D-2 (OQ-1) and the estimator decision (OQ-29) were already accepted at G−1.

A-14's registry and activation text is appended to the spec as each accepted amendment folds in,
so A-14 itself stays open until the last fold; §8.1 and §8.3 already list the instances the
accepted set creates.

Acceptance makes these amendments normative text. It does not schedule them: each still activates
in the phase its §1.2 row names, and folding one into the spec is not complete until
`make spec-tables` covers its tables (A-14).

**Folded so far**, in 1.10.0. A fold is the amendment's text in `SMYSL_FORMAT_SPEC.md` *and* the
`spec-tables` gate reading its constants out of the document (A-14); anything less leaves the
document and the wire free to drift, which is the condition §2.2 of the spec records four times.

| amendment | spec section | gate covers |
|---|---|---|
| A-4 record-set digest | new §2.7 | the prefix `smysl/rsd/1` and the domain byte, against the one function that computes it |
| A-8.1 opening | §3.1, §8.3 | the reserved code, against the `Unknown` discriminant of each enumeration that has one |
| A-9 estimator | §3.1, §8.1 | the registry's ids against `TokenEstimator::id`, and granularity key 5 against `keys::granularity` |
| A-10 item 3 strict `source` | §4, §8.3 | — (a parser rule, with no constant to compare) |

Those are the amendments whose phase has landed: TX-P0 shipped in 1.9.0, so its four fold now.
The rest wait for theirs, and the spec says nothing about them in the meantime — which is correct
rather than a gap, since spec §8.1 makes every one of them an addition an older reader already
round-trips.

**§2.6 is held, not skipped.** A-3's library identities were given it, and A-4 was given §2.7;
folding A-4 first would have renumbered A-3's section if §2.6 were filled in, so the spec carries
§2.6 as a held heading naming what arrives there.

**Author:** Vladimir Ulogov
**Date:** 2026-10-02
**Part of:** RFC set SMYSL-2. See SMYSL-2.0 for the index and roadmap.
**For:** crate `1.9.0-dev` (`d25ec9e`), format `smysl/1.0`, kernel `smysl.kernel/0.1`.
**Derived from:** RFC SMYSL-2 draft 3, §2.2–2.6, §4.2–4.3, §4.8, §6, §7.1–7.4, §7.9, §9.1, §12.4,
§15.2–15.3, §16.8, §21 and Appendices A and D. Also the decisions D-1 to D-12 of the set, and
findings reported by SMYSL-2.1, 2.4, 2.5 and 2.7.
**Phases:** each amendment states the phase that activates it (§1.2).

---

## 0. Summary

This document is the **only** normative text in the SMYSL-2 set. The implementation RFCs
(2.1, 2.4–2.8) refer to it and do not redefine anything it defines. It amends
`SMYSL_FORMAT_SPEC.md` in place: each amendment below names the spec section it changes and gives
the text to insert or replace.

**Every amendment is an addition that spec §8.1 permits, or a tightening under spec §8.3.** The
format version stays `smysl/1.0`. The test is the spec's own: a reader at the older revision must
round-trip a document containing the addition, byte for byte. Where that test fails for a
specific older reader, the amendment says so and states the cost.

| # | amends | what | draft-3 id | activated in |
|---|---|---|---|---|
| A-1 | spec §2.2 | unit core key 9 `lang` | FC-1 | TX-P5 |
| A-2 | spec §2.2 | `source` keys 4 `published`, 5 `span`, 6 `manifest`, and the copy and omission rules | FC-2, FC-3 | TX-P3 (4), TX-P5 (5, 6) |
| A-3 | new spec §2.6 | library identities: tid, mid, rdid, did | — | TX-P1 (tid, mid, rdid), TX-P3 (did) |
| A-4 | new spec §2.7 | record-set digest | — | TX-P0 |
| A-5 | spec §3.1 | records 14 manifest, 15 part text, 17 dating, 18 part reading, 19 redaction; 16 reserved | FC-4 | TX-P1 (14, 15, 18), TX-P2 (19), TX-P3 (17) |
| A-6 | spec §3.1 | withdrawal and commitment may name a did | — | TX-P3 |
| A-7 | spec §3.1 | schema declaration: key 4 `payload_shape` structure, key 5 `lenses` | FC-9 | TX-P12. Until then, ingest resolves uid-typed payload keys itself (SMYSL-2.4 §3.4). |
| A-8 | spec §3.1 | open enumerations: code 255 reserved; new codes for thread schemas, roles, detection kinds | FC-8 | TX-P0 (opening four; reserving 255 in five), TX-P1 (opening `admission`), TX-P8 (codes) |
| A-9 | spec §3.1 | granularity key 5 `estimator`; estimator registry | FC-6 | TX-P0 |
| A-10 | spec §4 | surface: reserved words, `lang`, `source` keys, `@schema` fields and lenses, strict `source` | — | per construct |
| A-11 | spec §5 | standard extension schemas `x.text/v1`, `x.query/v1`; uid-typed payload keys | FC-5 | TX-P5, TX-P8 |
| A-12 | spec §6 | rules **N** (naming), **E** (effective time), **Z** (redaction); proposition classes; time contentions | — | TX-P2 (Z), TX-P3 (E), TX-P7 (classes), TX-P12 (N) |
| A-13 | spec §7 | conformance: C-Read additions, new class **C-Library**, C-Produce and C-Merge additions | — | per phase |
| A-14 | spec §8 | registry and activation | — | at acceptance |

**Not in this document, by design:**
- Normaliser V2 (FC-7) and the quote check are library contract (`API_CONTRACT.md`), not format.
  SMYSL-2.1 defines them.
- The export inclusion table and the round-trip properties P1–P3 are **tool-level**. They define
  what `lib export` and `lib import` must do, not what a store is, and SMYSL-2.7 owns them
  (resolving OQ-61 there). This document defines the record-set digest they are measured with
  (A-4), the records an import manifest uses (A-5), and the alias grammar (A-5).

---

## 1. Conventions

### 1.1 Language

MUST, MUST NOT, SHOULD and MAY are used as in the spec. "Older reader" means an implementation
built against the spec before the amendment. Byte layouts use the spec's notation: integer-keyed
CBOR maps, ascending key order, absent optional fields omitted, never `null` (spec §3).

### 1.2 Activation

An amendment is **active** in an implementation from the release that ships the phase named in
§0's table.
- Before activation, an implementation MUST NOT **emit** the construct.
- It MUST still **preserve** the construct if it receives it. Rule X already requires this for
  unknown keys and record types. A-8 makes it true for unknown enumeration codes.

Amendments therefore land one at a time. No release has to implement this whole document.

### 1.3 What older readers do

| construct | reader ≥ 1.9 with SMYSL-2.1's fixes | reader 1.7–1.9 as shipped | reader < 1.7 |
|---|---|---|---|
| unit core key 9 | preserved | preserved | preserved |
| `source` keys 4–6 | preserved | preserved in CBOR. **Dropped by the surface parser**, which changes the uid silently (SMYSL-2.1, F-13). | dropped, so a different uid |
| records 14–19 | preserved (`SMY-W014`) | preserved (`SMY-W014`) | preserved (`SMY-W014`) |
| enumeration value unknown to the reader | preserved as `Unknown` (`SMY-W409`) for thread schema, role, source kind and detection kind; admission from 1.10 | **store fails to open** (F-12) | same |
| granularity key 5 | preserved | **dropped** by `dec_granularity` (SMYSL-2.1, H-1) | dropped |
| schema declaration key 5 | preserved (`extra`) | preserved | preserved |

Two rows carry a real cost for 1.7–1.9 readers: unknown enumeration values, and dropped
granularity and surface `source` keys. SMYSL-2.1 (TX-P0) fixes both before anything in this
document is emitted. That ordering is mandatory: **no producer may activate A-1, A-2, A-8 (new
codes) or A-9 before it ships SMYSL-2.1's fixes.**

---

## A-1. Unit core key 9 `lang` (spec §2.2)

Insert into the unit core table, after key 8:

| key | field | type | presence |
|---:|---|---|---|
| 9 | lang | text: a BCP-47 tag, lowercase, NFC | optional |

Insert after the table:

> `lang` (key 9) is the language the unit's text fields are written in. An absent `lang` means
> "the language of the view that presents the unit" (view key 6), which is what every unit meant
> before 1.10. Absent and present are different unit cores and different uids: the language a
> gist is written in is part of what it says. Producers SHOULD write `lang` on every unit whose
> text fields are natural language. A unit whose fields are not natural language (identifiers,
> data) omits it. `und` and `mul` are permitted values. A tag that is not well-formed BCP-47 is
> `SMY-E001` at the surface and a decode error in CBOR.

---

## A-2. `source` keys 4–6 and the copy and omission rules (spec §2.2)

### A-2.1 Table

Insert into the `source` table, after key 3:

| key | field | type | presence |
|---:|---|---|---|
| 4 | published | text: EDTF, ISO 8601-2 level 1 (A-2.3) | optional |
| 5 | span | array of two unsigned integers `[start, end]`, `start < end` | optional; requires key 1 to name a part (A-2.2) |
| 6 | manifest | 32 bytes: a mid (A-3) | optional; required when key 5 is present |

### A-2.2 Passage provenance

> When a unit is drawn from a part (A-5, record 15), its source has `kind: doc` (4), and its
> `reference` (key 1) is the part's tid in text form, optionally followed by `#` and a locator
> alias: `t3:<52 chars>[#<locator>]`. `span` (key 5) is a pair of byte offsets into the part's
> normalised bytes, half-open. `manifest` (key 6) names the manifestation the unit was extracted
> under.
>
> The span is authoritative. The locator is informative: when it disagrees with the span, the
> span wins and `SMY-W405` is reported. A span past the part's length is `SMY-E404`. A reference
> with `t3:` that is not a well-formed tid is `SMY-E403`.
>
> No new source kind is introduced. A new `SourceKind` value makes a store unreadable to readers
> that predate SMYSL-2.1 (F-12). New keys in the source map do not.

### A-2.3 `published`

> `published` is the publication time of the manifestation the unit was drawn from, **as the
> source states it**: a title page, a byline, a dateline, an export header. It is EDTF level 1
> text. The format stores the string. It defines no calendar arithmetic beyond what A-12 rule E
> needs to order intervals. A malformed value is `SMY-E410`.
>
> EDTF is proleptic Gregorian. A manifest that records a date in another calendar says so (A-5,
> manifest key 18). The value is stored **as written** and converted only when intervals are
> derived. A unit copying such a date carries the as-written value too. Its calendar is the
> calendar of the manifest it names (key 6).

`observed` (key 3) keeps its 1.8 meaning unchanged: an instant recorded by a system, supplied and
never read from a clock. `captured` (key 2) keeps its meaning: when the operator obtained the
file. The three fields answer different questions, and an implementation MUST NOT derive one from
another when writing.

### A-2.4 Copy and omission rules (normative)

`source` is inside identity, so these rules decide uids. They depend only on the unit and the
manifest it names, never on what else a store holds.

1. **The manifest is named.** A unit carrying `span` MUST carry `manifest`.
2. **Copy.** A unit that names a manifest whose key 8 (`published`) is present, and that carries
   no `observed`, MUST carry `published` equal to manifest key 8, byte for byte.
3. **Omission.** `published` MUST be omitted when its normalised interval equals the degenerate
   interval `[observed, observed]` at millisecond precision. The comparison is between
   intervals, so it is exact. `2026-09-30T14:05:00Z` is a one-second interval and is not equal
   to any millisecond instant inside it.
4. **Both present** is permitted. `SMY-W411` is reported when the `observed` instant lies
   outside the `published` interval.
5. **Neither present** means the unit is undated on the *said* axis (A-12 rule E). It is not
   early or late.
6. **The copy set (D-1).** A unit extracted from a part carries exactly these provenance fields:
   `source.reference` (tid and locator), `span`, `manifest`, `published` or `observed` (by rules
   2–4), core `lang`, and for chat readers the payload key `text:speaker` (A-11). Producers MUST
   NOT copy other manifest metadata (title, creators, page, URL) onto units. It is reachable
   through `manifest`.
7. **Entities have no span.** A unit of type `x.text/entity` MUST NOT carry `span`, `manifest`,
   `published` or `grounds`. Its source is `{kind: doc, reference: "alias:<expression alias>"}`.
   Its identity is therefore a function of its name, language and expression, and it collapses
   across extraction windows. A violation is `SMY-E403`.

---

## A-3. Library identities (new spec §2.6)

Insert as spec §2.6:

> Four identities are derived for library records. Each is a BLAKE3-256 digest over a one-byte
> domain prefix followed by a preimage. The domain byte is the record type code that the identity
> names, following the convention rid already uses (`0x03`). A unit's preimage is a CBOR map,
> whose first byte is `0xa0`–`0xbf`, so no identity can equal a uid, and no two kinds of identity
> can equal each other.

| identity | names | preimage | domain byte | text form |
|---|---|---|---|---|
| **tid** | a part (record 15) | the part's normalised bytes (below) | `0x0F` | `t3:` + base32 |
| **mid** | a manifest (record 14) | the canonical CBOR of the manifest body | `0x0E` | `m3:` + base32 |
| **did** | a dating (record 17) | the canonical CBOR of the dating body | `0x11` | `d3:` + base32 |
| **rdid** | a part reading (record 18) | the canonical CBOR of the reading body | `0x12` | `r3:` + base32 |

> The base32 alphabet, bit order, and the 26–52 character acceptance rule are those of §2.1.
>
> **Normalised bytes** of a part: UTF-8, NFC, line endings LF, no byte order mark. Nothing else is
> changed: no whitespace collapsing, no case change. An implementation that accepts a part MUST
> verify that its bytes are normalised and that they hash to the tid it claims (`SMY-E446` if
> not). `SMY-E401` keeps its meaning: a re-read part whose structure hash or rdid does not match
> its manifest's part entry.
>
> None of these identities is hashed into a uid by itself. A tid appears inside units only in
> `source.reference` text and `span`'s meaning. A mid appears as `source.manifest`.

---

## A-4. Record-set digest (new spec §2.7)

Insert as spec §2.7:

> The **record-set digest** of a set of records is
>
> ```
> rsd = BLAKE3-256( "smysl/rsd/1" ‖ 0x00 ‖ h₁ ‖ h₂ ‖ … ‖ hₙ )
> ```
>
> where each `hᵢ` is the BLAKE3-256 digest of one record's canonical encoding (§3.1 framing
> included), duplicates are removed, and `h₁ < h₂ < … < hₙ` in byte order. The digest covers
> every record type, including types the implementation does not understand. It is independent
> of order and of duplicate delivery.
>
> Two stores **converge** (rule U) when their record-set digests are equal. An implementation
> MAY also compare digests of derived state (salience, labels). It MUST NOT report convergence on
> derived state alone: before SMYSL-2.1, `state_hash` ignored commitments, schema declarations,
> pack infos and unknown records (F-17).

---

## A-5. Records 14–19 (spec §3.1)

Insert into the record table:

| code | record |
|---:|---|
| 14 | manifest |
| 15 | part text |
| 16 | *reserved: hold (telemetry). MUST NOT be emitted until a later amendment defines it.* |
| 17 | dating |
| 18 | part reading |
| 19 | redaction |

**Manifest (14).** A manifest says that an expression, at one version, is these parts in this
order, read this way, recorded with this metadata.

| key | field | type | presence |
|---:|---|---|---|
| 0 | alias | text: the expression alias | required |
| 1 | parts | array of part entries, in reading order. MAY be empty (an import manifest). | required |
| 2 | lang | text: BCP-47; `mul` for mixed, `und` for unknown | required |
| 3 | reader | text: reader id and version, e.g. `osis/1`, `smysl/1` | required |
| 4 | licence | text: an SPDX id, `public-domain` or `unknown` | required |
| 5 | carry | unsigned: `0` none, `1` ref, `2` text | required |
| 6 | title | text, as recorded | optional |
| 7 | creators | array of text, as recorded | optional |
| 8 | published | text: EDTF, as recorded | optional |
| 9 | identifiers | map text → text (`isbn`, `doi`, `url`, `chat`, `channel`, `post`, `original`, `records`) | optional |
| 10 | origin | a `source` map (§2.2), **without** `captured` when the manifest records an import | optional |
| 11 | parent | 32 bytes: the mid this manifest derives from | optional |
| 12 | parent-kind | text: `translation`, `edition`, `excerpt`, `transcription` | required with 11 |
| 13 | supersedes | 32 bytes: the mid of the previous version of this expression | optional |
| 14 | versification | text | optional |
| 15 | lossy | bool, written only when true | optional |
| 16 | raw | map: the reader's manifest-level metadata, verbatim | optional |
| 17 | part-policy | text: the boundary rule and size targets | required |
| 18 | calendar | text: `julian` when key 8 is in the Julian calendar; absent means Gregorian | optional |

**Aliases** (manifest key 0, and the `alias:` form of A-2.4 rule 7) are lowercase ASCII:

```
alias   = segment *( ( "/" / ":" ) segment )        ; at most 128 bytes
segment = 1*( %x61-7A / DIGIT / "-" / "_" / "." )
```

The prefix `import:` is reserved in the **manifest alias** namespace (import manifests,
SMYSL-2.7). The prefix `export/` is reserved in the **view id** namespace (generated views,
SMYSL-2.7). An alias longer than 128 bytes is shortened by the producing tool (SMYSL-2.7's slug
rule: truncate, then a hash suffix), never rejected after the fact.

A **part entry** is `{0: tid, 1: length (unsigned), 2: structure hash (32 bytes), 3: rdid,
4: lang (text, only when it differs from key 2)}`. The structure hash is BLAKE3-256 of the
canonical CBOR of the structure table carried in the reading's segments (record 18 key 2).

**Part text (15).** `{0: tid (32 bytes), 1: text (bytes, the normalised text)}`, both required.
A reader MUST verify the tid against key 1 and reject a mismatch (`SMY-E446`). Record 15 has no surface form.

**Dating (17).** A dating is a statement about when something happened. It is a record about
units, parts and manifests, never an edit to them.

| key | field | type | presence |
|---:|---|---|---|
| 0 | target | array `[kind, id]`: `0` unit uid, `1` part tid, `2` manifest mid, `3` window `[tid, from_ms, to_ms]` (half-open, against **as-recorded** `observed`) | required |
| 1 | axis | unsigned: `0` said, `1` composed, `2` about | required |
| 2 | value | a one-entry map: `{0: EDTF text}` absolute; `{1: integer}` offset in milliseconds (said axis, instants only); `{2: [allen, target]}` relative, where `allen` is text (`before`, `after`, `meets`, `overlaps`, `during`, `contains`, `equals`) and `target` has key 0's form | required |
| 3 | basis | uid of the unit giving the evidence | optional |
| 4 | agent | text: agent id | required |
| 5 | ts | HLC `[wall_ms, counter, agent]` | required |

**Part reading (18).** What one reader derived from one part.

| key | field | type | presence |
|---:|---|---|---|
| 0 | tid | 32 bytes | required |
| 1 | reader | text: reader id and version | required |
| 2 | segments | canonical CBOR table, one row per segment: `{0: start, 1: end, 2: level (text), 3: locator (text), 4: lang?, 5: speaker?, 6: observed?, 7: ids? (map text → text), 8: tz offset in minutes?}` | required, MAY be empty |
| 3 | raw | map: the reader's raw metadata for this part, verbatim | optional |

Record 18 has no surface form.

**Redaction (19).** `{0: tid, 1: agent, 2: ts (HLC), 3: reason (uid), optional}`. Its meaning is
rule Z (A-12).

---

## A-6. Withdrawals and commitments that name a dating (spec §3.1)

Append to the withdrawal (11) and commitment (13) definitions:

> Key 0 MAY name a **did** (A-3) as well as a rid (withdrawal) or a uid (commitment). The domain
> bytes keep the kinds of identity from colliding.
>
> A withdrawal naming a did makes that dating not live (rule E). A commitment naming a did
> records how settled that dating is; a `canonical` one holds conflicting datings for review
> (rule E, locks).
>
> Readers that predate this amendment see a withdrawal of an edge they never receive (inert), and
> a commitment on a unit they never receive. The latter may appear in their commitment listings.
> That is a stated cost, not a corruption.

---

## A-7. Schema declaration keys 4 and 5 (spec §3.1)

Key 4 (`payload_shape`) has been opaque CBOR that no implementation reads. It gains a structure.
Key 5 is new.

| key | field | content |
|---:|---|---|
| 4 | payload_shape | `{0: fields}`. `fields` maps a payload key (text) to a field map: `{0: type, 1: values, 2: retired-in, 3: note, 4: required-from, 5: required-when, 6: literals}` |
| 5 | lenses | array of lens steps: `{0: from revision, 1: to revision, 2: [op, …]}` (Appendix B) |

Field map entries:
- **0 `type`**: text, one of `text`, `int`, `bool`, `uid`, `edtf`, `enum`, `list<T>`.
- **1 `values`**: array of text, for `enum`.
- **2 `retired-in`**: unsigned revision.
- **3 `note`**: text.
- **4 `required-from`**: unsigned revision.
- **5 `required-when`**: a map with one key → one value. The key is required only when that
  condition holds.
- **6 `literals`**: array of text values admitted besides the type. `text:holder` is a `uid` that
  also admits the literal `text`.

**Payload values of type `uid` are canonical uids** (32 bytes in CBOR, `b3:` text at the
surface). A label written at the surface is resolved before hashing, so identity never depends on
how an author named things.

A declaration whose existing key 4 does not match this structure is treated as opaque, exactly as
before, and reported (`SMY-W430`). It is never misread.

The schema declaration's identity, version semantics and the naming rule are A-12 rule N.

---

## A-8. Open enumerations and new codes (spec §3.1)

### A-8.1 Opening

Insert into spec §3.1:

> The enumerations **thread schema**, **role**, **source kind** and **detection kind** are open
> from TX-P0 (release 1.9.0). **Admission** is open from TX-P1 (release 1.10.0). Code 255 is
> reserved in it from TX-P0, so opening it needs no registry change.
> - A decoder MUST preserve a code it does not know and re-encode it unchanged. It reports
>   `SMY-W409` and treats the value as *unknown* wherever it is interpreted.
> - **Code 255 is reserved in each of the five and is never assigned.** An implementation MAY use
>   it internally to represent "unknown".
> - `status`, `lod`, `op`, `rung` and `commitment` stay **closed**. Rules M, T and L read them, and
>   an unknown value cannot be checked. An unknown code there is still an error.

This is a tightening of what 1.9 does today (F-12: an unknown code fails the whole store), so it
falls under spec §8.3. SMYSL-2.1 implements it for the four enumerations opened in 1.9.0, and
reserves code 255 in `admission`. **SMYSL-2.4 opens `admission` in TX-P1** (1.10.0), with the
same `Unknown` design.

### A-8.2 New codes

**Thread schemas:**

| code | schema | defined by |
|---:|---|---|
| 0–4 | analysis, narrative, brief, qa, plan | existing |
| 5 | timeline | the 1.9 `timeline` work (roles from code 49 upward, allocated there) |
| 6 | exposition | SMYSL-2.5 |
| 7 | dialogue | SMYSL-2.5 |
| 8 | dossier | SMYSL-2.5 |
| 255 | reserved | — |

**Roles.** Existing codes 0–23 keep their meaning. New roles reuse an existing role wherever its
meaning matches: `question` 16, `evidence` 17, `answer` 18, `caveat` 19, `support` 13. New codes:

| code | role | used by |
|---:|---|---|
| 24 | introduces | exposition |
| 25 | develops | exposition |
| 26 | exemplifies | exposition, dossier (`example`) |
| 27 | concedes | exposition |
| 28 | concludes | exposition |
| 29 | utterance | dialogue |
| 30 | reply | dialogue |
| 31 | uptake | dialogue |
| 32 | reversal | dialogue |
| 33 | definition | dossier |
| 34 | introduction | dossier |
| 35 | characteristic | dossier |
| 36 | counter | dossier |
| 37 | reported | dossier |
| 38 | contrast | dossier |
| 39 | open-question | dossier |
| 40 | related | dossier |
| 41 | cause | dossier |
| 42 | hypothesis | dossier |
| 43 | holder | dossier |
| 44 | stance | dossier |
| 45 | revision | dossier |
| 46 | location | dossier |
| 47 | dependent | dossier |
| 48 | gap | dossier |
| 49–63 | reserved for `timeline` | the 1.9 `timeline` work |
| 255 | reserved | — |

The dossier's `evidence` and `support` slots use roles 17 and 13, and its `question` slot uses 16.

**Detection kinds:**

| code | kind | meaning |
|---:|---|---|
| 0–3 | supersession fork, live rebuttal, label collision, commitment fork | existing |
| 4 | dating not applied | a live dating held, or of lower status than what it would override (rule E). Reported as `SMY-W412`. |
| 5 | temporal inconsistency | a negative cycle or empty interval (rule E). Reported as `SMY-W413`. |
| 255 | reserved | — |

**Time contentions are derived, not recorded** (resolves OQ-35).
- Their identity is the spec §6.2 contention identity, with detection kind 4 or 5.
- A resolution (record 12) names that identity. No contention record (6) is written for them,
  the same choice merge makes for detection today.
- An implementation MUST NOT write record 6 with kind 4 or 5. This keeps stores readable by
  readers that predate A-8.1.

---

## A-9. Granularity key 5 and estimator ids (spec §3.1, spec §4)

Resolves OQ-29 as a **wire field**.

> The view's granularity map gains key 5, `estimator`: text, an estimator id. It is written
> **only when it is not `smysl/utf8-div4`**, so every existing view encodes to the bytes it had.
> The `l0_max` bound (`SMY-E022`) is evaluated with the named estimator's **count**.

**Estimator registry:**

| id | count of a text `t` | notes |
|---|---|---|
| `smysl/utf8-div4` | `ceil(utf8_len(t) / 4)` | the default; what check has always counted. Pack adds `+2` framing per item for every estimator, and the framing is not part of the id. |
| `smysl/content/1` | `ceil(Σ_c n_c(t) · w_c / 1000)`, with integer milli-weights `w_c` per character class | the classes and weights are fixed by `fixtures/estimator/content-1.json`, calibrated over 7,932 verses present in all seven editions of a parallel corpus and pinned by a test. An id never changes: new weights are a new id. |

A reader that does not know an estimator id preserves it and treats `l0_max` as **unevaluable**,
reporting `SMY-W025`. It MUST NOT evaluate the bound with the default count.

**Two departures from this amendment as accepted at G−1, both made when F-2 shipped in 1.9.0.**
Draft 1 called the second estimator `smysl/script-aware/1` and said an unknown id is evaluated
with the default and reported as `SMY-W409`. Neither survived building it:

- The id is `smysl/content/1`. "Script-aware" named the mechanism; what the weights buy is that
  one proposition costs the same in either language, so the id names the property a bound is
  stated in. The weights are a calibration over a corpus, and `/1` is the calibration, which is
  what an id has to be pinned to.
- An unknown id leaves the bound **unevaluable**, not evaluated with the default. Evaluating it
  under a different count returns a verdict on a question nobody asked, and a verdict is harder
  to ignore than a gap — a reader cannot tell a bound that was met from one that was measured
  wrong. `SMY-W409` is the wrong code for the same reason: it says a value was preserved and
  treated as unknown, and this is a bound that was not checked, which is `SMY-W025`.

The amendment text above is the corrected version; `SMYSL_FORMAT_SPEC.md` §3.1 carries it.

---

## A-10. Surface syntax (spec §4)

1. **Reserved words**: `@manifest`, `@date` and `@redact`, for records 14, 17 and 19. Records 15
   and 18 have no surface form. Text is not written in surface syntax.
2. **`lang` is reserved** as a unit attribute: a bare `lang:` on a unit is core key 9. A payload
   key named `lang` must be quoted (`"lang":`). SMYSL-2.1 ships a warning for documents that rely
   on the old reading (`SMY-W432`) before A-1 activates. This changes the meaning of a document
   written before the change, and SMYSL-2.1's lint is the mitigation.
3. **`source { }`** accepts `kind`, `ref`, `captured`, `observed`, `published`, `span` and
   `manifest`. Any other key is `SMY-E001` (tightening, F-13). An `observed` outside `u64`, or a
   malformed `captured`, is `SMY-E001`, not a silent drop (F-14).
4. **`@schema`** accepts `version`, `types`, `relations`, `fields` and `lenses`. `fields` and
   `lenses` are spelled as HJSON maps and arrays with the keys of A-7 and Appendix B written as
   names (`type`, `required-from`, `rename-key`). A declaration that cannot be spelled stays
   CBOR-only and is counted, as today (F-18).
5. **Uid-typed payload keys** may be written as labels at the surface. They are resolved to
   canonical uids before hashing (A-7). An unresolvable label is `SMY-E060`.

---

## A-11. Standard extension schemas (spec §5)

Two extension schemas are registered so that implementations agree on their names. Their
declarations are in Appendix C. Being registered does not make them kernel: every rule of spec
§5 (rule X) applies.

- **`x.text/v1`**: types for text research (event, entity, theme, citation, meta), relation
  kinds (same-as, entails, contradicts, translates, quotes, alludes, mentions, replies, precedes,
  summarises), and payload keys (`text:holder`, `text:mode`, `text:speaker`, `text:locator`,
  `text:when`, `text:kind`, `meta:field`, `meta:value`).
- **`x.query/v1`**: the saved query.
  - **Results are linked by `x.query/answers`, not by kernel `answers`** (resolves OQ-49).
    Kernel edges feed derived salience. Saving a query must not change the ranking of later
    queries.

Holder, mode and speaker stay in `x.text/v1` and do not become kernel fields (D-6, OQ-6).

---

## A-12. Rules (spec §6)

Add three rows to the rules table:

| rule | obligation |
|---|---|
| **N** | Naming: a schema revision never gives an existing name a new meaning, except through a declared widening with a paired key, and a retired name is never reintroduced (A-12.1). |
| **E** | Effective time is derived from as-recorded time, datings and free constraints by the stratified procedure of A-12.2. As-recorded fields are never edited. |
| **Z** | Redaction: a store holding a redaction for a tid holds no part text or reading for it, on any merge (A-12.3). |

### A-12.1 Rule N — naming

A schema id is evolved by **revision**: the `version` key of successive declarations of the same
id. Declarations accumulate, and the highest revision is current.

1. A revision MAY add names (types, relation kinds, payload keys, enum values), retire names, and
   make a key required (`required-from`, optionally `required-when`).
2. A revision MUST NOT give an existing name a different meaning, with one exception: a
   **declared widening**.
   - A widening is a `split-value` lens op whose target value already exists.
   - It MUST be paired with a key that is required from the widening revision, under a condition
     covering the widened value, and that has a `default` under the same condition for older
     units.
3. A retired name MUST NOT be reintroduced by any later revision.
4. Two declarations with the same id and version and different content are a **fork**
   (`SMY-W429`). Neither is current until a resolution names the fork.

A schema declaration that violates 1–3 against **any** earlier revision in the store is
`SMY-E426`.

**Consequence: units carry no revision stamp.** A unit's revision is inferred:
- from the retired names it uses
- from required keys it lacks while satisfying their condition
- for extracted units, from the schema revisions recorded in their attestation's recipe

A unit that satisfies none of these reads identically under every revision, by rule N itself.

**Lenses** (A-7 key 5, Appendix B) map one revision's names to the next.
- **Applying them is not a conformance obligation** (A-13). A consumer without lenses reads names
  as written, which is narrower but never wrong.
- An implementation that applies lenses MUST apply the semantics of Appendix B exactly.
- **An unknown op tag makes its lens step unusable** (D-5). Units that need that step are
  unmappable (`SMY-W420`), never guessed.

### A-12.2 Rule E — effective time

**Axes.** Each unit has up to four times:
- **said**: when it was said or published
- **composed**: when the work was written; only through datings on the work's catalog entity
- **about**: the time the claim refers to; through `text:when` and datings
- **known**: the earliest HLC `ts` of its attestations; never corrected

Rule E derives the first three. Each is an interval `[earliest, latest]` of instants, or
*undated*, or *contested*.

**Time status of an as-recorded value:**

| as-recorded value | time status |
|---|---|
| `observed` on a unit attested with `op: Imported` at rung `computed` | measured |
| `observed` supplied by a reader from a platform export | cited |
| `published` on a unit or manifest | cited |
| a value supplied with no source | speculative |

**Status of a dating:** the status of its basis unit. With no basis, `speculative`.

**Free constraints** are derived from the store, with fixed statuses (D-3):

| constraint | rule | status |
|---|---|---|
| quotation | if B `x.text/quotes` A, then said(A) ≤ said(B) | the status of the quoting unit |
| derivation | a manifest is no earlier than its parent (key 11) | cited |
| supersession | a manifest is no earlier than the one it supersedes (key 13) | cited |
| reply | a reply is no earlier than the message it answers, using the first version's as-recorded `observed` | cited |
| first seen | nothing is published after the earliest attestation `ts` of a unit grounded on its part | derived |

**Liveness.** A dating is **live** unless:
- a withdrawal names its did, or
- its basis is unfounded (rule R), or
- it is **held** by a lock. A dating is held when a `canonical` commitment names it, or names its
  target's existing dating, and the new dating would change the effective value, until a
  resolution names the resulting detection-kind-4 contention.

**Procedure (normative).** For one axis, over the constraint graph of a scope:

1. **Seed.** Each subject (a unit, or the units under a part, manifest or window target) starts
   from its as-recorded interval:
   - `published` if present
   - else `[observed, observed]`
   - else *undated*, with no bound
   
   Each bound carries the time status of the value that set it.
2. **Strata.** For each status level `s`, from `measured` down to `speculative`: build a simple
   temporal network from the live absolute datings, offsets and relative datings, and the free
   constraints, whose status is ≥ `s`.
3. **Tighten.** Solve each stratum's network by shortest paths.
   - A bound moves only if `s` is at least the time status of the value currently setting it.
   - A move that `s` would make but may not is not applied. It is recorded as a position of a
     detection-kind-4 contention (`SMY-W412`).
   - A path's status is the minimum of its edges' statuses. A stratum contains only edges of
     status ≥ `s`, so every move in stratum `s` is justified at status `s` or better. This
     answers OQ-42: a chain of mixed-status constraints tightens at the status of its weakest
     link.
4. **Offsets** are applied to instants before propagation. An offset targets the as-recorded
   instant.
5. **Inconsistency.** A negative cycle, or an empty interval, in any stratum marks every subject
   on it **contested**. A detection-kind-5 contention (`SMY-W413`) names the datings and
   constraints involved. Nothing is chosen.
6. **Order independence.** Every stratum is a plain simple temporal network, whose tightest
   solution is unique. Effective time is therefore a function of the store's record set (rule U),
   not of arrival order.

**Calendars.** A Julian as-recorded value (manifest key 18) is converted to a Gregorian interval
in step 1. It is never written back.

**Effective time is never stored.** An implementation MAY index it. It MUST recompute it when the
record set changes.

### A-12.3 Rule Z — redaction

> A redaction (record 19) for a tid means: the store holds no record 15 and no record 18 for that
> tid. An implementation MUST drop them on receipt, now and on every later merge.
>
> The store's state is the union of records, **filtered** by the union of redactions. Redactions
> only accumulate, so union-then-filter is commutative, associative and idempotent: rule U holds.
> A merge with a peer that never received the redaction cannot bring the text back into a store
> that holds it.
>
> Units, manifests and spans remain. Spans that name a redacted part no longer resolve, and
> consumers report this (`SMY-W414`).
>
> A peer that predates this amendment preserves record 15 as an unknown record and can pass the
> text on to third parties. Rule Z binds the stores that implement it, and only those.

### A-12.4 Proposition classes (normative definition of `strict`)

Resolves OQ-7. Class policies `component` and `attested:n` are tool-level. **`strict`** is
normative, so two implementations count the same classes:

> Let `E` be the set of live `x.text/same-as` relations: not withdrawn, and with both endpoints
> present. Edges are undirected for this purpose. Let `V` be the units with at least one edge in
> `E`, in ascending uid order. Repeat until every unit of `V` is assigned:
> 1. The smallest unassigned unit **opens** a class.
> 2. Every later unassigned unit, in ascending uid order, **joins** that class if it has an edge
>    in `E` to every current member.
>
> A class's id is its smallest member uid, which is the unit that opened it. Units with no edge
> in `E` form no class.

This greedy clique partition (seed, then grow in uid order) is deterministic, and independent of
the order records arrived in. It is not a minimum clique cover, and does not claim to be.
SMYSL-2.4 §3.6 implements exactly this procedure.

---

## A-13. Conformance (spec §7)

Amend the conformance table (D-8):

| class | additions |
|---|---|
| **C-Read** | decode and re-encode byte-identically: records 14, 15, 17, 18 and 19; unit key 9; source keys 4–6; schema declaration key 5; granularity key 5; enumeration codes unknown to the reader (A-8.1). Compute tid, mid, did and rdid, and the record-set digest (A-4). Reject a part whose bytes do not hash to its tid (`SMY-E446`). |
| **C-Consume** | read `published` and `observed` as the *said* axis where it interprets time. Treat unknown enumeration values as unknown, never as an error, except in the closed set. |
| **C-Produce** | follow the copy and omission rules (A-2.4). Write `lang` per A-1. Write only well-formed EDTF. Obey rule N for any schema declaration it writes. Write uid-typed payload values as canonical uids. |
| **C-Merge** | honour withdrawals and commitments that name a did (A-6). Apply rule Z on every merge. Check convergence by record-set digest (A-4). |
| **C-Library** (new) | structural + epistemic + **library**: verify part texts and readings against their ids and the manifest's structure hashes; apply rule Z; derive effective time per rule E and report contested subjects; derive `strict` classes per A-12.4. Like C-Merge, it branches from C-Consume and does not subsume C-Produce. |

**Lens application is not a conformance obligation** (A-12.1).

**First targets.** The Python, JavaScript and Go implementations target the C-Read additions
first: ids, digests and byte-identical round trips of records 14–19. SMYSL-2.7 sizes the digest at
about 15 lines in each.

---

## A-14. Registry and activation (spec §8)

Append to spec §8.1's list of permitted changes, as instances that have been made:
- record types 14, 15, 17, 18 and 19 (16 reserved)
- unit core key 9
- source keys 4–6
- schema declaration key 5
- granularity key 5
- new enumeration codes under A-8

Append to spec §8.3, as tightenings:
- an unknown enumeration code is preserved rather than failing the store
- the surface parser rejects unknown `source` keys and out-of-range time values

The registry tables in Appendix A of this document are copied into the spec's appendix when the
first amendment activates, and are kept there.

**The `spec-tables` gate** (`make spec-tables`, `scripts/verify-spec-tables.py`) parses the spec's
tables and checks them against the code's constants. Each fold of an amendment into the spec
extends that script to the new tables:
- unit core key 9
- source keys 4–6
- record codes 14–19
- the bodies of records 14, 15, 17, 18 and 19
- identity domain bytes
- thread schema, role and detection-kind codes
- estimator ids

A fold is not complete until the gate covers it.

---

## Appendix A — Registry

**Record types:** 1–13 existing; 14 manifest; 15 part text; 16 reserved (hold); 17 dating;
18 part reading; 19 redaction.

**Unit core keys:** 0–8 existing; 9 `lang`.

**`source` keys:** 0–3 existing; 4 `published`; 5 `span`; 6 `manifest`.

**Schema declaration keys:** 0–3 existing; 4 `payload_shape` (structured by A-7); 5 `lenses`.

**Granularity keys:** 0–4 existing; 5 `estimator`.

**Identities:**

| identity | domain byte | text prefix |
|---|---|---|
| uid | (a CBOR map, `0xa0`–`0xbf`) | `b3:` |
| rid | `0x03` | `b3:` |
| mid | `0x0E` | `m3:` |
| tid | `0x0F` | `t3:` |
| did | `0x11` | `d3:` |
| rdid | `0x12` | `r3:` |

**Digest prefix:** `smysl/rsd/1` followed by `0x00`.

**Open enumerations:** thread schema, role, source kind and detection kind (from 1.9.0); admission
(from 1.10.0). Code 255 is reserved in all five from 1.9.0. **Closed:** status, lod, op, rung, commitment.

**New enumeration codes:** thread schemas 5–8; roles 24–48 (49–63 reserved for `timeline`);
detection kinds 4–5 (A-8.2).

**Estimator ids:** `smysl/utf8-div4`, `smysl/content/1` (A-9; the second was
`smysl/script-aware/1` in draft 1).

**Rules:** N, E, Z (A-12), alongside M, T, L, R, U, I, S, V1/V2, X, D, P.

**Diagnostics** referenced here: E001, E022, E060, W014, E401, E403, E404, W405, W409, E410,
W411, W412, W413, W414, W420, E426, W429, W430, W432, E446. The full registry of the set, with
meanings for E401–W431 and each RFC's allocation range, is SMYSL-2.0 §6.1.

## Appendix B — Lens operations

A lens step is `{0: from, 1: to, 2: [op…]}`. An op is a map whose key 0 is its **tag** (text).
At the surface, keys are written as names.

| tag | keys | reading (forward) | backward |
|---|---|---|---|
| `rename-type` | 1 from, 2 to | a unit of type `from` is read as `to` | yes |
| `rename-relation` | 1 from, 2 to | an edge of kind `from` is followed as `to` | yes |
| `rename-key` | 1 from, 2 to | payload key `from` is read as `to` | yes |
| `map-value` | 1 key, 2 map text → text | enum values are renamed | yes, if the map is one-to-one |
| `split-value` | 1 key, 2 from, 3 to, 4 add (map text → value) | value `from` is read as `to`, plus the `add` keys. When `to` already exists, this is a widening (rule N). | yes: `to` together with `add` maps back to `from` |
| `require` | 1 key, 2 when (map, optional) | marks the key required from `to` | yes: the requirement is not applied |
| `default` | 1 key, 2 value, 3 when (map, optional) | a unit that lacks the key, and satisfies `when`, reads it as `value`. Never written. Only for a key `require`d from the same revision under the same condition. | yes, when the unit's value equals `value`: the key is dropped |
| `retire` | 1 key | the key is read as absent | **no** |

**Composition.** Steps compose in revision order. The **lens-chain hash** is

```
BLAKE3-256( "smysl/lens-chain/1" ‖ 0x00 ‖ canonical_cbor([schema id, from, to, [step…]]) )
```

where each `step` is a lens-step element **as received** in record 8 key 5 (not a re-encoding),
in application order. Hashing the received bytes makes the hash identical across implementations
that agree on the record. Composition concatenates the step lists. A tool records the hash
wherever a lensed result is recorded (`query:lens-chain`, measure recipes, export indexes).

**Lenses never write.** A lensed unit is a reading. Its bytes, uid and attestations are untouched.

## Appendix C — Declarations of `x.text/v1` and `x.query/v1`

```
@schema x.text/v1 {
  version: 1,
  types: ["x.text/event", "x.text/entity", "x.text/theme", "x.text/citation", "x.text/meta"],
  relations: ["x.text/same-as", "x.text/entails", "x.text/contradicts", "x.text/translates",
              "x.text/quotes", "x.text/alludes", "x.text/mentions", "x.text/replies",
              "x.text/precedes", "x.text/summarises"],
  fields: {
    "text:holder":  { type: uid, literals: ["text"] },
    "text:mode":    { type: enum, values: ["asserted", "reported", "hypothetical", "negated",
                                           "interrogative", "imperative", "quoted"] },
    "text:speaker": { type: uid },
    "text:locator": { type: text },
    "text:when":    { type: edtf },
    "text:kind":    { type: enum, values: ["person", "place", "group", "work", "object",
                                           "concept"] },
    "meta:field":   { type: text },
    "meta:value":   { type: text },
  },
}

@schema x.query/v1 {
  version: 1,
  types: ["x.query/saved"],
  relations: ["x.query/answers"],
  fields: {
    "query:text":        { type: text },
    "query:recipe":      { type: text },
    "query:models":      { type: list<text> },
    "query:result-hash": { type: text },
    "query:sq":          { type: text },
    "query:schemas":     { type: list<text> },
    "query:lens-chain":  { type: text },
  },
}
```

The `fields:` spelling requires A-10 item 4. Until a parser implements it, these declarations are
CBOR-only and are counted as omitted by surface writers (F-18). Field types follow A-7.

## Appendix D — Conformance fixtures to add

Each goes under `fixtures/` with an `.expected` file, generated by the existing tooling and run by
all four implementations where the class applies:

| fixture | class | checks |
|---|---|---|
| `wire/ids/cases.json` | C-Read | tid, mid, did, rdid preimages and digests, as `wire/uid` and `wire/relation-id` already do |
| `wire/rsd/cases.json` | C-Read | record-set digests: order independence, duplicates, unknown records |
| `conformance/codec/records-14-19.cbor` | C-Read | byte-identical round trip of every new record |
| `conformance/codec/enum-unknown.cbor` | C-Read | unknown codes in each open enumeration are preserved, and code 255 round-trips |
| `conformance/codec/granularity-key5.cbor` | C-Read | estimator key preserved |
| `library/copy-omission/` | C-Produce | published copy, omission on interval equality, both-present `W411` |
| `library/rule-z/` | C-Merge, C-Library | merge with a stale peer never restores redacted text |
| `library/rule-e/` | C-Library | strata, holds, offsets, window targets, contested results; a brute-force reference on small networks |
| `library/classes-strict/` | C-Library | `strict` class ids and counts |
| `schema/rule-n/` | C-Produce | widenings with and without a paired key, reintroduced names, forks |

## Appendix E — Decisions recorded by this amendment

| decision | resolves | where |
|---|---|---|
| copy set D-1 | OQ-17 | A-2.4 rule 6 |
| surface spelling D-2 | OQ-1 | A-10 items 2–3 |
| statuses of free constraints D-3 | OQ-10 | A-12.2 |
| scholarly datings take their basis's status D-4 | OQ-12 | A-12.2 (status of a dating) |
| closed lens op set, open tag space D-5 | OQ-23 | A-12.1, Appendix B |
| holder, mode and speaker stay in `x.text/v1` D-6 | OQ-6 | A-11 |
| `strict` classes normative D-7 | OQ-7 | A-12.4 |
| conformance mapping D-8 | — | A-13 |
| rules N, E, Z D-9 | — | A-12 |
| estimator as a wire field | OQ-29 | A-9 |
| code 255 reserved in five enumerations; four opened in 1.9.0, `admission` in 1.10.0 | OQ-30 (`status` and `lod` stay closed) | A-8.1 |
| time contentions derived, kinds 4 and 5 | OQ-35 | A-8.2 |
| a chain tightens at its weakest link's status | OQ-42 | A-12.2 step 3 |
| saved-query results by `x.query/answers` | OQ-49 | A-11 |
