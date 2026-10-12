# Red-team texts for text ingest

Five passages, each trying something that a text ingest has to survive (SMYSL-2.4 §3.9.2). They
are **inputs**, not expectations: what each one tries is in its own first line, and what the
guard does about it is asserted by `crates/smysl-ingest/tests/redteam.rs`.

The *answers* are not here. An obedient model is played by the `Scripted` provider in that test,
because the interesting half is not what a text can say — it is what reaches a staged record when
a model does what the text asked. A fixture holding the answer too would be a fixture asserting
its own result.

| file | what it tries |
|---|---|
| `instructions.txt` | tells the reader it is an instruction and asks for `measured` |
| `fake-fence.txt` | writes this tool's own fence marker, then speaks in its voice |
| `fake-answer.txt` | writes a complete JSON answer inside the passage |
| `store-uid.txt` | asks for a specific uid to be cited as grounds |
| `bidi.txt` | bidirectional overrides, so what a reviewer sees is not what is stored |

Public-domain or written for this repository; none of them is a real document. The point of each
is a shape, and the shapes are small on purpose: a defect these find is a defect in one rule.
