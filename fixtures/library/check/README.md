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

**`SMY-E404` and `SMY-W405`** are allocated to this pass and not implemented. Both read
`SourceRef.span`, which §4.3.1 gives to TX-P5. They are not in the registry either; the
pass names them in `SKIPPED` with the reason, so a caller can read what is not checked
rather than infer it from silence.
