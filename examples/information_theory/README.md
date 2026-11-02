# Information theory: SMYSL-2 end to end

One program that walks a small library through every layer of SMYSL-2. The library holds four
texts about information theory, all written for this example:

- notes in Markdown;
- one paragraph added twice, as a lecture handout and as an anthology reprint;
- a study group's Telegram chat;
- a draft that is later redacted.

```sh
cargo run --example information_theory
cargo run --example information_theory --features store-redb   # adds §14
```

It runs offline. The one model call is answered by a scripted provider (`provider.rs`), and every
identity it prints is the same on every run and every machine. The library is left in the
system's temporary directory, and the last line names it, so the CLI can be pointed at it
afterwards. For example, `smysl text ls <path>` or `smysl q 'from texts find "entropy"' <path>`.

| § | what it shows | RFC |
|---|---|---|
| 1 | readers (`md/1`, `txt/1`), manifests, parts, readings; tid, mid, rdid and the structure hash; one part under two manifests | 2.3 A-3, A-5; 2.4 §3.1 |
| 2 | a chat export with its speakers pseudonymised under a key | 2.4 §3.2 |
| 3 | passages by locator; units drawn with provenance and byte spans; an entity and its `x.text/mentions` | 2.4 §3.4, A-2.4 rule 7 |
| 4 | `ingest_text`: windows, two consensus passes, `identical-span` agreement, whether the provider reproduced (OQ-74), `x.text/replies` edges, `SMY-W448` for a reply to a message the export lacks | 2.4 §3.4 |
| 5 | the `identical-span` same-as engine, and `strict` proposition classes | 2.4 TX-P7 |
| 6 | an absolute and a relative (Allen) dating with a basis; effective time under rule E; a `canonical` lock that holds a dating until a resolution reviews it; the lock written as `@commit d3:…` (OQ-73) | 2.3 A-5, A-6, A-12 |
| 7 | sq/1: scopes, `find`, `about entity(…)`, time on both axes, `expand … only` over reply edges, `where` with multi-key `order by`, `intersect` | 2.5, 2.9 |
| 8 | a saved query, rerun after the data grew: the data effect is exactly the new unit | 2.5 §3.9 |
| 9 | a dossier for "Who is Shannon?" | 2.5 TX-P8 |
| 10 | measures: T1 entropy, T2 compression, T4 Zipf, T5 n-gram surprisal, T7 divergence, A2 redundancy, A4 status entropy, A9 entity communities | 2.6 |
| 11 | export by expression with text, `verify` (P1, P3), import into a second library | 2.7 |
| 12 | redaction: bytes and readings removed, claims kept | 2.3 A-5, rule Z |
| 13 | `check` over everything, which comes back clean | 2.1 |
| 14 | the redb unit store over a log (`--features store-redb`) | 2.8 |

**Not shown, and why:**
- **Versification alignment and the `pivot:` language policy** need a second numbering scheme or
  a second language.
- **Semantic retrieval** needs an embedding model file.
- **Live providers** need network and keys.

Each has its own tests: `crates/smysl-text/tests/align.rs`, `tests/cmd_ingest_pivot.rs`,
`crates/smysl-query/tests/semantic.rs` and `crates/smysl-ingest/tests/providers_live.rs`.

Three things the example does on purpose, because a first program over SMYSL-2 tends to get them
wrong:

- **It declares `x.text/v1` and `x.query/v1`** before using their edges. Otherwise a store
  reports `SMY-W013` for each undeclared relation kind.
- **It gives `ingest_text` a clock** (`IngestOptions::with_now`). Rule E bounds a unit's said
  time by when the store first saw it, so an ingest stamped before the chat took place makes
  every message's time contradict its own attestation (`SMY-W413`).
- **It gives each dating a basis.** A dating with nothing grounding it is not applied
  (`SMY-W412`).
