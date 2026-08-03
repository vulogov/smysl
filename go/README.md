# smysl — a fourth implementation, in Go

Written from [`../Documentation/SMYSL_FORMAT_SPEC.md`](../Documentation/SMYSL_FORMAT_SPEC.md),
like the Python and JavaScript packages beside it.

**What makes this one different: it is the first written against the *revised* spec.** The
earlier two both had to guess in three places, and those guesses became clauses 1, 2 and 8 of
§3. So this reading is a test of the revision — if the document now says enough, a fresh
reader should not have to invent anything there.

It did not. Constraint 2's scope and the prohibition on tags were both stated plainly enough
to implement without inference, which is the outcome the clarifications were written for.

**Conformance target: C-Read** — decode, re-encode byte-identically, preserve what is not
understood.

```sh
cd go
go test ./...
```

## The one dependency, and why

`golang.org/x/text/unicode/norm`, for the NFC check in §3 constraint 6. Python and JavaScript
have Unicode normalisation in their standard libraries; Go does not.

The other two packages take no dependencies at all, on the grounds that a dependency doing
some of the *format's* work would weaken the evidence they exist to provide. A Unicode
normalisation table is not format work — it is the same table the other two get for free — so
this is a difference in what the standard libraries include rather than in what was
implemented here.

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
