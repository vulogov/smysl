# smysl-conformance — a second implementation, in Python

Written from [`../Documentation/SMYSL_FORMAT_SPEC.md`](../Documentation/SMYSL_FORMAT_SPEC.md)
alone, not from the Rust. That is the whole point: the format's proposition is that two
implementations agree on what a document says, and until this existed that had never been
tested by anything except the implementation that defined it.

**Conformance target: C-Read and C-Produce.**

C-Read is the floor the spec names: decode, re-encode byte-identically, preserve what is not
understood.

C-Produce was added in 0.10.0, and it is the one that matters. C-Read never reaches §2.1,
because reading a document does not require deriving a uid — so three independent readers could
round-trip every fixture byte for byte while remaining ignorant of what a uid *is*, and §2.3,
*status is part of identity*, stayed verified by the Rust alone across nine releases. This
package derives uids now and reproduces the reference implementation's, canonical bytes
included.

No dependencies, including the hash. BLAKE3 is hand-rolled in `smysl/blake3.py` — a binding to
the same C library the Rust uses would have tested two callers of one implementation rather
than two implementations. It is slow, and that does not matter: it hashes unit cores, which are
kilobytes.

```sh
cd python
pip install -e '.[test]'
python -m pytest
```

## What it verifies

Every `.cbor` fixture in `tests/fixtures/` was produced by the Rust implementation and is
decoded here, re-encoded, and compared **byte for byte** — both whole-store and record by
record. Byte-identity is the only assertion worth making: two implementations that both
"parse fine" while disagreeing about bytes disagree about *identity*, because a uid is a hash
of the encoding.

Each constraint of §3 is also asserted as a rejection: non-shortest integers, indefinite
lengths, `null`, binary64 floats, out-of-order and duplicate map keys, tags, floats off the
1/1024 grid, nesting past 128, and non-NFC text.

## What writing it found

Three places where the spec was insufficient. **All three are folded into the spec as of
0.9.0** — constraints 1, 2 and 8 of §3 now say what they used to leave to the reader. The
comments below stay where the holes were, because what a specification failed to say is worth
keeping once it says it:

1. **Constraint 2 (shortest form) does not say what it applies to.** Read literally it covers
   every head, including major type 7 — where the trailing bytes are a float's payload, so
   `0x3F800000` is 1.0 and not an over-long encoding of 1 065 353 216. Implementing it
   literally rejected every fixture. The spec needs a sentence saying the rule is about
   integers and lengths.

2. **Major type 6 (tags) is not mentioned at all.** §3 constrains what may appear without
   saying whether an unrecognised tag is an error. This implementation rejects one, reasoning
   that constraint 1 makes the kernel's shape exhaustive — but that is a reading, not
   something the spec states.

3. **Constraint 1 says what is permitted, not what a decoder must do.** "Integer map keys in
   the kernel. Text keys are permitted only inside a payload" describes a valid encoder. It
   does not say whether a decoder meeting a text key at kernel level must reject it
   (`SMY-E080`) or accept it. This implementation accepts, reading the clause as descriptive
   — a guess, and marked as one.

None is a defect in the Rust. All three are places a second implementer has to guess, which is
exactly what this exercise was for.

## Two files, two questions

`test_conformance.py` asks whether this implementation agrees with the Rust: fixtures in,
byte-identical bytes out. `test_spec.py` asks whether it does what the *document* says,
section by section, with each test naming its clause. They are not the same question — a
library can agree with a reference implementation and still not obey the specification, and
only one of those had ever been tested before this package existed.

`test_spec.py` ends with `test_what_c_read_cannot_check`, which lists the clauses this
conformance class provably cannot reach and fails if that list is ever silently emptied. The
most consequential entry is §2.3, *status is part of identity* — the paragraph the whole
format rests on, and one nothing here can test, because uids need C-Produce.
