#import "design.typ": *

#chapter(number: 19, title: "pack — Budget-Bounded Selection")

Every earlier chapter in this part assumed the whole store travels: `view`
picks roots, `salience` ranks, `bundle` groups, but nothing so far has had to
throw anything away. Sooner or later something downstream has a ceiling —
a model's context window, a chat message limit, a person's patience — and
the document has to fit inside it. `pack` is the command that does the
fitting, and the whole chapter is really about one design decision: what
survives the cut is never chosen by how important a sentence sounds.

#callout(label: "Why")[
  The obvious way to shrink a document is to keep the most important-sounding
  sentences and drop the rest. That is exactly the failure mode
  `Documentation/SMYSL_RATIONALE.typ` opens with: paraphrase a document enough
  times and disagreement is the first thing to go, because nothing marks an
  objection as *load-bearing* to the claim it argues with — a summariser
  keeps the confident-sounding claim and drops the hedge sitting next to it.
  `pack` is the alternative: budget-bounded selection where a claim's
  rebuttal is not optional cargo. If the budget cannot hold a claim *and*
  what argues with it, the claim does not travel alone — the whole operation
  fails, loudly, rather than shipping something that reads as uncontested
  when it was not.
]

#section("The seven constraints")

Every pack is checked against seven named constraints (`smysl-pack`'s
`constraints.rs`). Six are closure obligations — things a selected unit
drags in with it — and the seventh is the budget itself. `--explain` prints
the constraint code next to every forced unit, so this table is the key to
reading that output rather than guessing at it from the letter:

#dtable(
  (auto, auto, 1fr),
  (
    ([Code], [Binds when], [What it demands]),
    ([C1], [a unit sits at L1 or above], [Its `deps` — interpretive prerequisites — must also be selected, at least at L0. A claim about "p95" cannot be understood without the definition of p95 sitting next to it.]),
    ([C2], [a unit sits at L1 or above], [Its `grounds` — the evidence it rests on — must also be selected. This is the ordinary evidential floor: a claim at depth without its support is a claim with nothing under it.]),
    ([C3], [always, at every level, including L0], [Every unit that rebuts it must also be selected. This is *rule R*, the constraint the whole chapter is really about: a claim never travels unopposed, even as a bare gist.]),
    ([C4], [a unit is a position in an open contention], [Every other position in that contention must also be selected — a live disagreement is presented whole or not at all, never as one side.]),
    ([C5], [a unit is pinned], [It must reach L1. Pinning is how `--focus` and an active thread's referenced units make a demand: not "include this," but "include this at enough depth to read."]),
    ([C6], [a unit sits at L1 or above], [Any unit it names in a `warrant` edge — the inferential licence for the step it took — must also be selected, at L0.]),
    ([C7], [always], [Total selected cost must not exceed the budget. The one constraint that is a number rather than a graph shape.]),
  ),
)

`--explain` reports the *reason* a unit is in using a slightly different
vocabulary — `Reason` in `closure.rs` — because a reason names *what pulled
it in*, not just which rule fired:

#dtable(
  (auto, auto, 1fr),
  (
    ([`--explain` says], [Constraint], [Meaning]),
    ([`in focus`], [C5], [Named directly in `--focus`.]),
    ([`pinned by the thread`], [C5], [Named by the active thread's steps, when packing under `--thread`.]),
    ([`rebuts U`], [C3], [It rebuts unit `U`, which is in the selection — this is rule R showing its work.]),
    ([`contests U`], [C4], [It is the other position in an open contention that touches `U`.]),
    ([`dep of U`], [C1], [`U` needs it to be interpretable at L1+.]),
    ([`ground of U`], [C2], [`U` needs it as evidence at L1+.]),
    ([`warrant of U`], [C6], [`U` needs it as the licence for an inference at L1+.]),
    ([`earned on density`], [`-`], [Nothing forced it in — it won a place by value per token, same as any other budget-driven selection.]),
  ),
)

#section("A worked pack, read line by line")

`fixtures/corpus/F1-incident.smy` is the postmortem fixture used throughout
this book: eight units arguing that a saturated connection pool in `eu-west`
caused an auth-latency regression, with one canary reading that complicates
the story. `c/pool-saturation` is the contested claim, and its uid — a real,
resolved content hash, not a label — is what `--focus` takes:

#callout(label: "Note")[
  `--focus`, `--seed`, and similar flags take a *uid*, never a label. Labels
  are a surface-file convenience for the person writing the `.smy` file; a
  uid is the only identifier the store actually resolves. Passing a label
  fails cleanly rather than silently doing the wrong thing:

  #screen(caption: "$ smysl pack --budget 200 --focus c/pool-saturation fixtures/corpus/F1-incident.smy")[
```
smysl pack: `c/pool-saturation` is not a uid
```
  ]

  Mine the real uid from `thread --show` (Chapter 20 covers it directly) or
  `salience`, then pass that instead.
]

#screen(caption: "$ smysl --format surface pack --budget 200 --explain --focus b3:cvhirtgs2mpvli2ethhyeo32uf fixtures/corpus/F1-incident.smy")[
```
b3:cvhirtgs2mpvli2ethhyeo32uf @L1  C5  in focus
b3:ekitkvj75uvgzxpvq3ad2nrv3b @L0  -  earned on density
b3:izyuzlt42mqcvgdfb4nfpllxyq @L0  C2  ground of b3:cvhirtgs2mpvli2ethhyeo32uf
b3:js4xzessu5zwjpv2rawtugnuvj @L0  -  earned on density
b3:phsoomklkmlq3sjvbe6cyuqy5v @L0  C3  rebuts b3:cvhirtgs2mpvli2ethhyeo32uf
b3:re42iey2e7syg6zp73tfrlqbvh @L0  -  earned on density
b3:wo4t2c46lq45fnakd6tajlgcac @L1  -  earned on density
b3:xkys7j42mcuyiaxiyh73xddimr dropped: low-value
fixtures/corpus/F1-incident.smy: 7 of 8 unit(s), 193 of 200 tokens, greedy mode, gap 0.010
```
]

Cross-referencing this against the surface excerpt the same command writes
to stdout resolves every uid to a label:

#dtable(
  (auto, auto, 1fr),
  (
    ([Unit], [Level], [Why it is here]),
    ([`c/pool-saturation`], [L1], [The focus itself. Pinned by C5, so it reaches full depth.]),
    ([`e/pool-wait`], [L0], [C2: the evidence `c/pool-saturation` grounds itself in. Without it the focus would be a claim resting on nothing visible.]),
    ([`c/canary-clean`], [L0], [C3, the interesting line: this unit rebuts the focus (`@rel c/canary-clean --rebuts--> c/pool-saturation` in the source), so it is *forced* in, not merely likely to be picked.]),
    ([`d/p95`, `f/root-cause`, `e/trace`], [L0], [Earned their place on density — no constraint demanded them, they were simply worth their token cost.]),
    ([`c/regression`], [L1], [Also earned on density, but at L1 rather than L0 — greedy bought it depth because the budget had room and the value justified it.]),
    ([`e/canary`], [dropped], [`low-value`: below the value floor at every level it could be bought at, and nothing forces it in.]),
  ),
)

The footer line is the packinfo, spoken: *7 of 8 units, 193 of 200 tokens,
greedy mode, gap 0.010.* Read the middle line of the whole example again —
`b3:phsoo… rebuts b3:cvhi…` — because that line is the same argument
`SMYSL_RATIONALE.typ`'s section on compression makes, compressed into one
row of output: what
survived was not "the most important-sounding claim," it was *the claim and
whatever argues with it*, selected together or not at all.

#subsection("Budget syntax: plain numbers and the `k` suffix")

`--budget` takes a token count, optionally with a `k` suffix meaning
thousands (`parse_budget` in `main.rs`: strip a trailing `k`/`K`, parse the
remainder, multiply by `1000`; anything else parses as a literal integer).
`8000` and `8k` are the same budget:

#screen(caption: "$ smysl --format surface pack --budget 1k --explain fixtures/corpus/F1-incident.smy")[
```
fixtures/corpus/F1-incident.smy: 8 of 8 unit(s), 244 of 1000 tokens, greedy mode, gap 0.000
```
]

A thousand-token budget is generous enough to take the whole eight-unit
store at full depth, which is also the cleanest way to see what "nothing
was left behind" looks like: `gap 0.000` with every unit selected is the
uninteresting, correct baseline every tighter budget below is measured
against.

#subsection("Naming more than one focus")

`--focus` can be repeated. Each named unit is pinned independently, and
their closures are computed and merged — asking for two focuses is not the
same as asking for one twice as important, it is asking for two independent
floors that both must be met:

#screen(caption: "$ smysl pack --budget 400 --explain --focus b3:cvhirtgs2mpvli2ethhyeo32uf --focus b3:js4xzessu5zwjpv2rawtugnuvj fixtures/corpus/F1-incident.smy")[
```
b3:cvhirtgs2mpvli2ethhyeo32uf @L1  C5  in focus
b3:ekitkvj75uvgzxpvq3ad2nrv3b @L0  -  earned on density
b3:izyuzlt42mqcvgdfb4nfpllxyq @L0  C2  ground of b3:cvhirtgs2mpvli2ethhyeo32uf
b3:js4xzessu5zwjpv2rawtugnuvj @L1  C5  in focus
b3:phsoomklkmlq3sjvbe6cyuqy5v @L0  C3  rebuts b3:cvhirtgs2mpvli2ethhyeo32uf
b3:re42iey2e7syg6zp73tfrlqbvh @L0  -  earned on density
b3:wo4t2c46lq45fnakd6tajlgcac @L2  C2  ground of b3:js4xzessu5zwjpv2rawtugnuvj
b3:xkys7j42mcuyiaxiyh73xddimr @L0  -  earned on density
fixtures/corpus/F1-incident.smy: 8 of 8 unit(s), 244 of 400 tokens, greedy mode, gap 0.000
```
]

The second focus (`f/root-cause`, the finding) pulls its own grounds in at
C2 — one of which, `c/regression`, ends up at L2 because greedy had budget
left after meeting both floors and bought it full depth. With two focuses
and a comfortable budget everything in the store ends up selected anyway, so
this example is really about the *mechanism*: each `--focus` is its own
demand, checked and satisfied independently, before density gets to spend
what is left.

#subsection("Capping detail with `--lod`")

`--lod L0`/`L1`/`L2` caps *every* unit at that level, whatever it was
authored at and whatever a constraint would otherwise buy it. Because
depth costs tokens, a cap can change what fits — the same budget that drops
a unit at full depth can afford every unit once none of them is allowed past
a gist:

#dtable(
  (auto, auto, auto, 1fr),
  (
    ([Command], [Units in], [Tokens used], [What changed]),
    ([`pack --budget 200 --focus …`], [7 of 8], [193 of 200], [`e/canary` dropped as `low-value`; nothing left to buy it with.]),
    ([`pack --budget 200 --focus … --lod L0`], [8 of 8], [137 of 200], [Every unit capped at L0 costs less, so the same 200-token budget now holds the whole store, gap 0.000.]),
  ),
)

#screen(caption: "$ smysl pack --budget 200 --explain --focus b3:cvhirtgs2mpvli2ethhyeo32uf --lod L0 fixtures/corpus/F1-incident.smy")[
```
b3:cvhirtgs2mpvli2ethhyeo32uf @L0  C5  in focus
b3:ekitkvj75uvgzxpvq3ad2nrv3b @L0  -  earned on density
b3:izyuzlt42mqcvgdfb4nfpllxyq @L0  -  earned on density
b3:js4xzessu5zwjpv2rawtugnuvj @L0  -  earned on density
b3:phsoomklkmlq3sjvbe6cyuqy5v @L0  C3  rebuts b3:cvhirtgs2mpvli2ethhyeo32uf
b3:re42iey2e7syg6zp73tfrlqbvh @L0  -  earned on density
b3:wo4t2c46lq45fnakd6tajlgcac @L0  -  earned on density
b3:xkys7j42mcuyiaxiyh73xddimr @L0  -  earned on density
fixtures/corpus/F1-incident.smy: 8 of 8 unit(s), 137 of 200 tokens, greedy mode, gap 0.000
```
]

Notice the focus itself is now `@L0` rather than `@L1` — a cap wins over a
pin's usual promise to reach L1 (C5's own text: a gist-only unit is
satisfied by L0 because there is nothing higher to buy; `--lod L0` puts
every unit, including the focus, in exactly that position on purpose). What
you get back is breadth without depth: every claim named, none of them
argued out past a sentence. Whether that trade is the right one depends on
what happens next to the pack — Chapter 24 covers the render side of that
same trade in `--lod`'s other home.

#subsection("Two ways to solve: `--mode greedy` and `--mode exact`")

The default solver is greedy by density (`solve.rs`): fast, and for this
fixture at this budget, exactly as good as anything else could do — but
*greedy is not proven optimal, it is merely checked afterward*.

There used to be a local-improvement pass after it, which downgraded the least
valuable depth and spent what that freed on breadth. It was removed in 0.8
because it was finally measured: across 28 000 generated packs it changed 26,
and 22 of those 26 came out *worse* by the value function it existed to
maximise. It fired on 0.09% of packs, which is how it survived eight releases
looking harmless. `--mode exact` asks for a proof instead, by
branch-and-bound search over closure-complete anchors, pruned against a
fractional-relaxation upper bound (`exact.rs`). That search is compiled in
only behind the `exact-pack` feature — the default build does not carry it:

#screen(caption: "$ smysl pack --budget 200 --explain --focus b3:cvhirtgs2mpvli2ethhyeo32uf --mode exact fixtures/corpus/F1-incident.smy   (default build)")[
```
fixtures/corpus/F1-incident.smy: warning: SMY-W202: exact packing is not compiled in; rebuild with the `exact-pack` feature
b3:cvhirtgs2mpvli2ethhyeo32uf @L1  C5  in focus
...
fixtures/corpus/F1-incident.smy: 7 of 8 unit(s), 193 of 200 tokens, exact mode, gap 0.010
```
]

`cargo build --features exact-pack` from the repository root does build it
— it is a real, working feature, not a stub — and rerunning the identical
command against that binary gets you the proof rather than the warning:

#screen(caption: "$ smysl pack --budget 200 --explain --focus b3:cvhirtgs2mpvli2ethhyeo32uf --mode exact fixtures/corpus/F1-incident.smy   (built with --features exact-pack)")[
```
b3:cvhirtgs2mpvli2ethhyeo32uf @L1  C5  in focus
...
fixtures/corpus/F1-incident.smy: 7 of 8 unit(s), 193 of 200 tokens, exact mode, gap 0.000 (proven optimal)
```
]

Same selection, same eight units — but the `gap` field is the whole point of
the exercise, and it is worth being precise about what it means (`Optimality`
in `smysl-core`'s `aux.rs`): *an upper bound on the value left on the table,
in `[0, 1]`; zero means proven optimal.* Greedy's `0.010` on this fixture is
not "10% of the tokens wasted" — it is a provable ceiling on how much better
an optimal pack *could* have scored, computed from the fractional relaxation
of the remaining budget. On this small, easy fixture the ceiling was low and
exact mode found nothing greedy had missed. On a larger store with a tighter
budget the two numbers can diverge a lot more, and that is precisely the
situation `--mode exact` exists for: a gap you can act on ("this greedy pack
might be leaving real value on the table, go check") rather than one you
have to take on faith.

#callout(label: "Note")[
  Branch-and-bound is worst-case exponential — the problem is NP-hard — so
  `exact.rs` caps the search at `NODE_LIMIT` (250,000 nodes) and, above
  `EXACT_THRESHOLD` (256 units) in scope, declines to run it at all and packs
  greedily instead, saying so via `SMY-W202` rather than hanging. Hitting
  either limit does not invalidate the pack — the incumbent selection is
  still valid — it just means the reported gap is an estimate rather than a
  proof for that run.
]

#subsection("The cost model: `--tokenizer`")

Packing has to be pure (rule D: same graph, same budget, same thread yields
identical bytes everywhere), and no provider's real tokeniser is available
offline or stable across its own versions — so `smysl` ships one
deterministic, approximate estimator and records which one produced every
pack, always:

#term("Estimator")[
  `smysl`'s bundled cost model, identified by `smysl/utf8-div4`
  (`DEFAULT_ESTIMATOR` in `smysl-pack`). The formula is
  `cost(text) = ceil(utf8_len(text) / 4) + 2` — a quarter-byte-per-token
  approximation plus two tokens of per-item framing overhead. It is not
  claiming to match any real model's tokeniser; it is claiming to be
  *deterministic and disclosed*, which a real tokeniser bundled offline
  could not promise to stay.
]

Every `packinfo` carries the estimator's id regardless of whether you ever
pass `--tokenizer` (D-2: a budget that does not say what it was counted with
is a number without a unit). Today there is exactly one estimator shipped
(`Estimator::ALL` has one member), so `--tokenizer smysl/utf8-div4` is a
no-op that happens to match the default — the flag exists so that a second
estimator, when one ships, has somewhere to be named without a wire-format
change:

#screen(caption: "$ smysl pack --budget 200 --tokenizer tiktoken/cl100k fixtures/corpus/F1-incident.smy")[
```
smysl pack: unknown tokenizer `tiktoken/cl100k`
```
]

#subsection("When the floor doesn't fit: infeasible budgets")

C3 is not a suggestion. If the mandatory floor — the focus plus everything
C1 through C6 force in around it — costs more than the budget, packing does
not quietly ship the focus alone and call it done. It fails, with the exact
minimum budget that would have worked:

#screen(caption: "$ smysl pack --budget 45 --focus b3:cvhirtgs2mpvli2ethhyeo32uf fixtures/corpus/F1-incident.smy")[
```
smysl pack: SMY-E200: budget 45 but the mandatory floor needs 46
```
]

The exit code is `4` (`PackInfeasible` in the exit-code contract, Appendix
C) — distinct from an ordinary failure, so a caller can tell "this budget
was too small" apart from "something else went wrong" without parsing
stderr. And the reported minimum is exact, not a rounded estimate — one
token less than it and the pack still fails; exactly it and the pack
succeeds, buying precisely the focus, its ground, and its rebuttal, with
every other unit priced out on budget alone:

#screen(caption: "$ smysl --format surface pack --budget 46 --explain --focus b3:cvhirtgs2mpvli2ethhyeo32uf fixtures/corpus/F1-incident.smy")[
```
b3:cvhirtgs2mpvli2ethhyeo32uf @L1  C5  in focus
b3:izyuzlt42mqcvgdfb4nfpllxyq @L0  C2  ground of b3:cvhirtgs2mpvli2ethhyeo32uf
b3:phsoomklkmlq3sjvbe6cyuqy5v @L0  C3  rebuts b3:cvhirtgs2mpvli2ethhyeo32uf
b3:ekitkvj75uvgzxpvq3ad2nrv3b dropped: budget
b3:js4xzessu5zwjpv2rawtugnuvj dropped: budget
b3:re42iey2e7syg6zp73tfrlqbvh dropped: budget
b3:wo4t2c46lq45fnakd6tajlgcac dropped: budget
b3:xkys7j42mcuyiaxiyh73xddimr dropped: low-value
```
]

This is the whole chapter in one pair of runs: at 45 tokens there is no
version of this pack that presents `c/pool-saturation` without also
presenting the reading that complicates it, so `pack` refuses outright
rather than pick one. At 46 tokens it delivers exactly that pair, nothing
more. A budget failing loudly here is not a bug to work around — it is the
one behaviour that makes every *successful* pack trustworthy without your
having to re-audit it by hand.

#whatsnext[
  A packed, budget-bounded excerpt like the ones above is exactly the shape
  you hand to a model with a context limit, or to a person who does not have
  time for the whole store — that is the whole job this chapter's command
  does. Two things follow from here. If what you actually want is prose
  rather than a truncated surface excerpt, Chapter 24's `render` is the
  command that turns a selection into an artifact meant to be *read*, not
  decoded; `pack`'s non-surface output — the default CBOR form this chapter
  mostly suppressed with `--format surface` for readability — is a portable
  sub-store in its own right, small enough to ship and still a real store
  any later command can open. And a pack has no opinion yet about *order* —
  it says what survives, not what order a reader should meet it in. That
  question is Chapter 20's.
]

#exercises((
  [Run `smysl pack --budget 60 --explain --format surface
   fixtures/corpus/F1-incident.smy`. Three units survive and five are dropped
   under *three different reasons*: `budget`, `low-value`, and `closure-cost`.
   Explain what makes `closure-cost` a different kind of rejection from
   `budget`.],
  [In that same run, `f/root-cause` is kept while `c/pool-saturation`, one of
   the two units it grounds on, is dropped. That looks like a closure
   violation. Consult the constraint table at the top of this chapter and
   explain why it is not.],
  [Feed the packed surface output back into `check`. It reports `SMY-E031` and
   `SMY-E032` — the packed document does not validate. Is that a bug? Answer
   before reading on.],
))

#answers((
  [`budget` means the unit was affordable in isolation and there was no room
   left by the time the packer reached it. `closure-cost` means the unit itself
   was cheap but bringing it *legally* was not — selecting it would have
   obliged the packer to bring its grounds, or its rebuttals under rule R, and
   the whole bundle did not fit. The distinction is worth having because the
   fixes differ: a `budget` drop is answered by raising the budget, a
   `closure-cost` drop often by restructuring what the unit depends on.],
  [Because the closure constraints are level-dependent. C1, C2 and C6 — deps,
   grounds, warrants — bind only at L1 and above. `f/root-cause` was selected
   at *L0*, a bare gist, where the only constraint that still binds is C3:
   rule R, rebuttals. A gist is an assertion the reader can see is
   unelaborated; an L1 block that showed its reasoning while hiding the ground
   that reasoning rests on would be the dishonest case, and that is the one
   the constraints forbid.],
  [Not a bug — but the packed output is not a document. At L0 a unit is emitted
   as a gist with its fields stripped, so a `derived` claim arrives with no
   `grounds` and a `cited` definition with no `source`, which is exactly what
   `SMY-E031` and `SMY-E032` describe. A pack is a *payload* built to fit a
   budget, with a `@packinfo` receipt saying what was dropped and degraded; it
   is meant to be read, not re-validated as a store. If you want something that
   checks clean, `bundle` a view — closure there is the whole point.],
))

#recap((
  [Seven constraints, C1 through C7, govern every pack; six are closure
   obligations and the seventh is the budget. `--explain` names which one
   forced each unit in, and rule R (C3) is the one that matters most: a
   selected claim's rebuttals travel with it, always, at every level.],
  [`--budget` takes a plain integer or a `k`-suffixed thousands shorthand;
   `--focus` may be repeated, each occurrence an independent floor that must
   be met; `--lod` caps every unit's depth and can change what fits inside
   an unchanged budget.],
  [`--mode exact` asks for a branch-and-bound proof instead of a greedy
   estimate; it needs the `exact-pack` feature to run at all, and its `gap`
   field is a provable ceiling on value left on the table, not a rounded
   guess — `0.0` means proven optimal.],
  [Every pack records the estimator that costed it, whether or not
   `--tokenizer` was passed, because a budget without a disclosed cost model
   is a number without a unit.],
  [A budget too small for the mandatory floor fails outright, with exit code
   `4` and the exact minimum that would have worked — never a one-sided pack
   that looks uncontested because it was too small to hold the objection.],
))

#chapter(number: 20, title: "thread — Deriving Structure")

`salience` ranks. `pack` compresses to a budget. Neither one arranges what
survives into a path a specific reader is meant to walk. A finding, its
grounds, and the claim that rebuts it can all clear the same budget and
still arrive as an unordered pile — true, complete, and no shape at all. A
`thread` is what gives a selection of units a *reading order* and says what
role each one plays in it, for one named audience at a time.

#callout(label: "Why")[
  The same graph can be a good brief for an executive, a good walkthrough
  for the engineer who has to fix it, and a good Q&A transcript for the
  person who only asked one question. Nothing about the underlying units
  changes between those three readings — only which units matter, in what
  order, and under what label. Rather than write three summaries by hand
  and hope they stay consistent with each other and with the graph, `thread`
  derives all three *from* the graph, deterministically, so the same store
  always yields the same brief, the same narrative, the same Q&A — and a
  later change to the graph is a re-derivation, not a rewrite.
]

#section("Roles, schemas, and arities")

#term("Thread schema")[
  One of five closed shapes a thread can take in `smysl` 0.1: `analysis`,
  `narrative`, `brief`, `qa`, `plan`. Each schema declares an ordered list of
  *roles* — `bottom-line`, `support`, `setup`, `finding`, and so on — and,
  for each role, how many units it may hold. The set is closed by design
  (D-4): the rule language that assigns units to roles is not yet stable
  enough to expose as a user-facing extension point, so a sixth schema
  waits for a later format version rather than shipping something that
  would then have to be supported forever.
]

#term("Role")[
  A named slot in a schema's narrative order — `finding`, `risk`, `question`,
  `step`, and twenty more across the five schemas, sharing one wire-stable
  numbering. A role belongs to whichever schemas declare it; `risk` happens
  to appear in both `brief` and `plan` as the same role, not two
  coincidentally-named ones.
]

Every schema's table is small enough to read whole. Weight is what a role
contributes to salience's role-weight term when a thread is active — the
schema's own statement of which of its roles matters most, independent of
where that role sits in the reading order:

#dtable(
  (auto, auto, auto, auto),
  (
    ([Schema], [Roles, in order], [Arity], [Weight]),
    ([`analysis`], [context], [0..2], [0.4]),
    ([], [tension], [1..2], [0.8]),
    ([], [approach], [0..2], [0.5]),
    ([], [finding], [1..3], [1.0]),
    ([], [rebuttal], [0..3], [0.9]),
    ([], [implication], [0..2], [0.6]),
    ([], [next], [0..1], [0.3]),
    ([`narrative`], [setup], [1..1], [0.6]),
    ([], [complication], [0..2], [0.8]),
    ([], [turn], [0..2], [1.0]),
    ([], [resolution], [0..2], [0.9]),
    ([], [coda], [0..1], [0.5]),
    ([`brief`], [bottom-line], [1..1], [1.0]),
    ([], [support], [1..3], [0.7]),
    ([], [risk], [0..2], [0.8]),
    ([], [ask], [0..1], [0.5]),
    ([`qa`], [question], [1..1], [0.9]),
    ([], [evidence], [0..3], [0.6]),
    ([], [answer], [1..2], [1.0]),
    ([], [caveat], [0..2], [0.7]),
    ([`plan`], [goal], [1..1], [1.0]),
    ([], [constraint], [0..3], [0.6]),
    ([], [step], [1..5], [0.8]),
    ([], [decision], [0..2], [0.7]),
    ([], [risk], [0..2], [0.7]),
  ),
)

Four schemas assign roles by *kind* — a rule table matches a unit's type,
its status, or which edges point at or from it, first match wins.
`narrative` is the one exception: it assigns by *position* in the graph's
ordering chain (`--sequences-->` edges), because a narrative is about
sequence, not about what kind of thing each beat is.

#section("Deriving all five, one worked example each")

Derivation is a pure function of the graph — no model is consulted, so the
same store always derives the same thread on any machine (rule D again; the
same discipline that makes `pack` reproducible). `--explain` prints how many
units landed in each role against its declared arity, names any required
role nothing could fill, and lists what coherence repair added.

#subsection("`analysis` — F6-adversarial")

`F6-adversarial.smy` is a small, deliberately loose corpus: a hunch, a
rumour, and two claims built on top of them. It happens to derive a clean
`analysis` thread — every required role filled, nothing left over:

#screen(caption: "$ smysl thread --derive analysis --explain --only fixtures/corpus/F6-adversarial.smy")[
```
fixtures/corpus/F6-adversarial.smy: context      0 of 0..2
fixtures/corpus/F6-adversarial.smy: tension      1 of 1..2
fixtures/corpus/F6-adversarial.smy: approach     1 of 0..2
fixtures/corpus/F6-adversarial.smy: finding      1 of 1..3
fixtures/corpus/F6-adversarial.smy: rebuttal     0 of 0..3
fixtures/corpus/F6-adversarial.smy: implication  2 of 0..2
fixtures/corpus/F6-adversarial.smy: next         0 of 0..1
fixtures/corpus/F6-adversarial.smy: 0 unit(s) not selected
@thread t/derived-analysis { schema: analysis, owner: tool:smysl, ts: [0, 0] }
~ The eu-west capacity shortfall is the root cause of the auth regression.
  tension → o/rumour
  approach → h/guess
  finding → f/laundered
  implication → c/laundered-once
  implication → c/laundered-twice
```
]

`o/rumour` (an `@observation`) took `tension`, `h/guess` (a `@hypothesis`)
took `approach`, and the two claims built on top of them landed as
`implication` — exactly the rule table's stated mapping (`Matcher::Type` on
observation, hypothesis, and claim respectively), with the finding itself
weighing heaviest in the gist that opens the thread.

#subsection("`narrative` — F3-postmortem")

`F3-narrative.smy` is a five-beat prose postmortem already linked end to end
with `--sequences-->` edges, and it already carries an authored `t/story`
thread. Deriving a *fresh* one under a different id, over the same graph,
reproduces the same five-beat shape from the edges alone — evidence that the
positional rule table is doing real work, not just repeating what a human
already typed:

#screen(caption: "$ smysl thread --derive narrative --id t/derived-narrative --only fixtures/corpus/F3-narrative.smy")[
```
@thread t/derived-narrative { schema: narrative, owner: tool:smysl, ts: [0, 0] }
~ The pool wait metric had been visible the whole time, on a dashboard
  nobody opened.; Rolling back the shard restored…
  setup → p/setup
  complication → p/complication
  turn → p/turn
  resolution → p/resolution
  coda → p/coda
```
]

The unit with nothing sequenced before it (`p/setup`) opens; the one nothing
follows (`p/coda`) closes; the three in between land in thirds of the
remaining chain. Every role is exactly one unit deep here because the chain
itself is exactly five units long — a longer chain bands the same way, just
with more than one unit per middle role, up to each role's arity.

#subsection("`brief` and `qa` — F4 checkout-latency")

`F4-qa.smy` is written as a support session: a question, the evidence
chased down in answering it, and a caveat about over-attributing the fix.
It derives a full `brief` and a full `qa` thread from the same nine units —
same graph, two different readings, because the two schemas' rule tables
route the same units to different roles for different purposes:

#screen(caption: "$ smysl thread --derive brief --only fixtures/corpus/F4-qa.smy")[
```
@thread t/derived-brief { schema: brief, owner: tool:smysl, ts: [0, 0] }
~ The 4.3 serialiser change introduced an n-plus-one, made visible by peak
  load.; Attributing the regression to the…
  bottom-line → f/answer
  support → e/black-friday
  support → c/load-contribution
  support → c/n-plus-one
  risk → c/not-only-deploy
  ask → q/checkout-latency
```
]

#screen(caption: "$ smysl thread --derive qa --only fixtures/corpus/F4-qa.smy")[
```
@thread t/derived-qa { schema: qa, owner: tool:smysl, ts: [0, 0] }
~ The 4.3 serialiser change introduced an n-plus-one, made visible by peak
  load.; Why did checkout latency regress after…
  question → q/checkout-latency
  evidence → e/black-friday
  evidence → c/load-contribution
  evidence → c/n-plus-one
  answer → f/answer
  caveat → c/not-only-deploy
```
]

The finding (`f/answer`) is the `bottom-line` in one reading and the
`answer` in the other; the original question, asked before anyone had
looked at a trace, is a supporting `ask` in the brief and the `question`
that opens the Q&A. Same nine units, same edges, two audiences.

#subsection("`plan` — F1-incident")

Nothing in `F1-incident.smy` was authored as a plan, but a `@finding` reads
as a `goal`, its grounds and the evidence under them read as `step`s, and
the unit that rebuts the leading claim reads as the one `risk` worth
flagging — which is exactly what the `plan` rule table does with them:

#screen(caption: "$ smysl thread --derive plan --only fixtures/corpus/F1-incident.smy")[
```
@thread t/derived-plan { schema: plan, owner: tool:smysl, ts: [0, 0] }
~ Pool saturation is the leading cause but is not consistent with the
  canary.; The 95th percentile of request latency…
  goal → f/root-cause
  step → d/p95
  step → e/pool-wait
  step → e/trace
  step → c/regression
  step → c/pool-saturation
  risk → c/canary-clean
```
]

This is not a plan anyone would call well-written — a postmortem's units are
not steps toward a goal, they are the argument for a root cause — and that
mismatch is the honest lesson of this example: derivation always produces
*a* thread that satisfies the schema's arities, but whether that schema was
the right lens for this graph is a judgment call the tool does not make for
you. Reach for `plan` on a graph that has genuine goals, constraints, and
decisions in it; reach for `analysis` or `brief` on one that does not.

#section("Widening a role: `--arity`")

A schema's arity is a default ceiling, not a hard limit on the graph — it
can be overridden per role with `--arity ROLE=N`. `F7-mixed-granularity.smy`
has six units that would plausibly support its `brief`'s bottom line, but
`brief`'s default `support` arity is `1..3`, so three of them sit out:

#screen(caption: "$ smysl thread --derive brief --explain --only fixtures/corpus/F7-mixed-granularity.smy")[
```
fixtures/corpus/F7-mixed-granularity.smy: bottom-line  1 of 1..1
fixtures/corpus/F7-mixed-granularity.smy: support      3 of 1..3
fixtures/corpus/F7-mixed-granularity.smy: risk         2 of 0..2
fixtures/corpus/F7-mixed-granularity.smy: ask          0 of 0..1
fixtures/corpus/F7-mixed-granularity.smy: 3 unit(s) not selected
```
]

#screen(caption: "$ smysl thread --derive brief --arity support=6 --explain --only fixtures/corpus/F7-mixed-granularity.smy")[
```
fixtures/corpus/F7-mixed-granularity.smy: bottom-line  1 of 1..1
fixtures/corpus/F7-mixed-granularity.smy: support      6 of 1..3
fixtures/corpus/F7-mixed-granularity.smy: risk         2 of 0..2
fixtures/corpus/F7-mixed-granularity.smy: ask          0 of 0..1
fixtures/corpus/F7-mixed-granularity.smy: 0 unit(s) not selected
```
]

`--arity support=6` widens exactly that one role; every other role's ceiling
is untouched, and `smysl` refuses an override for a role the schema does not
declare at all (`{schema} has no {role} role`, a usage error) rather than
silently accepting a slot that would never be read.

#section("Restricting the graph: `--scope`")

`--scope` limits which units derivation is even allowed to see, by uid — the
rest of the store still exists but is invisible to this derivation, the same
restriction `pack --focus` uses for its own closure. Scoping `F1-incident`
down to four units before deriving a `brief` produces a thread built only
from what was named, nothing pulled in from outside it:

#screen(caption: "$ smysl thread --derive brief --explain --only --scope … fixtures/corpus/F1-incident.smy")[
```
$ smysl thread --derive brief --explain --only \
    --scope b3:izyuzlt42mqcvgdfb4nfpllxyq --scope b3:cvhirtgs2mpvli2ethhyeo32uf \
    --scope b3:phsoomklkmlq3sjvbe6cyuqy5v --scope b3:js4xzessu5zwjpv2rawtugnuvj \
    fixtures/corpus/F1-incident.smy
fixtures/corpus/F1-incident.smy: bottom-line  1 of 1..1
fixtures/corpus/F1-incident.smy: support      2 of 1..3
fixtures/corpus/F1-incident.smy: risk         1 of 0..2
fixtures/corpus/F1-incident.smy: ask          0 of 0..1
fixtures/corpus/F1-incident.smy: 0 unit(s) not selected
@thread t/derived-brief { schema: brief, owner: tool:smysl, ts: [0, 0] }
~ Pool saturation is the leading cause but is not consistent with the
  canary.; The canary rules out a pure configuration…
  bottom-line → f/root-cause
  support → e/pool-wait
  support → c/pool-saturation
  risk → c/canary-clean
```
]

Compare this against the unscoped `brief` derivation earlier in this
chapter, over the full eight-unit store: `d/p95` and `c/regression` are
gone, because they were never in scope to be picked, and `0 unit(s) not
selected` here means exactly that — every unit *this derivation was allowed
to see* found a role, which is a different and narrower claim than "every
unit in the store found a role."

#section("Identity is `(id, owner)`")

#term("Thread identity")[
  A thread's register key is the pair `(id, owner)`, not the id alone
  (`Thread::register_key` in `smysl-core`). Two threads with the same id and
  the *same* owner are the same register, resolved last-writer-wins by
  timestamp; two threads with the same id and *different* owners are two
  independent registers that happen to share a name. Neither has to
  coordinate with the other, and neither ever overwrites the other.
]

This is why `--as` matters as much as `--id`: it is not a display label, it
is half of what makes a thread's name safe to reuse. Two agents deriving a
brief under the same id `t/exec`, as different owners, coexist in the same
store without either clobbering the other:

#screen(caption: "$ smysl thread --derive brief --id t/exec --as human:alice … | smysl thread --derive brief --id t/exec --as human:bob … | smysl thread --list")[
```
$ smysl thread --derive brief --id t/exec --as human:alice --only F1-incident.smy >> two-owners.smy
$ smysl thread --derive brief --id t/exec --as human:bob   --only F1-incident.smy >> two-owners.smy
$ smysl thread --list two-owners.smy
t/brief  brief  3 step(s)  Auth p95 tripled in eu-west; pool saturation is leading but contested.
t/exec  brief  3 step(s)  Alice's cut: pool saturation is the story, canary is the caveat.
t/exec  brief  3 step(s)  Bob's cut: keep it to the regression and the one risk.
```
]

Three threads, one of them the file's original authored `t/brief`, two of
them named identically as `t/exec` — and `--list` shows all three without
complaint, because `owner` is part of what makes each one a distinct
register. If Alice and Bob had instead both derived under `--as
human:vladimir`, the second derivation would simply supersede the first as a
later write to the same `(id, owner)` key — the ordinary last-writer-wins
rule that governs every owned register in the format.

#section("Reading a thread: `--list` and `--show`")

`--list` is the inventory of every thread a store already holds, authored
or derived; `--show ID` walks one thread step by step, resolving each uid to
the gist of the unit it names:

#screen(caption: "$ smysl thread --list fixtures/corpus/F1-incident.smy")[
```
t/brief  brief  3 step(s)  Auth p95 tripled in eu-west; pool saturation is leading but contested.
```
]

#screen(caption: "$ smysl thread --show t/brief fixtures/corpus/F1-incident.smy")[
```
t/brief  brief
~ Auth p95 tripled in eu-west; pool saturation is leading but contested.
  1. bottom-line  b3:js4xzessu5zwjpv2rawtugnuvj  Pool saturation is the leading cause but is not consistent with the canary.
  2. support      b3:cvhirtgs2mpvli2ethhyeo32uf  The eu-west connection pool is saturated.
  3. risk         b3:phsoomklkmlq3sjvbe6cyuqy5v  The canary rules out a pure configuration cause.
```
]

`t/brief` here is authored by hand in the fixture, not derived — a person
wrote exactly these three steps, in exactly this order, as the intended
reading of the incident. That is the other half of what a thread is for:
`--derive` is the tool proposing a reading; nothing stops a person from
writing one directly and having `thread --show` treat it identically.

#callout(label: "Note")[
  `--show`'s second column is the uid sitting right next to the gist it
  names — the practical technique this whole book leans on for finding a
  real uid to hand to `--focus`, `--seed`, or `--scope` elsewhere. Every
  worked `pack` example in Chapter 19 started life as a line from exactly
  this output.
]

#section("Foreign roles: reported, not rejected")

A schema's role table names which roles it *expects*; it is not a fence the
parser enforces. `Role::parse` accepts any of the twenty-four roles across
all five schemas, regardless of which schema a given thread declares — so a
`brief` thread with a step in `coda` (a `narrative` role `brief` never
lists) parses cleanly, shows cleanly, and passes `check` with zero
diagnostics:

#screen(caption: "hand-authored: a brief thread reaching for narrative's `coda` role")[
```
@thread t/odd-brief { schema: brief, owner: "human:vladimir" }
~ A brief that reaches for a role brief never declared.
  bottom-line → f/root-cause
  support → c/pool-saturation
  coda → c/canary-clean
```
]

#screen(caption: "$ smysl thread --show t/odd-brief foreign-role.smy")[
```
t/odd-brief  brief
~ A brief that reaches for a role brief never declared.
  1. bottom-line  b3:js4xzessu5zwjpv2rawtugnuvj  Pool saturation is the leading cause but is not consistent with the canary.
  2. support      b3:cvhirtgs2mpvli2ethhyeo32uf  The eu-west connection pool is saturated.
  3. coda         b3:phsoomklkmlq3sjvbe6cyuqy5v  The canary rules out a pure configuration cause.
```
]

#screen(caption: "$ smysl check foreign-role.smy")[
```
foreign-role.smy: 22 records, 8 units, 0 diagnostic(s)
```
]

Nothing here rejects the thread. That is deliberate, not an oversight:
`Thread::foreign_roles()` in `smysl-core` exists precisely to let a caller
ask the question directly — it returns the roles a thread uses that its own
declared schema does not list (`[coda]`, for this example) — and it is a
library-level check a consumer can run rather than a gate the parser
imposes. An *authored* thread is a person's stated intent, including,
sometimes, reaching outside a schema's usual vocabulary on purpose; a
*derived* one never does this, because `derive_thread` only ever assigns
roles its own schema table lists. The asymmetry is the point: derivation is
trusted to stay inside the lines because it is a pure function of a fixed
table, and a human is trusted to say what they meant even when it does not
fit the table — with the means to check, on demand, when it matters.

#whatsnext[
  A thread names an order and a role for each unit; it does not yet say
  what words those roles are rendered in, what register they read at, or
  what target format they land in — `smysl thread --derive schema | render
  --profile … --target …` is Chapter 24's command, and it composes directly
  with this one because a derived thread carries its originating store with
  it (`--only` is what strips that away, for exactly the cases where you
  want the thread record alone). Everything in this chapter has been about
  *which* reading; Chapter 24 is about how that reading actually gets said.
]

#exercises((
  [Run `smysl thread --derive analysis --format surface
   fixtures/corpus/F1-incident.smy` and read the derived thread's steps. Then
   derive under `brief` instead. `analysis` gives you six steps under
   `context`, `finding`, `rebuttal` and `implication`; `brief` gives five under
   `bottom-line`, `support` and `risk`. Same graph, same facts, different walk.
   What does that tell you about where a thread's meaning lives — in the units,
   or in the thread?],
  [The derived thread's owner is `tool:smysl` and its timestamp is `[0, 0]`.
   Both are deliberate. Explain the timestamp in terms of rule D, and the owner
   in terms of what a reader needs to know when two threads disagree.],
  [`thread --derive` is deterministic; `--refine` would not be. Given that the
   derived analysis thread is serviceable but plainly mechanical, argue for
   shipping the deterministic one anyway rather than waiting for the model.],
))

#answers((
  [In the thread. The units are the same facts either way — a schema decides
   which of them the reader is walked through, in what order, and under what
   label. `analysis` opens with context and works toward implications;
   `brief` leads with the bottom line. Writing three summaries by hand would
   mean three artifacts to keep in sync; deriving three threads over one graph
   means the facts cannot drift apart, because there is only one copy of them.],
  [A timestamp is a clock reading, and rule D says a pure operation must be a
   function of its inputs alone — so `thread --derive` supplies a fixed clock
   rather than reading the real one, and running it twice produces the same
   bytes. The owner matters for a different reason: when two threads over the
   same graph tell different stories, the first question is who made each, and
   `tool:smysl` versus `human:vladimir` is the difference between a
   disagreement worth investigating and a machine doing what it was told.],
  [Because a mechanical ordering that is always available, always free, and
   always the same beats a good ordering that costs a call, varies between
   runs, and cannot be regenerated years later to compare against what was
   signed off. The derived thread is a floor, not a ceiling — and a floor that
   a pipeline can depend on is worth more than a better answer it cannot.],
))

#recap((
  [A thread is a named, ordered, role-annotated walk over existing units —
   `salience` ranks and `pack` compresses, but neither one arranges a
   reading path for a specific audience the way a thread does.],
  [Five closed schemas — `analysis`, `narrative`, `brief`, `qa`, `plan` —
   each declare an ordered role list, a per-role arity, and a rule table
   that assigns units to roles; every one but `narrative` assigns by kind,
   `narrative` alone assigns by position in the graph's ordering chain.],
  [`--arity ROLE=N` widens one role's ceiling without touching the others;
   `--scope` restricts which units a derivation is even allowed to see,
   which changes what "nothing left over" means.],
  [Thread identity is the pair `(id, owner)`, not the id alone — two agents
   deriving under the same name are two independent registers, not a
   conflict, and only a later write from the *same* owner ever supersedes
   an earlier one.],
  [`--show` prints each step's uid next to its unit's gist — the practical
   way to mine a real uid for `pack --focus`, `salience --seed`, or
   `thread --scope` elsewhere in this book.],
  [A role outside a thread's declared schema is reported to a caller who
   asks (`Thread::foreign_roles()`), never silently rejected by the parser
   or `check` — a derived thread never produces one, because derivation is
   bound to its own schema's table by construction.],
))
