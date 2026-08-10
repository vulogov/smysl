# smysl — a fourth implementation, in Go

Written from [`../Documentation/SMYSL_FORMAT_SPEC.md`](../Documentation/SMYSL_FORMAT_SPEC.md),
like the Python and JavaScript packages beside it.

**What makes this one different: it is the first written against the *revised* spec.** The
earlier two both had to guess in three places, and those guesses became clauses 1, 2 and 8 of
§3. So this reading is a test of the revision — if the document now says enough, a fresh
reader should not have to invent anything there.

It did not. Constraint 2's scope and the prohibition on tags were both stated plainly enough
to implement without inference, which is the outcome the clarifications were written for.

**Conformance target: C-Read and C-Produce** — decode and re-encode byte-identically,
preserving what is not understood; and derive uids, laying out a unit core canonically and
refusing to emit one that is not well formed.

```sh
cd go
go test ./...
```

## C-Produce, and why it is the half that matters

C-Read never reaches §2.1. Reading a document does not require deriving a uid, so this package
round-tripped every fixture byte for byte for two releases while having no idea what a uid
*was* — and §2.3, *status is part of identity*, stayed witnessed by the Rust and `python/`
alone. Two implementations of the paragraph the whole format rests on.

This is the third. It needs three things reading does not:

- **A hash.** `blake3.go`, written here rather than imported, for the reason `python/` gives:
  a binding to the same C library the reference implementation uses would test two callers of
  one hash rather than two hashes. Checked against the published BLAKE3 vectors — external
  ground truth, unlike every other fixture here — including the lengths that straddle the
  1024-byte chunk boundary, because a single-chunk shortcut is correct on every small input
  and wrong on the first large one.
- **A canonical layout.** `uid.go`, from §2.2 and §3, checked against
  `fixtures/wire/uid/cases.json` — which carries the Rust's canonical bytes as well as its
  uids, so a disagreement says whether the encoding or the hash was wrong.
- **The shape clause.** §7 defines C-Produce as "structural + epistemic + *shape*", and
  `Validate` is the shape half: a gist present, grounds where `derived` or `inferred` demand
  them, a source where `measured` or `cited` demands one, and no authored `unfounded`. `Uid`
  runs it first, so this package cannot hand out an identity for a unit the format says does
  not exist.

**Writing it found a defect in the fixture.** Removing NFC from the encoder failed the property
test and left the fixture comparison green, which should have been impossible — the pair
`unicode-composed` / `unicode-decomposed` exists to catch exactly that. The generator had been
recording each gist *after* `UnitCoreBuilder` normalised it, so both cases carried the same
composed string: one input under two names, and a witness that could not witness. The fixture
now records the gist as authored, and a reader that skips constraint 6 no longer reproduces the
recorded bytes — in Go or in Python, both of which were checked against the repair.

## The one dependency, and why

`golang.org/x/text/unicode/norm`, for the NFC that §3 constraint 6 requires. Python and
JavaScript have Unicode normalisation in their standard libraries; Go does not.

The other two packages take no dependencies at all, on the grounds that a dependency doing
some of the *format's* work would weaken the evidence they exist to provide. A Unicode
normalisation table is not format work — it is the same table the other two get for free — so
this is a difference in what the standard libraries include rather than in what was
implemented here. The hash is format work, which is why it is not a dependency.

## Two things Go forced that the spec does not discuss

**Maps keep their entries in a slice, not a `map`.** §3 constraint 4 makes key order part of
the encoding rather than a presentation detail, and a Go map has no order to re-encode from.
JavaScript needed `Map` rather than an object for a related reason — its keys would have been
stringified. Neither is a defect in the spec, which should not legislate host-language
representation, but both are decisions an implementer has to reach on their own.

**Integer width.** The spec's constraint 2 is about encoded form, and Go's decoder returns
`uint64` for unsigned and `int64` for negative, so a round trip preserves the encoding rather
than the host type. An implementation that decoded everything to `int` would re-encode a large
value differently and fail C-Read for a reason that has nothing to do with the format.
