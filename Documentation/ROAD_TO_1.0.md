# The road to 1.0.0

**Status after 0.15.0: Phases 0 to 3 are done.**

The format is `smysl/1.0`. The surface is worth freezing and is frozen in practice — two
consecutive published cycles have ended with `SEMVER_BREAKING` empty. What stands between here
and 1.0.0 is §2.3: whether OpenAI and Anthropic accept the translated schema, which needs a key
for each and nothing else.

Each phase below names what it produced and how you know it is done. Nothing here ever had a
date, because the only honest gate on 1.0 is evidence, and evidence arrives when it arrives.

| phase | | |
|---|---|---|
| 0 | two decisions | ✅ both taken |
| 1 | make the surface worth freezing | ✅ 1.1, 1.2 (S1–S6), 1.3 |
| 2 | the verification a 1.0 should not ship without | ✅ 2.1, 2.2 — 2.3 needs provider keys |
| 3 | evidence of stillness | ✅ 0.14.0 and 0.15.0, both published |
| 4 | the cut | **the remaining gate is §2.3, which needs provider keys** |

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
   243 names at `--all-features` and 199 at `--no-default-features`, but the real figure is
   every public item in each of the eleven library crates, which `make semver` enforces
   individually. **11 643 today**, down from the 12 111 §0.2 measured before §1.2 narrowed the
   seam. §0.2 is the analysis of how much of that was ever a choice — about 8% — and §1.2 is
   what was done about it.
2. **The format stays readable.** A 1.0 reader reads every document a 1.x writer produces.
3. **The guarantees hold.** A1–A6 and rules M, T, L, R, U, I, S, V1, V2, X, D, P are part of
   the contract, and A5 already says making an operation non-reproducible is breaking whatever
   the signature does.

What 1.0 does **not** commit to: that everything is finished, that every survivor is dead, or
that the CLI is beautiful. It commits to not moving.

---

## Phase 0 — two decisions

Neither is work. Both block the phases after them. The first is settled; the second is not.

### 0.1 The format version — **done in 0.15.0**

`smysl/0.1` becomes `smysl/1.0`, after every preparation and migration is complete and
**before** the source tree goes to 1.0.0. The order is the decision: the format arrives at 1.0
already supported by everything that reads it, and the crate version follows.

That order exists because a format bump is not a rename. Four implementations read this format
— the Rust, `python/`, `nodejs/`, `go/` — and §8.2 says a reader MUST reject a version absent
from its list and MUST NOT infer compatibility from one that looks close. Flip the writer first
and every other reader refuses the output.

**The migration, in order:**

1. **Readers accept both.** ✅ *done in 0.14.0.* `FORMAT_VERSIONS_SUPPORTED` is
   `["smysl/0.1", "smysl/1.0"]`. Nothing writes `smysl/1.0` yet, so nothing breaks — and this
   step must be *released* before step 3, or a 1.0 writer emits documents the field cannot
   read.

   **The plan was wrong about the other three implementations.** It said the same list grows in
   `python/`, `nodejs/` and `go/`. There is no such list: all three read CBOR only, the wire
   carries no version string (§8.5), and `go/conformance_test.go` says so outright — *"surface
   syntax is not decoded here"*. Only a surface parser ever sees a `@doc` header, so step 1 is
   Rust-only and the migration is smaller than it looked.

   `FORMAT_VERSION_DEFAULT` is new and separate from `FORMAT_VERSIONS_SUPPORTED[0]`, because
   the two stopped meaning the same thing the moment the list grew: one is what we *emit*, the
   other is what we *accept*. Step 3 changes the first and not the second.

2. **Fix the writer.** ✅ *done in 0.14.0.* `ParseOutcome` carries the version the document
   declared, `WriteContext` carries what the header will say, and `write_surface` emits that
   instead of a build-time constant. `smysl fmt` — the round trip a user runs on purpose —
   passes one to the other.

   `tests/versioning.rs` fired exactly as intended. It was written in 0.10 to fail the moment
   the list grew, and it did, along with a sibling test that had picked `smysl/1.0` as its
   example of an *unknown* version. What stands there now is the property the count was
   standing in for: a document declaring either supported version comes back declaring the one
   it declared. Verified by restoring the old writer and watching it report *"a document
   declaring `smysl/1.0` was rewritten as: @doc smysl/0.1"*.

   **And it was not a breaking change**, which §1.1 is why: both `ParseOutcome` and
   `WriteContext` are `#[non_exhaustive]`, so each could gain a field without a major bump, and
   `write_surface`'s signature never moved. `make semver` confirms 12/12 clean.

3. **Flip the writer.** ✅ *done in 0.15.0, one release after the readers.*
   `FORMAT_VERSION_DEFAULT` is `smysl/1.0`. `smysl/0.1` is still read and still round-trips
   unchanged — the point of step 1 is that old documents keep working forever.

   One line, and it waited a whole release because of §8.2: a reader must refuse a version
   absent from its list, so until 0.14 was on crates.io a document declaring `smysl/1.0` was a
   document most readers reject. 0.14 taught the readers and wrote nothing new; it is
   published; this became safe.

   Verified end to end rather than by unit test alone. A document declaring `smysl/0.1`, run
   through `smysl fmt`, comes back declaring `smysl/0.1`. The same document bundled to CBOR —
   where the wire carries no version — and formatted back comes out declaring `smysl/1.0`.
   Both halves matter: the first is the promise to old documents, the second is the bump.

4. **Then, and only then, the crate goes to 1.0.0.** Still ahead: it needs Phase 3's second
   quiet cycle, which is 0.15 itself — cut with `SEMVER_BREAKING` empty, and published.

**One thing this does not decide.** `KERNEL_SCHEMA` is `smysl.kernel/0.1` and is a third axis,
independent of both the format string and the crate version — it names the shape of the kernel
fields, and §8 keeps it separate on purpose. Bumping it is a *different* migration with
different consequences, and nothing above requires it. Whether `smysl/1.0` should ship with
`smysl.kernel/0.1` is a decision to take explicitly rather than by symmetry.

**Also updated:** §8.6 of the format spec asked "is `smysl/0.1` frozen?" and answered "no, and
not stable-forever either", resting on the `0.` prefix. It now asks whether the *format* is
frozen and answers with the record: unchanged across fourteen crate releases and four
implementations, which is what `smysl/1.0` reports. §8.2's rule — that a version bump signals
a break — is reconciled there too: this one carries none, so the version marks the format
settled rather than changed, and the compatibility event was teaching the readers in step 1.

**Done.** Readers accept both strings; `versioning.rs` passes with two entries and asserts the
round-trip property rather than a count; a `smysl/0.1` document still comes back declaring
`smysl/0.1`; and one with no version to preserve comes out declaring `smysl/1.0`. The three
outside implementations needed no change at all — they read CBOR, and the wire has no version.

**And it was not a breaking change.** `make semver` is 12/12 clean across both releases that
carried it, which is what let the whole migration happen inside Phase 3's quiet cycles instead
of costing one.

### 0.2 What freezing the seam costs

**The size of the freeze.** *(Measured before §1.2. These are the figures that prompted the
analysis; after the narrowing it is 11 643 items and 49 top-level public modules.)*

The facade's golden file lists **239 names**. The eleven library crates export **12 111 public
items across 52 public modules**, and since they share one version and all must be published
for the facade to resolve, 1.0 freezes the larger number.

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

**C — `#[doc(hidden)]`.** Keep the items reachable, take them out of the contract.

I first wrote this off as prose dressed up as a promise. That was wrong, and measuring settled
it: `cargo-semver-checks` ships lints named `struct_now_doc_hidden` and
`pub_module_level_const_now_doc_hidden`, and their description is *"removing it from the
crate's public API"*. Hiding an item is reported as a **major** break — once, when you hide it
— and the item is outside the contract from then on. `cargo public-api` drops hidden items too:
marking one module in `smysl-provider` took its count from 988 to 958.

So C is not a weaker A. It is B's other half: same effect on the contract, but the item stays
visible to integration tests, which are separate crates and cannot see `pub(crate)`. That
makes it the right tool for exactly the population §1.2 S2 is about, and `pub(crate)` right for
everything reached by nothing at all.

**Recommendation: B and C together, plus closing the gate gap.** The scope is two crates, and it is the same
two the seam review has to read anyway — so do them together: decide the shape, then decide
what stays public. Then regenerate the golden file so the documented surface and the enforced
surface agree, and 1.0 promises one thing rather than two. **§1.2 is that plan.**

Option A is defensible if the appetite for it is low. It should then be chosen out loud, with
the maintenance cost named, rather than by leaving 0.2 undecided.

---

## Phase 1 — make the surface something worth freezing

The engineering core of this plan.

### 1.1 The `#[non_exhaustive]` audit ✅ *done in 0.13.0*

The plan said **72 of 179 public types** lack the attribute. That was measured on the facade;
counted across all eleven library crates and deduplicated by type identity rather than by
re-export path, it is **191 distinct public types, 98 with the attribute and 93 without**.

Of the 93, only 60 were decisions at all. **33 are closed by encapsulation** — 24 structs whose
fields are all private and 9 newtypes — where the attribute changes nothing, because a caller
already cannot write a literal or match exhaustively. That distinction came out of S3, where
`Registry`, `Bm25`, `Semantic` and `Hybrid` turned out to need no attribute for the same reason.

**The argument for the attribute is §8, not taste.** The specification says the crate and
format versions are independent axes, and gives a precedent: *"record type 10 was added in 0.2
without a format bump"*. An exhaustive `UnitCore` or `Relation` turns the next such addition
into a crate major, coupling the two axes the specification separates. The clearest case is
already scheduled — §0.1's migration to `smysl/1.0` must add a field to `ParseOutcome` to carry
the version a document declared, and exhaustive that is a 2.0.

**54 gained the attribute. Six are closed on purpose**, each saying so where it is declared:
`Hlc`, `Date`, `Span`, `Spanned`, `HValue`, `Severity` — shapes that are complete rather than
unfinished. A hybrid logical clock has no fourth component; a byte range is two offsets;
`HValue`'s variants are the JSON data model's rather than this crate's, and callers match on it
exhaustively on purpose.

**Cost: four construction sites**, which is the measurement that says the attribute was cheap.
`Constraints` and `SalienceWeights` are now built from their defaults and adjusted;
`ParseOutcome` likewise; `Detected::new` and `Optimality::new` were added because those two are
built at call sites in three other crates. Public types now stand at 152 with the attribute.

Nine of twelve crates are in `SEMVER_BREAKING` as a result. That is what this cycle is for.

**Done:** every public type has an answer — the attribute, a written reason for being closed,
or no public fields at all. `API_CONTRACT.md` records the rule rather than the list, which is
what the plan asked for.

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

**S1 — give rule A a gate, and watch it fail.** ✅ *done in 0.13.0*

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

The gate is small: an `xtask` check that nothing under `src/` outside `lib.rs` names a sibling
crate. It went into `xtask/src/purity.rs`, whose first line had claimed rules A and B since it
was written and checked only B.

It failed on both sites the first time it ran, which is the whole argument for it. The fixes:

- `from_csv`, `ImportOptions` and `Imported` are now facade re-exports. `Imported` is there
  because it is `from_csv`'s return type — without it the function is callable and its result
  unnameable, which is a bypass in a subtler form.
- `smysl-tui` is re-exported whole, as `smysl::tui`, still behind `--features tui`. Name-by-name
  would have recreated the same gap in miniature: the browser is one capability with several
  entry points, and `render_to_string` is how you test a frame without a terminal.

The facade went from 239 names to 243; the `--no-default-features` surface is unchanged at 199,
since both additions are feature-gated.

This came first because it is the instrument for everything after it. Narrowing without it is
guesswork; with it, "did I take away something the CLI needs" is a command.

**Done:** the check is in `make ci` via `gates`, it passes, and re-introducing a bypass in a
different file was confirmed to fail it — a gate nobody has watched fail is not yet a gate.

---

**S2 — decide what the tests are allowed to see.** ✅ *done in 0.13.0*

Integration tests in `tests/` are separate crates: they see `pub`, not `pub(crate)`. Eight
files reach into modules the narrowing would otherwise close —

`smysl-provider/tests/`: `a2_lazy_runtime.rs`, `status_taxonomy.rs`,
`capabilities_are_honest.rs`, `ollama_live.rs`, `deepseek_live.rs`;
`smysl-ingest/tests/`: `gate.rs`, `providers_live.rs`; and the workspace's
`tests/interactions.rs`.

**What changed the answer.** I had planned to route these case by case — move what could move
inside its crate, keep the rest public with a recorded reason — and had rejected
`#[doc(hidden)]` on the grounds that `cargo-semver-checks` would either flag hidden items as
noise or be told to ignore them. Measuring showed the opposite. It has lints named
`struct_now_doc_hidden` and `pub_module_level_const_now_doc_hidden`, described as *"removing it
from the crate's public API"*; hiding is a major break, once, and the item is out of the
contract afterwards. `cargo public-api` drops hidden items as well.

So the route is uniform, and no test had to move or be weakened:

| hidden | why it is reachable at all |
|---|---|
| `provider::runtime` | A2 — `is_started` asserts the runtime has **not** started, observable only in a fresh process. A unit test cannot see it: inside the crate, something has always started it first, and `runtime.rs`'s own unit test says so. |
| `provider::stream` → later narrowed to `Stream` alone | `ollama_live.rs` drives a real streaming response. Hiding the whole module also hid `StreamMsg`, which is contract — see S6. |
| `provider::map::{anthropic, deepseek, gemini, ollama, openai, openai_compat}` | four tests build the concrete mappers. `build` returns `Box<dyn Provider>`, so no consumer needs them by name. |
| `ingest::prompt` | `gate.rs` asserts against `FENCE` and `content_ingest_json` themselves; asserting against a copy would test the copy. |
| `ingest::quote` | `gate.rs` and `interactions.rs` check a quoted body survives the ingest round trip. |

**Result:** `smysl-provider` 988 → 733 public items, `smysl-ingest` 751 → 696. **310 items out
of the contract**, with `map::build`, `StreamMsg` and every facade re-export untouched, and all
eight test files passing unchanged. Both crates are now in `SEMVER_BREAKING` — hiding is a
break, which is the whole reason it belongs before 1.0 and not after.

What is left for S4 is the other population: modules no test and no sibling crate reaches, for
which `pub(crate)` is right rather than hiding — `provider::http`, `map::auth`, and
`ingest::{chunk, json_ast, monotone, repair, schema}`.

**Done:** every one of the eight files compiles and passes against the narrowed crates, and
each hidden module carries the reason it stayed reachable.

**S3 — the shape review, before the visibility change.** ✅ *done in 0.13.0*

Seventeen seam types read once with "would we regret this at 2.0" in mind. Eleven already
carried `#[non_exhaustive]`. Four more — `Registry`, `Bm25`, `Semantic`, `Hybrid` — turned out
not to need it: every field is private, so external code can neither construct them nor match
them exhaustively. They are closed by encapsulation, which is a better answer than the
attribute and should be recorded as such rather than counted as a gap.

Two findings had substance.

**`status_error` does not belong on `Provider` — and the plan was wrong to say it did.**

The defect was real: five mappers exposed `status_error(&self, u16, &str) -> ProviderError` as
an inherent method, identical in all five, enforced by nothing, which is why
`status_taxonomy.rs` had to hold them as boxed closures. §1.2 as first written proposed moving
it onto the `Provider` trait.

Reading the trait says otherwise. **`Provider` names no HTTP anywhere** — not `http`, not
`status`, not `u16`; it is `id`, `caps`, `complete`, `stream`, `count_tokens`, `probe`. And
three of its eight implementors speak no HTTP at all: `registry::Mock`, ingest's `Fake`, and
`gate.rs`'s `Scripted`. Putting an HTTP-shaped method on it would have forced those three to
implement something meaningless to them, permanently, from 1.0 — the exact regret this step
exists to find, and it would have been introduced *by* this step.

So the shared shape went onto a new `#[doc(hidden)] pub trait StatusMapping` in
`smysl-provider::map`, next to the five HTTP mappers, and `Provider` keeps the generality that
made three non-HTTP implementors possible.

A trait alone still would not have forced anything — a sixth mapper could skip it and compile.
Every mapper reaches a caller through `build`, so `build` boxes through
`fn boxed<P: Provider + StatusMapping>`, which turns the convention into a rule: a mapper that
cannot say what a 401 means does not compile into the registry. Confirmed by removing one
mapper's impl and watching the compiler name the missing trait.

`status_taxonomy.rs` now holds `Box<dyn StatusMapping>` instead of closures — the simplification
the file predicted in its own comments.

**`Hit` was exhaustive and should not have been.**

`Query` carries `#[non_exhaustive]`; `Hit`, three declarations above it, did not — two types
facing each other across the same call, with no reason for the asymmetry. It matters more than
for most output types because `Retriever` is a public trait anyone may implement, so `Hit` is a
type third parties must *construct*: `smysl-embed` builds it at two sites today. A retrieval
result plausibly grows — which field matched, a snippet, an explanation of the score — and an
exhaustive struct could not gain one after 1.0 without a 2.0. Now `#[non_exhaustive]` with
`Hit::new(uid, score)`, and `smysl-retrieve` joins `SEMVER_BREAKING`.

**One thing noted and not changed.** `Retriever` is an unsealed public trait, so any method
added after 1.0 is breaking unless it carries a default — as `is_empty` already does. That is
inherent to public traits rather than a defect in this one; the discipline it implies is
recorded in `API_CONTRACT.md` rather than fixed in code.

**Done:** each of the seventeen has an answer, and the reasons sit where the types are.

**S4 — narrow, one crate at a time, provider first.** ✅ *done in 0.13.0*

`pub(crate)` for what nothing outside reaches: `provider::http`, `map::auth`,
`ingest::{chunk, monotone}`. `#[doc(hidden)] pub` for three more that only an external *test*
crate reaches — `ingest::repair` (`tests/gate.rs`) and `ingest::{json_ast, schema}`
(`smysl-eval`'s `quoting_live.rs`, which sends `batch_schema()` to live providers).

**Result, with S2:** `smysl-provider` 988 → **716**, `smysl-ingest` 751 → **541**. **482 items
out of the contract** across the two crates, and the facade unchanged at 243 throughout —
which is the check that says none of it was anything a consumer had. §0.2 put the
discretionary surface in these two crates at 536 by name-matching; the measurement came in at
520, so the estimate held.

**A correction worth recording.** The first pass at deciding what was unreached grepped for
`smysl_ingest::repair` and found nothing — but `tests/gate.rs` imports it as
`smysl_ingest::{repair, ...}`, which that pattern cannot see. The narrowing therefore broke the
build, which is how it was caught. A brace-aware re-check moved `repair` to the hidden group
and found nothing else missed. Searching for a qualified path is not the same as searching for
a use.

**What narrowing found.** Closing a module lets dead-code analysis see inside it for the first
time. Seven items surfaced, and they were not all the same kind of thing:

- `auth::Secret::is_empty` — no caller anywhere. Deleted. The empty check happens at the call
  site before a `Secret` is constructed, so the method was redundant by construction too.
- `repair::Attempted` and its `is_clean` — never *constructed*, anywhere, including tests. The
  live `is_clean` belongs to `Report`, which is what `check_local` returns. Deleted.
- `Chunk::tokens`, `Window::of`, `Window::without_overlap` — real API whose only callers are
  the module's own tests. Marked `#[cfg(test)]` rather than deleted: `Window::of` is the only
  way to build a window at a chosen budget, and removing it would have cost sixteen tests of
  real chunking behaviour. `#[cfg(test)]` says what is true and makes the compiler object if
  production ever needs one.
- `split_oversized` — **not dead code. A defect.**

**The defect.** `split_oversized` breaks a paragraph too large for any window, and its own
comment says why: *"Refusing would mean one runaway paragraph could fail an ingest, which rule
I forbids."* `chunk` never called it. The grouping loop starts a new group when the *next*
paragraph would overflow, which does nothing about a paragraph already over budget on its own —
it goes into a group regardless.

Measured: one 5 000-token paragraph produced one 5 000-token chunk against a budget of 50. A
hundredfold overshoot, sent to the model as a single request.

It survived because the function was tested and its tests passed. Nothing tested that anything
*used* it — the same shape as rule A in S1, and the second time in this phase that the missing
check was "is this connected" rather than "is this correct".

Two adjacent tests in the same file had been contradicting each other, both green:

- `a_paragraph_larger_than_the_window_is_still_emitted` asserted `chunk` returns exactly one
  chunk — *"one paragraph is one group, however large"*.
- `an_oversized_paragraph_splits_on_lines_then_characters` asserted the opposite of the helper,
  and quotes rule I in its own doc comment while doing it.

Both passed because one tested `chunk` and the other tested `split_oversized`, and nothing
joined them. The first had encoded the defect as the expected behaviour; its emission half was
the part worth keeping, and it now asserts that too.

Now wired in ahead of grouping, with a test asserted on the output of `chunk` rather than on
the helper, and confirmed to fail without the fix.

**A rustdoc trap, for anyone repeating this.** An intra-doc link inside a module that is
`pub(crate)` or `#[doc(hidden)]` has no rendered page to point at, so every one of them breaks
the moment the module closes — nine across the two crates, plus one in the *public* `stage`
module that pointed into the now-private `monotone`. They become code spans.

Worse, one form does not fail cleanly: a `///` doc comment on a `pub(crate) mod` declaration
whose module contains a cross-crate link **ICEs rustdoc 1.94.1** — `no resolutions for a doc
link`, `rustc_metadata/src/rmeta/encoder.rs:2577`. Reported by the compiler as a bug worth
filing. Plain `//` comments on private module declarations avoid it, and lose nothing: rustdoc
does not render private items anyway.

**Done:** both crates build with no dead-code warnings, every still-public module is either
facade-reachable or carries the reason it stayed, and `make ci` is green.

**S5 — make the documented contract equal the enforced one.** ✅ *done in 0.13.0*

§0.2 found the two gates disagreed and assumed the resolution: `make semver` is the real
contract, `tests/public-api.txt` an index of it. Measuring which gate sees what turned that
round.

**`cargo-semver-checks` has the same blind spot as `cargo public-api`, on the facade.** Run
against `smysl` it reports *"no semver update required"* for the 0.12 rename of `Error` to
`AnyError` — although `v0.11.0` exported `smysl::Error` and nothing exports it now. A
cross-crate re-export is a `pub use` line neither tool expands. **The golden file caught that
rename; the semver gate did not**, which is the reverse of the assumption.

So there are three gates with three jobs, now written down in `API_CONTRACT.md`:

| gate | sees | blind to |
|---|---|---|
| `tests/public-api.txt` | every name the facade exports | anything behind a name |
| `make semver` | every item in each library crate, under the real semver rules | the facade |
| `tests/public-api-counts.txt` — new | a crate's surface changing size | an addition and removal that cancel |

The third is eleven lines, one per library crate. Per-item goldens were considered and
rejected again for the reason already recorded in `public-api.txt`: the eleven crates expand
to ~11 600 lines, and a diff nobody reads is decoration. A count is read in five seconds, and
it covers the one case neither other gate does — a public item added by accident, which is
nobody's break and therefore nobody's failure. Confirmed by adding one `pub const` to
`smysl-check` and watching the gate fail with `-242 / +243`.

**`make semver` no longer skips.** A crate in `SEMVER_BREAKING` was `continue`d with a one-line
SKIP, so a crate with one deliberate break had *nothing* watching it and a second, unintended
break would ride along invisibly for the rest of the cycle. Those crates now run, ungated, with
their failures printed to be checked against the reasons recorded beside the list.

**That found a wrong entry on its first run.** `smysl-core` was listed for the `AnyError`
rename and reported no failures, because it never broke — the type there is still `Error`, and
the rename is `pub use smysl_core::Error as AnyError` in the facade's `src/lib.rs`. The entry
names `smysl` now. A skip had been hiding the fact that the list was wrong about which crate.

Everything else matched: `smysl-graph` and `smysl-retrieve` one `struct_marked_non_exhaustive`
each (`SalienceRequest`, `Hit`), `smysl-provider` and `smysl-ingest` eight each from S2 and S4.

**Done:** `API_CONTRACT.md` says which artefact is the contract for what, and each claim in it
was measured rather than assumed.

**S6 — land it as a break.** ✅ *audited in 0.13.0; emptying the list needs publication*

All of S1–S5 goes through `SEMVER_BREAKING`, and the list must be empty before the cut. Empty
is reached only by publishing — a break stops being a break once the version carrying it is the
baseline — so what S6 can finish now is the other half: **proving the list is complete and
correct**, with every reported break mapped to a recorded reason and nothing broken that is not
recorded.

Seven gated crates pass, `smysl-core` among them, which confirms S5's correction that it never
broke. The five listed report:

| crate | reported | recorded reason |
|---|---|---|
| `smysl-graph` | `SalienceRequest` marked `#[non_exhaustive]` | as recorded |
| `smysl-retrieve` | `Hit` marked `#[non_exhaustive]` | S3, as recorded |
| `smysl-provider` | 8 lints: `http` and `map::auth` gone (module, 11 fns, 2 consts, 2 structs); `runtime`, `Stream`, the mappers hidden | S2 and S4, as recorded |
| `smysl-ingest` | 8 lints: `chunk` and `monotone` gone; `quote`, `prompt`, `repair`, `schema`, `json_ast` hidden | S2 and S4, as recorded |
| `smysl` | *nothing* — `cargo-semver-checks` cannot see through `pub use` | the `AnyError` rename, caught by `api-check` instead |

**The audit found one thing wrong, and it was mine.** S2 hid the whole `stream` module and
recorded that *"`StreamMsg` stays contract via the root `pub use`; only the module path went"*.
Half true. `cargo public-api` agreed — it sees the type through the root re-export and reported
no change. `cargo-semver-checks` reported `enum_now_doc_hidden` on `StreamMsg`, which is
removal from the API. The two gates disagreed about one type, and a disagreement between gates
is the answer being wrong, not a tie.

`StreamMsg` is contract: the facade exports it and `Provider::stream` takes a channel of it.
Hiding it would have quietly dropped a name from the 243 while the golden file said nothing.
Now `Stream` alone is hidden and the module is public; `Emitter` went `pub(crate)`. The
provider surface is 716 rather than 678 — the cost of getting it right — and the count golden
is what showed the 38-item move.

A cross-check of all 243 facade exports against every `*_missing` and `*_doc_hidden` failure
found no others. **The lesson generalises: hide the item, not the module, whenever the module
holds anything the facade exports.**

**Done:** every break is recorded and every recorded break is real. `SEMVER_BREAKING` names
five crates and empties when 0.13.0 is published, which is a release decision rather than an
engineering one.


---

## Phase 2 — the verification a 1.0 should not ship without

Not "fix everything". These three, because each is a place where the project cannot presently
say what it knows.

### 2.1 Make the CLI measurable, then measure it ✅ *done in 0.13.0*

`make doc-output` replays 46 of the manual's 168 documented transcripts against the built
binary. It has been a good gate since 0.3 — it caught the 34 that went stale when `check`
changed what it reports — but no `cargo test` invoked it, so nothing that *counts* coverage
could see it. `tests/doc_output.rs` runs it now, against `CARGO_BIN_EXE_smysl`, which is the
binary `cargo-mutants` rebuilds with each mutation applied.

**The measurement was a controlled two-run experiment**, because the 73.6% recorded in 0.12 was
an `--all-features` run and the new test only compiles under default features — the manual
documents a default build. Comparing across feature sets would have measured the wrong thing.
Both runs use default features and differ only in whether the test file is present.

| | mutants | caught | missed | viable | **survivors** |
|---|---|---|---|---|---|
| **A** — suite as it was | 302 | 75 | 193 | 268 | **72.0%** |
| **B** — with the doc-output test | 302 | 96 | 172 | 268 | **64.2%** |

**21 newly caught, 0 newly missed.** A's 72.0% against 0.12's `--all-features` 73.6% says the
earlier figure was not an artefact of the feature set.

**The answer is neither of the two the plan anticipated.** It expected either a collapse — the
CLI better tested than it looked, the finding being about measurement — or barely any movement,
in which case four tests across 3 600 lines was the whole story. 7.8 points is a real
improvement, the largest single contribution any one test makes to the CLI, and it leaves 172
survivors. Both readings are partly right: the measurement did understate the CLI, and the CLI
is still the least-tested thing in the workspace.

What the 21 are is the more useful output. They span nine `cmd_*` functions — `cmd_check`,
`cmd_diff`, `cmd_trace`, `cmd_retract`, `cmd_pack`, `cmd_salience`, `cmd_thread` — plus `main`,
`cli` and `load_registry`. One is `replace emit_pack_surface -> String with String::new()`:
replaying the manual catches an entire output function being emptied, which no other test did.

**Where the remaining 172 sit**, which is Phase 2.2's brief:

| file | survivors | concentration |
|---|---|---|
| `src/main.rs` | 121 | `cmd_providers` 12, `cmd_fmt` 12, `cmd_merge` 11, `main` 9, `cli` 8 |
| `src/progress.rs` | 51 | `Bar::draw` 23 — unchanged, and still arithmetic under twelve tests that check structure and never numbers |

**Two defects found in the wiring, both of the kind this phase keeps producing.**

The first version of `tests/doc_output.rs` **passed while the binary's output was changed**. It
passes an absolute `CARGO_BIN_EXE_smysl`; the script substitutes that into each command and
then scans the tokens for absolute *input* paths, which are narrative state it cannot replay —
so the program itself matched the rule and all 168 transcripts were skipped. It printed
`ran 0, skipped 168, MISMATCHED 0` and the test asserted only that a summary line existed.

Both halves are fixed: the script skips token 0, and the test asserts `ran >= 40`. **A gate
that cannot tell "nothing was wrong" from "nothing was checked" is the failure this repository
keeps rediscovering** — this is the fourth instance in 0.13 alone.

**Done:** the doc-output replay is inside `cargo test`, the number has been read, and the
control — changing the binary and watching the test fail — was run before trusting it.

### 2.2 Whatever 2.1 reveals ✅ *done in 0.13.0*

2.1 left 172 survivors: 51 in `src/progress.rs`, 121 in `src/main.rs`.

| file | before | after |
|---|---|---|
| `src/progress.rs` | 51 | **1** |
| `src/main.rs` | 121 | **109** |

**`progress.rs`: the cause was not missing tests.** Every decision was welded to the
environment or to `stderr`, so nothing could be observed. The twelve tests all used
`Style::silent()` because that was the only thing available to them. Three separations fixed
it — `Style::decide(tty, quiet, json, no_color)` split from reading `is_terminal()`;
`render(done, total, label, color) -> (String, usize)` split from writing it; and a `Sink`,
shared, so `finish` and `abandon` can be tested *taking `self`*, which is how callers use them.

**Two of the 51 were defects rather than untested code.**

`advance` clamped with `(done + n).min(total.max(done + n))`. Since `y.max(x)` is never below
`x`, that is `x` — a no-op, verified over 200 000 random triples before it was changed. A
caller that overshot printed `105/100`. The existing test was called
`advancing_past_the_total_does_not_panic`: it asserted no panic and never looked at the result.

`draw` ended with `self.width = printed.max(self.width.min(printed + pad));` immediately
followed by `self.width = printed;`. Dead, and the two mutants on its arithmetic survived
because a discarded value cannot be observed.

Tests went 12 → 43, and the new ones assert numbers: filled cells across the fraction, that
the recorded width equals the *visible* width with colour on and off — the invariant `clear`
depends on — and that a tick inside the rate-limit interval does **not** repaint while the last
step always does.

**The one survivor left is unreachable, not unfixed**, and says so where it is: the `||` in
`Style::detect` reading `NO_COLOR` only matters when stderr is a terminal, which a test process
never has. Recorded the way 0.12 recorded `support_cycles`.

**`main.rs`: the pure helpers, which is what is reachable without driving the binary.**
`looks_like_surface`, `root_beside` (extracted from `project_root` and `project_file`, whose
shared `!` was untestable behind an `ArgMatches`), `read_input`, `finish_over` — 11 survivors
killed, and one more explained.

`worse`'s `>` survives mutation to `>=` and is an **equivalent mutant**: `ExitCode` is a
fieldless enum with distinct discriminants, so equal codes are the same variant and returning
either is the same answer. Tested anyway, so the survivor is a decision on the record.

**One test found the code right and my expectation wrong**, which is worth keeping. I asserted
that an indented `#` is a comment; `looks_like_surface` says otherwise, and so does `lex.rs`:
*"Column 0 only. An indented `#` is inside a body or a step, where it is prose."* A heuristic
that disagreed with the lexer would classify as surface a document the parser then rejects. The
test now asserts the rule rather than my guess at it.

**What is left, and why it is a different job.** The remaining 109 are in the `cmd_*` functions
— `cmd_providers` 12, `cmd_fmt` 12, `cmd_merge` 11, `main` 9, `cli` 8. Those are command bodies
that read a filesystem, build a store and print; reaching them means driving the binary, as
`tests/global_flags.rs` does, rather than calling a function. That is a body of work in its own
right and is not a prerequisite for 1.0 — the format, the library and the gates are what 1.0
freezes, and the CLI's remaining survivors are display and control flow in a binary that
`doc_output` now replays 46 documented transcripts against.

**Done:** the two defects are fixed, the tractable survivors are dead, and the ones that remain
are either recorded as unreachable, recorded as equivalent, or scoped as `cmd_*` work.

### 2.3 Provider acceptance

Gate 4's remaining item, and the only one with a hard external dependency: whether OpenAI and
Anthropic **accept the translated schema**. Unreachable by reading — and reading has now found
two defects without a key, so this is genuinely the residue.

The 0.12 mutation run reframed what "verified" has meant here: the failure taxonomy was
untested on all five mappers, including the three exercised live, because nobody provokes a 401
against a real endpoint. `tests/status_taxonomy.rs` closed that. What a key adds is narrower
than gate 4 has implied, and it is still needed.

---

## Phase 3 — evidence of stillness ✅ *satisfied by 0.14.0 and 0.15.0*

The part that could not be rushed, and the only real gate. **Two consecutive cycles ended with
`SEMVER_BREAKING` empty at the cut, and both are published.**

| | `SEMVER_BREAKING` at the cut | `make semver` | published |
|---|---|---|---|
| 0.13.0 | nine crates | 12/12 after the fact | yes — *cycle zero* |
| **0.14.0** | **empty** | **12/12 clean** | **yes** |
| **0.15.0** | **empty** | **12/12 clean** | **yes** |

0.13 was cycle zero: the largest deliberate break in the project's history, and the right shape
for the cycle before 1.0 because it was the last one in which narrowing was free. The two that
follow it are the evidence.

### What the two cycles actually carried

Not nothing — that would prove only that nobody worked. Both carried real change and neither
broke anything, which is the harder and more useful claim.

**0.14** taught every reader `smysl/1.0` while writing nothing new, stopped `write_surface`
relabelling documents, and closed the half of gate 4 that never needed a key — finding on the
way that the schema its strict-mode translation was tested against had 2 of the 13 kernel types
and 2 of the 5 statuses, while the code documented it as "the full Appendix C schema rather
than a miniature of it".

**0.15** made `smysl/1.0` what new documents declare, one release after the readers, which is
the ordering §8.2 requires and the reason the flip could not simply be done when it was ready.

### Why the format migration did not cost a cycle

This was the open question when Phase 3 was written, and the answer is §1.1's doing.
`ParseOutcome` and `WriteContext` were made `#[non_exhaustive]` in 0.13, so each could gain the
field the migration needed without a major bump, and `write_surface`'s signature never moved.
`make semver` stayed 12/12 across both releases that carried it.

An audit whose payoff arrives two cycles later, in a form nobody predicted when it was done, is
the most that can be asked of one.

### What the gate is watching now

`BASELINE` is 0.15.0 — a version that is actually on the registry, which through 0.12 and 0.13
it was not. `SEMVER_BREAKING` is empty and every one of the twelve crates is checked rather
than skipped. That is the difference between a stillness gate and the appearance of one, and it
took 0.13's S5 to notice the difference existed.

**Done.** Two cuts in a row, both on crates.io, both with an empty list, and `make semver`
reporting no failures rather than no checks.

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

The **surface** is ready. Phase 1 finished in 0.13: 482 items out of the contract with the
facade's 243 untouched, 152 of 191 public types carrying `#[non_exhaustive]` and the other 39
each with an answer, three gates that each know what they are blind to, and rule A checked
instead of asserted.

The **verification** is as far as it goes without keys. The CLI's survivors went 172 → 110,
`progress.rs` from 51 to 1, and the manual's 46 replayed transcripts are inside `cargo test`
where the tool that measures coverage can see them. What is left is `cmd_*` work in the binary
and §2.3, which needs an OpenAI and an Anthropic key and nothing else.

**What that work actually found is worth stating plainly, because it is not what "make the
surface worth freezing" sounds like.** Rule A was stated in two places and enforced in none.
`split_oversized` was written, tested and never called, while a test asserted the defect it
prevents. `status_error` was a contract shared by convention. `SEMVER_BREAKING` named a crate
that had not broken. `cargo-semver-checks` reported no change on the facade for a rename that
removed a published name. A clamp that never clamped. A doc-output test that passed while the
binary's output was wrong. **In every case but two the code was fine and the check was
missing** — and the two exceptions had survivors sitting on them for two releases.

The **evidence** needs Phase 3, and Phase 3 cannot be hurried: it is two quiet cycles, and the
only way to get them is to have them. 0.13 is cycle zero — it breaks nine of twelve crates,
deliberately, because this is the last cycle in which narrowing is free. **The clock starts
when it is published**, and publishing is a release decision rather than an engineering one.
