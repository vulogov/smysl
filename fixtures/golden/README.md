# Golden artifacts

Byte-for-byte expected outputs. A diff here is a determinism failure (rule D) or a
deliberate format change — never noise, because nothing in this tree is produced by a
model.

| Directory | Contents | Lands |
|---|---|---|
| `cbor/` | canonical encodings of the corpus | SM-P1 |
| `surface/` | canonicalised surface text, for `fmt --check` | SM-P2 |
| `render/` | rendered markdown and Typst artifacts per profile | SM-P12 |
| `explain/` | `pack --explain` and `salience --explain` output | SM-P9, SM-P8 |

Regenerating a golden file is a reviewable act: the diff is the evidence that a change to
packing, salience, threading, or rendering did what it claimed.
