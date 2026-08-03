# Byte strings that are not smysl documents

Twenty-eight of them, each violating one clause of §3 of `SMYSL_FORMAT_SPEC.md`, consumed by
all four implementations: the Rust, `python/`, `nodejs/` and `go/`.

## Why a shared corpus

0.9 established that four implementations agree on **accepting** four documents — in, out,
byte for byte. That is the weaker half of the claim.

Determinism is enforced by refusal. §3 exists so that one value has exactly one encoding, and
every clause in it is a rule about what must be **rejected**: non-shortest integers,
indefinite lengths, unsorted or duplicated map keys, nulls, non-NFC text, unquantised floats,
tags. If one implementation accepted a non-shortest integer that another refused, every suite
would stay green, every valid fixture would still round-trip, and two implementations would
nonetheless disagree about whether a given byte string is a smysl document.

Nothing checked that, because each implementation invented its own invalid inputs — fifteen
cases in Python, sixteen in JavaScript, eight in Go, no two of them the same bytes.

## What it found immediately

The Rust walker accepted seven of these twenty-eight that the other three all rejected:
unsorted and duplicate map keys, non-NFC and invalid UTF-8 text, and floats that were
unquantised, infinite, or NaN.

That was not cosmetic. `Dec::skip_item` is what preserves unknown keys for rule X; its result
is stored verbatim in `Extra`; `unit_core_bytes` writes `Extra` into the bytes that
`hash::uid` hashes. So a non-canonical extension payload reached content-addressed identity —
the same logical unit, with its extension map keyed in two orders, produced two different
uids. The comment above the call site asserted the opposite in as many words.

Fixed in 0.10.0, with the demonstration kept as a regression test.

## Shape

`manifest.json` lists each case with the §3 constraint it violates and why. The constraint
number is what implementations are compared on, not the error message: independent
implementations word their errors differently and should. Constraint `0` means malformed
input rather than a numbered clause.

Regenerate with `python3 scripts/gen-invalid-corpus.py`. These bytes cannot be produced by
the Rust encoder — a correct encoder never emits them — which is why they are authored rather
than captured.

Each suite pairs the corpus with a **control**: canonical counterparts of the same shapes that
must still be accepted. A decoder that rejected everything would otherwise pass the corpus
while meaning nothing.
