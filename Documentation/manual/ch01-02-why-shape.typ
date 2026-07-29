// Chapters 1-2 — the two chapters that come before any command.
//
// Chapter 1 answers "why would I put this in my pipeline at all", written for
// a reader who has never built an AI pipeline and should not need to have.
// It is the long form of Documentation/SMYSL_RATIONALE.typ: same argument,
// same measured numbers, but paced for someone meeting the problem for the
// first time rather than someone who already recognises it.
//
// Chapter 2 answers "what is this thing made of" — the layering, the kernel,
// and the twelve rules. The rules are referenced by name in eight later
// chapter files and were, until this chapter existed, never introduced
// anywhere; a reader met "rule M" in the middle of Chapter 6 and had to infer
// it from context.

#import "design.typ": *

// The four-hop lossy-handoff figure, shared in spirit with SMYSL_RATIONALE.typ
// but redrawn here rather than imported, since the two documents build with
// separate design modules.
#let lossy_hops() = {
  v(3mm)
  align(center, diagram(
    spacing: (13mm, 9mm),
    dnode((0, 0), [*Model A*\ drafts]),
    dnode((1, 0), [*Model B*\ merges]),
    dnode((2, 0), [*Human*\ reviews]),
    dnode((3, 0), [*Model C*\ reports]),
    edge((0, 0), (1, 0), "->", label: text(size: 7pt, fill: ink_gray, [hedge dropped]), label-side: left),
    edge((1, 0), (2, 0), "->", label: text(size: 7pt, fill: ink_gray, [source dropped]), label-side: left),
    edge((2, 0), (3, 0), "->", label: text(size: 7pt, fill: ink_gray, [conflict dropped]), label-side: left),
    dnode((1.5, 1.3), [*the same `.smy` document*\ read, not re-derived, at every stage], fill: ink_recap_bg),
    edge((0, 0), (1.5, 1.3), "->", stroke: (paint: ink_recap, thickness: 0.7pt, dash: "dashed")),
    edge((1, 0), (1.5, 1.3), "->", stroke: (paint: ink_recap, thickness: 0.7pt, dash: "dashed")),
    edge((2, 0), (1.5, 1.3), "->", stroke: (paint: ink_recap, thickness: 0.7pt, dash: "dashed")),
    edge((3, 0), (1.5, 1.3), "->", stroke: (paint: ink_recap, thickness: 0.7pt, dash: "dashed")),
  ))
  figure_note[Top row: prose re-told four times, losing something specific at each hop. Bottom: one document, present unaltered at all four stages — there is no re-telling for anything to be lost in.]
  v(3mm)
}

#part(number: "I", title: "Foundations")

#chapter(number: 1, title: "Why This Belongs in Your Pipeline")

You can read this whole manual as a description of a file format and a
command-line tool, and it will be accurate. It will also leave out the reason
any of it exists. This chapter is the reason. It assumes you have never built
an AI pipeline, uses no vocabulary it does not define, and ends with the one
thing worth knowing before you spend an afternoon on the rest: what problem
this solves, how big that problem measurably is, and when you can safely
ignore it.

#section("What a pipeline is, if you have never built one")

#callout(label: "Why")[
  "Pipeline" is one of those words that everybody in the field uses and
  nobody defines, because the people using it built one before they named
  it. If the word is doing any work in your head other than "several
  programs in a row", the rest of this chapter will read as abstract when it
  is in fact extremely concrete.
]

#term("Pipeline")[
  A *pipeline* is a chain of steps in which the output of one step is the
  input to the next. Some steps are programs, some are language models, and
  at least one is usually a person. Nothing about the word implies
  automation, scale, or sophistication — two steps and an email is a
  pipeline.
]

Here is one, and it is the example this manual keeps returning to — it ships in
the repository as `fixtures/corpus/F1-incident.smy`, and from Chapter 13 onward
most chapters operate on it directly. A service got slow on Thursday. Four
things happen:

+ Somebody — or something — *reads the raw material*: the alert history, the
  dashboards, the deploy log, three messages in a chat channel.
+ A model *drafts an account* of what happened, drawing on that material.
+ A second model, or the same one an hour later, *merges that draft* with a
  second account written by someone looking at a different subsystem.
+ A person *reviews the result* and signs off — and then a fourth step turns
  the signed-off version into a report that goes to people who were not in
  the room.

Every arrow in that list is a *handoff*: one participant finishes and passes
something to the next. Nothing here requires artificial intelligence — this
was the shape of incident review long before models could write. What the
models changed is the number of handoffs anyone is willing to tolerate,
because each one is now cheap.

#section("The handoff is the lossy part")

#callout(label: "Why")[
  The failure this tool exists for is not in any step. Every step in the
  example above can be done impeccably and the pipeline will still degrade,
  because the loss happens in the gaps. That is an unintuitive enough claim
  that it is worth being slow about.
]

Ask what actually crosses each arrow, and the answer, almost always, is
*prose*. A paragraph. Some Markdown. A chat message. That is the only medium
every participant in the chain can both produce and consume, so that is what
gets used.

Now consider what the drafting model *knew* while it was working. It knew that
the latency figure came off a dashboard and the theory about the connection
pool was its own guess. It knew the canary data pointed the other way. It knew
which of its sentences it would defend and which it was floating. All of that
structure existed, in the model's working state, at the moment it wrote — and
then it wrote paragraphs, because paragraphs were the only thing on offer, and
the structure did not survive the trip.

The next participant now has to guess that structure back out of the prose. It
guesses, acts on the guess, and flattens *its* conclusions into prose for
whoever is next. Do this more than once and the same three things go missing
every time:

#list(
  [*Hedges vanish.* "The data suggests" becomes "the data shows" becomes "we
   found." Each retelling is slightly more confident than the one before it,
   and nobody along the way earned the extra confidence. This is the
   dangerous one, because the output gets *more* persuasive as it gets less
   warranted.],
  [*Sources evaporate.* Nothing about a citation survives being paraphrased.
   By the third hop, the number is still there and the reason to believe it
   is not, and there is no way to tell from the text which numbers still
   have a source behind them.],
  [*Disagreement disappears.* Two conflicting findings go in; whichever one
   the last participant found more convincing comes out. The conflict was
   not resolved — it was dropped — and the result reads as unanimous
   precisely because the dissent is gone.],
)

#lossy_hops()

The property that makes this hard to catch is that *prose does not announce
what it dropped.* A summary that lost every citation looks exactly like a
summary that had none to lose. There is no error, no warning, no diff. The
pipeline reports success at every step and the artifact quietly gets worse.

#section("How much is lost — measured, not asserted")

#callout(label: "Why")[
  Everyone who has run a multi-hop pipeline already believes the three losses
  above happen. Almost nobody has a number for them, and the intuition most
  people carry — "a bit, at the margins" — is wrong by a large factor. The
  size of the effect is what decides whether this tool is worth your
  afternoon.
]

The project ships an evaluation harness that measures exactly this, and it is
worth understanding the shape of the experiment before the numbers, because
the shape is what makes them mean anything.

Five documents. Five hops each — five successive summarisations, each one
reading only the previous hop's output, the way a real chain does. The whole
thing run twice: once with `gemini-3.5-flash-lite` doing the summarising and
once with `deepseek-chat`. Ninety original claims across the five documents.

Then a *different* model reads the far end and is asked, for each of the
ninety original claims: does the final text still state this? How confidently
does the text now put it? And what, if anything, does the text say it rests on?

#screen(caption: "$ make eval-live")[
```
                          tokens   claims kept   hedges lost   sources kept
  control (no summarising)  1.00        90/90          0/90          50/50
  prose baseline            0.29        72/90         42/90           1/50
  smysl                     0.49        90/90          0/90          50/50
```
]

Three rows, and the order to read them in is bottom-up.

The *prose baseline* is the pipeline almost everyone is running: summarise,
pass along, repeat. It kept 72 of the 90 claims. Of the 72 that survived, 42
came out the far end reading as flat statements of fact when the document that
went in had marked them as inferred, cited or derived — the hedge was lost, but
the sentence was not, so the claim is still there and is now stronger than it
was entitled to be. And of the 50 claims that originally named a source,
exactly one still named it. Not one in ten. One.

The *smysl* row is the same five documents, the same five hops, the same two
models — passing a `.smy` document instead of prose. Every claim kept, every
hedge intact, every source intact.

The *control* row is the row that makes the other two trustworthy, and it is
the one worth pausing on. It is the same judge, with the same prompt, reading
each document *before any summarising happened at all*. If the judge were
unreliable — if it simply failed to notice hedges, or could not find sources —
the control row would show losses too. It shows none: 90 of 90, 0 lost, 50 of
50, and it declined to rule on nothing. So whatever went missing over five hops
went missing *in the chain*, not in the instrument measuring the chain.

#callout(label: "The point")[
  None of this is a criticism of the models. Both were asked to summarise and
  both summarised well — the prose output reads fluently and says broadly the
  right things. Hedges and citations are simply not what prose is built to
  preserve, and a summariser has no way to know it dropped one, because
  nothing in its input marked them as things that could be dropped.

  On the `smysl` side, both numbers are structural rather than lucky.
  Confidence is a field that a rule checks; a source is a field that travels
  with whatever travels. Neither depends on the model choosing to be careful.
]

Note the token column, because it is the honest part. The prose chain
compresses harder — 0.29 of the original against 0.49 — and that is not
`smysl` being inefficient. Prose compresses well *because* it is dropping the
hedges and the citations. You are paying roughly 1.7× the tokens to keep them.
Whether that is a good trade is a real question with a real answer that depends
on your pipeline; this manual's position is only that you should know you are
making the trade.

#callout(label: "Honest limits")[
  Two models, five documents, one run each. That is enough to say the effect
  is not one model's quirk. It is not enough to put an error bar on it, and
  this manual will not pretend otherwise. The harness is the `smysl-eval`
  crate and the `make eval-live` target; pointing it at your own documents
  gives you the number you should actually act on, which is not this one.
]

#section("Two fixes that seem obvious and do not work")

Before reaching for a new format, most people try one of two things. Both are
reasonable, and it is worth knowing exactly where each one runs out.

#subsection("Fix one: just use a schema")

Agree on a JSON shape between each pair of steps and stop passing free text.
This genuinely solves the machine-to-machine problem — a field called
`confidence` does not evaporate the way the word "suggests" does.

It fails at the human step, and the human step is the one that mattered. Nobody
reviews a wire format. When the artifact becomes a nested JSON document with
`{"claims": [{"id": "c-0417", "conf": 0.6, ...}]}`, the person in the chain
stops reading the artifact and starts reading a rendering of it — and the
rendering is prose again, produced by a step that can drop things. The one
place a person could have caught the drift is now the one place they cannot
see.

#subsection("Fix two: just tell the model to be careful")

Put "preserve all hedges and citations" in the prompt. This helps, somewhat,
and it is the right thing to do regardless. It does not solve the problem for a
structural reason: *the instruction is unverifiable.* When the summariser
returns, nothing checks whether it complied. There is no assertion to fail, no
exit code, no diff — you have added an intention to a system that had no way of
noticing intentions being violated, and the failure mode is unchanged. It fails
silently and reports success.

#term("The position `smysl` takes")[
  Make the artifact itself *precise enough* to carry hedges, sources and
  disagreement as structure a program can check — and *plain enough* that the
  same file is what a person opens to review the work. One artifact, not a
  wire format plus a rendering of it. The bytes the machine reads and the
  bytes the human reads are the same bytes.
]

#section("The four things a unit carries that a paragraph cannot")

Here is a complete, real excerpt — not a simplified illustration. This is what
crosses the wire, and it is also what a reviewer opens:

```
@evidence e/pool-wait { status: measured, source: { kind: metric, ref: "pool.wait_ms" } }
~ Pool acquisition wait rose from 2 ms to 310 ms over the same window.

@claim c/pool-saturation { status: inferred, grounds: [e/pool-wait] }
~ The eu-west connection pool is saturated.

@rel c/canary-clean --rebuts--> c/pool-saturation { weight: 0.6 }
```

#term("Unit")[
  A *unit* is one recorded thing — a piece of evidence, a claim, a finding, a
  definition, a question. The `@evidence` and `@claim` blocks above are two
  units. The `~` line is the unit's *gist*: the one-sentence prose form, which
  is what a person reads and what a renderer emits.
]

Four things are being carried here that the paragraph version would have thrown
away, and each one maps directly onto a loss from the measurement above.

#dtable(
  (1.05fr, auto, 1.75fr),
  (
    ([What it carries], [Written as], [The loss it prevents]),
    ([*How sure the document is entitled to be*], [`status:`], [The hedge. `measured` (an instrument recorded it) is a different claim from `inferred` (a model reasoned it out), and the difference is a field rather than a tone of voice. It cannot be lost in a retelling because retelling is not how it travels.]),
    ([*What the claim rests on*], [`grounds:`], [The chain of support. `grounds: [e/pool-wait]` says this claim stands or falls with that specific piece of evidence — so retracting the evidence tells you exactly what falls with it, by name, before you do it.]),
    ([*Where it came from outside the document*], [`source:`], [The citation. A machine-checkable pointer travelling in a field, rather than a phrase in a sentence that paraphrasing dissolves.]),
    ([*What contradicts it*], [`@rel ... --rebuts-->`], [The disagreement. The rebuttal is part of the document, sitting next to the claim it argues with — not filed in a review thread nobody opens twice. Selection routines are required to keep the two together.]),
  ),
)

The mechanism that makes the first two stick is a single rule, and it is worth
stating now because it is the spine of everything: *a claim's status may be
weakened as it crosses a hop, but never strengthened.* A model reading this
document may propose at most `inferred` for what it reasons out. If it writes
`measured`, the tool does not take its word for it. Chapter 2 states this
precisely and names the two rules that enforce it.

#section("What the tool refuses to do")

#callout(label: "Why")[
  A tool's refusals tell you more about whether it fits your pipeline than its
  features do, and they are usually buried. These are stated up front because
  two of them will disqualify `smysl` for some readers, and finding that out
  now is better than finding it out in Chapter 13.
]

#list(
  [*It does not judge whether anything is true.* `status: measured` records
   that a unit was produced by measurement, not that the measurement was
   right. The tool checks that claims do not out-run their support; it has no
   opinion on whether the support is any good.],
  [*It does not pick a winner when two documents disagree.* `merge` records
   the disagreement as a first-class object and hands it to you. If you wanted
   a tool that resolves conflicts, this is not it — and the refusal is
   deliberate, because silently picking a winner is precisely the third loss.],
  [*It does not write your prose.* Every gist in a document was typed by a
   person or proposed by a model you explicitly invoked. Nothing generates
   text on its own initiative.],
  [*It does not phone home.* Of the seventeen commands, exactly two —
   `ingest` and `attest`, whose entire job is calling a model — can open a
   socket, and they will tell you what they would send before they send it.
   The restriction is enforced by a build-time check rather than by promise.],
)

#section("Three places to put it, and what each one buys")

You do not adopt this all at once, and the three insertion points are
independently useful.

#dtable(
  (auto, 1fr),
  (
    ([Where], [What it gets you]),
    ([*At the boundary* — the last step before a person reads the output], [The cheapest option and the one with the clearest payoff. Your pipeline stays exactly as it is; you add one `render` step so the reviewer sees status and sources rather than confident prose. You get the review surface without changing anything upstream.]),
    ([*Between two model steps* — the handoff itself], [This is where the measured numbers above come from. The document is read rather than re-derived at each hop, so there is no retelling for anything to be lost in.]),
    ([*At the entrance* — `ingest`, turning source prose into units], [The most work and the most value, because it is the only one that puts a trust ceiling on material entering the system in the first place. A model reading a document you supplied may propose at most `cited`, never `measured` — enforced at the door.]),
  ),
)

#section("When you do not need this")

Being honest about this is more useful than being persuasive about it. Skip
`smysl`, or use only the rendering half, when:

#list(
  [*Your pipeline has one hop.* Almost all of the measured loss is
   accumulation across hops. One model, one output, one reader: prose is
   fine, and the structure is overhead you will not get back.],
  [*Nobody reviews the output.* The format's second job is being readable by
   a person checking the work. If no person ever checks the work, you are
   paying for half a tool.],
  [*The stakes do not justify the tokens.* You are spending roughly 1.7× on
   the handoffs. For a draft nobody will act on, that is a bad trade and you
   should make it knowingly.],
)

The pipelines where it pays are the ones with several hops, an artifact
somebody is accountable for, and a reviewer who has to be able to ask "where
did this number come from" and get an answer.

#whatsnext[
  Chapter 2 is the other half of the orientation: what the system is made of,
  and the twelve rules it enforces. Those rules are named by letter throughout
  the rest of the book — you will meet "rule M" and "rule T" repeatedly — and
  Chapter 2 is the one place they are all defined. If you would rather have a
  file in your hands before any more theory, Chapter 5 builds a small real one
  end to end, and Chapter 2 will still be there.
]

#exercises((
  [Run `smysl check fixtures/corpus/F1-incident.smy`. It reports *13 records,
   8 units*. Open the file and account for the five records that are not
   units — what are they, and why would a format that only had units be
   unable to carry Chapter 1's third loss?],
  [In that file, `c/pool-saturation` is `inferred` and rests on `e/pool-wait`,
   which is `measured`. Suppose a downstream model wanted to publish
   `c/pool-saturation` as `measured` too. Its ground is already `measured`,
   so the "status may not exceed its weakest ground" rule permits it. What
   stops it anyway? (This is the distinction between the two rules Chapter 2
   names; try to state it before reading on.)],
  [Chapter 1 claims prose gives no signal when it drops something. Find the
   single line in `F1-incident.smy` that carries the disagreement, then read
   the gist of `f/root-cause`. What would a summariser have to do — not
   choose to do, but *be able* to do — for that disagreement to survive a
   paraphrase?],
))

#answers((
  [Three `@rel` edges, one `@thread`, and the `@doc` header, which is
   8 + 3 + 1 + 1 records in total. The relations are the point:
   `--rebuts-->` is an edge, not a unit, so a
   format carrying only units would have nowhere to put a disagreement except
   inside the prose of one of the two units that disagree — which is exactly
   where a paraphrase loses it.],
  [Rule M permits it; *rule T* forbids it. M caps a unit by what it rests on,
   and its ground is `measured`, so M has no objection. T caps a unit by the
   *rung of whoever produced it* — a model reasoning from its own priors works
   at the `model` rung, whose ceiling is `inferred`. The two rules constrain
   different things, which is why both exist: M watches the graph, T watches
   the door.],
  [The line is the `--rebuts-->` edge from `c/canary-clean` to
   `c/pool-saturation`, and `f/root-cause`'s gist openly says the leading cause
   "is not consistent with the canary." For that to survive, the summariser
   would need
   to know that the contradiction was load-bearing rather than a hedge worth
   tidying — and nothing in a paragraph marks the difference. In the `.smy`
   file it is a typed edge that selection routines are *required* to keep with
   the claim (rule R), so no judgement about its importance is ever made.],
))

#recap((
  [A *pipeline* is a chain of steps where each one's output feeds the next.
   The loss this tool addresses happens in the *handoffs*, not in any step —
   every step can be done well and the artifact still degrades.],
  [Prose is the only medium every participant can produce and consume, and it
   cannot carry hedges, sources, or disagreement. Worse, it gives no signal
   when it drops them.],
  [Measured over five hops and two models: a prose chain kept 72 of 90 claims,
   turned 42 surviving hedged claims into flat assertions, and preserved 1 of
   50 sources. The same chain over `.smy` documents lost none of the three, at
   roughly 1.7× the tokens.],
  [A schema fixes the machines and blinds the reviewer; a careful prompt is an
   unverifiable instruction. `smysl` makes one artifact that is both checkable
   and readable.],
  [The tool judges no truth, resolves no disagreement, writes no prose, and
   cannot reach the network in fifteen of its seventeen commands.],
))

#chapter(number: 2, title: "The Shape of the System, and the Twelve Rules")

Chapter 1 argued that structure has to survive the handoff. This chapter is
about what actually enforces that — first the shape of the software, then the
twelve rules that are the substance of the guarantee.

You are not expected to memorise the rules. They are stated here because the
rest of this book names them by letter, in passing, dozens of times, and a
reader who has never been introduced to "rule M" has to reconstruct it from
context every time. This is the page to come back to.

#section("One binary, ten libraries")

#callout(label: "Why")[
  Knowing the layering answers a question you will otherwise hit in Chapter
  27: what can I use without taking all of it? The split is not decorative —
  the boundaries are where the guarantees are enforced, and two of the layers
  can be compiled out entirely.
]

`smysl` is one command-line binary sitting on a stack of libraries, each of
which is usable on its own. The important structural fact is that the layer
holding the data model has *no* dependency on the layers that talk to models
or to the network — that direction of dependency is checked at build time, not
maintained by discipline.

#dtable(
  (auto, auto, 1fr),
  (
    ([Layer], [Crate], [What lives there]),
    ([*Kernel*], [`smysl-core`], [The types, the deterministic binary codec, the surface syntax parser, and the diagnostic catalogue. Everything else is built on this. It cannot open a socket.]),
    ([*Graph*], [`smysl-graph`], [The append-only store, its derived index, adjacency and lineage walking, merge, salience, and compaction.]),
    ([*Verification*], [`smysl-check`], [The `check` pipeline — the passes that enforce most of the rules below.]),
    ([*Selection*], [`smysl-pack`], [Budget-bounded, closure-complete selection: what survives when you have fewer tokens than document.]),
    ([*Structure*], [`smysl-thread`], [The five thread schemas and their deterministic derivation.]),
    ([*Voice*], [`smysl-render`], [Turning a graph into something a person reads — six target formats, driven by profiles.]),
    ([*Model boundary*], [`smysl-provider`], [The only crate that speaks HTTP. Provider mappings, retry and backpressure, the usage ledger.]),
    ([*Ingest boundary*], [`smysl-ingest`], [Prose in, staged units out — with the trust ceiling and the repair loop.]),
    ([*Harness*], [`smysl-eval`], [The evaluation harness from Chapter 1. Not published.]),
    ([*Terminal UI*], [`smysl-tui`], [The seven-pane interactive view.]),
  ),
)

Two of these — `smysl-provider` and `smysl-ingest` — are behind feature flags.
A build with them compiled out is a build that provably cannot reach a model,
which is a useful thing to be able to hand someone. Chapter 27 covers the flags
in detail.

#section("A kernel, and deliberate room outside it")

#term("Kernel")[
  The *kernel* is the fixed part of the data model: the unit types, the six
  status rungs, the relation kinds, and the record shapes. Everything the
  kernel defines, every conforming implementation must agree about.
]

Outside the kernel there is deliberate room. A header field the grammar does
not recognise is not an error — it is kept, verbatim, in the unit's payload,
and written back out in the same place. A record type from a later version
decodes to an "unknown" record rather than failing the parse. This is rule X
below, and its purpose is specific: a document from a team that records
something yours does not, or from a version of the format newer than your
binary, loses nothing by passing through your pipeline.

#section("Identity is content")

#callout(label: "Why")[
  This one fact explains more surprising behaviour than anything else in the
  system. Every reader eventually asks why a unit's identifier changed after
  an operation that "only" adjusted its status. This is the answer, and
  knowing it early saves the confusion.
]

A unit's identifier is a hash of its canonical binary encoding — its type, its
gist, its status, its grounds, its payload. Not a counter, not a UUID, not
something assigned. Two people who record the same thing, in different field
orders, on different machines, compute the same identifier; the encoding is
canonical, so field order and formatting cannot affect it.

The consequence follows immediately and is not optional: *any transform that
changes a unit changes its identity.* When a rule weakens a unit's status, the
result is a different unit with a different identifier, and everything that
pointed at the old one has to be moved to point at the new one — which the tool
does, transitively, because a dangling reference would be worse than the
original problem. When you author a violation yourself, `check` reports it
instead of silently rewriting your file (Chapter 21); the automatic weakening
path is the one `ingest` takes on a model's output, where there is no author to
tell.

#section("The twelve rules")

Each rule is a promise the tool keeps, enforced in one identified place rather
than by convention. The middle column is the promise; the right column is what
you would see go wrong without it.

#dtable(
  (auto, 1fr, 1fr),
  (
    ([Rule], [The promise], [What it prevents]),
    ([*M*], [A unit's status may not exceed its weakest ground.], [A `derived` claim resting on a `speculative` one reading as settled. Confidence can fall through a chain of support, never rise.]),
    ([*T*], [A unit may not exceed the ceiling of the rung it was produced at.], [Laundering. A model asserting from its own priors is capped at `inferred` however confidently it phrases the claim.]),
    ([*L*], [Closure: whatever a unit needs travels with it.], [A selection or a bundle that arrives referring to grounds that were left behind — a claim with its support cut off.]),
    ([*R*], [A selected claim's rebuttals are selected with it.], [The one-sided pack: compressing a document down to only the side that won, which is the third loss from Chapter 1 committed by your own tooling.]),
    ([*U*], [Merge is a join-semilattice union — commutative, associative, idempotent.], [Order-dependent results. Merging A then B differs from B then A, and nobody can reproduce anyone else's store.]),
    ([*I*], [Ingest always makes progress.], [A run that fails outright because one span of prose could not be structured. It degrades that span to opaque prose and continues.]),
    ([*S*], [Model output never enters a store directly.], [A model's proposal silently becoming part of your document. It lands in staging, and a human decision moves it.]),
    ([*V1*], [A rendering profile must render every status distinctly.], [Two different confidence levels coming out looking identical — the hedge loss, reintroduced at the last step.]),
    ([*V2*], [Open disagreements are surfaced in the output.], [A rendered report that reads unanimous over a document that was not.]),
    ([*X*], [Unknown extensions survive verbatim.], [Data loss on round-trip through a version or a peer that did not know about some field.]),
    ([*D*], [Pure operations are bit-reproducible.], [Silent drift — a command that quietly stops being a function of its inputs, so a rebuild produces a different artifact and nobody knows which was right.]),
    ([*P*], [`stdout` defaults to the binary form when it is not a terminal.], [The pipeline surprise: piping a command into another and getting human-formatted text where structure was expected.]),
  ),
)

#subsection("M and T are the pair that matter")

If you remember two, remember these. They are the mechanical form of *a guess
cannot quietly become a fact*, and they work at different places.

Rule T stops it at the door. A model producing units is working at a *rung* —
`computed`, `document`, `web`, or `model` — and the rung caps the highest status
it may claim. A model reasoning from its own knowledge is at the `model` rung
and cannot exceed `inferred`, no matter what it writes.

Rule M stops it inside the graph. Once units are in, confidence flows through
`grounds`, and it can only fall: a claim resting on a `speculative` unit is
itself no better than speculative, whatever status was written on it.

#callout(label: "Where the rules live")[
  Rules are not enforced by everyone remembering them. Each has an address:
  M and T are passes 6 and 7 of `check`, and are also applied at ingest; L is
  pass 4 and the thread repair step; R is constraint C3 of the packer; U is
  the merge implementation; I is the ingest repair loop; S is the staging file
  and exit code `10`; V1 is profile loading and V2 is render IR construction;
  X is the extension-carrying maps; P is the CLI's format selection. D is the
  odd one out — it is enforced by a test, `cargo xtask determinism`, which
  builds permutations of the same inputs specifically to catch a pure command
  that has stopped being one.
]

#whatsnext[
  Chapter 3 turns from what the system promises to how you will actually
  interact with it: what a store is, which records you write versus which the
  tool writes, and which commands can cost you money. Chapter 5 then builds a
  real document end to end.
]

#exercises((
  [Run `smysl check fixtures/corpus/F6-adversarial.smy`. It reports three
   `SMY-E030` errors. Which of the twelve rules is `SMY-E030`, and what does
   the fact that there are exactly three of them tell you about how that rule
   is checked — once per document, or once per unit?],
  [Read one of those three messages closely. It offers two ways out: "weaken
   this unit" or "strengthen its ground". Why is a tool that enforces "*status
   may only fall*" willing to suggest strengthening something?],
  [`smysl fmt` on any corpus file returns the same records with fields in
   canonical order. Given that a unit's identity is a hash of its canonical
   encoding, what would break if `fmt` sorted fields the way you typed them
   instead? Name a specific two-person scenario.],
))

#answers((
  [Rule M — a unit's status may not exceed its weakest ground. Three errors
   means it is checked *per unit*, not per document: `check` walks every unit
   and compares it against its own grounds, so a document with thirty
   violations reports thirty. That is what makes the output actionable — each
   message names the offending unit and the ground that capped it.],
  [Because rule M constrains the *relationship* between a unit and its
   grounds, not the absolute level of either. There are genuinely two ways to
   restore the relationship, and the tool does not know which one is right —
   maybe you understated the evidence, maybe you overstated the claim. Only
   one of those is a licence to raise a status: strengthening the ground is
   subject to rules M and T in its own right, so the suggestion cannot be used
   to launder anything. The tool declines to guess and says so.],
  [Two people record the same evidence with the same content but type the
   fields in different orders. With canonical ordering, both compute the same
   uid and `merge` recognises them as one unit. With as-typed ordering they
   compute different uids, so the merged store holds two units that are
   identical in meaning and unrelated in identity — every `grounds` reference
   points at one or the other, and a `--rebuts-->` edge aimed at one leaves
   the other unopposed.],
))

#recap((
  [One binary over ten libraries; the kernel has no path to the network, and
   the two crates that do can be compiled out.],
  [The kernel is fixed and small; everything outside it survives verbatim
   rather than being rejected, so a peer or a version that knows more than you
   loses nothing passing through.],
  [A unit's identity is a hash of its canonical content — so any transform
   that changes a unit moves its identity, and everything pointing at it is
   moved too.],
  [Twelve rules, each enforced at a named address rather than by convention.
   M and T are the pair worth memorising: T caps what a producer may claim,
   M caps what a claim may inherit from its support. Neither lets confidence
   rise.],
))
