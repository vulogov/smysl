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

1. **No breaking change to any exported name without a 2.0.** All 239 names at
   `--all-features`, all 199 at `--no-default-features`. Including the seam.
2. **The format stays readable.** A 1.0 reader reads every document a 1.x writer produces.
3. **The guarantees hold.** A1–A6 and rules M, T, L, R, U, I, S, V1, V2, X, D, P are part of
   the contract, and A5 already says making an operation non-reproducible is breaking whatever
   the signature does.

What 1.0 does **not** commit to: that everything is finished, that every survivor is dead, or
that the CLI is beautiful. It commits to not moving.

---

## Phase 0 — two decisions

Neither is work. Both block the phases after them, and neither is mine to make.

### 0.1 The format version

The crate goes to 1.0. `smysl/0.1` is the wire format, and §8 says the two axes are
independent — so `smysl 1.0.0` shipping `format smysl/0.1` is *coherent*, and looks odd on a
title page.

The alternative, bumping to `smysl/1.0`, is not free. §8.2 reserves a format bump for breaks;
bumping without one means every reader must be taught to accept both strings, and there are
four of them now — the Rust, `python/`, `nodejs/`, `go/`. It is a compatibility event in
service of a cosmetic alignment.

**Recommendation: keep `smysl/0.1`.** The document already explains why the numbers differ,
and §8.6 exists to be pointed at. Revisit when the format actually changes.

### 0.2 What the seam costs

Stabilising bucket 2 means freezing shapes that have moved recently. `Hybrid` changed twice
inside 0.7. `Retriever` is one cycle old. `ProviderConfig`, `Request`, `Completion`, `Usage`,
`Capabilities`, `StructuredMode` become permanent.

The honest question is not "can we freeze them" — `#[non_exhaustive]` makes most of it
survivable — but "would we regret the *shape*". A trait with the wrong method set cannot be
saved by an attribute. Phase 1.2 is where that gets looked at, and if the answer for some type
is *yes, we would regret it*, then it is better broken now than frozen wrong.

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

### 1.2 The seam review

`ProviderConfig`, `Request`, `Completion`, `Usage`, `Capabilities`, `StructuredMode`, `Probe`,
`Provider`, `Registry`; `Bm25`, `Hybrid`, `Semantic`, `Query`, `Hit`, `Retriever`; `Ir`,
`Profile`, `BuildOptions`.

One concrete defect is already known and belongs here: **`status_error` is a shared contract
shared by convention.** All five mappers expose `status_error(u16, &str) -> ProviderError` with
the same signature, and it is an inherent method on each rather than part of the `Provider`
trait — which is why `tests/status_taxonomy.rs` has to reach for boxed closures to test them
together. Before 1.0 it should be on the trait, where the compiler enforces the shape a sixth
mapper must have.

**Done when:** each seam type has been read once with "would we regret this at 2.0" in mind,
and the answers are in `API_CONTRACT.md`.

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

The **surface** needs Phase 1 — 72 type decisions and one honest look at the seam.

The **evidence** needs Phase 3, and Phase 3 cannot be hurried: it is two quiet cycles, and the
only way to get them is to have them.
