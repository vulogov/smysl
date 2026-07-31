// smysl — Rationale
// Why one document, and what it costs information to skip.
//
// Compile with:
//   typst compile Documentation/SMYSL_RATIONALE.typ
//
// Palette, term/callout/recap boxes and fletcher diagram vocabulary are
// adapted from Book/RESEARCH/design.typ (blackInkhaven) — warm paper, cool
// ink, the trust-ladder idiom — reskinned for a single standalone document
// rather than a multi-chapter book. Uses fletcher (Typst Universe) for the
// three diagrams instead of screenshots.

#import "@preview/fletcher:0.5.8" as fletcher: diagram, node, edge

// ── Palette — warm paper, cool ink, restrained accents (as reviewed) ────────
#let ink_black    = rgb("#1a1a1a")
#let ink_gray     = rgb("#5d5d5d")
#let ink_faint    = rgb("#9a9a9a")
#let ink_rule     = rgb("#c6c0b5")
#let ink_accent   = rgb("#2f5d7a")   // slate blue — the analytic thread
#let ink_smoke    = rgb("#7d736a")   // muted brown — eyebrow labels
#let ink_paper    = rgb("#fdfaf3")   // warm cream — masthead ground
#let ink_term     = rgb("#2f5d7a")
#let ink_code_bg  = rgb("#f3eee4")
#let ink_call_bg  = rgb("#f6f1e6")
#let ink_term_bg  = rgb("#eef3f7")
#let ink_recap    = rgb("#3f6b4a")   // muted green — the smysl badge's own green, kept
#let ink_recap_bg = rgb("#e9f3ea")

#let body_family = ("Libertinus Serif", "New Computer Modern")
#let mono_family = ("DejaVu Sans Mono",)

#set document(title: "smysl — Rationale", author: "Vladimir Ulogov")
#set page(
  paper: "a4",
  margin: (inside: 26mm, outside: 20mm, top: 22mm, bottom: 24mm),
  numbering: "1", number-align: center,
  header: context {
    if counter(page).get().first() > 1 {
      align(center, text(font: body_family, size: 8pt, fill: ink_faint,
        tracking: 1.5pt, upper("smysl — rationale")))
    }
  },
)
#set text(font: body_family, size: 11pt, fill: ink_black, lang: "en")
#set par(leading: 0.72em, justify: true)

#show raw.where(block: true): it => block(
  fill: ink_code_bg, stroke: 0.5pt + ink_rule, inset: 7pt, radius: 2pt, width: 100%,
  text(font: mono_family, size: 9pt, it),
)
#show raw.where(block: false): it => box(
  fill: ink_code_bg, inset: (x: 2pt, y: 0pt), outset: (y: 2pt), radius: 1pt,
  text(font: mono_family, size: 9.5pt, it),
)

// ── Section / subsection — hidden-outline headings, real breathing room ────
#let section(title) = {
  hide(heading(level: 2, numbering: none, outlined: true, title))
  block(sticky: true, above: 8mm, below: 3.2mm,
    text(font: body_family, size: 15pt, weight: "bold", fill: ink_black, title))
}

// ── Term box ─────────────────────────────────────────────────────────────
#let term(name, body) = {
  v(2mm)
  block(
    fill: ink_term_bg, stroke: (left: 2pt + ink_term),
    inset: (left: 9pt, right: 9pt, top: 7pt, bottom: 7pt),
    width: 100%, radius: 1pt, breakable: false,
    {
      text(font: body_family, size: 8pt, weight: "bold", fill: ink_term, tracking: 1pt, "TERM")
      h(6pt)
      text(font: body_family, size: 11pt, weight: "bold", fill: ink_term, name)
      v(2mm)
      body
    },
  )
  v(2mm)
}

// ── Note callout ─────────────────────────────────────────────────────────
#let callout(label: "Note", body) = {
  v(2mm)
  block(
    fill: ink_call_bg, stroke: (left: 2pt + ink_accent),
    inset: (left: 9pt, right: 9pt, top: 7pt, bottom: 7pt),
    width: 100%, radius: 1pt, breakable: false,
    {
      text(font: body_family, size: 8pt, weight: "bold", fill: ink_accent, tracking: 1.5pt, upper(label))
      v(2mm)
      body
    },
  )
  v(2mm)
}

// ── Chapter-end-style recap ──────────────────────────────────────────────
#let recap(items) = {
  v(7mm)
  block(
    fill: ink_recap_bg, stroke: (left: 2pt + ink_recap),
    inset: (left: 9pt, right: 9pt, top: 8pt, bottom: 8pt),
    width: 100%, radius: 1pt, breakable: false,
    {
      text(font: body_family, size: 9pt, weight: "bold", fill: ink_recap, tracking: 1.5pt, "WHAT TO CARRY AWAY")
      v(2mm)
      list(..items)
    },
  )
}

// ── Terminal screen — a faithful monospace rendering of CLI output ────────
#let screen(caption: "", body) = {
  v(2mm)
  block(breakable: false, width: 100%, {
    block(
      fill: ink_smoke, inset: (left: 8pt, right: 8pt, top: 3pt, bottom: 3pt),
      width: 100%, radius: (top-left: 2pt, top-right: 2pt),
      {
        text(font: mono_family, size: 8pt, fill: ink_paper, "● ● ●")
        h(6pt)
        text(font: body_family, size: 8.5pt, style: "italic", fill: ink_paper, caption)
      },
    )
    block(
      fill: ink_code_bg, stroke: 0.5pt + ink_rule, inset: 8pt, width: 100%,
      radius: (bottom-left: 2pt, bottom-right: 2pt),
      text(font: mono_family, size: 8.5pt, body),
    )
  })
  v(2mm)
}

#let figure_note(body) = align(center,
  text(font: body_family, style: "italic", size: 9pt, fill: ink_gray, body))

// ── Fletcher node style, shared across the three diagrams ───────────────
#let dnode(pos, body, fill: ink_call_bg) = node(
  pos, align(center, text(font: body_family, size: 8.5pt, body)),
  stroke: 0.6pt + ink_rule, fill: fill, corner-radius: 2pt, inset: 6pt,
)

// Diagram 1 — the ordinary hand-off loses three things; the smysl hand-off
// is the same artifact observed at four points, so there is nothing to lose.
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
  figure_note[Top row: prose re-told four times, losing something specific at each hop. Bottom: one document, present unaltered at all four stages — there is no re-telling to lose anything in.]
  v(3mm)
}

// Diagram 2 — the trust ladder, doubling as a picture of the Data Processing
// Inequality: a hop may move a claim down, never up, and this is the spine
// of the whole document.
#let smysl_ladder() = {
  v(3mm)
  align(center, diagram(
    spacing: (22mm, 5mm),
    dnode((0, 0), [*measured*\ an instrument recorded it], fill: ink_recap_bg),
    dnode((0, 1), [*cited*\ a source you can open says so]),
    dnode((0, 2), [*derived*\ computed from what's above it]),
    dnode((0, 3), [*inferred*\ a model reasoned it out]),
    dnode((0, 4), [*speculative*\ offered, not yet grounded]),
    dnode((0, 5), [*unfounded*\ reachable only by retracting], fill: ink_code_bg),
    // Plain connectors, not arrows. These join the rungs of a ladder — they say
    // which rung sits above which, and nothing about direction of travel. Drawn
    // as arrowheads they pointed *up*, alongside a caption reading "may only move
    // down", and the eye believes the arrowhead over the sentence.
    edge((0, 5), (0, 4), "-"), edge((0, 4), (0, 3), "-"),
    edge((0, 3), (0, 2), "-"), edge((0, 2), (0, 1), "-"),
    edge((0, 1), (0, 0), "-"),
    // The one arrow in the figure, carrying the one directional claim. `label-side`
    // is relative to the direction of travel, so a downward arrow needs `left` to
    // keep its caption on the page's right.
    edge((1, -0.2), (1, 5.2), "->", stroke: 1pt + ink_accent,
      label: text(font: body_family, size: 8pt, fill: ink_accent, [a hop may only\ move *down*]),
      label-side: left),
  ))
  figure_note[Six rungs, checked mechanically on every unit: nothing here is a matter of tone or house style — it is validated the same way a type error is.]
  v(3mm)
}

// Diagram 3 — the information bottleneck behind `pack --budget --focus`:
// everything not relevant to the focus is dropped hard; anything that would
// object to what survives travels with it, or the pack fails outright.
#let bottleneck_squeeze() = {
  v(3mm)
  align(center, diagram(
    spacing: (13mm, 4mm),
    dnode((0, 0), [full corpus\ every unit you kept]),
    dnode((1, 0.5), [*budget: N tokens*\ *focus: one claim*], fill: ink_call_bg),
    dnode((2, 0), [claim + its rebuttal\ kept together], fill: ink_recap_bg),
    dnode((2, 1), [low-value unit\ dropped], fill: ink_code_bg),
    edge((0, 0), (1, 0.5), "->"),
    edge((1, 0.5), (2, 0), "->"),
    edge((1, 0.5), (2, 1), "->", stroke: (paint: ink_faint, thickness: 0.7pt, dash: "dashed")),
  ))
  figure_note[A budget too small to hold a claim *and* its rebuttal fails the pack outright, rather than shipping the claim looking uncontested.]
  v(3mm)
}

// ═══════════════════════════════════════════════════════════════════════
// Masthead
// ═══════════════════════════════════════════════════════════════════════

#block(
  width: 100%, fill: ink_paper, radius: 2pt,
  stroke: 0.6pt + ink_rule,
  inset: (x: 14mm, y: 13mm),
  align(center)[
    #image("images/logo.png", width: 20mm)
    #v(4mm)
    #{
      let dot(r) = circle(radius: r, fill: ink_accent)
      stack(dir: ltr, spacing: 4mm, dot(0.7mm), dot(1.2mm), dot(0.7mm))
    }
    #v(6mm)
    #text(font: body_family, size: 9pt, tracking: 3pt, fill: ink_gray, upper("Rationale"))
    #v(3mm)
    #text(font: body_family, size: 21pt, weight: "bold", fill: ink_black,
      "Why one shared document, and what it costs to skip it")
    #v(5mm)
    #line(length: 34%, stroke: 0.6pt + ink_accent)
    #v(5mm)
    #text(font: body_family, size: 10pt, style: "italic", fill: ink_smoke,
      "Vladimir Ulogov · 2026 · smysl 0.5.0 · format smysl/0.1 · kernel smysl.kernel/0.1")
  ],
)
#v(6mm)

This is a document about a narrow, specific problem: what happens to a claim as
it is handed from one system to the next — model to model, model to person,
person back to model — and why the usual fix for that problem, a bespoke wire
format between each pair of systems, only ever solves half of it. `smysl` is a
plain-text document format built to close the other half. This document makes
the case for it twice: once in the terms anyone doing this work already
reasons in, and once in the terms information theory gives a name to. The two
are the same argument; the second is just precise about what the first was
gesturing at.

#section("The problem, plainly")

When one AI system hands work to another, what actually crosses the wire is
prose. The system that wrote it *had* a structure in mind while it worked —
this rests on that, this is a guess, these two things disagree — and then
collapsed all of it into paragraphs anyway, because paragraphs are the only
thing on offer. The next system in the chain has to guess that structure back
out of the prose, act on its guess, and then flatten its own version into
prose again for whoever is next. Do this more than once and the same three
things go missing every time:

#list(
  [*Hedges vanish.* "The data suggests" becomes "the data shows" becomes "we
   found" — a chain of retellings, each one slightly more confident than the
   one before it, with nobody having earned the extra confidence.],
  [*Sources evaporate.* By the third hop nobody can say where a number came
   from, because nothing about a citation survives being paraphrased.],
  [*Disagreement disappears.* Two conflicting findings go in; whichever one
   the last system preferred comes out. The conflict is not resolved — it is
   just dropped, and the output looks unanimous.],
)

#lossy_hops()

The obvious fix is a schema: agree on a JSON shape between each pair of
systems and stop passing free text. That solves the problem for the machines
and creates a new one for everyone else — nobody reviews a wire format, so the
one place a person could have caught this drift is now the one place they
cannot see. `smysl` takes a different position: make the artifact itself
precise enough to carry hedges, sources and disagreement structurally, *and*
plain enough that the same file is what a person opens to check the work.

#section("What that costs, measured")

The three losses above are the ones everybody who has run a multi-hop pipeline
already believes in. It is worth knowing how large they actually are, because
the answer is not "a little, at the margins" — and because a pipeline losing
them gives no sign that it has. Prose does not announce what it dropped.

So: five documents, five hops each, summarised by a real model at every hop,
run twice over — once through `gemini-3.5-flash-lite` and once through
`deepseek-chat`. Ninety claims in total. A second model reads the output
afterwards and is asked, for each original claim, whether the final text still
states it, how confidently the *text* now puts it, and what the text says it
rests on.

#screen(caption: "the evaluation harness, `make eval-live`")[
```
                          tokens   claims kept   hedges lost   sources kept
  control (no summarising)  1.00        90/90          0/90          50/50
  prose baseline            0.29        72/90         42/90           1/50
  smysl                     0.49        90/90          0/90          50/50
```
]

Read the bottom two rows against each other. The prose chain compresses
*harder* than it was asked to — 0.29 of the original against smysl's 0.49 —
and pays for it twice. Forty-two of ninety surviving claims came out the far
end reading as measurements when the document that went in had called them
inferred, cited or derived. And of the fifty claims that named where they came
from, exactly one still named it.

The top row is what makes the other two mean anything. The same judge, the
same prompt, reading the document *before* any summarising, recovers every
hedge and every source — it declined to rule on none of the ninety. Whatever
went missing over five hops went missing in the chain, not in the instrument
that measured it.

#callout(label: "The point")[
  Nothing here is a criticism of the models. Both were asked to summarise and
  both summarised well — the output reads fluently and says broadly the right
  things. Hedges and citations are simply not what prose is built to preserve,
  and a summariser has no way to know it dropped one. On the `smysl` side both
  numbers are structural rather than lucky: confidence is a field that a rule
  checks, and a source is a field that travels with whatever travels.
]

Two models, five documents, one run each — enough to say the effect is not one
model's quirk, not enough to put an error bar on it.

#section("What the document actually looks like")

There is no separate machine version. This is a complete, real excerpt — the
review copy and the wire format are the same bytes:

```
@evidence e/pool-wait { status: measured, source: { kind: metric, ref: "pool.wait_ms" } }
~ Pool acquisition wait rose from 2 ms to 310 ms over the same window.

@claim c/pool-saturation { status: inferred, grounds: [e/pool-wait] }
~ The eu-west connection pool is saturated.

@rel c/canary-clean --rebuts--> c/pool-saturation { weight: 0.6 }
```

#term("Unit")[
  A *unit* is one recorded thing — a piece of evidence, a claim, a finding, a
  definition. Every unit carries a *status* (how sure the document is entitled
  to be about it) and, where it rests on something else, a `grounds` list
  naming exactly which units it depends on. A `rel` records a relationship
  *between* units — `rebuts`, `causes`, `warrant` — including disagreement,
  which is a first-class thing to record rather than an anomaly to hide.
]

Three things are being carried here that a paragraph would have thrown away.
First, *how sure* the document is entitled to be: `measured` (an instrument
recorded it) is not the same claim as `inferred` (a model reasoned it out),
and the difference is a field, not a tone of voice. Second, *what it rests
on*: `grounds: [e/pool-wait]` means the claim stands or falls with that one
piece of evidence — retract the evidence and the tool can tell you exactly
what falls with it. Third, *what contradicts it*: the `rebuts` edge is part of
the document, sitting right next to the claim it argues with, not filed away
in a review thread nobody reads twice.

#section("The trust ladder, and why it can only fall")

Every unit's status sits on a six-rung ladder, and the document's central
mechanical rule is about that ladder: a claim's status can be *weakened* as it
crosses a hop, but never *strengthened*. A model reading this document may
propose at most `inferred`; if it writes `measured` anyway, the tool
downgrades it and says so, before the claim ever reaches whoever reads it
next.

#smysl_ladder()

That single rule is doing more work than it looks like. It is the mechanism
that stops the "the data suggests" → "we found" drift described above from
ever compounding, because there is no step in the chain where confidence is
allowed to go up, only ones where it is allowed to fall or hold.

#section("The same rule, said precisely")

Everything so far has been the practitioner's version of one argument: a hop
must never come away knowing a claim more certainly than the hop before it
did. Information theory has an exact name for that argument, and getting to
it takes three short steps in order — what information *is*, what a
multi-hop pipeline *is* as a mathematical object, and what necessarily
happens to information as it crosses one.

#term("Information")[
  Used here in Shannon's sense: information is not "content," it is the
  amount by which a message reduces someone's uncertainty about something. A
  coin flip you already saw carries no information; one you have not carries
  some. A claim that only restates what you already believed carries less
  information than one that would change what you are willing to act on.
]

A single message's information is not yet the interesting quantity, though —
a pipeline is many messages end to end, each one built only from the one
just before it. That shape has a name too.

#term("Markov chain")[
  A chain of steps, `A → B → C`, in which each step depends only on the one
  immediately before it. Once you know `B`, learning more about `A` cannot
  tell you anything further about `C` that `B` did not already carry. A
  multi-hop pipeline behaves exactly this way: each hop only ever sees what
  the hop before it handed forward.
]

Model, then model, then person, then model again is a chain of exactly that
shape. And once a chain has that shape, one inequality governs everything
that is allowed to happen to information as it moves along it.

#term("Data Processing Inequality")[
  For any chain `X → Y → Z`, the information `Z` carries about `X` can never
  exceed the information `Y` carried about `X`. Processing can preserve
  information or destroy it, but it cannot manufacture more of it than
  arrived. Photocopy a photocopy and you do not get a sharper original — you
  get a copy of a copy, at best.
]

#callout(label: "The point")[
  `smysl`'s falling-only status rule *is* the Data Processing Inequality,
  applied to one number instead of a whole distribution, and checked on every
  document rather than trusted to whoever is doing the copying. This is not
  an analogy borrowed after the fact — it is what makes the rule correct
  engineering rather than a house style: no hop can legitimately know a claim
  more certainly than the hop before it did, so no hop is allowed to say so.
]

#section("Compressing without losing the argument")

The inequality above is a ceiling: it stops a hop from *inflating* what it
is entitled to claim. It has nothing to say about a second, equally ordinary
problem — every hop also has to fit inside a budget, which means throwing
part of the document away *on purpose*. That is a different question, with
its own exact answer, and it is the question `pack` exists to answer.

A document does not always travel whole. `pack` fits a document to a token
budget, and it has to decide what survives the cut:

#screen(caption: "fixtures/corpus/F1-incident.smy, packed to 200 tokens")[
```
$ smysl --format surface pack --budget 200 --explain --focus c/pool-saturation F1-incident.smy
b3:cvhirtgs2mpvli2ethhyeo32uf @L0  -  earned on density
b3:phsoomklkmlq3sjvbe6cyuqy5v @L0  C3  rebuts b3:cvhirtgs2mpvli2ethhyeo32uf
b3:xkys7j42mcuyiaxiyh73xddimr dropped: low-value
F1-incident.smy: 7 of 8 unit(s), 193 of 200 tokens, greedy mode, gap 0.011
```
]

Read the middle line again: that unit was kept *because* it rebuts one that
was also kept. The two vocabulary items below are what make that not a
coincidence of the implementation.

#term("Rate–distortion")[
  Fix a budget of bits (the *rate*) and a way of scoring how far a compressed
  version departs from the original (the *distortion*). For any rate there is
  a best fidelity achievable — a real, computable number, not a rule of
  thumb. Spend fewer bits and the achievable fidelity only gets worse; there
  is a curve, and you choose where on it you stand, not whether it applies.
]

#term("Information bottleneck")[
  A refinement of rate–distortion for when fidelity to the *whole* original
  does not matter — only fidelity to one downstream question does. The
  method compresses hard everywhere except along whatever predicts the
  answer to that one question, and lets the rest go without ceremony.
]

`--focus c/pool-saturation` names the question; `--budget 200` names the
rate. What must survive is not "the most important-sounding sentences" but
whatever the focus claim's answer actually depends on — which is why its
rebuttal travels with it and a lower-density unit does not. A budget too
small to hold both the claim and its objection does not quietly ship the
claim alone and call it packed; the operation fails, loudly, on stderr.

#bottleneck_squeeze()

#section("Merging without a referee")

Packing decides what a *single* hop is allowed to keep. The last piece is
what happens when *two* hops — two agents, working from the same corpus at
the same time — hand back documents that were never reconciled with each
other. Nothing above solves that; it needs its own operation.

Two agents can work on the same document at once, and neither has to wait for
the other or ask a coordinator who goes first.

#screen(caption: "two independently-authored documents, combined")[
```
$ smysl merge agent-a.smy agent-b.smy -o combined.cbor
agent-b.smy: contention k/ccm3actwjjti65famnoe6mapo5d over b3:cvhirtgs2mpvli2ethhyeo32uf
  (2 positions, live-rebuttal)
```
]

#term("Canonical identity")[
  An identifier computed from *what a unit says*, not from where it sits in a
  file or who wrote it down. Two agents that independently record the exact
  same claim compute the exact same identifier without comparing notes, and
  a later merge collapses the two into one entry automatically — there is no
  registry to consult and nobody assigning ids.
]

#term("Join")[
  From lattice math: given two partial pictures of the same situation, their
  *join* is the smallest single picture containing everything both of them
  knew — the least upper bound. A join is commutative and order-independent:
  it makes no difference which partial picture the merge sees first, and it
  never has to choose a winner to produce an answer.
]

Merging two `smysl` documents is a join over their units, keyed by canonical
identity. Where the two genuinely disagree — not the same claim twice, but
two different claims about the same thing — the join does not silently
prefer whichever arrived second, the way an ordinary "last write wins" merge
would. It records the disagreement as a *contention*, on the document, for a
person to resolve deliberately. Nothing is picked for you behind your back.

#section("Finding things again")

A pipeline that has run for a while produces a store nobody has read. It holds
units written by agents on other machines, named by hashes, and the question
you arrive with is not "what does the graph say is important" but "where is
the part about the connection pool".

`salience` cannot answer that. It ranks by structure — what grounds what, what
the argument leans on — and never reads a word. So `find` ranks by words
instead, over the *gist* of every unit.

That last detail is what makes it work at all. A unit's payload might be a
stack trace, a table of measurements, a diff or a page of prose, and no single
way of searching covers all four. But every unit carries a gist, because the
format requires one, and a gist is a sentence about whatever the payload
happens to be. You never search the telemetry. You search the sentence
somebody — or some model — wrote about it.

It is worth saying plainly what this is not. It is lexical search: it matches
words, not meanings. Measured over the reference corpus it is perfect on
identifiers like `pool.wait_ms` and near-perfect when your words match the
author's, and it is weak when they do not — a paraphrased query finds the right
claim somewhere in the top five three times in four, but puts it first only
once in eight. Evidence and data are found reliably because they name concrete
things; claims are found less reliably because a claim is an interpretation,
and interpretations get phrased differently by different people.

The honest position is that this is the floor rather than the ceiling, and it
was built as a seam so the ceiling can be raised without disturbing anything
beneath it. What it buys today is that a store stops being opaque, with no
model, no index on disk, no network call, and the same answer on every machine.

#section("Where it sits in a pipeline")

None of the above requires rebuilding a pipeline, and it is worth being
concrete about what adopting it does and does not involve, because the honest
answer is narrower than the argument might suggest.

There is exactly one place a model is needed: the entrance. `ingest` turns a
document into units, and it is the only operation here that costs a model
call. Everything downstream of it — fitting to a budget, merging two agents'
work, ordering a reading, rendering an artifact, checking any of it — is
ordinary computation over the graph. Same input, same output, byte for byte,
no inference and no variance.

That is the shape of the trade. A pipeline that summarises at every hop pays a
model at every hop and cannot tell you what each one discarded. A pipeline
carrying `smysl` pays a model once at the boundary and then stops paying:
the five hops in the measurement above cost five model calls on the prose side
and zero on the other.

#term("The boundary")[
  Model output does not enter the graph directly. It is parsed, checked
  against the rules above, and written to a staging file for a person or a
  later step to accept — `merge --staged` is that acceptance. A model
  overstating its confidence is corrected and the correction recorded, at the
  door, before anything downstream can rest on it.
]

Two further things follow from that boundary being the only model-dependent
part. Instrument data does not go through it at all: `import` transcribes a
table of readings directly, which is the only path allowed to record
`measured`, because a tool copying a number is doing something a model reading
a document cannot. And adoption is incremental — one pipeline, or even one
hop of one pipeline, produces documents that the rest of the system can keep
treating as text, because that is what they are.

#callout(label: "What it does not do")[
  The boundary is the one place with no guarantee behind it. A model
  converting prose to units can misread the prose, and everything downstream
  inherits whatever it got wrong; the rules constrain what it may *claim*, not
  whether it read correctly. What the format offers is that from the boundary
  onward nothing degrades further, and that what did arrive is reviewable by
  the person it arrives at.
]

#section("What this buys, both ways")

#recap((
  [A claim and its citation travel as one artifact from the model that wrote
   it to the person who reviews it — nothing is translated, so nothing is
   lost in translation.],
  [Confidence falling but never rising across a hop is the Data Processing
   Inequality, engineered into a document format and checked mechanically,
   not left to whoever happens to be doing the summarising.],
  [Fitting a document to a budget is an information bottleneck around the
   question you actually asked: what the focus claim depends on survives,
   including its rebuttal, or the operation fails rather than mis-reporting.],
  [Merging is a mathematical join — commutative, order-independent, and
   honest about disagreement — computed from what a claim *says*, so two
   agents recording the same fact need no registry to agree on its identity.],
  [Across five documents and two models, prose summarising lost 42 of 90
   hedges and 49 of 50 citations while compressing *harder* than the format
   did; on the `smysl` side both are zero by construction, not by care.],
  [One model call at the entrance, none afterwards: every operation downstream
   of ingest is deterministic computation over the graph rather than another
   round of inference.],
))
