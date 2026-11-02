# Examples

Two runnable programs over the `smysl` facade crate. Both run offline and print the same
identities on every run and every machine.

| example | run | shows | needs |
|---|---|---|---|
| [`simple_demo`](simple_demo/) | `cargo run --example simple_demo` | the format and its core operations: parse, identity, CBOR, store and `check`, sq/1, salience, pack, render, withdraw, `fmt` | nothing beyond the facade; builds under `--no-default-features` |
| [`information_theory`](information_theory/) | `cargo run --example information_theory` | every layer of SMYSL-2 over one library of texts: readers and identities, pseudonyms, drawn units, consensus text ingest, same-as, time, queries, saved queries, dossiers, measures, export and import, redaction, `check` | the default features (`cli`, `ingest`); add `--features store-redb` for the unit store |

Start with `simple_demo`. It is one file and one short document, and every later idea rests on
it: a unit's identity is its content, a claim carries its grounds and its status, and
everything after parsing is a pure function of the records.

Then `information_theory`. Its fourteen sections follow RFC SMYSL-2 layer by layer, and its
README maps each section to the RFC document that specifies it. That README also names the
three steps a first program over a library tends to miss:

- declare `x.text/v1` and `x.query/v1` before using their edges;
- give `ingest_text` a clock (`IngestOptions::with_now`);
- give a dating a basis.

For the same operations from the command line, see the manual,
[`Documentation/SMYSL_MANUAL.typ`](../Documentation/SMYSL_MANUAL.typ), and `smysl --help`.
