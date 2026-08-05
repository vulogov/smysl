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
| `provider::stream` | `ollama_live.rs` drives a real streaming response. `StreamMsg` stays contract via the root `pub use`; only the module path went. |
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

**Result, with S2:** `smysl-provider` 988 → **678**, `smysl-ingest` 751 → **541**. **520 items
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
