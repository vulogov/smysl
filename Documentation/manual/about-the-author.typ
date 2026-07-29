// Afterword — About the author.
//
// Adapted from blackInkhaven/Book/POETRY/chapters/99-about-the-author.typ:
// same structure (portrait grid, "a work of love", "a note on cooperation",
// "where to find more", closing line) and the same voice, reskinned to this
// book's design module and turned toward this project's subject.
//
// Two things are deliberately changed rather than copied. The licence
// paragraph names MPL-2.0 rather than describing the licence as permissive,
// because MPL is weak-copyleft and the distinction is real. And the bridging
// sentence from the author's day work to the tool runs through *provenance*
// rather than metre, which is the honest version of the same observation.

#import "design.typ": *

#pagebreak(weak: true, to: "odd")

#hide(heading(
  level: 1, numbering: none, outlined: true, bookmarked: true,
  "About the Author",
))

#v(2cm)
#align(left)[
  #text(font: body_family, size: 9pt, tracking: 2pt, fill: ink_gray, upper("Afterword"))
  #v(4mm)
  #text(font: body_family, size: 36pt, weight: "regular", fill: ink_black, "About the author")
]
#v(1cm)
#line(length: 100%, stroke: 0.5pt + ink_rule)
#v(12mm)

#grid(
  columns: (56mm, 1fr),
  gutter: 7mm,
  [
    #image("../images/author-portrait.png", width: 100%)
    #v(2mm)
    #align(center, text(font: body_family, style: "italic", size: 9pt, fill: ink_gray, "Vladimir Ulogov."))
  ],
  [
    Vladimir Ulogov has spent decades building infrastructure for distributed
    systems — the kind of software that watches other software. Early in his
    career he worked on monitoring and telemetry platforms; later years took
    him into federated observability, telemetry buses, and the architecture of
    systems that have to make sense of millions of data points without losing
    the thread.

    Observability, in the end, is a discipline of *coherence* — of never
    reporting a state the system cannot account for, of insisting that every
    signal follow from something real. A monitoring system that cannot say
    where a number came from is not a monitoring system; it is a rumour with a
    dashboard.
  ],
)

#v(4mm)

That instinct is the whole of what this book describes. `smysl` records where a
claim came from, refuses to let confidence rise as it travels, and never
invents a verdict it cannot account for — and it never writes the document.
The move is the same one observability made thirty years ago, pointed at a
newer problem: as soon as several systems hand work to each other, somebody has
to be able to ask *where did this come from* and get an answer that is not a
guess. Language models made that question urgent. They did not make it new.

What makes him slightly unusual in his corner of the industry is a tendency to
write his own tools — not small utilities, but programming languages. The Bund
language (its compiler, its VM, its document store, its parser) lives in a long
series of Rust crates on crates.io. `rust_dynamic`, `rust_multistackvm`,
`bundcore` — each is a building block that exists because the off-the-shelf
options didn't fit the shape of the work. `smysl` grew the same way: the
question of how to move a claim between two systems without losing what backs
it turned out to have no answer worth adopting, so it got one.

#section("A work of love")

`smysl` is open source, under the Mozilla Public License — you can read it,
fork it, study it, modify it, and pass it on, and the licence asks only that
improvements to its own files travel with them. Strictly speaking the licence
also lets you sell it; the author would, gently but firmly, disagree with your
doing that. `smysl` was not designed as a #emph[for sale] project. It is a work
of love made for the people who can least afford to pay for software — the
researcher on a battered laptop, the graduate student whose pipeline has to be
defensible at a viva, the small team accountable for a system nobody funded
properly — and turning it into a commercial product would betray the reason it
exists. It carries no analytics, no telemetry, no upsell. Fifteen of its
seventeen commands cannot open a socket at all, and the two that can will tell
you what they would send before they send it. The binary will never phone home.

#section("A note on cooperation")

Vladimir believes firmly in the human capacity for mutual help — that we make
better work, and live better lives, when we share what we know and what we
build. Open source is one of the most concrete expressions of cooperation our
era has produced: code read, improved, and passed forward without payment,
without permission, by people who will never meet.

There is a version of the current moment in which machines talk mostly to other
machines and people are handed the summary afterwards. This project is a small
argument against that version — not by keeping models out, but by insisting
that whatever passes between them stays legible to the person who has to answer
for it. If this book helps you hand somebody a document they can actually check
— or lets you check one that was handed to you — that is enough.

#section("Where to find more")

/ *GitHub*: `@vulogov` — the source for `smysl`, Bund, and the dozen-plus Rust crates that carry the infrastructure. Issues and pull requests welcome.
/ *LinkedIn*: `/in/vladimirulogov` — posts on observability, the occasional long-form essay.
/ *YouTube*: `@vulogov` — talks and walkthroughs from the conference trail.

#v(8mm)

#text(font: body_family, style: "italic", size: 11pt, fill: ink_gray,
  "If the tool ever gets in the way of the work instead of out of it, open an issue on GitHub. The author reads them."
)

#v(2cm)
#align(center, text(font: body_family, size: 8pt, fill: ink_faint, tracking: 4pt,
  upper("end of the book")))
