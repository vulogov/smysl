#import "design.typ": *

#chapter(number: 16, title: "view and bundle — Naming and Packaging Reachability")

You have a store — a fully-parsed, checked `.smy` file, or several merged
together. It might hold a hundred units built over months: measurements,
claims, findings, threads, contentions. Nobody wants a hundred units on
screen when they ask "what is the incident brief." They want the six units
that brief actually rests on, named once, so the next command — `salience`,
`pack`, `render` — knows where to start without re-deriving it.

That is what a view is for. It is not a folder you drop units into; it is a
*name for a starting point*, and the graph does the rest.

#section("A view is a root set, not a container")

#term("View")[
  A name plus a root set — nothing else. A view owns no units. Reachability
  does the owning: every unit that a root can reach by following its
  `grounds`, `deps`, and outgoing relation edges belongs to the view, at
  exactly zero copying cost, for as long as the store exists. `@doc`'s
  `roots` field is how a document declares its own default view; `smysl
  view --roots` builds another one on the fly without touching the file.
]

#callout(label: "Why")[
  A container has to decide, unit by unit, whether something is "in." Two
  views that both care about the same finding would each need their own
  copy of it, and the moment one copy changes, the two views disagree about
  a unit that is supposed to be the same thing. A view sidesteps the
  question entirely: a unit is in every view whose roots can reach it,
  simultaneously, for free. Combining two views is then a plain set union
  of their roots — no copying, no reconciling, no "which copy wins." That
  is only available to you because membership is computed, not stored.
]

#subsection("Printing an existing view")

`fixtures/corpus/F1-incident.smy` declares one view in its `@doc` header —
`v/f1`, rooted at a single finding, `f/root-cause`. Printing it with no
further arguments prints exactly that view, resolved:

#screen(caption: "$ smysl view fixtures/corpus/F1-incident.smy")[
```
v/f1: 1 root(s), 0 thread(s), 6 unit(s) reachable
  b3:cvhirtgs2mpvli2ethhyeo32uf
  b3:ekitkvj75uvgzxpvq3ad2nrv3b
  b3:izyuzlt42mqcvgdfb4nfpllxyq
  b3:js4xzessu5zwjpv2rawtugnuvj
  b3:re42iey2e7syg6zp73tfrlqbvh
  b3:wo4t2c46lq45fnakd6tajlgcac
```
]

The file has eight units total (confirmed in Chapter 11 by `pack --explain`,
which lists every one of them), but the view reaches only six. The two left
out are `c/canary-clean` and its evidence `e/canary`. That is not an
oversight in the fixture — it is the single most important thing to
understand about reachability before you build your own views:

#callout(label: "Why")[
  `f/root-cause`'s only `grounds` are `c/pool-saturation` and `c/regression`.
  `c/canary-clean` reaches the graph through
  `@rel c/canary-clean --rebuts--> c/pool-saturation` — and that edge runs
  *from* the rebuttal *to* the claim it rebuts, the direction the author
  wrote it in. Reachability follows edges the way they point. Starting from
  `f/root-cause` and walking forward, there is no outgoing edge from
  `c/pool-saturation` back to the thing objecting to it, so the objection is
  never visited. A view rooted downstream of a contested claim does not
  automatically pull in its objections; you get them by rooting on them
  directly, or by rooting further out.
]

#subsection("Building an ad-hoc view")

You do not need a unit's `@doc` to declare a view for you. `--roots` builds
one from the command line, over any real uid — never a label, which only
ever exists inside the one file's parse that wrote it. Rooted at
`c/pool-saturation` alone, the reachable set shrinks to five units — and
picks up a unit `f/root-cause`'s view never touched:

#screen(caption: "$ smysl view --roots b3:cvhirtgs2mpvli2ethhyeo32uf --id v/pool-only fixtures/corpus/F1-incident.smy")[
```
v/pool-only: 1 root(s), 0 thread(s), 5 unit(s) reachable
  b3:cvhirtgs2mpvli2ethhyeo32uf
  b3:ekitkvj75uvgzxpvq3ad2nrv3b
  b3:izyuzlt42mqcvgdfb4nfpllxyq
  b3:re42iey2e7syg6zp73tfrlqbvh
  b3:wo4t2c46lq45fnakd6tajlgcac
```
]

`c/pool-saturation` grounds only on `e/pool-wait` — one unit — yet the view
reaches five. The other three arrive through `@rel c/pool-saturation
--causes--> c/regression`: `causes` is one of the two support-bearing
relation kinds (with `answers`), so it is an outgoing edge exactly like a
`ground`, and walking it pulls in `c/regression` and, from there,
`c/regression`'s own grounds and deps (`e/trace`, `d/p95`). A relation edge
is not a footnote here — it is as real a path into the view as `grounds`
is.

Two more flags shape an ad-hoc view without changing what reachability
means: `--threads t/brief` publishes a named thread alongside the roots
(the reachable unit count does not change — a thread is a curated reading
order over units already in the view, not a new way to reach one); and
`--requires x.sre/mitigations` records which schema extensions a full-
fidelity reader of this view must understand, the same field `@doc
requires` fills in for a whole document. Multiple `--roots` behave exactly
like `@doc roots: [...]` with more than one entry — the view is the union
of what each root reaches:

#screen(caption: "$ smysl view --roots b3:cvhirtgs2mpvli2ethhyeo32uf --roots b3:phsoomklkmlq3sjvbe6cyuqy5v --id v/two-roots fixtures/corpus/F1-incident.smy")[
```
v/two-roots: 2 root(s), 0 thread(s), 7 unit(s) reachable
  b3:cvhirtgs2mpvli2ethhyeo32uf
  b3:ekitkvj75uvgzxpvq3ad2nrv3b
  b3:izyuzlt42mqcvgdfb4nfpllxyq
  b3:phsoomklkmlq3sjvbe6cyuqy5v
  b3:re42iey2e7syg6zp73tfrlqbvh
  b3:wo4t2c46lq45fnakd6tajlgcac
  b3:xkys7j42mcuyiaxiyh73xddimr
```
]

Rooting on the objection directly (`b3:phsoomklkmlq...`, `c/canary-clean`)
is exactly how you recover the unit `v/f1` left out: it and its own
evidence `e/canary` (`b3:xkys7j...`) both appear now, alongside everything
`c/pool-saturation` already reached. Nothing was copied to get there — the
union of two root sets is still just a root set.

#whatsnext[
  A view names *what is reachable*. It says nothing about which of those
  six, or five, or seven units matters most if you can only keep three.
  That ranking is `salience` — Chapter 17 — and it is exactly why `view`
  exists as a separate, cheap step: rank a hundred units and you are
  wasting cycles on the ninety a reader will never see; name the view
  first, then rank only what is actually in play.
]

#section("bundle — the reachable closure, made portable")

A view is free precisely because it does not copy anything — it is a
statement about *this* store, in memory, right now. The moment you want to
hand that view to somebody else — a teammate, a downstream pipeline, a
render step running on a different machine — "free" stops being enough:
they need the units themselves, not a promise about how to compute them
from a store they may not have.

`bundle` is the answer: it walks the same reachable closure `view` reports
and serialises exactly that closure as a self-contained, binary CBOR
sequence — a store in miniature, containing nothing the view did not reach.

#screen(caption: "$ smysl bundle fixtures/corpus/F1-incident.smy | wc -c")[
```
1424
```
]

For comparison, the whole source file — parser input, comments-are-errors
surface syntax, every one of the eight units whether `v/f1` reaches them or
not — is 2192 bytes:

#screen(caption: "$ wc -c fixtures/corpus/F1-incident.smy")[
```
    2192 fixtures/corpus/F1-incident.smy
```
]

#callout(label: "Why")[
  Handing someone the whole file works until the file is not the whole
  story — until it is one document merged from three, with a contention
  report and units three other views depend on that this recipient has no
  business seeing. `bundle` gives them precisely the closure of the view
  they were handed, nothing upstream, nothing from a sibling view, and
  nothing they cannot already reach by the same `grounds`/`deps`/relation
  edges `view` prints. It is the reachable set turned into bytes you can
  put in an email, and it stays load-bearing: whoever receives it can run
  `check` or `render` on it exactly as if it were the original store.
]

`--view v/f1` picks which of a store's declared views to bundle, if there
is more than one; the first declared view is the default. `--include-
retracted` controls one specific edge case in that closure — a unit that
has been retracted (Chapter 18) but that something *else* in the bundle
still points at by `grounds` or `deps`. `bundle`'s default already keeps
that unit, because a bundle with a dangling reference is strictly worse
than one carrying a unit nobody believes anymore; `--include-retracted`
only matters for a retracted unit nothing else in the closure references at
all, which the default quietly drops and `--include-retracted` keeps. On a
two-root view built from an unrelated claim and a retracted, unreferenced
"stray" claim, the difference is visible in the byte count itself:

#screen(caption: "$ smysl bundle v/demo.cbor | wc -c   /   smysl bundle --include-retracted v/demo.cbor | wc -c")[
```
162
279
```
]

162 bytes is the surviving claim alone; 279 bytes is both, because
`--include-retracted` kept the withdrawn one in, gist and all, rather than
silently dropping it because a consumer might still want to know it was
ever there.

#whatsnext[
  You now have two ways to name a reachable set — cheaply as a view,
  durably as a bundle — but neither one tells you which units in that set
  are load-bearing and which are along for the ride. Chapter 17's
  `salience` answers exactly that, over the same reachability this chapter
  built.
]

#exercises((
  [Run `smysl view --id v/narrow --roots b3:wo4t2c46lq45fnakd6tajlgcac
   --format surface fixtures/corpus/F1-incident.smy`. One root reaches three
   units. Trace by hand, in the source file, why those three and not the other
   five.],
  [A view stores *roots*, not a list of members. Suppose you add a new claim
   that grounds on a unit already inside `v/narrow`. Without re-running
   anything, is it in the view? Now suppose you had stored a member list
   instead. What breaks?],
  [`bundle` takes a view and emits its reachable closure. Given rule L, predict
   what `bundle` must do about a unit that is reachable but whose grounds are
   not — and explain why the alternative would make a bundle dangerous to
   send.],
))

#answers((
  [Reachability follows `grounds` and `deps` downward from the root. The root
   is `c/regression`; it grounds on `e/trace` and depends on the definition
   `d/p95` — three units, and the walk stops there because measured evidence
   and a cited definition rest on nothing further. The other five sit *above*
   the root — `f/root-cause` and what it reaches through its other branch —
   and a downward walk never sees them. A view is a question asked of the
   graph, and the answer is whatever the edges say.],
  [Yes, it is in the view the moment you add it, with no re-run — because
   membership is computed from the roots every time rather than stored. With a
   member list you would have two copies of the truth, and they would diverge
   the first time anyone edited the graph without remembering to update the
   list. Worse, two views naming the same unit would each hold their own copy,
   and a change to one would silently disagree with the other.],
  [It must pull the grounds in too — that is rule L, closure: whatever a unit
   needs travels with it. A bundle that arrived containing a `derived` claim
   whose evidence had been left behind would be a document that cannot be
   checked by whoever receives it: the claim would read as supported, its
   support would be a dangling reference, and the recipient has no way to
   fetch what they were not sent. Closure is what makes a bundle safe to hand
   to someone who has nothing else.],
))

#recap((
  [A view is a name plus a root set, never a container: every unit reachable
    from the roots belongs to it at zero copying cost, which is what makes
    combining views a plain union rather than a copy-and-reconcile problem.],
  [Reachability follows edges the way they are authored, forward: `grounds`,
    `deps`, and the two support-bearing relation kinds (`causes`, `answers`)
    all count; a `rebuts` edge runs from the objection to the thing objected
    to, so rooting downstream of a contested claim does not pull its
    objection in for free.],
  [`view --roots` builds an ad-hoc view over real uids only — never a
    label, which exists only inside the one file's parse that defined it.
    `--id`, `--threads`, and `--requires` name it, curate a reading order
    over it, and declare what a full-fidelity reader of it needs, in that
    order.],
  [`bundle` serialises a view's reachable closure as a portable, self-
    contained CBOR store — strictly smaller than shipping the whole
    document, and load-bearing on its own.],
  [`bundle`'s default keeps a retracted unit only if something else in the
    closure still references it; `--include-retracted` keeps it
    unconditionally, and the difference shows up directly in byte count.],
))

#chapter(number: 17, title: "salience — What Matters, and Why")

`view` tells you what is reachable. It does not tell you that a seven-day
trace and a one-line definition matter more to an incident brief than the
canary run that turned out to be a red herring — and once a store has
dozens of units, "what matters" stops being obvious from staring at the
graph. `salience` answers that question with a number, and — this is the
part that matters for trusting the number — every digit of it is
arithmetic you can re-run by hand.

#section("Three named terms, not one opaque score")

#term("Salience")[
  A score in `[0, 1]` for every unit in a store, built from exactly three
  named, independently-weighted terms: *centrality* (how much of the graph
  depends on this unit, via personalised PageRank over `grounds`, `deps`,
  `causes`, and `answers`), *corroboration* (how many independent agents
  attested it, saturating at four independent groups), and *role* (a
  caller-supplied weight for where a unit sits in the thread currently
  being packed). `raw = w_c·centrality + w_r·corroboration + w_t·role`,
  quantised to the nearest 1/1024 once, at the end. An author who sets a
  unit's `salience` explicitly overrides all three terms outright — a
  human who says what matters is not second-guessed by an algorithm.
]

#callout(label: "Why")[
  An importance score you cannot decompose is a score you cannot argue
  with. If `salience` returned one float and stopped there, disagreeing
  with a ranking would mean disagreeing with a black box — you would have
  no way to say *which* part of the reasoning was wrong, only that the
  number felt off. Three named terms mean a disagreement has an address:
  "this unit's centrality is right but corroboration is overcounting two
  attestations that share ancestry" is a claim you can check against the
  graph, the same way `check`'s diagnostics are addressed at a specific
  unit rather than a verdict about the file as a whole. Nothing in this
  system asks you to trust a number you cannot recompute.
]

#subsection("Ranking a real store")

Run without arguments, `salience` scores every unit in the file and prints
them in descending order — the full ranking, not a sample:

#screen(caption: "$ smysl salience fixtures/corpus/F1-incident.smy")[
```
0.5000  b3:js4xzessu5zwjpv2rawtugnuvj
0.3027  b3:wo4t2c46lq45fnakd6tajlgcac
0.2129  b3:cvhirtgs2mpvli2ethhyeo32uf
0.1289  b3:ekitkvj75uvgzxpvq3ad2nrv3b
0.1289  b3:re42iey2e7syg6zp73tfrlqbvh
0.0898  b3:izyuzlt42mqcvgdfb4nfpllxyq
0.0000  b3:phsoomklkmlq3sjvbe6cyuqy5v
0.0000  b3:xkys7j42mcuyiaxiyh73xddimr
```
]

The top scorer, `b3:js4xzessu...`, is `f/root-cause` — the finding, and
also the sole root of the file's declared view, `v/f1`. That is not a
coincidence: with no `--seed` given, `salience` personalises against a
store's view roots by default (§16.4's "no thread and no roots" case falls
back to plain, unweighted PageRank only when a store declares *no* view at
all). The finding gets first claim on rank because dangling mass returns to
the seed on every one of the walk's 32 iterations, and the unit you are
ranking *for* is the one thing you certainly care about. The two zero
scores are exactly the two units Chapter 16 found `v/f1` cannot reach at
all — `c/canary-clean` and `e/canary`. A unit that is not reachable from
the active seed has no path for rank to travel along, so it scores
nothing; unreachable is not "low salience," it is *no* salience, and that
distinction matters when you are deciding whether a unit was excluded on
purpose or simply never considered. `--top 3` trims the same ranking to
the highest three without changing a single number:

#screen(caption: "$ smysl salience --top 3 fixtures/corpus/F1-incident.smy")[
```
0.5000  b3:js4xzessu5zwjpv2rawtugnuvj
0.3027  b3:wo4t2c46lq45fnakd6tajlgcac
0.2129  b3:cvhirtgs2mpvli2ethhyeo32uf
```
]

#subsection("--explain — the arithmetic behind one number")

`--explain` is the term-by-term breakdown that makes the callout above more
than a promise. Run against `c/pool-saturation` (`b3:cvhirtgs2mpvli2eth...`,
scored `0.2129` above):

#screen(caption: "$ smysl salience --explain b3:cvhirtgs2mpvli2ethhyeo32uf fixtures/corpus/F1-incident.smy")[
```
b3:cvhirtgs2mpvli2ethhyeo32uf: 0.2129
  centrality      0.4248 x 0.50
  corroboration   0.0000 x 0.20  (0 independent group(s))
  role            0.0000 x 0.30
  raw             0.2129
```
]

Multiply it out by hand: `0.4248 × 0.5 + 0.0000 × 0.2 + 0.0000 × 0.3 =
0.2124`, which lands on `0.2129` once you account for the fact that
`0.4248` is itself already quantised for display — the arithmetic checks
out to the quantum, because it is the same arithmetic `salience` ran, not a
paraphrase of it. Zero independent attesting groups is the honest reading
here: nothing in this fixture stamps an attestation on `c/pool-saturation`
at all, so corroboration contributes nothing to its score, and centrality
alone is carrying the whole 0.2129.

#subsection("--weights c,r,t — the same store, a different lens")

The default weights — centrality `0.5`, corroboration `0.2`, role `0.3` —
are a judgement call, not a law of the graph, and `--weights` lets you make
a different one on the same store without touching a single unit. Set
weights to `1,0,0` and every score becomes pure centrality, rescaled so the
top unit reaches `1.0`:

#screen(caption: "$ smysl salience --weights 1,0,0 --top 5 fixtures/corpus/F1-incident.smy")[
```
1.0000  b3:js4xzessu5zwjpv2rawtugnuvj
0.6055  b3:wo4t2c46lq45fnakd6tajlgcac
0.4248  b3:cvhirtgs2mpvli2ethhyeo32uf
0.2578  b3:ekitkvj75uvgzxpvq3ad2nrv3b
0.2578  b3:re42iey2e7syg6zp73tfrlqbvh
```
]

The *order* of these five is identical to the default-weight ranking — in
this particular fixture, centrality alone already predicts the whole
ordering, because nothing here is independently corroborated and no role
weights are set. That is a real, checkable fact about this store, not a
general property of the three terms: give a unit a strong role weight or a
second attesting agent and the order can change (the next two subsections
do exactly that). The library ships a second preset for a specific reason
rather than as a demonstration: `SalienceWeights::untrusted()` zeroes
corroboration outright (`w_r = 0.0`), because corroboration is only honest
where agent identity can be trusted, and until attestations are
cryptographically signed, a deployment that cannot vouch for who an agent
actually was should not let a forged second opinion buy a unit extra rank.

#subsection("--seed — personalising against a different question")

Every ranking above answers "what matters to `f/root-cause`," because that
is `v/f1`'s only root and the default seed. `--seed` overrides that,
personalising the same walk against different units — and because rank
starts at the seed and returns dangling mass to it, seeding on a unit the
default view never reaches surfaces a completely different ranking from
the same store. Seed on `c/canary-clean` (`b3:phsoomklkmlq...`, one of the
two units `v/f1` scored `0.0000` above) instead of the view's default root:

#screen(caption: "$ smysl salience --seed b3:phsoomklkmlq3sjvbe6cyuqy5v --top 8 fixtures/corpus/F1-incident.smy")[
```
0.5000  b3:phsoomklkmlq3sjvbe6cyuqy5v
0.4209  b3:xkys7j42mcuyiaxiyh73xddimr
0.0000  b3:cvhirtgs2mpvli2ethhyeo32uf
0.0000  b3:ekitkvj75uvgzxpvq3ad2nrv3b
0.0000  b3:izyuzlt42mqcvgdfb4nfpllxyq
0.0000  b3:js4xzessu5zwjpv2rawtugnuvj
0.0000  b3:re42iey2e7syg6zp73tfrlqbvh
0.0000  b3:wo4t2c46lq45fnakd6tajlgcac
```
]

Every unit that led the default ranking — the finding, `c/regression`,
`c/pool-saturation` itself — drops to exactly `0.0000`, and the canary claim
and its evidence take the entire top of the ranking instead. This is the
seed doing its job: `c/canary-clean` grounds only on `e/canary` and reaches
nothing else, so once you ask "what matters to the canary claim," the rest
of the incident brief is, correctly, irrelevant. Contrast this with seeding
on a *leaf* unit — say `e/trace`, which has no outgoing `grounds` of its
own — and the seed's entire mass simply returns to itself every iteration
with nowhere else to go, scoring `0.5000` and leaving every other unit at
`0.0000`. A meaningful seed is a unit with somewhere further to reach; a
pure leaf, seeded alone, only ever ranks itself.

#subsection("Corroboration groups: what counts as a second opinion")

None of the units in `F1-incident.smy` carry more than one attestation, so
the fixture cannot show corroboration doing anything by itself. The
mechanism is worth seeing for real rather than taking on faith, so this is
one built from the same library `smysl` itself is written against: one
evidence unit, one claim grounded on it, first with a single attesting
agent, then with two attesting agents that share no ancestry:

#screen(caption: "$ smysl salience --seed <claim uid> --explain <evidence uid> corrob-one.cbor")[
```
b3:vxqhovkdp36ndqe454ot6cfkcx: 0.4707
  centrality      0.8418 x 0.50
  corroboration   0.2500 x 0.20  (1 independent group(s))
      counted: model:openai/gpt-4
  role            0.0000 x 0.30
  raw             0.4707
```
]

#screen(caption: "$ smysl salience --seed <claim uid> --explain <evidence uid> corrob-two.cbor")[
```
b3:vxqhovkdp36ndqe454ot6cfkcx: 0.5205
  centrality      0.8418 x 0.50
  corroboration   0.5000 x 0.20  (2 independent group(s))
      counted: model:anthropic/claude
      counted: model:openai/gpt-4
  role            0.0000 x 0.30
  raw             0.5205
```
]

Adding one more independent attesting agent moves corroboration from
`0.2500` to `0.5000` — one group out of the four-group cap to two — and the
final score rises from `0.4707` to `0.5205` with centrality untouched,
because centrality only depends on graph structure, which did not change.
Two details in `corroboration()`'s grouping matter more than the count
itself: attestations group by `(agent, recipe)`, so the *same* model
answering under the *same* recipe twice is one group, not two — a model
cannot corroborate itself by repeating itself — and two groups that trace
back to the same parent ancestry collapse into one, because two agents
agreeing because they both read the same upstream source is not
independent evidence, it is one piece of evidence read twice.

#whatsnext[
  Salience is what lets `pack` (Chapter 19) fit a large store into a small
  budget without asking a model which units to keep. `pack` does not derive
  its own notion of importance — it asks exactly this ranking, unit by
  unit, and spends the budget top-down. Everything demonstrated in this
  chapter — the three terms, the weights, the seed — is the same knob you
  will reach for again once the question changes from "what matters" to
  "what fits."
]

#exercises((
  [Run `smysl salience --explain b3:js4xzessu5zwjpv2rawtugnuvj
   fixtures/corpus/F1-incident.smy`. The unit scores 0.5000, made of
   centrality 1.0000 at weight 0.50, corroboration 0.0000, and role 0.0000.
   It is the highest-ranked unit in the document *and* scores zero on two of
   three terms. Explain how both are true.],
  [That unit's corroboration is `0 independent group(s)`. Look at
   `F1-incident.smy` and say what would have to be added — not changed — to
   raise it, and why "add another claim agreeing with it" is not the answer.],
  [Run plain `smysl salience` on the same file. Two units score exactly
   `0.0000`. Find them in the source. Are they unimportant?],
))

#answers((
  [The three terms measure different things and most units score zero on most
   of them. Centrality is structural — how much of the argument flows through
   this unit — and the finding is where everything converges, so it maxes out.
   Corroboration asks whether *independent* lines of support arrive at it, and
   role asks whether a thread has assigned it a job. Scoring 0.5 out of a
   possible 1.0 while leading the document tells you the document is a single
   chain of reasoning rather than several converging ones — which is a real
   and useful thing to learn about an incident brief.],
  [A second, independent evidential path to the same finding — a different
   measurement, not resting on the ones already there. Another *claim* agreeing
   with it adds no corroboration if it grounds on the same evidence, because
   the term counts independent groups rather than voices; two claims sharing a
   ground are one line of support wearing two hats. This is precisely the
   property that stops a document from looking better-supported by restating
   itself.],
  [They are the two units nothing else points at and no thread names — in this
   document, the ones off the main argumentative spine. Zero salience means
   *nothing in this store's structure asks for it*, which is a statement about
   the graph and not about the world. A retracted-but-important caveat, or a
   piece of evidence somebody forgot to ground anything on, scores zero too.
   Salience decides what survives a budget; it does not decide what matters.],
))

#recap((
  [Salience is `[0,1]`, built from three named, independently-inspectable
    terms — centrality, corroboration, role — never a single opaque
    number; `--explain` reproduces the exact arithmetic behind any one
    unit's score.],
  [With no `--seed`, `salience` personalises against a store's declared
    view roots by default, falling back to plain PageRank only when the
    store declares no view at all. A unit the seed cannot reach scores
    exactly `0.0000` — unreachable, not merely low.],
  [`--weights c,r,t` re-lenses the same store without touching a unit;
    `SalienceWeights::untrusted()` zeroes corroboration for deployments
    that cannot yet vouch for agent identity.],
  [`--seed` changes which question is being asked. Seeding on a unit the
    default view never reaches can invert the entire ranking; seeding on a
    pure leaf ranks only that leaf, because its own mass has nowhere else
    to flow.],
  [Corroboration counts independent `(agent, recipe)` groups, capped at
    four; the same agent repeating itself is one group, and two groups
    sharing ancestry collapse into one, because agreement that traces to a
    shared source is not two independent checks.],
))

#section("The other ranking: `find`")

`salience` answers *what matters here* by looking at the graph — what grounds
what, what rebuts what, how much of the argument leans on a unit. It never
reads a word of the content. That is a strength when you want the shape of an
argument, and useless when you arrive knowing only what you are looking for.

`find` is the complement. It ranks by *words*, over the gist of every unit and
— at lower weight — the body and detail beneath it.

#screen(caption: "$ smysl find \"connection pool saturated\" fixtures/corpus/F1-incident.smy")[
```
7.5637  b3:cvhirtgs2mpvli2ethhyeo32uf  The eu-west connection pool is saturated.
0.8732  b3:xkys7j42mcuyiaxiyh73xddimr  The 4.2 canary ran the same pool configuration without the regression.
0.8635  b3:js4xzessu5zwjpv2rawtugnuvj  Pool saturation is the leading cause but is not consistent with the canary.
0.8541  b3:izyuzlt42mqcvgdfb4nfpllxyq  Pool acquisition wait rose from 2 ms to 310 ms over the same window.
0.4362  b3:wo4t2c46lq45fnakd6tajlgcac  p95 auth latency tripled after the 4.2 rollout.
```
]

#callout(label: "Why")[
  A store that has been through a few hops holds units nobody on this machine
  wrote, named by uids nobody can read. `trace` needs a starting uid. `view`
  needs roots. `pack` needs a budget and gives you what fits. None of them
  answers the question you actually have when you open a file someone handed
  you, which is *where is the bit about the connection pool*.

  It searches the *gist* principally, and that is what makes it work whatever
  the units contain. A unit's payload might be a stack trace, a metric series
  or a diff — but every unit carries a gist, because the format requires one,
  and the gist is a sentence about whatever the payload is. You are never
  searching the telemetry; you are searching the sentence that describes it.
]

Two properties are worth knowing because they are unusual for a search tool.

It is *pure*. No model, no index written to disk, no network. The same store
and the same query give the same ranking on any machine, with ties broken by
uid so there is one right answer rather than an arbitrary one. That is the same
guarantee `pack` and `merge` carry, and it is why `find` can sit in a
reproducible pipeline rather than beside one.

And it does *not stem*. A search tool for prose would reduce `latencies` to
`latenc` so it matches `latency`. That helps English and destroys
`connection_pool_size`, which is exactly the term someone types. Identifiers
are split into parts *and* kept whole, so `pool.wait_ms` finds its own unit and
so does `pool`:

#screen(caption: "$ smysl find \"pool.wait_ms\" --kind evidence fixtures/corpus/F1-incident.smy")[
```
6.4462  b3:izyuzlt42mqcvgdfb4nfpllxyq  Pool acquisition wait rose from 2 ms to 310 ms over the same window.
0.8732  b3:xkys7j42mcuyiaxiyh73xddimr  The 4.2 canary ran the same pool configuration without the regression.
```
]

#subsection("Where it is weak, measured rather than guessed")

The project evaluates this rather than asserting it. Twenty queries over the
corpus, in three classes — queries that share the gist's vocabulary, queries
that paraphrase it, and bare identifiers:

#dtable(
  (auto, auto, auto, auto),
  (
    ([class], [recall\@5], [MRR], [first place]),
    ([shared vocabulary], [1.00], [0.94], [0.88]),
    ([paraphrase], [0.75], [0.41], [0.12]),
    ([identifier], [1.00], [1.00], [1.00]),
  ),
)

Identifiers are perfect and shared vocabulary is nearly so. Paraphrase is
where it falls down: the right unit reaches the top five three times in four,
but ranks *first* once in eight. Recall without precision is a list to read,
not an answer.

Broken down by what kind of unit was being looked for, the reason is plain.
`evidence` and `data` units score 1.00 — they name concrete things, and
concrete things are findable by name. `claim` units score 0.67, because a claim
is an interpretation phrased in whatever words its author reached for, and you
will reach for different ones.

#callout(label: "What that means for you")[
  Use `--kind` when you know what you are after. Narrowing to `evidence` when
  you want a measurement, or to `question` when you want what was asked, beats
  reading past four things you did not want — and it is a bigger improvement
  than any tuning of the ranking would be.

  And expect to search for *nouns from the domain* rather than for the
  sentence you would write. `connection pool` works; "why was it slow" does
  not. That is a real limitation of lexical search, not a bug, and closing it
  is what a semantic backend would be for. `Retriever` is a trait precisely so
  one can be added without disturbing any of this.
]

#chapter(number: 18, title: "retract — Blast Radius First")

Every other command in this part of the book adds information to a store,
or names a slice of what is already there. `retract` is the one command
whose entire job is *taking belief away* — and unlike deleting a line from
a file, withdrawing belief in a unit can leave other units standing on
nothing. Get this wrong and a finding three hops downstream silently keeps
citing evidence nobody believes anymore. `retract`'s whole design is built
around never letting that happen by accident.

#section("Blast radius, computed before anything happens")

#term("Blast radius")[
  Every unit a retraction would reach, computed and printed *before* the
  retraction is applied — the target itself, plus every unit left with no
  surviving `grounds` once the target's status becomes `unfounded`. Under
  the default (`strict`) retraction policy this is transitive: a unit
  orphaned by the retraction can itself orphan whatever rests only on it,
  and the blast radius follows the whole chain, not just the first hop.
]

#callout(label: "Why")[
  Retraction is the one operation in this system that can make another
  unit's grounds disappear out from under it without touching that unit at
  all. A claim you have not looked at in months can go from `derived` to
  effectively unfounded because someone three hops upstream retracted the
  measurement it was built on — and if the tool only told you that after
  doing it, you would find out by `check` failing later, or worse, by not
  finding out at all. `retract --dry-run` exists so the blast radius is
  something you read *before* you decide, not something you discover by
  having already decided.
]

#subsection("Dry run, then the real thing")

`fixtures/corpus/F1-incident.smy`'s `c/regression` (`b3:wo4t2c46lq...`)
grounds on exactly one unit: `e/trace` (`b3:re42iey2e7syg6zp...`), the
seven-day trace. Retracting the trace, dry-run first:

#screen(caption: "$ smysl retract --dry-run b3:re42iey2e7syg6zp73tfrlqbvh fixtures/corpus/F1-incident.smy")[
```
fixtures/corpus/F1-incident.smy: retracting b3:re42iey2e7syg6zp73tfrlqbvh would reach 2 unit(s), orphaning 1
fixtures/corpus/F1-incident.smy:   b3:wo4t2c46lq45fnakd6tajlgcac would lose all of its grounds
```
]

Two units reached — the trace itself, plus `c/regression`, which loses its
only ground and is named explicitly rather than left for you to work out.
Note what is *not* in the blast radius: `f/root-cause` grounds on both
`c/pool-saturation` and `c/regression`, and only one of those two goes
unfounded, so the finding keeps standing on the one that survives. A unit
with any surviving ground is never orphaned — orphaning requires *every*
ground to be gone, which is exactly why the blast radius has to be computed
rather than assumed from "this reaches the trace, and the trace reaches the
regression, and the regression reaches the finding."

Dropping `--dry-run`, given an authority that accepts the request (the next
section covers exactly when that is true), reports the same reach and then
recomputes effective status with the retraction in place:

#screen(caption: "$ smysl retract --as model:openai/gpt-4 --authority any b3:re42iey2e7syg6zp73tfrlqbvh corrob-two.cbor")[
```
corrob-two.cbor: retracting b3:vxqhovkdp36ndqe454ot6cfkcx would reach 2 unit(s), orphaning 1
corrob-two.cbor:   b3:2tbhvo5rklopzpxb44o4nua2zb would lose all of its grounds
corrob-two.cbor: 2 unit(s) now read as unfounded
```
]

The dry run and the real run report an identical blast radius — that
agreement is not incidental, it is the specific property `retract`'s own
test suite gates on: a dry run that disagreed with what applying it
actually does would be worse than no dry run at all. The final line is the
number that matters operationally: two units now read as `unfounded`,
where a moment ago they read as `measured` and `inferred`. Declared status
— what an agent originally wrote — never changes; *effective* status is
what a reader sees once every retraction in the store has been accounted
for, and it is effective status, not declared status, that `check` and
`render` act on downstream.

#section("Authority: who is allowed to retract what")

Computing the blast radius is only half of `retract`'s job. The other half
is deciding whether the agents asking are *allowed* to cause it — because
"anyone can withdraw anything" is a censorship vector the moment more than
one agent writes to a shared store.

#dtable(
  (auto, 1fr),
  (
    ([`--authority`], [Who may retract the target]),
    ([`origin` (default)], [Only an agent that has already attested this
      specific unit. A stranger cannot silence work they had no hand in.]),
    ([`any`], [Any single named agent — the permissive setting, for a
      single-writer store where the question does not arise.]),
    ([`quorum:N`], [At least `N` *distinct* agents must jointly issue the
      retraction; the same agent repeated does not count twice.]),
  ),
)

`origin` is the default for a reason: it is the direct defence against one
adversarial agent erasing another's work. `fixtures/corpus/F1-incident.smy`
demonstrates the refusal for free, because none of its units carry an
explicit attestation at all — asking to retract one under the default
authority fails before the blast radius even gets a chance to matter:

#screen(caption: "$ smysl retract --as human:vladimir b3:cvhirtgs2mpvli2ethhyeo32uf fixtures/corpus/F1-incident.smy")[
```
fixtures/corpus/F1-incident.smy: retracting b3:cvhirtgs2mpvli2ethhyeo32uf would reach 1 unit(s), orphaning 0
smysl retract: origin authority: none of the 1 requesting agent(s) attested this unit
```
]

The blast radius still printed — refusal does not withhold the information
a caller would need to argue for the retraction, it only withholds
permission to apply it. The same command, on a store where the retracting
agent genuinely is an attestor, succeeds:

#screen(caption: "$ smysl retract --as model:openai/gpt-4 --authority origin b3:vxqhovkdp36ndqe454ot6cfkcx corrob-two.cbor")[
```
corrob-two.cbor: retracting b3:vxqhovkdp36ndqe454ot6cfkcx would reach 2 unit(s), orphaning 1
corrob-two.cbor:   b3:2tbhvo5rklopzpxb44o4nua2zb would lose all of its grounds
corrob-two.cbor: 2 unit(s) now read as unfounded
```
]

— `model:openai/gpt-4` is one of the two agents Chapter 17 attested that
same evidence unit with, so `origin` authority accepts it. A stranger, on
the same store, is refused exactly like the F1 example above. `quorum:2` on
the same target shows the same pattern with a distinct-agent count instead
of an attestation check — two different named agents succeed, and the same
agent named twice does not:

#screen(caption: "$ smysl retract --as human:a --as human:a --authority quorum:2 b3:vxqhovkdp36ndqe454ot6cfkcx corrob-two.cbor")[
```
corrob-two.cbor: retracting b3:vxqhovkdp36ndqe454ot6cfkcx would reach 2 unit(s), orphaning 1
corrob-two.cbor:   b3:2tbhvo5rklopzpxb44o4nua2zb would lose all of its grounds
smysl retract: quorum:2 requires 2 distinct agents, got 1
```
]

Naming the same agent twice on the command line is one agent, not two — the
count is over distinct identities, not over how many `--as` flags you
typed.

#section("Retraction policy, chosen at merge time")

`--authority` decides *who* may retract; it has nothing to do with *how
far* a retraction reaches once it is allowed. That second question —
`strict`, `advisory`, or `ignore` — is the retraction policy Chapter 13
covers as one of `merge`'s three knobs, and it governs the exact
transitivity this chapter has been demonstrating under its default,
`strict`. Under `advisory`, the same retraction marks the target
`unfounded` and reports it (`SMY-W052`) but never touches a dependent's
status; under `ignore`, the retraction is recorded and changes nothing at
all. `plan_retraction` takes whichever policy the store was merged under as
a parameter for exactly this reason — a blast radius computed under the
wrong policy would tell you the wrong thing about what is actually at
stake. If a retraction here looks smaller than you expected, check which
policy the store you are working in was merged with before assuming the
blast radius is wrong.

#whatsnext[
  A retraction changes what a store *means* without changing a single
  byte of any other unit's declared content — which is exactly the kind of
  change that is easy to lose track of. `diff --hop` (Chapter 14) is the
  most direct way to confirm the aftermath matches what the blast radius
  promised, unit by unit; `check` (Chapters 8 and 21) is the broader
  sweep, and will surface any unit left orphaned as `SMY-E050` if the
  active retraction policy is transitive. Run one of the two after any
  retraction you did not just dry-run — the whole point of computing the
  blast radius in advance was to know what to expect from it.
]

#exercises((
  [Run `smysl retract --dry-run b3:re42iey2e7syg6zp73tfrlqbvh
   fixtures/corpus/F1-incident.smy`. It reports reaching 2 units and orphaning
   1, and *names* the unit that would lose all of its grounds. Now run the same
   dry run against `b3:ekitkvj75uvgzxpvq3ad2nrv3b`. Why does one orphan
   something and the other not?],
  [The command is `--dry-run` by default in every example this chapter shows.
   Make the argument for why a tool would make you ask twice to retract, when
   it does not make you ask twice to merge.],
  [A unit that loses all of its grounds becomes `unfounded`. Look back at
   Chapter 6's exercise, where authoring `unfounded` directly was an error.
   Reconcile the two: why may the tool write a status you may not?],
))

#answers((
  [Because blast radius follows `grounds`, and the two units sit at different
   depths of the argument. One is a leaf that nothing else rests on; the other
   is the sole support of a claim above it, so retracting it leaves that claim
   holding nothing. The output names the orphan rather than counting it,
   because "1 unit would be orphaned" is not actionable and
   "`b3:wo4t2…` would lose all of its grounds" is.],
  [Because merge is additive and retraction is not. A merge that surprises you
   leaves everything it found still in the store, and you can look again. A
   retraction propagates: it can change the standing of units nobody has looked
   at in months, several hops from the one you named. The asymmetry is between
   operations you can inspect *after* and operations you had better inspect
   *before* — and the tool refuses to let the second kind be discovered by
   running it.],
  [Because the tool is recording a history that actually happened and you would
   be asserting one that did not. `unfounded` means *something this rested on
   was withdrawn* — when `retract` writes it, that is a true statement about an
   event in the store. Typed by hand on a unit that never had support, it
   claims a collapse that never occurred; `speculative` is the honest word for
   a claim that never had grounds. The status is not reserved because it is
   dangerous, but because it is a *consequence*, and only the tool is in a
   position to observe it.],
))

#recap((
  [Blast radius is every unit a retraction would reach, computed and
    printed before anything is applied — the target, plus every unit that
    would lose *every* surviving ground, transitively, under the active
    retraction policy.],
  [`--dry-run` and the real retraction report an identical blast radius by
    construction; the real run additionally recomputes and prints the
    effective-status count once the retraction is in place.],
  [Declared status never changes; effective status is what a retraction
    changes, and it is effective status that downstream commands like
    `check` and `render` act on.],
  [`--authority origin` (default) requires the retracting agent to have
    already attested the target — the direct defence against a stranger
    silencing work they had no hand in. `any` accepts a single named agent
    unconditionally; `quorum:N` requires `N` distinct agents, where the
    same agent named twice still counts as one.],
  [Authority governs *who* may retract; retraction policy — chosen at
    merge time, Chapter 13 — governs *how far* an authorised retraction
    reaches, from fully transitive (`strict`) to recorded-but-inert
    (`ignore`).],
))
