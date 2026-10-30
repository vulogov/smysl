# The chat wire fixture

The records a chat `text add` **emits**, for the three C-Read implementations to recompute
`tid`, `rdid` and `mid` from. This is GE-T1's chat half (RFC SMYSL-2.4 §5.6, TX-P2 step 6).

`records.cbor` is a CBOR sequence of three manifests and the part texts and readings they name —
three exports, three readers, one catalog. `ids.json` holds every identity with the figures it
was derived from. Both are written by
`cargo test -p smysl-text --all-features --test gen_chat_wire` under `SMYSL_BLESS=1`, which
changes what the Python, JavaScript and Go suites are checked against and is therefore a
decision rather than a step.

## Why it exists beside `../wire/`

`../wire/` carries a scripture-shaped expression whose records are hand-built in `smysl-core` —
correctly, in a crate that has no reader. What it cannot offer is the records a *reader* produced,
and that is where the identities are derived over the things scripture has none of:

- a segment table with a **speaker**, an **observed** timestamp and the platform's own **ids** in
  it — all three inside the structure hash, and therefore inside the rdid;
- a **`mul`** manifest, because a chat export is not one language;
- a manifest carrying opaque **`raw`** metadata (key 16) inside its mid. A port that dropped an
  opaque key would decode the record perfectly and re-encode a shorter body, and the only thing
  that would notice is the mid.

Two of the three exports carry `raw`; WhatsApp's transcript has no header to keep.

The ports **implement no reader** and are not asked to — GE-T1 says so. They decode these records
and hash them.

## The suites

| implementation | test |
|---|---|
| `python/smysl` | `python/tests/test_chat_wire.py` |
| `nodejs/src` | `nodejs/test/chat_wire.test.js` |
| `go/` | `go/chat_wire_test.go` |

The Rust side is `crates/smysl-text/tests/gen_chat_wire.rs`, which also asserts that the fixture
holds what GE-T1 asks it to hold — three manifests, both objects for every part, at least one
speaker, and exactly two manifests with `raw` — so a future edit cannot quietly reduce it to
something that merely decodes.
