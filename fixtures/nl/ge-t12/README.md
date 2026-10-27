# GE-T12's question set

SMYSL-2.5 §5.6's **GE-T12** asks whether the natural-language front end (`q --from-question`)
writes the query a person would have: a hundred questions with expert-written queries, scored by
**equivalent results** — the same set of units, or the same number for a `count` — and not by
equal text. Below 70% the front end stays experimental.

`questions.tsv` is that set: `id`, `store`, `question`, `gold`, tab-separated, the gold query's
stages separated by ` | `. Twenty questions over each of `fixtures/corpus/F1`–`F5`, covering
kinds, statuses, term filters, negation, traversals in both directions, closures, payload fields
and counts. `tests/ge_t12.rs` checks on every `make test` that each gold query passes the same
check a translation must and answers with something; the live measurement is `--ignored`.

**One hand wrote the questions and the gold queries together, before any model saw them.** That
is the set's limit: the difficulty is the difficulty one person chose, as with GE-T11's planted
contradictions, and this repository has one author. Questions that say "include the …" say so
because an `expand` adds to the set it was given, and a question silent on it would mark a model
wrong for a reading of English rather than of the store.

`SMYSL_GE_T12_SET=<file>` replaces the set, in this format, without a code change.
