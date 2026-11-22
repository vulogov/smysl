# Examples

Runnable programs over the `smysl` facade crate. All run offline and print the same
identities on every run and every machine.

| example | run | shows | needs |
|---|---|---|---|
| [`simple_demo`](simple_demo/) | `cargo run --example simple_demo` | the format and its core operations: parse, identity, CBOR, store and `check`, sq/1, salience, pack, render, withdraw, `fmt` | nothing beyond the facade; builds under `--no-default-features` |
| [`information_theory`](information_theory/) | `cargo run --example information_theory` | every layer of SMYSL-2 over one library of texts: readers and identities, pseudonyms, drawn units, consensus text ingest, same-as, time, queries, saved queries, dossiers, measures, export and import, redaction, `check` | the default features (`cli`, `ingest`); add `--features store-redb` for the unit store |
| [`oko_ledger`](oko_ledger/) | `make oko-ledger` | RFC SMYSL-3 §24's OKO ledger profile: one rotation's investigations, views, probes and actions with redacted output, hypotheses under rule TQ, Zabbix trends through `zabbix/1`, timed per append | `--features tel` |
| [`tel_info`](tel_info/) | `cargo run --features tel --example tel_info` | RFC SMYSL-5's information measures over GE-M3's four series: `jsd/1` of each series' second half against its first, `perm/1`, an `mi/1` lag scan with surrogates, then `tel-evidence` recomputing every unit, and a late sample making them `SMY-E604` | `--features tel` |
| [`ge_m8`](ge_m8/) | `make ge-m8` | RFC SMYSL-3 §26's GE-M8: 120 simulated episodes of twelve fault types, ranked by MinHash over typed features, by exact Jaccard, and by Model2Vec embeddings of the same units (`SMYSL_EMBED_MODEL`); recall@5 decides whether similarity is model-dependent | `--features tel,semantic` |
| [`ge_m9`](ge_m9/) | `make ge-m9` | RFC SMYSL-3 §14's GE-M9: six months of simulated samples and windows, evicted under rule TL, exported with `--with-substrate`, imported, and merged with a peer that kept them; a P1 failure or a changed unit blocks TL-P8 | `--features tel` |

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
