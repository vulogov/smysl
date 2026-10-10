# smysl — normative format specification

**Status:** normative. This document is the contract.
**Format version:** `smysl/1.0` — `smysl/0.1` is also accepted and always will be (§8.6).
**Kernel schema:** `smysl.kernel/0.1`.
**Describes:** crate `1.10.0`.

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
| 3 | observed | uint, epoch milliseconds | optional |
| 4 | published | text, EDTF (below) | optional |

`observed` was added in 1.8 and is written only when present, so a source without one encodes to
the bytes it always did. It is the instant the observation was taken, **supplied by the
instrument and never read from a clock by an implementation** — the same arrangement `ts`
(§2.5) has, and what keeps a pure path pure. `captured` stays a date: it answers when a document
or dataset was obtained, which is a different and coarser question, and an implementation MUST
NOT derive one from the other.

Both are inside `source` and therefore inside the uid, so two readings of one metric at different
instants are **two units**. That is deliberate: collapsing them would lose the series.

`published` (key 4, added in 1.10) is the publication time of the manifestation the unit was
drawn from, **as the source states it**: a title page, a byline, a dateline, an export header. It
is written only when present, so a source without one encodes to the bytes it always did. The
three fields answer three different questions and an implementation MUST NOT derive one from
another when writing.

`published` is **EDTF** — ISO 8601-2, levels 0 and 1 — and the format stores the string. This is
what lets a source that says "about 1920" be recorded as `1920~` instead of a producer inventing
`1920-01-01T00:00:00Z`: the uncertainty is in the evidence. The format defines no calendar
arithmetic over it beyond what rule E (§6) needs to order intervals. A malformed value is
`SMY-E410`.

What levels 0 and 1 cover: dates at year, month or day precision; date-times with `Z` or a
numeric offset; the qualifiers `?` (uncertain), `~` (approximate) and `%` (both); unspecified
rightmost digits written `X` (`201X`, `1984-XX`); intervals with an **open** end (`..`, there is
no bound) or an **unknown** end (empty, there is one and nobody recorded it); negative years;
`Y`-prefixed years for the ones four digits cannot hold; and the seasons 21–24. Level 2 — sets,
interior `X`, per-component qualifiers — is not part of this format.

**One value has one spelling.** `published` is inside the uid, so a producer MUST write the
canonical form: four-digit zero-padded years with no leading `+`, `Y` only where four digits do
not suffice, uppercase `T` and `Z`, and `+00:00` rather than `-00:00`. Two byte strings for one
date would be two uids for one claim. `fixtures/library/edtf/cases.json` lists every accepted
form and, as importantly, the values that MUST be refused.

EDTF is proleptic Gregorian: year zero exists and is a leap year. A manifest that records a date
in another calendar says so (§3.1, manifest key 18); the value is stored **as written** and
converted only when intervals are derived.

Two rules bind a producer that draws a unit from a part, and both decide uids:

- **Copy.** A unit that names a manifest whose `published` (key 8) is present, and that carries
  no `observed`, MUST carry `published` equal to that value, byte for byte.
- **Omission.** `published` MUST be omitted when its normalised interval equals the degenerate
  interval `[observed, observed]` at millisecond precision. The comparison is between intervals,
  so it is exact: `2026-09-30T14:05:00Z` is a one-second interval and is not equal to any
  millisecond instant inside it.

Both present is permitted; `SMY-W411` is reported when the `observed` instant lies outside the
`published` interval. Neither present means the unit is undated on the *said* axis — not early,
not late.

A **decoder** does not refuse a malformed `published`. The field is inside the uid, so a value
that arrived has to leave again byte for byte or the unit silently changes identity, and a reader
that refused it would make a whole store unopenable over one field. The malformed value is
reported instead.

and its `kind`:

| value | kind |
|---:|---|
| 0 | url |
| 1 | file |
| 2 | metric |
| 3 | tool |
| 4 | doc |
| 5 | node |

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

### 2.6 Library identities

Four identities are derived for the library records of §3.1. Each is a BLAKE3-256 digest over a
one-byte domain prefix followed by a preimage, the convention §2.5 already uses for a rid.

| identity | names | preimage | domain byte | text form |
|---|---|---|---|---|
| **tid** | a part text (record 15) | the part's normalised bytes | `0x0f` | `t3:` + base32 |
| **mid** | a manifest (record 14) | the canonical CBOR of the manifest body | `0x0e` | `m3:` + base32 |
| **did** | a dating (record 17) | the canonical CBOR of the dating body | `0x11` | `d3:` + base32 |
| **rdid** | a part reading (record 18) | the canonical CBOR of the reading body | `0x12` | `r3:` + base32 |

The domain byte is the record type code the identity names. A unit's preimage is a canonical
CBOR map, whose first byte is `0xa0`–`0xbf`, so **no library identity can equal a uid, and no
two kinds of identity can equal each other** however their preimages collide. That is what lets
a record carry any of them in a 32-byte slot it defines as holding a uid — a withdrawal or a
commitment naming a dating (§3.1), a contention position — without the kinds becoming
confusable.

The base32 alphabet, the bit order, and the rule that a canonical form is 52 characters while 26
is a display abbreviation are §2.1's. A record carrying a 26-character form is `SMY-E071`, as it
is for a uid.

**Normalised bytes** of a part are UTF-8, NFC, line endings LF, no byte order mark. Nothing else
is changed: no whitespace collapsing and no case folding. An implementation that accepts a part
MUST verify both that its bytes are normalised and that they hash to the tid the record claims,
and report `SMY-E446` if either fails. It MUST NOT fail the *decode* on a mismatch: a record
that cannot be decoded cannot be reported, and one bad part would otherwise stop a whole store
from opening. `SMY-E401` is the neighbouring case — a re-read part whose structure hash or rdid
does not match its manifest's entry, where the bytes are intact and the reading changed.

`SMY-E401`, `SMY-E446` and `SMY-E452` are **allocated** by RFC SMYSL-2.4 §8 and named here so
that two implementations report the same condition under the same code. Each enters the
reference implementation's registry with the pass that raises it, not before: a code nothing can
trigger is worse than a missing one, because a reader waits for it.

**None of these is hashed into a uid.** A tid appears inside a unit only as text in
`source.reference` and as the referent of a span; a mid appears as `source.manifest`. So adding
them moved no existing byte.

`fixtures/library/wire/ids.json` carries vectors with the preimage, the body bytes and the
identity apart, for the reason `fixtures/wire/uid/cases.json` does: deriving an identity is not
reachable by reading a document, so an implementation that only reads could agree with every
byte here and still have no derivation at all.

**Added in 1.10**, specified by RFC SMYSL-2.3 A-3.

### 2.7 Record-set digest

The **record-set digest** of a set of records is

```
rsd = BLAKE3-256( "smysl/rsd/1" ‖ 0x00 ‖ h₁ ‖ h₂ ‖ … ‖ hₙ )
```

where each `hᵢ` is the BLAKE3-256 digest of one record's canonical encoding, §3.1 framing
included, duplicates are removed, and `h₁ < h₂ < … < hₙ` in byte order. It is therefore
independent of the order records arrived in and of duplicate delivery, which is what rule U
asks of any convergence test.

It covers **every record type, including types the implementation does not understand.** That is
the property it exists for, and the one a digest over derived state cannot have: derived state is
only as complete as the code that derives it, so a record the deriving code skips is a record the
digest cannot see.

Two stores **converge** (rule U) when their record-set digests are equal. An implementation MAY
also compare digests of derived state — salience, labels — and MAY require both to agree, which
makes a disagreement about derivation visible rather than silent. It MUST NOT report convergence
on derived state **alone**: the reference implementation's derived-state hash ignored
commitments, schema declarations, pack infos and records of unknown type, so two stores differing
in any of those compared equal, which is the opposite of what rule U needs.

**Added in 1.9, and no byte changed.** The digest is a function of records a store already held,
and it is written nowhere — no record carries it and no header declares it. So §8 does not reach
it in either direction: there is no byte added for an older reader to preserve, and nothing for a
format version to gate. What it is instead is a *test* this document can state, in place of the
one rule U needed and did not have.

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
| 14 | manifest |
| 15 | part text |
| 16 | reserved |
| 17 | dating |
| 18 | part reading |
| 19 | redaction |

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

Records 14, 15 and 18 were added in 1.10, in the same way again. Records 16, 17 and 19 are
**allocated and not yet defined here**: 16 is reserved and MUST NOT be emitted until a later
revision defines it, and 17 and 19 are specified by RFC SMYSL-2.3 A-5 and are folded in with the
release that writes them. A decoder MUST treat all three as unknown type codes, which is what it
already does with 9.

The three together are a **library**: a manifest says that an expression, at one version, is
these parts in this order, read this way; a part text carries one part's bytes; a part reading
carries what one reader derived from one part. Their identities are §2.6's.

**Where these records may rest** is normative, and it is the one rule here that is about storage
rather than about bytes. A log MAY hold 14. A log MUST NOT hold 15 or 18; an implementation
offered one for a log MUST refuse it with `SMY-E452`. Part texts and readings live in a content-
addressed object store, as their record envelopes, so that a bundle or an export emits them
verbatim and verification is decode-then-hash.

The reason is what record 19 asks for. Honouring a redaction means a store no longer holds
the part, and for an object store that is deleting a file; for a log it would mean rewriting the
log. That is the one operation an append-only log cannot survive as evidence, because it resets
the same hash chain that would have shown a rewrite — afterwards the log cannot distinguish the
redaction from an edit, so honouring the redaction and destroying the audit trail become the
same act. Refusing the record at the point it would enter the log is therefore cheaper than it
looks: a store with no library beside it can neither resolve a locator nor check a span, so all
it could do with the bytes is hold them. The rule that governs redaction is **rule Z**, folded in
with record 19 below.

**Manifest (14)** — an expression at one version.

| key | field | type | presence |
|---:|---|---|---|
| 0 | alias | text, the expression alias | required |
| 1 | parts | array of part entries, in reading order; MAY be empty | required |
| 2 | lang | text, BCP-47; `mul` for mixed, `und` for unknown | required |
| 3 | reader | text, a reader id and version, and its parameters: `osis/1`, `whatsapp/1 date-format=dmy` | required |
| 4 | licence | text, an SPDX id, `public-domain` or `unknown` | required |
| 5 | carry | unsigned: `0` none, `1` ref, `2` text | required |
| 6 | title | text, as recorded | optional |
| 7 | creators | array of text, as recorded | optional |
| 8 | published | text, EDTF, as recorded | optional |
| 9 | identifiers | map text → text | optional |
| 10 | origin | a `source` map (§2.2) | optional |
| 11 | parent | 32 bytes, the mid this derives from | optional |
| 12 | parent-kind | text: `translation`, `edition`, `excerpt`, `transcription` | required with 11 |
| 13 | supersedes | 32 bytes, the mid of the previous version of this expression | optional |
| 14 | versification | text | optional |
| 15 | lossy | bool, written **only** when true | optional |
| 16 | raw | map, the reader's manifest-level metadata, verbatim | optional |
| 17 | part-policy | text, the boundary rule and size targets | required |
| 18 | calendar | text: `julian` when key 8 is a Julian date; absent means Gregorian | optional |

Key 12 is required with key 11 and meaningless without it; either alone MUST be rejected, for
the reason a resolution with one target is. Key 15 has no `false` encoding: an absent key is
`false`, and a decoder MUST reject `false` on the wire, because admitting it would give one
manifest two byte strings and therefore two mids.

The **reader field** (key 3, and key 1 of a part reading) is the reader's id followed by the
parameters it was run with, which is why it is not an id alone: a parameter changes a reader's
output, so a corpus that recorded only the id would mean something else on re-read. A WhatsApp
transcript is the case that forced it — `03/04/2024` is the third of April or the fourth of
March, the file does not say which, and both readings are complete conversations that differ in
which **day** each message falls on, and therefore in how the text is cut into parts.

```
reader-field = reader-id *( SP parameter )          ; parameters in key order
reader-id    = name "/" 1*DIGIT                     ; name is lowercase ASCII and "-"
parameter    = key "=" value
key          = %x61-7A *( %x61-7A / DIGIT / "-" )
value        = 1*( %x21-7E )                        ; no space, no "="
```

Parameters MUST be in ascending key order, and a key MUST appear at most once: a field whose
spelling depended on the order a caller happened to pass its settings in would give one corpus
two manifest identities. A reader that takes no parameters writes the id alone, which is every
reader before `telegram/1` and `whatsapp/1`.

An **alias** is lowercase ASCII:

```
alias   = segment *( ( "/" / ":" ) segment )        ; at most 128 bytes
segment = 1*( %x61-7A / DIGIT / "-" / "_" / "." )
```

`import:` is reserved in the alias namespace. An over-long alias is shortened by the tool that
produces it, never rejected after the fact — so a decoder MUST NOT reject a record over its
alias, and the grammar is checked where there is an author to tell.

A **part entry** is `{0: tid, 1: length (unsigned), 2: structure hash (32 bytes), 3: rdid,
4: lang (text, only when it differs from key 2)}`. The structure hash is BLAKE3-256 of the
canonical CBOR of the segment table in the reading's key 2 — **over the table, with no domain
byte**, because it names a table and not a record. So a reading that gains raw metadata keeps
its structure hash and changes its rdid, which is why a part entry carries both.

**Part text (15)** — `{0: tid (32 bytes), 1: text (bytes, the normalised text)}`, both required.
It has no surface form: text is not written in surface syntax.

**Part reading (18)** — what one reader derived from one part.

| key | field | type | presence |
|---:|---|---|---|
| 0 | tid | 32 bytes | required |
| 1 | reader | text, a reader id and version, and its parameters, as manifest key 3 | required |
| 2 | segments | canonical CBOR, one row per segment | required, MAY be empty |
| 3 | raw | map, the reader's raw metadata for this part, verbatim | optional |

A segment row is `{0: start, 1: end, 2: level (text), 3: locator (text), 4: lang?, 5: speaker?,
6: observed?, 7: ids? (map text → text), 8: tz offset in minutes?}`. An empty table is an empty
**array**, not an absent value. Record 18 has no surface form either.

**Dating (17)** — a statement about when something happened.

| key | field | type | presence |
|---:|---|---|---|
| 0 | target | array `[kind, id]` (below) | required |
| 1 | axis | uint: `0` said, `1` composed, `2` about | required |
| 2 | value | a one-entry map (below) | required |
| 3 | basis | 32 bytes, the uid of the unit giving the evidence | optional |
| 4 | agent | text, an agent id | required |
| 5 | ts | `[wall_ms, counter]`, as every record with an `agent` carries | required |

A **target** is `[kind, id]`:

| kind | name | id | what it dates |
|---:|---|---|---|
| 0 | unit | a uid | one unit |
| 1 | part | a tid | every unit drawn from one part |
| 2 | manifest | a mid | every unit drawn under one manifest |
| 3 | window | `[tid, from_ms, to_ms]` | every unit of that part whose as-recorded `observed` falls in `[from_ms, to_ms)` |

A window is half-open and is matched against the **as-recorded** instant, never the effective
one: a window that moved as the datings it selects took effect would select a different set on
every pass, and rule E would not converge. A window with `from_ms >= to_ms` selects nothing;
that is a fact to report, not a record to refuse.

A **value** is a map with exactly one entry:

| key | value | means |
|---:|---|---|
| 0 | EDTF text (§2.2) | an absolute dating: "published in 1920?" |
| 1 | integer, milliseconds | an offset: "every clock in this window was 3 h slow" |
| 2 | `[allen, target]` | a relative dating: "this is during that" |

`allen` is **text**, one of `before`, `after`, `meets`, `overlaps`, `during`, `contains`,
`equals`. Text rather than a code because the relations are a closed set with standard names, so
a reader meeting one of the six this format does not use can say which word it did not
understand; a code could only say `7`. The offset is **signed** — a clock can be fast as well as
slow — and is the only field in this format that carries a negative integer; it is defined on the
*said* axis over instants only, because an offset applied to an interval of unknown width means
nothing. A value map with two entries is malformed: it would be a dating that says two things
with no rule for which wins.

Rule E (§6) names a fourth axis, *known*, and no dating can speak about it: it is the earliest
attestation clock of a unit and is never corrected.

A dating is a record **about** units, parts and manifests, and never an edit to them. Correcting
a unit's `observed` in place would change its uid, and a store that re-identified its contents
whenever a clock turned out to be wrong could not be cited. So a dating stands beside what it
dates, carries who said it and on what evidence, and can be withdrawn.

It has its own identity, a **did** (§2.6), because the records that name a dating need one. Its
surface form is
`@date <target> { axis: …, <value>, agent: …, ts: […][, window: [from, to]][, basis: <unit>] }`,
where the value is `when:` for an absolute EDTF value, `offset:` for a correction in
milliseconds, or the Allen relation itself as the key (`during: <target>`). A positional target
is a uid, a tid or a mid spelled in full; a window is its part's tid with a `window:` key, which
is how the one target kind that is not a single identity is written without a second grammar. A
relative value whose own target is a window has no surface form and travels as CBOR only.

**Withdrawals and commitments may name a did.** Key 0 of a withdrawal (11) MAY be a did as well
as a rid, and key 0 of a commitment (13) MAY be a did as well as a uid; the domain bytes of §2.6
keep the kinds of identity from colliding. A withdrawal naming a did makes that dating not live
(rule E). A commitment naming a did records how settled it is, and a `canonical` one holds
conflicting datings for review. A reader that predates this sees a withdrawal of an edge it never
receives, which is inert, and a commitment on a unit it never receives, which may appear in its
commitment listings. That is a stated cost, not a corruption.

**Redaction (19)** — this part's text is to be held no longer.

| key | field | type | presence |
|---:|---|---|---|
| 0 | tid | 32 bytes, the part whose text is redacted | required |
| 1 | agent | text, an agent id | required |
| 2 | ts | `[wall_ms, counter]`, as every record with an `agent` carries | required |
| 3 | reason | 32 bytes, the uid of a unit saying why | optional |

A redaction has **no identity of its own**: it is a statement about a part, named by that part's
tid, as a withdrawal is a statement about an edge. Its surface form is
`@redact <tid> { agent: …, ts: […][, reason: <unit>] }`, and the tid is spelled in full — the
26-character short form is refused here as it is everywhere, because an abbreviated identity in a
redaction names either no part or the wrong one.

**Rule Z** is normative and is about storage, like the rule above it:

> A store MUST NOT hold a record 15 or 18 whose tid any redaction in that store names. An
> implementation offered one MUST drop it rather than refuse the batch, and MUST count what it
> dropped. The rule is enforced over the **union** of the redactions a store holds and those
> arriving in the same batch.

Dropped rather than refused, and the distinction carries the whole design. A peer that never saw
a redaction will offer the part text in good faith on every merge; refusing the batch would make
one redaction anywhere in a network a permanent merge failure, and the obvious workaround is to
stop merging. Dropping makes the union of two stores the same store whatever order they merge in,
and keeps a redaction a fact that only ever spreads. (`SMY-E452` is the *other* case and stays a
refusal: a record 15 or 18 offered to a log is refused whether or not anything is redacted,
because a log is not where text may rest.)

Honouring a redaction is therefore an **unlink in an object store** and never a rewrite of a log,
which is the point of the paragraph above: a log that held text would one day have to be rewritten
to honour one of these, and rewriting an append-only log resets the hash chain that would have
shown the rewrite.

A **text-keyed map** — key 9 of a manifest, key 7 of a segment row, the `raw` maps — is ordered
by its **encoded keys**, as §3 constraint 4 requires of any map. A text head carries its length
first, so `"a"` precedes `"doi"` whatever the letters are. That is not string order, and a
decoder MUST enforce the encoded order rather than the decoded one, or it accepts stores that a
decoder treating the same map as an unknown value rejects.

**Pack info (7)** gained key 7, `reserved`, in 1.6: an unsigned integer, what the caller set aside
out of `budget` for the rest of the prompt a pack lands in. A new key in a record body above that
record's highest, which §8.1 permits — an older reader preserves it verbatim.

It is written **only when non-zero**, and a decoder that does not find it MUST read it as zero.
That is the one case where the rule above — a decoder MUST NOT supply a default for a field the
encoder always writes — does not apply, because the encoder does not always write it: a missing key
and a zero re-encode to the same bytes, so every pack written before 1.6 keeps the encoding it had.
A packer that reserves `r` out of `b` MUST solve against `b - r` and MUST report `budget` as `b`,
so that `used + reserved <= budget` holds for any reader that checks it.

**Open and closed enumerations.** Five of the format's enumerations are **open**: `source`
`kind` (§2.2), thread schema, step role and detection kind from 1.9, and `admission` from 1.10.

- A decoder MUST preserve a code it does not know, re-encode it unchanged, and report
  `SMY-W409`. Wherever the value is interpreted it is treated as *unknown* — not as a default,
  and not as an error. Failing the whole store on an unrecognised code is what this replaces,
  and correcting it is a tightening of readers rather than a change of format (§8.3): a document
  carrying a future code was always legal, and refusing it broke rule X one level below the
  record type, where nothing was watching.
- **Code 255 is reserved in each of the five and is never assigned.** An implementation MAY use
  it internally to mean "unknown" — except for `source` `kind`, which is inside the uid (§2.2),
  where normalising an unrecognised code to 255 would give the unit a different identity and do
  it silently. There the raw code travels with the container, and from 1.10 an unknown
  `admission` travels with the granularity map the same way, for the weaker reason that two
  readers disagreeing about whether to keep it compute different record-set digests (§2.7) for
  one store.
- **`admission` opened in 1.10 and cost nothing**, which is what reserving 255 for it in 1.9 was
  for: no registry change, no renumbering and no new diagnostic. 1.9 held it closed on the
  argument that the granularity bounds read it and a reader that guessed would report the wrong
  verdict. The argument was right about guessing and wrong about the remedy: failing the decode
  does not avoid a wrong verdict, it refuses to open the store. An unknown admission is
  therefore preserved, reported, and **suspends** the checks that read it — which is not a
  verdict in either direction.
- `status` (§2.2), `lod`, `op`, `rung` and `commitment` stay **closed**. Rules M, T and L read
  the first of these and an unknown value cannot be compared, so an unknown code in a closed
  enumeration is still an error. A decoder MUST NOT invent a member of one.

The *members* of these enumerations, beyond `status` and `source` `kind` in §2.2, are not
specified here — the appendix says why. Their openness is, because preserving a member you do not
recognise is a round-trip obligation and therefore interoperability rather than product.

A new code in an open enumeration is an addition §8.1 permits. A new code in a closed one is not.

**The view's granularity map gained key 5, `estimator`, in 1.9**: text, the id of the estimator
whose count the map's size bounds are stated in. A new key in a record body above the highest
that body defines, which §8.1 permits — an older reader preserves it verbatim.

What those bounds *are* is a product decision, and the appendix omits it. Which count they are
read in is not, because two readers that disagree about the count reach different verdicts on
one document while both believing themselves conformant — the §2.2 failure mode, one level out
from identity.

It is written **only when it is not `smysl/utf8-div4`**, so every view written before 1.9 encodes
to the bytes it had, and a decoder that does not find the key MUST read it as `smysl/utf8-div4`.
That is the second case where the rule above — a decoder MUST NOT supply a default for a field the
encoder always writes — does not apply, and for the same reason as `reserved`: the encoder does
not always write it, and a missing key and the default re-encode to the same bytes.

A bound and the number compared against it MUST come from one estimator:

| id | count of a text `t` | notes |
|---|---|---|
| `smysl/utf8-div4` | `ceil(utf8_len(t) / 4)` | the default, and the meaning of an absent key |
| `smysl/content/1` | `ceil( Σ_c n_c(t) · w_c / 1000 )`, with integer milli-weights `w_c` per character class | the classes and weights are fixed by `fixtures/estimator/content-1.json` |

An id names one count for ever: changed weights are a **new id**, never a redefinition, because a
bound written against the old weights would otherwise start meaning something else without
moving a byte.

A reader that does not know an id preserves it and treats every bound counted with it as
**unevaluable**. It MUST NOT substitute the default count. The bound is then neither met nor
breached and the reader says so — evaluating it under an estimator it was not written for would
return a verdict on a question nobody asked, which is worse than returning none.

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
- **`@manifest` spells record 14** (1.10), naming the expression alias in its header:
  `@manifest kjv/1769 { lang: en, reader: osis/1, licence: public-domain, carry: text,
  part-policy: "top/64Ki-4Mi", parts: [{ tid: t3:…, length: 97, structure: b3:…, rdid: r3:… }] }`.
  `manifest` is a reserved word in the same way, as are `date` and `redact` for records 17 and
  19. Records 15 and 18 have **no** surface form, so the set of records surface text cannot hold
  grew with this release. `@date` takes exactly one value key — `when`, `offset`, or one of the
  seven Allen words — and a header with two of them is `SMY-E001`: a dating says one thing, and
  the wire form is a one-entry map for the same reason. The five required keys are named rather than defaulted: `part-policy`
  in particular, because the policy a corpus was cut by is not recoverable from the parts and a
  default that changes later would silently redefine every manifest that omitted it. `parts` is
  the exception — omitting it means an empty one, which is an import manifest. A manifest
  carrying key 16, `raw`, has no surface form: it is opaque reader metadata, and a writer
  spelling it would be deciding what the bytes mean.
- **`source { }` accepts a closed set of keys** (1.9): `kind`, `ref` or `reference`, `captured`,
  `observed` and, from 1.10, `published`. Any other key is `SMY-E001`, a `captured` that is not a
  date is `SMY-E001`, an `observed` outside `u64` is `SMY-E001`, a `published` that is not EDTF
  is `SMY-E410`, and a `source` that fails to parse refuses the unit — none of the five is a
  silent drop. `source` is inside the uid (§2.2), so a key the parser
  ignored was a key that never reached the encoder: the unit written back was a *different unit*
  from the one the document described, carrying an identity nothing else refers to. Forward
  compatibility inside `source` is the wire's business, where an unknown key is preserved
  verbatim; surface text has an author to tell, which is the asymmetry rule X does not cover.
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
| **Z** | Redaction — a store holding a redaction for a tid holds no part text or reading for it, on any merge (§3.1, record 19). |
| **E** | Effective time — a unit's time on an axis is derived from the record set, never stored, and a weakly evidenced value never overrides a better one (§6.4). |
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

`kind` is one byte: 0 supersession fork, 1 live rebuttal, 2 label collision, 3 commitment fork
(1.7). `over` is 32 bytes;
`positions` are the uids sorted and deduplicated, 32 bytes each. The base32 is §2.1's, 26
characters. The clock a detection is stamped with is not identity.
`fixtures/wire/contention-id/cases.json` carries vectors.

A live-rebuttal contention is detected only over a live rebuttal (§6.1) whose claim is not
`unfounded`.

A **commitment fork** is detected when two distinct agents' latest commitments to one unit name
different levels. Per agent, not per record: one author revising their own commitment over time is
a revision and MUST NOT be reported. Its `over` is the unit and its single position is the same
unit — unlike a supersession fork there are no rival uids, because the rival claims are levels,
which a reader finds in the commitment records themselves. A store still derives a single answer
(§3.1); the detection says a person should look, which is the division rule C draws everywhere:
merge computes, and does not adjudicate.

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

### 6.4 Rule E — effective time

> Each unit has up to four times: **said** (when it was said or published), **composed** (when
> the work was written), **about** (the time the claim refers to) and **known** (the earliest
> attestation clock of the unit, never corrected). Rule E derives the first three. Each is an
> interval of instants, or *undated*, or *contested*.
>
> **Effective time is never stored.** An implementation MAY index it and MUST recompute it when
> the record set changes.

That sentence is the rule. A clock in a corpus turns out to be wrong — an export ran three
hours slow, a title page says 1769 and the translation is from 1611 — and the repair cannot be
to correct the record, because `observed` and `published` are inside a unit's uid (§2.2) and a
store that re-identified its contents whenever a clock turned out to be wrong could not be
cited. So a correction is a **dating** (§3.1, record 17) standing beside the unit, and the time
a unit effectively has is recomputed.

**Time status.** Every time value is as well evidenced as whatever put it there:

| as-recorded value | time status |
|---|---|
| `observed` on a unit attested `Imported` at rung `computed` | measured |
| `observed` supplied by a reader from a platform export | cited |
| `published` on a unit or manifest | cited |
| a value supplied with no source | speculative |

A **dating** takes the status of its basis unit, and `speculative` with no basis. That is the
whole mechanism by which "the export header says so" outranks "it must have been about then"
without anybody ranking the two by hand.

**Free constraints** are orderings a store implies without anybody writing a dating:

| constraint | rule | status |
|---|---|---|
| quotation | if B quotes A then said(A) ≤ said(B) | the quoting unit's |
| derivation | a manifest is no earlier than its parent (key 11) | cited |
| supersession | a manifest is no earlier than the one it supersedes (key 13) | cited |
| reply | a reply is no earlier than the message it answers, by first-version `observed` | cited |
| first seen | nothing is said after the earliest attestation of it | derived |

**Liveness.** A dating is live unless a withdrawal names its did (§3.1), its basis is unfounded
(rule R), or it is **held**: a `canonical` commitment names it, or names the target's existing
dating and this one would change the effective value, until a resolution names the resulting
contention.

**Procedure** (normative). For one axis, over the constraint graph of a scope:

1. **Seed.** Each subject starts from `published` if present, else `[observed, observed]`, else
   *undated* with no bound. Each bound carries the time status of the value that set it.
2. **Strata.** For each status level `s`, from measured down to speculative: build a simple
   temporal network from the live absolute and relative datings and the free constraints whose
   status is at least `s`.
3. **Tighten.** Solve each stratum by shortest paths. A bound moves only if `s` is at least the
   time status of the value currently setting it. A move that `s` would make and may not is
   **not applied and not discarded**: it is recorded as a position of a detection-kind-4
   contention (`SMY-W412`). A path's status is the minimum of its edges', and a stratum holds
   only edges of status `s` or better, so every move it makes is justified at `s` — which is
   to say a chain tightens at its weakest link.
4. **Offsets** are applied to instants before propagation, and target the as-recorded instant.
5. **Inconsistency.** A negative cycle, or an empty interval, in any stratum marks every
   subject on it **contested**: a detection-kind-5 contention (`SMY-W413`) names the datings
   and constraints involved, and nothing is chosen.
6. **Order independence.** Every stratum is a simple temporal network, whose tightest solution
   is unique, so effective time is a function of the record set and not of arrival order
   (rule U).

A **window** target (§3.1) is matched against the **as-recorded** `observed`, never the
effective one: a window that moved as the datings it selects took effect would select a
different set on every pass and the solve would not converge.

**Calendars.** A value a manifest says is Julian (key 18) is converted to a Gregorian interval
in step 1, and never written back.

**Time contentions are derived, not recorded.** Their identity is §6.2's contention identity
with detection kind 4 or 5. A resolution (record 12) names that identity, and an implementation
MUST NOT write a record 6 with either kind — which keeps a store readable by an implementation
that predates them.

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
- **A new key in the `source` sub-map (§2.2)**, above its highest. This one needs the care a
  core key needs, because `source` is *inside* the uid: a source that does not carry the new key
  MUST encode to the bytes it always did, and an older reader MUST preserve one that does.
  `observed` (key 3, in 1.8) is the instance, and the preservation it relies on has been there
  since 1.7. **Stated in 1.10**, which is a release after the addition it permits — the list
  above had no bullet covering it, and §8.2's first line, read quickly, forbids it. Adding a key
  above the highest is not *changing* the meaning, type or number of anything already in §2.2,
  which is what §8.2 names; the two sections only looked as though they disagreed.

The test of "permitted" is mechanical: a reader written against this document at the *older*
revision must still round-trip a document containing the addition, byte for byte. If it
cannot, the change is a break however small it looks.

**The additions actually made**, so that the list above is checkable against something rather
than read as a policy nobody exercised:

| release | addition | kind |
|---|---|---|
| 0.2 | record type 10, label binding | new record type |
| 1.3 | `@schema` | new reserved word |
| 1.4 | record types 11 and 12, withdrawal and resolution | new record types |
| 1.4 | `@withdraw`, `@resolve` | new reserved words |
| 1.6 | pack info key 7, `reserved` | new key above a body's highest |
| 1.7 | record type 13, commitment; `@commit` | new record type, new reserved word |
| 1.8 | `source` key 3, `observed` | new key above a sub-map's highest |
| 1.9 | granularity key 5, `estimator` | new key above a body's highest |
| 1.9 | four enumerations opened (§3.1) | not an addition itself but a §8.3 tightening — it is what makes a later code in one of them an addition at all |
| 1.10 | record types 14, 15, 18 and 19; `@manifest`, `@redact` | new record types, new reserved words |
| 1.10 | `admission`, the fifth enumeration opened (§3.1) | a §8.3 tightening, as 1.9's four were |
| 1.10 | record type 17, dating; `@date` | new record type, new reserved word |
| 1.10 | `source` key 4, `published` | new key above a sub-map's highest |
| 1.10 | a did in withdrawal key 0 and commitment key 0 | a new value in an existing key, domain-separated from the kinds already there (§2.6) |

Keeping this list is what RFC SMYSL-2.3 A-14 asks for, and it is cheap insurance: an addition
nobody wrote down is an addition the next implementer rediscovers by decoding a fixture, which
§2.2 records happening four times in one release.

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

**The tightenings actually made**, named because this section's own paragraph above says a
changelog entry is owed to whoever has stored documents:

- **0.5** — a decoder stopped accepting records it should never have accepted.
- **0.10** — `skip_item` stopped accepting seven classes of §3 violation inside extension
  payloads, after nine releases of accepting them.
- **1.9** — an unrecognised code in one of the four open enumerations is preserved and reported
  (`SMY-W409`) instead of failing the store (§3.1). This one runs the other way: the reader became
  *more* accepting, and it belongs here anyway, because what it stopped doing was rejecting
  documents that were always conformant. No document stops loading; some start.
- **1.9** — the surface parser rejects an unknown key in `source { }`, a malformed `captured` and
  an out-of-range `observed`, and refuses the unit rather than building it without its source
  (§4). **This is the case the paragraph above is about:** surface documents that load today stop
  loading, and "they were never conformant" is true and no help to whoever holds one. What they
  were producing was a unit whose provenance differed from what its author wrote, with a uid to
  match.

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

Command-line surface, exit codes, rendering profiles, salience weights, the packing algorithm
and its constraints C1–C8, the diagnostic registry, ingest and provider behaviour.

Also the *members* of three enumerations §3.1 calls open — thread schema, step role and
detection kind — and of `lod`, `op`, `rung` and `admission`. What each code means is a product
decision: two implementations that disagree about whether code 6 is `exposition` still exchange
documents byte for byte, and each reports the codes it cannot name. What is **not** omitted is
whether those enumerations are open, and the reserved code, because preserving a member you do
not recognise is a round-trip obligation. `status` and `source` `kind` are in §2.2 instead, for
the one reason that overrides this: they are inside the uid.

None of it is required for interoperability. All of it is in the manual, and an
implementation is free to do any of it differently — or not at all — and still be conformant
at a class it declares. That freedom is the point: the original RFC specified a product, and
what actually needs specifying is an interchange format.
