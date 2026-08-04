// smysl — The Manual: book design tokens + page chrome.
//
// Book-scale sibling of the single-document design used in SMYSL_RATIONALE.typ
// and SMYSL_FORMAT_GUIDE.typ. Same palette, same term/callout/recap/screen/dtable
// vocabulary, extended with cover/part/chapter machinery adapted from
// blackInkhaven's Book/RESEARCH/design.typ pattern (a cream cover, a slate-blue
// accent, chapter openers with a big number) but reskinned with the smysl owl
// and its own accent, and scaled for a 100-200 page technical manual rather
// than a single scannable document.

#import "@preview/fletcher:0.5.8" as fletcher: diagram, node, edge

// ── Palette — identical across all four smysl documents ────────────────────
#let ink_black    = rgb("#1a1a1a")
#let ink_gray     = rgb("#5d5d5d")
#let ink_faint    = rgb("#9a9a9a")
#let ink_rule     = rgb("#c6c0b5")
#let ink_accent   = rgb("#2f5d7a")
#let ink_smoke    = rgb("#7d736a")
#let ink_paper    = rgb("#fdfaf3")
#let ink_term     = rgb("#2f5d7a")
#let ink_code_bg  = rgb("#f3eee4")
#let ink_call_bg  = rgb("#f6f1e6")
#let ink_term_bg  = rgb("#eef3f7")
#let ink_recap    = rgb("#3f6b4a")
#let ink_recap_bg = rgb("#e9f3ea")

#let body_family = ("Libertinus Serif", "New Computer Modern")
#let mono_family = ("DejaVu Sans Mono",)

#let book_title    = "The smysl Manual"
#let book_subtitle = "A complete, worked guide to every workflow — the CLI, the document format, and the library beneath both"
#let book_author   = "Vladimir Ulogov"
#let book_year     = "2026"
#let book_version  = "smysl 0.11.0 · format smysl/0.1 · kernel smysl.kernel/0.1"

#let book_page = (
  paper: "a4",
  margin: (inside: 26mm, outside: 20mm, top: 22mm, bottom: 24mm),
)

// ── Part divider ─────────────────────────────────────────────────────────
#let part(number: "I", title: "") = {
  pagebreak(weak: true)
  hide(heading(level: 1, numbering: none, outlined: true, bookmarked: true, [Part #number — #title]))
  v(7cm)
  align(center)[
    #text(font: body_family, size: 11pt, tracking: 3pt, fill: ink_gray, upper("Part " + number))
    #v(6mm)
    #line(length: 36%, stroke: 0.5pt + ink_rule)
    #v(6mm)
    #text(font: body_family, size: 26pt, weight: "bold", fill: ink_black, title)
  ]
}

// ── Chapter opener ───────────────────────────────────────────────────────
#let chapter(number: 0, title: "") = {
  pagebreak(weak: true)
  hide(heading(level: 1, numbering: none, outlined: true, bookmarked: true, [#str(number) — #title]))
  v(1.6cm)
  align(left)[
    #text(font: body_family, size: 9pt, tracking: 2pt, fill: ink_gray, upper("Chapter " + str(number)))
    #v(1mm)
    #text(font: body_family, size: 68pt, weight: "bold", fill: ink_accent, str(number))
    #v(-5mm)
    #text(font: body_family, size: 21pt, weight: "regular", fill: ink_black, title)
  ]
  v(9mm)
  line(length: 100%, stroke: 0.5pt + ink_rule)
  v(7mm)
}

// ── Appendix opener ──────────────────────────────────────────────────────
#let appendix(letter: "A", title: "") = {
  pagebreak(weak: true)
  hide(heading(level: 1, numbering: none, outlined: true, bookmarked: true, [Appendix #letter — #title]))
  v(1.6cm)
  align(left)[
    #text(font: body_family, size: 9pt, tracking: 2pt, fill: ink_gray, upper("Appendix " + letter))
    #v(1mm)
    #text(font: body_family, size: 68pt, weight: "bold", fill: ink_accent, letter)
    #v(-5mm)
    #text(font: body_family, size: 21pt, weight: "regular", fill: ink_black, title)
  ]
  v(9mm)
  line(length: 100%, stroke: 0.5pt + ink_rule)
  v(7mm)
}

// ── In-chapter headings ──────────────────────────────────────────────────
#let section(title) = {
  hide(heading(level: 2, numbering: none, outlined: true, title))
  block(sticky: true, above: 8mm, below: 3.2mm,
    text(font: body_family, size: 14pt, weight: "bold", fill: ink_black, title))
}
#let subsection(title) = block(sticky: true, above: 5.5mm, below: 2.4mm,
  text(font: body_family, size: 11.5pt, weight: "bold", fill: ink_black, title))

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

// ── Note / why / next callout ───────────────────────────────────────────
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

// ── "What's next, and why" — the connective-tissue box every workflow
//    section ends on, naming the next reasonable step and the reason for it. ──
#let whatsnext(body) = {
  v(2mm)
  block(
    fill: ink_recap_bg, stroke: (left: 2pt + ink_recap),
    inset: (left: 9pt, right: 9pt, top: 7pt, bottom: 7pt),
    width: 100%, radius: 1pt, breakable: false,
    {
      text(font: body_family, size: 8pt, weight: "bold", fill: ink_recap, tracking: 1.5pt, "WHAT'S NEXT, AND WHY")
      v(2mm)
      body
    },
  )
  v(2mm)
}

// ── "Try this" — a short, checkable exercise the reader can actually run.
//    Distinct from `callout` on purpose: a callout is something to read, this
//    is something to do, and the reader should be able to tell at a glance
//    which is which while skimming. Every exercise in this book was run
//    against the real binary before it was written down. ──
#let ink_try    = rgb("#8a5a2b")
#let ink_try_bg = rgb("#f7f0e6")

#let exercise(label: "Try this", body) = {
  v(2mm)
  block(
    fill: ink_try_bg, stroke: (left: 2pt + ink_try),
    inset: (left: 9pt, right: 9pt, top: 7pt, bottom: 7pt),
    width: 100%, radius: 1pt, breakable: false,
    {
      text(font: body_family, size: 8pt, weight: "bold", fill: ink_try, tracking: 1.5pt, upper(label))
      v(2mm)
      body
    },
  )
  v(2mm)
}

// ── Chapter-end exercises, and their answers ─────────────────────────────
//
// Answers sit immediately below the questions rather than in an appendix.
// That is a deliberate trade: it costs a reader the chance to be tested
// honestly, and it buys a reader who is *not* at a terminal the ability to
// still learn something from the exercise. This book assumes the second
// reader is more common. The answers are set smaller and greyer so the eye
// can be told to stop at the questions.
#let exercises(items) = {
  v(7mm)
  block(
    fill: ink_try_bg, stroke: (left: 2pt + ink_try),
    inset: (left: 9pt, right: 9pt, top: 8pt, bottom: 8pt),
    width: 100%, radius: 1pt, breakable: true,
    {
      text(font: body_family, size: 9pt, weight: "bold", fill: ink_try, tracking: 1.5pt, "TRY THIS")
      v(2mm)
      enum(..items)
    },
  )
}

#let answers(items) = {
  v(2mm)
  block(
    stroke: (left: 1pt + ink_rule),
    inset: (left: 9pt, right: 9pt, top: 5pt, bottom: 5pt),
    width: 100%, breakable: true,
    {
      text(font: body_family, size: 8pt, weight: "bold", fill: ink_faint, tracking: 1.5pt, "ANSWERS")
      v(1.5mm)
      set text(size: 9.5pt, fill: ink_gray)
      // Inline code inherits the book-wide 9.5pt rule, which is set against 11pt body
      // text. At the answers' 9.5pt it reads a size too large and, worse, an over-long
      // span cannot break - it shoves the line it sits on into a river of white space.
      // Scaling it here keeps the proportion the rest of the book has.
      show raw.where(block: false): it => box(
        fill: ink_code_bg, inset: (x: 2pt, y: 0pt), outset: (y: 1.5pt), radius: 1pt,
        text(font: mono_family, size: 8.2pt, it),
      )
      enum(..items)
    },
  )
  v(3mm)
}

// ── Chapter-end recap ────────────────────────────────────────────────────
#let recap(items) = {
  v(7mm)
  block(
    fill: ink_recap_bg, stroke: (left: 2pt + ink_recap),
    inset: (left: 9pt, right: 9pt, top: 8pt, bottom: 8pt),
    width: 100%, radius: 1pt, breakable: false,
    {
      text(font: body_family, size: 9pt, weight: "bold", fill: ink_recap, tracking: 1.5pt, "WHAT YOU LEARNED")
      v(2mm)
      list(..items)
    },
  )
}

// ── Terminal screen — a faithful, unbreakable rendering of real CLI output ──
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

// ── Styled reference table, header repeating across a page break ──────────
#let dtable(columns, rows) = {
  v(2mm)
  table(
    columns: columns,
    stroke: (y: 0.5pt + ink_rule),
    fill: (x, y) => if y == 0 { ink_term_bg } else { white },
    inset: (x: 7pt, y: 5pt),
    align: left + horizon,
    table.header(..rows.at(0).map(cell =>
      text(font: body_family, size: 8.5pt, weight: "bold", fill: ink_black, tracking: 0.3pt, upper(cell))
    )),
    ..rows.slice(1).map(row => row.map(cell =>
      text(font: body_family, size: 9.5pt, fill: ink_black, cell)
    )).flatten()
  )
  v(2mm)
}

// ── Shared fletcher node style, for the handful of pipeline diagrams ───────
#let dnode(pos, body, fill: ink_call_bg) = node(
  pos, align(center, text(font: body_family, size: 8.5pt, body)),
  stroke: 0.6pt + ink_rule, fill: fill, corner-radius: 2pt, inset: 6pt,
)

// ═══════════════════════════════════════════════════════════════════════
// Master document wrapper — cover, contents, running header/footer.
// ═══════════════════════════════════════════════════════════════════════
#let book(pages) = {
  set document(title: book_title, author: book_author)
  set text(font: body_family, size: 11pt, fill: ink_black, lang: "en")
  set par(leading: 0.72em, justify: true)

  show raw.where(block: true): it => block(
    fill: ink_code_bg, stroke: 0.5pt + ink_rule, inset: 7pt, radius: 2pt, width: 100%,
    text(font: mono_family, size: 9pt, it),
  )
  show raw.where(block: false): it => box(
    fill: ink_code_bg, inset: (x: 2pt, y: 0pt), outset: (y: 2pt), radius: 1pt,
    text(font: mono_family, size: 9.5pt, it),
  )

  // ── Cover — cream ground, slate-blue rule frame, the owl lockup. ──
  set page(paper: book_page.paper, margin: 0pt, numbering: none, header: none, fill: ink_paper)
  block(width: 100%, height: 100%)[
    #place(top + left, dx: 12mm, dy: 12mm,
      rect(width: 100% - 24mm, height: 100% - 24mm, stroke: 1pt + ink_accent))
    #place(top + left, dx: 14mm, dy: 14mm,
      rect(width: 100% - 28mm, height: 100% - 28mm, stroke: 0.4pt + ink_accent))
    #place(top + center, dy: 46mm, image("../images/logo.png", width: 26mm))
    #place(top + center, dy: 84mm, {
      let dot(dx, r) = place(top + center, dx: dx, dy: 0pt, circle(radius: r, fill: ink_accent))
      dot(-14mm, 1.3mm); dot(-7mm, 0.9mm); dot(0mm, 1.8mm); dot(7mm, 0.9mm); dot(14mm, 1.3mm)
    })
    #place(top + center, dy: 100mm, block(width: 74%)[
      #set par(justify: false)
      #align(center)[
        #text(font: body_family, size: 12pt, tracking: 4pt, fill: ink_smoke, upper("The smysl Manual"))
        #v(11mm)
        #text(font: body_family, size: 27pt, weight: "bold", fill: ink_black,
          "Working with the CLI, the Document Format, and the Library")
        #v(6mm)
        #line(length: 55%, stroke: 0.6pt + ink_accent)
        #v(6mm)
        #text(font: body_family, size: 12pt, style: "italic", fill: ink_smoke, book_subtitle)
      ]
    ])
    #place(bottom + center, dy: -30mm, align(center)[
      #text(font: body_family, size: 10pt, fill: ink_smoke, book_author)
      #v(2mm)
      #text(font: body_family, size: 9pt, fill: ink_smoke, book_year + " · " + book_version)
    ])
  ]
  pagebreak()

  // ── Contents ──
  set page(margin: book_page.margin, fill: white)
  text(font: body_family, size: 22pt, weight: "bold", fill: ink_black, "Contents")
  v(7mm)
  outline(title: none, indent: auto, depth: 2)
  pagebreak()

  // ── Body ──
  set page(
    numbering: "1", number-align: center,
    header: context {
      if counter(page).get().first() > 1 {
        align(center, text(font: body_family, size: 8pt, fill: ink_faint, tracking: 1.5pt, upper(book_title)))
      }
    },
  )
  counter(page).update(1)
  for p in pages [ #p ]
}
