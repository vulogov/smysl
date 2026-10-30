# `fixtures/library/check`

The `Library` check pass (pass 12, RFC SMYSL-2.4 §4.3.3), in the `.smy` + `.expected`
format the other two check trees use: one document per defect, and the expected file
holds exactly the diagnostic codes it must produce — no more and no fewer.

| fixture | expects | what it is |
|---|---|---|
| `clean-control.smy` | *nothing* | a manifest with a part entry and a unit citing that part by its canonical tid. The control: without it, a pass that reported every manifest would pass every other fixture here. |
| `superseded-chain.smy` | *nothing* | the second control, and the sharper one — the same two manifests as `expression-fork`, with the second superseding the first. A `heads` that returned every manifest under an alias would still pass the fork fixture; it fails this one. |
| `malformed-tid.smy` | `SMY-E403` | a unit whose `source.ref` claims a part identity and carries an abbreviation of one. |
| `malformed-origin.smy` | `SMY-E403` | the same defect in a manifest's `origin`, which is the case with no unit to hang a diagnostic on. |
| `expression-fork.smy` | `SMY-W418` | one alias, two manifests, neither superseding the other. |
| `entity-clean.smy` | *nothing* | the third control: an `x.text/entity` whose provenance is exactly what A-2.4 rule 7 allows — an `alias:` reference, a language, and no span, manifest, published date or grounds. |
| `entity-provenance.smy` | `SMY-E403` | two entities breaking rule 7 two ways: one carries a `published` date, the other's source names a part rather than an expression. Both are reported, and each diagnostic names its field — which a code set cannot pin, so `tests/library_check.rs` pins it. |

## What is not here, and why

**`SMY-E452`** — a log holding a record 15 or 18 — has no `.smy` form at all: records 15
and 18 are among the records surface syntax cannot write, deliberately (a megabyte of
someone else's prose inside a quoted string is neither readable nor diffable). It is
tested programmatically in `tests/library_check.rs`, as rule T's `SMY-E033` is for the
same kind of reason.

**`SMY-E446` and `SMY-E401`** need bytes, and a log holds none: text lives in the object
store (OQ-39). They are checked only when `check` is handed a resolver, which a `.smy`
file cannot supply, so they are tested against a real object store in
`crates/smysl-text/tests/object_check.rs` — beside the only implementation of the
resolver trait, and in the one crate whose dependency tree the manual does not print.

**`SMY-E404` and `SMY-W405`** were allocated to this pass and unimplemented until TX-P5
step 1 gave the format `SourceRef.span`. They are raised now, and they are not here for
the reason `SMY-E446` and `SMY-E401` are not: both need the part's bytes, so both need a
resolver, and they are tested in `crates/smysl-text/tests/object_check.rs`. `SKIPPED` is
empty and stays, with its invariant inverted — whatever is in it must be unregistered.
