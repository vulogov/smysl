#import "design.typ": *

#part(number: "II", title: "Creating and Formatting Documents")

#chapter(number: 6, title: "Writing Surface Syntax, With Room to Get It Wrong")

Chapter 5 got you to a two-unit file that passed `check` on the first try —
a `.smy` file small enough that nothing in it could plausibly be wrong. Real
documents are not like that. You will write a claim before its evidence
exists, forget that `derived` demands `grounds`, indent a line by the wrong
amount, or reach for a unit you meant to cite but never declared. This
chapter is built around that fact: every construct below is introduced
once, correctly, and then broken on purpose, so the
diagnostic that catches the mistake is something you have already seen with
your own eyes before it ever surprises you mid-sentence.

The exhaustive reference — every field, every type, one example each — is
`SMYSL_FORMAT_GUIDE.typ`. This chapter does not repeat it. What it adds is
the experience the reference guide deliberately leaves out: what it feels
like to get one of these wrong, and exactly what the tool says back.

#callout(label: "Why")[
  It is 02:40, the incident is live, and you are typing evidence into a file
  while a second person keeps talking. You will refer to `e/pool-wait` two
  minutes before you declare it, because that is the order the conversation
  happened in. You will write `derived` on a claim whose grounds you fully
  intend to fill in as soon as somebody finds the dashboard link.

  Both are correct things to do while thinking and dangerous things to leave in
  a document somebody else will act on tomorrow. The format is deliberately
  strict about each of them — and strictness only helps if the message you get
  back tells you *which* mistake you made, in a file you are still holding in
  your head. That is what this chapter is for: to make every one of those
  diagnostics something you have already seen on purpose, in daylight, before
  you meet one at 02:40.
]

#section("What you hand-author")

Four kinds of record are ever typed by a person. Two more — `attestation`
and `contention` — exist in the format, but tooling stamps them on for you
(`attest`, `merge`); nothing in this chapter asks you to write either by
hand.

#dtable(
  (auto, 1fr),
  (
    ([Construct], [What it declares]),
    ([`@doc`], [The document header — at most one per file, and optional.]),
    ([a unit], [`@claim`, `@evidence`, and eleven more kernel types — the
      thing being claimed, cited, defined, or asked.]),
    ([`@rel`], [A typed, directed edge between two units — `causes`,
      `rebuts`, `warrant`, and eleven more.]),
    ([`@thread`], [A named, ordered, role-annotated walk over units already
      in the document.]),
  ),
)

#callout(label: "Note")[
  For the full field list of each construct — every status, every source
  kind, every relation kind, every thread schema and role — see
  `SMYSL_FORMAT_GUIDE.typ`. It is organised as a specification; this chapter
  is organised as a sequence of decisions you make while writing.
]

#section("Building a document, unit by unit")

The scenario: checkout latency spiked, and you suspect a price cache went
cold after an eviction. You will build the whole argument as you would in
an editor, running `check` after every addition.

#subsection("One piece of evidence")

Start with the reading that kicked off the investigation:

```
@evidence e/cache-miss { status: measured, source: { kind: metric, ref: "cache.miss_rate{svc=checkout}", captured: 2026-07-20 } }
~ Cache miss rate on checkout rose from 1% to 42% over the same hour.
```

#screen(caption: "$ smysl check step1.smy")[
```
step1.smy: 2 records, 1 units, 0 diagnostic(s)
```
]

#callout(label: "Why")[
  `measured` is the rung reserved for something an instrument recorded, and
  the format ties it to a `source` immediately rather than letting you add
  one later "when you get to it" — a measured reading with nowhere it
  grounds out externally is not yet a measured reading, whatever the status
  field says. You will see the diagnostic for skipping this in a few pages.
]

#subsection("A claim that leans on it")

Add the claim the evidence is for:

```
@claim c/cache-cold { status: inferred, grounds: [e/cache-miss] }
~ The checkout price cache was evicted and is serving cold.
```

#screen(caption: "$ smysl check step2.smy")[
```
step2.smy: 4 records, 2 units, 0 diagnostic(s)
```
]

#callout(label: "Why")[
  `inferred` requires `grounds` because an inference with nothing named as
  its basis is indistinguishable from a guess wearing a confident label.
  Rule M then pins the ceiling: this claim can never outrank
  `e/cache-miss` — retract the evidence, and the claim is reopened along
  with it, mechanically, not by someone remembering to revisit it.
]

#subsection("Naming a term so later claims can rely on it")

"Cold" needs a fixed meaning before anything downstream leans on the word:

```
@definition d/cold-cache { status: cited, source: { kind: doc, ref: "sre-handbook#cache-eviction" } }
~ A cache is "cold" when its hit rate has not yet recovered after an eviction event.
```

Wire it into the claim as a `deps` entry — the claim's *wording* depends on
this definition to be read correctly, even though its *truth* rests on the
evidence, not on the definition:

```
@claim c/cache-cold { status: inferred, grounds: [e/cache-miss], deps: [d/cold-cache] }
~ The checkout price cache is cold after an eviction, not merely slow.
```

#screen(caption: "$ smysl check step3.smy")[
```
step3.smy: 6 records, 3 units, 0 diagnostic(s)
```
]

#callout(label: "Why")[
  `deps` and `grounds` answer two different questions, and the format keeps
  them separate on purpose: `grounds` is what the claim would fall without;
  `deps` is what a reader needs already in hand to parse the claim's
  wording at all. A definition rarely makes something *true*, but it very
  often makes a sentence *mean* something specific instead of something
  vague.
]

#subsection("A competing line of evidence, and two more claims")

Real incidents have more than one thread. Add a second reading, a claim
about latency, and a claim the second reading supports:

```
@evidence e/deploy-clean { status: measured, source: { kind: metric, ref: "deploy.canary_p95", captured: 2026-07-20 } }
~ The same-hour checkout canary deploy shows no latency regression on its own.

@claim c/latency-up { status: derived, grounds: [e/cache-miss] }
~ Checkout p95 latency rose from 80 ms to 410 ms in the same hour.

@claim c/deploy-clean { status: derived, grounds: [e/deploy-clean] }
~ The canary deploy rules out the new release as the direct cause.
```

`c/latency-up` is `derived`, not `inferred` — the rise is read straight off
the metric by a deterministic procedure, no model judgement involved, and
the format has a separate rung for exactly that distinction.

#subsection("Relating them: causes and rebuts")

The argument isn't complete until the units are connected. `causes` says
what produced what; `rebuts` records the objection that argues against the
cache theory, sitting right next to the claim it argues with rather than in
a review thread nobody reopens:

```
@rel c/cache-cold --causes--> c/latency-up
@rel c/deploy-clean --rebuts--> c/cache-cold { weight: 0.4 }
```

#screen(caption: "$ smysl check checkout.smy")[
```
checkout.smy: 14 records, 6 units, 0 diagnostic(s)
```
]

The complete file, for reference — six units, two relations, built up
exactly as above:

```
@evidence e/cache-miss { status: measured, source: { kind: metric, ref: "cache.miss_rate{svc=checkout}", captured: 2026-07-20 } }
~ Cache miss rate on checkout rose from 1% to 42% over the same hour.

@evidence e/deploy-clean { status: measured, source: { kind: metric, ref: "deploy.canary_p95", captured: 2026-07-20 } }
~ The same-hour checkout canary deploy shows no latency regression on its own.

@definition d/cold-cache { status: cited, source: { kind: doc, ref: "sre-handbook#cache-eviction" } }
~ A cache is "cold" when its hit rate has not yet recovered after an eviction event.

@claim c/cache-cold { status: inferred, grounds: [e/cache-miss], deps: [d/cold-cache] }
~ The checkout price cache is cold after an eviction, not merely slow.

@claim c/latency-up { status: derived, grounds: [e/cache-miss] }
~ Checkout p95 latency rose from 80 ms to 410 ms in the same hour.

@claim c/deploy-clean { status: derived, grounds: [e/deploy-clean] }
~ The canary deploy rules out the new release as the direct cause.

@rel c/cache-cold --causes--> c/latency-up
@rel c/deploy-clean --rebuts--> c/cache-cold { weight: 0.4 }
```

#section("Eight ways to get it wrong")

Everything above worked because every rule was already respected. The rest
of this section breaks the same kind of file eight distinct ways, one rule
at a time, and shows the exact diagnostic `check` gives back — not a
paraphrase of `--help`, the real text.

#subsection("Missing gist")

```
@claim c/cache-cold { status: speculative }
```

#screen(caption: "$ smysl check missing-gist.smy")[
```
missing-gist.smy: error: SMY-E021: record ends before its gist (at 20..43)
```
]

#callout(label: "Why")[
  The gist is the one thing every unit guarantees a reader at L0 — the
  level a packed or budget-constrained view falls back to when nothing else
  survives. A unit with no gist has nothing to fall back to, so the parser
  refuses it outright rather than let a placeholder stand in for meaning
  that was never written.
]

*Fix*: add the line the header is missing, immediately below it — `~ The
checkout price cache is cold.`

#subsection("Detail without a body")

```
@claim c/cache-cold { status: speculative }
~ The checkout price cache looks cold.

--
A longer breakdown of the eviction timeline.
```

#screen(caption: "$ smysl check detail-no-body.smy")[
```
detail-no-body.smy: error: SMY-E023: SMY-E023: detail without body (at 0..131)
```
]

#callout(label: "Why")[
  L2 detail exists to go *further* than L1 body — it is the fine grain a
  reader reaches for after the ordinary account, not a substitute for one.
  A detail with nothing under it is a level with no level beneath it to
  extend, so the format treats it the same as a floor with no ground under
  it: not allowed to stand alone.
]

*Fix*: either add a body paragraph above the `--` line, or delete the
detail if there was never an ordinary account to extend.

#subsection("`derived` with empty grounds")

```
@claim c/latency-up { status: derived }
~ Checkout p95 latency rose from 80 ms to 410 ms.
```

#screen(caption: "$ smysl check derived-empty-grounds.smy")[
```
derived-empty-grounds.smy: error: SMY-E031: SMY-E031: derived/inferred with empty grounds (at 0..89)
```
]

#callout(label: "Why")[
  `derived` and `inferred` are both claims about *how* the unit came to be
  believed — computed, or reasoned — and both claims are empty without
  something named as the input to that process. An empty `grounds` list
  under either status is a status describing a computation with no
  operands.
]

*Fix*: add `grounds: [e/cache-miss]`, or drop to `status: speculative` if
nothing grounds it yet.

#subsection("`measured` without a source")

```
@evidence e/cache-miss { status: measured }
~ Cache miss rate on checkout rose sharply.
```

#screen(caption: "$ smysl check measured-no-source.smy")[
```
measured-no-source.smy: error: SMY-E032: SMY-E032: measured/cited without source (at 0..87)
```
]

This is the rule the first example in this chapter was built to satisfy
from the start — this is what skipping it looks like.

#callout(label: "Why")[
  `measured` and `cited` both ground out *externally*: an instrument, or a
  document a reader can open. Without `source`, the claim to have grounded
  out anywhere is unverifiable — there is nothing for a later reader, or
  `attest`, to go check.
]

*Fix*: add `source: { kind: metric, ref: "cache.miss_rate" }` — pick the
`kind` that matches where the number actually came from.

#subsection("`unfounded` authored directly")

```
@claim c/cache-cold { status: unfounded }
~ The checkout price cache is cold.
```

#screen(caption: "$ smysl check unfounded-authored.smy")[
```
unfounded-authored.smy: error: SMY-E034: SMY-E034: unfounded authored (at 0..77)
```
]

#callout(label: "Why")[
  `unfounded` is the bottom rung of the trust ladder — reachable only by
  *retracting* something that used to stand higher. Writing it directly
  would let a unit start life already withdrawn, which is not a thing an
  argument can coherently do. If a unit belongs at the bottom, it belongs
  there because something above it was pulled out, not because you typed
  the word.
]

*Fix*: use `speculative` if this is a fresh, ungrounded claim — that is the
actual floor for anything authored.

#subsection("A body reaching for a unit it never declared")

```
@evidence e/cache-miss { status: measured, source: { kind: metric, ref: "cache.miss_rate" } }
~ Cache miss rate on checkout rose sharply.

@evidence e/deploy-clean { status: measured, source: { kind: metric, ref: "canary.p95" } }
~ The canary deploy shows no regression on its own.

@claim c/cache-cold { status: inferred, grounds: [e/cache-miss] }
~ The checkout price cache is cold.

The miss-rate curve and the checkout latency curve track each other closely
enough over the same window that the direction is not seriously in doubt,
and this reading tracks closely with e/deploy-clean, which this claim never
listed as a dep or a ground anywhere in its header above.
```

#screen(caption: "$ smysl check body-not-declared.smy")[
```
body-not-declared.smy: error: SMY-E020: the body references b3:aman2pifslep4jh4umauo5zfu7, which is neither a dep nor a ground (at b3:cxzv6o3tbghkhpvvzspo67fd5q) [try: add it to `deps` if it is a prerequisite, or `grounds` if it is support]
```
]

#callout(label: "Why")[
  Rule L says a body should be interpretable from the L0 of whatever it
  leans on — `deps` and `grounds` together. A body that reaches for a unit
  by name without listing it either place is asking the reader to already
  know something the header never promised them. `check` only catches this
  *structurally* — an explicit `e/deploy-clean` token, or a `b3:` hash, in
  the body text — not every English way of alluding to another unit; that
  softer, semantic version of the same question is what `attest` is for.
]

*Fix*: list the referenced unit — `deps: [e/deploy-clean]` here, since the
body leans on it for context rather than for grounding the claim's truth.

#callout(label: "Note")[
  The line that becomes a *body* has to start at column 0. A continuation
  line indented by exactly two spaces is read as more *gist*, not body —
  easy to get backwards when you're used to indenting for readability.
  Compare:
  ```
  @claim c/cache-cold { status: inferred, grounds: [e/cache-miss] }
  ~ The checkout price cache is cold.
    This looks like a body, but two-space indent makes it a gist continuation.
  ```
  `smysl fmt` on this file folds the second line straight into the gist —
  `~ The checkout price cache is cold. This looks like a body, but
  two-space indent makes it a gist continuation.` — one long sentence, not
  two levels. A real body needs a blank line after the gist, then text
  starting at column 0.
]

#subsection("A dangling label reference")

```
@claim c/cache-cold { status: inferred, grounds: [e/cache-miss] }
~ The checkout price cache is cold.
```

Nothing in this file defines `e/cache-miss` at all — the label was typed,
or the unit that defines it was deleted, or it lives in a different file
that was never passed to `check` alongside this one.

#screen(caption: "$ smysl check dangling-label.smy")[
```
dangling-label.smy: error: SMY-E060: unresolved reference `e/cache-miss` (at 50..62)
dangling-label.smy: error: SMY-E031: SMY-E031: derived/inferred with empty grounds (at 0..101)
```
]

Notice the second line: once the dangling reference is dropped, `grounds`
is empty, and `inferred` with empty grounds is its own separate violation.
One root cause, two diagnostics — fix the label and both disappear together.

#callout(label: "Why")[
  A label that resolves to nothing is worse than a typo the parser could
  guess past: guessing wrong here would silently point a claim at the
  wrong unit. `check` refuses to guess, drops the unresolved reference, and
  reports exactly what it dropped — which is also why the knock-on
  diagnostic above appears; the tool is being honest about what the file
  actually contains after the bad reference is removed, not what you meant.
]

*Fix*: define `e/cache-miss` in this file, or pass every file that shares
labels to `check` together.

#subsection("A cycle in grounds")

```
@claim c/cache-cold { status: inferred, grounds: [c/latency-up] }
~ The checkout price cache is cold.

@claim c/latency-up { status: inferred, grounds: [c/cache-cold] }
~ Checkout latency rose because the cache is cold.
```

#screen(caption: "$ smysl check cycle.smy")[
```
cycle.smy: error: SMY-E061: cycle in deps or grounds; the back edge is dropped (at 0..102)
cycle.smy: error: SMY-E061: cycle in deps or grounds; the back edge is dropped (at 103..219)
cycle.smy: error: SMY-E031: SMY-E031: derived/inferred with empty grounds (at 0..102)
cycle.smy: error: SMY-E031: SMY-E031: derived/inferred with empty grounds (at 0..102)
cycle.smy: error: SMY-E031: SMY-E031: derived/inferred with empty grounds (at 103..219)
cycle.smy: error: SMY-E031: SMY-E031: derived/inferred with empty grounds (at 103..219)
```
]

The cycle is reported once from each unit's own traversal, and each unit's
now-empty `grounds` is reported once for each of those traversals too — six
lines from one mistake, all pointing at the same two units.

#callout(label: "Why")[
  Rule M walks `grounds` to find the *weakest* thing a status rests on, and
  that walk has to terminate. Two claims each grounding the other have no
  base case — no unit either of them could actually be weaker than — so
  the graph cannot be evaluated, only diagnosed. `check` breaks the cycle by
  dropping the back edge rather than looping forever, which is exactly why
  `grounds` comes back empty on both sides afterward.
]

*Fix*: this pair is circular reasoning stated as syntax, and the real fix
is to find the actual, non-circular ground for one of them — typically new
evidence, not just a rearranged edge.

#section("Annotating what you are writing")

#callout(label: "Why")[
  You are reading somebody else's incident document at 09:00 and you want to
  leave a note for them — *check this against the deploy log* — without asserting
  it as a claim. A note is not a unit: it has no status, nothing grounds it, and
  it should not survive into a rendered brief. It is a message to a person.
]

A line beginning `#` or `//` at column 0 is a comment. Both markers, because an
HJSON header inside a record already accepted both, and the surface used to
contradict itself by rejecting between records what it took within one.

```
# Checked against the deploy log; the 4.2 timing lines up.
@claim c/regression { status: derived, grounds: [e/trace] }
~ p95 auth latency tripled after the 4.2 rollout.

// TODO: get a link for the canary run before this goes out.
@evidence e/canary { status: measured, source: { kind: metric, ref: "canary.p95" } }
~ The 4.2 canary ran the same pool configuration without the regression.
```

Two things to know, and the second is a real limitation rather than a detail.

#subsection("A comment is a comment wherever it sits")

Including inside a body. That costs you the ability to open a body line with `#`
or `//` — a Markdown heading in a body is read as a comment and dropped.

The alternative was worse. A body runs from the gist to the next record, so a
comment sitting *between* two records falls inside that range; treating it as
prose made the comment become the previous unit's body. Content invented out of
a note, with a granularity warning fired about the invented content. A rule that
depends on how far a line happens to be from the next record is not a rule
anybody can hold in their head.

#subsection("`fmt` cannot keep them, and says so")

A comment is not part of any record, so canonical form has nowhere to put one.

#screen(caption: "$ smysl fmt --write draft.smy")[
```
draft.smy: warning: 2 comment line(s) are not part of any record and will not
survive formatting
```
]

That warning exists because this book recommends `fmt --write` as a pre-commit
habit, and silently deleting a reviewer's notes is the difference between a
formatter and a hazard. If a note has to survive, it belongs in a unit — a
`@question`, or a body paragraph — where it is content and travels like content.

#whatsnext[
  Comments are for the humans in the chain. If what you want to record is
  something the *graph* should know — an open question, a caveat, a
  disagreement — Chapter 6's schemas and `@rel` are where it goes, and it will
  then survive a merge, a pack and a render like anything else.
]

#section("Reaching past the kernel: extension types")

The thirteen kernel unit types cover claims, evidence, definitions, and the
rest of general-purpose reasoning — but an SRE team running its own runbook
process may want a unit type the kernel was never going to add, without
waiting on a kernel RFC revision to get one. That is what
`x.<domain>/<type>` is for:

```
@x.sre/runbook-step rb/rollback { status: cited, source: { kind: doc, ref: "ops-runbook#eu-west-rollback" } }
~ Roll the eu-west pool config back to the 4.1 settings and re-run the canary.
```

#screen(caption: "$ smysl check runbook.smy")[
```
runbook.smy: 2 records, 1 units, 0 diagnostic(s)
```
]

It checks clean — an extension type still carries the ordinary unit
anatomy (status, gist, `source`) and is bound by the same rules as any
kernel type. Relations get the same escape hatch:

```
@rel c/cache-cold --x.sre/mitigates--> p/warm-cache
```

#screen(caption: "$ smysl check extrel.smy")[
```
extrel.smy: warning: SMY-W013: relation kind `x.sre/mitigates` is undeclared; treated as elaborates
extrel.smy: 4 records, 3 units, 1 diagnostic(s)
```
]

#callout(label: "Why")[
  `x.sre/mitigates` is only a warning, not an error, and the message says
  exactly what happens next: a reader without the `x.sre` extension treats
  the edge as a plain `elaborates` link rather than dropping it. That is
  rule X in miniature — an org can extend the format for its own tooling,
  and a reader who never installed that extension still gets *something*
  true about the edge, not silence.
]

An extension type is the right tool exactly when the unit or relation is
genuinely domain-specific and you control both ends of the pipeline that
reads it — reach for `x.<domain>/<segment>`, keep the structure in the
header the way any kernel unit would, and a plain kernel-only reader
degrades gracefully instead of losing the record.

#whatsnext[
  You now have a file that `check` accepts. That is not the same thing as
  a file that is *canonically formatted* — the next question, and Chapter
  7's whole subject, is what `fmt` does to the exact bytes you just wrote
  and why it insists on doing it. Once formatting is routine, Chapter 8
  goes back to `check` itself, in full: the ten-pass pipeline, conformance
  classes, and what it means to check a document against a *named*
  consumer rather than against the kernel alone.
]

#exercises((
  [Write a two-line file containing only `@claim c/a { status: unfounded }`
   and a gist, and run `check` on it. You get `SMY-E034`. Chapter 2's status
   table lists `unfounded` as a real rung — so why is authoring one an error,
   and what is the only legitimate way for a unit to reach that status?],
  [Add a key the kernel has never heard of to a valid claim — say
   `x.sre/severity: "sev2"` — and run `check`, then `fmt`. `check` reports no
   diagnostic and `fmt` hands the key back to you. Which rule is this, and
   what would the alternative behaviour cost a team whose peer records
   something yours does not?],
  [Take any clean file, delete one character from a label inside a `grounds`
   list, and run `check`. Count the diagnostics. Now predict what happens to
   that count when you fix the single character, and check whether you were
   right.],
))

#answers((
  [`unfounded` means *this was knocked out from under* — it is the state a
   unit lands in when something it rested on is retracted. It is a
   consequence, not a claim you are entitled to assert directly; a unit that
   never had support was `speculative` all along, and saying `unfounded`
   instead is claiming a history that did not happen. The only way in is
   `retract` (Chapter 18).],
  [Rule X — unknown extensions survive verbatim. Without it, a document from
   a team that records incident severity, or from a version of the format
   newer than your binary, would either be rejected at your boundary or come
   out the far side quietly stripped. Neither failure is visible to the person
   who sent it, which is the same silent-loss problem Chapter 1 is about,
   committed by your own tooling.],
  [You will usually get more than one, and fixing the character usually clears
   all of them at once. A broken reference means the unit that referred to it
   may fail to admit at all, and every *other* unit that referred to *that*
   one now dangles in turn. This is why the advice is to fix the first
   structural diagnostic and re-run rather than working down the list — the
   list is frequently one fault wearing several hats.],
))

#recap((
  [Four constructs are hand-authored — `@doc`, units, `@rel`, `@thread` —
   and the exhaustive field-by-field reference for each is
   `SMYSL_FORMAT_GUIDE.typ`, not this chapter.],
  [`deps` and `grounds` answer different questions: what a reader needs to
   parse the wording, versus what the claim would fall without.],
  [Every shape rule this chapter broke on purpose has a specific reason
   tied to how the corpus is read later — a missing gist breaks the L0
   fallback, empty `grounds` breaks Rule M's walk, a cycle breaks its
   termination.],
  [`check`'s diagnostics can cascade from one root cause — a dropped
   dangling reference emptied `grounds`, which tripped a second, separate
   rule. Fix the first diagnostic and re-run before chasing the rest.],
  [A body/detail line must start at column 0; two-space indent after the
   gist is a gist continuation, not a body — `fmt` will fold it straight
   into the gist if you get this backwards.],
  [`x.<domain>/<type>` and `x.<domain>/<kind>` extend units and relations
   past the kernel without a kernel revision, and degrade to a warning and
   a graceful fallback for a reader who doesn't have the extension.],
))

#chapter(number: 7, title: "Canonical Form and fmt")

#term("Canonical form")[
  The one, unique byte-for-byte spelling of a given set of records. Two
  people who wrote the exact same claims, quoted differently, with fields
  in a different order, converge on identical bytes once `fmt` has touched
  both files. Nothing about *what the document says* changes; everything
  about *how it happens to be spelled* is fixed.
]

#callout(label: "Why")[
  Three things break if spelling is left to whoever typed the file. A diff
  between two versions of a document should mean *the claims changed* —
  not that someone re-quoted a field or reordered `deps` and `grounds`,
  which is exactly the kind of noise that trains a reviewer to stop reading
  diffs closely. Hashes are computed over CBOR, not surface text, so
  identity never moves when you reformat — but a person still reads the
  surface text, and *that* is what canonical form is actually protecting:
  one document, one spelling, so `surface → CBOR → surface` is not just
  lossless in principle but produces the same bytes back out in practice.
]

#section("`fmt --check` finds drift")

The file from Chapter 6, as hand-typed, is not canonical — nobody's
first draft is:

#screen(caption: "$ smysl fmt --check checkout.smy")[
```
checkout.smy: not canonically formatted
```
]

That command exits `3` — the same code `check` uses for a validation
failure. It is not a coincidence: reformatting *is* a check, in exactly the
sense that a file either already has its one canonical spelling or it does
not, and `fmt --check` is the way to ask without touching the file.

#section("`fmt --write`, field by field")

Run it without `--check` and against a plain `@doc` header first, to see
what "expanded, nothing left implicit" actually means:

```
@doc smysl/0.1 {
  id: v/checkout-incident
  intent: incident-brief
  granularity: { profile: default }
}
```

#screen(caption: "$ smysl fmt doc-header.smy")[
```
@doc smysl/0.1 {
  id: v/checkout-incident
  intent: incident-brief
  lang: en
  granularity: { profile: default, l0_max: 30, l1_range: [40, 120], admission: single-assertion }
}
```
]

`lang` appeared from nowhere — its default, `en`, is written out explicitly
rather than left to be inferred later by whoever reads the file next — and
`granularity` grew from a bare profile name into the full set of numbers
that name actually means. Now the rest of Chapter 6's file, before and
after, field by field. As typed:

```
@evidence e/deploy-clean { status: measured, source: { kind: metric, ref: "deploy.canary_p95", captured: 2026-07-20 } }
~ The same-hour checkout canary deploy shows no latency regression on its own.

@definition d/cold-cache { status: cited, source: { kind: doc, ref: "sre-handbook#cache-eviction" } }
~ A cache is "cold" when its hit rate has not yet recovered after an eviction event.

@claim c/cache-cold { status: inferred, grounds: [e/cache-miss], deps: [d/cold-cache] }
~ The checkout price cache is cold after an eviction, not merely slow.

@rel c/cache-cold --causes--> c/latency-up
@rel c/deploy-clean --rebuts--> c/cache-cold { weight: 0.4 }
```

After `fmt --write`:

```
@evidence e/deploy-clean { status: measured, source: { kind: metric, ref: deploy.canary_p95, captured: 2026-07-20 } }
~ The same-hour checkout canary deploy shows no latency regression on its own.

@definition d/cold-cache { status: cited, source: { kind: doc, ref: sre-handbook#cache-eviction } }
~ A cache is "cold" when its hit rate has not yet recovered after an eviction event.

@claim c/cache-cold { status: inferred, deps: [d/cold-cache], grounds: [e/cache-miss] }
~ The checkout price cache is cold after an eviction, not merely slow.

@rel c/cache-cold --causes--> c/latency-up

@rel c/deploy-clean --rebuts--> c/cache-cold { weight: 0.400391 }
```

Four independent decisions, each verified against the real writer rather
than assumed:

#dtable(
  (auto, 1fr),
  (
    ([Change], [What the writer actually does]),
    ([Quoting], [A string is quoted only if it needs to be — it contains a
      comma, a brace, a bracket, a quote mark, or a backslash, or it has
      leading or trailing whitespace, or it would parse as a number,
      `true`, `false`, or `null`. `deploy.canary_p95` and
      `sre-handbook#cache-eviction` need none of that and lose their
      quotes; `pool.wait_ms{shard=eu-west}` keeps its quotes because of the
      braces inside it.]),
    ([Field order], [`status`, `deps`, `grounds`, `source`, `salience`, in
      that fixed order, regardless of the order you typed them in.]),
    ([Weight / salience], [Quantised to steps of 1/1024 and rendered to as
      many decimal places as that needs — `0.4` becomes `0.400391`, the
      nearest 1/1024 step, because both fields are stored as quantised
      binary32 values, never arbitrary-precision decimals.]),
    ([Blank lines], [Every unit and every relation is followed by exactly
      one blank line in canonical form — two `@rel` lines typed back to
      back gain a blank line between them.]),
  ),
)

#callout(label: "Note")[
  Canonical form doesn't just re-punctuate records in place — it
  *regroups* them. Units come first, in the order they were parsed;
  relations next; threads last — regardless of how you interleaved them
  while writing. A thread written *before* the units it points to is
  written *after* them once `fmt` is done.
]

Take a file with the thread written first and the claim it points to
written second:

```
@thread t/brief { schema: brief, owner: "human:vladimir" }
~ Checkout latency rose after a cache eviction.
  bottom-line -> c/cache-cold

@claim c/cache-cold { status: speculative }
~ The checkout price cache is cold.
```

#screen(caption: "$ smysl fmt interleaved.smy")[
```
@claim c/cache-cold { status: speculative }
~ The checkout price cache is cold.

@thread t/brief { schema: brief, owner: human:vladimir, ts: [0, 0] }
~ Checkout latency rose after a cache eviction.
  bottom-line → c/cache-cold
```
]

The thread was first in the source and last in the canonical output.
`check` never cared about the order — order carries no meaning in the
format — but two files that differ only in *which* valid order someone
chose are exactly the diff-noise canonical form exists to kill. Notice
also the arrow, `->` as typed and `→` as written, always, and the
`ts: [0, 0]` timestamp pair that appeared on the thread the same way
`lang` appeared on the doc header — nothing stays implicit once `fmt`
has run.

#whatsnext[
  Format one file until `fmt --check` is silent, then try it on all of
  them at once — the next section.
]

#section("Piping and batch formatting")

`fmt` with no files reads stdin and writes stdout, so it composes directly
with `check`:

#screen(caption: "$ cat checkout.smy | smysl fmt | smysl check -")[
```
-: 14 records, 6 units, 0 diagnostic(s)
```
]

One pipeline, no temporary file: canonicalise, then validate the result,
in that order, exactly as the exit code `1` you'd get from a shell that
short-circuits on the first stage's failure would want.

`fmt` also accepts more than one path, and `--write` rewrites every one of
them in place. Start from two files that are independent, differently
hand-typed copies of the same records — one with a multi-line header and a
quoted date, the other with `grounds` and `status` swapped — and format
both in one command:

#screen(caption: "$ smysl fmt --write batch-a.smy batch-b.smy")[
```
```
]

`fmt --write` prints nothing on success. Run `diff batch-a.smy batch-b.smy`
afterward and it prints nothing either: two files that started out spelled
completely differently converge on byte-for-byte identical canonical text.
That is the entire point of a *canonical* form — it does not just tidy a
file relative to itself, it converges every spelling of the same meaning
onto one, which is exactly what makes a later `diff` between two documents
mean something again.

#section("The round-trip guarantee")

`fmt` does not trust its own output blindly. Reading `cmd_fmt` in
`src/main.rs`: after writing the canonical text, it re-parses that exact
text and compares the result against what it started from —

```
match parse_surface(&formatted) {
    Ok(again) if again.labels == out.labels && again.records == out.records => {}
    _ => {
        eprintln!("{path}: canonical form does not reproduce the same records");
        worst = worse(worst, ExitCode::HashVerification);
        continue;
    }
}
```

— and only proceeds to `--check`'s comparison or `--write`'s rewrite if
that reparse matches exactly. A mismatch here refuses to write anything and
exits `9`, before either `--check` or `--write` ever runs.

#callout(label: "Why")[
  This assertion is not about your document — it is `fmt` checking
  *itself*, on every single run, rather than trusting once during
  development that the writer and the parser agree. If a future change to
  the writer ever produced text the parser would read back differently,
  this is the guard that would catch it immediately, on the very first
  file anyone ran it against, rather than as a silent corruption discovered
  much later.
]

Constructing a real failure meant finding a string the writer would emit
unquoted that the parser would then read back as something other than a
string. The obvious candidate is a `ref` that looks like a number:

#screen(caption: "$ smysl fmt ticket.smy")[
```
@evidence e/ticket { status: measured, source: { kind: tool, ref: "42" } }
~ Ticket number for the follow-up.
```
]

It stays quoted. The quoting rule (Chapter 7's field-by-field table, above)
already special-cases exactly this: a quoteless value that would parse as
a number, `true`, `false`, or `null` is quoted anyway, specifically because
an unquoted `42` would come back as an integer, not the string it started
as. The round-trip guard and the quoting rule were written by the same
hand for the same reason, and between them there is no string this manual
could find that survives being written unquoted and comes back changed —
which is the guarantee doing its job, not a gap in the testing.

#whatsnext[
  A canonical file is a clean starting point, not a finished one — `fmt`
  never asked whether any of the claims in it are actually well-formed
  arguments, only whether the bytes have one correct spelling. Chapter 8
  picks the validation question back up in full: the ten-pass pipeline
  `check` actually runs, in order, and why that order is fixed. And because
  `fmt --check` is silent exactly when a file needs no attention and exits
  `3` the moment it does, it is the kind of command that belongs in CI
  before a commit lands — Chapters 21 and 23 come back to that same exit
  code from the other direction, as part of what a reproducible, verifiable
  pipeline demands on every run, not just the ones you remember to check
  by hand.
]

#exercises((
  [Write the same two units into two files, but order the fields differently
   in each — `{ grounds: [...], status: derived }` in one and
   `{ status: derived, grounds: [...] }` in the other, and swap `kind` and
   `ref` inside a `source` block. Run `fmt` on both and `diff` the results.
   Predict the outcome first.],
  [Run `fmt` on an already-canonical file. Then run `fmt` on *that* output.
   Compare. What property are you demonstrating, and why would `fmt` be
   unusable in a pre-commit hook without it?],
  [`fmt --check` on a non-canonical file exits `3`, not `1`. Look up both
   codes in Chapter 4's table and explain, in one sentence, why `3` is the
   defensible choice.],
))

#answers((
  [The two outputs are byte-identical. Field order in the surface text is
   yours; field order in canonical form is the format's, and `fmt` is the
   function that maps the first onto the second. This is the property that
   makes a `diff` between two canonical files a diff of *content* — if it were
   not true, every reviewer would learn to skim `.smy` diffs, and the format's
   central promise of being reviewable would quietly stop being cashed.],
  [Idempotence: `fmt(fmt(x)) = fmt(x)`. Without it, a pre-commit hook that
   runs `fmt --write` would produce a new diff every time it ran, so the hook
   would never converge and a repository could never be in a formatted state.
   Idempotence is what makes "canonical" a place a file can arrive at rather
   than a direction it can be pushed in.],
  [`1` is a generic failure — something went wrong. `3` is *check errors*: the
   tool looked at your document and found it wanting in a specific,
   documented way. `fmt --check` is a check, so it reports like one, which
   means a CI pipeline can treat `fmt --check` and `check` under the same
   branch without special-casing either.],
))

#recap((
  [Canonical form is one unique spelling per set of records — quoting
   decided by content, fixed field order, quantised floats, `→` always
   emitted — so a diff between two documents reflects a change in meaning,
   not a change in whoever's keyboard typed it last.],
  [`fmt --check` exits `3`, exactly like a failed `check`; `fmt --write`
   rewrites in place and expands every implicit default — `lang`,
   `granularity`'s full field set, a thread's `ts` — so nothing about the
   document stays unstated.],
  [Canonical form regroups records by kind — units, then relations, then
   threads — regardless of how they were interleaved while writing; order
   never carried meaning in the first place.],
  [`fmt` composes through stdin/stdout (`cat x.smy | smysl fmt | smysl check -`) and accepts multiple files with `--write`, converging every
   independently-spelled copy of the same records onto identical bytes.],
  [Every `fmt` run re-parses its own output and compares it, structurally,
   against what it started from — a mismatch refuses to write anything and
   exits `9`. The writer's own quoting rules are built specifically to
   prevent that mismatch from ever firing, which is why constructing a real
   failure of it was not possible from the outside.],
))
