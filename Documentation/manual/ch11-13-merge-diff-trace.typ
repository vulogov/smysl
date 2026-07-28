#import "design.typ": *

#part(number: "V", title: "Operating on Documents")

#chapter(number: 11, title: "merge — Union Without a Referee")

Two agents — or the same person, an hour apart — can work from the same
corpus at once. Neither has to wait for the other, neither has to ask a
coordinator who goes first, and neither's work is allowed to quietly
overwrite the other's. That is the whole job of `merge`: take two or more
stores and produce the one store that knows everything either of them knew.

#callout(label: "Why")[
  The ordinary way to combine two versions of anything is "last write wins" —
  whichever save happened second is what survives. Applied to a document full
  of claims and evidence, that is not a merge, it is a coin toss with the
  claim's certainty as the stake: one agent's belief is silently destroyed,
  and nothing in the output says so. `smysl` treats a merge as a
  #term("Join")[
    From lattice math: given two partial pictures of the same situation,
    their *join* is the smallest single picture containing everything both
    of them knew — the least upper bound. A join is commutative and
    order-independent: `merge(A, B)` and `merge(B, A)` produce the same
    store, and merging the same input twice costs nothing extra. Nobody has
    to go first, and nobody has to be asked twice.
  ]
  instead — a union over both stores' units, attestations, relations, and
  contentions. Nothing is picked for you behind your back. Where the two
  sides genuinely disagree, that disagreement becomes a record, not a
  casualty.
]

Units are content-addressed: two agents that independently write down the
exact same claim compute the exact same identifier without comparing notes,
and the join collapses the two into one entry with two attestations rather
than two units. That is agreement, and it needs no ceremony. The interesting
case is the other one — two *different* claims that cannot both be quietly
true — and that is what the rest of this chapter is about.

#section("Merging one document already tells you something")

It is tempting to think of `merge` as a two-input operation, but it is really
a fold: even a single store, merged into an empty one, can already contain a
disagreement it was never told about. `F1-incident.smy` threads
`c/pool-saturation` and `c/canary-clean` together as one argument — the
thread's `support` step names the claim, its `risk` step names the very unit
that rebuts it — and a rebuttal that a thread presents as one line of
reasoning is exactly what `smysl` calls live:

#screen(caption: "$ smysl merge fixtures/corpus/F1-incident.smy -o /dev/null")[
```
fixtures/corpus/F1-incident.smy: contention k/ccm3actwjjti65famnoe6mapo5d over b3:cvhirtgs2mpvli2ethhyeo32uf (2 positions, live-rebuttal)
```
]

`b3:cvhirtgs2mpvli2ethhyeo32uf` is `c/pool-saturation` — the same real uid
this book has used since Chapter 17's `pack --focus`, because these are
content hashes and this fixture has not changed. The contention id,
`k/ccm3actwjjti65famnoe6mapo5d`, is derived from the contention's own content
— the kind, the unit it is over, and its positions — never allocated, so two
peers that independently detect the same disagreement name it identically.
Detection is not something `merge` invents on the spot: it is a pure
function of whatever the union already contains, run once at the end of
every merge, however many inputs there were.

#term("Contention")[
  A materialised disagreement — a record naming a unit, the positions that
  disagree about it, and *why* it was raised. Merge never adjudicates a
  contention: nothing is superseded, nothing is dropped, and no side is
  marked as the winner. A contention is *reported* by every merge that would
  imply it, not written permanently into the log, because whether two
  successors fork or chain can change the moment a third store supplies the
  edge that orders them — writing a stale finding into an append-only log
  would make it permanent for no good reason. If a finding should travel
  with the document, someone resolves it deliberately (Chapter 16) and that
  resolution is what gets recorded.
]

There are exactly three shapes a contention can take, and a single two-agent
merge below produces all three in one run.

#section("Two agents, one root cause")

`F8a-agent-alpha.smy` and `F8b-agent-beta.smy` are two independent triage
documents about the same us-east gateway incident, both declaring
`roots: [c/cause]` — the same *label*, used by two people who never saw each
other's file. Merging them is the ordinary case this chapter exists for:

#screen(caption: "$ smysl merge fixtures/corpus/F8a-agent-alpha.smy fixtures/corpus/F8b-agent-beta.smy -o /dev/null")[
```
fixtures/corpus/F8a-agent-alpha.smy: contention k/co7i7feme3q3sed4l63dkcgmmuy over b3:v5sjn55ujeflrdyu5qycgmtvz3 (2 positions, supersession-fork)
fixtures/corpus/F8b-agent-beta.smy: contention k/c3xa27zib4d7t5rme4xekauzh4t over b3:t65rff76bcbxnwzw4oxzcadthe (2 positions, live-rebuttal)
fixtures/corpus/F8b-agent-beta.smy: contention k/cboxvjp36tnst3vmpsoo2mmvhqu over b3:cjixk2inyftvxj55d53w2ivxej (2 positions, label-collision)
fixtures/corpus/F8b-agent-beta.smy: contention k/co7i7feme3q3sed4l63dkcgmmuy over b3:v5sjn55ujeflrdyu5qycgmtvz3 (2 positions, supersession-fork)
```
]

Four lines, three distinct contentions — the fourth is the same
`supersession-fork` re-detected once the second file is in, since detection
runs fresh against the whole union at every step. Read each one against its
actual source:

#dtable(
  (auto, 1.4fr, 2.4fr),
  (
    ([Kind], [Fires when], [What actually happened here]),
    ([`supersession-fork`], [Two distinct units supersede the same target, and neither supersedes the other.], [Alpha's own file already disagrees with itself: `c/pool-size-32` and `c/pool-size-64` both `--supersedes-->` `c/alpha-scope` (`b3:v5sjn55…`), and nothing orders one revision after the other. This one needed no second file — alpha alone was already forked.]),
    ([`live-rebuttal`], [A `rebuts` edge joins two units both selected by a common thread.], [Beta's thread `t/beta-brief` puts `c/cause` (`b3:t65rff76…`, "regressed because 7.1 shortened the upstream timeout") at `bottom-line` and `c/timeout-not-culprit` at `risk` — and `c/timeout-not-culprit --rebuts--> c/cause` is a real edge in the same file. Beta's document argues with itself, on purpose, and the thread is what makes that argument *live* rather than a stray edge nobody has read together.]),
    ([`label-collision`], [One label resolves to different uids across the sources in scope.], [Both files bind the label `c/cause` — but alpha's `c/cause` ("exhausting its upstream connection pool") and beta's `c/cause` ("regressed because 7.1 shortened the upstream timeout") are different content, hence different real uids. The label agrees; the claims underneath it do not.]),
  ),
)

Nothing here was adjudicated. All five of alpha's and beta's competing claims
are in the merged store, queryable, exactly as their authors wrote them —
`diff` (Chapter 12) will show you all ten units are present, and `trace`
(Chapter 13) can walk from either `c/cause` to what it actually rests on.
What changed is that a human reading only the contention list — not the
whole ten-unit store — already knows there are three specific places where
the two agents' work does not simply combine, and exactly which units are on
each side of each one.

#section("Three ways to handle a fork, on the same two inputs")

`--policy` controls what happens to the `supersession-fork` kind
specifically — the other two kinds are unaffected by it, because they are
not about competing revisions of one thing, they are about live arguments
and label conflicts, which stay worth reporting under every policy. Run the
identical two-file merge three times, changing only `--policy`:

#dtable(
  (auto, 3.6fr),
  (
    ([Policy], [What changes in the report]),
    ([`contend` — default], [All three kinds are reported. The fork over `c/alpha-scope` is surfaced explicitly, alongside the live rebuttal and the label collision.]),
    ([`latest`], [The fork over `c/alpha-scope` is dropped from the report. The live rebuttal and label collision are still reported — they are not what this policy governs.]),
    ([`all`], [Identical output to `latest` in this build: the fork is suppressed, the other two kinds still fire. `all` and `latest` differ only in *name*, not in behaviour, until something downstream (a view, a render pass) actually picks a winner by HLC — which nothing in this chapter's commands does.]),
  ),
)

#screen(caption: "$ smysl merge --policy latest fixtures/corpus/F8a-agent-alpha.smy fixtures/corpus/F8b-agent-beta.smy -o /dev/null")[
```
fixtures/corpus/F8b-agent-beta.smy: contention k/c3xa27zib4d7t5rme4xekauzh4t over b3:t65rff76bcbxnwzw4oxzcadthe (2 positions, live-rebuttal)
fixtures/corpus/F8b-agent-beta.smy: contention k/cboxvjp36tnst3vmpsoo2mmvhqu over b3:cjixk2inyftvxj55d53w2ivxej (2 positions, label-collision)
```
]

#callout(label: "Note")[
  This is worth being exact about, because the policy names invite a wrong
  guess. Units in `smysl` are immutable and content-addressed — a store
  cannot delete `c/pool-size-32` just because `--policy latest` was passed,
  any more than it could delete `c/pool-size-64`. Both revisions are in the
  merged store under every policy; you can `trace --parents` from either one
  straight to `c/alpha-scope` regardless of `--policy` (Chapter 13 does
  exactly this). What `--policy` actually decides is narrower and more
  honest than "which one wins": whether the fork gets raised as an open
  contention for a human to see, or passes through silently because you
  already know you do not need `merge` to flag every fork out loud. `Contend`
  is the default because silence is the one behaviour this design exists to
  avoid — pick `latest` or `all` deliberately, not by habit.
]

#whatsnext[
  A merge that reports a fork but leaves both revisions live is not the end
  of the story for that disagreement — someone still has to decide what
  `c/alpha-scope` actually is. Resolving it is an act of ordinary authoring,
  not a special command: write a new unit that lists both `c/pool-size-32`
  and `c/pool-size-64` as grounds and `--supersedes-->` the original, and
  merge that in like anything else — the fork closes because there is now a
  latest that both prior revisions feed into. Withdrawing belief outright,
  as opposed to superseding it with something better, is `retract`'s job
  instead (Chapter 16), and it computes exactly what else would fall before
  it lets you do that.
]

#section("When a retraction outruns its dependents")

A fork is two sides that never agreed. A retraction is different: one side
withdraws something outright, and the question is what happens to whatever
the *other* side was still resting on it. `--retraction` governs exactly
that, and none of this book's corpus fixtures retract anything on purpose —
so here is the smallest real case, two hand-authored files that do:

```
@evidence e/dashboard-read { status: measured, source: { kind: metric, ref: "pool.wait_ms{shard=eu-west}", captured: 2026-07-09 } }
~ Pool acquisition wait read 310 ms at the time of capture.

@claim c/still-standing { status: derived, grounds: [e/dashboard-read] }
~ The eu-west pool was under load at capture time.
```

The second file retracts the evidence the claim above rests on — a
retraction is a `rel` from a unit to the target it withdraws, and retracting
a unit's own self (`from == to`) is how a document withdraws belief in
*itself*:

```
@evidence e/dashboard-read { status: measured, source: { kind: metric, ref: "pool.wait_ms{shard=eu-west}", captured: 2026-07-09 } }
~ Pool acquisition wait read 310 ms at the time of capture.

@rel e/dashboard-read --retracts--> e/dashboard-read
```

Merged under the three policies, in order:

#screen(caption: "$ smysl merge base.smy withdraw.smy -o strict.cbor   (--retraction strict, the default)")[
```
withdraw.smy: error: SMY-E050: every ground of this unit has been retracted (at b3:x65pg5dyx2chhnfiytd5isug3k) [try: retract this unit too, or re-ground it on something surviving]
```
]

#screen(caption: "$ smysl merge --retraction advisory base.smy withdraw.smy -o advisory.cbor")[
```
withdraw.smy: warning: SMY-W052: retracted, but retained under the advisory policy (at b3:5ksv36cuc6crzd66aiad45iu66)
```
]

#screen(caption: "$ smysl merge --retraction ignore base.smy withdraw.smy -o ignore.cbor")[
```
```
]

#dtable(
  (auto, 3.6fr),
  (
    ([Policy], [What it does to `c/still-standing`]),
    ([`strict` — default], [The retraction is transitive over `grounds`: `e/dashboard-read` is gone, `c/still-standing` has nothing left under it, and the merge reports `SMY-E050` — an error, not a silent downgrade. The unit's effective status reads `unfounded`, the ladder's own floor, reachable only by a retraction like this one.]),
    ([`advisory`], [The retraction is recorded and flagged (`SMY-W052`, a warning) but does not propagate — `c/still-standing` keeps its `derived` status. You are told the ground under it was withdrawn; the tool does not decide for you whether that is fatal to the claim resting on it.]),
    ([`ignore`], [The retraction is recorded — it is still a real relation in the store, and `trace --parents` will still find it — but it has no effect on anyone's status at all. Silence in the report is this policy working as designed, not a sign nothing happened.]),
  ),
)

`strict` is the default for the same reason `contend` is: it is the policy
that cannot quietly let something rest on ground that no longer exists.
`RetractionAuthority` (`origin` by default) is the companion knob this
chapter does not need to demonstrate directly — only an agent that already
attested a unit may retract it, which is the defence against one adversarial
source erasing work it had no hand in.

#section("Making disagreement block the pipeline, or not")

Everything above *reports*. Two flags exist for when a caller — a script, a
CI step — needs disagreement to actually stop something.

#screen(caption: "$ smysl merge --fail-on-contention fixtures/corpus/F8a-agent-alpha.smy fixtures/corpus/F8b-agent-beta.smy -o /dev/null")[
```
smysl merge: 1 open contentions
```
]

Exit code `5` — `ExitCode::Contentions`. Read the count carefully: it says
`1`, not `3`, because `--fail-on-contention` checks after *each* input is
folded in, and alpha's file alone already carries one contention (the
`supersession-fork` over `c/alpha-scope`) before beta's file is ever merged.
The pipeline stops at the first offending step, not at the end of the whole
run — a caller who wants to know about *all* three contentions, not just the
first one that tripped the gate, still wants the default `contend` policy
without `--fail-on-contention`, reads the report, and decides by hand.

`--max-contentions-per-agent` is a softer, different concern: not "stop on
any contention" but "warn if one source is raising an unreasonable number of
them" — the mitigation for a single flooding source drowning a merge in
disagreements nobody can review one at a time:

#screen(caption: "$ smysl merge --max-contentions-per-agent 0 fixtures/corpus/F8a-agent-alpha.smy fixtures/corpus/F8b-agent-beta.smy -o /dev/null")[
```
fixtures/corpus/F8a-agent-alpha.smy: warning: SMY-W055: 1 contentions from one source exceeds the cap of 0
fixtures/corpus/F8a-agent-alpha.smy: contention k/co7i7feme3q3sed4l63dkcgmmuy over b3:v5sjn55ujeflrdyu5qycgmtvz3 (2 positions, supersession-fork)
fixtures/corpus/F8b-agent-beta.smy: warning: SMY-W055: 3 contentions from one source exceeds the cap of 0
fixtures/corpus/F8b-agent-beta.smy: contention k/c3xa27zib4d7t5rme4xekauzh4t over b3:t65rff76bcbxnwzw4oxzcadthe (2 positions, live-rebuttal)
fixtures/corpus/F8b-agent-beta.smy: contention k/cboxvjp36tnst3vmpsoo2mmvhqu over b3:cjixk2inyftvxj55d53w2ivxej (2 positions, label-collision)
fixtures/corpus/F8b-agent-beta.smy: contention k/co7i7feme3q3sed4l63dkcgmmuy over b3:v5sjn55ujeflrdyu5qycgmtvz3 (2 positions, supersession-fork)
```
]

The cap of `0` here is deliberately absurd, to force the warning on the
smallest possible corpus. Exit code stays `0`: `SMY-W055` is a warning, not a
gate — it tells you a source is behaving unusually without stopping the
merge, which is the right default for a signal that is a heuristic ("this
looks like flooding") rather than a certainty. If your pipeline needs the cap
to actually block, pair it with `--fail-on-contention`, or read the report
and decide.

#section("--staged: the other half of ingest's pause")

Chapter 8 left `ingest` exiting `10`, a batch of proposed units sitting in
`.smysl/staged.smy`, waiting for a decision — and promised that committing
it was "the ordinary merge join." That promise is literal: `--staged` reads
the project's staged file and merges its records into your store exactly
like any other document, no separate code path, no special ceremony.

Here is that loop closed for real. A small store already holding
`F1-incident.smy`, and a staged batch — hand-authored here, standing in for
what a real `ingest` run would have written, since Chapter 8's own live
attempt degraded to opaque prose in this offline sandbox:

#screen(caption: "$ cat .smysl/staged.smy")[
```
@evidence e/pool-config { status: measured, source: { kind: file, ref: "pool.yaml", captured: 2026-07-10 } }
~ The eu-west pool's max size was left at the 4.1 default of 32 connections.

@claim c/config-drift { status: derived, grounds: [e/pool-config] }
~ The pool was never resized when 4.2 doubled per-request hold time.
```
]

#screen(caption: "$ smysl --store project/store.cbor merge project/store.cbor --staged -o project/store2.cbor")[
```
project/store.cbor: contention k/ccm3actwjjti65famnoe6mapo5d over b3:cvhirtgs2mpvli2ethhyeo32uf (2 positions, live-rebuttal)
smysl merge: committed 2 staged record(s)
```
]

The live-rebuttal line is the same one this chapter opened with — merging
`store.cbor` at all re-detects it, because detection is a property of the
union, not something remembered between runs. The line that matters for this
section is the second one: two staged records, committed, into the real
store. `.smysl/staged.smy` is ordinary surface text and this is an ordinary
merge — the only thing distinguishing it from any other input file is where
`--staged` looks for it (`.smysl/staged.smy`, relative to whichever path
`--store` resolves the project root against). Nothing about a staged batch
gets special treatment once it is committed: it is subject to the same
supersession policy, the same retraction policy, the same contention
detection as `F8a-agent-alpha.smy` or any other input.

#whatsnext[
  You now have a merged store — from two agents' work, from a retraction,
  or from a staged batch. The next honest question is *what actually
  changed*: which units are new, which survived untouched, which one side
  had that the other did not. That is exactly what Chapter 12's `diff`
  answers, and it is not a coincidence that the staged-merge example above
  reappears there as a live before/after. If instead you want to know why
  one specific unit ended up the way it did — what it rests on, who is
  behind it — Chapter 13's `trace` walks that lineage directly.
]

#recap((
  [`merge` is a join over stores keyed by canonical identity — commutative,
   order-independent, and idempotent. It never adjudicates a disagreement;
   it materialises one as a `Contention` and leaves both sides in the
   store.],
  [Three kinds of contention exist: `supersession-fork` (two successors,
   neither orders the other), `live-rebuttal` (a `rebuts` edge inside a
   common thread), and `label-collision` (one label, two real uids). A
   single two-agent merge can — and, in `F8a`/`F8b`, does — produce all
   three.],
  [`--policy` (`contend` default, `latest`, `all`) governs only whether a
   `supersession-fork` is *reported*; units are immutable, so no policy
   deletes a competing revision. `--retraction` (`strict` default,
   `advisory`, `ignore`) governs whether a retraction propagates to what
   was grounded on it, is merely flagged, or is recorded inertly.],
  [`--fail-on-contention` turns any open contention into exit code `5`,
   checked after each input is folded in — it can stop at the first
   offending file, before later inputs are even considered.
   `--max-contentions-per-agent` warns (`SMY-W055`) on a suspiciously large
   batch from one source without blocking the merge on its own.],
  [`--staged` merges `.smysl/staged.smy` into the store exactly like any
   other input — the confirmation `ingest`'s exit `10` was waiting for,
   with no separate commit mechanism to learn.],
))

#chapter(number: 12, title: "diff — What Changed")

`merge` tells you what a union contains. It does not tell you, in plain
terms, what is *new* since the last time you looked, what one source has
that another does not, or who is responsible for a given change. That is a
different question — comparison rather than combination — and `diff`
answers it two ways: between two stores, or across one store's own history.

#callout(label: "Why")[
  "The two documents are different" is rarely the useful fact on its own. A
  human reviewing a merge, or a pipeline gating on one, needs to know
  exactly *which* units moved and into which of a small number of honest
  categories: present in one but not the other, present in both, or — across
  time rather than across sources — survived untouched, superseded by
  something newer, retracted outright, or newly added. `diff` partitions
  uids into exactly those categories and nothing fuzzier.
]

#section("Two stores: only in A, only in B, common")

`F1-incident.smy` and `F6-adversarial.smy` both start from the same eu-west
latency incident and reach opposite conclusions by design — F6 launders a
guess and a rumour into a root cause with no real evidence underneath. As
stores, they share no unit at all: every claim, however similar the incident
it discusses, is different content, hence a different uid.

#screen(caption: "$ smysl diff fixtures/corpus/F1-incident.smy fixtures/corpus/F6-adversarial.smy")[
```
8 only, 5 only, 0 common
- b3:cvhirtgs2mpvli2ethhyeo32uf
- b3:ekitkvj75uvgzxpvq3ad2nrv3b
- b3:izyuzlt42mqcvgdfb4nfpllxyq
- b3:js4xzessu5zwjpv2rawtugnuvj
- b3:phsoomklkmlq3sjvbe6cyuqy5v
- b3:re42iey2e7syg6zp73tfrlqbvh
- b3:wo4t2c46lq45fnakd6tajlgcac
- b3:xkys7j42mcuyiaxiyh73xddimr
+ b3:bacqe7jyc3pfhpnmn2vivvpz47
+ b3:hdn4uifpmzzopwuyajmhyf5xzu
+ b3:xa7undjm37yl57reyhuxgkiwmm
+ b3:2hkacsatxuvcywj6f4w2lkzojx
+ b3:4p7cfgyytyimdbqyhq7hkuw2sb
```
]

`-` marks a uid only in the first store (`only_in_a`), `+` only in the
second (`only_in_b`) — the same convention a text diff uses, applied to
content-addressed units instead of lines. Zero common is the honest result
for two documents that never shared a source: `diff` does not try to guess
that two differently-worded claims about the same incident are "the same
idea," because that judgement is exactly the kind of soft equivalence a
content hash refuses to manufacture.

`common` becomes the informative column once the two stores in question
really did share a starting point — which is precisely what Chapter 11's
`--staged` example produced. Diff the original store against the one with
the staged batch committed into it:

#screen(caption: "$ smysl diff fixtures/corpus/F1-incident.smy project/store2.cbor")[
```
0 only, 2 only, 8 common
+ b3:qdypl5q72cqdk6oqfk4tzzkcgo
+ b3:wicfl5usmccnimzzmlo3jvpg3f
```
]

Zero only in A, exactly the two committed units only in B, and all eight of
F1's original units in common — this is what a clean, additive merge looks
like from the outside: nothing lost, nothing changed, two new units. Diffing
two totally unrelated stores and diffing a store against its own later state
are the same operation; the difference in what comes out is entirely a
difference in what actually happened between them.

#section("Across time: --hop")

The same partition applies to one store's own history, if that store's units
carry attestations placing them at specific hops — a hop being the ingest or
merge event that first attested a unit. `--hop A..B` asks: of what existed
at hop `A`, what survived to hop `B` untouched, what was superseded, what was
retracted, and what is new in the window?

#screen(caption: "$ smysl diff --hop 0..5 fixtures/corpus/F1-incident.smy")[
```
fixtures/corpus/F1-incident.smy: 8 unit(s), none attested - a store with no provenance cannot be asked what changed when
```
]

Every fixture in this corpus produces exactly this message under `--hop`,
and it is worth being honest about why rather than reaching for a fixture
that happens to look different. Every `.smy` file this book has used is
hand-authored surface text — a person wrote `@claim c/pool-saturation`
directly. Attestations, and the hop number carried on them, are stamped by
`ingest` (Chapter 8) or by a semantic pass (Chapter 9), not by the surface
grammar itself: there is no way to write a hop into a `.smy` file by hand,
because attesting *is* the act of a pipeline recording who touched a unit
and when. A store built entirely from hand-authored text has zero
attestations by construction, and `hop_diff`'s own rule is exactly this
message's second half: a unit nobody has attested cannot be placed in time,
so it is excluded rather than guessed at — the same conservative instinct
that governs every ambiguous case in this design.

This is not a dead end so much as a description of when `--hop` becomes
useful: once a store has actually passed through more than one distinct,
attested pass — one `ingest` run this week, another next week, each stamped
with an increasing hop — `--hop 0..1` partitions exactly what that week's
pass did. A single ingest pass, or a hand-authored file, has nothing to
compare a hop against; a store's *history*, not its content, is what this
flag reads.

#callout(label: "Note")[
  `--by-agent` and `--recipe` are both extensions of this same `--hop`
  output, not independent flags: `--by-agent` attributes each addition,
  supersession, and retraction in the window to the agent responsible;
  `--recipe` additionally separates *the prompt changed* from *the provider
  changed* for any unit superseded across the window, using the
  #term("Recipe")[
    A hash of the full conditions of one model call — provider, model,
    prompt template and its version. Two attestations sharing a
    `recipe_family` but not a `recipe` were produced by a different vendor
    answering the same logical question; two differing in `recipe_family`
    came from a genuinely different prompt. Chapter 10 covers where this
    hash comes from in full.
  ]
  each attestation carries. Both inherit the same requirement `--hop` has:
  a store with no attested hops has nothing for either flag to attribute.
]

#whatsnext[
  `diff` tells you *that* something changed and, across a hop range, *who*
  changed it. It does not tell you *why* a specific surviving unit is
  entitled to the status it has, or what would fall if one of its grounds
  were pulled out from under it. That question — walking a single unit's
  actual support, not the whole store's turnover — is Chapter 13's `trace`.
]

#recap((
  [`diff` between two stores partitions uids into `only in A`, `only in B`,
   and `common` by exact content-addressed membership — never by
   similarity. Two unrelated documents about the same incident can, and
   often will, share nothing.],
  [`common` is the informative column once two stores share real history —
   diffing a store against itself after a merge is the direct way to see
   exactly what that merge added, with nothing lost or altered along the
   way.],
  [`--hop A..B` partitions one store's units by attested hop into survived,
   superseded, retracted, and added, plus a measured survival rate. It
   requires attestations to exist at all — a store built from hand-authored
   surface text, with no `ingest` or `attest` pass behind it, has none, and
   says so rather than guessing.],
  [`--by-agent` and `--recipe` refine a `--hop` diff further, attributing
   changes to specific agents and separating a provider swap from an actual
   prompt change — both inherit `--hop`'s dependency on real attested
   provenance.],
))

#chapter(number: 13, title: "trace — Walking Provenance")

`salience` (Chapter 15) ranks a whole store; `pack` (Chapter 17) decides what
survives a budget. Neither one answers the question you actually reach for
when someone pushes back on a specific claim: *where does this one thing's
certainty actually come from?* `trace` walks exactly that — one unit's
ancestry, in the direction you ask for, as far back as you ask it to go.

#callout(label: "Why")[
  A claim that reads `inferred` is only as defensible as what it rests on.
  When someone challenges `f/root-cause`, "trust me" is not an answer this
  format is built to need — the real answer is a specific, walkable chain
  of evidence and definitions that either holds up or does not. `trace` is
  that chain, made explicit rather than left for a person to reconstruct by
  re-reading the whole document.
]

#section("Grounds versus parents: two different questions")

`trace` follows one of two distinct kinds of edge, and confusing them
answers the wrong question. `--grounds` (the default) walks *evidential*
support — a unit's `grounds` and `deps`, what it rests on to be true.
`--parents` walks *causal* lineage — attestation `parents` and
`supersedes`, where a unit's content actually came from. `--both` walks
both at once. `F1-incident.smy`'s finding, `f/root-cause`, makes the
distinction concrete:

#screen(caption: "$ smysl trace b3:js4xzessu5zwjpv2rawtugnuvj fixtures/corpus/F1-incident.smy   (--grounds is the default)")[
```
b3:js4xzessu5zwjpv2rawtugnuvj (root)
  b3:cvhirtgs2mpvli2ethhyeo32uf (grounds)
  b3:wo4t2c46lq45fnakd6tajlgcac (grounds)
    b3:ekitkvj75uvgzxpvq3ad2nrv3b (deps)
    b3:izyuzlt42mqcvgdfb4nfpllxyq (grounds)
    b3:re42iey2e7syg6zp73tfrlqbvh (grounds)
fixtures/corpus/F1-incident.smy: 6 unit(s) over 2 step(s)
```
]

Six units, resolved back to real labels: the root is `f/root-cause`, whose
two direct grounds are `c/pool-saturation` and `c/regression`; `c/regression`
in turn grounds on `e/trace` and depends on the definition `d/p95`, and
`c/pool-saturation` grounds on `e/pool-wait`. That is the whole evidential
case for the finding, two steps deep, and nothing here is causal — no author,
no attestation, just what the claim rests on to be believed.

`--parents` on the same root asks a different question and gets a different,
honest answer:

#screen(caption: "$ smysl trace --parents b3:js4xzessu5zwjpv2rawtugnuvj fixtures/corpus/F1-incident.smy")[
```
b3:js4xzessu5zwjpv2rawtugnuvj (root)
fixtures/corpus/F1-incident.smy: 1 unit(s) over 0 step(s)
```
]

Just the root. `F1-incident.smy` is a single hand-authored document — no
unit here was transformed from an earlier one, and nothing supersedes
anything else — so there is no causal ancestry to walk. This is the correct
answer to "where did this content come from," not a broken trace: for a
document nobody derived from an earlier version, the honest lineage is one
step long.

`F8a-agent-alpha.smy` has real `supersedes` edges — Chapter 11's fork over
`c/alpha-scope` — and `--parents` finds them from either side:

#screen(caption: "$ smysl trace --parents b3:e3f6wpuw6ie4ruf2ba62yrtagg fixtures/corpus/F8a-agent-alpha.smy")[
```
b3:e3f6wpuw6ie4ruf2ba62yrtagg (root)
  b3:v5sjn55ujeflrdyu5qycgmtvz3 (supersedes)
fixtures/corpus/F8a-agent-alpha.smy: 2 unit(s) over 1 step(s)
```
]

That revision of the pool-size claim supersedes `c/alpha-scope`
(`b3:v5sjn55…`) — the exact unit Chapter 11 showed forked, from the other
direction: tracing *from* `c/alpha-scope` itself with `--parents` finds
nothing, because `trace` follows `supersedes` outward from whichever unit
you name, and `c/alpha-scope` is the *target* of that edge, not its source.
Naming the successor, not the superseded unit, is what makes the ancestry
walk.

#section("Bounding the walk: --depth")

An unbounded trace walks to the roots of whatever it is following — fine for
a six-unit fixture, less fine for a corpus with real depth. `--depth`
truncates it:

#screen(caption: "$ smysl trace --depth 1 b3:js4xzessu5zwjpv2rawtugnuvj fixtures/corpus/F1-incident.smy")[
```
b3:js4xzessu5zwjpv2rawtugnuvj (root)
  b3:cvhirtgs2mpvli2ethhyeo32uf (grounds)
  b3:wo4t2c46lq45fnakd6tajlgcac (grounds)
fixtures/corpus/F1-incident.smy: 3 unit(s) over 1 step(s)
```
]

Three units instead of six, one step instead of two — `f/root-cause`'s two
direct grounds, with everything beneath *them* cut off. `--depth 0` would
return only the root itself. This is the tool a person reaches for once a
trace's full answer is a wall of text: name how many steps of "why" you
actually need before deciding whether to go deeper.

#section("--agents: who is behind each step")

`--agents` names, for every node in the walk, every agent that attested it —
answering not just *what* a claim rests on but *who* put each piece there.
On this book's hand-authored fixtures, that answer is honestly empty:

#screen(caption: "$ smysl trace --both --agents b3:js4xzessu5zwjpv2rawtugnuvj fixtures/corpus/F1-incident.smy")[
```
b3:js4xzessu5zwjpv2rawtugnuvj (root)
  b3:cvhirtgs2mpvli2ethhyeo32uf (grounds)
  b3:wo4t2c46lq45fnakd6tajlgcac (grounds)
    b3:ekitkvj75uvgzxpvq3ad2nrv3b (deps)
    b3:izyuzlt42mqcvgdfb4nfpllxyq (grounds)
    b3:re42iey2e7syg6zp73tfrlqbvh (grounds)
fixtures/corpus/F1-incident.smy: 6 unit(s) over 2 step(s)
```
]

No bracketed agent names appear, for the same reason Chapter 12's `--hop`
came up empty: attribution reads from attestations, and a store built
straight from hand-authored surface text has none. `--agents` earns its
keep once a corpus has actually been through `ingest` or `attest` — at that
point every node in a trace can carry `[human:vladimir]`, `[model:a/x]`, or
whichever agents actually touched it, turning "what does this rest on" into
"what does this rest on, and who is answerable for each piece of it."

#section("The label-versus-uid rule, one more time, verified fresh")

Every uid-taking flag in this book — `--focus`, `--seed`, `--roots`, and
`trace`'s own positional argument — takes a real content hash, never a
surface label. This is not a suggestion; the store enforces it, and the
failure is exactly as blunt as Chapter 17 already showed for `pack`:

#screen(caption: "$ smysl trace c/pool-saturation fixtures/corpus/F1-incident.smy")[
```
smysl trace: `c/pool-saturation` is not a uid
```
]

Exit code `1`. `c/pool-saturation` is only ever a name inside the one file
that declared it — it has no existence at the store level, where the only
identity a unit has is the hash of what it says. The real uid behind that
label, `b3:cvhirtgs2mpvli2ethhyeo32uf`, is what every example in this chapter
actually used, and it is exactly the same hash Chapter 17's `pack --focus`
resolved for the same claim — content hashes are stable across every command
that ever touches this fixture, which is the whole point of computing
identity from content rather than position.

When you do not already know a unit's real uid, the practical path is the
one this book has used throughout: run `smysl --format surface pack --budget
<large> --explain <file>` and read every unit's hash straight off the top of
the output, or reach for `thread --show` once a thread already names the
unit you care about (Chapter 18). Either way, the uid you paste into `trace`
has to have come from the store itself — never typed from memory, never
guessed from a label that merely looks similar.

#whatsnext[
  Once you can walk from a claim to exactly what it rests on, the next
  question is usually "how do I hand *that* to someone else" — not the
  whole corpus, just the reachable set a trace just proved matters.
  Chapter 14's `view` and `bundle` take a set of roots and compute exactly
  that reachable closure as a portable, self-contained store — the natural
  next step once `trace` has told you what the roots should be.
]

#recap((
  [`trace` walks one unit's ancestry in one of two directions: `--grounds`
   (the default) for evidential support — what a claim rests on to be
   true — or `--parents` for causal lineage — attestation parents and
   `supersedes`, where its content actually came from. `--both` walks both
   at once, and the two answers can be, and often are, completely
   different.],
  [`--depth` bounds how far back the walk goes, in steps from the root;
   `--depth 0` returns only the unit you named.],
  [`--agents` names every agent behind each step in the walk, read from
   attestations — a store with no attestation history, like a
   hand-authored fixture, has nothing for it to name, which is a fact about
   the store's provenance, not a broken flag.],
  [Every uid-taking flag across the whole CLI — `trace`'s target included —
   rejects a label outright and reports so plainly. A label exists only
   inside the one file that declared it; the real, content-derived uid is
   what the store actually knows, and it is stable across every command
   that touches the same content.],
))
