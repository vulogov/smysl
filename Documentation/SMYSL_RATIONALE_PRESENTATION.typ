// smysl — the rationale, as a talk.
//
// Source material: Documentation/SMYSL_RATIONALE.typ and Chapter 1 of the
// manual (Documentation/manual/ch01-02-why-shape.typ). Same argument, same
// measured numbers, cut to fifteen cards.
//
// One card carries one point. If a card needs a second sentence to justify
// its own headline it has been split. Language is kept plain on purpose —
// this is the deck you give to somebody who has never built a pipeline and
// has fifteen minutes.
//
// Compile with:
//   SOURCE_DATE_EPOCH=0 typst compile Documentation/SMYSL_RATIONALE_PRESENTATION.typ

#import "@preview/touying:0.6.1": *
#import "@preview/fletcher:0.5.8" as fletcher: diagram, node, edge

// ── Palette — identical to the other four smysl documents ──────────────────
#let ink_black    = rgb("#1a1a1a")
#let ink_gray     = rgb("#5d5d5d")
#let ink_faint    = rgb("#9a9a9a")
#let ink_rule     = rgb("#c6c0b5")
#let ink_accent   = rgb("#2f5d7a")
#let ink_smoke    = rgb("#7d736a")
#let ink_paper    = rgb("#fdfaf3")
#let ink_code_bg  = rgb("#f3eee4")
#let ink_call_bg  = rgb("#f6f1e6")
#let ink_recap    = rgb("#3f6b4a")
#let ink_recap_bg = rgb("#e9f3ea")
#let ink_alarm    = rgb("#8a3b2b")

#let body_family = ("Libertinus Serif", "New Computer Modern")
#let mono_family = ("DejaVu Sans Mono",)

// ── Deck chrome ────────────────────────────────────────────────────────────
#let deck-title = "smysl"
#let deck-sub   = "Keeping meaning between machines"

#show: touying-slides.with(
  config-page(
    paper: "presentation-16-9",
    margin: (x: 2.4em, y: 2.0em),
    fill: ink_paper,
  ),
  config-common(
    slide-fn: slide,
    // Every card is one point, so no card ever needs to advance in steps.
    handout: true,
  ),
  config-methods(
    alert: (self: none, it) => text(fill: ink_alarm, weight: "bold", it),
  ),
)

#set text(font: body_family, size: 21pt, fill: ink_black, lang: "en")
#set par(leading: 0.72em, justify: false)

#show raw.where(block: false): it => box(
  fill: ink_code_bg, inset: (x: 3pt, y: 0pt), outset: (y: 3pt), radius: 1pt,
  text(font: mono_family, size: 0.82em, it),
)
#show raw.where(block: true): it => block(
  fill: ink_code_bg, stroke: 0.5pt + ink_rule, inset: 10pt, radius: 2pt, width: 100%,
  text(font: mono_family, size: 13pt, it),
)

// ── Card furniture ─────────────────────────────────────────────────────────

// A card's headline. One line, and it is the whole claim of the card.
#let headline(body) = {
  block(below: 0.9em,
    text(size: 30pt, weight: "bold", fill: ink_black, body))
  block(above: 0em, below: 1.0em,
    line(length: 100%, stroke: 0.6pt + ink_rule))
}

// The one-sentence takeaway some cards close on.
#let landing(body) = {
  v(0.5em)
  block(
    fill: ink_recap_bg, stroke: (left: 2.5pt + ink_recap),
    inset: (left: 11pt, right: 11pt, top: 9pt, bottom: 9pt),
    width: 100%, radius: 1pt,
    text(size: 19pt, fill: ink_black, body),
  )
}

// The same, when the point is a warning rather than a reassurance.
#let alarm(body) = {
  v(0.5em)
  block(
    fill: rgb("#f7ece8"), stroke: (left: 2.5pt + ink_alarm),
    inset: (left: 11pt, right: 11pt, top: 9pt, bottom: 9pt),
    width: 100%, radius: 1pt,
    text(size: 19pt, fill: ink_black, body),
  )
}

// `shape: "rect"` is explicit because fletcher picks a shape from the aspect ratio
// otherwise, and a short label like "signs it off" comes out as a circle among rectangles.
#let dnode(pos, body, fill: ink_call_bg, stroke: 0.7pt + ink_rule) = node(
  pos, align(center, text(font: body_family, size: 14pt, fill: ink_black, body)),
  stroke: stroke, fill: fill, corner-radius: 3pt, inset: 9pt, shape: "rect",
)

#let elabel(body) = text(font: body_family, size: 11pt, fill: ink_alarm, body)

// The five-dot ornament from the manual cover, used on the art cards.
#let dots(accent: ink_accent) = {
  let d(dx, r) = place(top + center, dx: dx, dy: 0pt, circle(radius: r, fill: accent))
  d(-16mm, 1.5mm); d(-8mm, 1.0mm); d(0mm, 2.1mm); d(8mm, 1.0mm); d(16mm, 1.5mm)
}

// ═══════════════════════════════════════════════════════════════════════════
// 1 — Title card
// ═══════════════════════════════════════════════════════════════════════════
#slide(config: config-page(margin: 0em))[
  #set page(fill: ink_paper)
  #block(width: 100%, height: 100%)[
    #place(top + left, dx: 9mm, dy: 9mm,
      rect(width: 100% - 18mm, height: 100% - 18mm, stroke: 1.1pt + ink_accent))
    #place(top + left, dx: 11mm, dy: 11mm,
      rect(width: 100% - 22mm, height: 100% - 22mm, stroke: 0.4pt + ink_accent))

    #place(top + center, dy: 15mm, image("images/logo.png", width: 26mm))
    #place(top + center, dy: 46mm, dots())

    #place(top + center, dy: 55mm, block(width: 78%)[
      #align(center)[
        #text(size: 62pt, weight: "bold", fill: ink_black, deck-title)
        #v(2mm)
        #text(size: 22pt, style: "italic", fill: ink_smoke, deck-sub)
        #v(7mm)
        #line(length: 42%, stroke: 0.6pt + ink_accent)
        #v(6mm)
        #text(size: 16pt, fill: ink_gray,
          "Why a document format, and what it is measured to save")
      ]
    ])

    #place(bottom + center, dy: -13mm,
      text(size: 12pt, fill: ink_smoke, "Vladimir Ulogov · 2026 · smysl 0.3.0"))
  ]
]

// ═══════════════════════════════════════════════════════════════════════════
// 2 — What a pipeline is
// ═══════════════════════════════════════════════════════════════════════════
#slide[
  #headline[A pipeline is a chain of handoffs]

  Something reads. A model writes. A second model merges. A person signs off.
  Each step passes its result to the next.

  #v(0.7em)
  #align(center, diagram(
    spacing: (17mm, 10mm),
    dnode((0, 0), [reads the\ raw material]),
    dnode((1, 0), [drafts an\ account]),
    dnode((2, 0), [merges a\ second view]),
    dnode((3, 0), [signs it\ off]),
    edge((0, 0), (1, 0), "->"),
    edge((1, 0), (2, 0), "->"),
    edge((2, 0), (3, 0), "->"),
  ))
  #v(0.3em)

  #landing[
    Nothing here is new, and none of it needs AI. Models only made the
    handoffs cheap enough to do many times.
  ]
]

// ═══════════════════════════════════════════════════════════════════════════
// 3 — What actually crosses the arrow
// ═══════════════════════════════════════════════════════════════════════════
#slide[
  #headline[What crosses each arrow is prose]

  It is the only thing every participant can both write and read. So it is
  what gets used.

  #v(0.6em)

  But the model that wrote it knew more than it said. It knew which number
  came off a dashboard and which sentence was its own guess. It knew the
  canary data pointed the other way.

  #v(0.5em)

  Then it wrote paragraphs, because paragraphs were the only thing on offer.

  #landing[
    The structure existed. It just had nowhere to go.
  ]
]

// ═══════════════════════════════════════════════════════════════════════════
// 4 — The three losses
// ═══════════════════════════════════════════════════════════════════════════
#slide[
  #headline[Three things go missing, every hop]

  #v(0.2em)
  #align(center, diagram(
    spacing: (21mm, 9mm),
    dnode((0, 0), [*Model A*\ drafts]),
    dnode((1, 0), [*Model B*\ merges]),
    dnode((2, 0), [*Human*\ reviews]),
    dnode((3, 0), [*Model C*\ reports]),
    edge((0, 0), (1, 0), "->", label: elabel[hedge], label-side: left),
    edge((1, 0), (2, 0), "->", label: elabel[source], label-side: left),
    edge((2, 0), (3, 0), "->", label: elabel[disagreement], label-side: left),
  ))
  #v(0.5em)

  #grid(columns: (1fr, 1fr, 1fr), gutter: 8mm,
    [*Hedges.* "suggests" becomes "shows" becomes "we found."],
    [*Sources.* A citation does not survive a paraphrase.],
    [*Disagreement.* Two views go in, the winner comes out.],
  )
]

// ═══════════════════════════════════════════════════════════════════════════
// 5 — The core problem
// ═══════════════════════════════════════════════════════════════════════════
#slide[
  #headline[Prose never tells you what it dropped]

  #v(0.6em)

  A summary that lost every citation looks exactly like a summary that had
  none to lose.

  #v(0.6em)

  No error. No warning. No diff.

  #alarm[
    Every step reports success, and the artifact quietly gets worse.
    This is the whole problem.
  ]
]

// ═══════════════════════════════════════════════════════════════════════════
// 6 — The measurement
// ═══════════════════════════════════════════════════════════════════════════
#slide[
  #headline[So we measured it]

  Five documents. Five hops each. Two models. Ninety claims. A different
  model reads the far end and reports what survived.

  #v(0.6em)
  ```
                        tokens   claims kept   hedges lost   sources kept
  no summarising          1.00        90/90          0/90          50/50
  prose, five hops        0.29        72/90         42/90           1/50
  smysl, five hops        0.49        90/90          0/90          50/50
  ```
  #v(0.4em)

  #alarm[
    Of fifty claims that named where they came from, *one* still named it.
    Not one in ten. One.
  ]
]

// ═══════════════════════════════════════════════════════════════════════════
// 7 — Why the measurement is trustworthy
// ═══════════════════════════════════════════════════════════════════════════
#slide[
  #headline[The top row is why you can believe the other two]

  #v(0.5em)

  Same judge. Same prompt. Reading each document *before* any summarising
  happened.

  #v(0.5em)

  It found every hedge and every source, and declined to rule on nothing.
  If the judge were blind, that row would show losses too.

  #landing[
    So what went missing went missing *in the chain* — not in the instrument
    measuring the chain.
  ]
]

// ═══════════════════════════════════════════════════════════════════════════
// 8 — Fix one
// ═══════════════════════════════════════════════════════════════════════════
#slide[
  #headline["Just use a schema"]

  #v(0.5em)

  Pass JSON between the steps instead of text. This genuinely fixes the
  machine-to-machine problem — a `confidence` field does not evaporate.

  #v(0.6em)

  Then the artifact becomes `{"claims":[{"id":"c-0417","conf":0.6}]}` and the
  person in the chain stops reading it.

  #alarm[
    Nobody reviews a wire format. The one place a human could have caught the
    drift is now the one place they cannot see.
  ]
]

// ═══════════════════════════════════════════════════════════════════════════
// 9 — Fix two
// ═══════════════════════════════════════════════════════════════════════════
#slide[
  #headline["Just tell the model to be careful"]

  #v(0.5em)

  Put *preserve all hedges and citations* in the prompt. Worth doing. It
  helps, somewhat.

  #v(0.6em)

  Now: when the summariser returns, what checks whether it complied?

  #v(0.5em)

  Nothing does. There is no assertion to fail, no exit code, no diff.

  #alarm[
    You have added an intention to a system that cannot notice intentions
    being broken.
  ]
]

// ═══════════════════════════════════════════════════════════════════════════
// 10 — The document
// ═══════════════════════════════════════════════════════════════════════════
#slide[
  #headline[One artifact, for the machine and the person]

  #v(0.3em)
  ```
  @evidence e/pool-wait { status: measured, source: { kind: metric, ref: "pool.wait_ms" } }
  ~ Pool acquisition wait rose from 2 ms to 310 ms.

  @claim c/pool-saturation { status: inferred, grounds: [e/pool-wait] }
  ~ The eu-west connection pool is saturated.

  @rel c/canary-clean --rebuts--> c/pool-saturation { weight: 0.6 }
  ```
  #v(0.4em)

  #landing[
    There is no separate machine version. The bytes that cross the wire are
    the bytes a reviewer opens.
  ]
]

// ═══════════════════════════════════════════════════════════════════════════
// 11 — The ladder
// ═══════════════════════════════════════════════════════════════════════════
#slide[
  #headline[Confidence can fall. It can never rise.]

  #v(0.2em)
  #grid(columns: (auto, 1fr), gutter: 12mm, align: horizon,
    align(center, diagram(
      spacing: (5mm, 5.5mm),
      dnode((0, 0), [*measured* — an instrument recorded it], fill: ink_recap_bg),
      dnode((0, 1), [*cited* — a source you can open says so]),
      dnode((0, 2), [*derived* — computed from what is above]),
      dnode((0, 3), [*inferred* — a model reasoned it out]),
      dnode((0, 4), [*speculative* — offered, not yet grounded]),
      // Points down, because that is what the label says. The rationale's
      // version of this figure draws the arrow upward against a "moves down"
      // caption, which reads as a contradiction on a slide.
      // `label-side` is relative to the direction of travel, so a downward arrow
      // needs `left` to put its caption on the page's right, clear of the ladder.
      edge((1, -0.3), (1, 4.3), "->", stroke: 1.1pt + ink_accent,
        label: text(size: 12pt, fill: ink_accent)[a hop may\ only move\ *down*],
        label-side: left),
    )),
    [
      Every claim carries how sure the document is entitled to be.

      #v(0.5em)

      It is a field a program checks — not a tone of voice a summariser can
      talk itself out of.
    ],
  )
]

// ═══════════════════════════════════════════════════════════════════════════
// 12 — The two rules
// ═══════════════════════════════════════════════════════════════════════════
#slide[
  #headline[Two rules stop a guess becoming a fact]

  #v(0.6em)
  #grid(columns: (1fr, 1fr), gutter: 10mm,
    block(fill: ink_call_bg, stroke: (left: 2.5pt + ink_accent),
      inset: 12pt, radius: 1pt, width: 100%)[
      *Rule T — at the door*

      #v(0.4em)
      A model reasoning from its own knowledge may claim `inferred` and no
      more, however confidently it writes.
    ],
    block(fill: ink_call_bg, stroke: (left: 2.5pt + ink_accent),
      inset: 12pt, radius: 1pt, width: 100%)[
      *Rule M — inside the graph*

      #v(0.4em)
      A claim can never be stronger than the weakest thing it rests on.
    ],
  )

  #landing[
    Checked mechanically, on every claim, every time — not left to whoever is
    doing the summarising.
  ]
]

// ═══════════════════════════════════════════════════════════════════════════
// 13 — Disagreement
// ═══════════════════════════════════════════════════════════════════════════
#slide[
  #headline[Disagreement is part of the document]

  #v(0.5em)
  #align(center, diagram(
    spacing: (34mm, 10mm),
    dnode((0, 0), [*c/canary-clean*\ the canary was clean]),
    dnode((1, 0), [*c/pool-saturation*\ the pool is saturated]),
    edge((0, 0), (1, 0), "->", stroke: 1pt + ink_alarm,
      label: elabel[rebuts], label-side: left),
  ))
  #v(0.5em)

  The objection sits next to the claim it argues with — not in a review
  thread nobody opens twice.

  #landing[
    Cutting a document to fit a budget must keep the rebuttal with the claim.
    A one-sided summary is not an option the tool offers.
  ]
]

// ═══════════════════════════════════════════════════════════════════════════
// 14 — The cost
// ═══════════════════════════════════════════════════════════════════════════
#slide[
  #headline[Pay a model once, at the entrance]

  #v(0.4em)
  #align(center, diagram(
    spacing: (15mm, 11mm),
    dnode((0, 0), [prose\ pipeline], fill: rgb("#f7ece8")),
    dnode((1, 0), [call], fill: rgb("#f7ece8")),
    dnode((2, 0), [call], fill: rgb("#f7ece8")),
    dnode((3, 0), [call], fill: rgb("#f7ece8")),
    dnode((4, 0), [call], fill: rgb("#f7ece8")),
    dnode((5, 0), [call], fill: rgb("#f7ece8")),
    dnode((0, 1), [smysl\ pipeline], fill: ink_recap_bg),
    dnode((1, 1), [`ingest`], fill: ink_recap_bg),
    dnode((2, 1), [free], fill: ink_recap_bg),
    dnode((3, 1), [free], fill: ink_recap_bg),
    dnode((4, 1), [free], fill: ink_recap_bg),
    dnode((5, 1), [free], fill: ink_recap_bg),
  ))
  #v(0.5em)

  Everything after the entrance — merging, selecting, ordering, rendering,
  checking — is ordinary computation. Same input, same bytes out, forever.

  #landing[
    Five model calls on the prose side of that measurement. Zero on the other.
  ]
]

// ═══════════════════════════════════════════════════════════════════════════
// 15 — Closing card
// ═══════════════════════════════════════════════════════════════════════════
#slide(config: config-page(margin: 0em))[
  #set page(fill: ink_paper)
  #block(width: 100%, height: 100%)[
    #place(top + left, dx: 9mm, dy: 9mm,
      rect(width: 100% - 18mm, height: 100% - 18mm, stroke: 1.1pt + ink_accent))
    #place(top + left, dx: 11mm, dy: 11mm,
      rect(width: 100% - 22mm, height: 100% - 22mm, stroke: 0.4pt + ink_accent))

    #place(top + center, dy: 13mm, image("images/logo.png", width: 19mm))
    #place(top + center, dy: 36mm, dots())

    #place(top + center, dy: 45mm, block(width: 76%)[
      #align(center)[
        #text(size: 27pt, weight: "bold", fill: ink_black,
          "A claim and its citation travel as one thing")
        #v(4mm)
        #text(size: 19pt, fill: ink_gray,
          "from the model that wrote it to the person who checks it.")
        #v(9mm)
        #line(length: 34%, stroke: 0.6pt + ink_accent)
        #v(8mm)
        #text(size: 17pt, style: "italic", fill: ink_smoke,
          "Nothing is translated, so nothing is lost in translation.")
      ]
    ])

    #place(bottom + center, dy: -14mm, align(center)[
      #text(size: 13pt, fill: ink_smoke, "github.com/vulogov/smysl")
      #v(2mm)
      #text(size: 11pt, fill: ink_faint, tracking: 3pt, upper("MPL-2.0 · free, and meant to stay that way"))
    ])
  ]
]
