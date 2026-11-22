#import "design.typ": *

// ═══════════════════════════════════════════════════════════════════════
#part(number: "XII", title: "Programmable queries (SMYSL-4)")
// ═══════════════════════════════════════════════════════════════════════
//
// SMYSL-4 is the Bund2 bridge RFC. 1.18 builds all four of its phases. Every terminal block
// below was produced by the smysl 1.18 binary, the ones marked "built with --features bund"
// against bund2 0.56.0 on macOS, in the sandbox.

#chapter(number: 39, title: "Bund2 scripts over a library")

`sq/1` answers the questions a query language can ask. Some questions need a program: a
knowledge base built from the library's units and queried with rules, a constraint problem
over what the library says, a walk that decides at each step where to go next. Bund2 is a
stack language with a logic engine and CLP(FD), and it reads a smysl library through its
`smysl.*` words. RFC SMYSL-4 (`Documentation/SMYSL-4_RFC/SMYSL-4_BUND2_BRIDGE.md`) lets
smysl run a Bund2 script over a store or a library and read back its answer.

Bund2 is not linked into smysl. It pins smysl at 1.15, so the two meet where every smysl
since 1.0 meets: the format. `smysl bund` writes a fresh copy of the store, as a library, into
a private directory, runs the `bund2` program over it, and reads the value the script left on
its stack. The live library is never handed to the process and is never changed.

#section("What it needs")

`smysl bund` is behind the `bund` feature, which is off by default, and it needs a `bund2`
built with `smysl`:

```
cargo install --path . --features bund
cargo build --release -p bund2-cli --features smysl     # in a Bund2 checkout
```

A default build says so and does nothing else:

#screen(caption: "$ smysl bund -s fixtures/corpus/F1-incident.smy --eval 1")[
```
smysl bund: this build has no Bund2 bridge (build with --features bund)
```
]

`--probe` says which `bund2` would run, and whether this platform has the sandbox:

#screen(caption: "$ smysl bund --probe --bund2 ./bund2   (built with --features bund)")[
```
bund2:   ./bund2
version: 0.56.0
smysl:   yes
sandbox: available
```
]

#section("A script and its answer")

The script reads the store as `:lib var?`. Whatever it leaves on top of the stack is the
answer, printed as indented text, or as JSON with `--json`. This one builds a knowledge base
from F1's units and asks for every claim, its status and its gist:

#screen(caption: "$ smysl bund -s fixtures/corpus/F1-incident.smy --eval ':lib var? \"from library\" smysl.kb [ [ unit U claim S ] [ gist U G ] ] kb.query'   (built with --features bund)")[
```
-
  G: "The eu-west connection pool is saturated."
  S: "inferred"
  U: b3:cvhirtgs2mpvli2ethhyeo32ufud72l42pa3zxfmsq7dgg722ada
-
  G: "The canary rules out a pure configuration cause."
  S: "derived"
  U: b3:phsoomklkmlq3sjvbe6cyuqy5v3sl6srqxpimogpgmoxvg2pit6q
-
  G: "p95 auth latency tripled after the 4.2 rollout."
  S: "derived"
  U: b3:wo4t2c46lq45fnakd6tajlgcacsalprapxhu7g73kfmzwvvqrlfa
```
]

Every uid in an answer is checked against the store before it is printed. A script cannot
answer with a unit the store does not hold.

`--script FILE` reads the script from a file. `--sq QUERY` runs an `sq/1` query first and binds
the uids it answers to `:seed`, so the script starts from what the query found:

#screen(caption: "$ smysl bund -s fixtures/corpus/F1-incident.smy --sq 'from library | where kind = claim' --eval ':seed var? len'   (built with --features bund)")[
```
3
```
]

`-s` takes a store file or a library root, as every other command does. Two runs of one script
over one store print the same bytes, and so do two stores holding the same records in a
different order.

#section("What a script may do")

A script reads. It runs in a separate process whose environment is cleared, under a wall clock
(`--timeout-ms`, 2,000 by default), with its output capped at 1 MiB, and it is checked three
times:

- *The word gate.* Before anything starts, every word the script names is checked against an
  allowlist: the read-only words of Bund2 0.56.0. Words that write a library, call a model, run
  code by a name built at run time, or reach the host (a shell, files, the network, the clock,
  randomness) are refused by name:

#screen(caption: "$ smysl bund -s fixtures/corpus/F1-incident.smy --eval '\"ls\" sh'   (built with --features bund)")[
```
smysl bund: refused `sh`: runs /bin/sh (at byte 5)
```
]

- *The sandbox.* The gate is not the boundary: Bund2 can assemble a word's name from strings at
  run time. The process runs under `sandbox-exec` on macOS and `bwrap` on Linux, where it cannot
  open a socket, start a program, or read or write a file outside its private directory. On a
  platform without either, `smysl bund` refuses unless given `--trusted`.
- *The inference cap.* Every knowledge base the script makes is capped at 1,000,000 inferences
  (`--max-inferences`). A rule that never terminates stops with `inference cap of N reached`.

`--trusted` runs without the sandbox. It is for a script you wrote yourself. No smysl command
that has a model write a script hands that script to `smysl bund`.

The exit status is 0 for an answer, 2 for a script refused before it ran (a word, the size, a
missing sandbox), and 1 for one that failed while running: a Bund2 error, a smysl refusal, the
clock, the cap.

#callout(label: "Not yet")[
  - A clause variable inside a knowledge-base bridge lambda, such as `[ test { T C > } ]`, is
    read by the gate as a word, and the allowlist refuses it.
  - A Windows sandbox.
  - A tab in `smysl ui`.
]
