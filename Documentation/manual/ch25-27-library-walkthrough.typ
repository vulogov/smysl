#import "design.typ": *

// ═══════════════════════════════════════════════════════════════════════
#part(number: "VIII", title: "smysl as a Library")
// ═══════════════════════════════════════════════════════════════════════

#chapter(number: 27, title: "The Facade Crate and Feature Flags")

Every workflow so far has gone through the `smysl` binary. This chapter is about the
thing underneath it — because the binary is not where the work happens.

#section("Why the library is the product")

#callout(label: "Why")[
  *The library is the product; the CLI is its first consumer.* This is not a slogan —
  it is a concrete promise: nothing the CLI does is unreachable from the crate, and no
  code path is CLI-only. If you can do it with `smysl pack --budget …`, you can do it from
  Rust, with the same function, the same guarantees, and no process boundary in between.
]

`src/lib.rs` opens by saying exactly that:

```
//! The library is the product; the CLI is its first consumer (principle P8). Rule A
//! holds throughout: no CLI capability may be unreachable from here, and no code path
//! may be CLI-only.
```

The rest of this section is that promise checked, not asserted. Take `pack` — the command
Chapter 18 built a whole workflow around. `src/lib.rs` re-exports it from `smysl-pack`:

```
pub use smysl_pack::{
    pack, verify as verify_pack, Constraints, Estimator, Pack, PackError, PackRequest, Reason,
    Selection, Violation,
};
```

And `cmd_pack` in `src/main.rs` — the function that runs when you type `smysl pack` — calls
exactly that function, on exactly the types the facade exports, with nothing standing
between the parsed command line and the library call:

```
let sal = smysl::salience(&store, &SalienceRequest::default().seeded(smysl::view_roots(&store)));
let outcome = smysl::pack(&store, &sal, &req);
```

There is no private `pack_impl` inside the binary that the public `smysl::pack` merely
wraps. `req` is a `PackRequest` you could have built yourself; `store` is a `Store` you
could have built yourself; `sal` is a `SalienceReport` from a call you could have made
yourself. Chapter 28 does exactly that, end to end, with nothing named `main`.

#term("Facade crate")[
  The `smysl` crate itself: a `[lib]` re-exporting the public surface of every other crate
  in the workspace, plus a `[[bin]]` (`required-features = ["cli"]`) that is a thin shell
  over it. `src/lib.rs`'s re-export list is the entire public API; nothing else in the
  workspace is meant to be depended on directly by an embedder.
]

#section("The feature table")

An embedder does not want a terminal UI, an argument parser, or four HTTP clients linked
into their service just to call `check` and `pack`. The facade's `[features]` table, from
`Cargo.toml`, is how much of the tree you actually pull in — transcribed here exactly, edges
and all:

#dtable(
  (auto, 1fr),
  (
    ([Feature], [Turns on]),
    ([`default`], [`cli`, `local`, `render-typst` — what plain `cargo build` gives you: the binary, local (Ollama) model access, and Typst rendering. *Not* `tui`: `ratatui` and `crossterm` in every default build is a cost an embedder who only calls the library never opted into, so the browser is `--features tui`.]),
    ([`cli`], [`dep:clap`. Without it there is no `[[bin]]` at all — the binary's own manifest entry requires this feature to exist.]),
    ([`tui`], [`dep:smysl-tui`, and `cli` (the TUI is reached through the same binary, so it pulls the parser in too).]),
    ([`providers`], [`dep:smysl-provider` — the provider abstraction: registry, capabilities, ledger. No vendor is wired in by this alone.]),
    ([`ingest`], [`dep:smysl-ingest`, and `providers` (ingest always needs a provider to call, so the dependency is unconditional).]),
    ([`local`], [`ingest`, plus `smysl-provider/ollama` and `smysl-ingest/ollama` — everything `ingest` and `attest` need, wired to a local Ollama server that never leaves the machine.]),
    ([`remote`], [`ingest`, plus `smysl-provider/{anthropic,openai,gemini,deepseek}` and the matching `smysl-ingest/*` features — the four hosted providers, on both the calling side and the ingest-specific side.]),
    ([`render-typst`], [`smysl-render/typst` — the backend behind `render --target typst`.]),
    ([`render-html`], [`smysl-render/html` — the backend behind `render --target html`.]),
    ([`exact-pack`], [`smysl-pack/branch-and-bound` — what `pack --mode exact` needs to prove optimality instead of packing greedily.]),
    ([`tls-pure`], [`smysl-provider/tls-pure` — a pure-Rust TLS stack for provider connections, keeping C out of the network path the way `blake3`'s `pure` feature keeps it out of the hash path (§13).]),
  ),
)

Notice what is *not* in that table: `smysl-core`, `smysl-graph`, `smysl-check`,
`smysl-pack`, `smysl-thread`, and `smysl-render` are plain, non-optional dependencies in
`[dependencies]` — every build gets the kernel, the store, the ten check passes, packing,
thread derivation, and the render IR, whatever features you choose. Only the things with a
real cost — an argument parser, a terminal UI, a model runtime, a specific render backend —
are behind a flag.

#section("A build with nothing turned on")

`default-features = false` is the embedder's starting point, and it is worth not taking on
faith. Building the facade crate's own library target with every feature off:

#screen(caption: "$ cargo tree --no-default-features -p smysl")[
```
smysl v0.13.0
├── smysl-check v0.13.0
│   ├── smysl-core v0.13.0
│   │   ├── blake3 v1.8.5
│   │   │   ├── arrayref v0.3.9
│   │   │   ├── arrayvec v0.7.8
│   │   │   ├── cfg-if v1.0.4
│   │   │   └── constant_time_eq v0.4.2
│   │   │   [build-dependencies]
│   │   │   └── cc v1.4.0
│   │   │       ├── find-msvc-tools v0.1.9
│   │   │       └── shlex v2.0.1
│   │   └── unicode-normalization v0.1.25
│   │       └── tinyvec v1.12.0
│   │           └── tinyvec_macros v0.1.1
│   └── smysl-graph v0.13.0
│       └── smysl-core v0.13.0 (*)
├── smysl-core v0.13.0 (*)
├── smysl-graph v0.13.0 (*)
├── smysl-pack v0.13.0
│   ├── smysl-core v0.13.0 (*)
│   └── smysl-graph v0.13.0 (*)
├── smysl-render v0.13.0
│   ├── smysl-core v0.13.0 (*)
│   ├── smysl-graph v0.13.0 (*)
│   └── smysl-thread v0.13.0
│       ├── smysl-core v0.13.0 (*)
│       └── smysl-graph v0.13.0 (*)
├── smysl-retrieve v0.13.0
│   ├── bm25 v2.3.2
│   │   └── fxhash v0.2.1
│   │       └── byteorder v1.5.0
│   ├── smysl-core v0.13.0 (*)
│   └── smysl-graph v0.13.0 (*)
└── smysl-thread v0.13.0 (*)
[dev-dependencies]
└── serde_json v1.0.151
    ├── itoa v1.0.18
    ├── memchr v2.8.3
    ├── serde_core v1.0.229
    └── zmij v1.0.23
```
]

No `tokio`, no `clap`, no `ureq`, no `crossterm`, no `ratatui` — grep the real output for
any of them and it comes back empty. That is the point of the flag, and `cargo xtask
check-purity` fails the build if any of them ever appears.

What *is* there is worth reading. `blake3` hashes, built with `pure` so there is no C SIMD
backend — the hash path is Rust all the way down. (`cc` appears beneath it under
`[build-dependencies]`: `blake3` declares it unconditionally, and `pure` is what stops it
being used.) `unicode-normalization` NFC-normalises surface text on parse. And `bm25`, with
`fxhash` and `byteorder` behind it, is lexical retrieval — `smysl-retrieve` is a plain
dependency rather than an optional one, because its default engine is pure: BM25 with
`default-features = false` brings in no runtime, which is a claim `check-purity` enforces
rather than asserts.

That is the whole dependency graph of a fully synchronous library: no async runtime, no HTTP
client, no argument parser, anywhere.

#callout(label: "Checked, not just intended")[
  This is not a documentation promise you have to trust — `cargo xtask check-purity` runs
  exactly this kind of dependency-tree audit in CI on every change (rule B). The library's
  shape is a gate, not a description.
]

The same flag makes the test suite itself meaningfully smaller: `cargo test
--no-default-features --lib` builds and runs the crate's own unit tests, including the one
Chapter 28 walks through, with nothing beyond `smysl-core`, `smysl-graph`, `smysl-check`,
`smysl-pack`, `smysl-thread`, and `smysl-render` compiled at all.

#whatsnext[
  You now know *what* is reachable from Rust and *how little* it costs to reach it.
  Chapter 28 reaches it — a real program, compiled and run against exactly this facade,
  with `default-features = false` the whole way through.
]

#exercises((
  [Chapter 27 claims nothing the CLI does is unreachable from the crate. Pick a
   command you have used in this book and find the library function behind it.
   Then say why "the library is the product, the CLI is its first consumer" is
   a testable claim rather than a slogan.],
  [Run `cargo build --no-default-features` and then
   `cargo build --no-default-features --bin smysl`. Explain the difference in
   outcome to somebody who has not read Chapter 4.],
  [You are asked to add `smysl` to a service that must be certified as unable
   to send data to a third party. Which feature flags do you enable, and what
   can you tell the auditor that a runtime flag could not?],
))

#answers((
  [Because if it were false, some behaviour would exist only inside
   `src/main.rs` and could not be reached from a test that imports the crate.
   The claim is checked by the CLI itself being written against the public API
   — every subcommand is a thin argument-parsing shell over a library call.
   The consequence for you is that anything this book demonstrates at the
   command line is something you can do in-process, with the same guarantees
   and no subprocess.],
  [The first builds the library and quietly skips the binary, because the
   binary target declares that it requires the `cli` feature and cargo builds
   only what the enabled features permit. The second asks for the binary by
   name, so cargo can no longer skip it and reports the missing feature. The
   library is the crate's primary product; the binary is an optional target
   layered on top.],
  [Neither `remote` nor anything that pulls it in — the default set is `cli`,
   `tui`, `local`, `render-typst`, and `local` wires ingest to a model running
   on your own machine. For the strongest statement, build with
   `--no-default-features` plus only what you need. What you can tell the
   auditor is that the HTTP client is not in the binary: not disabled, not
   configured off, but absent from the dependency tree, which is a property
   they can verify from `cargo tree` rather than take on trust.],
))

#recap((
  [Rule A is load-bearing, not aspirational: every `cmd_*` function in `src/main.rs` calls
   straight into a facade re-export — `cmd_pack` calls `smysl::pack`, with no private
   implementation hiding behind it. Since 0.13 that is checked rather than asserted:
   `cargo xtask check-purity` fails if anything under `src/` outside `lib.rs` names a
   sibling crate. It found two bypasses the first time it ran, which is the honest reason
   the check exists — `cmd_import` was reaching into `smysl-ingest` for the CSV reader, so
   a consumer holding the facade could not do what `smysl import` does.],
  [`smysl-core`, `smysl-graph`, `smysl-check`, `smysl-pack`, `smysl-thread`, and
   `smysl-render` are always present; `cli`, `tui`, `providers`, `ingest`, `local`,
   `remote`, the two render backends, `exact-pack`, and `tls-pure` are each opt-in.],
  [`cargo build --no-default-features` produces a dependency tree with no `tokio`, `clap`,
   `ureq`, `crossterm`, or `ratatui` in it — verified here by `cargo tree`, and checked on
   every change by `cargo xtask check-purity`.],
))

// ═══════════════════════════════════════════════════════════════════════

#chapter(number: 28, title: "A Minimal Embedding, End to End")

Chapter 27 established that nothing stops you from calling `smysl` from Rust directly.
This chapter does it — twice: once at the smallest possible scale, and once with two units
and a real dependency between them.

#callout(label: "Why")[
  You maintain a service that already produces incident write-ups — a bot that
  watches alerts, or an internal tool that collects postmortems. It has its own
  storage, its own web UI, and no interest in acquiring a command-line tool, a
  TUI, an argument parser, or an async HTTP client just to emit a document
  other systems can check.

  Shelling out to a binary would mean shipping that binary, matching its
  version to your code, parsing its stdout, and turning its exit codes back
  into your own error type. Every one of those is a place for the two to
  drift. Calling the library means the units your service builds are the same
  units `check` validates, constructed through the same constructors, failing
  the same way — and the dependency you add is a synchronous crate with no
  I/O in it at all.
]

#section("The smallest possible round trip")

`src/lib.rs` carries its own proof that the facade alone is enough to build, hash, encode,
and decode a unit. It is a real test in the crate — `a_unit_round_trips_through_the_facade_alone`
— and it passes under `--no-default-features`, which is the whole point: nothing here needs
`cli`, `tui`, or any model feature.

```rust
let core = UnitCoreBuilder::new(
    KernelType::Claim,
    "p95 auth latency tripled",
    Status::Speculative,
)
.build()
.unwrap();
let uid = canonical_uid(&core);
let bytes = to_cbor(&Record::Unit(core.clone()));
let (decoded, n) = from_cbor(&bytes).unwrap();
assert_eq!(n, bytes.len());
assert_eq!(decoded.as_unit(), Some(&core));
assert!(verify(decoded.as_unit().unwrap(), &uid).is_ok());
```

Walking it line by line is worth doing once, because every later chapter's CLI output is
this same sequence with a terminal wrapped around it.

- `UnitCoreBuilder::new(KernelType::Claim, "p95 auth latency tripled", Status::Speculative)`
  starts a builder for a claim at the lowest non-`unfounded` rung — appropriate, since
  nothing grounds it yet.
- `.build()` returns a `Result`, and that is deliberate. Building is where the shape rules
  of `UnitCore::new` are enforced — a missing gist, a `detail` with no `body`, a
  `derived`/`inferred` unit with empty `grounds`, a `measured`/`cited` unit with no
  `source`. Get one of those wrong and `.build()` fails with a `ShapeError` carrying the
  exact `SMY-E0xx` code from Appendix B, before a `UnitCore` value exists at all.
- Once `.build()` succeeds, the value is *already* NFC-normalised and *already*
  shape-valid — `UnitCore::new`'s whole job is to make that true before it returns, the
  same invariant the ingest layer relies on when it says "a core is shape-valid from the
  moment it exists." That is why nothing downstream of `.build()` is fallible in this
  example.
- `canonical_uid(&core)` hashes the already-canonical bytes. It cannot fail, because there
  is no invalid input left to reject by this point — the builder already rejected it.
- `to_cbor(&Record::Unit(core.clone()))` encodes to `Vec<u8>` directly, no `Result`, for
  the same reason: a built `UnitCore` is always encodable.
- `from_cbor(&bytes)` is where fallibility returns, because now the input is *untrusted*
  again — arbitrary bytes, not a value this process just built. It returns the decoded
  `Record` and how many bytes it consumed, which is what lets `from_cbor_seq` walk a
  concatenated log of them without a length prefix.
- `verify(decoded.as_unit().unwrap(), &uid)` recomputes the hash of the decoded core and
  checks it against the uid you started with. This is the exact check behind `fmt`'s
  round-trip guarantee and `SMY-E070`/exit `9` (Appendix C) — here, called directly, with
  no file and no process in between.

#callout(label: "Why building is fallible and encoding is not")[
  This asymmetry is the point of validating on construction (§15.1, guarantee A4): once a
  `UnitCore` exists, every later operation on it — hashing, encoding, comparing — can be
  infallible, because "does this shape make sense" was answered exactly once, at the
  narrowest point, by the builder. Nothing later has to re-check it.
]

#section("A richer example: two units, grounds, and a check")

One unit alone never exercises the interesting rule — rule M, that a claim's status can
never outrank the weakest thing it rests on. This example builds two units with a real
`grounds` relationship and runs the same `check` pipeline `smysl check` runs on the CLI,
entirely in memory:

```rust
use smysl::{
    canonical_uid, check, CheckOptions, KernelType, Record, SourceKind, SourceRef, Status,
    Store, UnitCoreBuilder,
};

let evidence = UnitCoreBuilder::new(
    KernelType::Evidence,
    "pool wait rose from 2ms to 310ms",
    Status::Measured,
)
.source(SourceRef::new(SourceKind::Metric, "pool.wait_ms"))
.build()
.unwrap();
let evidence_uid = canonical_uid(&evidence);

let claim = UnitCoreBuilder::new(
    KernelType::Claim,
    "the eu-west pool is saturated",
    Status::Inferred,
)
.grounds([evidence_uid])
.build()
.unwrap();

let store = Store::from_records(vec![
    Record::Unit(evidence.clone()),
    Record::Unit(claim.clone()),
]);

let report = check(&store, CheckOptions::default());
assert!(report.is_clean());
assert_eq!(store.units().count(), 2);
```

Two builder calls are worth pausing on. `.source(...)` on the evidence is not decoration —
`Status::Measured` requires it; leave it off and `.build()` fails with `SMY-E032`.
`.grounds([evidence_uid])` on the claim is the same story from the other rule:
`Status::Inferred` requires non-empty `grounds`, or `.build()` fails with `SMY-E031`. The
builder is enforcing the same shape rules Appendix B lists, at the point where you would
otherwise author a document by hand and let `smysl check` catch the mistake later — here,
you cannot construct the mistake in the first place.

`Store::from_records` takes the two units with no file, no path, and no parsing — a
`Store` is a value, not a handle to something on disk. `check(&store, CheckOptions::default())`
is the exact function `cmd_check` calls after `load_store` turns a `.smy` file or a CBOR
log into the same kind of `Store` value. The report comes back clean because the claim's
`Inferred` status does not outrank its one ground's `Measured` status — rule M, satisfied,
with the arithmetic entirely under your control and no subprocess anywhere in the call
stack.

#callout(label: "How this was verified")[
  This is not sample code copied from a comment — it is a real `#[test]` that was compiled
  and run against the `smysl` crate with `cargo test --no-default-features --test
  ch26_scratch`, and it passed. `--no-default-features` matters here: it is the same proof
  as Chapter 27's `cargo tree`, done by actually building and running code against the bare
  facade rather than only inspecting its dependency graph.
]

#whatsnext[
  You have now seen every operation this book covers from both sides: the CLI, which
  parses a command line and prints a report, and the library underneath it, which takes
  and returns ordinary Rust values. Chapter 29 goes back to the CLI side and chains
  everything — authoring, checking, merging, retracting, packing, threading, and rendering
  — into one continuous piece of work, the way a person actually does it.
]

#exercises((
  [The round-trip test in `src/lib.rs` passes under `--no-default-features`.
   Build the crate that way yourself and confirm no binary appears. Then list
   what your dependency tree does *not* contain in that configuration, and say
   which absence you would care about most when adding this to an existing
   service.],
  [The richer example builds two units with a real `grounds` dependency between
   them. Try building the dependent unit *first*, before the unit it grounds
   on exists. What does the API let you do, and at what point does the mistake
   surface?],
  [Take the two-unit example and change the dependent unit's status to
   something its grounds cannot support. Where does that fail — at
   construction, at encoding, or at `check`? Explain why that is the right
   place.],
))

#answers((
  [No `clap`, no TUI, no HTTP client, no async runtime, no TLS stack. Which
   matters most depends on your service, but the async runtime is usually the
   answer: pulling one into a synchronous codebase is invasive in a way an
   argument parser is not, and a library that quietly required `tokio` would
   rule itself out of a whole class of programs. The facade is synchronous with
   no I/O, which is what makes "add this to an existing service" a small
   decision.],
  [The API lets you: a unit's `grounds` names uids, and nothing stops you
   naming one you have not built. The mistake surfaces when something walks the
   graph — a closure check, a `check` run, a bundle — and finds the reference
   unresolved. This is the same behaviour the surface syntax has, and for the
   same reason: documents are written in the order people think, not in
   dependency order, and forcing the second would make the API unusable for
   the case it exists to serve.],
  [At `check`, not at construction or encoding. A unit is a value; you can
   build one that says anything, and encoding it is a mechanical translation
   that has no opinion. Rule M is a property of a unit *in relation to its
   grounds*, so it can only be evaluated once the graph is assembled — which
   is precisely why `check` is a separate pass rather than a constructor
   guard. The design lets you hold an invalid intermediate state and refuses to
   let you certify it.],
))

#recap((
  [`UnitCoreBuilder::new(...).build()` is where shape validation happens, once, at
   construction — every later operation on a built `UnitCore` (hashing, encoding,
   comparing) is infallible because of it.],
  [`to_cbor`/`from_cbor`/`canonical_uid`/`verify` are the same functions behind `fmt`'s
   round-trip guarantee, callable directly with no file and no subprocess.],
  [`Store::from_records` builds a store from values in memory; `check` runs the same ten
   passes against it that `smysl check` runs against a file — both are ordinary function
   calls, not a wrapper around the binary.],
  [Both examples in this chapter were compiled and run for real, against the facade, with
   `--no-default-features` — not printed from memory.],
))

// ═══════════════════════════════════════════════════════════════════════
#part(number: "IX", title: "A Complete Walkthrough")
// ═══════════════════════════════════════════════════════════════════════

#chapter(number: 29, title: "Incident to Report: The Full Pipeline")

Every other chapter in this book takes one command at a time. This one does not — it is a
single incident, worked start to finish, with every command earning its place in the story
rather than being demonstrated for its own sake. The scenario is a real gateway latency
incident with two engineers investigating the same page from different angles, and it uses
every part of the tool this book has covered: authoring, checking, merging, retracting,
packing, threading, rendering, and a closing conformance check.

#callout(label: "Why")[
  A chapter per command teaches you what each one does and leaves out the thing
  that actually makes the tool worth adopting: that the commands compose, and
  that the document survives every hop between them.

  The scenario here is the ordinary one. Two engineers page in on the same
  alert, look at different subsystems, and reach conclusions that partly
  contradict each other. Overnight a metric arrives that disproves one of the
  theories. In the morning somebody has to hand a leadership audience a brief
  that fits on a page — and be able to answer, three weeks later in a review,
  where any sentence in it came from.

  Done in prose, that last question has no answer by the second retelling;
  Chapter 1 measured how fast it stops having one. This chapter is the same
  incident with the answer still attached at the end.
]

#section("The page, and the first write-up")

`gateway.p99{region=us-east}` alerts. You are on call, and the first thing you do is not
open a dashboard for someone else to read later — you write down what you see, as a
document, while you still remember why you looked at what you looked at:

```
@doc smysl/0.1 {
  id: v/f8a
  intent: triage
  lang: en
  granularity: { profile: default }
  roots: [c/cause]
}

@evidence e/alpha-trace { status: measured, source: { kind: metric, ref: "gateway.p99{region=us-east}", captured: 2026-07-22 } }
~ Gateway p99 in us-east rose to 2.4 s between 11:00 and 11:40.

Sampled at ten-second resolution from the edge collector, with the rise confined
to us-east and every other region flat inside its usual band for the whole
forty-minute window.

@claim c/cause { status: inferred, grounds: [e/alpha-trace] }
~ The us-east gateway is exhausting its upstream connection pool.

@evidence e/alpha-pool { status: measured, source: { kind: metric, ref: "gateway.pool_wait{region=us-east}", captured: 2026-07-22 } }
~ Pool acquisition wait reached 1.9 s at the peak.

@claim c/alpha-scope { status: derived, grounds: [e/alpha-trace] }
~ No region other than us-east moved outside its band.

@claim c/pool-size-32 { status: derived, grounds: [e/alpha-pool] }
~ The pool was sized at 32 connections when the rise began.

@claim c/pool-size-64 { status: derived, grounds: [e/alpha-pool] }
~ The pool was sized at 64 connections when the rise began.

@rel e/alpha-pool --backs--> c/cause
@rel c/pool-size-32 --supersedes--> c/alpha-scope
@rel c/pool-size-64 --supersedes--> c/alpha-scope
```

You do not remember the exact pool size, so you write down both figures you saw quoted in
chat, each `supersedes`-ing your scope claim — a decision to reconcile later, not now, and
one this format lets you defer honestly rather than silently pick a number. Check it before
moving on, the way Chapter 8 taught you to:

#screen(caption: "$ smysl check alpha.smy")[
```
alpha.smy: 16 records, 6 units, 0 diagnostic(s)
```
]

Clean. Nothing here is *true* yet — `check` never promises that — but the document is
internally consistent: every status respects its grounds, every reference resolves.

#section("A second read on the same incident")

A teammate is paged too, and investigates independently — no coordination, no "who's
looking at what" thread, because none is needed yet:

```
@doc smysl/0.1 {
  id: v/f8b
  intent: triage
  lang: en
  granularity: { profile: default }
  roots: [c/cause]
}

@evidence e/beta-deploy { status: measured, source: { kind: file, ref: "deploy/gateway-7.1.manifest", captured: 2026-07-22 } }
~ Gateway 7.1 reached us-east at 10:58, two minutes before the rise.

@claim c/cause { status: inferred, grounds: [e/beta-deploy] }
~ The us-east gateway regressed because 7.1 shortened the upstream timeout.

@evidence e/beta-retries { status: measured, source: { kind: metric, ref: "gateway.retries{region=us-east}", captured: 2026-07-22 } }
~ Upstream retries rose eleven-fold over the same forty minutes.

@claim c/timeout-not-culprit { status: derived, grounds: [e/beta-retries] }
~ Retries were already elevated before 7.1 reached us-east.

@rel e/beta-retries --backs--> c/cause
@rel c/timeout-not-culprit --rebuts--> c/cause { weight: 0.6 }

@thread t/beta-brief { schema: brief, owner: "model:beta" }
~ 7.1 shortened the upstream timeout; the retry evidence is not unambiguous.
  bottom-line → c/cause
  risk → c/timeout-not-culprit
```

Notice: your teammate already drafted their own `brief` thread while writing this up, and
already recorded an objection to their own leading theory. Checked clean on its own:

#screen(caption: "$ smysl check beta.smy")[
```
beta.smy: 14 records, 5 units, 0 diagnostic(s)
```
]

Two independently-authored, individually clean documents about the same incident, both
using the label `c/cause` for what each of you believes is *the* root cause — and they
disagree. This is exactly what `merge` exists for:

#screen(caption: "$ smysl merge alpha.smy beta.smy -o merged1.cbor")[
```
alpha.smy: contention k/co7i7feme3q3sed4l63dkcgmmuy over b3:v5sjn55ujeflrdyu5qycgmtvz3 (2 positions, supersession-fork)
beta.smy: contention k/c3xa27zib4d7t5rme4xekauzh4t over b3:t65rff76bcbxnwzw4oxzcadthe (2 positions, live-rebuttal)
beta.smy: contention k/cboxvjp36tnst3vmpsoo2mmvhqu over b3:cjixk2inyftvxj55d53w2ivxej (2 positions, label-collision)
```
]

Three contentions, three different reasons, and none of them is `merge` failing — they are
`merge` doing its job. Read them in order:

- *`supersession-fork`* over the scope claim: your own two pool-size claims both
  `supersedes` the same target. You raised this yourself, in the first write-up.
- *`live-rebuttal`* over your teammate's root-cause claim: their own `rebuts` edge landed
  both sides of that disagreement in the same graph.
- *`label-collision`*: `c/cause` in your file and `c/cause` in theirs point at two different
  uids. Same nickname, two different real claims — exactly the situation `Label` (Appendix
  D) warns about.

`check` on the result reports no *errors* — a contention is a materialised disagreement,
not an invalidity:

#screen(caption: "$ smysl check merged1.cbor")[
```
merged1.cbor: 30 records, 11 units, 0 diagnostic(s)
```
]

#whatsnext[
  Nothing has been decided yet — the merge recorded three open disagreements rather than
  picking winners for any of them. The next step is finding evidence that actually resolves
  one.
]

#section("New evidence, staged and merged")

Twenty minutes later, someone rolls gateway 7.1 back, and the incident channel gets a note
about it. There is no model provider configured in this environment to run `smysl ingest`
against live, so — as Chapters 10 and 13 do when the same gap comes up — this step is
honestly hand-simulated at exactly the point `ingest` would hand off: a reviewed batch
sitting in `.smysl/staged.smy`, waiting for `merge --staged` to confirm it, precisely as
rule S requires.

```
@evidence e/rollback { status: measured, source: { kind: file, ref: "deploy/gateway-7.1-rollback.log", captured: 2026-07-22 } }
~ Rolling gateway 7.1 back at 11:52 returned p99 to its pre-incident baseline within four minutes.

@claim c/rollback-confirms { status: derived, grounds: [e/rollback] }
~ The rollback's effect confirms the 7.1 timeout change as the cause, not the connection pool.
```

Committing it folds straight into the same `merge` you already ran:

#screen(caption: "$ smysl merge alpha.smy beta.smy --staged -o merged2.cbor")[
```
alpha.smy: contention k/co7i7feme3q3sed4l63dkcgmmuy over b3:v5sjn55ujeflrdyu5qycgmtvz3 (2 positions, supersession-fork)
beta.smy: contention k/c3xa27zib4d7t5rme4xekauzh4t over b3:t65rff76bcbxnwzw4oxzcadthe (2 positions, live-rebuttal)
beta.smy: contention k/cboxvjp36tnst3vmpsoo2mmvhqu over b3:cjixk2inyftvxj55d53w2ivxej (2 positions, label-collision)
smysl merge: committed 2 staged record(s)
```
]

#screen(caption: "$ smysl check merged2.cbor")[
```
merged2.cbor: 34 records, 13 units, 0 diagnostic(s)
```
]

The rollback evidence does not automatically resolve anything in the graph by itself — it
is just two more units. What it gives *you* is the confidence to act on the
`label-collision` contention: alpha's pool-exhaustion theory is now the one to retire.

#whatsnext[
  With the deciding evidence in hand, the next step is not another merge — it is
  withdrawing the theory the evidence ruled out, and doing that honestly means reporting
  what else would be affected before touching anything.
]

#section("Retracting the disproven theory")

`retract --dry-run` reports the blast radius without applying anything, and it is safe to
run before you have even decided who is retracting:

#screen(caption: "$ smysl retract --dry-run b3:cjixk2inyftvxj55d53w2ivxej merged2.cbor")[
```
merged2.cbor: retracting b3:cjixk2inyftvxj55d53w2ivxej would reach 1 unit(s), orphaning 0
```
]

One unit — itself — and nothing orphaned. That is worth reading as information, not just a
number: nothing else in the graph was ever built on top of alpha's pool-exhaustion claim.
It was a dead end from the start, not a foundation, which is exactly why retiring it now is
safe. Applying it for real, without `--dry-run`, hits something instructive:

#screen(caption: "$ smysl retract b3:cjixk2inyftvxj55d53w2ivxej merged2.cbor")[
```
merged2.cbor: retracting b3:cjixk2inyftvxj55d53w2ivxej would reach 1 unit(s), orphaning 0
smysl retract: origin requires 1 distinct agents, got 0
```
]

`retract`'s default authority is `origin` — only an agent that *attested* the unit may
retract it. Attestations have no surface syntax (they are authored programmatically, never
by hand), so a unit written straight into a `.smy` file has none, and `origin` can never be
satisfied for it. That is not a bug to work around quietly; it is the tool refusing to guess
who has standing to retract something. Naming an issuer and relaxing the authority is the
honest way through it:

#screen(caption: "$ smysl retract b3:cjixk2inyftvxj55d53w2ivxej merged2.cbor --as human:oncall --authority any --reason \"rollback confirms 7.1 timeout, not pool exhaustion\"")[
```
merged2.cbor: retracting b3:cjixk2inyftvxj55d53w2ivxej would reach 1 unit(s), orphaning 0
merged2.cbor: 1 unit(s) now read as unfounded
```
]

One thing to know before relying on this in a pipeline: this build's `retract` reports the
blast radius and the resulting effective status, but does not itself write the mutated
store to `--output` — the retraction lives only in that process's memory. The way to carry
it forward is the same way everything else here travels: as a record. `RelKind::Retracts`
is an ordinary relation, so a one-line file naming it and folding it back in with `merge`
persists exactly what `retract` just reported:

```
@rel b3:cjixk2inyftvxj55d53w2ivxejoderrdj73hnpaprphlpzqpjhyq --retracts--> b3:cjixk2inyftvxj55d53w2ivxejoderrdj73hnpaprphlpzqpjhyq
```

#screen(caption: "$ smysl merge merged2.cbor retract.smy -o merged3.cbor  &&  smysl check merged3.cbor")[
```
merged3.cbor: 35 records, 13 units, 0 diagnostic(s)
```
]

#whatsnext[
  The store now honestly reflects what happened: one theory retired, its blast radius on
  record, nothing orphaned. What it is still missing is the thing a postmortem is actually
  *for* — a conclusion.
]

#section("A finding, and a budget-bounded pack")

Everything so far has been claims and evidence. A postmortem needs a `finding` — the unit
type that says what the incident actually concluded, grounded in the claim the evidence
now supports:

```
@finding f/root-cause { status: inferred, grounds: [b3:t65rff76bcbxnwzw4oxzcadthe2s35hisuingd2p6gieh6ayejea] }
~ The gateway 7.1 timeout regression caused the us-east p99 spike; the pool-exhaustion theory is retracted.
```

#screen(caption: "$ smysl merge merged3.cbor finding.smy -o merged4.cbor  &&  smysl check merged4.cbor")[
```
merged4.cbor: 37 records, 14 units, 0 diagnostic(s)
```
]

Fourteen units now sit in this store — both original theories, the evidence behind each,
the rollback confirmation, the retraction, and the finding. Nobody reading a report wants
all fourteen. `pack --focus` fits the graph to a budget around the claim that matters now,
and it is the moment to see, concretely, whether retiring the wrong theory actually mattered
to what gets shipped:

#screen(caption: "$ smysl --format surface pack --budget 120 --explain --focus b3:t65rff76bcbxnwzw4oxzcadthe merged4.cbor")[
```
b3:t65rff76bcbxnwzw4oxzcadthe @L1  C5  in focus
b3:wo5nb45msupuxfptfxpbh5q4ws @L0  C3  rebuts b3:t65rff76bcbxnwzw4oxzcadthe
b3:34xv6klbhpxqsqsvhm5s3xuegn @L0  C2  ground of b3:t65rff76bcbxnwzw4oxzcadthe
b3:cjixk2inyftvxj55d53w2ivxej dropped: budget
b3:hpffca2bw76gvsrc6tjvr2q6uw dropped: budget
b3:bwgbs3amtoatm36pawri7snlz4 dropped: low-value
...
merged4.cbor: 3 of 13 unit(s), 112 of 120 tokens, greedy mode, gap 0.154
```
]

Read the three kept lines the way Chapter 18 taught you to: the surviving claim (`C5`, in
focus), its rebuttal (`C3` — "retries were already elevated" travels with the claim it
objects to, not silently dropped), and its ground (`C2`). Alpha's retracted claim is not
merely low-priority here — it is `dropped: budget`, competing on density like anything
else and losing, the same as beta's own weaker units. Retraction changed what the graph
*means*; a tight, well-chosen budget is what keeps a resolved dead end from crowding out
the version worth shipping.

#whatsnext[
  A budget-bounded selection is for a prompt or a quick read — it truncates by design.
  A report a person reads front to back needs an actual order: a thread.
]

#section("Deriving and rendering the brief")

`thread --derive` builds that order deterministically from the graph itself:

#screen(caption: "$ smysl thread --derive brief --explain --id t/postmortem --as human:oncall merged4.cbor")[
```
merged4.cbor: bottom-line  1 of 1..1
merged4.cbor: support      3 of 1..3
merged4.cbor: risk         1 of 0..2
merged4.cbor: ask          0 of 0..1
merged4.cbor: 9 unit(s) not selected
```
]

Every required role filled — `bottom-line` from the new finding, three units backing it as
`support`, the surviving rebuttal carried into `risk`. This is the moment to check something
before building further on it, in the spirit of this whole book: a thread derived over units
that lost their labels crossing a CBOR merge is correct in memory, but re-serialising it to
surface text and re-parsing it is not yet a clean round trip in this build when the units it
references have no label — a real rough edge, not a hidden one, and exactly why `check`
exists between every step rather than only at the end. The safe path here is the one your
teammate already handed you: they filed `t/beta-brief` back when they wrote `beta.smy`, and
because a thread's steps are stored by uid from the moment it is authored, it survived every
merge since untouched. Render it now that the story is settled:

#screen(caption: "$ smysl render --thread t/beta-brief --profile exec --target markdown merged4.cbor")[
```
# 7.1 shortened the upstream timeout; the retry evidence is not unambiguous.

*brief · profile exec*
*for engineering leadership*

## bottom-line

≈ The us-east gateway regressed because 7.1 shortened the upstream timeout.

A shortened timeout turns a slow upstream into a retried one, and retries are
what fill the queue, so beta reads the same latency curve as a consequence of
the release rather than of the pool being undersized.

> **contested** — k/c3xa27zib4d7t5rme4xekauzh4t: contested, 2 position(s) on record

## risk

⊢ On the other hand, retries were already elevated before 7.1 reached us-east.

> **contested** — k/c3xa27zib4d7t5rme4xekauzh4t: contested, 2 position(s) on record

---

**Open contentions:** k/c3xa27zib4d7t5rme4xekauzh4t, k/co7i7feme3q3sed4l63dkcgmmuy
```
]

Two things worth noticing. First, the `live-rebuttal` contention over the bottom-line claim
survives rendering under `exec` — that is rule V2, and it means a reader of the *exec*
brief still sees that the risk section is a live, recorded objection, not editorial hedging.
Second, `k/co7i7feme3q3sed4l63dkcgmmuy` — the `supersession-fork` over the pool size — is
*still open*. Retiring alpha's root-cause theory did not resolve the smaller disagreement
about how big the pool actually was; that is a second, independent decision this report is
honestly still waiting on, and the render says so rather than letting it quietly disappear.

#whatsnext[
  The report is written. The last thing worth doing before handing it off is asking the
  store the same question `check` has been answering all along, but at the strength a
  handoff deserves.
]

#section("Certifying the store")

Every step in this incident went through `check` on the way. The closing move is asking a
stronger question of the *final* result: not just "is this valid" but "is this safe to hand
to someone else to read, merge, or build on":

#screen(caption: "$ smysl check --conformance C-Full merged4.cbor")[
```
merged4.cbor: C-Full: pass
merged4.cbor: 37 records, 14 units, 0 diagnostic(s)
```
]

`C-Full` is every conformance class at once — safe to read, safe to consume, safe to author
into, safe to merge. The store passes it with the retraction, the finding, and both open
contentions all still on record. Nothing about certifying the store required resolving
`k/co7i7feme3q3sed4l63dkcgmmuy` first, and that is deliberate: conformance is a statement
about the store's *shape*, never about whether every question in it has been answered.

#whatsnext[
  This chapter moved quickly through commands the rest of the book gave a full workflow
  each — Appendix A has the complete flag reference for every one of them when you need the
  exact syntax again. Appendix B and Appendix C are the same kind of quick lookup for every
  diagnostic code and exit code this incident could have hit but did not. And where this
  walkthrough moved past *why* the format is shaped the way it is, or exactly how a
  construct like `--supersedes--` or `granularity` is spelled, `SMYSL_RATIONALE.typ` and
  `SMYSL_FORMAT_GUIDE.typ` are the two sibling documents that slow back down.
]

#exercises((
  [Work the whole chapter's scenario yourself, from the two write-ups to the
   final `check --conformance`. Then answer the question the chapter opens
   with: pick any sentence in the rendered brief and say, using only the store,
   where it came from and how strong it is entitled to be.],
  [At the retraction step, a theory is disproven by new evidence. Run the
   retraction with `--dry-run` first and note what it says it will reach. Now
   consider the counterfactual: if this incident had been handled in a chat
   thread and a shared document, what would have happened to the claims that
   rested on the disproven theory?],
  [The chapter ends with a conformance check rather than a render. Argue for
   that ordering — why is certifying the store the last step rather than
   producing the artifact?],
))

#answers((
  [Every sentence traces to a unit, every unit carries a status and its
   grounds, and `trace` walks the chain to whatever measurement or citation
   sits at the bottom. That is the entire claim of this book made concrete: not
   that the brief is *better written* than a prose one, but that it is
   answerable. If you found a sentence you could not trace, it came from the
   thread's framing rather than a unit, and that is worth noticing too.],
  [Nothing would have happened to them, which is the problem. They would have
   remained in the document, reading exactly as confident as before, because
   prose has no mechanism by which withdrawing one paragraph reaches the
   paragraphs that depended on it. Somebody would have had to remember — weeks
   later, under time pressure — which conclusions rested on the theory that
   turned out to be wrong. `retract --dry-run` answers that question by
   walking `grounds`, before you commit to anything.],
  [Because the artifact is disposable and the store is not. A rendered brief
   can be regenerated at any time from the store — `render` is pure, so the
   same store gives the same bytes forever. The store is the thing that has to
   be right, and certifying it last means you are asserting the *final* state
   is consumable, after every merge, retraction and derivation has landed.
   Checking before the last edit would certify something that no longer
   exists.],
))

#recap((
  [Two independently-authored, individually clean documents about the same incident can
   disagree, and `merge` records exactly how — `supersession-fork`, `live-rebuttal`, and
   `label-collision` are three different shapes of the same underlying event, none of them
   an error.],
  [`retract --dry-run` is always safe and needs no authority; applying a retraction for
   real defaults to `origin`, which surface-authored units can never satisfy — `--as` and
   `--authority any` are how a human takes explicit responsibility for one instead.],
  [A retraction's blast radius is a fact about the graph worth reading even when it is
   small: zero orphaned units meant the retired theory was never load-bearing.],
  [`pack --focus` at a real budget is where a retraction's consequences become visible —
   the retracted claim did not just rank lower, it lost outright to units that still
   mattered.],
  [A thread that was authored with labels intact and carried through every merge survives
   cleanly; deriving a fresh one over units that already lost their labels is correct in
   memory but not yet guaranteed to round-trip through surface text in this build — check
   what you built before you build the next thing on it.],
  [`check --conformance C-Full` on the final store is a statement about safety to hand off,
   not a claim that every open question — like the still-unresolved pool size — has been
   settled.],
))
