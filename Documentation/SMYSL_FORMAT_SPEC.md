# smysl — normative format specification

**Status:** normative. This document is the contract.
**Format version:** `smysl/1.0` — `smysl/0.1` is also accepted and always will be (§8.6).
**Kernel schema:** `smysl.kernel/0.1`.
**Describes:** crate `1.7.0`.

This is the whole of what a second implementation must obey to interoperate. It is
deliberately short. Everything it does not say is a free choice.

Three other documents exist and **none of them is normative**:

- `SMYSL_MANUAL.typ` teaches the tool and the reasoning. 254 pages, and none of it binds you.
- `SMYSL_ARCHITECTURE_RFC.md` describes how this implementation is built. Useful, not required.
- `RFC_PROPOSAL.md` is a historical record of decisions taken while implementing the original
  sketch. Its conclusions are folded in here; it is kept for the reasoning, not the rules.

**RFC SMYSL-1 is retired.** It was the product idea, written before there was an
implementation, and it stated it had zero open design decisions. Building it produced 69
places where it was silent, self-contradictory, or contradicted by a live endpoint. The
implementation had to choose something in order to exist, and it chose. Where this document
and RFC SMYSL-1 disagree, this document is correct and the RFC is history.

---

## 0. Implementations of this document

Three, besides the Rust that defined the format, each written from this document: `python/`,
`nodejs/` and `go/` in this repository. **All three reach C-Produce** as of 1.2.0. They run in
CI against fixtures the Rust produced, and a byte-for-byte match is what "two implementations
agree" means in practice.

The distinction between the classes is worth stating, because it is the difference between two
useful things. C-Read says a document means the same to two readers. It does **not** reach
§2.1 — reading never requires deriving a uid — so all three could round-trip every fixture byte
for byte while remaining ignorant of what a uid is, which is what happened for a full release.
§2.3, *status is part of identity*, is the paragraph this format rests on, and C-Produce is the
lowest class that touches it. Each of the three derives uids over a BLAKE3 written for the
purpose, and reproduces the reference implementation's — canonical bytes checked separately
from the hash, in `fixtures/wire/uid/`. Four independent derivations of §2.1, where there was
one until 0.10, two until 1.1 and three until 1.2.

`go/` and `nodejs/` also implement the *shape* half of C-Produce that §7 names and `python/`
does not: they refuse to derive a uid for a unit with no gist, with `derived` or `inferred` and
no grounds, with `measured` or `cited` and no source, or with an authored `unfounded`.

**Each reading has found something, which is the argument for the next one.** The first two
found three places §3 was insufficient, now constraints 1, 2 and 8. The third found a fixture
that could not fail. The fourth found four things §2.2 and §2.1 did not say — the status
integers, the `source` map's layout, the base32 alphabet, and one place the table said the
*opposite* of what the encoder does — all four now written down, and all four previously
recoverable only by decoding a fixture. Two implementations had already reached C-Produce
through those gaps without recording that they had guessed.

**As of 1.4 all three name records 11 and 12, round-trip them, and derive the rid (§2.5) and the
contention id (§6.2)** from the vectors in `fixtures/wire/relation-id/` and
`fixtures/wire/contention-id/` — so the two identities a withdrawal and a resolution depend on
had four derivations before the format committed to them.

They exist because every other check in this repository tests whether the Rust is
self-consistent, and none of them would notice if this document were blank. If you are
implementing the format, read them as worked examples — and read their `SPEC:` comments as a
record of where this document has already been found wanting.

## 1. What interoperability means here

Identity is content. A unit's uid is a hash of its content, so two implementations that
encode the same unit differently do not merely differ in bytes — they **disagree about what
the unit is**, and every reference, merge and pack built on that uid is wrong.

So the requirement is stronger than "parse each other's files." It is:

> Given the same unit, two conformant implementations MUST produce byte-identical canonical
> CBOR, and therefore the same uid.

Everything in §2–§4 exists to make that achievable. If you implement nothing else here,
implement those.

## 2. Identity

### 2.1 Uid derivation

```
uid = BLAKE3-256( canonical_cbor( unit_core ) )
```

The digest is the full 32 bytes. The **canonical text form** is `b3:` followed by all 256
bits as 52 base32 characters. A **display form** of `b3:` plus the first 130 bits as 26
characters is permitted where a human reads it; a parser MUST accept 26 to 52 characters and
MUST NOT accept fewer. An abbreviated uid is a display convenience and never appears in
canonical CBOR, which carries the raw 32 bytes.

The base32 is **RFC 4648, lowercased, unpadded** — `abcdefghijklmnopqrstuvwxyz234567`, not
base32hex — and bits are taken most-significant first, so the 52nd character covers bits
255–259 and its last four are zero. This does not affect a uid, which travels as raw bytes;
it is stated because the obligation to accept 26 to 52 characters means nothing between
implementations that disagree about which thirty-two symbols are meant. base32hex was an
equally faithful reading of the sentence above until this paragraph existed.

### 2.2 What is hashed

The **unit core**, and only the unit core: a CBOR map with integer keys, emitted in
ascending key order, omitting absent optional fields entirely.

| key | field | type | presence |
|---:|---|---|---|
| 0 | schema | text | required |
| 1 | gist | text | required |
| 2 | body | text | optional |
| 3 | detail | text | optional |
| 4 | deps | set of uid | required, MAY be empty |
| 5 | grounds | set of uid | required, MAY be empty |
| 6 | status | uint | required |
| 7 | source | map | optional |
| 8 | payload | bytes | optional |
| ≥9 | unknown keys | any | preserved verbatim (§5, rule X) |

An absent optional field MUST be omitted, never encoded as `null`. `deps` and `grounds` are
**sets**: deduplicated, and sorted by uid bytes.

**An empty `deps` or `grounds` is omitted, exactly as an absent optional is.** "Required, MAY
be empty" above is a statement about the set, not about the encoding: the key is required to
*mean* something — the empty set, not "unspecified" — and an empty one is spelled by leaving
the key out. The `minimal` case in `fixtures/wire/uid/cases.json` is a three-key map over 0, 1
and 6. This sentence exists because the table without it reads naturally as "the key is always
present", which is off by two keys and therefore by a whole identity; a fourth implementer read
it that way in 1.2.

`status` is one of six values, and the integers are **normative** — rule M compares them as
integers, so an implementation that agreed on the names and not on the order would derive wrong
uids *and* enforce a different monotonicity rule while believing itself conformant:

| value | status |
|---:|---|
| 0 | unfounded |
| 1 | speculative |
| 2 | inferred |
| 3 | derived |
| 4 | cited |
| 5 | measured |

`source` (key 7) is a map with integer keys on the same rules as the core:

| key | field | type | presence |
|---:|---|---|---|
| 0 | kind | uint | required |
| 1 | reference | text | required |
| 2 | captured | text | optional |

and its `kind`:

| value | kind |
|---:|---|
| 0 | url |
| 1 | file |
| 2 | metric |
| 3 | tool |
| 4 | doc |

The three tables above were added in 1.2.0. Until then a C-Produce implementer could recover
them only by decoding `core_bytes_hex` in the uid fixtures — which is to say the fixtures were
carrying normative content this document did not admit to having, and two implementations
reached C-Produce by reading them without either recording that it had to.

### 2.3 Status is part of identity

`status` is inside the hash. This is the single most consequential rule in the format and
the one most likely to be implemented by accident.

It means **a unit's uid changes when its status changes.** Promoting a claim from
`speculative` to `derived` does not update a unit; it produces a different unit. Anything
that transforms a unit moves its identity, and the old uid remains the name of the old
content. Implementations that treat status as mutable metadata will produce uids this
specification does not.

### 2.4 What is not hashed

Attestations, relations, threads, views, contentions, pack info, schema declarations, label
bindings, withdrawals and resolutions are records *about* units. They are never part of a unit's
uid. Two stores holding the same units with different attestations hold the same units.

An attestation's `uid` (key 0) names either a unit or a relation, by the relation's rid (§2.5).
It is attached to whichever the store holds, and kept until that arrives. Rule T does not read an
attestation on a relation, since a relation has no status; everything else in it means what it
means for a unit, which is what lets a store say who asserted an edge.

### 2.5 Relation identity

A relation has an identity of its own, derived rather than stored:

```
rid = BLAKE3-256( 0x03 ‖ kind ‖ 0x00 ‖ from ‖ to )
```

- `kind` is the relation kind's **name** in UTF-8 — `rebuts`, `x.verify/supports` — whether the
  record encodes it as a kernel integer or as text. Two encodings of one kind are one edge.
- `from` and `to` are the raw 32-byte uids.
- `0x03` is the relation record's type code. A unit's preimage is a canonical CBOR map, whose
  first byte is `0xa0`–`0xbf`, so no uid can equal a rid.
- `0x00` terminates the name; no kind name contains NUL.

`weight`, `note` and unknown keys are not identity. Two relation records with the same kind and
endpoints are one edge. A rid is written as a uid is: 32 raw bytes in CBOR, `b3:` and base32 in
text. `fixtures/wire/relation-id/cases.json` carries vectors, preimage and digest apart.

**Added in 1.4**, and no existing byte changed: every rid is a function of fields relations
already carried.

## 3. Deterministic CBOR

A conformant encoder MUST satisfy all of the following. A conformant decoder MUST reject
input that violates any of them (`SMY-E080`), rather than accepting and normalising it —
otherwise two byte strings decode to one record and a uid stops naming exactly one thing.

1. **Integer map keys** in the kernel. Text keys are permitted only inside a payload (§5).

   This binds an encoder. A decoder that knows it is reading a kernel map MUST reject a text
   key there; a decoder reading a value without that context — a generic reader, or one
   already inside a payload — MAY accept either, since it has nothing to check against.
2. **Shortest-form integers and lengths.** No integer, and no length prefix, encoded in more
   bytes than it needs.

   This does **not** apply to the payload of a float. `0xFA 3F 80 00 00` is 1.0, not an
   over-long encoding of 1 065 353 216, and a decoder that enforces shortest form on major
   type 7 rejects almost every real document.
3. **Definite lengths.** Indefinite-length arrays, maps, strings and byte strings are
   forbidden.
4. **Ascending key order**, by integer value in the kernel and by encoded key bytes in a
   payload map.
5. **No `null` for an absent optional.** Omit the key.

   The qualifier is load bearing. This forbids `null` as a stand-in for an omitted kernel
   field; it does not forbid an explicit null *inside a payload*, where the value is user data
   and `{"n": null}` is meant to be distinguishable from `{}`. The two do not collide: a
   payload is carried in the kernel as a byte string, so a reader walking the kernel never
   enters it, and a reader that does enter one is reading a document within a document.
6. **NFC text.** Every text string is Unicode-normalised to NFC before encoding, including
   unknown payload keys and their string values. Normalise *at the encoder*, not only in the
   constructors that happen to be remembered: this implementation asserted the invariant in
   debug and trusted it in release until 0.6, and six free-text fields reached the encoder
   unchecked the whole time. Two of them were found by fuzzing, in separate releases.
7. **Floats are binary32, quantised to 1/1024.** `round(v · 1024) / 1024`. Non-finite input
   saturates to the largest representable multiple rather than encoding an infinity.
8. **No tags.** Major type 6 does not appear in this format, and a decoder MUST reject one
   (`SMY-E080`). The kernel's shape is fully described by constraint 1 and §2.2, so a tag
   could only introduce a second encoding of a value already expressible — which is the thing
   every constraint here exists to prevent.
9. **Nesting is bounded at 128.** Deeper input is rejected. Unbounded recursive descent
   aborts the process on hostile input, which is worse than an error because it cannot be
   caught.

The paragraph on scope was added in 0.10.0, after a shared corpus of deliberately invalid
byte strings (`fixtures/wire/invalid/`) found the four implementations disagreeing about
seven of them. That exercise is the mirror of the one below: the three outside readers had
been checked only on what they *accept*, and the disagreement turned out to be in the
reference implementation rather than in the document.

Constraints 1, 2 and 8 read as they do because two independent implementations — `python/`
and `nodejs/`, each written from this document without consulting the other — both had to
guess here. Rules 2 and 8 caught both of them; rule 1 caught one. Their guesses agreed, which
is fortunate rather than reassuring: agreement between readers who both had to invent the same
answer is not the same as a document that told them.

Rule 4 has a consequence worth stating: a payload map is sorted by **encoded key bytes**, not
by the string's code points, and duplicate keys are collapsed keeping the first.

**Scope.** These constraints bind everywhere in a document, including inside payloads and
inside the values of keys the reader does not recognise. The latitude in constraint 1 is
narrow and specific — a reader without kernel context cannot tell whether an integer key is
*required* there — and it is not licence to relax constraints 2 through 9 for content that is
merely being passed through.

This is worth saying because preserved bytes are not inert. Rule X requires an unknown key to
survive verbatim, §2.1 derives a uid by hashing the unit core, and the unit core includes
those preserved bytes. A reader that skipped an unknown value without checking it would let
one logical unit have two encodings and therefore two uids, which is precisely what §3 exists
to prevent — and it would do so in the one place where nothing downstream can notice, because
the bytes are never interpreted. The reference implementation had this defect until 0.10.0.

### 3.1 Record framing

Every record is a two-element array: `[type_code, body]`.

| code | record |
|---:|---|
| 1 | unit core |
| 2 | attestation |
| 3 | relation |
| 4 | thread |
| 5 | view |
| 6 | contention |
| 7 | pack info |
| 8 | schema declaration |
| 9 | checkpoint |
| 10 | label binding |
| 11 | withdrawal |
| 12 | resolution |
| 13 | commitment |

An **unknown type code MUST be preserved verbatim and skipped semantically** (`SMY-W014`),
not rejected. Its body is still parsed strictly, so an unknown record cannot smuggle in a
non-deterministic encoding. A store is a concatenation of records with no framing envelope.

A decoder MUST NOT supply a default for a field the encoder always writes. If a record
cannot be re-encoded to the bytes it was read from, it MUST be rejected.

Records 11 and 12 were added in 1.4. A reader that predates them preserves them as unknown
records (`SMY-W014`), which is what makes them an addition rather than a break (§8.1). Their bodies:

**Withdrawal (11)** — an edge that should no longer be followed.

| key | field | type | presence |
|---:|---|---|---|
| 0 | relation | rid, 32 bytes | required |
| 1 | agent | text, an agent id | required |
| 2 | ts | HLC, `[wall_ms, counter, agent]` | required |
| 3 | reason | uid of a unit saying why | optional |

**Resolution (12)** — a record that a disagreement was reviewed.

| key | field | type | presence |
|---:|---|---|---|
| 0 | contention | text, a contention id (§6.2) | exactly one of 0 and 1 |
| 1 | relation | rid, 32 bytes | exactly one of 0 and 1 |
| 2 | agent | text, an agent id | required |
| 3 | ts | HLC, `[wall_ms, counter, agent]` | required |
| 4 | note | uid of a unit recording the decision | optional |

A resolution with both keys 0 and 1, or neither, MUST be rejected. What each record means is
§6.1 and §6.3.

Record 13 was added in 1.7, in the same way and for the same reason: a reader that predates it
preserves it as an unknown record (`SMY-W014`).

**Commitment (13)** — how settled a unit is, as a matter of decision rather than of evidence.

| key | field | type | presence |
|---:|---|---|---|
| 0 | unit | uid, 32 bytes | required |
| 1 | level | unsigned, `Commitment` below | required |
| 2 | agent | text, an agent id | required |
| 3 | ts | HLC, `[wall_ms, counter, agent]` | required |
| 4 | note | uid of a unit saying why | optional |

`Commitment` is an ordered enumeration, higher being more settled: `0 floated`, `1 drafted`,
`2 committed`, `3 canonical`, `4 retconned`. A value outside it MUST be rejected rather than
defaulted — the axis exists to record what an author decided, and a decoder guessing at it would
put a word in their mouth.

It is a **second axis, independent of `status` (§2.2)**. Status says how well the world supports a
unit and its order is rule M; commitment says how settled its author considers it, which for a body
of work settled by fiat is a different question. Neither order constrains the other.

**A unit's identity is untouched.** `commitment` is a record naming a uid, not a field of
`UnitCore`, so committing to a unit does not move it: *the content did not change, the commitment
to it did*, which is the event a development history exists to record.

**How settled a unit is, derived** (normative, because two implementations disagreeing would read
each other's ledgers differently): the level of the commitment with the greatest
`(ts.wall_ms, ts.counter, agent)`, which is a total order over a set and therefore independent of
the order records arrived in (rule U). A unit with no commitment record has **no** commitment,
which is not the same as `floated`. Taking the highest level ever asserted is NOT permitted: it
would make a commitment impossible to walk back, so `retconned` could never take effect.

**Pack info (7)** gained key 7, `reserved`, in 1.6: an unsigned integer, what the caller set aside
out of `budget` for the rest of the prompt a pack lands in. A new key in a record body above that
record's highest, which §8.1 permits — an older reader preserves it verbatim.

It is written **only when non-zero**, and a decoder that does not find it MUST read it as zero.
That is the one case where the rule above — a decoder MUST NOT supply a default for a field the
encoder always writes — does not apply, because the encoder does not always write it: a missing key
and a zero re-encode to the same bytes, so every pack written before 1.6 keeps the encoding it had.
A packer that reserves `r` out of `b` MUST solve against `b - r` and MUST report `budget` as `b`,
so that `used + reserved <= budget` holds for any reader that checks it.

## 4. Canonical surface form

Surface syntax is the human-facing form. It is **not** the identity-bearing form — uids come
from CBOR — but round-tripping must not silently change content:

> `parse → write → parse` MUST be a fixed point.

Consequences that are easy to get wrong, each of which has been a real defect:

- **Line endings are not content.** All trailing carriage returns are stripped, not one. A
  document checked out with CRLF and the same document checked out with LF are the same
  document. A carriage return inside a line is content and survives.
- **Whitespace around a gist is not content.** The assembled gist, including continuation
  lines, is trimmed.
- **A value that would re-parse as something else MUST be quoted** on output: one that looks
  like a number, `true`, `false` or `null`; one beginning `#` or `//`, which the header
  comment syntax would otherwise consume to end of line.
- **Keys need quoting too**, on the same principle: a key containing whitespace, `:`, `,`,
  `{`, `}`, `"` or `\`.
- **A unit carries one name.** Two labels may denote one uid, because identity is content;
  only one survives a round trip, and it MUST be the canonically first (`SMY-W054`).
- **A known field appearing twice keeps the first**, and the duplicate does not become an
  unknown-key payload — surface syntax cannot spell a second one.
- **`@withdraw` and `@resolve` spell records 11 and 12** (1.4), naming an edge as
  `from --kind--> to` or by its rid, and a contention by its id:
  `@withdraw c/a --rebuts--> c/b { agent: human:r, ts: [1726500000000, 0], reason: c/why }`.
  `ts` is `[wall_ms, counter]` with the record's agent as the clock's, as for `@thread`, so a
  record whose clock names a different agent, or that carries unknown keys, has no surface
  spelling and travels as CBOR only. A writer that knows the edge MUST spell it by its endpoints.
  `withdraw` and `resolve` are reserved words, as `doc`, `rel`, `thread` and `schema` are.
- **`@commit` spells record 13** (1.7), naming a unit by label or uid:
  `@commit d/motive { level: canonical, agent: human:vu, ts: [1726500000001, 0], note: c/why }`.
  `level` is one of the five names above. `commit` is a reserved word in the same way, and unlike a
  withdrawal or a resolution a commitment always has a surface form, because a uid can always be
  written.
- **A body or detail line opening `#`, `//` or `\` MUST be escaped with a leading `\`.** A
  line starting with a comment marker is a comment wherever it sits, so an unescaped one is
  read as a comment and the content is lost. Only those three sequences, and only at the
  start of a line.

## 5. Extensions (rule X)

Unknown header keys, unknown record types and unknown kernel types MUST survive a round trip
byte for byte. An implementation that drops what it does not understand breaks the format's
central claim, because a pipeline is a chain of implementations and the weakest one would
silently erase what the others rely on.

Unknown header keys are carried as a payload map with text keys. Unknown *kernel* types
degrade to a preserved-verbatim form and are reported (`SMY-W010`), never rejected.

Decoding and surface parsing deliberately differ here: an unknown type on the wire degrades,
but an unknown type in hand-written surface text is a typo and stays an error.

## 6. The rules

Named so they can be cited. Numbered constraints C1–C8 for packing are in the manual; these
are the format-level obligations.

| rule | obligation |
|---|---|
| **M** | Monotonicity — a `derived` or `inferred` unit MUST NOT exceed the status of its weakest present ground. |
| **T** | Trust ceiling — a status MUST NOT exceed the ceiling its attestation's rung allows. |
| **L** | Closure — a thread's steps MUST reference units whose dependencies are present. |
| **R** | Rebuttals travel — a selection containing a claim MUST contain its live rebuttals (§6.1). |
| **U** | Merge is a join-semilattice: commutative, associative, idempotent. |
| **I** | Ingest progress — a unit that cannot be repaired degrades rather than failing the batch. |
| **S** | Staging — ingested units are staged, not committed, until accepted. |
| **V1/V2** | Rendering — provenance and contentions are shown or suppressed per profile, never silently. |
| **X** | Extensions survive (§5). |
| **D** | Determinism — pure operations are bit-reproducible functions of their inputs. |
| **P** | On a pipe, stdout defaults to CBOR. |

Rule **U** deserves emphasis for the same reason as §2.3: nothing detects a violation from
inside one peer. Two agents gossiping in different orders reach different stores and each
believes itself. It holds at the level of records, not only of meaning: a store merged with
itself gains no records, whatever their type.

### 6.1 Withdrawal and live rebuttals

**A relation is withdrawn in a store iff the store holds at least one withdrawal naming its
rid.** A withdrawn relation MUST be preserved — its record round-trips — and MUST NOT be followed
by anything that interprets the graph. A withdrawal is permanent, as a retraction is: records
only accumulate. One whose relation is not yet in the store takes effect when it arrives.

A `retracts` or `supersedes` edge **cannot be withdrawn**. A withdrawal naming one is preserved
and has no effect (`SMY-W056`): un-retracting a unit and re-pointing supersession need rules of
their own.

In a store under a retraction policy — **strict** where an implementation has no notion of one —
a relation `(rebuts, a, b)` is **live** iff it is not withdrawn, `a` is present, and `a`'s
effective status is not `unfounded` (retracted, or orphaned by retraction). Rule R binds live
rebuttals only: **a retracted or withdrawn rebuttal no longer travels with its claim.**
Supersession of `a` does not end liveness; a better version of an objection is not a withdrawal
of it.

### 6.2 Contention identity

Merge detects contentions and does not record them, so two implementations that detect the same
disagreement must name it the same thing — and a resolution (§6.3) names it by that name:

```
digest = BLAKE3-256( kind ‖ over ‖ positions )
id     = "k/c" ‖ base32( first 130 bits of digest )
```

`kind` is one byte: 0 supersession fork, 1 live rebuttal, 2 label collision. `over` is 32 bytes;
`positions` are the uids sorted and deduplicated, 32 bytes each. The base32 is §2.1's, 26
characters. The clock a detection is stamped with is not identity.
`fixtures/wire/contention-id/cases.json` carries vectors.

A live-rebuttal contention is detected only over a live rebuttal (§6.1) whose claim is not
`unfounded`.

### 6.3 Resolution

**A resolution records that a disagreement was reviewed. It never decides the outcome.** A
reviewer who finds a claim wrong retracts it; one who finds a rebuttal wrong retracts it or
withdraws the edge. Those records have their effects; the resolution records only that someone
looked, and who.

- A contention named by at least one resolution reads as **resolved**, whatever status its own
  record carries, and no longer pins its positions into a selection.
- A recorded live-rebuttal contention whose rebuttal is no longer live, or whose claim is
  `unfounded`, reads as **stale**, and pins nothing.
- A `rebuts` edge named by a resolution **stays live** — rule R still binds — and is no longer
  an item awaiting review.

Resolutions accumulate like withdrawals. There is no reopening a resolved item in this version.

## 7. Conformance classes

An implementation declares what it does, not how complete it is.

The classes are **not a single ladder**. They branch, because a consumer and a merger need
different things and neither needs everything.

| class | obligations |
|---|---|
| **C-Read** | *structural* — decode, re-encode byte-identically, reject non-deterministic encoding, preserve unknowns (§5). |
| **C-Consume** | structural + *epistemic* — enforce rules M and T when interpreting status, and reject an authored `unfounded`. |
| **C-Produce** | structural + epistemic + *shape* — emit well-formed units: a gist present, grounds where the status demands them, a source where `measured` or `cited` demands one. |
| **C-Merge** | structural + epistemic + *lifecycle* — honour retraction and supersession, withdrawal and resolution (§6.1, §6.3), attach attestations to relations (§2.4), and detect contentions under §6.2's identity. |
| **C-Full** | all of the above, plus *rendering* obligations. |

Note that **C-Merge does not subsume C-Produce**: an implementation that merges stores need
not be able to author well-formed units of its own, and one that authors need not implement
retraction. Declare what you do.

C-Read is the floor and everything rests on it. An implementation that cannot round-trip
bytes is not conformant at any class.

## 8. Versioning

The **crate version and the format version are independent axes**. A crate major bump does
not imply a format break, and a format break does not require one. `smysl/0.1` has not
changed across crate versions 0.1 through 0.9, and record type 10 was *added* in 0.2 without
a format bump — an older reader preserves it verbatim under rule X, which is exactly what
rule X is for.

An implementation MUST reject a format version it does not support and MUST NOT guess.

### 8.1 What may change within a format version

Three kinds of change are permitted without a bump, and they are permitted because rule X
already obliges every reader to cope with them:

- **A new record type code.** Older readers preserve it verbatim and report `SMY-W014`.
- **A new unit-core key ≥ 9, or a new header key.** Older readers preserve it verbatim.
- **A new key in any other record body**, above the highest key that record defines. Older
  readers preserve it verbatim. Stated in 1.4; the reference implementation always did.
- **A new reserved word in surface syntax** (`@schema` in 1.3, `@withdraw` and `@resolve` in
  1.4, `@commit` in 1.7). An older reader rejects a *surface* document using it — it reads the word as a unit type
  and fails on the label — while the CBOR form of the same store reads everywhere. Surface text is
  not the identity-bearing form (§4), so this is a cost to state rather than a break.
- **A new value in an open enumeration** where this document says unknown values are
  preserved rather than rejected.

The test of "permitted" is mechanical: a reader written against this document at the *older*
revision must still round-trip a document containing the addition, byte for byte. If it
cannot, the change is a break however small it looks.

### 8.2 What requires a new format version

- Changing the meaning, type, or key number of anything in §2.2 or §4.
- Adding a **required** field, or making an optional one required.
- Removing or renumbering a record type.
- Any change to §3, because §3 decides which byte strings are documents at all.

A new format version is a new string — `smysl/0.2` — and `FORMAT_VERSIONS_SUPPORTED` is a
list precisely so an implementation can accept several at once. Readers MUST NOT accept a
document whose version is absent from their list, and MUST NOT infer compatibility from the
version *looking* close to one they know.

### 8.3 Tightening an implementation is not a format change

This is the case that actually comes up, and the one most likely to be got wrong.

When an implementation has been *more permissive than this document requires*, correcting it
is not a format break, because the documents it stops accepting were never conformant. The
format did not change; an implementation stopped disagreeing with it.

0.5 made a decoder stricter about records it should never have accepted. 0.10 fixed
`skip_item`, which had been accepting seven classes of §3 violation inside extension payloads
for nine releases. Neither is a bump. Both are worth a changelog entry loud enough that
somebody with stored documents can check them, because *in practice* a document that used to
load may stop loading — and "it was never legal" is true and unhelpful to whoever has one.

The converse also holds, and is the harder discipline: if an implementation is more
permissive than this document and the permissive behaviour turns out to be *wanted*, the fix
is to change this document and bump, not to leave the two disagreeing.

### 8.4 Deprecation

Within a format version, nothing is removed. A field that should no longer be written is
marked deprecated here, writers stop emitting it, and readers keep accepting it — a reader
that started rejecting a document it used to accept has broken the format for everyone
holding one, which is the whole hazard content addressing is supposed to avoid.

Removal waits for a format bump. When one happens, implementations SHOULD accept both
versions for at least one release so that a pipeline with mixed implementations keeps
working, which is the only condition under which anybody can upgrade at all.

### 8.5 Where the version actually lives

Only in surface syntax, in the `@doc` header. **The wire carries no format version string.**

That is a deliberate consequence of rule X rather than an omission: a CBOR record sequence
describes itself through its type codes, and a reader meeting a code it does not know
preserves it verbatim instead of needing a version to tell it to. A version field would let a
reader refuse a whole document on sight, which is the opposite of what rule X asks for.

It had one consequence worth stating, because it was invisible until it bit. A surface parser
validated the declared version and then discarded it — there was nowhere in the parsed result
to keep it — so a writer reconstructed the header from its own `FORMAT_VERSIONS_SUPPORTED[0]`.
While that list had one entry the reconstruction was correct by coincidence. The moment it had
two, a document declaring the second would be read and written back claiming to be the first.
Uids are unaffected, because they are over CBOR and CBOR has no version — but the header would
have lied, and the next reader trusts the header.

**Fixed in 0.14, before the list grew rather than after.** `ParseOutcome` carries the version
the document declared, `WriteContext` carries what the header will say, and `write_surface`
emits that rather than a build-time constant. `smysl fmt` — the round trip a user runs on
purpose — passes one to the other.

`crates/smysl-core/tests/versioning.rs` was written in 0.10 to fail the moment the list grew,
and it did. What stands there now is the property it was standing in for: a document declaring
either supported version comes back declaring the one it declared. A count cannot say that; a
round trip can.

**One consequence for other implementations, which is that there is none.** The wire carries no
version, so an implementation that reads only CBOR — `python/`, `nodejs/` and `go/` all do —
has no version list to grow and nothing to change. Only a surface parser ever sees a `@doc`
header. This is worth stating because the migration plan first assumed otherwise.

### 8.6 Is the format frozen?

It was `smysl/0.1` for fourteen crate releases and four independent implementations, unchanged
throughout. The `0.` said a break was permitted if this document turned out to be wrong about
something load bearing. It never was, and that record — not a promise — is what `smysl/1.0`
is reporting.

What §8.2 buys either way is that a break would be *visible*: a new version string, refused by
old readers rather than silently misread.

**`smysl/1.0` arrived in 0.15, and is not a break.** 0.14 taught readers to accept both
strings and wrote nothing new; 0.15 made `smysl/1.0` what new documents declare. That order is
the whole point and §8.2 is why: a reader must refuse a version absent from its list, so
flipping the writer before the field has a reader for it makes every other implementation
reject the output. Readers first, released; the writer a release later.

`smysl/0.1` is still accepted and always will be. A document declaring it round-trips
declaring it — the writer emits the version a document *arrived* as, not the version this
build prefers. Only a document with no version to preserve, which means one built from CBOR,
gets the new default.

The bump carries no format change at all, which sits oddly beside §8.2's rule that a version
bump signals a break. The honest reading is that `smysl/1.0` marks the format *settled* rather
than changed, and that the compatibility event was teaching the readers — an event that
happened quietly, a release before anybody noticed the version.

**Nothing in this document changed when the writer flipped.** That is the claim `smysl/1.0`
makes, and it is checkable: the same fixtures parse, the same uids come out, and the
conformance suite did not move.

---

## Appendix: what this document deliberately omits

Command-line surface, exit codes, thread schemas, rendering profiles, salience weights, the
packing algorithm and its constraints C1–C8, the diagnostic registry, ingest and provider
behaviour.

None of it is required for interoperability. All of it is in the manual, and an
implementation is free to do any of it differently — or not at all — and still be conformant
at a class it declares. That freedom is the point: the original RFC specified a product, and
what actually needs specifying is an interchange format.
