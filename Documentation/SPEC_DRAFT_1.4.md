# smysl 1.4 — lifecycle of edges and disagreements (specification draft)

**Status:** draft. Nothing here is normative until it is folded into
[`SMYSL_FORMAT_SPEC.md`](SMYSL_FORMAT_SPEC.md) at the 1.4 cut. Each section names the spec
section it amends.
**Format version:** stays `smysl/1.0`. Every change below is one §8.1 already permits — two new
record types, a derived identity, and a meaning for fields that exist — and the compatibility
section shows what a 1.3 reader does with each.
**For:** crate `1.4.0`.
**Implemented:** all six sections in the Rust library (`smysl-core`, `smysl-graph`, `smysl-pack`,
`smysl-check`), with tests in `crates/smysl-graph/tests/lifecycle.rs`,
`crates/smysl-pack/tests/lifecycle.rs` and `crates/smysl-core/tests/lifecycle_fixtures.rs`; and in
the CLI as `review`, `withdraw` and `resolve` (`tests/cmd_review.rs`). Not yet: surface syntax and
the other three implementations.

## Why

rust_smysl verifies extracted claims against the commit they came from. A contradicted claim goes
to review and is never retracted automatically. Checked against that design, 1.3 cannot close
what verification opens:

1. **An edge cannot be withdrawn.** Retraction targets units; a `rebuts` edge a reviewer
   rejects stays in the graph for good.
2. **Nobody can say who asserted an edge.** `Relation.attestations` exists in memory, but no
   attestation record can reach it, so a model-matched `rebuts` and a human's are the same bytes.
3. **"Live rebuttal" is undefined.** §6 rule R obliges a selection to carry a claim's *live*
   rebuttals and never says what live means. Packing treats every `rebuts` edge as live — a
   retracted rebuttal still pins its claim — and merge's detection means something else again.
4. **A disagreement cannot be closed.** `ContentionStatus::Resolved` is on the wire and nothing
   produces it.

All four come back to one missing thing: **a relation has no identity the format states.** The
implementation has had one since 0.2 (`Relation::uid`), used by nothing on the wire.

## 1. Relation identity — amends §2 (new §2.5)

```
rid = BLAKE3-256( 0x03 ‖ kind ‖ 0x00 ‖ from ‖ to )
```

- `kind` is the relation kind's **name** in UTF-8 — `rebuts`, `x.verify/supports` — whether the
  record encodes it as a kernel integer or as text. Two encodings of one kind are one edge.
- `from` and `to` are the raw 32-byte uids.
- `0x03` is the relation record's type code. It separates the domains: a unit's preimage is a
  canonical CBOR map, whose first byte is `0xa0`–`0xbf`, so no unit uid can equal a rid.
- `0x00` terminates the name. No kind name contains NUL.

`weight`, `note` and unknown keys are **not identity**. Two records with the same kind and
endpoints are one edge; a store holds it once and unions what is attached to it.

A rid is written exactly as a uid: 32 raw bytes in CBOR, `b3:` plus base32 in text (§2.1).
`fixtures/wire/relation-id/cases.json` holds vectors.

## 2. Withdrawing an edge — amends §3.1 (record type 11)

| key | field | type | presence |
|---:|---|---|---|
| 0 | relation | rid, 32 bytes | required |
| 1 | agent | text, an agent id | required |
| 2 | ts | HLC, as attestation key 4 | required |
| 3 | reason | uid of a unit saying why | optional |
| ≥4 | unknown keys | any | preserved verbatim |

**A relation is withdrawn in a store iff the store holds at least one withdrawal naming its
rid.** A withdrawal is permanent: records only accumulate (rule U), so there is no un-withdrawing,
exactly as there is no un-retracting a unit. A withdrawal whose relation is not (yet) in the
store is kept; it takes effect when the relation arrives, whatever order they were delivered in.

A withdrawn relation **MUST be preserved** — its record round-trips — and **MUST NOT be followed**
by anything that interprets the graph: closure, lineage, contention detection, packing, salience.

**`retracts` and `supersedes` edges cannot be withdrawn in 1.4.** A withdrawal naming one is
preserved and has no effect (`SMY-W056`). Un-retracting a unit changes effective status and
supersession changes what dependents rebind to; both need their own rules, and a withdrawal that
silently restored a retracted unit would be worse than none.

Why a record type and not a `retracts` edge pointing at a rid: a 1.3 checker reports an edge to
a uid it cannot find as `SMY-E060`, an error, so every 1.4 store using it would fail `check` on
1.3. An unknown record type is `SMY-W014`, a warning, which is what §8.1 promises.

## 3. Who asserted an edge — amends §2.4 and the attestation record

**An attestation's `uid` (key 0) MAY name a rid.** A store attaches it to that relation as it
attaches a unit's; attestations on one edge union across stores. Rule T does not apply — a
relation has no status to cap — but `agent`, `op`, `rung`, `hop` and `recipe` mean what they
mean for a unit, so a model-proposed `rebuts` edge (`op: imported`, `rung: model`) is
distinguishable from a reviewer's (`op: authored`, a `human:` agent).

No wire change: the field already holds 32 bytes. An attestation naming neither a present unit
nor a present relation is kept and attached when its subject arrives.

## 4. Live rebuttal — amends §6 rule R

In a store `S` under a retraction policy `P` (an implementation without a policy notion uses
**strict**), a relation `r = (rebuts, a, b)` is **live** iff:

1. `r` is not withdrawn (§2 of this draft);
2. `a` is present in `S`; and
3. `a`'s effective status under `P` is not `unfounded` — `a` is neither retracted nor orphaned.

Rule R then reads: **a selection containing `b`, at any level, MUST contain every `a` such that
`(rebuts, a, b)` is live.** A retracted rebuttal no longer pins its claim.

Supersession of `a` does **not** end liveness. `supersedes` says a better version exists, not
that the old one is false; the objection stands until someone retracts it or withdraws the edge.

Contention detection kind 1 (`live-rebuttal`) uses this definition, and additionally requires
`b`'s effective status not to be `unfounded`: a disagreement with a retracted claim is over.

## 5. Resolving a disagreement — amends §3.1 (record type 12)

| key | field | type | presence |
|---:|---|---|---|
| 0 | contention | text, a contention id | exactly one of 0 and 1 |
| 1 | relation | rid, 32 bytes | exactly one of 0 and 1 |
| 2 | agent | text, an agent id | required |
| 3 | ts | HLC | required |
| 4 | note | uid of a unit recording the decision | optional |
| ≥5 | unknown keys | any | preserved verbatim |

A resolution names what was reviewed — a contention, or a `rebuts` edge nobody had threaded into
one — and who reviewed it. A record with both keys 0 and 1, or neither, MUST be rejected.

**A resolution records that a review happened. It never decides the outcome.** Merge must not
adjudicate (§5.4 of the architecture), and neither does a resolution: a reviewer who finds the
claim wrong retracts it, one who finds the rebuttal wrong withdraws the edge or retracts the
rebutting unit, and those are separate records with their own effects. The resolution is what
takes the item off the review queue.

Its effects:

- **A contention** named by at least one resolution has effective status `resolved`, whatever
  status its own record carries, and no longer pins its positions into a selection (packing
  constraint C4).
- **A `rebuts` edge** named by a resolution stays live — rule R still binds, the objection still
  travels with the claim — but is no longer an open review item.

Resolutions accumulate like withdrawals. Reopening a resolved item is not in 1.4.

### Contention identity becomes normative

A resolution names a contention by id, so two implementations must derive the same id:

```
digest = BLAKE3-256( kind ‖ over ‖ positions )
id     = "k/c" ‖ base32( first 130 bits of digest )
```

`kind` is the detection kind as one byte (0 supersession fork, 1 live rebuttal, 2 label
collision); `over` is 32 bytes; `positions` is the sorted, deduplicated uids, 32 bytes each. The
base32 is §2.1's, 26 characters. The clock a detection is stamped with is not identity.

### Stale

A recorded contention of kind 1 whose rebuttal is no longer live, or whose claim is `unfounded`,
has effective status `stale` and pins nothing. (Detected contentions are derived from the store,
so a stale one is simply not detected.)

## 6. Unknown keys in every record body — amends §8.1

§8.1 permits "a new unit-core key ≥ 9, or a new header key". It says nothing of the other record
bodies, and the reference implementation already preserves unknown keys in all of them. Stated:

> **A new key in any record body**, above that record's highest key in §3.1's tables, is
> permitted. Readers MUST preserve it verbatim.

This is what makes a later addition to a relation, withdrawal or resolution possible without a
new record type.

## Compatibility with 1.3 readers

| a 1.4 store holds | a 1.3 reader |
|---|---|
| a withdrawal or resolution | preserves it, `SMY-W014`, ignores it: the edge still reads as live, the contention as recorded |
| an attestation naming a rid | preserves it, attaches it to nothing |
| a `retracts`-then-`rebuts` history | packs the retracted rebuttal with its claim, as before |

No 1.3 reader rejects a 1.4 store, and none of its `check` diagnostics becomes an error. What a
1.3 reader gets wrong is only what 1.4 adds: it does not honour withdrawals or resolutions.

## Conformance

- **C-Read** gains nothing to do: records 11 and 12 are preserved like any unknown record until
  an implementation decodes them, and then they must round-trip.
- **C-Merge** — "honour retraction and supersession" — gains: honour withdrawal (§2), attach
  edge attestations (§3), and apply the liveness definition (§4) wherever it detects contentions.
- Packing is not a conformance class (the constraints are in the manual), but rule R is a format
  rule, so an implementation that packs MUST use §4.

## Fixtures

- `fixtures/wire/relation-id/cases.json` — kind, from, to, expected rid, including an extension
  kind and a kernel kind encoded as an integer.
- `fixtures/wire/contention-id/cases.json` — kind, over, positions in unsorted order with a
  duplicate, expected id.
- A wire fixture holding one of each new record, for the four implementations' round-trip suites.

## Rejected

- **`retracts` pointing at a rid.** A 1.3 checker reports `SMY-E060` (above).
- **An `attestations` key on the relation record.** Attestations are records everywhere else;
  a second channel for edges would give merge two places to union and a way for them to disagree.
- **Every `rebuts` edge a contention.** Detection kind 1 requires the units to share a thread by
  design — a disagreement in waiting, not in progress — and dropping that would make packing a
  rebuttal pull in the claim it rebuts (C4). The review queue lists unthreaded rebuttals without
  making them contentions, and §5 lets a resolution name them directly.
- **A resolution with an outcome** (`upheld`, `rejected`). It would be a second way to say what
  retraction and withdrawal already say, and two resolutions with different outcomes would be a
  disagreement about the disagreement, which merge could not settle without adjudicating.

## Open

- **Surface syntax** for records 11 and 12. They travel as CBOR only in 1.4, as attestations and
  contentions do; a reviewed `.smy` file loses them.
- **Withdrawing `retracts` and `supersedes`** (§2).
- **Reopening** a resolved item.
- **Authority**: who may withdraw an edge. Retraction's `RetractionAuthority` is enforced by the
  tool, not the format; the same is proposed here, and `origin` authority would read the edge's
  attestations (§3).
