# smysl-conformance — a third implementation, in JavaScript

Written from [`../Documentation/SMYSL_FORMAT_SPEC.md`](../Documentation/SMYSL_FORMAT_SPEC.md),
like the Python package beside it and for the same reason: the format's proposition is that
independent implementations agree on what a document says.

**Why a third and not just a second.** Two implementations that agree could still have made
the *same guess* where the document is silent — agreement is only evidence if the readings
were independent. This one was written from the spec without consulting the Python, and the
places where a guess was unavoidable are marked `SPEC:` in both. Comparing those marks is more
informative than either agreeing with the Rust.

They found the same two ambiguities independently, which is the useful result: constraint 2
did not say it applies to integers and lengths rather than float payloads, and major type 6
(tags) was not mentioned at all. Two readers, one document, the same two holes — **both now
written into §3**, along with a third that only the Python reader hit.

**Conformance target: C-Produce** — structural + epistemic + shape (§7). It decodes and
re-encodes byte-identically, preserves what it does not understand, derives uids, and refuses
to give one to a unit whose shape §7 forbids. No dependencies; `node --test` is built in. A
dependency doing part of the work would weaken the evidence.

```sh
cd nodejs
npm test          # 126 tests, no install step, Node 18+
```

## What it verifies

Every `.cbor` fixture in `../fixtures/wire/` was produced by the Rust and is decoded here,
re-encoded, and compared byte for byte — whole-store and record by record. Byte-identity is
the only assertion worth making: two implementations that both "parse fine" while disagreeing
about bytes disagree about *identity*, because a uid is a hash of the encoding.

Then the spec is walked clause by clause: each of §3's eight constraints as a rejection, the
§2.2 key table and §3.1 type-code table, unknown keys and unknown record types surviving under
rule X, and a store as bare concatenation with a truncated tail rejected rather than ignored.

Those two tables are checked against a **hand-typed copy** of the document, and this README
used to say "against the document", as did the READINESS gate. Nothing here parses markdown;
editing §2.2 and editing the test are two separate acts, and the test cannot notice the first.
`../scripts/verify-spec-tables.py` is what ties the copy to the document — it parses the spec's
tables and compares all four implementations against them, in both directions, and it runs in
CI. Until it existed in 1.2.0, every implementation read `SMYSL_FORMAT_SPEC.md` only to assert
that it contained the string "Deterministic CBOR".

**And, since 1.2.0, uids.** `src/blake3.js` is a hand-rolled BLAKE3-256 — hand-rolled because a
binding to the same C library the Rust calls would test two callers of one implementation — and
`src/uid.js` lays out the unit core, normalises at the encoder, and reproduces all sixteen
uids in `fixtures/wire/uid/cases.json`. Canonical bytes are compared separately from the hash,
so a disagreement says which half broke.

`src/uid.js` also implements §7's shape clause, and `uid()` runs it first: a unit with no gist,
with `derived` or `inferred` and no grounds, with `measured` or `cited` and no source, or with
an authored `unfounded`, cannot get an identity out of this package. The class is about what an
implementation *emits*, so validating on request while deriving anyway would not be it.

Every one of those checks was verified capable of failing before being trusted — status removed
from the hashed core, NFC removed from the encoder, empty sets emitted rather than omitted, the
source map's keys shifted, the sort dropped, the alphabet swapped for base32hex, and the
BLAKE3 tree ignored. Eight breakages, eight distinct failures, each naming the clause it broke.

## Four places the spec did not say enough to derive a uid

Marked `SPEC:` in `src/uid.js`, as the CBOR reader marks its own, and **all four are now in the
document**. They are listed here because the pattern is the point rather than the individual
gaps: every one was settled by decoding `fixtures/wire/uid/cases.json` rather than by reading
`SMYSL_FORMAT_SPEC.md`, which means the fixtures had been carrying normative content the spec
did not admit to having. `python/` and `go/` necessarily reached the same answers — they
reproduce the same bytes — but neither recorded that it had to guess, so the gaps survived two
readings unseen.

1. **§2.2 said the opposite of what the encoder does.** `deps` and `grounds` are listed
   "required, MAY be empty", and an empty one is *omitted*. A literal reading emits a five-key
   map where the reference emits three — a different uid for every unit with neither.
2. **The status integers appeared nowhere.** §2.2 typed `status` as `uint`; §6 and §7 named six
   statuses without mapping one to a number. The order is not arbitrary either: rule M compares
   these as integers.
3. **The `source` map had no key layout**, and `kind` was a second undocumented enum. The
   fixture JSON gives names, so the only statement of the integers was the hex.
4. **The base32 alphabet was unnamed.** This one does not move a uid — the wire carries raw
   bytes — but §2.1 requires a parser to accept 26 to 52 characters, and base32hex was an
   equally faithful reading that would produce mutually unreadable names for the same unit.

## What this still does not reach

C-Consume's rule M. §6 says a `derived` or `inferred` unit must not exceed the status of its
weakest **present** ground, and a unit core carries its grounds as uids — the statuses they
name are not in hand. M is checkable against a store and not against a unit, so a `validate`
claiming to enforce it here would be a check that cannot fail. C-Merge and C-Full are likewise
out of scope: this is a conformance witness, not a second tool.

## One place JavaScript forced a decision the spec does not discuss

Maps decode to `Map`, not to a plain object. §3 constraint 1 distinguishes integer keys in the
kernel from text keys inside a payload, and a JavaScript object turns `0` into `"0"` — losing
exactly the distinction the constraint draws. The spec says nothing about host-language
representation, and should not, but an implementer meeting this in a language with weak key
types has to work it out. Worth knowing that it comes up.
