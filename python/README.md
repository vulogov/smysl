# smysl-conformance — a second implementation, in Python

Written from [`../Documentation/SMYSL_FORMAT_SPEC.md`](../Documentation/SMYSL_FORMAT_SPEC.md)
alone, not from the Rust. That is the whole point: the format's proposition is that two
implementations agree on what a document says, and until this existed that had never been
tested by anything except the implementation that defined it.

**Conformance target: C-Read** — the floor the spec names. Decode, re-encode byte-identically,
preserve what is not understood. Nothing above it is attempted, because the spec says an
implementation should declare what it does rather than how complete it is.

No dependencies. A dependency that did some of the work would weaken the evidence.

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

Three places where the spec is insufficient, each marked `SPEC:` where it bites:

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
