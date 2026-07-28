#import "design.typ": *

#part(number: "I", title: "Foundations")

#chapter(number: 1, title: "The Mental Model")

Every later chapter in this book points a command at some data and reads back
a report, a transformed store, or an artifact. That sentence is almost the
whole tool. What takes a chapter to explain is not the mechanics of any one
command — it's the small set of ideas that make all seventeen of them feel
like the same tool rather than seventeen different tools that happen to share
a binary. Get these ideas straight now and every subsequent chapter is a
variation on a theme instead of a fresh pile of trivia.

#section("The store")

#callout(label: "Why")[
  Every command needs to know two things before it can do anything: what data
  it's operating on, and where the result goes. `smysl` answers both with the
  same small vocabulary everywhere, so once you've read one command's
  `--help` you've effectively read the shape of all of them.
]

#term("Store")[
  A *store* is whatever `smysl` reads its records from: a `.smy` surface
  file, a CBOR log (the binary form `merge -o file` and `pack` write by
  default), or `-` for stdin. Almost every command accepts one, either as a
  trailing positional argument or via the global `-s`/`--store` flag — the
  two are interchangeable. A store is not a database or a service; it is
  just bytes, in one of two encodings, that decode to the same set of
  records either way.
]

Concretely, a store is a sequence of *records*. `smysl check` reports the
count of both when it looks at one:

#screen(caption: "$ smysl check fixtures/corpus/F1-incident.smy")[
```
fixtures/corpus/F1-incident.smy: 13 records, 8 units, 0 diagnostic(s)
```
]

13 records reduce to 8 units because a store holds more than one record
shape. The rest of this section is about what those shapes are, and which
ones you'll ever type yourself.

#section("What you author, and what the tool derives")

#callout(label: "Why")[
  The previous shallow pass at this manual left readers guessing which parts
  of a `.smy` file are theirs to write and which parts show up on their own
  after running a command. That confusion is exactly what turns "staged" or
  "attestation" into a mystery word instead of an expected next step. Sorting
  records into *authored* and *derived* up front removes that guess before
  it can form.
]

A record is one of eight kinds (`smysl-core`'s `Record` enum, verbatim):
`Unit`, `Attestation`, `Relation`, `Thread`, `View`, `Contention`,
`PackInfo`, `SchemaDecl`. You will hand-author three of these regularly, one
occasionally, and never touch the rest directly:

#dtable(
  (auto, auto, 1fr),
  (
    ([Record], [Who writes it], [What it is]),
    ([Unit], [You], [A single claim, piece of evidence, finding, definition, hypothesis, prose beat, question, observation, data table, or artifact reference — the atoms of the document. Full grammar is Chapter 4 and the format guide.]),
    ([Relation], [You], [A typed edge between two units — `--causes-->`, `--rebuts-->`, `--warrant-->`, and the rest — the connective tissue that turns a pile of units into an argument.]),
    ([Thread / the `@doc` header], [You (or `thread --derive`)], [The document's own identity, roots, and granularity — and, separately, an ordered narration over existing units. You can write a thread by hand or have `smysl thread` derive one; Chapter 18 covers both paths.]),
    ([View], [Occasionally, via `smysl view`], [A named, reusable selection of roots — a saved answer to "which units does this document actually need."]),
    ([Attestation], [The tool], [The record an operation like `attest` or `ingest` attaches to a unit to say who backed it, at what trust rung, and when — never something you type.]),
    ([Contention], [The tool, during `merge`], [A materialised disagreement between two stores that both spoke about the same unit — the thing rule C promises never gets silently dropped.]),
    ([PackInfo], [The tool, during `pack`], [A self-describing receipt: how much of the budget was used, what got dropped or degraded, and under which estimator.]),
    ([SchemaDecl], [The tool, rarely you], [A declaration of a non-kernel schema extension a store depends on.]),
  ),
)

The pattern above is the one to keep: *you write intent, the tool writes
provenance.* Nowhere in this book will you type an `@attestation` or a
`@contention` block by hand — if you ever see one in a file, a command put
it there, and that command's chapter explains why.

#whatsnext[
  Chapter 4 gives you the full authoring grammar for units and relations —
  every field, every status, worked examples of each unit schema. Chapter 3
  gets you there faster with one small worked file, if you'd rather see the
  shape before the reference.
]

#section("Purity: what can and can't reach off your machine")

#callout(label: "Why")[
  This is the single fact in this chapter with the most operational
  consequence. If you don't know which commands are deterministic, you don't
  know which ones are safe to run in a tight loop, wire into CI, run
  offline, or re-run without a second thought about cost or drift — and
  which three need the trust and staging machinery the rest of this book
  spends four chapters on.
]

#term("Purity")[
  Every command is classified as one of three things. *Pure* means the
  command is a bit-reproducible function of its inputs: same bytes in, same
  bytes out, on any machine, forever — no clock, no randomness, no network.
  *Mixed* means the command is pure except for one opt-in mode. *Model*
  means the command's entire job is to call out to a language model, and its
  output is therefore neither reproducible nor free.
]

Exactly three commands ever touch a model: `ingest`, `attest`, and
`thread --refine`. Everything else in this book — canonicalising a file,
validating it, packing it to a budget, merging stores, walking provenance,
rendering to Markdown — is pure. The command table itself carries this
classification, and each subcommand's own `--help` repeats it under
`Purity:`:

#dtable(
  (auto, auto),
  (
    ([Command], [Purity]),
    ([`fmt`], [pure]),
    ([`check`], [pure]),
    ([`pack`], [pure]),
    ([`merge`], [pure]),
    ([`diff`], [pure]),
    ([`trace`], [pure]),
    ([`view`], [pure]),
    ([`bundle`], [pure]),
    ([`thread`], [mixed — pure except `--refine`, which calls a model]),
    ([`salience`], [pure]),
    ([`retract`], [pure]),
    ([`render`], [pure]),
    ([`ingest`], [model-dependent]),
    ([`attest`], [model-dependent]),
    ([`providers`], [pure — it reports what *would* egress, without egressing]),
    ([`usage`], [pure]),
    ([`reindex`], [pure]),
  ),
)

This is not a marketing claim; it is a tested one. The project's CI runs
`cargo xtask determinism`, which builds permutations of the same inputs
(reordered records, rebuilt indices, repeated runs) specifically to catch any
pure command that has quietly stopped being a function of its bytes.

Operationally, the split means: run the fourteen pure commands as often as
you like, in a pre-commit hook, in a CI matrix, on a laptop with no network
— the answer is always the same answer, and there is no bill for asking
twice. The three model commands are different in kind, not degree: their
output depends on which model answered, may differ between runs, costs
tokens, and can send your document's content somewhere else. That is exactly
why `ingest` and `attest` never write directly into your store — Chapter 8
and Chapter 9 cover the staging and confirmation apparatus (exit code `10`,
`--yes`, `merge --staged`) that exists specifically to put a human decision
between a model's answer and your document.

#whatsnext[
  You don't need the model commands to get real work done — most of a
  document's life is pure operations. Chapter 7 explains the trust model
  those three commands sit behind before Chapters 8–10 cover each in depth.
]

#section("Five phases, not an alphabet")

#callout(label: "Why")[
  Seventeen commands sorted alphabetically teach you nothing about order of
  use — `attest` would sit next to `bundle` next to `check`, three commands
  from three different moments in a document's life. This book is organised
  by what you are actually doing at each point, because that is the question
  you have when you reach for a command: not "what does `X` do" but "I have
  a document in this state, what do I do next."
]

A `.smy` document moves through five phases, and this book's Parts II
through VII are exactly those five phases in order:

- *Create* — write or generate the units and relations that are the
  document's raw material (Part II: Chapters 4–5).
- *Infer and enrich* — optionally hand parts of that raw material to a model,
  under trust rungs and staging (Part IV: Chapters 7–10).
- *Operate* — combine, select, retract, thread, and query the resulting
  graph (Part V: Chapters 11–18).
- *Verify* — confirm the store is internally consistent, conformant, and
  reproducible before anyone downstream relies on it (Part VI: Chapters
  19–21).
- *Export* — turn the graph into something a person reads (Part VII:
  Chapters 22–24).

Validation — Chapter 6's `check` — sits deliberately in its own short part
right after creation, because in practice it isn't a phase you visit once;
it's a loop you run after almost every edit, the way a compiler error is
something you react to immediately rather than batch up. The five-phase
structure describes the large-scale journey of a document; `check` is the
thing you do continuously along the way.

#recap((
  [A *store* is a `.smy` file, a CBOR log, or stdin — the same small
   vocabulary (`-s`/`--store`, a trailing path, `-`) works everywhere.],
  [Eight record kinds exist; you author units, relations, and threads/views
   by hand, the tool writes attestations, contentions, and pack info for
   you.],
  [Only `ingest`, `attest`, and `thread --refine` are model-dependent —
   every other command is a deterministic, CI-verified function of its
   inputs, safe to run as often as you like.],
  [The book follows five phases — create, infer/enrich, operate, verify,
   export — because that mirrors the order you actually use the tool in,
   not the alphabet.],
))

#chapter(number: 2, title: "Installing, Building, and Global Flags")

#section("Building the binary")

#callout(label: "Why")[
  `smysl` ships as one crate with a library and a CLI binary layered on top
  of it (Chapter 25 covers the library side in full). Before anything else
  in this book works you need that binary, and the way you build it already
  makes a decision about which parts of the tool you're getting.
]

From the repository root:

#screen(caption: "$ cargo build")[
```
   Compiling smysl v0.1.0 (/Users/gandalf/Src/smysl)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.37s
```
]

That produces `target/debug/smysl`, which every example in this book invokes
directly. A release build (`cargo build --release`) is the same binary,
optimised; nothing about its behaviour changes, only its speed and size.

#subsection("The feature matrix")

`smysl`'s `Cargo.toml` defines a `[[bin]]` that `required-features =
["cli"]` — the binary does not exist at all unless the `cli` feature is on.
This is deliberate: the crate's *primary* product is the library
(`src/lib.rs`), and the CLI is its first, but not only, consumer. The
feature flags decide how much of the tool you're compiling:

#dtable(
  (auto, 1fr),
  (
    ([Feature], [What it buys you]),
    ([`cli`], [The `smysl` binary itself and its argument parser (`clap`). Nothing below reaches you without this — it's why `--no-default-features` alone doesn't produce a binary.]),
    ([`tui`], [The `ui` subcommand: a terminal UI (`smysl-tui`, `ratatui`, `crossterm`) on top of the CLI.]),
    ([`providers`], [The provider registry and usage ledger (`smysl-provider`) with no concrete backend wired in yet — the plumbing `local`/`remote` build on.]),
    ([`ingest`], [`smysl-ingest`, plus `providers` — the library-level machinery behind `ingest` and `attest`, still with no model backend.]),
    ([`local`], [`ingest` wired to Ollama — a model running on your own machine, the only egress-free way to use `ingest`/`attest`.]),
    ([`remote`], [`ingest` wired to Anthropic, OpenAI, Gemini, and DeepSeek — every backend that leaves the machine, which is exactly what `--offline` and `providers --tasks` (Chapters 7–10) exist to police.]),
    ([`render-typst`], [The Typst backend for `smysl render`, on top of the always-available Markdown/HTML/text/JSON targets — see Chapter 23.]),
    ([`render-html`], [The HTML render backend.]),
    ([`exact-pack`], [Branch-and-bound search in `smysl pack --mode exact`, which proves optimality instead of only approximating it greedily (Chapter 17).]),
    ([`tls-pure`], [A Rust-only TLS stack for the provider crate, if you need to avoid linking a system TLS library.]),
  ),
)

`default = ["cli", "tui", "local", "render-typst"]` is what plain
`cargo build` gives you: a full interactive tool that can run a local model,
render Typst, and show a TUI, but cannot reach a remote model unless you also
turn on `remote`.

#callout(label: "Why choose otherwise")[
  If you're embedding `smysl` as a library rather than shelling out to the
  binary (Chapter 25's whole subject), you almost certainly want none of the
  above: no `clap`, no TUI, no HTTP client, no async runtime pulled into your
  dependency tree. `--no-default-features` is how you ask for exactly that —
  the crate becomes a synchronous library with the same guarantees the
  facade documents: no panics on untrusted input, no global state, no hidden
  I/O.
]

Verify what actually happens, because it's easy to assume a flag like this
either silently no-ops or breaks the build — it does neither:

#screen(caption: "$ cargo build --no-default-features")[
```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.01s
```
]

That succeeds — quietly — because `cargo build` without `--bin` only builds
targets that are buildable with the enabled features, and the `[[bin]]`
requires `cli`. Ask for the binary explicitly and the real refusal appears:

#screen(caption: "$ cargo build --no-default-features --bin smysl")[
```
error: target `smysl` in package `smysl` requires the features: `cli`
Consider enabling them by passing, e.g., `--features="cli"`
```
]

In other words: `--no-default-features` builds the *library* only, silently
skipping the binary target rather than erroring — cargo simply narrows what
it builds to what the enabled features permit. That's the mechanism Chapter
25 relies on when it walks through embedding the crate directly.

#whatsnext[
  If you only ever plan to use the `smysl` binary interactively, the default
  build is all you need — skip ahead to Chapter 3. If you're evaluating
  `smysl` as a dependency inside your own Rust program, keep this feature
  table in mind; Chapter 25 returns to it with a working embedding.
]

#section("Global flags")

#callout(label: "Why")[
  Twelve flags are declared once, globally, and accepted by every
  subcommand — so instead of relearning them per command, learn them once
  here. Several don't matter yet and won't until a later chapter gives them
  a job; this table tells you which chapter that is so a flag you skip past
  today isn't a mystery when it resurfaces.
]

#dtable(
  (auto, 1fr, auto),
  (
    ([Flag], [Why it exists], [Matters most in]),
    ([`-s, --store PATH`], [Names the input store; `-` reads stdin. Interchangeable with a trailing positional on most subcommands.], [Every chapter]),
    ([`-o, --output PATH`], [Where to write the result; defaults to stdout so commands compose in a pipeline.], [Ch. 11–18, 22]),
    ([`--format surface|cbor`], [Chooses the *shape* of output — human-readable surface text or the binary CBOR log. Defaults to `cbor` when stdout isn't a terminal, so a script gets bytes and a person at a keyboard gets text.], [Ch. 4–5]),
    ([`-C, --config FILE`], [Points at a configuration file — provider credentials, defaults — instead of environment variables or flags on every call.], [Ch. 9–10]),
    ([`--strict`], [Promotes warning-severity diagnostics to the failure threshold, so a CI gate can refuse anything `check` merely frowns at today.], [Ch. 6, 19]),
    ([`--offline`], [Hard-fails rather than letting any byte leave the machine — the enforcement half of the trust model, not just a warning.], [Ch. 7–10]),
    ([`--json`], [Machine-readable diagnostics instead of formatted text, for a caller that parses output rather than a person who reads it.], [Ch. 6, 19]),
    ([`-q, --quiet`], [Suppresses non-error output — useful once you trust a pipeline and only want to hear about failures.], [Any scripted use]),
    ([`-v, --verbose`], [Increases verbosity; repeatable (`-vv`, `-vvv`) for progressively more diagnostic detail.], [Debugging any command]),
    ([`--no-color`], [Disables ANSI colour, for logs and terminals that don't want it.], [Scripted use]),
    ([`--noprogress`], [Disables progress bars unconditionally, whatever the terminal thinks it can render.], [Large stores, CI logs]),
    ([`--seed-check`], [Asserts this exact invocation is bit-reproducible — the command-line hook onto rule D, the same determinism `cargo xtask determinism` checks in CI.], [Ch. 21]),
  ),
)

A quick, real illustration of `--format` and stdout detection: `pack` (Chapter
17) writes CBOR by default because its natural home is a pipe into another
command, but ask for `--format surface` and you get the same result as text
you can read in this book.

#section("The exit code contract")

#callout(label: "Why")[
  A pipeline that calls `smysl` needs to branch on *what happened*, not
  scrape stderr for a string. Every exit code below is part of the tool's
  stable contract — guaranteed not to shift meaning across a minor version —
  so a script can test `$?` the same way years from now. This table is the
  authoritative version; Appendix C only restates it.
]

#dtable(
  (auto, auto, 1fr),
  (
    ([Code], [Name], [What it means]),
    ([0], [Success], [Nothing to report.]),
    ([1], [Failure], [A generic failure — usually an I/O or resolution problem that doesn't fit a more specific code below.]),
    ([2], [Usage], [The command line itself was wrong — a missing required argument, an unknown subcommand — before `smysl` even looked at a store.]),
    ([3], [CheckErrors], [`check` (or an operation that checks as part of its job, like `fmt --check`) found error-severity diagnostics, or `--strict` promoted warnings into that threshold.]),
    ([4], [PackInfeasible], [`pack`'s mandatory floor doesn't fit in the requested budget — the budget is too small to hold what rule R says must survive together.]),
    ([5], [Contentions], [`merge --fail-on-contention` found at least one open contention in the merged result.]),
    ([6], [Provider], [A model backend failed — unreachable, unauthorized, rate-limited, or returned something malformed — for any reason other than the offline policy below.]),
    ([7], [Offline], [The specific case of 6 where the failure was `--offline` refusing to let a call leave the machine at all — a policy outcome, not an accident.]),
    ([8], [UnsupportedVersion], [The store declares a format or kernel major version this build doesn't understand.]),
    ([9], [HashVerification], [A stored hash doesn't match its recomputed value — includes a truncated or corrupted uid, and `fmt`'s own round-trip check if canonicalising ever moved an identity.]),
    ([10], [Staged], [Model output (from `ingest`) was written to a staging area rather than merged, and is waiting for `--yes` or `merge --staged` to confirm it.]),
  ),
)

Two of these are worth internalising now because they recur constantly:
exit `3` is the one you'll see the most (Chapter 6 is entirely about
avoiding and interpreting it), and exit `10` is not a failure at all — it's
`ingest`'s way of refusing to finish the job for you. Chapter 9 explains why
that refusal is the point, not a bug.

#whatsnext[
  You now have everything global out of the way. Chapter 3 puts it to work
  on one small, real document from start to finish, before Chapters 4 and
  6 go deep on authoring and validation respectively.
]

#recap((
  [`cargo build` from the repository root produces `target/debug/smysl`;
   every example in this book runs that binary.],
  [The default feature set (`cli`, `tui`, `local`, `render-typst`) is a full
   interactive build; `--no-default-features` builds the library alone and
   silently skips the `[[bin]]`, which requires `cli` explicitly.],
  [Twelve global flags apply to every subcommand — most importantly
   `--store`/`--output`/`--format` for where data comes from and goes, and
   `--strict`/`--offline`/`--json` for how a pipeline enforces policy.],
  [Eleven exit codes (0–10) are a stable contract a script can branch on;
   `3` (check errors) and `10` (staged, awaiting confirmation) are the two
   you'll meet soonest.],
))

#chapter(number: 3, title: "Your First Document, Start to Finish")

This chapter has one job: take you through a document's whole early life —
write it, break it, fix it, canonicalise it, and glimpse the pipeline waiting
beyond it — as one connected walkthrough rather than a list of flags. Nothing
here is invented; every command below was run for real against the file
you're about to write.

#section("Writing a minimal document by hand")

#callout(label: "Why")[
  The format guide documents every field a unit can carry. You don't need
  all of them to get started — you need exactly enough to make one true,
  well-supported claim, which is the smallest unit of work `smysl` is built
  around.
]

Create `first.smy` with one piece of evidence, one claim it supports, and
the relation between them:

```
@doc smysl/0.1 {
  id: v/first
  intent: incident-brief
  lang: en
  roots: [c/root-cause]
}

@evidence e/cpu-spike { status: measured, source: { kind: metric, ref: "host.cpu.pct", captured: 2026-07-20 } }
~ CPU on worker-3 sat above 95 percent for eleven minutes starting at 02:14.

@claim c/root-cause { status: derived }
~ A stuck cron job pinned worker-3's CPU and starved the request queue.

@rel e/cpu-spike --warrant--> c/root-cause
```

Four constructs, and each one answers a different question: `@doc` says
what this file is and where it's rooted; `@evidence` records something
measured, with a source so the claim behind it isn't free-floating;
`@claim` states the conclusion; `@rel` says *how* the evidence backs the
claim (`--warrant-->`, one of several relation kinds Chapter 4 covers in
full).

#section("Running `check`, and reading the report honestly")

#callout(label: "Why")[
  A hand-typed file is a hypothesis about what you meant, not a guarantee.
  `check` is how you find out whether the tool agrees with you before
  anything downstream — a merge, a pack, a render — has to guess.
]

#screen(caption: "$ smysl check first.smy")[
```
first.smy: error: SMY-E060: unresolved reference `c/root-cause` (at 77..89)
first.smy: error: SMY-E060: unresolved reference `c/root-cause` (at 397..439)
first.smy: error: SMY-E031: SMY-E031: derived/inferred with empty grounds (at 284..396)
```
]

That's three errors from one mistake, and reading it in the wrong order
looks like three separate bugs. `status: derived` requires non-empty
`grounds` (rule M: a derived claim must name what it derives *from*) — that
is `SMY-E031`, the third line, and it's the actual root cause. Because that
claim never became a valid unit, both the `@doc`'s `roots: [c/root-cause]`
and the `@rel` line at the bottom point at something that doesn't exist —
hence two `SMY-E060` "unresolved reference" errors from a claim that failed
to admit at all. (The doubled `SMY-E031: SMY-E031:` text is a real quirk of
how the diagnostic wraps the underlying shape error's own message — copied
here verbatim rather than cleaned up, because that's what you'll actually
see.) The lesson generalises: when `check` reports a cluster of dangling
references alongside one shape error, fix the shape error first and see how
many of the others disappear on their own.

Add the missing grounds:

```
@claim c/root-cause { status: derived, grounds: [e/cpu-spike] }
~ A stuck cron job pinned worker-3's CPU and starved the request queue.
```

#screen(caption: "$ smysl check first.smy")[
```
first.smy: 4 records, 2 units, 0 diagnostic(s)
```
]

Clean. Four records — the doc header, the evidence unit, the claim unit,
and the relation — reducing to two units, with nothing `check` objects to.

#whatsnext[
  This file only exercises two unit schemas and one relation kind out of the
  full grammar. Chapter 4 covers every schema, every status, and the rules
  (like the one you just tripped) that govern which combinations are legal.
  Chapter 6 goes back through `check` pass by pass — conformance, fidelity,
  granularity — once you have more than one small file to worry about.
]

#section("Canonicalising with `fmt`, and reading the diff")

#callout(label: "Why")[
  A file you typed by hand almost never matches `smysl`'s canonical
  spelling — field order, quoting, and implicit defaults are all fixed by
  the writer, not by how you happened to type them. `fmt` is the one command
  whose entire job is closing that gap, safely: it re-parses its own output
  and refuses to write anything that wouldn't decode back to the identical
  records.
]

`fmt --check` tells you, without touching the file, whether it's already
canonical:

#screen(caption: "$ smysl fmt --check first.smy")[
```
first.smy: not canonically formatted
```
]

That's not a warning — it exits `3`, the same code `check` uses for a
validation failure, because reformatting *is* a check on the round-trip
property. Run it for real with `--write` and diff the result to see exactly
what canonical form means in practice:

#screen(caption: "$ smysl fmt --write first.smy  (then diff against the original)")[
```
4a5
>   granularity: { profile: default, l0_max: 30, l1_range: [40, 120], admission: single-assertion }
8c9
< @evidence e/cpu-spike { status: measured, source: { kind: metric, ref: "host.cpu.pct", captured: 2026-07-20 } }
---
> @evidence e/cpu-spike { status: measured, source: { kind: metric, ref: host.cpu.pct, captured: 2026-07-20 } }
```
]

Two real changes, both instructive. First, the granularity profile you left
implicit (just `default`, if you'd written it at all) comes back fully
expanded to its four underlying fields — nothing about a document's shape is
left for a later reader to look up elsewhere. Second, the quotes around
`host.cpu.pct` disappear: canonical form only quotes a string when its
content actually requires it, so the writer normalises quoting by content
rather than preserving however you happened to type it. Neither change moved
a uid — hashes are computed over CBOR, never over surface text, which is the
property that makes `fmt` safe to run automatically rather than something
you audit by hand every time.

#term("Canonical form")[
  The one, unique byte-for-byte spelling of a given set of records: field
  order fixed, quoting decided by content, implicit defaults expanded. Two
  files that parse to the same records always canonicalise to the same
  bytes, which is what makes a diff between two canonical files a diff of
  actual content rather than of formatting noise.
]

#callout(label: "No file, no problem")[
  `fmt` with no files reads stdin and writes stdout, so
  `cat draft.smy | smysl fmt | smysl check -` canonicalises and validates in
  one pipeline, with no temporary file in between.
]

#whatsnext[
  Chapter 5 covers `fmt` and canonical form in full — every normalisation
  rule, not just the two you happened to trigger here. Run `fmt --check` as
  a habit after every hand-edit; it costs nothing and it's exactly the kind
  of drift you don't want to notice for the first time in a diff review.
]

#section("One glimpse ahead: `pack --explain`")

#callout(label: "Why")[
  You don't need to understand packing yet — that's Chapter 17's job in
  full. What's worth seeing right now, on your own two-unit file, is that a
  whole pipeline exists past validation: documents don't just get checked
  and formatted, they get *selected down* to fit a budget before anything
  reads them, and the tool will tell you exactly why it kept what it kept.
]

#screen(caption: "$ smysl pack --budget 200 --explain --format surface first.smy")[
```
b3:fh63h4ededeae3dpn6ov4y4cuo @L0  -  earned on density
b3:uxjafqrmtdo2i5nbi2mlma2ylv @L0  -  earned on density
first.smy: 2 of 2 unit(s), 41 of 200 tokens, greedy mode, gap 0.000
@doc smysl/0.1 { id: v/pack, intent: pack }

@evidence e/cpu-spike { status: measured }
~ CPU on worker-3 sat above 95 percent for eleven minutes starting at 02:14.

@claim c/root-cause { status: derived }
~ A stuck cron job pinned worker-3's CPU and starved the request queue.
```
]

With a 200-token budget and only 41 tokens of content, both units fit
easily, each "earned on density" rather than forced in by a hard constraint
— `--explain` is telling you *why* each survivor made the cut, unit by unit,
before the summary line. A two-unit toy file was never going to be
interesting to pack; the point is that this same flag, pointed at a store
with hundreds of units and a budget that can't hold all of them, is how
Chapter 17 explains what gets kept, what gets dropped, and what degrades to
a shorter form instead of disappearing outright.

#whatsnext[
  Two chapters are waiting on what you just did. Chapter 4 is the full
  authoring grammar — every unit schema, every relation kind, every status
  and the rules governing it, so the next file you write doesn't rely on
  guesswork. Chapter 6 is `check` in real depth — conformance classes,
  fidelity against a named consumer, granularity reporting — for the moment
  your documents outgrow two units and one relation.
]

#recap((
  [A minimal document is four constructs: `@doc`, one or more units, the
   relations between them, nothing else required to get started.],
  [`check`'s errors can cascade from one root cause — a shape error
   (`SMY-E031`, empty grounds) produced two knock-on dangling-reference
   errors (`SMY-E060`) because the malformed unit never admitted at all.
   Fix the shape error first.],
  [`fmt --check` exits `3` if a file isn't canonical; `fmt --write` rewrites
   it in place, expanding implicit defaults and normalising quoting without
   ever moving a unit's identity.],
  [`pack --budget N --explain` previews the selection pipeline every store
   eventually passes through — full depth is Chapter 17, but the shape is
   visible on a two-unit file already.],
))
