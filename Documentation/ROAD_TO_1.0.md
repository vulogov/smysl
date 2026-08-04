# The road to 1.0.0

**Status:** a plan, not a schedule. Each phase names what it produces and how you know it is
done. Nothing here has a date, because the only honest gate on 1.0 is evidence and evidence
arrives when it arrives.

**The decision this rests on:** smysl is one product — format, libraries and CLI — under one
version. The workspace is not split. That settles a contradiction `API_CONTRACT.md` left open:
its "bucket 2" said the provider, render and retrieval surfaces are *a seam, not a promise*,
and a single-version 1.0 cannot say that about names it exports. The seam is therefore
**stabilised**, not exempted, and that is the largest item below.

---

## What 1.0 commits to

Three things, and it is worth being exact because the rest of this document is downstream of
them.

1. **No breaking change to any exported name without a 2.0.** The facade's golden file lists
   239 names at `--all-features` and 199 at `--no-default-features`, but the real figure is
   **12 111 public items across 52 public modules**: the eleven library crates ship at one
   version, all must be published, and `make semver` already enforces every item in each.
   §0.2 measures how much of that is a choice — about 8% — and where it sits.
2. **The format stays readable.** A 1.0 reader reads every document a 1.x writer produces.
3. **The guarantees hold.** A1–A6 and rules M, T, L, R, U, I, S, V1, V2, X, D, P are part of
   the contract, and A5 already says making an operation non-reproducible is breaking whatever
   the signature does.

What 1.0 does **not** commit to: that everything is finished, that every survivor is dead, or
that the CLI is beautiful. It commits to not moving.

---

## Phase 0 — two decisions

Neither is work. Both block the phases after them. The first is settled; the second is not.

### 0.1 The format version — **decided: bump, and bump it last**

`smysl/0.1` becomes `smysl/1.0`, after every preparation and migration is complete and
**before** the source tree goes to 1.0.0. The order is the decision: the format arrives at 1.0
already supported by everything that reads it, and the crate version follows.

That order exists because a format bump is not a rename. Four implementations read this format
— the Rust, `python/`, `nodejs/`, `go/` — and §8.2 says a reader MUST reject a version absent
from its list and MUST NOT infer compatibility from one that looks close. Flip the writer first
and every other reader refuses the output.

**The migration, in order:**

1. **Readers accept both.** `FORMAT_VERSIONS_SUPPORTED` (`smysl-core/src/lib.rs:43`) becomes
   `["smysl/0.1", "smysl/1.0"]`, and the same list grows in `python/`, `nodejs/` and `go/`.
   Nothing writes `smysl/1.0` yet, so nothing breaks — and this step must ship, and be
   *released*, before step 3, or a 1.0 writer emits documents the field cannot read.

   Two tests pin the current list and will need updating with it:
   `smysl-core/src/lib.rs:74` and `tests/versioning.rs:32`. Both are meant to be updated by
   hand — they exist so the declared versions cannot drift from the specification unnoticed.

2. **Fix the writer, which this bump makes urgent.** §8.5 records a trap that is harmless at
   one supported version and a defect at two: the wire carries no version, a surface parser
   validates the declared one and *discards* it, and `write_surface`
   (`smysl-core/src/surface/write.rs:86`) reconstructs the header from
   `FORMAT_VERSIONS_SUPPORTED[0]`. With two entries a document declaring one version is read
   and written back declaring the other. Uids are unaffected — they are over CBOR, which
   carries no version — but the header would lie, and the next reader trusts the header.

   `tests/versioning.rs:54` fails the moment that list grows, deliberately, and says what has
   to be decided first. This is the moment it was written for. `ParseOutcome` needs to carry
   the version the document declared, and `write_surface` needs to emit that. Note this is a
   change to a public type, so it is a `SEMVER_BREAKING` entry — another reason it lands
   before the cut and not after.

3. **Flip the writer.** `smysl/1.0` becomes the version new documents declare, with `smysl/0.1`
   still read. Fixtures stay as they are — the point of step 1 is that old documents keep
   working forever.

4. **Then, and only then, the crate goes to 1.0.0.**

**One thing this does not decide.** `KERNEL_SCHEMA` is `smysl.kernel/0.1` and is a third axis,
independent of both the format string and the crate version — it names the shape of the kernel
fields, and §8 keeps it separate on purpose. Bumping it is a *different* migration with
different consequences, and nothing above requires it. Whether `smysl/1.0` should ship with
`smysl.kernel/0.1` is a decision to take explicitly rather than by symmetry.

**Also to update:** §8.6 of the format spec currently answers "is `smysl/0.1` frozen?" with
"no, and it is not stable-forever either", resting on the `0.` prefix. At `smysl/1.0` that
answer changes, and §8.2's rule — that a version bump signals a break — has to be reconciled
with a bump that deliberately carries none. The honest wording is that 1.0 marks the format
*settled*, and that the compatibility event is the readers being taught in step 1.

**Done when:** all four implementations accept both strings, `versioning.rs` passes with two
entries in the list, a `smysl/0.1` fixture still round-trips byte for byte, and a document
declaring each version is written back declaring the one it declared.

### 0.2 What freezing the seam costs

**The size of the freeze.** The facade's golden file lists **239 names**. The eleven library
crates export **12 111 public items across 52 public modules**, and since they share one
version and all must be published for the facade to resolve, 1.0 freezes the larger number.

That is not a hypothetical. `make semver` already runs `cargo-semver-checks` per crate over
every published crate, so all 12 111 are enforced as contract today. What 1.0 changes is that
a break stops being a minor bump and becomes a 2.0.

**A gap worth fixing first.** The two gates measure different things.
`cargo public-api` on the facade returns 239 items with *or* without `--simplified`, because
re-exports from other crates are listed as `pub use` and never expanded. So
`tests/public-api.txt` records that `Store` is exported and cannot see a single one of its
methods — change `Store::matching_prefix`'s signature and the golden file does not move.
`make semver` catches it, per crate. The documented contract and the enforced contract are
therefore not the same set, and before 1.0 they should be, or `API_CONTRACT.md` describes
something narrower than what the version number promises.

**How much of the freeze is actually a choice.** Splitting the surface by whether an item
belongs to a type the facade re-exports:

| crate | items | tied to an exported type | discretionary |
|---|---|---|---|
| `smysl-core` | 6 125 | 5 825 | 300 (4%) |
| `smysl-graph` | 2 084 | 2 076 | 8 (0%) |
| `smysl-provider` | 988 | 727 | **261 (26%)** |
| `smysl-render` | 907 | 848 | 59 (6%) |
| `smysl-ingest` | 751 | 476 | **275 (36%)** |
| `smysl-pack` | 414 | 383 | 31 (7%) |
| these six | 11 269 | 10 335 | 934 (8%) |

*(The attribution is a name-matching heuristic, so "tied" is over-counted and 8% is a floor.)*

The shape of that table is the answer. Around 92% of the surface is methods and variants on
types the facade deliberately exports — `Store`, `Doc`, `UnitCore`. Freezing those is not a
cost to be managed; it is what 1.0 *means*, and no amount of narrowing touches it.

The discretionary remainder concentrates in `smysl-provider` and `smysl-ingest` — which is to
say, in the seam. `API_CONTRACT.md`'s instinct to treat those crates as a category apart was
right about *which* surface is unsettled, and wrong only about what follows from it. The seam
is where nearly all the genuinely optional surface lives, and it is also the part that has
moved most recently: `Hybrid` changed twice inside 0.7, and `Retriever` is one cycle old.

**The options, then, are narrower than they first look.**

**A — freeze as it stands.** No work. Every mapper struct, every ingest internal, becomes
contract. The cost is that ordinary refactoring in the two least-settled crates becomes a 2.0;
given that 0.10 alone changed `skip_one`, `Dec`'s traversal and `SourceRef::new`, that is a
live constraint rather than a theoretical one.

**B — narrow the seam, not the workspace.** Demote to `pub(crate)` in `smysl-provider` and
`smysl-ingest` what the facade does not reach. Roughly 500 items, in two crates, rather than a
sweep across 52 modules. It is a breaking change, which is why it belongs before 1.0. Rule A
is what says whether a narrowing went too far — but it has no gate today, so §1.2 builds one
first. (`make purity` enforces rule B, not rule A.)

**C — tier the promise in prose.** Keep everything public, mark the rest `#[doc(hidden)]`, say
only the facade is contract. Cheap, and it reinstates exactly the contradiction that deciding
"one product" removed: `cargo-semver-checks` does not read intent, so it either flags hidden
items — noise — or is told to ignore them, at which point the guarantee is a comment.

**Recommendation: B, plus closing the gate gap.** The scope is two crates, and it is the same
two the seam review has to read anyway — so do them together: decide the shape, then decide
what stays public. Then regenerate the golden file so the documented surface and the enforced
surface agree, and 1.0 promises one thing rather than two. **§1.2 is that plan.**

Option A is defensible if the appetite for it is low. It should then be chosen out loud, with
the maintenance cost named, rather than by leaving 0.2 undecided.

---

## Phase 1 — make the surface something worth freezing

The engineering core of this plan.

### 1.1 The `#[non_exhaustive]` audit

**72 of 179 public types do not carry it.** Post-1.0 each one is a type that cannot gain a
field. That is not "add 72 attributes": it is 72 decisions, and a good number should stay
exhaustive on purpose — `Hlc`, `Date` and the identifier newtypes have shapes that are complete
by definition, and forcing callers through a constructor buys nothing.

The work is to go through them once, deciding *closed by design* or *open for growth*, and to
write the reason where the type is. Output: every public type has an answer, and
`API_CONTRACT.md` records the rule rather than the list.

**Done when:** no public type lacks either the attribute or a sentence saying why it is closed.

### 1.2 The seam: review it, then narrow it

The largest item in the plan, and §0.2 is why: the seam is where almost all of the
discretionary public surface lives. It is also the surface that has moved most recently, which
is the same fact from the other side.

**What the facade actually reaches.** Measured, not assumed:

| | modules | reached by the facade | untouched |
|---|---|---|---|
| `smysl-ingest` | 13 | `attest`, `stage` (whole); one fn each from `ceiling`, `path`, `recipe` | `chunk`, `import`, `json_ast`, `monotone`, `prompt`, `quote`, `repair`, `schema` |
| `smysl-provider` | 7 | `config::ProviderConfig`, `map::build`, `usage::{GroupBy, Totals}`, and `registry`'s types via the crate root | `http`, `runtime`, `stream` |

Eleven of twenty modules are exported to nobody in particular. That is the narrowing target,
and it is a smaller and better-defined job than "audit 52 modules".

The steps are ordered because each one produces the thing the next needs.

---

**S1 — give rule A a gate, and watch it fail.**

Rule A is stated in `src/lib.rs`: *"no CLI capability may be unreachable from here, and no code
path may be CLI-only."* The manual restates it as a checked fact — *"every `cmd_*` function in
`src/main.rs` calls straight into a facade re-export"*. Nothing enforces it. `make purity` is
rule B, a different rule.

It is also, today, false. `src/main.rs` names exactly two paths that go around the facade:

- **`src/main.rs:3509`** — `cmd_import` does `use smysl_ingest::import::{from_csv,
  ImportOptions}`. `smysl import` is the only producer of `measured` units, and a consumer
  holding only the facade cannot do what it does.
- **`src/main.rs:3623`** — `smysl_tui::run` / `smysl_tui::App`; the facade re-exports nothing
  from `smysl-tui`. Arguably presentation rather than capability, but the rule as written does
  not carve that out, so the carve-out should be written down or the re-export added.

The gate is small: an `xtask` check that `src/main.rs` contains no `smysl_[a-z]*::` path.
Write it, watch it fail on those two, then fix them — re-export `from_csv` and `ImportOptions`
from the facade, and decide the `smysl-tui` question explicitly.

This comes first because it is the instrument for everything after it. Narrowing without it is
guesswork; with it, "did I take away something the CLI needs" is a command.

**Done when:** the check exists, is in `make ci`, and passes.

---

**S2 — decide what the tests are allowed to see.**

The real cost of narrowing, and the step most likely to be underestimated. Integration tests in
`tests/` are separate crates: they see `pub`, not `pub(crate)`. Eight files reach into the
modules S3 and S4 would demote —

`smysl-provider/tests/`: `a2_lazy_runtime.rs`, `status_taxonomy.rs`,
`capabilities_are_honest.rs`, `ollama_live.rs`, `deepseek_live.rs`;
`smysl-ingest/tests/`: `gate.rs`, `providers_live.rs`; and the workspace's
`tests/interactions.rs`.

Three routes, and the choice is per item rather than global:

1. **Move the test inside the crate.** A unit test sees `pub(crate)`, so the item can be
   demoted. Cheapest where it works — and it does not always work. `a2_lazy_runtime.rs` exists
   *because* an integration test is a fresh process: it asserts the provider runtime has **not**
   started, which is unobservable in a unit test where an earlier test has already started it.
   Its own header says so. Moving that one destroys the guarantee it checks.
2. **Keep the item public, and record why.** For `runtime::is_started` this is the honest
   answer: it is public so that A2 can be tested from outside, and that reason belongs next to
   it. A short list of named exceptions is a contract; an unexamined 12 111 is not.
3. **A `testing` feature that re-exports internals.** Rejected unless 1 and 2 run out.
   `cargo-semver-checks` runs `--all-features`, so a feature-gated internal is frozen exactly
   like a public one — it moves the names without shrinking the promise.

**Done when:** every one of the eight files is on route 1 or route 2, and each route-2 item has
its reason written where it is declared.

---

**S3 — the shape review, before the visibility change.**

Reading a type for "would we regret this at 2.0" has to happen before deciding it stays public,
because a type that is wrong should be fixed, not frozen quietly.

`ProviderConfig`, `Request`, `Completion`, `Usage`, `Capabilities`, `StructuredMode`, `Probe`,
`Provider`, `Registry`; `Bm25`, `Hybrid`, `Semantic`, `Query`, `Hit`, `Retriever`; `Ir`,
`Profile`, `BuildOptions`.

One defect is already known and belongs here: **`status_error` is a shared contract shared by
convention.** All five mappers expose `status_error(u16, &str) -> ProviderError` with the same
signature, as an inherent method rather than part of the `Provider` trait — which is why
`tests/status_taxonomy.rs` has to build boxed closures to test them together. It should be on
the trait, where the compiler enforces the shape a sixth mapper must have. Note this cuts
against S4: a trait method is public by necessity, so this decision comes first.

**Done when:** each type has been read once with that question in mind, and the answers — not
just the changes — are in `API_CONTRACT.md`.

---

**S4 — narrow, one crate at a time, provider first.**

Provider first because it is smaller, better understood, and its live tests give a second
signal. For each of the eleven untouched modules: demote to `pub(crate)`, build, and let S1's
gate and the test suite say what was needed after all. Anything that has to come back comes
back deliberately, with a reason, as an S2 route-2 exception.

Expect the count to land above zero and well below 934. The measurement's 8% was a floor
computed by name-matching, and some of those modules will turn out to have a legitimate
consumer — that is a result, not a failure.

**Done when:** both crates build, `make ci` is green, and every still-public module in them is
either facade-reachable or a recorded exception.

---

**S5 — make the documented contract equal the enforced one.**

§0.2 found the two gates disagree: `cargo public-api` on the facade returns 239 items with or
without `--simplified`, because cross-crate re-exports are never expanded, so
`tests/public-api.txt` cannot see a single method of any type it lists. `make semver` sees all
of them, per crate.

After S4 the surfaces have moved, so this is the moment to fix it: record per-crate goldens
alongside the facade's, or state in `API_CONTRACT.md` that the facade file is an index and
`make semver` is the contract. Either is defensible; having neither written down is not.

**Done when:** `API_CONTRACT.md` says which artefact is the contract, and it is true.

---

**S6 — land it as a break.**

All of S1–S5 goes through `SEMVER_BREAKING`, which then has to be empty again before the cut.
This is the last cycle in which narrowing is free; after 1.0 every item on this list is a 2.0.

### 1.3 Empty `SEMVER_BREAKING`

Everything Phase 1 breaks goes through the list, and the list must be empty at the 1.0 cut.
It currently names three crates from 0.12.

---

## Phase 2 — the verification a 1.0 should not ship without

Not "fix everything". These three, because each is a place where the project cannot presently
say what it knows.

### 2.1 Make the CLI measurable, then measure it

The CLI scored **73.6% mutation survivors**, more than twice the worst library crate. Part of
that is real — `src/main.rs` has four tests across 3 600 lines — and part is that `make
doc-output`, which replays 46 documented transcripts against the built binary, is a Python
script no `cargo test` invokes, so nothing that counts coverage can see it.

Wire `doc-output` in as an integration test, re-run mutation testing on `smysl`, and read the
new number. **The output of this step is a fact, not a fix.** If the rate collapses, the CLI is
better tested than it looks and the finding was about measurement. If it barely moves, then
four tests across 3 600 lines is the whole story and Phase 2.2 gets larger.

### 2.2 Whatever 2.1 reveals

Sized by the answer. `progress.rs` is already known: 52 survivors in 394 lines, every one an
arithmetic operator or comparison in bar drawing, under twelve tests that check structure and
never numbers. A wrong percentage is not a correctness defect for the format, but it is on
screen every time anybody runs the tool.

### 2.3 Provider acceptance

Gate 4's remaining item, and the only one with a hard external dependency: whether OpenAI and
Anthropic **accept the translated schema**. Unreachable by reading — and reading has now found
two defects without a key, so this is genuinely the residue.

The 0.12 mutation run reframed what "verified" has meant here: the failure taxonomy was
untested on all five mappers, including the three exercised live, because nobody provokes a 401
against a real endpoint. `tests/status_taxonomy.rs` closed that. What a key adds is narrower
than gate 4 has implied, and it is still needed.

---

## Phase 3 — evidence of stillness

The part that cannot be rushed, and the only real gate.

**Two consecutive cycles that end with `SEMVER_BREAKING` empty at the cut, both published.**

Published matters. `cargo-semver-checks` compares against the registry, so an unpublished
release leaves the baseline stale and the gate measures nothing — 0.10 and 0.11 demonstrated
exactly that, and it parked the `parse` repair for a whole cycle. A stillness gate that is not
watching is not evidence.

0.12 broke three things. That is not a criticism of 0.12 — each break was a repair — but a
crate that broke three things last cycle has not yet shown it can go one without.

**Done when:** two cuts in a row, both on crates.io, both with an empty list.

---

## Phase 4 — the cut

1. Phase 0's format decision applied to the title pages and §8.
2. `READINESS.md`: every gate closed, or explicitly waived with the reason written down. A
   waived gate is fine; an unmentioned one is not.
3. `API_CONTRACT.md` promoted from "decided for three names" to the whole surface.
4. Version to 1.0.0, `BASELINE` to the last published, tag, merge, **publish**.

---

## What is deliberately not on this list

**The 357 recorded mutation survivors.** A measurement is not a to-do list. The library band —
2.6% to 31% — has been yielding roughly one real gap per crate, and reading each survivor is
the expensive part. Phase 2.1 is on the list because it produces a *number*; the rest is not,
because it produces a backlog.

**doc-output at 46 of 168.** The remaining 97 name files the prose asks the reader to create,
and the fix is in the book: commit the tutorial files as fixtures so the manual and the
verifier read the same bytes. Worth doing, not worth blocking 1.0 on, and a decision about the
book rather than a task.

**Feature completeness.** Twenty-two commands, unchanged since `find` in 0.5. 1.0 is a promise
about stability, not about scope.

---

## The shortest honest summary

The **format** is arguably ready now: unchanged across twelve releases, a written versioning
policy, four independent implementations, and §2.3 verified by something other than the
implementation that defined it.

The **surface** needs Phase 1 — 72 type decisions, and the seam narrowed from eleven
unexported modules down to what is actually consumed. That last one starts by giving rule A a
gate and watching it fail, which is the shape of most of the work here: the checks that would
have told us were the things missing.

The **evidence** needs Phase 3, and Phase 3 cannot be hurried: it is two quiet cycles, and the
only way to get them is to have them.
