# A first look at smysl

```sh
cargo run --example simple_demo
```

One short document in smysl's surface syntax: evidence, claims and a finding about why a CI build
got slow, how they relate, and a brief thread that tells it. The program takes the document
through the basics in ten steps:

| step | what it shows |
|---|---|
| 1 | parsing surface text into records and labels |
| 2 | identity is content: the same claim built in code has the parsed claim's uid, one word changes it, and a status must be earned (`inferred` needs grounds) |
| 3 | CBOR, the wire form, round-trips exactly |
| 4 | a store and `check` (clean), then a broken document's diagnostics: an unresolved reference (`E060`), grounds missing (`E031`), a source missing (`E032`) |
| 5 | sq/1 queries: a filter, a walk down a finding's grounds, what rebuts a claim, lexical search |
| 6 | salience: personalised PageRank seeded at the finding |
| 7 | packing into a token budget, at two budgets |
| 8 | rendering the brief thread as Markdown |
| 9 | withdrawing an edge: the rebuttal is no longer live |
| 10 | `fmt`: canonical surface text that parses back to the same records |

The program is pure. It needs no network, no clock and no files, and it uses no features beyond
the defaults. To see the whole of SMYSL-2 over a library of texts, run
[`information_theory`](../information_theory/).
