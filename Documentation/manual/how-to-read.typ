// Front matter — how to read this book.
//
// Sits between the table of contents and Part I. Exists because the book is
// long enough that "start at page one" is bad advice for at least three of
// the four people who open it, and because a reader who cannot tell which
// chapters they can skip will skip the wrong ones.

#import "design.typ": *

#v(1.2cm)
#align(left)[
  #text(font: body_family, size: 9pt, tracking: 2pt, fill: ink_gray, upper("Before you start"))
  #v(4mm)
  #text(font: body_family, size: 30pt, weight: "regular", fill: ink_black, "How to read this book")
]
#v(8mm)
#line(length: 100%, stroke: 0.5pt + ink_rule)
#v(7mm)

This book is about twenty-nine chapters long, and reading it front to back is
the right plan for exactly one kind of reader. Everyone else should skip, and
this page is about skipping well.

Whichever path you take, *Chapters 1 and 2 are the ones not to skip.* Chapter 1
is why this exists — the problem, and how big it measurably is. Chapter 2 is
what the system is made of and the twelve rules it enforces. Every later
chapter names those rules by letter and assumes you have met them.

#section("Four ways in")

#dtable(
  (auto, auto, 1fr),
  (
    ([If you are…], [Read], [Why that path]),
    ([*Deciding whether to adopt this at all*], [1, 2, then 5], [Chapter 1 gives you the measured cost of not having it and the honest cases where you do not need it. Chapter 5 builds a real document end to end in a few pages, which is the fastest way to find out whether the shape suits you. That is about an hour, and it is enough to make the call.]),
    ([*Going to write documents by hand*], [1–8, then 21], [Part I orients you, Chapters 6–7 are the full authoring grammar, Chapter 8 is the validation loop you will live in. Chapter 21 explains what `check` is actually doing when it complains, which turns error messages from obstacles into information.]),
    ([*Wiring this into a pipeline*], [1–3, 9–12, 19–20, 23], [The trust and staging machinery is Chapters 9–12 and is the part that has real operational consequences. Chapters 19–20 are budget-bounded selection — what survives when you have fewer tokens than document. Chapter 24 is how the output reaches a person.]),
    ([*Embedding the library in your own program*], [1–2, 27–29], [Chapter 27 is the crate layout and the feature flags, Chapter 28 is a minimal working embedding, Chapter 29 runs the whole pipeline start to finish. Come back for the command chapters when you need the behaviour one of them describes.]),
  ),
)

#section("What the boxes mean")

Five kinds of block interrupt the prose, and each one is a promise about what
it contains, so you can skim on them.

#callout(label: "Why")[
  A *Why* box appears before a piece of machinery and says what goes wrong
  without it. If you already know why something exists, this is the box to
  skip. If you are lost, it is usually the box you needed.
]

#term("Term")[
  A *Term* box defines a word this book will then use precisely and without
  further apology. Every one of them is restated in the glossary, Appendix D.
]

#exercise[
  A *Try this* box is something to run, not something to read — a short,
  checkable task using files that ship in the repository, with the answer
  stated so you can tell whether you got it. Every one was run against the
  real binary before it was written down. Skipping them is fine; doing them is
  the difference between recognising the format and being able to use it.
]

#whatsnext[
  A *What's next* box ends a section by naming the next reasonable step and
  the reason for it. These are the connective tissue — if you are reading
  selectively, they tell you where the thread you are pulling actually goes.
]

#recap((
  [A *What you learned* box closes each chapter. Read these first if you are
   deciding whether a chapter is worth your time — they are the chapter's
   claims, without the evidence.],
))

#section("What you need in front of you")

Two things, both covered in Chapter 4: a built `smysl` binary, and a checkout
of the repository — because nearly every example in this book operates on
files under `fixtures/corpus/`, and the incident document
`fixtures/corpus/F1-incident.smy` is the running example from Chapter 13
onward. Commands are shown with their real output, produced by running them;
where output is long it is trimmed, and the trimming is marked.

No model access is needed for any chapter except 10 and 11, and both say so at
the top. Everything else runs offline — including Chapter 12, whose whole
subject is reporting what *would* be sent without sending it.

#callout(label: "A note on the numbers")[
  Where this book states a measurement — token ratios, claim-retention rates,
  how much a `pack` dropped — it is reporting a run, not an estimate, and the
  command that produced it is shown. Where something has not been measured,
  the book says that instead. Chapter 1's headline numbers come from two
  models and five documents, which is enough to establish an effect and not
  enough to put an error bar on it; that limitation is stated there too.
]
