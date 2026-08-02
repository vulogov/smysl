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
does not say it applies to integers and lengths rather than float payloads, and major type 6
(tags) is not mentioned at all. Two readers, one document, the same two holes.

**Conformance target: C-Read** — decode, re-encode byte-identically, preserve what is not
understood. No dependencies; `node --test` is built in. A dependency doing part of the work
would weaken the evidence.

```sh
cd nodejs
npm test          # 38 tests, no install step, Node 18+
```

## What it verifies

Every `.cbor` fixture in `../fixtures/wire/` was produced by the Rust and is decoded here,
re-encoded, and compared byte for byte — whole-store and record by record. Byte-identity is
the only assertion worth making: two implementations that both "parse fine" while disagreeing
about bytes disagree about *identity*, because a uid is a hash of the encoding.

Then the spec is walked clause by clause: each of §3's eight constraints as a rejection, the
§2.2 key table and §3.1 type-code table asserted against the document, unknown keys and
unknown record types surviving under rule X, and a store as bare concatenation with a
truncated tail rejected rather than ignored.

## One place JavaScript forced a decision the spec does not discuss

Maps decode to `Map`, not to a plain object. §3 constraint 1 distinguishes integer keys in the
kernel from text keys inside a payload, and a JavaScript object turns `0` into `"0"` — losing
exactly the distinction the constraint draws. The spec says nothing about host-language
representation, and should not, but an implementer meeting this in a language with weak key
types has to work it out. Worth knowing that it comes up.
