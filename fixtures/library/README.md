# Library fixtures

The conformance artifacts for records 14–19 and the four library identities
(SMYSL-2.3 A-3, A-5). Separate from `fixtures/wire/` because the identities here are
not uids: a tid is a hash of bytes and a mid and an rdid are hashes of record bodies,
each domain-separated by the record code it names, and none of that is reachable by
reading a document.

That is the gap this tree exists to close. A C-Read implementation can round-trip every
record here byte for byte without ever deriving an identity, so `wire/ids.json` carries
the **preimage, the body bytes and the identity** for each record. An implementation that
disagrees can tell whether it encoded differently or hashed differently.

| Tree | Contents | Status |
|---|---|---|
| `wire/` | records 14, 15, 18 (`records.cbor`), their identities and bodies (`ids.json`), and the surface document the manifest is parsed from (`manifest.smy`) | TX-P1 |
| `edtf/` | level-0/level-1 cases, rejects with byte offsets, intervals | TX-P3 |
| `readers/<id>/` | input file, params, expected manifest and mids per reader | TX-P1–TX-P2 |
| `check/` | `.smy` plus the exact diagnostic set the library pass must produce | TX-P1 |
| `time/` | stores and their expected effective intervals and contentions | TX-P3 |
| `redaction/` | peer stores and the expected merged record set | TX-P2 |
| `redteam/` | untrusted texts and scripted answers | TX-P5 |

Only `wire/` exists so far; the rest land with the phases named. The layout is
SMYSL-2.4 §5.4's.

## Regenerating

`wire/` is produced and checked by `crates/smysl-core/tests/gen_library_fixtures.rs`,
which is a plain test rather than an `#[ignore]`d generator: it rebuilds the files from
source and compares, so a change to the encoder fails CI instead of leaving the Python,
JavaScript and Go suites checked against bytes this build no longer produces.

```
SMYSL_BLESS=1 cargo test -p smysl-core --test gen_library_fixtures
```

rewrites them. That should be a decision — it changes what those implementations are
checked against.

## Expected-diagnostic format

As `fixtures/README.md` describes it: each `NAME.<ext>` has a sibling `NAME.expected`
listing one diagnostic code per line, in no particular order, and the set is exact.
