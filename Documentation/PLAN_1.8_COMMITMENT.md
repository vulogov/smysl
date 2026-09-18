# smysl 1.8 — commitment and host sources (implementation plan)

**Status:** plan, written 2026-09-18, in response to
`../blackInkhaven/Documentation/PROPOSALS/SMYSL-1_RFC.md` (inkhaven, `3.11.0-dev`).
**For:** crate `1.8.0`, format `smysl/1.0`.
**Verdict:** both asks are worth doing and neither is as small as the RFC estimates. Two of its
compatibility claims do not survive contact with the tree, and both failures are the same shape:
a thing that *looks* like a safe place to put data is not, because it has no wire representation
or no forward-compatible decode. The designs below fix that; the asks themselves stand.

---

## 0. What the RFC got right

Independently re-verified, all confirmed:

- `Unit` carries `attestations`, `salience` and `labels` outside `core`, and `canonical_uid`
  hashes `UnitCore` only (`crates/smysl-core/src/hash.rs`), pinned by
  `attestations_labels_and_salience_do_not_change_identity`. A commitment annotation must not go
  on `UnitCore`, and the RFC is right that it must not.
- `Status` and `SourceKind` are both `#[repr(u8)] #[non_exhaustive]`, and `Status`'s integer order
  *is* rule M (`check/src/passes/epistemics.rs`). A parallel axis cannot reuse it.
- Proposal B is already served. `SalienceRequest::role_weights` and `seed` blend a caller's
  relevance in, `Unit.salience` overrides it outright, and `pack` consumes the resulting
  `SalienceReport`. No smysl change is needed, and the RFC withdrawing its own ask is the right
  call.
- The narrative motion is already in the kernel vocabulary: `Decision`, `Supersedes`, `Retracts`,
  `Rebuts`, plus `x.<domain>/…` extension types for anything else.

The framing is also right, and worth saying plainly: smysl is a poor fit for inkhaven's
current-state epistemics and a good one for decision history. That is the axis the format was
built for.

---

## 1. Finding one — `Unit.salience` is not persisted, so it is the wrong model to copy

The RFC proposes putting `commitment` "outside the hashed `UnitCore` — the same place
`attestations` / `salience` / `labels` already live", and calls the result "stored data".

**`salience` is not stored data.** There is no `salience` key in `cbor/keys.rs`, nothing writes one
in `cbor/envelope.rs`, and the behaviour follows:

```
$ cat sal.smy
@claim c/one { status: speculative, salience: 0.9 }
~ The eu-west connection pool saturated.

$ smysl merge sal.smy -o sal.cbor
$ smysl --format surface fmt sal.cbor
@claim c/one { status: speculative }
~ The eu-west connection pool saturated.
```

The authored salience is gone. It survives in surface text and dies at the first store write.
`attestations` and `labels` look similar and are *not* the same: each has its own record type
(2 and 10), so they persist as records and the `Unit` view is assembled from them. `salience` has
none, which is defensible for an authoring hint recomputed per run and fatal for a canon ledger —
whose whole purpose is to persist, merge and diff across drafts.

A commitment level put beside `salience` would therefore be lost the moment inkhaven wrote the
ledger to disk, and lost silently.

### Consequence for the design

Commitment needs **its own record type**, exactly as withdrawal and resolution did in 1.4. That is
a larger change than the RFC's estimate and a well-trodden one for this project: 1.4 added two
record types, their codecs, surface syntax, merge semantics, a conformance fixture and matching
support in the Python, JavaScript and Go implementations, and it went out as a minor version
because §8.1 permits a new record type.

A record is also *better for the stated use*. A bare field answers "how settled is this"; a record
answers "who settled it, when, and what did it say before" — which is the question a development
history exists to answer, and the one the RFC's §6 asks first ("what was decided, revised,
retracted"). The field design cannot express a retcon's history; the record design gets it free.

---

## 2. Finding two — a new `SourceKind` variant is rejected, not degraded, by every existing reader

`SourceKind::from_u8` returns `None` for an unrecognised value, and the decoder then fails the
whole record:

```rust
// crates/smysl-core/src/cbor/envelope.rs
kind = SourceKind::from_u8(u8::try_from(d.uint()?)...);   // None for an unknown kind
...
kind: kind.ok_or_else(|| bad(at))?,                        // → malformed envelope
```

So a store written by 1.8 using `SourceKind::Node` is **unreadable to 1.7 and to every other
implementation**, one record at a time. That is corruption rather than degradation, and it is the
exact asymmetry this project already closed once for unit types: `SchemaId::Unknown` exists
because "adding one kernel type in a later 0.x made every store carrying it unreadable to an
earlier build, while an unknown *record* type and an unknown *extension* type both degraded
correctly."

`SourceKind` never got the same treatment because nobody had added a variant since.

### Consequence for the design

Proposal C is two changes, not one, and they must land in this order:

1. **`SourceKind::Unknown(u8)`** (or a reserved decode arm preserving the byte), so that an
   unrecognised kind round-trips verbatim instead of failing the record. This is the
   forward-compatibility fix, and it is worth doing whether or not C lands — it is a latent trap
   for any future variant.
2. **`SourceKind::Node`** itself.

Note what this means for adoption: readers older than 1.8 still cannot read `Node` sources,
because the fix ships *with* the variant. The fix protects 1.9 and later, not 1.7. That is worth
stating to inkhaven plainly rather than discovering later — if they need 1.7 readers to cope, the
answer is the payload convention in §5, not a new variant.

---

## 3. Design — record type 13, `Commitment`

Modelled on records 11 and 12 (`crates/smysl-core/src/types/lifecycle.rs`).

```rust
/// A commitment to a unit (record type 13, 1.8).
pub struct Commit {
    pub unit: Uid,
    pub level: Commitment,
    pub agent: AgentId,
    pub ts: Hlc,
    /// A unit saying why, as a withdrawal's `reason` does.
    pub note: Option<Uid>,
    pub extra: Extra,
}

/// Higher is more settled. Independent of `Status`, whose order is rule M.
#[repr(u8)]
#[non_exhaustive]
pub enum Commitment {
    Floated = 0,
    Drafted = 1,
    Committed = 2,
    Canonical = 3,
    Retconned = 4,
}
```

Body keys: `0 unit`, `1 level`, `2 agent`, `3 ts`, `4 note`. `HIGHEST = 4`.

### Merge semantics, which the RFC does not address and must

A store is a join-semilattice (rule U): merge is a set union of records and every derived answer
must be independent of arrival order. Commitment records union like everything else; the question
is what `commitment_of(uid)` returns when two disagree.

**Proposed: the highest `(ts, agent)` wins, and concurrent disagreement is a detection, not a
silent pick.** Precisely:

- `Store::commitment_of(&Uid) -> Option<Commitment>` — the level from the record with the greatest
  `(ts.wall_ms, ts.counter, agent)`, which is a total order and therefore order-independent.
- When two agents' latest commitments differ and neither `ts` happens-before the other, merge
  reports a `DetectionKind::CommitmentFork` contention, exactly as a supersession fork is reported
  today. Detections are reported, never recorded (§5.4) — the same reasoning applies unchanged.

Taking the *maximum level* instead was considered and rejected: it makes retraction of a
commitment impossible, so `Retconned` could never take effect, and "the most committed anyone ever
was" is not what a ledger means.

### The check pass

`Pass::CommitmentSupport`: a unit may not be more committed than the weakest thing it grounds on,
mirroring rule M on the new axis. New diagnostic, appended to the registry so no published code is
renumbered:

```
SMY-W057  A unit is more committed than the weakest thing it rests on
```

A **warning, not an error**: unlike rule M, which is a claim about the world being wrong, an
author outrunning their own foundations is a normal intermediate state of a draft. A gate that
refused it would make the ledger unusable during the work it exists to support. inkhaven can
escalate with `--strict` as any consumer can.

The RFC is right that this costs ~6 hand-maintained sites, and right that an implemented pass runs
whenever `CheckOptions.only` is empty. Two consequences to decide up front:

- Every existing consumer's `check` output gains a warning class it has never seen. That is
  acceptable for a warning and would not be for an error.
- A pass that is *strictly* opt-in needs `only`, which is awkward for a caller who wants
  "everything except this one". If we want that, the honest fix is a `CheckOptions::without(Pass)`
  — additive, and useful beyond this pass.

### Surface syntax

`@commit <label|uid> { level: canonical, agent: human:vu, ts: [ms, counter] }`, following
`@withdraw` / `@resolve` (§4). A new reserved word is a §8.1-permitted change: an older reader
rejects the *surface* document and reads the CBOR form fine.

---

## 4. Work breakdown

Ordered so each step is independently green. Sizes are relative to the 1.4 cycle, which is the
closest precedent.

| # | Step | Touches | Notes |
|---|---|---|---|
| 1 | `SourceKind` forward compatibility | `epistemics.rs`, `cbor/envelope.rs`, spec §3.1 | Prerequisite for step 2; worth doing alone |
| 2 | `SourceKind::Node` + adapter convention | `epistemics.rs`, surface parse/write, spec | Small once 1 is done |
| 3 | `Commitment` + `Commit` record type 13 | `types/`, `cbor/{keys,envelope}.rs`, `Record` | The core; mirrors 1.4's records 11/12 |
| 4 | Surface `@commit` | `surface/{lex,parse,write}.rs` | `LineClass` variant appended last (semver) |
| 5 | `Store::commitment_of`, `commitments_of` | `smysl-graph/src/store/` | Derived, order-independent |
| 6 | `CommitmentFork` detection | `smysl-graph/src/merge/` | `DetectionKind` variant appended last |
| 7 | `Pass::CommitmentSupport` + `SMY-W057` | `smysl-check`, `diag.rs` | Registry entry appended in a trailing group |
| 8 | CLI: `smysl commit`, `check` reporting | `src/main.rs`, cli-surface | 26th command |
| 9 | Conformance fixture + the other three implementations | `fixtures/wire/`, `python/`, `nodejs/`, `go/` | Non-negotiable: 1.4's precedent, and 1.6 proved a fixture catches real defects |
| 10 | Rendering the axis | `smysl-render` | New code, V1/V2 precedent; **propose deferring** |
| 11 | Spec, book, changelog | `SMYSL_FORMAT_SPEC.md` §3.1/§4/§8.1, manual, `CHANGELOG.md` | |

**Proposed cut for 1.8:** steps 1–9 and 11. Step 10 (rendering) is the one piece with no consumer
yet — inkhaven's §6 describes a ledger it reads through `trace`, `diff` and `retract`, not through
a rendered profile — and rule V1 is `Status`-specific enough that doing it speculatively would be
guessing at a marker vocabulary nobody has asked for. Ship the data and the checks; render when
someone needs a rendered commitment.

---

## 5. What inkhaven can do today, without waiting

Worth stating because it may change their sequencing, and because a plan that only says "wait for
1.8" is not useful:

- **Commitment as an extension payload.** `@x.canon/decision d/motive { "canon:commitment":
  "canonical" }` works now, is filterable now (`find --payload canon:commitment=canonical`, 1.6),
  and is retrievable now (extension-typed units became retrievable in 1.7). The cost is exactly the
  one the RFC identifies: payload is inside `UnitCore`, so changing a commitment **changes the
  uid** — "my commitment changed, not the content" becomes a new unit and a `supersedes` edge. For
  a prototype answering "would the author keep a canon ledger", that may be perfectly adequate, and
  it tests the hypothesis before either project spends a cycle.
- **Host back-reference as a `Doc` source.** `source: { kind: doc, ref: "inkhaven:<uuid>#ch3/scene2" }`
  carries the same information as `SourceKind::Node` today, and 1.5's
  `Store::units_with_source_prefix` already answers "which units came from this node" by prefix.
  `Node` makes it *typed* rather than conventional, which is worth having and is not a blocker.

If both hold up in their ~1-user test, 1.8 turns the convention into format. If they do not, we
have not spent a cycle on a record type nobody uses.

---

## 6. Open questions for inkhaven

1. **Do you need 1.7 readers to read `Node` sources?** If yes, use the `Doc` convention (§5) and
   we skip the variant; if no, §2's ordering applies.
2. **Is `Retconned` a commitment level or a relation?** smysl already has `Supersedes`, and the RFC
   itself pairs them. A level *and* an edge can disagree; one of them should be the answer. Our
   inclination: keep `Retconned` as a level for the human-facing axis, and say in the spec that the
   edge is authoritative for traversal.
3. **Who commits?** If inkhaven harvests on save, most commitment records will carry `model:` or
   `tool:` agents. Does an author's explicit commitment need to outrank a harvested one — and if
   so, is that a rung question (rule T) rather than a timestamp question?
4. **Multilingual (RFC §5).** Agreed it is its own cycle. One correction: 1.6 made the retrieval
   tokenizer pluggable (`Bm25::index_with(Tokenizer)`), so the retrieval half is closer than the
   RFC assumes; the diagnostics and render registers are the larger part.
